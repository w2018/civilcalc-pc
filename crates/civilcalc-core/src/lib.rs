//! # civilcalc-core —— AI全能计算器 PC 端领域层
//!
//! 从源项目 `civilcalc-android-v2` 的 `:core` 模块移植（该模块是**纯 JVM**，
//! 逻辑与平台解耦，自建 `CivilLog` 门面即为证据）。
//!
//! ## 硬约束
//!
//! - **不得依赖 `tauri`**（ADR-002）—— 由 `Cargo.toml` 天然强制
//! - **不得依赖任何平台 API** —— 路径、凭据、网络均由上层注入
//!
//! ## 移植纪律
//!
//! 1. **语义等价优先**：先 1:1 复刻行为（含边界与错误文案），再考虑优化
//! 2. **保留源码注释里的"为什么"**：源项目 BUG-01~BUG-27 的修复理由必须作为 Rust 注释保留
//! 3. **每个模块移植后先过对应测试**：源项目 47 个测试文件是验收预言机
//!
//! ## 模块对应关系（源项目 `core/` 包）
//!
//! | 本 crate 模块 | 源项目包 | 任务 |
//! |---|---|---|
//! | [`schema`] | `core/schema/` | P0-2, P0-3 |
//! | [`engine`] | `core/formula/engine/` | P0-4 ~ P0-10 |
//! | [`excel`] | `core/formula/export/` | P3-1 ~ P3-3 |
//! | [`display`] | `core/formula/display/` | P3-5 |
//! | [`verify`] | `core/formula/verify/` | P3-7 |
//! | [`version`] | `core/version/` | P1-12 |
//! | [`search`] | `core/search/` | P2-4 |
//! | [`source`] | `core/source/` | P0-11 |
//! | [`reset`] | `core/reset/` | P5-2 |
//! | [`builtin`] | `app/src/main/assets/` | P0-11 |
//! | [`log`] | `core/log/` | P0-12 |

pub mod builtin;
pub mod display;
pub mod engine;
pub mod excel;
pub mod log;
pub mod number_format;
pub mod result_outputs;
pub mod reset;
pub mod schema;
pub mod search;
pub mod serde_util;
pub mod source;
pub mod verify;
pub mod version;

/// 当前时间戳（Unix 毫秒）。
///
/// 源项目各处用 `System.currentTimeMillis()`；PC 端统一走本函数，
/// 便于测试替换与保证单调性口径一致。
pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 统一错误类型。
///
/// 变体与错误码**逐一对齐**源项目 `core/error/AppError.kt` 的 9 个变体，
/// 便于跨端错误文案一致（见 `docs/04-数据契约.md` §7.1）。
///
/// ## 为什么全部用**结构变体**（而不是 `Validation(String)`）
///
/// 两个必须遵守的理由：
///
/// 1. **serde 内部标记（`tag = "kind"`）不支持 newtype 变体装 `String`** ——
///    写成 `Validation(String)` 时 `serde_json::to_value` 会直接 panic：
///    `cannot serialize tagged newtype variant CoreError::Validation containing a string`。
///    这是**运行时**才炸的坑，编译期查不出来。
/// 2. 结构变体让本类型与 `src-tauri` 的 `CommandError` **序列化形状完全一致**，
///    `From<CoreError> for CommandError` 因此是纯字段改名，零映射逻辑。
///
/// ## ⚠️ `rename_all_fields` 不可省
///
/// `#[serde(rename_all)]` 只重命名**变体名**，不重命名结构变体的**字段**。
/// 少了它 `ai_code` 会序列化成 `"ai_code"`，而契约要求 `aiCode`
/// （回归测试：`tests::ai_error_field_is_camel_case`）。
#[derive(Debug, Error, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum CoreError {
    /// Schema 校验失败（对齐 `AppError.Validation` / `VALIDATION_ERROR`）
    #[error("{message}")]
    Validation { message: String },

    /// 未找到（对齐 `AppError.NotFound` / `NOT_FOUND`）
    #[error("{message}")]
    NotFound { message: String },

    /// 未授权（对齐 `AppError.Unauthorized` / `UNAUTHORIZED`）
    #[error("{message}")]
    Unauthorized { message: String },

    /// 网络（对齐 `AppError.Network` / `NETWORK_ERROR`）
    #[error("{message}")]
    Network { message: String },

    /// 存储（对齐 `AppError.Storage` / `STORAGE_ERROR`）
    #[error("{message}")]
    Storage { message: String },

    /// AI 错误（对齐 `AppError.AiError` / `AI_ERROR`，含子错误码）
    #[error("{message}")]
    AiError { message: String, ai_code: String },

    /// 导出（对齐 `AppError.Export` / `EXPORT_ERROR`）
    #[error("{message}")]
    Export { message: String },

    /// 解析（对齐 `AppError.Parse` / `PARSE_ERROR`）
    #[error("{message}")]
    Parse { message: String },

    /// 内置公式来源校验失败（源项目规格中的独立错误类别）
    #[error("{message}")]
    BuiltinSource { message: String },

    /// 路径穿越（PC 端新增，对应导出安全校验）
    #[error("{message}")]
    PathTraversal { message: String },

    /// 参数非法
    #[error("{message}")]
    InvalidArgument { message: String },

    /// 未知（对齐 `AppError.Unknown` / `UNKNOWN_ERROR`）
    #[error("{message}")]
    Unknown { message: String },
}

