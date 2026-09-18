//! 组 14：AI 输入图片缓存（6 个命令）。
//!
//! | 命令 | 入参 | 返回 |
//! |---|---|---|
//! | `image_list` | — | [`ImageCacheItem`][] |
//! | `image_total_size` | — | `number`（字节） |
//! | `image_delete` | `ids: string[]` | `number`（实际删除数） |
//! | `image_load_data_url` | `id` | `string \| null` |
//! | `image_save_from_path` | `path` | `string`（图片 id） |
//! | `image_save_from_bytes` | `data`（data URL 或裸 base64）、`refKey?` | `string`（图片 id） |
//!
//! 存储语义见 [`civilcalc_store::LlmImageStore`]：图片按**内容 SHA-1 去重**，
//! 物理删除只走 [`image_delete`]，引用键只作展示与「删除前提示」用。
//!
//! ## 🔴 两个保存入口走**同一套**处理管线
//!
//! `image_save_from_path`（选文件）与 `image_save_from_bytes`（粘贴剪贴板）
//! 都必须经过 [`process_image_bytes`]：
//!
//! 1. **EXIF 方向修正**（相机竖拍场景；仅 JPEG 带 EXIF，其它格式忽略）
//! 2. **长边缩放到 1024**（更小则保持原尺寸，不放大）
//! 3. **统一转 JPEG q80**（透明区域填黑底）
//!
//! 否则同一张图会因入口不同而存成两份不同字节，内容去重也救不回来。
//!
//! 处理完得到 JPEG 字节，按内容哈希去重写入 —— 同一张图即使从不同路径导入也只存一份。
//!
//! ## ref_key 用源路径
//!
//! `image_save_from_path` 没有对应的 `add_ref` / `release_ref` 命令（契约未列），
//! 所以每导入一次就用**规范化后的源路径**作 ref_key。同一文件重复导入只计 1 次引用；
//! 从不同路径导入同一内容则计多次。`image_delete` 是强制删除，不受引用影响。
//!
//! [`ImageCacheItem`]: ImageCacheItem
//! [`image_delete`]: image_delete

use std::path::{Path, PathBuf};

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::metadata::Orientation;
use serde::Serialize;
use tauri::State;

use crate::error::{CmdResult, CommandError};
use crate::state::AppState;
use civilcalc_store::LlmImageStore;

/// 输出 JPEG 质量（对齐源 Android `DEFAULT_QUALITY`）。
const JPEG_QUALITY: u8 = 80;
/// 长边上限（对齐源 Android `DEFAULT_MAX_EDGE`）。
const MAX_EDGE: u32 = 1024;

/// `image_list` 的单条（前端九宫格 / 管理页用）。
///
/// 不含本地路径：预览统一走 [`image_load_data_url`]，避免把 `app_data` 内部路径
/// 泄漏给 webview，也避免路径随重装失效。
///
/// [`image_load_data_url`]: image_load_data_url
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageCacheItem {
    /// 图片 ID（落盘文件名是 `<sha1>.<ext>`，ID 是另发的 UUID）。
    pub id: String,
    /// 占用字节（文件真实大小）。
    pub size_bytes: i64,
    /// 入库时间（毫秒）。
    pub created_at: i64,
    /// 被引用处数（被多少条输入历史 / 公式 / 附件引用）。
    pub ref_count: usize,
}

/// 打开图片存储（目录不可用转成命令错误）。
fn open_store(dir: PathBuf) -> CmdResult<LlmImageStore> {
    LlmImageStore::new(dir).map_err(|e| CommandError::Storage {
        message: format!("图片目录不可用：{e}"),
    })
}

#[tauri::command]
pub fn image_list(state: State<'_, AppState>) -> CmdResult<Vec<ImageCacheItem>> {
    let store = open_store(state.paths.images_dir())?;
    Ok(store
        .list()
        .into_iter()
        .map(|info| {
            let rc = info.ref_count();
            ImageCacheItem {
                id: info.id,
                size_bytes: info.size_bytes,
                created_at: info.created_at,
                ref_count: rc,
            }
        })
        .collect())
}

