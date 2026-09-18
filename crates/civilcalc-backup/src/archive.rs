//! 备份包内容编解码：包内条目布局。
//!
//! 源：`civilcalc-android-v2/core/backup/BackupArchive.kt`（134 行）
//!
//! ```text
//! manifest.json      清单（格式版本 / App 版本 / 时间 / 各类条数）
//! tables.json        user_formulas / history / favorites / formula_versions / llm_usage_stats 的行数据
//! prefs.json         偏好
//! llm_config.json    真实生效的 AI 模型配置（加密存储里那份的原始 JSON）
//! secrets.json       模型密钥（仅在勾选「含 API Key」时存在）
//! images.json        图片登记信息（id / 文件名 / 引用键）
//! images/<fileName>  图片原始字节（逐条流式读写）
//! background.jpg     自定义背景图（可选）
//! ```
//!
//! ## 为什么拆成多个条目而不是一个大 JSON
//!
//! 1. 图片保持**原始字节**，不经过 base64（省 33% 体积）
//! 2. 导入时可以先只读 `manifest` / `prefs` / `tables` 做**清单确认**，再按需读图片
//!
//! ## 🔴 写入是**懒式**的
//!
//! `image_bytes` 回调按需调用，**一次只让一张图进内存**。
//! 若先收集成 `Vec<TarEntry>` 再写，几百 MB 图片会直接把内存打满。
//!
//! ## 🔴 读到的 JSON 解不开时**退化成默认值**，不报错
//!
//! 旧备份可能缺字段、多字段、甚至某个条目是坏的。只要 `manifest` / `tables` / `prefs`
//! 能读出一个「合理的默认」，用户至少还能把其它数据导进来 —— 比整包拒绝有用。

use std::io::{Read, Write};

use crate::crypto::{self, WriteLayer};
use crate::error::BackupError;
use crate::model::{
    BackupContent, BackupImageRecord, BackupManifest, BackupPrefs, BackupSecrets, BackupTables,
};
use crate::tar_gz::{self, TarEntry};

/// 清单条目名。
pub const ENTRY_MANIFEST: &str = "manifest.json";
/// 各表行数据。
pub const ENTRY_TABLES: &str = "tables.json";
/// 偏好。
pub const ENTRY_PREFS: &str = "prefs.json";
/// 模型配置（原始 JSON）。
pub const ENTRY_LLM_CONFIG: &str = "llm_config.json";
/// 模型密钥。
pub const ENTRY_SECRETS: &str = "secrets.json";
/// 图片登记信息。
pub const ENTRY_IMAGES: &str = "images.json";
/// 自定义背景图。
pub const ENTRY_BACKGROUND: &str = "background.jpg";
/// 图片字节条目的前缀。
pub const IMAGE_DIR: &str = "images/";

/// 包内 8 个条目名（`images/<hash>` 是第 8 类，前缀见 [`IMAGE_DIR`]）。
pub const ALL_ENTRY_NAMES: [&str; 7] = [
    ENTRY_MANIFEST,
    ENTRY_TABLES,
    ENTRY_PREFS,
    ENTRY_LLM_CONFIG,
    ENTRY_SECRETS,
    ENTRY_IMAGES,
    ENTRY_BACKGROUND,
];

// =============================================================================
// 写
// =============================================================================

/// 打包。
///
/// - `image_bytes`：**按需**读取每张图片的字节（返回 `None` 表示文件已不在，跳过该条目）
/// - `background_bytes`：自定义背景图字节（无则 `None`）
/// - `password`：非空则整包加密（先 gzip 后加密）
pub fn write_to<W, F>(
    out: W,
    content: &BackupContent,
    image_bytes: F,
    background_bytes: Option<Vec<u8>>,
    password: Option<&str>,
) -> Result<(), BackupError>
where
    W: Write,
    F: FnMut(&BackupImageRecord) -> Option<Vec<u8>>,
{
    let entries = EntryStream::new(content, image_bytes, background_bytes);
    let layer: WriteLayer<W> = crypto::writer_for(out, password)?;
    let layer = tar_gz::write_to(layer, entries).map_err(io_err)?;
    // 加密层写收尾块（明文层只是 flush）
    layer.finish()
}

