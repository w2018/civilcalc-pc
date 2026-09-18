//! 组 12b：二维数学排版（3 个命令）。
//!
//! | 命令 | 入参 | 返回 |
//! |---|---|---|
//! | `math_layout` | `expression`, `constants?` | `MathLayout`（多行） |
//! | `math_layout_equation` | `equation`, `constants?` | `MathNode[] \| null` |
//! | `math_layout_result_line` | `results` | `MathNode[]` |
//!
//! ## 为什么是三个命令而不是一个
//!
//! 源项目生产代码只用这三个 API（`FormulaScreen.kt:1205/1211/1218`）：
//!
//! 1. `build(schema.expression, schema.constants)` —— 公式主体（含分号分段 → 多行）
//! 2. `buildEquation(equation, schema.constants)` —— 用户写的**条件方程**（`x+y+z=6`）
//! 3. `buildResultLine(results)` —— 结果行（`(x, y) = (1, 2)`）
//!
//! 三者的**入参形态与失败语义都不同**，硬合成一个命令会让参数变成
//! 「按 mode 走不同字段」的联合体 —— 前端更难用，后端更容易错。
//!
//! > `buildNodes`（单串节点）**不单独暴露** —— 源项目生产代码没调它，
//! > 它只是 `buildEquation` 的内部实现。需要单串时用 `math_layout` 取 `lines[0].nodes`。
//!
//! ## 降级约定（**前端必读**）
//!
//! 排版失败**不报错**，而是：
//!
//! - `math_layout`：坏段不进 `lines`，但记进 `warnings`
//! - `math_layout_equation`：返回 `null`
//!
//! 两种情况下**前端都必须回落成等宽原文展示** —— 绝不静默丢内容。
//! 判据：`mathLayout.lines.length > 0` / `equationNodes !== null`。
//!
//! ## 无状态
//!
//! 三个命令都是纯函数（不碰数据库 / 偏好），因此不接收 `State`。

use crate::error::CmdResult;
use civilcalc_core::display::math_layout::{self, MathLayout, MathNode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 「符号 → 数值文本」对（结果行的入参）。
///
/// 用结构体而不是 `[string, string]` 元组：IPC 上可读性好得多，
/// 且前端能按名字取值而不是按下标。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultPair {
    /// 结果符号（如 `x`）
    pub symbol: String,
    /// 数值文本 —— **由调用方格式化**（排版层不管精度口径）
    pub value: String,
}

/// 把整条公式（含分号分段）排成二维布局树。
///
/// 多结果公式会排成多行，每行的 `symbol` 是分号段的赋值目标。
///
/// @param constants schema 自定义常量（如 `{"alpha": 1.5}`）；
///                  不传时只用内置的 `pi` / `e`
#[tauri::command]
pub fn math_layout(
    expression: String,
    constants: Option<HashMap<String, f64>>,
) -> CmdResult<MathLayout> {
    let constants = constants.unwrap_or_default();
    Ok(math_layout::build_with_constants(&expression, &constants))
}

/// 把用户写的**条件方程**排成 `左侧 = 右侧`。
///
/// ⚠️ 等式不是赋值语句（`=` 不在开头），所以先按**第一个** `=` 切两半再各自排版。
///
/// 下列情况返回 `null`（前端回落原文）：
/// - 没有 `=`、`=` 在开头（`=6`）、`=` 在结尾（`x+y=`）
/// - 任一侧解析失败（如 `x+y+z=??`）
#[tauri::command]
pub fn math_layout_equation(
    equation: String,
    constants: Option<HashMap<String, f64>>,
) -> CmdResult<Option<Vec<MathNode>>> {
    let constants = constants.unwrap_or_default();
    Ok(math_layout::build_equation_with_constants(&equation, &constants))
}

