//! 公式版本链与差异。
//!
//! 源：`civilcalc-android-v2/core/version/`
//!
//! | 文件 | 源文件 | 任务 | 状态 |
//! |---|---|---|---|
//! | `mod.rs`（本文件） | `FormulaVersion.kt` | P1-4 | ✅ 已移植 |
//! | `chain.rs` | `VersionChainManager.kt` | P1-12 | ✅ 已移植（6 维 diff + head 解析） |
//!
//! ## 版本号规则（对齐源项目 `generateVersion`）
//!
//! 父版本为 `null` → 首版固定 `1.0.0`；否则按段数决定**在哪一位进位**：
//!
//! | 父版本 | 子版本 | 进位位 |
//! |---|---|---|
//! | `null` | `1.0.0` | —（首版） |
//! | `1.0.0` | `1.0.1` | 补丁位 +1 |
//! | `1.0.9` | `1.0.10` | 补丁位 +1（**不进位到 minor**） |
//! | `1.0` | `1.1.0` | 次版本位 +1，补丁位归零 |
//! | `1` | `2.0.0` | 主版本位 +1，其余归零 |
//! | `1.x.0` | `1.0.1` | 非数字段按 **0** 处理（`toIntOrNull() ?: 0`） |
//!
//! ⚠️ 注意最后一行：源项目对非法段是**静默当 0**，不报错。
//!
//! ## diff 维度（6 个，对齐源项目）
//!
//! `expression` / `altExpressions` / `variables` / `source.ref` / `source.verified` / `constants`
//!
//! ## ⚠️ PC 端补充：head 语义（ADR-014）
//!
//! 源项目 `FormulaVersion` **没有 head 字段**（已知缺口）。
//! PC 端在 `user_formulas` 表新增 `headVersion TEXT DEFAULT NULL` 列承载：
//! - `NULL` → 视为"最新版本即 head"（退化兼容，与源项目当前行为一致）
//! - `switch_version()` 更新该列
//!
//! ## `schema_json` 为什么是 String
//!
//! 与 [`crate::schema::HistoryEntry`] 同理：源项目 `FormulaVersion.schemaJson`、
//! `FormulaVersionEntity.schemaJson`、`FormulaVersionRow.schemaJson` **三处都是 String**。
//! 保持裸字符串才能让备份包"原样搬运"（ADR-009 红线）。
//! 需要结构体时用 [`FormulaVersion::schema`]。

use serde::{Deserialize, Serialize};

use crate::schema::FormulaSchema;
use crate::serde_util::bool_as_int;

pub mod chain;

pub use chain::{
    chain_sorted, compare_version, diff, next_version_for, resolve_head, DiffItem, VersionDiff,
    DIFF_DIMENSION_COUNT,
};

/// 首版版本号（源项目 `generateVersion` 在 `parentVersion == null` 时的固定返回值）
pub const INITIAL_VERSION: &str = "1.0.0";

/// 一条公式版本记录。
///
/// 同时充当：① SQLite `formula_versions` 表行；② 备份包 `tables.versions[]` 行。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormulaVersion {
    pub formula_id: String,

    /// 语义化版本号，如 `"1.0.3"`
    pub version: String,

    /// 父版本号；首版为 `None`
    #[serde(default)]
    pub parent_version: Option<String>,

    /// `FormulaSchema` 的 JSON（**裸字符串**，见模块文档）
    pub schema_json: String,

    /// 变更类型：`create` / `edit` / `refine` / `verify` / `revert` 等（自由字符串，不做枚举约束）
    #[serde(default)]
    pub change_type: String,

    #[serde(default)]
    pub change_log: String,

    /// 编辑者：`user` / `ai` / `system`
    #[serde(default)]
    pub editor: String,

    #[serde(default)]
    pub created_at: i64,

    /// 是否经过来源核验。
    ///
    /// 由 `create_version` 从 `schema.source.verified` 派生（对齐源项目）。
    ///
    /// ⚠️ 落地为 **`0`/`1`**（源项目 `FormulaVersionRow.verified: Int`），
    /// 而非 `true`/`false` —— 否则 Android 端导入备份会失败，见 [`bool_as_int`]。
    #[serde(with = "bool_as_int", default)]
    pub verified: bool,
}

impl FormulaVersion {
    /// 解析版本快照（`ignoreUnknownKeys` 语义由 `serde` 默认行为提供：Rust 结构体天然忽略多余字段）。
    pub fn schema(&self) -> Result<FormulaSchema, serde_json::Error> {
        serde_json::from_str(&self.schema_json)
    }

    /// 新建一条版本记录（对齐源项目 `VersionChainManager.createVersion`）。
    ///
    /// - `version` 由 [`next_version`] 从 `parent_version` 推导
    /// - `verified` 取 `schema.source.verified`
    /// - `created_at` 取当前时间
    pub fn create(
        schema: &FormulaSchema,
        parent_version: Option<&str>,
        change_type: impl Into<String>,
        change_log: impl Into<String>,
        editor: impl Into<String>,
    ) -> Self {
        Self {
            formula_id: schema.id.clone(),
            version: next_version(parent_version),
            parent_version: parent_version.map(str::to_string),
            schema_json: serde_json::to_string(schema).unwrap_or_default(),
            change_type: change_type.into(),
            change_log: change_log.into(),
            editor: editor.into(),
            created_at: crate::now_ms(),
            verified: schema.source.verified,
        }
    }
}

