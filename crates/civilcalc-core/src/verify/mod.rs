//! 方程代入验算（本地逐条核对，**不靠模型判断**）。
//!
//! 源：`civilcalc-android-v2/core/formula/verify/EquationVerifier.kt`（203 行）
//!
//! 输入 `schema.sourceEquations`（用户需求里给出的原始方程，AI 原样抄录），
//! 逐条代入当前取值核对，输出 ✓ / ✗ 或标出缺哪个取值。
//!
//! ## 三种结果，不要混淆
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
//! ## 验算不经模型
//!
//! 全部由本地表达式引擎完成，因此结论可以直接当依据用。

pub mod equation_verifier;

pub use equation_verifier::{
    mark_of, verify as verify_equations, EquationCheck,
};