#[tauri::command]
pub fn image_total_size(state: State<'_, AppState>) -> CmdResult<i64> {
    let store = open_store(state.paths.images_dir())?;
    Ok(store.total_size_bytes())
}

#[tauri::command]
pub fn image_delete(state: State<'_, AppState>, ids: Vec<String>) -> CmdResult<usize> {
    let store = open_store(state.paths.images_dir())?;
    Ok(store.delete(&ids))
}

#[tauri::command]
pub fn image_load_data_url(state: State<'_, AppState>, id: String) -> CmdResult<Option<String>> {
    let store = open_store(state.paths.images_dir())?;
    Ok(store.load_data_url(&id))
}

#[tauri::command]
pub fn image_save_from_path(state: State<'_, AppState>, path: String) -> CmdResult<String> {
    let src = PathBuf::from(&path);
    if !src.exists() {
        return Err(CommandError::InvalidArgument {
            message: format!("图片文件不存在：{path}"),
        });
    }
    let bytes = process_external_image(&src).ok_or_else(|| CommandError::InvalidArgument {
        message: format!("无法读取或解码该图片文件：{path}"),
    })?;

    // ref_key 用规范化后的源路径：同文件重复导入只计 1 次引用。
    let ref_key = std::fs::canonicalize(&src)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.clone());

    let store = open_store(state.paths.images_dir())?;
    store.save_bytes(&bytes, "image/jpeg", &ref_key).ok_or_else(|| {
        CommandError::Storage {
            message: "图片落盘失败".to_string(),
        }
    })
}

/// 从**字节**保存一张图片（剪贴板粘贴用）。
///
/// ## 为什么需要它
///
/// 剪贴板里的图片（截图、复制的图）**没有文件路径** ——
/// `image_save_from_path` 用不上，只能由前端读成 base64 传过来。
///
/// ## 与 `image_save_from_path` 的关系
///
/// 走**完全相同**的处理管线（EXIF 方向 → 长边 ≤ 1024 → JPEG q80），
/// 落盘时同样按内容 SHA-1 去重。所以「粘贴截图」与「选文件」得到的
/// 图片在库里没有区别。
///
/// ## `ref_key`
///
/// 没有源路径可用，默认用**内容哈希**做引用键（`clip:<sha1>`）——
/// 于是「同一张截图粘贴两次」只计 1 次引用，而不是 2 次。
/// 调用方也可以显式传一个键（如公式 id），便于「删除前提示」时归因。
///
/// @param data data URL（`data:image/png;base64,...`）或**裸 base64**
#[tauri::command]
pub fn image_save_from_bytes(
    state: State<'_, AppState>,
    data: String,
    ref_key: Option<String>,
) -> CmdResult<String> {
    let raw = decode_image_payload(&data).ok_or_else(|| CommandError::InvalidArgument {
        message: "剪贴板图片数据不是合法的 base64 或 data URL".to_string(),
    })?;

    let bytes = process_image_bytes(&raw).ok_or_else(|| CommandError::InvalidArgument {
        message: "无法解码剪贴板里的图片（格式不支持或数据不完整）".to_string(),
    })?;

    let key = ref_key.unwrap_or_else(|| format!("clip:{}", civilcalc_store::sha1::sha1_hex(&raw)));

    let store = open_store(state.paths.images_dir())?;
    store
        .save_bytes(&bytes, "image/jpeg", &key)
        .ok_or_else(|| CommandError::Storage {
            message: "图片落盘失败".to_string(),
        })
}

