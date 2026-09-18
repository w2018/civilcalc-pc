//! AI 输入图片存储（data URL 落盘 + 内容哈希去重 + 引用计数）。
//!
//! 源：`civilcalc-android-v2/data/image/LlmImageStore.kt`（252 行）
//!
//! ## 生命周期模型：**图片独立留存**
//!
//! | 操作 | 行为 |
//! |---|---|
//! | [`LlmImageStore::save`] | 存图 + 登记一个引用键；**内容相同（SHA-1 一致）复用同一文件与 ID**，只追加引用键 |
//! | [`LlmImageStore::release_ref`] | 只摘引用键，**不删文件** —— 删掉一条输入历史不会连带删图 |
//! | [`LlmImageStore::delete`] | **唯一的物理删除入口**（图片管理里批量删除时调用） |
//! | [`LlmImageStore::load_data_url`] / `file_for` | 按 ID 取回内容（展示用，不影响引用） |
//!
//! ## 崩溃一致性
//!
//! **先写文件，后写注册表**。若在两者之间崩溃，会留下「有文件无记录」的孤儿文件 ——
//! 由 [`LlmImageStore::gc`] 兜底清理（`save` 时顺带执行一次）。
//! 反过来（先写注册表）会留下「有记录无文件」的**坏记录**，展示层会看到永远打不开的图。
//!
//! ## ⚠️ 注册表写失败**故意不抛**
//!
//! 与源一致：下次 `save` / `release_ref` 会再写一遍，孤儿文件由 `gc` 兜底。
//! 让「写注册表失败」中断存图流程，会让用户丢图却不知道为什么。
//!
//! ## 同步 API（不是 async）
//!
//! 源是 `suspend`（Android 主线程安全考量）。PC 侧这里是纯文件 IO，
//! 做成同步函数、由命令层决定是否丢进 `spawn_blocking`。
//! 内部用 `Mutex` 串行化，避免并发 `save` 撞车产生重复文件。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::sha1::sha1_hex;

/// 注册表文件名（与源一致）。
pub const REGISTRY_NAME: &str = "registry.json";

/// 图片引用记录。
///
/// `refs` 是引用键列表（如 `"hist:search:<entryId>"` / `"formula:<formulaId>"`），
/// **仅作引用标记与展示，不决定图片生命周期**。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageRecord {
    pub id: String,
    pub file_name: String,
    pub mime_type: String,
    #[serde(default)]
    pub refs: Vec<String>,
    #[serde(default)]
    pub created_at: i64,
}

/// 供图片管理展示的一条记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageInfo {
    pub id: String,
    pub path: String,
    pub size_bytes: i64,
    pub created_at: i64,
    pub refs: Vec<String>,
}

impl ImageInfo {
    /// 引用处数。
    #[must_use]
    pub fn ref_count(&self) -> usize {
        self.refs.len()
    }
}

/// 图片存储。
#[derive(Debug)]
pub struct LlmImageStore {
    dir: PathBuf,
    /// 串行化所有读写（源的 `Mutex().withLock`）。
    lock: Mutex<()>,
}

