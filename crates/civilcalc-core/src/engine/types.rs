//! 引擎类型定义。
//!
//! 源：`civilcalc-android-v2/core/formula/engine/FormulaEngine.kt`
//!
//! ## 设计要点
//!
//! - `CompiledExpr` 携带 **RPN**（逆波兰），而非 AST —— 因为源项目的
//!   **Excel 转换直接消费 RPN**（`convertRpnToExcelWithRefs`），
//!   保留 RPN 是该能力的前提（见 ADR-013）
//! - `RpnToken::Operator` 冗余存 `precedence` 与 `right_associative`：
//!   这是源项目的做法，让 Excel 转换在重建括号时无需回查运算符表
//! - `FormulaResult` 在 Kotlin 是自定义 sealed class（受泛型限制）；
//!   Rust 用标准 `Result` 即可，保留类型别名以便与源项目对照

use crate::schema::EvalError;
use serde::{Deserialize, Serialize};

/// 引擎结果别名（对应源项目 `FormulaResult<T, E>`）
pub type FormulaResult<T, E> = std::result::Result<T, E>;

/// 编译结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum CompileResult {
    /// 编译成功
    Ok { expr: CompiledExpr },
    /// 编译失败（`position` 为出错字符位置，可空）
    Error {
        #[serde(default)]
        position: Option<usize>,
        reason: String,
    },
}

impl CompileResult {
    pub fn is_ok(&self) -> bool {
        matches!(self, CompileResult::Ok { .. })
    }

    pub fn is_error(&self) -> bool {
        matches!(self, CompileResult::Error { .. })
    }

    /// 取编译后的表达式（失败时 `None`）
    pub fn expr(&self) -> Option<&CompiledExpr> {
        match self {
            CompileResult::Ok { expr } => Some(expr),
            CompileResult::Error { .. } => None,
        }
    }

    /// 取失败原因（成功时 `None`）
    pub fn reason(&self) -> Option<&str> {
        match self {
            CompileResult::Ok { .. } => None,
            CompileResult::Error { reason, .. } => Some(reason),
        }
    }
}

/// 编译后的表达式（RPN 形式）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledExpr {
    /// 逆波兰序列
    pub rpn: Vec<RpnToken>,
    /// 表达式中出现的变量名（去重，保持首现顺序）
    pub variables: Vec<String>,
    /// 表达式中出现的函数名（去重，保持首现顺序）
    pub functions: Vec<String>,
}

/// 逆波兰 token。
///
/// ⚠️ `rename_all_fields = "camelCase"` 不可省：源项目
/// `RpnToken.Operator(op, precedence, rightAssociative)` 与
/// `RpnToken.Function(name, argCount)` 都是 **camelCase**，
/// 少了它 `right_associative` / `arg_count` 会以 snake_case 出境，
/// 前端按 `rightAssociative` / `argCount` 取就取不到。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RpnToken {
    /// 数字字面量
    Number { value: f64 },
    /// 变量（需由调用方提供值）
    Variable { name: String },
    /// 运算符。
    ///
    /// `op` 取值：`+` `-` `*` `/` `^` `%` `u-`（一元负号）
    Operator {
        op: String,
        precedence: i32,
        right_associative: bool,
    },
    /// 函数调用
    Function { name: String, arg_count: usize },
    /// 常量（如 `pi`、`e`）—— 编译期即替换为字面量值
    Constant { name: String, value: f64 },
}

// =============================================================================
// 运算符优先级（对齐源项目 Parser）
// =============================================================================

/// `+` / `-`
pub const PREC_ADD: i32 = 1;
/// `*` / `/` / `%`
pub const PREC_MUL: i32 = 2;
/// `^`（右结合）
pub const PREC_POW: i32 = 3;
/// 一元负号（右结合，优先级最高）
pub const PREC_UNARY: i32 = 4;

/// 一元负号的运算符符号（源项目用 `"u-"`）
pub const OP_UNARY_MINUS: &str = "u-";

/// 引擎可接受的运算符（除一元负号）
pub const OPERATORS: &[&str] = &["+", "-", "*", "/", "^", "%"];

/// 判断某个 op 字符串是否为合法运算符（含一元负号）
pub fn is_operator(op: &str) -> bool {
    op == OP_UNARY_MINUS || OPERATORS.contains(&op)
}

/// 运算符优先级；未知运算符返回 0
pub fn precedence_of(op: &str) -> i32 {
    match op {
        "+" | "-" => PREC_ADD,
        "*" | "/" | "%" => PREC_MUL,
        "^" => PREC_POW,
        OP_UNARY_MINUS => PREC_UNARY,
        _ => 0,
    }
}

/// 运算符是否右结合
pub fn is_right_associative(op: &str) -> bool {
    matches!(op, "^" | OP_UNARY_MINUS)
}