/// 解析前端传来的图片载荷：data URL 或裸 base64。
///
/// 两种都接受，是因为浏览器 `FileReader.readAsDataURL` 给的是前者，
/// 而有的实现只给后者；让后端兼容比让前端判断更省事。
fn decode_image_payload(data: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;

    let s = data.trim();
    // 只取 `,` 之后的部分（data URL 的头部是元信息，不是数据）
    let b64 = match s.find(',') {
        Some(i) if s.starts_with("data:") => &s[i + 1..],
        _ => s,
    };

    // 去掉换行/空白（有的剪贴板实现会插换行）
    let cleaned: String = b64.chars().filter(|c| !c.is_whitespace()).collect();

    // 空载荷要显式拒掉：base64 解码空串会「成功」返回空字节，
    // 于是错误会被推迟到解码图片时才报，错误信息也就变成了
    // 「无法解码图片」而不是「没收到数据」—— 对排查没有帮助。
    if cleaned.is_empty() {
        return None;
    }

    base64::engine::general_purpose::STANDARD
        .decode(&cleaned)
        .ok()
        .or_else(|| {
            base64::engine::general_purpose::URL_SAFE
                .decode(&cleaned)
                .ok()
        })
}

/// 把前端传来的「图片引用」解析成 **data URL** 列表。
///
/// ## 🔴 为什么需要它（这是个真 bug 的修复）
///
/// 契约里 `images: string[]` 当初**没写清是 id 还是 data URL**，
/// 结果前端一直传 **id**（附图时图片就已入库、拿到 id），
/// 而后端两处都按 data URL 用，症状各不相同：
///
/// | 调用方 | 后果 |
/// |---|---|
/// | `normalize_from_*` | `store.save("<uuid>", …)` 解析失败 → **图片被静默丢掉**（请求成功但没有图） |
/// | `model_test_send` | 把 `<uuid>` 当成图片 URL 拼进请求 → API 返回 400 → **请求直接失败** |
///
/// 两个症状都只在「真的附了图 + 真的配了可用 Key」时才出现，
/// 所以单测与本地开发一直没暴露。
///
/// 现在统一在这里解析：**先当 id 试，再当 data URL 试** ——
/// 两种形态都认，前端不必改，也不会再出现「两边各错一半」。
///
/// 认不出来的**跳过但不占位**（调用方依赖序号连续）。
pub(crate) fn resolve_image_refs(state: &AppState, refs: &[String]) -> Vec<String> {
    resolve_image_refs_with_ids(state, refs).0
}

/// [`resolve_image_refs`] 的「带 id」版本：返回 `(data URL, 图片 id)` 两个
/// **等长、同序**的列表。
///
/// ## 为什么要把 id 也带出来
///
/// 归一化命令（`normalize_from_*` / `refine_formula`）返回的 schema 必须记住
/// **自己带了哪几张图**（`schema.imageIds`）—— 否则：
///
/// - 详解正文里的 `{{img:N}}` 渲染不出来（`ExplanationPanel` 靠 `imageIds` 解析）
/// - 再次微调时「父公式附图前置」拿不到父图，序号会错位
/// - 图片缓存页的「被引用」提示永远显示 0
///
/// 之前只回传 data URL，命令层没有 id 可填，`imageIds` 一直是空数组 ——
/// 图发出去了，公式却不记得自己有图。
pub(crate) fn resolve_image_refs_with_ids(
    state: &AppState,
    refs: &[String],
) -> (Vec<String>, Vec<String>) {
    resolve_refs_in(state.paths.images_dir(), refs)
}

/// 核心实现。
///
/// 单独拆出来是为了**可单测**：`AppState` 需要库 + 配置 + 检索索引才能构造，
/// 而这里只用到图片目录。测试直接传临时目录即可覆盖全部分支。
fn resolve_refs_in(dir: PathBuf, refs: &[String]) -> (Vec<String>, Vec<String>) {
    let Ok(store) = open_store(dir) else {
        return (Vec::new(), Vec::new());
    };

    let mut urls: Vec<String> = Vec::with_capacity(refs.len());
    let mut ids: Vec<String> = Vec::with_capacity(refs.len());

    for r in refs {
        // ① 正常形态：已入库的图片 id（ref 本身就是 id）
        if let Some(url) = store.load_data_url(r) {
            urls.push(url);
            ids.push(r.clone());
            continue;
        }
        // ② 兼容形态：直接给 data URL（未入库）→ 落盘后取回。
        //    内容按 SHA-1 去重，所以同一张图重复传也只存一份。
        //    ref_key 用固定标记：这条分支只在「前端直接给 data URL」时走到，
        //    正常路径（附图即入库）不会经过。
        if let Some(id) = store.save(r, "inline") {
            if let Some(url) = store.load_data_url(&id) {
                urls.push(url);
                ids.push(id);
            }
        }
        // 取不到的**跳过但不占位**（调用方依赖序号连续）
    }

    (urls, ids)
}

