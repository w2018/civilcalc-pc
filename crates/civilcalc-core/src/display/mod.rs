//! 二维数学排版（产出布局树，前端渲染）。
//!
//! 源：`civilcalc-android-v2/core/formula/display/MathLayout.kt`（243 行）
//!
//! ## 设计（ADR-019）
//!
//! - 本模块**只产出布局树（纯数据），不做绘制** —— 文本测量交给浏览器
//! - 一期支持核心形态：行内表达式 / 分式 / 上下标 / 根号 / 绝对值 / 函数调用 / 分组
//! - **解析失败的写法回落原文展示**（不报错、不空白 —— 源项目既有行为）
//!
//! ## 节点类型（按源项目实际源码对齐）
//!
//! | 节点 | 用途 | 触发写法 |
//! |---|---|---|
//! | `Text` | 数字 / 符号 / 运算符 / 函数名 | 任意 |
//! | `Fraction` | 分式 | `a/b` |
//! | `Superscript` | 上标（幂） | `h^2`、`pow(b,2)` |
//! | `Radical` | 根号（`index` 非空为 n 次根） | `sqrt(x)`、`cbrt(x)` |
//! | `Absolute` | 绝对值 | `abs(d)` |
//! | `Function` | 函数调用 | `sin(t)`、`log10(v)` |
//! | `Group` | 括号分组 | —（保留位，一期未产出） |
//!
//! > ⚠️ 本文件原先把节点列为 `Row` / `Subscript` / `Sqrt` / `Abs` / `BraceGroup`，
//! > 那是**规划时看图猜的**，与源项目实际不符。此处按实际源码对齐。
//! > 特别地：**`Subscript` 不是独立节点** —— 下标是 [`pretty_symbol`]
//! > 在**文本层**做的 Unicode 替换（`a11` → `a₁₁`），渲染层只管画字符串。
//!
//! ## 两个容易踩的点
//!
//! 1. **排版前必须先补隐式乘号**：`2x` 不补成 `2*x` 会解析失败，只能降级成原文。
//! 2. **`=` 的两种语义**：`x = …`（赋值，由 `splitter` 剥离）与 `x+y=6`
//!    （用户写的条件方程，不是赋值）。后者走 [`math_layout::build_equation`]，
//!    按**第一个** `=` 切两半再各自排版。

pub mod math_layout;

pub use math_layout::{
    build as build_math_layout,
    build_equation as build_math_equation,
    build_equation_with_constants as build_math_equation_with_constants,
    build_nodes as build_math_nodes,
    build_nodes_with_constants as build_math_nodes_with_constants,
    build_result_line as build_math_result_line,
    build_with_constants as build_math_layout_with_constants,
    pretty_symbol as pretty_math_symbol,
    MathLayout, MathLine, MathNode, TextKind,
};