/// 懒式条目迭代器：不预先收集，图片逐张产出。
struct EntryStream<'a, F> {
    content: &'a BackupContent,
    image_bytes: F,
    background: Option<Vec<u8>>,
    step: Step,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Manifest,
    Tables,
    Prefs,
    LlmConfig,
    Secrets,
    ImagesHeader,
    Image(usize),
    Background,
    Done,
}

impl<'a, F> EntryStream<'a, F> {
    fn new(content: &'a BackupContent, image_bytes: F, background: Option<Vec<u8>>) -> Self {
        Self {
            content,
            image_bytes,
            background,
            step: Step::Manifest,
        }
    }

    fn json_entry<T: serde::Serialize>(name: &str, value: &T) -> Option<TarEntry> {
        // `encodeDefaults` 语义：serde 默认就会写全字段（不加 skip_serializing_if）
        serde_json::to_vec(value)
            .ok()
            .map(|bytes| TarEntry::new(name, bytes))
    }
}

impl<F: FnMut(&BackupImageRecord) -> Option<Vec<u8>>> Iterator for EntryStream<'_, F> {
    type Item = TarEntry;

    fn next(&mut self) -> Option<TarEntry> {
        loop {
            match self.step {
                Step::Manifest => {
                    self.step = Step::Tables;
                    if let Some(e) = Self::json_entry(ENTRY_MANIFEST, &self.content.manifest) {
                        return Some(e);
                    }
                }
                Step::Tables => {
                    self.step = Step::Prefs;
                    if let Some(e) = Self::json_entry(ENTRY_TABLES, &self.content.tables) {
                        return Some(e);
                    }
                }
                Step::Prefs => {
                    self.step = Step::LlmConfig;
                    if let Some(e) = Self::json_entry(ENTRY_PREFS, &self.content.prefs) {
                        return Some(e);
                    }
                }
                Step::LlmConfig => {
                    self.step = Step::Secrets;
                    // 空串视为「没有」—— 与源 `takeIf { it.isNotBlank() }` 一致
                    if let Some(raw) = self
                        .content
                        .llm_config_json
                        .as_deref()
                        .filter(|s| !s.trim().is_empty())
                    {
                        return Some(TarEntry::new(ENTRY_LLM_CONFIG, raw.as_bytes().to_vec()));
                    }
                }
                Step::Secrets => {
                    self.step = Step::ImagesHeader;
                    if let Some(secrets) = self.content.secrets.as_ref() {
                        if let Some(e) = Self::json_entry(ENTRY_SECRETS, secrets) {
                            return Some(e);
                        }
                    }
                }
                Step::ImagesHeader => {
                    self.step = if self.content.images.is_empty() {
                        Step::Background
                    } else {
                        Step::Image(0)
                    };
                    if !self.content.images.is_empty() {
                        if let Some(e) = Self::json_entry(ENTRY_IMAGES, &self.content.images) {
                            return Some(e);
                        }
                    }
                }
                Step::Image(idx) => {
                    let Some(record) = self.content.images.get(idx) else {
                        self.step = Step::Background;
                        continue;
                    };
                    self.step = Step::Image(idx + 1);
                    // 文件已不在 → 跳过该条目（`continue` 而非产出空条目）
                    let Some(bytes) = (self.image_bytes)(record) else {
                        continue;
                    };
                    return Some(TarEntry::new(
                        format!("{IMAGE_DIR}{}", record.file_name),
                        bytes,
                    ));
                }
                Step::Background => {
                    self.step = Step::Done;
                    if let Some(bg) = self.background.take() {
                        return Some(TarEntry::new(ENTRY_BACKGROUND, bg));
                    }
                }
                Step::Done => return None,
            }
        }
    }
}

// =============================================================================
// 读
// =============================================================================