impl CoreError {
    /// 错误码（与源项目 `AppError.code` 一致）
    pub fn code(&self) -> &'static str {
        match self {
            CoreError::Validation { .. } => "VALIDATION_ERROR",
            CoreError::NotFound { .. } => "NOT_FOUND",
            CoreError::Unauthorized { .. } => "UNAUTHORIZED",
            CoreError::Network { .. } => "NETWORK_ERROR",
            CoreError::Storage { .. } => "STORAGE_ERROR",
            CoreError::AiError { .. } => "AI_ERROR",
            CoreError::Export { .. } => "EXPORT_ERROR",
            CoreError::Parse { .. } => "PARSE_ERROR",
            CoreError::BuiltinSource { .. } => "BUILTIN_SOURCE_ERROR",
            CoreError::PathTraversal { .. } => "PATH_TRAVERSAL",
            CoreError::InvalidArgument { .. } => "INVALID_ARGUMENT",
            CoreError::Unknown { .. } => "UNKNOWN_ERROR",
        }
    }
}

/// 领域层统一结果别名
pub type Result<T> = std::result::Result<T, CoreError>;

#[cfg(test)]
mod tests {
    use super::*;

    /// 12 个变体 → 12 个错误码，逐一对齐源项目 `AppError.code`
    #[test]
    fn error_codes_match_source() {
        let cases: Vec<(CoreError, &str)> = vec![
            (CoreError::Validation { message: String::new() }, "VALIDATION_ERROR"),
            (CoreError::NotFound { message: String::new() }, "NOT_FOUND"),
            (CoreError::Unauthorized { message: String::new() }, "UNAUTHORIZED"),
            (CoreError::Network { message: String::new() }, "NETWORK_ERROR"),
            (CoreError::Storage { message: String::new() }, "STORAGE_ERROR"),
            (
                CoreError::AiError {
                    message: String::new(),
                    ai_code: String::new(),
                },
                "AI_ERROR",
            ),
            (CoreError::Export { message: String::new() }, "EXPORT_ERROR"),
            (CoreError::Parse { message: String::new() }, "PARSE_ERROR"),
            (CoreError::BuiltinSource { message: String::new() }, "BUILTIN_SOURCE_ERROR"),
            (CoreError::PathTraversal { message: String::new() }, "PATH_TRAVERSAL"),
            (CoreError::InvalidArgument { message: String::new() }, "INVALID_ARGUMENT"),
            (CoreError::Unknown { message: String::new() }, "UNKNOWN_ERROR"),
        ];
        assert_eq!(cases.len(), 12);
        for (e, code) in cases {
            assert_eq!(e.code(), code);
        }
    }

    /// serde 判别联合的形状：`kind` 是 camelCase 变体名
    #[test]
    fn serde_kind_is_camel_case() {
        for (e, kind) in [
            (CoreError::Validation { message: "x".into() }, "validation"),
            (CoreError::NotFound { message: "x".into() }, "notFound"),
            (CoreError::Unauthorized { message: "x".into() }, "unauthorized"),
            (CoreError::Network { message: "x".into() }, "network"),
            (CoreError::Storage { message: "x".into() }, "storage"),
            (CoreError::Export { message: "x".into() }, "export"),
            (CoreError::Parse { message: "x".into() }, "parse"),
            (CoreError::BuiltinSource { message: "x".into() }, "builtinSource"),
            (CoreError::PathTraversal { message: "x".into() }, "pathTraversal"),
            (CoreError::InvalidArgument { message: "x".into() }, "invalidArgument"),
            (CoreError::Unknown { message: "x".into() }, "unknown"),
        ] {
            let v = serde_json::to_value(&e).unwrap();
            assert_eq!(v["kind"], serde_json::json!(kind), "变体名须为 camelCase");
        }
    }

    /// **回归**：`AiError.ai_code` 必须序列化为 `aiCode`
    ///
    /// `#[serde(rename_all)]` 只作用于变体名，不作用于结构变体的字段；
    /// 少了 `rename_all_fields` 会输出 `"ai_code"`，前端按 `aiCode` 取就取不到。
    #[test]
    fn ai_error_field_is_camel_case() {
        let e = CoreError::AiError {
            message: "调用失败".into(),
            ai_code: "AUTH_INVALID".into(),
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], serde_json::json!("aiError"));
        assert_eq!(v["message"], serde_json::json!("调用失败"));
        assert_eq!(v["aiCode"], serde_json::json!("AUTH_INVALID"));
        assert!(
            v.get("ai_code").is_none(),
            "不得泄漏 snake_case 字段名（前端契约是 aiCode）"
        );
    }

    #[test]
    fn serde_roundtrip() {
        let e = CoreError::AiError {
            message: "m".into(),
            ai_code: "X".into(),
        };
        let s = serde_json::to_string(&e).unwrap();
        let back: CoreError = serde_json::from_str(&s).unwrap();
        assert_eq!(back.code(), e.code());
    }

    #[test]
    fn now_ms_is_plausible() {
        // 2020-01-01 之后、2100 年之前
        let t = now_ms();
        assert!(t > 1_577_836_800_000, "时间戳过早: {t}");
        assert!(t < 4_102_444_800_000, "时间戳过晚: {t}");
    }
}