/// 把结果排成一行：多结果 `(x, y, z) = (1, 2, 3)`；单结果不带括号 `V = 31.4159`。
///
/// `results` 为空时返回空数组。
#[tauri::command]
pub fn math_layout_result_line(results: Vec<ResultPair>) -> CmdResult<Vec<MathNode>> {
    let pairs: Vec<(String, String)> = results
        .into_iter()
        .map(|r| (r.symbol, r.value))
        .collect();
    Ok(math_layout::build_result_line(&pairs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use civilcalc_core::display::math_layout::TextKind;

    fn texts(nodes: &[MathNode]) -> Vec<String> {
        nodes
            .iter()
            .filter_map(|n| match n {
                MathNode::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    // ============================================================ math_layout

    #[test]
    fn layout_single_expression() {
        let r = math_layout("x = a/b".to_string(), None).unwrap();
        assert_eq!(r.lines.len(), 1);
        assert_eq!(r.lines[0].symbol.as_deref(), Some("x"));
        assert!(matches!(r.lines[0].nodes[0], MathNode::Fraction { .. }));
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn layout_multi_output_gives_multiple_lines() {
        let r = math_layout("x = a+b; y = a-b".to_string(), None).unwrap();
        assert_eq!(r.lines.len(), 2);
        assert!(r.is_multi());
        assert_eq!(
            r.lines
                .iter()
                .map(|l| l.symbol.as_deref().unwrap_or(""))
                .collect::<Vec<_>>(),
            ["x", "y"]
        );
    }

    /// 排版失败**不报错**，而是空 lines + warnings（前端据此回落原文）
    #[test]
    fn layout_failure_is_warning_not_error() {
        let r = math_layout("x = a@b".to_string(), None).unwrap();
        assert!(r.lines.is_empty());
        assert!(!r.warnings.is_empty());
        assert!(!r.is_usable());
    }

    #[test]
    fn layout_empty_expression_is_warning() {
        let r = math_layout(String::new(), None).unwrap();
        assert!(r.lines.is_empty());
        assert!(!r.warnings.is_empty());
    }

    /// 自定义常量参与排版（`alpha` 被识别为常量而不是变量）
    #[test]
    fn layout_accepts_custom_constants() {
        let constants = HashMap::from([("alpha".to_string(), 1.5)]);
        let r = math_layout("x = alpha*b".to_string(), Some(constants)).unwrap();
        assert!(r.is_usable(), "自定义常量应能排版: {:?}", r.warnings);
    }

    /// 部分失败：好的段落保留
    #[test]
    fn layout_partial_failure_keeps_good_lines() {
        let r = math_layout("x = a+b; y = a@b".to_string(), None).unwrap();
        assert_eq!(r.lines.len(), 1);
        assert_eq!(r.warnings.len(), 1);
        assert!(r.warnings[0].contains("第 2 段"), "实际 {:?}", r.warnings);
    }

    // ============================================================ equation

    #[test]
    fn equation_renders_both_sides() {
        let nodes = math_layout_equation("x+y+z=6".to_string(), None)
            .unwrap()
            .expect("应能排版");
        assert_eq!(texts(&nodes), ["x", " + ", "y", " + ", "z", " = ", "6"]);
    }

    /// 教材写法 `2x` 要先补隐式乘号
    #[test]
    fn equation_inserts_implicit_multiplication() {
        let nodes = math_layout_equation("2x-y=3".to_string(), None)
            .unwrap()
            .expect("应能排版");
        assert_eq!(texts(&nodes), ["2", "×", "x", " − ", "y", " = ", "3"]);
    }

    #[test]
    fn equation_rejects_malformed_input() {
        for bad in ["x+y+z", "=6", "x+y+z=", "x+y+z=??", ""] {
            assert!(
                math_layout_equation(bad.to_string(), None).unwrap().is_none(),
                "`{bad}` 应返回 None（前端回落原文）"
            );
        }
    }

    // ============================================================ result line

    #[test]
    fn result_line_multi_uses_parentheses() {
        let nodes = math_layout_result_line(vec![
            ResultPair {
                symbol: "x".to_string(),
                value: "1".to_string(),
            },
            ResultPair {
                symbol: "y".to_string(),
                value: "2".to_string(),
            },
        ])
        .unwrap();
        assert_eq!(texts(&nodes), ["(", "x", ", ", "y", ")", " = ", "(", "1", ", ", "2", ")"]);
    }

    #[test]
    fn result_line_single_has_no_parentheses() {
        let nodes = math_layout_result_line(vec![ResultPair {
            symbol: "V".to_string(),
            value: "31.4159".to_string(),
        }])
        .unwrap();
        assert_eq!(texts(&nodes), ["V", " = ", "31.4159"]);
    }

    #[test]
    fn result_line_empty_returns_empty() {
        assert!(math_layout_result_line(Vec::new()).unwrap().is_empty());
    }

    /// 数值文本**原样透传**（排版层不管精度）
    #[test]
    fn result_line_passes_value_text_through() {
        let nodes = math_layout_result_line(vec![ResultPair {
            symbol: "V".to_string(),
            value: "1.000000".to_string(),
        }])
        .unwrap();
        assert!(texts(&nodes).contains(&"1.000000".to_string()));
    }

    // ============================================================ serde

    #[test]
    fn math_layout_serde_shape() {
        let r = math_layout("x = a/b".to_string(), None).unwrap();
        let v = serde_json::to_value(&r).unwrap();
        assert!(v.get("lines").is_some());
        assert!(v.get("warnings").is_some());
        // 方法不进序列化 —— 前端用 lines.length 判可用性
        assert!(v.get("isUsable").is_none());
    }

    #[test]
    fn math_node_serde_uses_type_tag() {
        let nodes = math_layout_result_line(vec![ResultPair {
            symbol: "x".to_string(),
            value: "1".to_string(),
        }])
        .unwrap();
        let v = serde_json::to_value(&nodes).unwrap();
        let first = &v[0];
        assert_eq!(first["type"], serde_json::json!("text"));
        assert_eq!(first["text"], serde_json::json!("x"));
        assert_eq!(first["kind"], serde_json::json!("symbol"));
    }

    #[test]
    fn result_pair_deserializes_from_camel_case() {
        let p: ResultPair = serde_json::from_str(r#"{"symbol":"x","value":"1"}"#).unwrap();
        assert_eq!(p.symbol, "x");
        assert_eq!(p.value, "1");
    }

    /// 前端可能直接传 `kind` 为字符串的节点（回传场景）
    #[test]
    fn text_kind_serde_is_lowercase() {
        let v = serde_json::to_value(TextKind::Operator).unwrap();
        assert_eq!(v, serde_json::json!("operator"));
        let back: TextKind = serde_json::from_str("\"number\"").unwrap();
        assert_eq!(back, TextKind::Number);
    }
}
