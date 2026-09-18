//! serde 辅助模块。
//!
//! ## 为什么需要 [`bool_as_int`]
//!
//! 备份包里的若干字段在源项目是 **`Int`（0/1）而不是 Boolean**：
//!
//! | 备份 JSON 路径 | 源类型 |
//! |---|---|
//! | `tables.versions[].verified` | `FormulaVersionRow.verified: Int` |
//! | `tables.formulas[].favorite` | `FormulaRow.favorite: Int` |
//!
//! 而 `kotlinx.serialization` 对类型是**严格**的 —— Rust 若写成
//! `"verified": false`，Android 端反序列化会直接抛异常，导入失败。
//!
//! 备份包是**对外契约**（ADR-009），因此 Rust 侧统一：
//!
//! - 内存里用语义清晰的 `bool`
//! - 序列化时用本模块保证落地为 `0` / `1`
//! - 反序列化时**同时容忍** `0/1` 与 `true/false`（便于回读 Rust 早期数据）
//!
//! 用法：
//!
//! ```ignore
//! #[serde(with = "crate::serde_util::bool_as_int", default)]
//! pub verified: bool,
//! ```

/// 把 `bool` 与整数 `0`/`1` 互转（供 `#[serde(with = ...)]` 使用）。
pub mod bool_as_int {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &bool, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_i64(if *v { 1 } else { 0 })
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Int(i64),
            Bool(bool),
        }
        Ok(match Raw::deserialize(d)? {
            Raw::Int(i) => i != 0,
            Raw::Bool(b) => b,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Holder {
        #[serde(with = "super::bool_as_int")]
        flag: bool,
    }

    #[test]
    fn serializes_bool_as_int() {
        let t = serde_json::to_string(&Holder { flag: true }).unwrap();
        let f = serde_json::to_string(&Holder { flag: false }).unwrap();
        assert_eq!(t, r#"{"flag":1}"#);
        assert_eq!(f, r#"{"flag":0}"#);
    }

    #[test]
    fn accepts_both_int_and_bool() {
        for (json, expect) in [
            (r#"{"flag":1}"#, true),
            (r#"{"flag":0}"#, false),
            (r#"{"flag":7}"#, true),
            (r#"{"flag":true}"#, true),
            (r#"{"flag":false}"#, false),
        ] {
            let got: Holder = serde_json::from_str(json).unwrap();
            assert_eq!(got.flag, expect, "输入: {json}");
        }
    }
}