impl LlmImageStore {
    /// 在 `dir` 上打开（不存在则创建）。
    ///
    /// PC 侧传 `DesktopPaths::images_dir()`（`app_data_dir/images`）——
    /// 源用的是 Android 的 `filesDir/llm_images`，目录名不同但语义相同。
    pub fn new(dir: impl Into<PathBuf>) -> std::io::Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            lock: Mutex::new(()),
        })
    }

    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn registry_file(&self) -> PathBuf {
        self.dir.join(REGISTRY_NAME)
    }

    /// 保存 data URL 并登记引用。
    ///
    /// 成功返回图片 ID；data URL 非法返回 `None`。
    /// **内容相同（SHA-1 一致）复用已有 ID**。
    /// 存一张图片并登记一个引用键，返回图片 ID（相同内容 SHA-1 一致复用已有 ID）。
    ///
    /// 与 [`Self::save`] 不同：这里直接接收**已编码的字节 + mime**，
    /// 命令层先把外部图片转码成 JPEG 再调本函数，省去 data URL 的 base64 往返。
    #[must_use]
    pub fn save_bytes(&self, bytes: &[u8], mime: &str, ref_key: &str) -> Option<String> {
        let _guard = self.lock.lock().ok()?;
        let hash = sha1_hex(bytes);
        let mut registry = self.load_registry();

        // 内容去重：只按哈希比对（不掺扩展名），同一张图即使 mime 声明不同也只存一份
        let existing = registry
            .values()
            .find(|r| r.file_name.split('.').next() == Some(hash.as_str()))
            .cloned();
        if let Some(mut e) = existing {
            let existing_id = e.id.clone();
            if !e.refs.iter().any(|r| r == ref_key) {
                e.refs.push(ref_key.to_string());
                registry.insert(existing_id.clone(), e);
                self.save_registry(&registry);
            }
            return Some(existing_id);
        }

        let id = uuid::Uuid::new_v4().to_string();
        let file_name = format!("{hash}.{}", extension_for(mime));
        std::fs::write(self.dir.join(&file_name), bytes).ok()?;
        registry.insert(
            id.clone(),
            ImageRecord {
                id: id.clone(),
                file_name,
                mime_type: mime.to_string(),
                refs: vec![ref_key.to_string()],
                created_at: now_millis(),
            },
        );
        self.save_registry(&registry);
        self.gc_locked(&registry);
        Some(id)
    }

    /// 保存 data URL 并登记引用；成功返回图片 ID，data URL 非法返回 `None`。
    /// 内容相同（SHA-1 一致）复用已有 ID。
    pub fn save(&self, data_url: &str, ref_key: &str) -> Option<String> {
        let (mime, bytes) = parse_data_url(data_url)?;
        self.save_bytes(&bytes, &mime, ref_key)
    }

    /// 追加一个引用键（同一图片被其它功能/记录复用时调用）。
    pub fn add_ref(&self, id: &str, ref_key: &str) {
        let Ok(_guard) = self.lock.lock() else { return };
        let mut registry = self.load_registry();
        let Some(record) = registry.get(id).cloned() else {
            return;
        };
        if !record.refs.iter().any(|r| r == ref_key) {
            let mut updated = record;
            updated.refs.push(ref_key.to_string());
            registry.insert(id.to_string(), updated);
            self.save_registry(&registry);
        }
    }

    /// 释放引用键：只摘引用标记，**不删文件**（图片独立留存，删除只走 [`Self::delete`]）。
    pub fn release_ref(&self, id: &str, ref_key: &str) {
        let Ok(_guard) = self.lock.lock() else { return };
        let mut registry = self.load_registry();
        let Some(record) = registry.get(id).cloned() else {
            return;
        };
        let before = record.refs.len();
        let mut updated = record;
        updated.refs.retain(|r| r != ref_key);
        if updated.refs.len() != before {
            registry.insert(id.to_string(), updated);
            self.save_registry(&registry);
        }
    }

    /// 按图片 ID 取回 data URL；记录或文件缺失（已被图片管理删除等）返回 `None`。
    pub fn load_data_url(&self, id: &str) -> Option<String> {
        let _guard = self.lock.lock().ok()?;
        let record = self.load_registry().get(id).cloned()?;
        let bytes = std::fs::read(self.dir.join(&record.file_name)).ok()?;
        Some(format!(
            "data:{};base64,{}",
            record.mime_type,
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ))
    }

    /// 按图片 ID 取本地文件（九宫格缩略图交给图片库降采样加载，
    /// 避免整批解 base64 占内存）。
    pub fn file_for(&self, id: &str) -> Option<PathBuf> {
        let _guard = self.lock.lock().ok()?;
        let record = self.load_registry().get(id).cloned()?;
        let path = self.dir.join(&record.file_name);
        path.exists().then_some(path)
    }

    /// 全部图片记录（**新 → 旧**），供图片管理按时间分组展示。
    #[must_use]
    pub fn list(&self) -> Vec<ImageInfo> {
        let Ok(_guard) = self.lock.lock() else {
            return Vec::new();
        };
        let mut items: Vec<ImageInfo> = self
            .load_registry()
            .into_values()
            .map(|record| {
                let path = self.dir.join(&record.file_name);
                let size_bytes = std::fs::metadata(&path).map(|m| m.len() as i64).unwrap_or(0);
                ImageInfo {
                    id: record.id,
                    path: path.to_string_lossy().to_string(),
                    size_bytes,
                    created_at: record.created_at,
                    refs: record.refs,
                }
            })
            .collect();
        // 源按 createdAt 降序；同一毫秒的并列用 id 兜底，保证**稳定排序**
        items.sort_by(|a, b| b.created_at.cmp(&a.created_at).then_with(|| a.id.cmp(&b.id)));
        items
    }

    /// 按记录里的文件名同步读字节。
    ///
    /// 备份打包是**流式写 tar**（阻塞 IO），那里调不了挂起函数 —— 故提供同步版本。
    /// 只接受注册表里的**裸文件名**，含路径分隔符或 `..` 一律拒绝（防越权读任意文件）。
    pub fn read_bytes_by_name(&self, file_name: &str) -> Option<Vec<u8>> {
        if file_name.is_empty()
            || file_name.contains('/')
            || file_name.contains('\\')
            || file_name.contains("..")
        {
            return None;
        }
        std::fs::read(self.dir.join(file_name)).ok()
    }

    /// 从云端备份恢复一张图片：**按原 id 与原文件名落盘**并合并注册表。
    ///
    /// ⚠️ **不能用 [`Self::save`] 代替**：`save` 会重新生成 UUID，而
    /// `FormulaSchema.imageIds` 与输入历史里存的是**旧 id** ——
    /// 换了 id 会让所有图片引用失效（公式里的内联图、历史缩略图全变「已删除」）。
    ///
    /// `refs` 取**并集**，恢复不会丢掉本机已有的引用关系。
    pub fn restore(
        &self,
        id: &str,
        file_name: &str,
        mime_type: &str,
        refs: &[String],
        created_at: i64,
        bytes: &[u8],
    ) {
        let Ok(_guard) = self.lock.lock() else { return };
        let mut registry = self.load_registry();
        let path = self.dir.join(file_name);
        // 已存在且大小一致 → 不重写（避免无谓 IO；内容相同是哈希去重的既定前提）
        let same_size = std::fs::metadata(&path)
            .map(|m| m.len() == bytes.len() as u64)
            .unwrap_or(false);
        if !same_size && std::fs::write(&path, bytes).is_err() {
            return;
        }
        let existing = registry.get(id).cloned();
        let mut merged: Vec<String> = existing
            .as_ref()
            .map(|e| e.refs.clone())
            .unwrap_or_default();
        for r in refs {
            if !merged.iter().any(|m| m == r) {
                merged.push(r.clone());
            }
        }
        let kept_created = existing
            .as_ref()
            .map(|e| e.created_at)
            .filter(|c| *c > 0)
            .unwrap_or(created_at);
        registry.insert(
            id.to_string(),
            ImageRecord {
                id: id.to_string(),
                file_name: file_name.to_string(),
                mime_type: mime_type.to_string(),
                refs: merged,
                created_at: kept_created,
            },
        );
        self.save_registry(&registry);
    }

    /// 图片缓存总占用（字节）。
    #[must_use]
    pub fn total_size_bytes(&self) -> i64 {
        let Ok(_guard) = self.lock.lock() else {
            return 0;
        };
        self.load_registry()
            .into_values()
            .map(|r| {
                std::fs::metadata(self.dir.join(&r.file_name))
                    .map(|m| m.len() as i64)
                    .unwrap_or(0)
            })
            .sum()
    }

    /// 强制删除指定图片（文件 + 记录），返回**实际删除数量**。
    ///
    /// 被引用的图**也照删** —— 调用方负责提示影响。
    #[must_use]
    pub fn delete(&self, ids: &[String]) -> usize {
        let Ok(_guard) = self.lock.lock() else {
            return 0;
        };
        if ids.is_empty() {
            return 0;
        }
        let mut registry = self.load_registry();
        let mut deleted = 0usize;
        // 去重（源用 `ids.toSet()`）
        let mut seen: Vec<&String> = Vec::new();
        for id in ids {
            if seen.contains(&id) {
                continue;
            }
            seen.push(id);
            if let Some(record) = registry.remove(id) {
                let _ = std::fs::remove_file(self.dir.join(&record.file_name));
                deleted += 1;
            }
        }
        self.save_registry(&registry);
        deleted
    }

    /// 清理孤儿文件（**有文件无记录**；先写文件后写注册表之间的崩溃残留）。
    /// 有记录的图片不会被清理。
    pub fn gc(&self) {
        let Ok(_guard) = self.lock.lock() else { return };
        let registry = self.load_registry();
        self.gc_locked(&registry);
    }

    // -------------------------------------------------------------------------
    // 内部
    // -------------------------------------------------------------------------

    fn gc_locked(&self, registry: &HashMap<String, ImageRecord>) {
        let known: std::collections::HashSet<&str> =
            registry.values().map(|r| r.file_name.as_str()).collect();
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name != REGISTRY_NAME && !known.contains(name.as_ref()) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    /// 读注册表；文件不存在或解析失败一律返回空表（**不拒绝启动**）。
    fn load_registry(&self) -> HashMap<String, ImageRecord> {
        let path = self.registry_file();
        let Ok(text) = std::fs::read_to_string(&path) else {
            return HashMap::new();
        };
        // 存储形态是**数组**（源 `registry.values.toList()`），不是对象
        match serde_json::from_str::<Vec<ImageRecord>>(&text) {
            Ok(list) => list.into_iter().map(|r| (r.id.clone(), r)).collect(),
            Err(_) => HashMap::new(),
        }
    }

    /// 写注册表；**失败不抛**（见模块文档）。
    fn save_registry(&self, registry: &HashMap<String, ImageRecord>) {
        let list: Vec<&ImageRecord> = registry.values().collect();
        if let Ok(text) = serde_json::to_string(&list) {
            let _ = std::fs::write(self.registry_file(), text);
        }
    }
}