/// 只读清单与结构化数据（**图片字节不读**）；缺条目按默认值处理，旧备份也能读。
///
/// 加密包没给密码 → [`BackupError::NeedPassword`]；密码不对 → [`BackupError::WrongPassword`]。
pub fn read_meta<R: Read>(input: R, password: Option<&str>) -> Result<BackupContent, BackupError> {
    let layer = crypto::reader_for(input, password)?;
    let mut manifest: Option<BackupManifest> = None;
    let mut tables: Option<BackupTables> = None;
    let mut prefs: Option<BackupPrefs> = None;
    let mut llm_config_json: Option<String> = None;
    let mut secrets: Option<BackupSecrets> = None;
    let mut images: Vec<BackupImageRecord> = Vec::new();
    let mut has_background = false;

    tar_gz::read_entries(layer, &mut |entry| match entry.name.as_str() {
        ENTRY_MANIFEST => manifest = decode_or_none(&entry.bytes),
        ENTRY_TABLES => tables = decode_or_none(&entry.bytes),
        ENTRY_PREFS => prefs = decode_or_none(&entry.bytes),
        ENTRY_LLM_CONFIG => {
            let s = String::from_utf8_lossy(&entry.bytes).to_string();
            llm_config_json = if s.trim().is_empty() { None } else { Some(s) };
        }
        ENTRY_SECRETS => secrets = decode_or_none(&entry.bytes),
        ENTRY_IMAGES => images = decode_or_none(&entry.bytes).unwrap_or_default(),
        ENTRY_BACKGROUND => has_background = true,
        _ => {}
    })
    .map_err(io_err)?;

    Ok(BackupContent {
        manifest: manifest.unwrap_or_default(),
        tables: tables.unwrap_or_default(),
        prefs: prefs.unwrap_or_default(),
        llm_config_json,
        secrets,
        images,
        has_background,
    })
}

/// 逐条读图片字节（导入时在用户确认后再调一次；**一次只让一张图进内存**）。
///
/// `on_image(file_name, bytes)` 的 `file_name` 已剥掉 [`IMAGE_DIR`] 前缀。
pub fn read_images<R, FI, FB>(
    input: R,
    password: Option<&str>,
    mut on_image: FI,
    mut on_background: FB,
) -> Result<(), BackupError>
where
    R: Read,
    FI: FnMut(&str, Vec<u8>),
    FB: FnMut(Vec<u8>),
{
    let layer = crypto::reader_for(input, password)?;
    tar_gz::read_entries(layer, &mut |entry| {
        if let Some(name) = entry.name.strip_prefix(IMAGE_DIR) {
            on_image(name, entry.bytes);
        } else if entry.name == ENTRY_BACKGROUND {
            on_background(entry.bytes);
        }
    })
    .map_err(io_err)?;
    Ok(())
}

/// 读图片条目的**名字**（不读字节，用于清单预览）。
pub fn read_image_names<R: Read>(
    input: R,
    password: Option<&str>,
) -> Result<Vec<String>, BackupError> {
    let mut names = Vec::new();
    read_images(input, password, |name, _| names.push(name.to_string()), |_| {})?;
    Ok(names)
}

// =============================================================================
// 内部
// =============================================================================

/// 解 JSON；解不开返回 `None`（**不报错**，由调用方退化成默认值）。
fn decode_or_none<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Option<T> {
    serde_json::from_slice(bytes).ok()
}

fn io_err(e: std::io::Error) -> BackupError {
    if let Some(inner) = e.get_ref() {
        if let Some(be) = inner.downcast_ref::<BackupError>() {
            return be.clone();
        }
    }
    BackupError::Io(e.to_string())
}