/// 推导下一个版本号（1:1 复刻源项目 `VersionChainManager.generateVersion`）。
///
/// 见模块文档的进位表。非法段按 `0` 处理，**不返回错误**。
pub fn next_version(parent_version: Option<&str>) -> String {
    let Some(parent) = parent_version else {
        return INITIAL_VERSION.to_string();
    };

    // 对齐 `parentVersion.split(".").map { it.toIntOrNull() ?: 0 }`
    let parts: Vec<i64> = parent
        .split('.')
        .map(|p| p.trim().parse::<i64>().unwrap_or(0))
        .collect();

    match parts.len() {
        0 => INITIAL_VERSION.to_string(),
        1 => format!("{}.0.0", parts[0].wrapping_add(1)),
        2 => format!("{}.{}.0", parts[0], parts[1].wrapping_add(1)),
        _ => format!("{}.{}.{}", parts[0], parts[1], parts[2].wrapping_add(1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_version_is_1_0_0() {
        assert_eq!(next_version(None), "1.0.0");
        assert_eq!(next_version(None), INITIAL_VERSION);
    }

    #[test]
    fn three_segments_bump_patch_without_carry() {
        assert_eq!(next_version(Some("1.0.0")), "1.0.1");
        // 关键：9 → 10 不进位到 minor（源项目行为）
        assert_eq!(next_version(Some("1.0.9")), "1.0.10");
        assert_eq!(next_version(Some("2.3.4")), "2.3.5");
        // 四段以上：只取前三位，等价于 3 段分支
        assert_eq!(next_version(Some("1.0.0.9")), "1.0.1");
    }

    #[test]
    fn two_segments_bump_minor_and_reset_patch() {
        assert_eq!(next_version(Some("1.0")), "1.1.0");
        assert_eq!(next_version(Some("2.7")), "2.8.0");
    }

    #[test]
    fn one_segment_bumps_major_and_resets_rest() {
        assert_eq!(next_version(Some("1")), "2.0.0");
        assert_eq!(next_version(Some("5")), "6.0.0");
    }

    #[test]
    fn non_numeric_segments_are_treated_as_zero() {
        // 源项目 toIntOrNull() ?: 0 —— 静默归零，不报错
        assert_eq!(next_version(Some("1.x.0")), "1.0.1");
        assert_eq!(next_version(Some("a.b.c")), "0.0.1");
        // ⚠️ 空串在 Kotlin/Rust 里 `split('.')` 都得 [""]（1 段，值为 0），
        //    走"1 段"分支 → major +1 → "1.0.0"
        assert_eq!(next_version(Some("")), "1.0.0");
    }

    #[test]
    fn verified_serializes_as_int_for_backup_compat() {
        let v = FormulaVersion {
            formula_id: "usr:1".into(),
            version: "1.0.0".into(),
            parent_version: None,
            schema_json: "{}".into(),
            change_type: "create".into(),
            change_log: String::new(),
            editor: "user".into(),
            created_at: 1,
            verified: true,
        };
        let json = serde_json::to_value(&v).unwrap();
        assert_eq!(json["verified"], serde_json::json!(1));
        assert_eq!(json["formulaId"], serde_json::json!("usr:1"));
        assert_eq!(json["parentVersion"], serde_json::Value::Null);
        assert_eq!(json["schemaJson"], serde_json::json!("{}"));

        let mut off = v.clone();
        off.verified = false;
        assert_eq!(
            serde_json::to_value(&off).unwrap()["verified"],
            serde_json::json!(0)
        );
    }

    #[test]
    fn verified_reads_legacy_bool_too() {
        let json = r#"{"formulaId":"a","version":"1.0.0","schemaJson":"{}","verified":true}"#;
        let v: FormulaVersion = serde_json::from_str(json).unwrap();
        assert!(v.verified);
        assert_eq!(v.change_type, ""); // 缺字段走 default
        assert_eq!(v.parent_version, None);
    }

    #[test]
    fn roundtrip_keeps_schema_json_verbatim() {
        // 关键：不得因解析-重序列化而改变 JSON 文本
        let raw = r#"{"z":1,"a":{"y":2,"b":3},"schemaVersion":3}"#;
        let v = FormulaVersion {
            formula_id: "f".into(),
            version: "1.0.0".into(),
            parent_version: None,
            schema_json: raw.into(),
            change_type: "create".into(),
            change_log: String::new(),
            editor: "user".into(),
            created_at: 0,
            verified: false,
        };
        let s = serde_json::to_string(&v).unwrap();
        let back: FormulaVersion = serde_json::from_str(&s).unwrap();
        assert_eq!(back.schema_json, raw, "schemaJson 必须逐字节保持");
    }

    #[test]
    fn create_derives_verified_from_source() {
        // ⚠️ SourceKind 的 serde 表示是 SCREAMING_SNAKE（对齐源项目 Kotlin 枚举名）
        let mut schema: FormulaSchema =
            serde_json::from_str(
                r#"{"id":"usr:1","resultName":"r","resultSymbol":"y","expression":"a+b",
                    "variables":[],"constants":{},"domain":"通用",
                    "source":{"kind":"CUSTOM","verified":true},"schemaVersion":3}"#,
            )
            .unwrap();
        let v = FormulaVersion::create(&schema, None, "create", "首版", "user");
        assert_eq!(v.version, "1.0.0");
        assert!(v.verified, "verified 应派生自 schema.source.verified");
        assert!(v.schema().is_ok());

        schema.source.verified = false;
        let v2 = FormulaVersion::create(&schema, Some("1.0.0"), "edit", "", "user");
        assert_eq!(v2.version, "1.0.1");
        assert!(!v2.verified);
        assert_eq!(v2.parent_version.as_deref(), Some("1.0.0"));
    }
}