/// 由 [`EvalError`] 构造编译错误（无位置）—— 供引擎内部错误上报使用
pub fn compile_error(reason: impl Into<String>) -> EvalError {
    EvalError::compile(reason)
}

/// 由 [`EvalError`] 构造编译错误（带位置）
pub fn compile_error_at(position: usize, reason: impl Into<String>) -> EvalError {
    EvalError::compile_at(position, reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_matches_source() {
        assert_eq!(precedence_of("+"), 1);
        assert_eq!(precedence_of("-"), 1);
        assert_eq!(precedence_of("*"), 2);
        assert_eq!(precedence_of("/"), 2);
        assert_eq!(precedence_of("%"), 2);
        assert_eq!(precedence_of("^"), 3);
        assert_eq!(precedence_of(OP_UNARY_MINUS), 4);
        assert_eq!(precedence_of("??"), 0);
    }

    #[test]
    fn associativity_matches_source() {
        // 只有 ^ 与一元负号是右结合
        assert!(is_right_associative("^"));
        assert!(is_right_associative(OP_UNARY_MINUS));
        for op in ["+", "-", "*", "/", "%"] {
            assert!(!is_right_associative(op), "{op} 应为左结合");
        }
    }

    #[test]
    fn operator_recognition() {
        for op in OPERATORS {
            assert!(is_operator(op));
        }
        assert!(is_operator(OP_UNARY_MINUS));
        assert!(!is_operator("="));
        assert!(!is_operator("sqrt"));
    }

    #[test]
    fn compile_result_accessors() {
        let ok = CompileResult::Ok {
            expr: CompiledExpr {
                rpn: vec![RpnToken::Number { value: 1.0 }],
                variables: vec![],
                functions: vec![],
            },
        };
        assert!(ok.is_ok());
        assert!(ok.expr().is_some());
        assert!(ok.reason().is_none());

        let err = CompileResult::Error {
            position: Some(3),
            reason: "缺少左括号".to_string(),
        };
        assert!(err.is_error());
        assert!(err.expr().is_none());
        assert_eq!(err.reason(), Some("缺少左括号"));
    }

    // ---------------- serde 契约（前端依赖的字段名） ----------------

    /// **回归**：`RpnToken` 的多词字段必须是 camelCase
    ///
    /// 源项目 `RpnToken.Operator(op, precedence, rightAssociative)` /
    /// `RpnToken.Function(name, argCount)` 都是 camelCase。
    /// 少了 `rename_all_fields` 会输出 `right_associative` / `arg_count`。
    #[test]
    fn rpn_token_fields_are_camel_case() {
        let op = RpnToken::Operator {
            op: "+".to_string(),
            precedence: 1,
            right_associative: false,
        };
        let v = serde_json::to_value(&op).unwrap();
        assert_eq!(v["kind"], serde_json::json!("operator"));
        assert_eq!(v["op"], serde_json::json!("+"));
        assert_eq!(v["precedence"], serde_json::json!(1));
        assert_eq!(v["rightAssociative"], serde_json::json!(false));
        assert!(v.get("right_associative").is_none(), "不得泄漏 snake_case");

        let f = RpnToken::Function {
            name: "sqrt".to_string(),
            arg_count: 1,
        };
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!(v["argCount"], serde_json::json!(1));
        assert!(v.get("arg_count").is_none(), "不得泄漏 snake_case");
    }

    #[test]
    fn rpn_token_serde_roundtrip() {
        let tokens = vec![
            RpnToken::Number { value: 1.5 },
            RpnToken::Variable {
                name: "a".to_string(),
            },
            RpnToken::Operator {
                op: "u-".to_string(),
                precedence: 4,
                right_associative: true,
            },
            RpnToken::Function {
                name: "max".to_string(),
                arg_count: 2,
            },
            RpnToken::Constant {
                name: "pi".to_string(),
                value: 3.25,
            },
        ];
        let s = serde_json::to_string(&tokens).unwrap();
        let back: Vec<RpnToken> = serde_json::from_str(&s).unwrap();
        assert_eq!(back, tokens);
    }

    /// `CompileResult` 的判别字段是 `status`（不是 `kind`），且字段为 camelCase
    #[test]
    fn compile_result_shape() {
        let ok = CompileResult::Ok {
            expr: CompiledExpr {
                rpn: vec![],
                variables: vec!["a".to_string()],
                functions: vec![],
            },
        };
        let v = serde_json::to_value(&ok).unwrap();
        assert_eq!(v["status"], serde_json::json!("ok"));
        assert_eq!(v["expr"]["variables"], serde_json::json!(["a"]));

        let err = CompileResult::Error {
            position: None,
            reason: "bad".to_string(),
        };
        let v = serde_json::to_value(&err).unwrap();
        assert_eq!(v["status"], serde_json::json!("error"));
        assert_eq!(v["position"], serde_json::Value::Null);
        assert_eq!(v["reason"], serde_json::json!("bad"));
    }
}
