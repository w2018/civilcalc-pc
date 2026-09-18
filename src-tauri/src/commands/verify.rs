//! 组 12c：方程代入验算（1 个命令）。
//!
//! | 命令 | 入参 | 返回 |
//! |---|---|---|
//! | `verify_equations` | `sourceEquations`, `inputs` | `EquationCheck[]` |
//!
//! ## 验算不经模型
//!
//! 全部由本地表达式引擎完成 —— 因此结论可以直接当依据用
//! （这是本项目与「让 AI 判断答案对不对」的根本区别）。
//!
//! ## 🔴 三种结果，前端不要混淆
//!
//! | `holds` | 含义 | 前端表现 |
//! |---|---|---|
//! | `true` | 两侧数值相等（按 4 位小数容差） | ✓ |
//! | `false` | 明确不相等 | ✗ |
//! | `null` | **无法核对**（不是不成立） | 显示 `note` 里的原因 |
//!
//! `null` 的三种来源：不是完整等式、缺取值、两侧无法解析。
//! **绝不静默丢掉一条方程** —— 宁可显示「未核对」。
//!
//! ## 无状态
//!
//! 纯函数（不碰数据库 / 偏好），因此不接收 `State`。

use crate::error::CmdResult;
use civilcalc_core::verify::equation_verifier::{self, EquationCheck};
use std::collections::HashMap;

/// 逐条代入核对原始方程。
///
/// @param source_equations 用户需求里给出的原始方程（`schema.sourceEquations`）；
///                         空白项会被跳过，序号按非空项连续编号
/// @param inputs 当前取值（用户填的系数 + 本次算出的结果量）
#[tauri::command]
pub fn verify_equations(
    source_equations: Vec<String>,
    inputs: HashMap<String, f64>,
) -> CmdResult<Vec<EquationCheck>> {
    Ok(equation_verifier::verify(&source_equations, &inputs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vals(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    fn eqs(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn verifies_system_of_equations() {
        let checks = verify_equations(
            eqs(&["x+y+z=6", "2x-y+z=3", "x+2y-z=2"]),
            vals(&[("x", 1.0), ("y", 2.0), ("z", 3.0)]),
        )
        .unwrap();

        assert_eq!(checks.len(), 3);
        assert!(checks.iter().all(|c| c.holds == Some(true)));
        assert_eq!(checks[0].mark, "①");
        assert_eq!(checks[1].left_text, "2*1-2+3");
    }

    #[test]
    fn detects_wrong_value() {
        let checks = verify_equations(eqs(&["x+y=3"]), vals(&[("x", 1.0), ("y", 5.0)])).unwrap();
        assert_eq!(checks[0].holds, Some(false));
    }

    #[test]
    fn missing_value_is_unchecked_with_note() {
        let checks = verify_equations(eqs(&["x+y+z=6"]), vals(&[("x", 1.0), ("y", 2.0)])).unwrap();
        assert_eq!(checks[0].holds, None);
        assert!(checks[0].note.as_deref().unwrap_or("").contains('z'));
        assert_eq!(checks[0].left_text, "1+2+z");
    }

    #[test]
    fn non_equation_is_unchecked() {
        let checks = verify_equations(eqs(&["x>1"]), vals(&[("x", 1.0)])).unwrap();
        assert_eq!(checks[0].holds, None);
        assert!(checks[0].note.as_deref().unwrap_or("").contains("等式"));
    }

    #[test]
    fn empty_input_yields_empty_result() {
        assert!(verify_equations(Vec::new(), vals(&[("x", 1.0)]))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn blank_items_skipped() {
        let checks = verify_equations(eqs(&["", "x=1", "  "]), vals(&[("x", 1.0)])).unwrap();
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].index, 1);
    }

    /// 结果可直接序列化为 IPC 形状（camelCase，`null` 保留）
    #[test]
    fn check_serde_shape() {
        let checks = verify_equations(eqs(&["x+y=3", "a+b=1"]), vals(&[("x", 1.0), ("y", 2.0)]))
            .unwrap();
        let v = serde_json::to_value(&checks).unwrap();

        assert_eq!(v[0]["holds"], serde_json::json!(true));
        assert_eq!(v[1]["holds"], serde_json::json!(null));
        assert!(v[1]["note"].is_string(), "无法核对时必须有 note");
        assert!(v[0].get("leftText").is_some());
        assert!(v[0].get("left_text").is_none(), "不得泄漏 snake_case");
    }
}