/// 当前毫秒时间戳（源的 `System.currentTimeMillis()`）。
fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 解析 data URL → `(mime, bytes)`；非法返回 `None`。
///
/// 与源等价：必须以 `data:` 开头、必须含 `;base64,`；
/// mime 取**第一个** `;base64,` 之前的部分，载荷取其之后的部分。
pub fn parse_data_url(data_url: &str) -> Option<(String, Vec<u8>)> {
    const MARKER: &str = ";base64,";
    let rest = data_url.strip_prefix("data:")?;
    let idx = rest.find(MARKER)?;
    let mime = rest[..idx].to_string();
    let payload = &rest[idx + MARKER.len()..];
    let bytes = decode_base64_lenient(payload)?;
    if bytes.is_empty() {
        return None;
    }
    Some((mime, bytes))
}

/// 宽松 base64 解码：先剔除空白，再依次尝试带/不带填充。
///
/// 源用 Android 的 `Base64.DEFAULT`，它**忽略换行**且容忍缺填充；
/// `base64` crate 的 `STANDARD` 两者都不容忍 —— 不补这两步会让
/// 「浏览器/Flutter 传来的带换行 data URL」解码失败。
pub fn decode_base64_lenient(s: &str) -> Option<Vec<u8>> {
    let cleaned: String = s.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let engine = base64::engine::general_purpose::STANDARD;
    engine
        .decode(&cleaned)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(&cleaned))
        .ok()
}