/// 单测入口：只关心 data URL 列表的那批老用例走这里
#[cfg(test)]
fn resolve_image_refs_in(dir: PathBuf, refs: &[String]) -> Vec<String> {
    resolve_refs_in(dir, refs).0
}

/// 把任意外部图片**文件**转成「长边 ≤ 1024 的 JPEG」字节。
///
/// 解码失败 / 处理失败返回 `None`，由调用方转成命令错误提示用户。
fn process_external_image(path: &Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    process_image_bytes(&bytes)
}

/// 把任意图片**字节**转成「长边 ≤ 1024 的 JPEG」字节（含 EXIF 方向修正、透明填黑底）。
///
/// 「选文件」与「粘贴剪贴板图片」共用这一份处理 —— 两条入口必须得到
/// 完全一致的图片，否则同一张图会因入口不同而在库里存成两份不同字节。
fn process_image_bytes(bytes: &[u8]) -> Option<Vec<u8>> {
    let orientation = if image::guess_format(bytes).ok() == Some(image::ImageFormat::Jpeg) {
        jpeg_orientation(bytes)
    } else {
        Orientation::NoTransforms
    };
    let mut img = image::load_from_memory(bytes).ok()?;
    img.apply_orientation(orientation);

    // 长边缩放到 MAX_EDGE（不放大）。
    let (w, h) = (img.width(), img.height());
    let long = w.max(h);
    if long > MAX_EDGE {
        let scale = MAX_EDGE as f32 / long as f32;
        let nw = ((w as f32 * scale).max(1.0)).round() as u32;
        let nh = ((h as f32 * scale).max(1.0)).round() as u32;
        img = img.resize(nw, nh, FilterType::Lanczos3);
    }

    // 统一转 JPEG：先把 RGBA 合成到黑底（透明区域填黑底，对齐源 Android），再编码 q80。
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    let mut rgb = image::RgbImage::new(w, h);
    for (x, y, px) in rgba.enumerate_pixels() {
        // 以黑底合成：out = rgb * alpha（alpha 归一化）
        let a = px[3] as f32 / 255.0;
        let r = (px[0] as f32 * a).round() as u8;
        let g = (px[1] as f32 * a).round() as u8;
        let b = (px[2] as f32 * a).round() as u8;
        rgb.put_pixel(x, y, image::Rgb([r, g, b]));
    }

    let mut buf = Vec::new();
    {
        let mut enc = JpegEncoder::new_with_quality(&mut buf, JPEG_QUALITY);
        enc.encode_image(&rgb).ok()?;
    }
    Some(buf)
}

/// 从 JPEG 字节里读 EXIF 方向（仅 JPEG 有；其它格式调用方已短路成 `NoTransforms`）。
///
/// `image` 0.25 的 `JpegDecoder::exif_metadata()` 是私有方法，外部拿不到；
/// 这里直接扫 APP1 段取出 TIFF/EXIF 块，再交给**公开**的 `Orientation::from_exif_chunk`。
/// 读不到 / 解析失败一律回落 `NoTransforms`，绝不让一张能看的图因为坏 EXIF 而报错。
fn jpeg_orientation(bytes: &[u8]) -> Orientation {
    let chunk = read_app1_exif(bytes).and_then(|c| Orientation::from_exif_chunk(&c));
    chunk.unwrap_or(Orientation::NoTransforms)
}