/// 便于调用方复用：判断某个条目名是否是图片字节条目。
#[must_use]
pub fn is_image_entry(name: &str) -> bool {
    name.starts_with(IMAGE_DIR) && name.len() > IMAGE_DIR.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BackupSection, FormulaRow, HistoryRow};
    use std::collections::BTreeMap;

    fn sample() -> BackupContent {
        let mut c = BackupContent::default();
        c.manifest.sections = vec![BackupSection::Formulas, BackupSection::Images];
        c.manifest.counts.insert("formulas".into(), 1);
        c.manifest.app_version_name = "1.0.0".into();
        c.tables.formulas.push(FormulaRow {
            id: "f1".into(),
            schema_json: r#"{"id":"f1"}"#.into(),
            favorite: 1,
            created_at: 10,
            updated_at: 20,
        });
        c.tables.history.push(HistoryRow {
            id: 1,
            formula_id: "f1".into(),
            formula_snapshot_json: "{}".into(),
            inputs_json: "{}".into(),
            result_json: "{}".into(),
            thinking_content: Some("想".into()),
            created_at: 5,
        });
        c.prefs.strings.insert("theme".into(), "dark".into());
        c.prefs.ints.insert("fontSize".into(), 14);
        c.llm_config_json = Some(r#"{"active":"a","profiles":[]}"#.into());
        c.secrets = Some(BackupSecrets {
            api_keys: BTreeMap::from([("a".to_string(), "sk-x".to_string())]),
        });
        c.images.push(BackupImageRecord {
            id: "i1".into(),
            file_name: "aaaa.png".into(),
            mime_type: "image/png".into(),
            refs: vec!["formula:f1".into()],
            created_at: 7,
        });
        c
    }

    /// 打包（**不带**背景图）
    fn pack(content: &BackupContent, password: Option<&str>) -> Vec<u8> {
        pack_bg(content, password, None)
    }

    /// 打包（显式指定背景图字节）
    fn pack_bg(content: &BackupContent, password: Option<&str>, bg: Option<Vec<u8>>) -> Vec<u8> {
        let mut out = Vec::new();
        write_to(
            &mut out,
            content,
            |r| Some(format!("bytes-of-{}", r.file_name).into_bytes()),
            bg,
            password,
        )
        .unwrap();
        out
    }

    /// 列出包内条目名（解密 + 解压后）
    fn entry_names(data: &[u8], password: Option<&str>) -> Vec<String> {
        let layer = crypto::reader_for(data, password).unwrap();
        tar_gz::read_all(layer)
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect()
    }

    // ---------------------------------------------------------------------
    // 条目布局
    // ---------------------------------------------------------------------

    /// 🔴 条目名与顺序（跨端契约）
    #[test]
    fn entry_names_and_order() {
        let data = pack_bg(&sample(), None, Some(b"BG".to_vec()));
        let names = entry_names(&data, None);
        assert_eq!(
            names,
            vec![
                "manifest.json",
                "tables.json",
                "prefs.json",
                "llm_config.json",
                "secrets.json",
                "images.json",
                "images/aaaa.png",
                "background.jpg",
            ]
        );
    }

    /// 常量与 `docs` 契约逐字一致
    #[test]
    fn entry_constants_match_contract() {
        assert_eq!(ENTRY_MANIFEST, "manifest.json");
        assert_eq!(ENTRY_TABLES, "tables.json");
        assert_eq!(ENTRY_PREFS, "prefs.json");
        assert_eq!(ENTRY_LLM_CONFIG, "llm_config.json");
        assert_eq!(ENTRY_SECRETS, "secrets.json");
        assert_eq!(ENTRY_IMAGES, "images.json");
        assert_eq!(ENTRY_BACKGROUND, "background.jpg");
        assert_eq!(IMAGE_DIR, "images/");
    }

    /// 无 llmConfig / 无 secrets / 无图片 / 无背景 → 只有前三个条目
    #[test]
    fn minimal_archive_has_only_three_entries() {
        let c = BackupContent::default();
        let names = entry_names(&pack(&c, None), None);
        assert_eq!(names, vec!["manifest.json", "tables.json", "prefs.json"]);
    }

    /// 🔴 空串 llmConfig 视为「没有」（源 `takeIf { isNotBlank() }`）
    #[test]
    fn blank_llm_config_is_omitted() {
        let c = BackupContent {
            llm_config_json: Some("   ".into()),
            ..Default::default()
        };
        let names = entry_names(&pack(&c, None), None);
        assert!(!names.contains(&ENTRY_LLM_CONFIG.to_string()));
    }

    /// 没有 secrets 条目（未勾选含密钥）
    #[test]
    fn secrets_omitted_when_none() {
        let c = BackupContent {
            secrets: None,
            ..Default::default()
        };
        assert!(!entry_names(&pack(&c, None), None).contains(&ENTRY_SECRETS.to_string()));
    }

    /// 有 images 但图片文件已不在 → 跳过该图片条目，`images.json` 仍在
    #[test]
    fn missing_image_bytes_are_skipped() {
        let mut out = Vec::new();
        let c = sample();
        write_to(&mut out, &c, |_| None, None, None).unwrap();
        let names = entry_names(&out, None);
        assert!(names.contains(&ENTRY_IMAGES.to_string()), "登记信息仍在");
        assert!(!names.contains(&"images/aaaa.png".to_string()), "字节缺失应跳过");
    }

    /// 多张图按顺序产出
    #[test]
    fn multiple_images_keep_order() {
        let mut c = BackupContent::default();
        for n in ["a.png", "b.png", "c.png"] {
            c.images.push(BackupImageRecord {
                id: n.into(),
                file_name: n.into(),
                mime_type: "image/png".into(),
                refs: vec![],
                created_at: 0,
            });
        }
        let names = entry_names(&pack(&c, None), None);
        let imgs: Vec<&String> = names.iter().filter(|n| n.starts_with(IMAGE_DIR)).collect();
        assert_eq!(imgs, ["images/a.png", "images/b.png", "images/c.png"]);
    }

    // ---------------------------------------------------------------------
    // read_meta 往返
    // ---------------------------------------------------------------------

    #[test]
    fn meta_roundtrip() {
        let c = sample();
        let got = read_meta(&pack(&c, None)[..], None).unwrap();
        assert_eq!(got, c, "读回的元信息应与写入一致");
    }

    /// 🔴 图片**登记信息**读得到，但字节不读（read_meta 不做图片 IO）
    #[test]
    fn read_meta_does_not_read_image_bytes() {
        let c = sample();
        let data = pack_bg(&c, None, Some(b"BG".to_vec()));
        let got = read_meta(&data[..], None).unwrap();
        assert_eq!(got.images.len(), 1);
        assert_eq!(got.images[0].file_name, "aaaa.png");
        assert!(got.has_background);
    }

    /// `encodeDefaults` 语义：写出的 JSON 含全部字段（含默认值）
    #[test]
    fn json_written_with_defaults() {
        let c = BackupContent::default();
        let data = pack(&c, None);
        let layer = crypto::reader_for(&data[..], None).unwrap();
        let entries = tar_gz::read_all(layer).unwrap();
        let manifest = entries
            .iter()
            .find(|e| e.name == ENTRY_MANIFEST)
            .expect("manifest 条目");
        let v: serde_json::Value = serde_json::from_slice(&manifest.bytes).unwrap();
        assert_eq!(v["backupFormatVersion"], 1, "默认值也要写出");
        assert!(v.get("appVersionName").is_some(), "空串也要写出");
        assert!(v.get("sections").is_some());
    }

    /// 缺条目 → 默认值（旧备份兼容）
    #[test]
    fn missing_entries_degrade_to_defaults() {
        let c = BackupContent::default();
        let got = read_meta(&pack(&c, None)[..], None).unwrap();
        assert_eq!(got.manifest, BackupManifest::default());
        assert_eq!(got.tables, BackupTables::default());
        assert_eq!(got.prefs, BackupPrefs::default());
        assert!(got.llm_config_json.is_none());
        assert!(got.secrets.is_none());
        assert!(got.images.is_empty());
        assert!(!got.has_background);
    }

    /// 🔴 某个条目 JSON 坏掉 → 该项退默认，**其它项照常读出**
    #[test]
    fn corrupt_entry_degrades_only_itself() {
        let mut c = BackupContent::default();
        c.manifest.app_version_name = "keep".into();
        c.prefs.strings.insert("k".into(), "v".into());

        // 手工拼一个 tables.json 坏掉的包
        let mut out = Vec::new();
        let entries = vec![
            TarEntry::new(ENTRY_MANIFEST, serde_json::to_vec(&c.manifest).unwrap()),
            TarEntry::new(ENTRY_TABLES, b"{ this is not json".to_vec()),
            TarEntry::new(ENTRY_PREFS, serde_json::to_vec(&c.prefs).unwrap()),
        ];
        tar_gz::write_to(&mut out, entries).unwrap();

        let got = read_meta(&out[..], None).unwrap();
        assert_eq!(got.manifest.app_version_name, "keep", "其它条目不受影响");
        assert_eq!(got.prefs.strings.get("k").map(String::as_str), Some("v"));
        assert_eq!(got.tables, BackupTables::default(), "坏条目退默认");
    }

    /// 未知条目被忽略（将来加条目不破坏旧版本读取）
    #[test]
    fn unknown_entries_are_ignored() {
        let mut out = Vec::new();
        let entries = vec![
            TarEntry::new(ENTRY_MANIFEST, serde_json::to_vec(&BackupManifest::default()).unwrap()),
            TarEntry::new("future_thing.json", b"{\"x\":1}".to_vec()),
        ];
        tar_gz::write_to(&mut out, entries).unwrap();
        let got = read_meta(&out[..], None).unwrap();
        assert_eq!(got.manifest, BackupManifest::default());
    }

    /// `llm_config.json` 是**原样字符串**搬运
    #[test]
    fn llm_config_is_verbatim() {
        let raw = r#"{"active":"a","thinkingLevel":"MAX","extra":123}"#;
        let c = BackupContent {
            llm_config_json: Some(raw.into()),
            ..Default::default()
        };
        let got = read_meta(&pack(&c, None)[..], None).unwrap();
        assert_eq!(got.llm_config_json.as_deref(), Some(raw));
    }

    // ---------------------------------------------------------------------
    // read_images
    // ---------------------------------------------------------------------

    #[test]
    fn read_images_streams_bytes() {
        let c = sample();
        let data = pack_bg(&c, None, Some(b"BG".to_vec()));
        let mut images: Vec<(String, Vec<u8>)> = Vec::new();
        let mut bg: Option<Vec<u8>> = None;
        read_images(
            &data[..],
            None,
            |name, bytes| images.push((name.to_string(), bytes)),
            |bytes| bg = Some(bytes),
        )
        .unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].0, "aaaa.png", "前缀应被剥掉");
        assert_eq!(images[0].1, b"bytes-of-aaaa.png");
        assert_eq!(bg.unwrap(), b"BG");
    }

    /// 🔴 `write_to` **不读** `content.has_background` —— 以 `background_bytes` 参数为准
    ///
    /// 所以「标志为 true 但没给字节」的包读回来是 `false`。这是源的刻意设计
    /// （标志只是**读入时**的产物），不对称但是契约。
    #[test]
    fn write_ignores_has_background_flag() {
        let c = BackupContent {
            has_background: true, // 只设标志，不给字节
            ..Default::default()
        };
        let got = read_meta(&pack(&c, None)[..], None).unwrap();
        assert!(!got.has_background, "没给字节就不该有背景条目");
    }

    /// 给了字节 → 读回 `has_background = true`（与标志无关）
    #[test]
    fn background_bytes_produce_entry() {
        let c = BackupContent::default();
        let got = read_meta(&pack_bg(&c, None, Some(b"BG".to_vec()))[..], None).unwrap();
        assert!(got.has_background);
    }

    /// 没有图片/背景时不回调
    #[test]
    fn read_images_no_callbacks_for_minimal_archive() {
        let data = pack(&BackupContent::default(), None);
        // 两个回调都要计数：用 `Cell` 避免同时可变借用同一个变量
        let called = std::cell::Cell::new(0usize);
        read_images(&data[..], None, |_, _| called.set(called.get() + 1), |_| {
            called.set(called.get() + 1)
        })
        .unwrap();
        assert_eq!(called.get(), 0);
    }

    #[test]
    fn read_image_names_lists_only_images() {
        let c = sample();
        let names = read_image_names(&pack(&c, None)[..], None).unwrap();
        assert_eq!(names, ["aaaa.png"]);
    }

    #[test]
    fn is_image_entry_works() {
        assert!(is_image_entry("images/abc.png"));
        assert!(!is_image_entry("images/"), "只有前缀不算条目");
        assert!(!is_image_entry("images.json"));
        assert!(!is_image_entry("manifest.json"));
    }

    // ---------------------------------------------------------------------
    // 加密包
    // ---------------------------------------------------------------------

    /// 🔴 加密包往返（读元信息 + 读图片）
    #[test]
    fn encrypted_archive_roundtrip() {
        let c = sample();
        let data = pack(&c, Some("pw123"));
        // 未给密码 → NeedPassword
        assert_eq!(read_meta(&data[..], None).unwrap_err(), BackupError::NeedPassword);
        // 错密码 → WrongPassword
        assert_eq!(
            read_meta(&data[..], Some("bad")).unwrap_err(),
            BackupError::WrongPassword
        );
        // 对密码 → 读回
        let got = read_meta(&data[..], Some("pw123")).unwrap();
        assert_eq!(got, c);

        let mut images = Vec::new();
        read_images(&data[..], Some("pw123"), |n, b| images.push((n.to_string(), b)), |_| {}).unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].1, b"bytes-of-aaaa.png");
    }

    /// 加密包的条目名仍可枚举（清单确认阶段不需要密码之外的信息）
    #[test]
    fn encrypted_entry_names_readable() {
        let names = entry_names(&pack(&sample(), Some("pw")), Some("pw"));
        assert!(names.contains(&"manifest.json".to_string()));
        assert!(names.contains(&"images/aaaa.png".to_string()));
    }

    /// 明文包误填密码仍可读（与加密层一致）
    #[test]
    fn plain_archive_with_password_still_reads() {
        let c = sample();
        let data = pack(&c, None);
        assert_eq!(read_meta(&data[..], Some("irrelevant")).unwrap(), c);
    }

    // ---------------------------------------------------------------------
    // 大图 / 懒式写入
    // ---------------------------------------------------------------------

    /// 🔴 图片字节是**按需**取的：未登记在 `images` 里的文件不会被读取
    #[test]
    fn image_bytes_requested_lazily_only_for_records() {
        let mut c = BackupContent::default();
        c.images.push(BackupImageRecord {
            id: "x".into(),
            file_name: "only.png".into(),
            mime_type: "image/png".into(),
            refs: vec![],
            created_at: 0,
        });
        let mut asked: Vec<String> = Vec::new();
        let mut out = Vec::new();
        write_to(
            &mut out,
            &c,
            |r| {
                asked.push(r.file_name.clone());
                Some(b"X".to_vec())
            },
            None,
            None,
        )
        .unwrap();
        assert_eq!(asked, ["only.png"], "只应请求已登记的那一张");
    }

    /// 大图（跨 tar 块边界）也能原样往返
    #[test]
    fn large_image_roundtrip() {
        let mut c = BackupContent::default();
        c.images.push(BackupImageRecord {
            id: "big".into(),
            file_name: "big.bin".into(),
            mime_type: "image/png".into(),
            refs: vec![],
            created_at: 0,
        });
        let payload: Vec<u8> = (0..200_000).map(|i| (i % 251) as u8).collect();
        let mut out = Vec::new();
        write_to(&mut out, &c, |_| Some(payload.clone()), None, None).unwrap();

        let mut got = Vec::new();
        read_images(&out[..], None, |_, b| got = b, |_| {}).unwrap();
        assert_eq!(got, payload);
    }
}