/// mime → 扩展名（与源一致：png/webp/gif，其余一律 jpg）。
#[must_use]
pub fn extension_for(mime: &str) -> &'static str {
    match mime.to_lowercase().as_str() {
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "jpg",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};

    fn tmp_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("cc-img-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn store() -> LlmImageStore {
        LlmImageStore::new(tmp_dir()).unwrap()
    }

    /// 1×1 PNG（最小合法 PNG 字节）
    const PNG_1PX: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    fn png_url() -> String {
        format!("data:image/png;base64,{}", STANDARD.encode(PNG_1PX))
    }

    // ---------------------------------------------------------------------
    // parse_data_url / extension_for
    // ---------------------------------------------------------------------

    #[test]
    fn parse_valid_data_url() {
        let (mime, bytes) = parse_data_url(&png_url()).unwrap();
        assert_eq!(mime, "image/png");
        assert_eq!(bytes, PNG_1PX);
    }

    #[test]
    fn parse_rejects_malformed() {
        assert!(parse_data_url("").is_none());
        assert!(parse_data_url("not a url").is_none());
        assert!(parse_data_url("data:image/png,AAAA").is_none(), "缺 ;base64,");
        assert!(parse_data_url("image/png;base64,AAAA").is_none(), "缺 data: 前缀");
        assert!(parse_data_url("data:image/png;base64,!!!!").is_none(), "非法 base64");
        assert!(parse_data_url("data:image/png;base64,").is_none(), "空载荷");
    }

    /// 换行/空格被容忍（对齐 Android `Base64.DEFAULT`）
    #[test]
    fn parse_tolerates_whitespace() {
        let b64 = STANDARD.encode(PNG_1PX);
        let with_newlines = format!("data:image/png;base64,{}\n", b64);
        assert_eq!(parse_data_url(&with_newlines).unwrap().1, PNG_1PX);
        let wrapped = format!("data:image/png;base64,{}\n", b64);
        assert!(parse_data_url(&wrapped).is_some());
    }

    /// 缺填充也能解（`STANDARD_NO_PAD` 兜底）
    #[test]
    fn parse_tolerates_missing_padding() {
        let unpadded = STANDARD_NO_PAD.encode(PNG_1PX);
        let url = format!("data:image/png;base64,{unpadded}");
        assert_eq!(parse_data_url(&url).unwrap().1, PNG_1PX);
    }

    #[test]
    fn extension_mapping() {
        assert_eq!(extension_for("image/png"), "png");
        assert_eq!(extension_for("image/PNG"), "png", "大小写不敏感");
        assert_eq!(extension_for("image/webp"), "webp");
        assert_eq!(extension_for("image/gif"), "gif");
        assert_eq!(extension_for("image/jpeg"), "jpg");
        assert_eq!(extension_for("image/bmp"), "jpg", "其余一律 jpg");
        assert_eq!(extension_for(""), "jpg");
    }

    // ---------------------------------------------------------------------
    // save / load / 去重
    // ---------------------------------------------------------------------

    #[test]
    fn save_then_load_roundtrip() {
        let s = store();
        let id = s.save(&png_url(), "formula:1").expect("保存应成功");
        assert!(!id.is_empty());
        assert_eq!(s.load_data_url(&id).unwrap(), png_url());
    }

    /// 🔴 内容去重：同一张图存两次 → 同一 id、只有一个文件
    #[test]
    fn same_content_dedups() {
        let s = store();
        let a = s.save(&png_url(), "formula:1").unwrap();
        let b = s.save(&png_url(), "formula:2").unwrap();
        assert_eq!(a, b, "内容相同必须复用同一 ID");
        let files: Vec<_> = std::fs::read_dir(s.dir())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name() != REGISTRY_NAME)
            .collect();
        assert_eq!(files.len(), 1, "同一内容只应有一个文件");
    }

    /// 去重时追加引用键
    #[test]
    fn dedup_appends_ref() {
        let s = store();
        let id = s.save(&png_url(), "formula:1").unwrap();
        s.save(&png_url(), "formula:2");
        let info = s.list().into_iter().find(|i| i.id == id).unwrap();
        assert_eq!(info.refs, vec!["formula:1", "formula:2"]);
        assert_eq!(info.ref_count(), 2);
    }

    /// 同一引用键重复登记不产生重复项
    #[test]
    fn duplicate_ref_key_not_repeated() {
        let s = store();
        let id = s.save(&png_url(), "formula:1").unwrap();
        s.save(&png_url(), "formula:1");
        let info = s.list().into_iter().find(|i| i.id == id).unwrap();
        assert_eq!(info.ref_count(), 1);
    }

    /// 不同内容 → 不同 id / 不同文件
    #[test]
    fn different_content_creates_new_id() {
        let s = store();
        let a = s.save(&png_url(), "k").unwrap();
        let b = s.save(&format!("data:image/png;base64,{}", STANDARD.encode(b"other")), "k")
            .unwrap();
        assert_ne!(a, b);
        assert_eq!(s.list().len(), 2);
    }

    #[test]
    fn save_invalid_url_returns_none() {
        let s = store();
        assert!(s.save("garbage", "k").is_none());
        assert!(s.list().is_empty());
    }

    /// 文件名 = `<sha1>.<ext>`
    #[test]
    fn file_name_is_hash_based() {
        let s = store();
        let id = s.save(&png_url(), "k").unwrap();
        let path = s.file_for(&id).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(name, format!("{}.png", sha1_hex(PNG_1PX)));
    }

    #[test]
    fn load_unknown_id_returns_none() {
        let s = store();
        assert!(s.load_data_url("nope").is_none());
        assert!(s.file_for("nope").is_none());
    }

    /// 记录在但文件被外部删掉 → 返回 None（不 panic）
    #[test]
    fn load_when_file_deleted_returns_none() {
        let s = store();
        let id = s.save(&png_url(), "k").unwrap();
        let path = s.file_for(&id).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(s.load_data_url(&id).is_none());
        assert!(s.file_for(&id).is_none());
    }

    // ---------------------------------------------------------------------
    // 引用：release 不删文件
    // ---------------------------------------------------------------------

    /// 🔴 `release_ref` 只摘引用键，**文件必须还在**
    #[test]
    fn release_ref_keeps_file() {
        let s = store();
        let id = s.save(&png_url(), "hist:1").unwrap();
        s.release_ref(&id, "hist:1");
        let info = s.list().into_iter().find(|i| i.id == id).unwrap();
        assert!(info.refs.is_empty(), "引用键已摘掉");
        assert!(s.file_for(&id).is_some(), "文件不得被删");
        assert_eq!(s.load_data_url(&id).unwrap(), png_url());
    }

    #[test]
    fn add_ref_works() {
        let s = store();
        let id = s.save(&png_url(), "a").unwrap();
        s.add_ref(&id, "b");
        s.add_ref(&id, "b");
        let info = s.list().into_iter().find(|i| i.id == id).unwrap();
        assert_eq!(info.refs, vec!["a", "b"]);
    }

    #[test]
    fn ref_ops_on_unknown_id_are_noop() {
        let s = store();
        s.add_ref("nope", "x");
        s.release_ref("nope", "x");
        assert!(s.list().is_empty());
    }

    // ---------------------------------------------------------------------
    // list / size / delete / gc
    // ---------------------------------------------------------------------

    #[test]
    fn list_sorted_newest_first() {
        let s = store();
        let a = s.save(&format!("data:image/png;base64,{}", STANDARD.encode(b"aa")), "k").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let b = s.save(&format!("data:image/png;base64,{}", STANDARD.encode(b"bb")), "k").unwrap();
        let list = s.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, b, "新的在前");
        assert_eq!(list[1].id, a);
    }

    #[test]
    fn total_size_sums_files() {
        let s = store();
        s.save(&png_url(), "k");
        let n = s.total_size_bytes();
        assert_eq!(n, PNG_1PX.len() as i64);
    }

    #[test]
    fn delete_removes_file_and_record() {
        let s = store();
        let id = s.save(&png_url(), "k").unwrap();
        let path = s.file_for(&id).unwrap();
        assert_eq!(s.delete(std::slice::from_ref(&id)), 1);
        assert!(!path.exists(), "文件应被删除");
        assert!(s.list().is_empty());
        assert!(s.load_data_url(&id).is_none());
    }

    #[test]
    fn delete_is_idempotent_and_dedups_input() {
        let s = store();
        let id = s.save(&png_url(), "k").unwrap();
        // 同一 id 传两次只算一次
        assert_eq!(s.delete(&[id.clone(), id.clone()]), 1);
        // 再删一次 → 0（幂等）
        assert_eq!(s.delete(&[id]), 0);
        assert_eq!(s.delete(&[]), 0);
    }

    #[test]
    fn delete_counts_only_existing() {
        let s = store();
        let a = s.save(&png_url(), "k").unwrap();
        assert_eq!(s.delete(&[a, "nope".to_string()]), 1);
    }

    /// 🔴 `gc` 删掉孤儿文件，但**保留**有记录的
    #[test]
    fn gc_removes_orphans_keeps_records() {
        let s = store();
        let id = s.save(&png_url(), "k").unwrap();
        let kept = s.file_for(&id).unwrap();
        let orphan = s.dir().join("deadbeef.jpg");
        std::fs::write(&orphan, b"orphan").unwrap();

        s.gc();

        assert!(kept.exists(), "有记录的文件不得被 gc");
        assert!(!orphan.exists(), "孤儿文件应被清理");
        assert_eq!(s.load_data_url(&id).unwrap(), png_url());
    }

    /// `gc` 不动注册表文件本身
    #[test]
    fn gc_keeps_registry() {
        let s = store();
        s.save(&png_url(), "k");
        s.gc();
        assert!(s.dir().join(REGISTRY_NAME).exists());
        assert_eq!(s.list().len(), 1);
    }

    // ---------------------------------------------------------------------
    // restore（备份恢复）
    // ---------------------------------------------------------------------

    /// 🔴 恢复必须**保留原 id 与原文件名**（换 id 会让所有引用失效）
    #[test]
    fn restore_keeps_original_identity() {
        let s = store();
        let refs = vec!["formula:old".to_string()];
        s.restore("orig-id", "abc123.png", "image/png", &refs, 1000, PNG_1PX);

        let info = s.list().into_iter().find(|i| i.id == "orig-id").expect("必须按原 id 存");
        assert_eq!(info.refs, refs);
        assert_eq!(info.created_at, 1000);
        assert!(s.dir().join("abc123.png").exists(), "文件名不得被改写");
        assert_eq!(s.load_data_url("orig-id").unwrap(), png_url());
    }

    /// refs 取并集，不丢本机已有引用
    #[test]
    fn restore_merges_refs() {
        let s = store();
        s.restore("id1", "h.png", "image/png", &["local".to_string()], 1, PNG_1PX);
        s.restore(
            "id1",
            "h.png",
            "image/png",
            &["local".to_string(), "cloud".to_string()],
            2,
            PNG_1PX,
        );
        let info = s.list().into_iter().find(|i| i.id == "id1").unwrap();
        assert_eq!(info.refs, vec!["local", "cloud"], "并集且不重复");
        assert_eq!(info.created_at, 1, "已有 createdAt > 0 时保留本机值");
    }

    /// 已有记录 createdAt 为 0 时用传入值
    #[test]
    fn restore_uses_incoming_created_at_when_local_zero() {
        let s = store();
        s.restore("id1", "h.png", "image/png", &[], 0, PNG_1PX);
        s.restore("id1", "h.png", "image/png", &[], 999, PNG_1PX);
        let info = s.list().into_iter().find(|i| i.id == "id1").unwrap();
        assert_eq!(info.created_at, 999);
    }

    // ---------------------------------------------------------------------
    // read_bytes_by_name（备份打包用）
    // ---------------------------------------------------------------------

    #[test]
    fn read_bytes_by_name_works() {
        let s = store();
        let id = s.save(&png_url(), "k").unwrap();
        let name = s.file_for(&id).unwrap().file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(s.read_bytes_by_name(&name).unwrap(), PNG_1PX);
    }

    /// 🔴 路径穿越一律拒绝
    #[test]
    fn read_bytes_by_name_rejects_traversal() {
        let s = store();
        for bad in ["", "../secret", "a/b.png", "a\\b.png", "..", "x..y"] {
            assert!(s.read_bytes_by_name(bad).is_none(), "应拒绝 {bad:?}");
        }
    }

    // ---------------------------------------------------------------------
    // 注册表形态 / 容错
    // ---------------------------------------------------------------------

    /// 注册表是**数组**（不是对象）—— 备份包与 Android 端都按此形态
    #[test]
    fn registry_is_json_array_with_camel_case() {
        let s = store();
        s.save(&png_url(), "k");
        let text = std::fs::read_to_string(s.dir().join(REGISTRY_NAME)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(v.is_array(), "必须是数组");
        assert!(v[0].get("fileName").is_some(), "字段名必须是 camelCase");
        assert!(v[0].get("file_name").is_none());
        assert!(v[0].get("mimeType").is_some());
        assert!(v[0].get("createdAt").is_some());
    }

    /// 注册表损坏 → 退化成空表，不 panic（不拒绝启动）
    #[test]
    fn corrupt_registry_degrades_to_empty() {
        let dir = tmp_dir();
        std::fs::write(dir.join(REGISTRY_NAME), "{ not json").unwrap();
        let s = LlmImageStore::new(&dir).unwrap();
        assert!(s.list().is_empty());
        // 仍能正常存图（会覆盖掉坏注册表）
        let id = s.save(&png_url(), "k").unwrap();
        assert_eq!(s.list().len(), 1);
        assert!(s.load_data_url(&id).is_some());
    }

    /// 目录不存在时自动创建
    #[test]
    fn creates_dir_if_missing() {
        let dir = std::env::temp_dir().join(format!("cc-img-new-{}", uuid::Uuid::new_v4()));
        assert!(!dir.exists());
        let s = LlmImageStore::new(&dir).unwrap();
        assert!(dir.exists());
        assert!(s.list().is_empty());
    }
}