/// 扫描 JPEG 的 APP1 段，返回其后的 TIFF/EXIF 数据（即 `Exif\0\0` 之后的部分）。
fn read_app1_exif(bytes: &[u8]) -> Option<Vec<u8>> {
    // JPEG 段结构：FF D8(SOI) 后跟若干 FF<marker><len 2B><data...>
    let mut i = 2; // 跳过 SOI（FF D8）
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = bytes[i + 1];
        // SOS(DA) / EOI(D9) 之后不再是元数据段
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        if i + 4 > bytes.len() {
            break;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if len < 2 {
            break;
        }
        if marker == 0xE1 {
            // APP1：看是否以 "Exif\0\0" 开头
            let seg = i + 4;
            if bytes[seg..].starts_with(b"Exif\0\0") {
                let tiff_start = seg + 6;
                let tiff_len = len.saturating_sub(2 + 6); // 去掉 2 字节长度 + "Exif\0\0"
                if tiff_start + tiff_len <= bytes.len() {
                    return Some(bytes[tiff_start..tiff_start + tiff_len].to_vec());
                }
            }
        }
        i += 2 + len;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::DynamicImage;
    use image::RgbImage;

    fn test_store(tag: &str) -> (PathBuf, LlmImageStore) {
        let root = std::env::temp_dir().join(format!("civilcalc-img-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let paths = crate::paths::DesktopPaths::for_test(root.clone());
        (root, LlmImageStore::new(paths.images_dir()).expect("测试目录应能打开"))
    }

    /// 生成一张纯色 JPEG 写到 `path`，并返回其字节。
    fn make_jpeg(path: &Path, w: u32, h: u32) -> Vec<u8> {
        let img = RgbImage::from_pixel(w, h, image::Rgb([12u8, 34, 56]));
        let mut buf = Vec::new();
        let mut enc = JpegEncoder::new_with_quality(&mut buf, 90);
        enc.encode_image(&img).unwrap();
        std::fs::write(path, &buf).unwrap();
        buf
    }

    #[test]
    fn list_empty_for_fresh_store() {
        let (_root, store) = test_store("empty");
        assert!(store.list().is_empty(), "新仓库应是空的");
        assert_eq!(store.total_size_bytes(), 0);
    }

    #[test]
    fn process_downscales_long_edge() {
        let root = std::env::temp_dir().join(format!("civilcalc-img-scale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("big.jpg");
        make_jpeg(&path, 2000, 1000); // 长边 2000

        let bytes = process_external_image(&path).expect("应能处理");
        let out = image::load_from_memory(&bytes).expect("输出应是合法图片");
        let (w, h) = (out.width(), out.height());
        assert!(
            w.max(h) <= MAX_EDGE,
            "长边应被缩放到 ≤ {MAX_EDGE}，实际 {w}x{h}"
        );
        assert_eq!((w, h), (1024, 512), "2:1 比例应保持一致");
    }

    #[test]
    fn process_keeps_small_image() {
        let root = std::env::temp_dir().join(format!("civilcalc-img-small-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("small.jpg");
        make_jpeg(&path, 100, 80);

        let bytes = process_external_image(&path).expect("应能处理");
        let out = image::load_from_memory(&bytes).expect("输出应是合法图片");
        // 不放大：100x80 保持原样
        assert_eq!((out.width(), out.height()), (100, 80));
    }

    #[test]
    fn save_from_path_roundtrip() {
        let (root, store) = test_store("roundtrip");
        let path = root.join("pic.jpg");
        make_jpeg(&path, 800, 600);

        let bytes = process_external_image(&path).expect("处理失败");
        let ref_key = path.to_string_lossy().to_string();
        let id = store
            .save_bytes(&bytes, "image/jpeg", &ref_key)
            .expect("落盘失败");

        // list 含一条，refCount = 1
        let items = store.list();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, id);
        assert_eq!(items[0].ref_count(), 1);
        assert!(store.total_size_bytes() > 0);

        // 能按 ID 取回 data URL
        let data_url = store.load_data_url(&id).expect("应能取回");
        assert!(data_url.starts_with("data:image/jpeg;base64,"));

        // 强制删除返回 1，list 清空
        assert_eq!(store.delete(std::slice::from_ref(&id)), 1);
        assert!(store.list().is_empty());
    }

    #[test]
    fn dedup_same_content_one_file() {
        let (root, store) = test_store("dedup");
        let path_a = root.join("a.jpg");
        let path_b = root.join("b.jpg");
        let src = make_jpeg(&path_a, 300, 200);
        std::fs::write(&path_b, &src).unwrap(); // 同内容、不同路径

        let bytes = process_external_image(&path_a).unwrap();
        let id_a = store.save_bytes(&bytes, "image/jpeg", "a").unwrap();
        // 同内容再从 b 导入：应复用同 ID，refCount 变 2
        let id_b = store.save_bytes(&bytes, "image/jpeg", "b").unwrap();
        assert_eq!(id_a, id_b, "同内容应去重到同一 ID");
        assert_eq!(store.list().len(), 1, "物理文件应只有一份");
        assert_eq!(store.list()[0].ref_count(), 2);
    }

    /// 🔴 EXIF 解析：手工构造一个「Orientation = 6」的 TIFF/EXIF 块喂给 `from_exif_chunk`。
    #[test]
    fn exif_chunk_orientation_6_is_rotate90() {
        // 小端 TIFF：II + 42 + IFD0@8；IFD0 含 1 个 entry（tag 0x0112, SHORT, val=6）
        let chunk: Vec<u8> = vec![
            0x49, 0x49, // "II" 小端
            0x2A, 0x00, // 42
            0x08, 0x00, 0x00, 0x00, // IFD0 偏移 = 8
            0x01, 0x00, // entry 数 = 1
            0x12, 0x01, // tag 0x0112 (Orientation)
            0x03, 0x00, // type SHORT
            0x01, 0x00, 0x00, 0x00, // count = 1
            0x06, 0x00, 0x00, 0x00, // value = 6
            0x00, 0x00, 0x00, 0x00, // next IFD = 0
        ];
        assert_eq!(
            Orientation::from_exif_chunk(&chunk),
            Some(Orientation::Rotate90),
            "Orientation=6 应解析为 Rotate90"
        );
    }

    /// EXIF 方向套用：Rotate90 应交换宽高（100x200 → 200x100）。
    #[test]
    fn apply_orientation_rotate90_swaps_dims() {
        let mut img = DynamicImage::ImageRgb8(RgbImage::new(100, 200));
        img.apply_orientation(Orientation::Rotate90);
        assert_eq!(
            (img.width(), img.height()),
            (200, 100),
            "Rotate90 应交换宽高"
        );
    }

    #[test]
    fn apply_orientation_no_transforms_keeps_dims() {
        let mut img = DynamicImage::ImageRgb8(RgbImage::new(100, 200));
        img.apply_orientation(Orientation::NoTransforms);
        assert_eq!((img.width(), img.height()), (100, 200));
    }

    #[test]
    fn jpeg_orientation_graceful_on_bad_exif() {
        // 非 JPEG / 坏数据都不应 Panic，回落 NoTransforms
        assert_eq!(jpeg_orientation(b"not a jpeg at all"), Orientation::NoTransforms);
    }

    /// `read_app1_exif` 能从手工 JPEG 里抠出 TIFF 块并解析出 Orientation=6。
    #[test]
    fn read_app1_exif_extracts_orientation() {
        // 构造：SOI + APP1(Exif, 含上面的 TIFF) + EOI
        let tiff: Vec<u8> = vec![
            0x49, 0x49, 0x2A, 0x00, 0x08, 0x00, 0x00, 0x00, 0x01, 0x00, 0x12, 0x01, 0x03, 0x00, 0x01,
            0x00, 0x00, 0x00, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let mut jpeg: Vec<u8> = vec![0xFF, 0xD8]; // SOI
        let mut app1 = vec![0xFF, 0xE1];
        let payload: Vec<u8> = b"Exif\0\0".iter().copied().chain(tiff.iter().copied()).collect();
        let len = (payload.len() + 2) as u16;
        app1.extend_from_slice(&len.to_be_bytes());
        app1.extend_from_slice(&payload);
        jpeg.extend_from_slice(&app1);
        jpeg.extend_from_slice(&[0xFF, 0xD9]); // EOI

        assert_eq!(
            jpeg_orientation(&jpeg),
            Orientation::Rotate90,
            "应从手工 JPEG 的 APP1 段解析出 Orientation=6"
        );
    }

    /// 命令层整链路：直接构造图片文件走 `process_external_image`（与命令同函数）。
    #[test]
    fn command_save_from_path_returns_id() {
        let (root, _store) = test_store("cmd");
        let path = root.join("photo.jpg");
        make_jpeg(&path, 400, 300);
        let bytes = process_external_image(&path).expect("处理失败");
        assert!(!bytes.is_empty());
    }

    // ---------------------------------------------------------------- 剪贴板载荷解析
    //
    // `image_save_from_bytes` 的前置解析：前端可能给 data URL 也可能给裸 base64
    // （浏览器 `readAsDataURL` 给前者，有的实现只给后者），两种都必须认。

    /// 标准 base64 的 data URL → 原字节
    #[test]
    fn decode_payload_accepts_data_url() {
        // "hello" 的 base64 是 aGVsbG8=
        let out = decode_image_payload("data:image/png;base64,aGVsbG8=").expect("应能解析");
        assert_eq!(out, b"hello");
    }

    /// 裸 base64（无 `data:` 前缀）也要认
    #[test]
    fn decode_payload_accepts_bare_base64() {
        let out = decode_image_payload("aGVsbG8=").expect("应能解析");
        assert_eq!(out, b"hello");
    }

    /// 带换行/空白的 base64 —— 有的剪贴板实现会插换行，必须先剔除再解码
    #[test]
    fn decode_payload_ignores_whitespace() {
        let out = decode_image_payload("aGVs\nbG8=\n").expect("应能解析");
        assert_eq!(out, b"hello");
    }

    /// URL-safe base64（`-` `_` 代替 `+` `/`）也要认
    #[test]
    fn decode_payload_accepts_url_safe() {
        // 0xFB 0xFF 的 URL-safe base64 是 "-_8="
        let out = decode_image_payload("-_8=").expect("应能解析");
        assert_eq!(out, vec![0xFB, 0xFF]);
    }

    /// 首尾空白不该影响解析（前端可能带上换行）
    #[test]
    fn decode_payload_trims_outer_whitespace() {
        let out = decode_image_payload("  aGVsbG8=  ").expect("应能解析");
        assert_eq!(out, b"hello");
    }

    /// 非法输入返回 `None`（由调用方转成 `invalidArgument`），**不能 panic**
    #[test]
    fn decode_payload_rejects_garbage() {
        assert!(decode_image_payload("这不是 base64！！").is_none());
        assert!(decode_image_payload("").is_none());
        // 长度不是 4 的倍数且含非法字符
        assert!(decode_image_payload("a").is_none());
    }

    /// `data:` 前缀但缺逗号 → 整串都不是合法 base64 → `None`
    /// （不能把 `data:image/png;base64` 这截头部当数据解）
    #[test]
    fn decode_payload_rejects_data_url_without_comma() {
        assert!(decode_image_payload("data:image/png;base64").is_none());
    }

    // ---------------------------------------------------------------- 图片引用解析
    //
    // 🔴 这是真 bug 的回归测试：前端传的是**图片 id**，而后端曾按 data URL 处理 ——
    // `normalize_from_*` 静默丢图、`model_test_send` 把 uuid 当 URL 发出去导致 400。

    /// 传**图片 id** 必须能解析出 data URL（正常形态）
    #[test]
    fn resolve_refs_accepts_image_id() {
        let (root, store) = test_store("refs-id");
        let id = store
            .save_bytes(b"fake-jpeg-bytes", "image/jpeg", "t")
            .expect("应能保存");

        let out = resolve_image_refs_in(root.join("images"), &[id]);
        assert_eq!(out.len(), 1, "按 id 必须解析出 1 张");
        assert!(out[0].starts_with("data:image/jpeg;base64,"));
    }

    /// 传 **data URL** 也要认（兼容形态）→ 落盘后取回
    #[test]
    fn resolve_refs_accepts_data_url() {
        let (root, _store) = test_store("refs-url");
        let url = "data:image/png;base64,aGVsbG8=".to_string();

        let out = resolve_image_refs_in(root.join("images"), &[url]);
        assert_eq!(out.len(), 1, "按 data URL 必须解析出 1 张");
        assert!(out[0].starts_with("data:image/png;base64,"));
    }

    /// 认不出来的引用**跳过但不占位**（调用方依赖序号连续）
    #[test]
    fn resolve_refs_skips_unknown() {
        let (root, store) = test_store("refs-skip");
        let id = store
            .save_bytes(b"x", "image/jpeg", "t")
            .expect("应能保存");

        let refs = vec![
            "不是id也不是dataURL".to_string(),
            id,
            "另一个无效引用".to_string(),
        ];
        let out = resolve_image_refs_in(root.join("images"), &refs);
        assert_eq!(out.len(), 1, "只有有效的那一张应被保留");
    }

    /// 空列表 → 空结果（不能 panic）
    #[test]
    fn resolve_refs_empty_is_empty() {
        let (root, _store) = test_store("refs-empty");
        assert!(resolve_image_refs_in(root.join("images"), &[]).is_empty());
    }

    /// 同一张图以「id」和「data URL」两种形态混着传 → 都解析成同一份内容
    /// （内容按 SHA-1 去重，不会因为入口不同变成两张）
    #[test]
    fn resolve_refs_mixed_forms_share_content() {
        let (root, store) = test_store("refs-mixed");
        let id = store
            .save_bytes(b"same-bytes", "image/jpeg", "t")
            .expect("应能保存");
        let url = store.load_data_url(&id).expect("应能取回");

        let out = resolve_image_refs_in(root.join("images"), &[id.clone(), url]);
        assert_eq!(out.len(), 2, "两张引用都应解析出来");
        assert_eq!(out[0], out[1], "同一内容两种形态应得到同一份 data URL");
    }

    /// 🔴 `resolve_refs_in` 必须同时给出 **id 列表**（等长同序）。
    ///
    /// 命令层要靠它回填 `schema.imageIds` —— 少了它，公式就不记得自己带了图，
    /// 详解里的 `{{img:N}}` 会渲染不出来。
    #[test]
    fn resolve_refs_returns_matching_ids() {
        let (root, store) = test_store("refs-ids");
        let id = store
            .save_bytes(b"jpeg-1", "image/jpeg", "t")
            .expect("应能保存");
        let url = "data:image/png;base64,aGVsbG8=".to_string();

        let (urls, ids) = resolve_refs_in(root.join("images"), &[id.clone(), url]);
        assert_eq!(urls.len(), ids.len(), "两个列表必须等长");
        assert_eq!(ids[0], id, "已入库的引用原样作为 id 返回");
        assert!(!ids[1].is_empty(), "data URL 落盘后应拿到新 id");
        assert_ne!(ids[0], ids[1], "两张不同的图应有不同的 id");
    }

    /// 取不到的引用在**两个列表里都不出现**（序号必须保持连续对齐）
    #[test]
    fn resolve_refs_ids_skip_unknown_together() {
        let (root, store) = test_store("refs-ids-skip");
        let id = store
            .save_bytes(b"jpeg-2", "image/jpeg", "t")
            .expect("应能保存");

        let refs = vec!["无效".to_string(), id.clone(), "也无效".to_string()];
        let (urls, ids) = resolve_refs_in(root.join("images"), &refs);
        assert_eq!(urls.len(), 1);
        assert_eq!(ids, vec![id]);
    }
}
