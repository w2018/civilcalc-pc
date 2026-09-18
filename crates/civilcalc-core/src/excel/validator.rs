//! Excel 公式校验器：比对 **AI 生成的 Excel 公式** 与 **本地转换结果** 是否等价。
//!
//! 源：`ExcelFormulaValidator.kt`（338 行）
//!
//! ## 为什么需要它
//!
//! AI 返回的 `excelExpression` 是**自由文本**，可能：
//! - 函数名写错（`SQR` 而不是 `SQRT`）
//! - 单元格引用错位（把 `a11` 写成 `A2`）
//! - 多结果时只给了最后一段
//!
//! 直接把 AI 的公式写进 Excel，用户会拿到一份**算错的工程计算书** —— 而且看不出来。
//! 所以导出前必须校验：**用同一组样本输入，分别跑「本地转换结果」与「AI 公式」，
//! 比较相对误差**。误差超过 `1e-9` 就判定不一致。
//!
//! ## 校验链路
//!
//! ```text
//! 样本输入（固定种子）
//!    ├─ 本地侧：converter.convert(schema, 样本) → valueFormula → 回译 → 本地引擎求值
//!    └─ AI 侧：  AI excelExpression → 单元格引用换成变量名 → 回译 → 本地引擎求值
//!                              ↓
//!                        逐组比相对误差
//! ```
//!
//! ⚠️ 两侧都要**回译成同一个引擎的语言**再比 —— 不能拿 Excel 语义和本地语义直接比。
//!
//! ## 三处与源项目的关键差异（都已处理）
//!
//! | # | 差异 | 处理 |
//! |---|---|---|
//! | 1 | **Rust 的 `regex` 不支持 lookaround**，源项目的 `(?<!…)`/`(?=…)` 用不了 | 手写 [`replace_whole_word`]（单趟构建输出，不边扫边改） |
//! | 2 | **Java 的 `java.util.Random` 与 Rust 随机库不同** | 复刻 Java LCG（[`JavaRandom`]），保证**跨端样本逐位一致** |
//! | 3 | 源项目用 `Regex.escape` 转义 needle | needle 全是 ASCII 字面量，用 `str::starts_with` 天然安全 |
//!
//! ## 关于「跨端样本一致」（第 2 条的理由）
//!
//! 源项目用固定种子 `0x5EED0000` 消除随机误报（BUG-12）。PC 端若用 Rust 的随机库，
//! 种子虽固定，但**生成的样本序列与 Android 端不同** ——
//! 于是同一 schema 在两端得出不同结论时，无法判断是「实现有差异」还是「只是样本不同」。
//!
//! 复刻 Java LCG 只需 15 行，换来的是**跨端可交叉验证**。值。

use super::converter::{split_output_formula, ExcelFormulaConverter};
use crate::engine::expr_utils::{replace_whole_word, AfterRule};
use crate::engine::{self, types::CompileResult};
use crate::schema::FormulaSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// 默认抽样组数（源项目默认值）
pub const DEFAULT_SAMPLE_COUNT: usize = 3;

/// 相对误差阈值。超过即判定两侧不等价。
///
/// 取 `1e-9` 而不是 `f64::EPSILON`：两侧的求值顺序可能不同（浮点不满足结合律），
/// 需要留一点余量；但也不能太松，否则会放过真实的公式错误。
pub const RELATIVE_ERROR_THRESHOLD: f64 = 1e-9;

/// 样本输入种子（BUG-12：固定种子 → 同一 schema 校验结果确定可复现）
const SAMPLE_SEED: i64 = 0x5EED_0000;

/// 校验结果。
///
/// 字段命名对齐源项目，便于对照日志排查。
/// 默认值即「不通过且无说明」—— 所有字段的 `Default` 恰好是失败态，
/// 因此结构体字面量里可以放心用 `..Default::default()`。
///
/// 实现 `Serialize` 是为了直接作为 IPC 返回值（`excel_validate` 命令），
/// 无需在命令层再定义一个镜像结构 —— 少一层就容易少一处漏字段。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationResult {
    /// 是否通过
    pub ok: bool,
    /// 不通过时的说明（通过时为 `None`）
    pub message: Option<String>,
    /// 本地转换出的 Excel 公式（基准）
    pub local_formula: Option<String>,
    /// AI 给出的 Excel 公式
    pub ai_formula: Option<String>,
    /// 本次使用的样本输入（排障用：可据此手工复算）
    pub sample_inputs: Vec<BTreeMap<String, f64>>,
}

impl ValidationResult {
    fn fail(message: impl Into<String>) -> Self {
        Self {
            message: Some(message.into()),
            ..Default::default()
        }
    }
}

// =============================================================================
// 公开入口
// =============================================================================

/// 校验主公式：AI 的 `excelExpression` 是否与 `expression` 等价（抽样 3 组）。
pub fn validate(schema: &FormulaSchema) -> ValidationResult {
    validate_with_samples(schema, DEFAULT_SAMPLE_COUNT)
}

/// 校验主公式（自定义抽样组数）。
pub fn validate_with_samples(schema: &FormulaSchema, sample_count: usize) -> ValidationResult {
    let Some(ai_excel) = schema.excel_expression.as_deref() else {
        return ValidationResult::fail("schema 中 excelExpression 为空");
    };

    // 本地转换作为基准
    let converter = ExcelFormulaConverter::new();
    let local = converter.convert(schema, &HashMap::new());
    let Some(local_formula) = local.cell_formulas.get("主公式").cloned() else {
        return ValidationResult::fail("本地转换失败");
    };

    let constants = merged_constants(schema);

    // 多结果：AI 侧与本地侧都按分号段逐条比对。
    //
    // AI 整条以「符号 =」开头，不能再按「必须以 = 开头」判 ——
    // 旧实现会把它直接误判为格式错误。
    //
    // 判定条件放宽到「schema 声明了多个输出」：这样 AI 只给一段时
    // 也能报出**段数不一致**，而不是含混的格式错误。
    let ai_outputs = split_output_formula(ai_excel);
    let is_multi_output =
        !ai_outputs.is_empty() || !local.output_formulas.is_empty() || schema.result_outputs.len() > 1;

    if is_multi_output {
        let local_outputs = &local.output_formulas;
        if local_outputs.len() != ai_outputs.len() {
            return ValidationResult {
                message: Some(format!(
                    "多结果公式段数不一致: local={}, ai={}",
                    local_outputs.len(),
                    ai_outputs.len()
                )),
                local_formula: Some(local_formula),
                ai_formula: Some(ai_excel.to_string()),
                ..Default::default()
            };
        }

        let samples = generate_sample_inputs(schema, sample_count);
        for (idx, (symbol, ai_segment)) in ai_outputs.iter().enumerate() {
            for (sample_idx, sample) in samples.iter().enumerate() {
                let param_values: HashMap<String, String> =
                    sample.iter().map(|(k, v)| (k.clone(), v.to_string())).collect();
                let local_val = eval_output_with_local_converter(schema, &constants, idx, &param_values);
                let ai_val = eval_ai_excel(schema, &constants, ai_segment, sample);

                let (Some(lv), Some(av)) = (local_val, ai_val) else {
                    return ValidationResult {
                        message: Some(format!(
                            "结果「{symbol}」第{}组求值失败: local={}, ai={}",
                            sample_idx + 1,
                            fmt_opt(local_val),
                            fmt_opt(ai_val)
                        )),
                        local_formula: Some(local_formula),
                        ai_formula: Some(ai_excel.to_string()),
                        sample_inputs: samples,
                        ..Default::default()
                    };
                };

                let rel_err = relative_error(lv, av);
                if rel_err > RELATIVE_ERROR_THRESHOLD {
                    return ValidationResult {
                        message: Some(format!(
                            "结果「{symbol}」第{}组不一致: local={lv:?}, ai={av:?}, relErr={rel_err:?}",
                            sample_idx + 1
                        )),
                        local_formula: Some(local_formula),
                        ai_formula: Some(ai_excel.to_string()),
                        sample_inputs: samples,
                        ..Default::default()
                    };
                }
            }
        }

        return ValidationResult {
            ok: true,
            local_formula: Some(local_formula),
            ai_formula: Some(ai_excel.to_string()),
            sample_inputs: samples,
            ..Default::default()
        };
    }

    // 单结果：检查基本格式
    if !ai_excel.starts_with('=') {
        return ValidationResult {
            message: Some("AI Excel 公式未以 = 开头".to_string()),
            local_formula: Some(local_formula),
            ai_formula: Some(ai_excel.to_string()),
            ..Default::default()
        };
    }

    let samples = generate_sample_inputs(schema, sample_count);
    for (idx, sample) in samples.iter().enumerate() {
        let local_val = eval_with_local_converter(schema, &constants, sample);
        let ai_val = eval_ai_excel(schema, &constants, ai_excel, sample);

        let (Some(lv), Some(av)) = (local_val, ai_val) else {
            return ValidationResult {
                message: Some(format!(
                    "第{}组求值失败: local={}, ai={}",
                    idx + 1,
                    fmt_opt(local_val),
                    fmt_opt(ai_val)
                )),
                local_formula: Some(local_formula),
                ai_formula: Some(ai_excel.to_string()),
                sample_inputs: samples,
                ..Default::default()
            };
        };

        let rel_err = relative_error(lv, av);
        if rel_err > RELATIVE_ERROR_THRESHOLD {
            return ValidationResult {
                message: Some(format!(
                    "第{}组结果不一致: local={lv:?}, ai={av:?}, relErr={rel_err:?}",
                    idx + 1
                )),
                local_formula: Some(local_formula),
                ai_formula: Some(ai_excel.to_string()),
                sample_inputs: samples,
                ..Default::default()
            };
        }
    }

    ValidationResult {
        ok: true,
        local_formula: Some(local_formula),
        ai_formula: Some(ai_excel.to_string()),
        sample_inputs: samples,
        ..Default::default()
    }
}

/// 校验分步：AI 的 `excelStepsTemplate` 是否与 `stepsTemplate` 等价。
pub fn validate_steps(schema: &FormulaSchema) -> ValidationResult {
    validate_steps_with_samples(schema, DEFAULT_SAMPLE_COUNT)
}

/// 校验分步（自定义抽样组数）。
pub fn validate_steps_with_samples(schema: &FormulaSchema, sample_count: usize) -> ValidationResult {
    let Some(ai_steps) = schema.excel_steps_template.as_ref() else {
        return ValidationResult::fail("schema 中 excelStepsTemplate 为空");
    };
    let Some(local_steps) = schema.steps_template.as_ref() else {
        return ValidationResult::fail("schema 中 stepsTemplate 为空");
    };

    if ai_steps.len() != local_steps.len() {
        return ValidationResult::fail(format!(
            "分步数量不一致: local={}, ai={}",
            local_steps.len(),
            ai_steps.len()
        ));
    }

    let constants = merged_constants(schema);
    let samples = generate_sample_inputs(schema, sample_count);

    for (local_step, ai_step) in local_steps.iter().zip(ai_steps.iter()) {
        for (sample_idx, sample) in samples.iter().enumerate() {
            let local_val = eval_step_expression(&constants, &local_step.expression, sample);
            let ai_val = eval_step_expression(&constants, &ai_step.expression, sample);

            // ⚠️ 这里是 `continue` 不是 `return`：单步求值不出来（如引用了前序步骤的符号）
            //    不应让整次校验失败 —— 源项目行为。
            let (Some(lv), Some(av)) = (local_val, ai_val) else {
                continue;
            };

            let rel_err = relative_error(lv, av);
            if rel_err > RELATIVE_ERROR_THRESHOLD {
                return ValidationResult {
                    message: Some(format!(
                        "步骤'{}'第{}组不一致: local={lv:?}, ai={av:?}",
                        local_step.label,
                        sample_idx + 1
                    )),
                    sample_inputs: samples,
                    ..Default::default()
                };
            }
        }
    }

    ValidationResult {
        ok: true,
        sample_inputs: samples,
        ..Default::default()
    }
}

// =============================================================================
// 样本生成
// =============================================================================

/// 生成样本输入（固定种子 → 结果可复现）。
///
/// ⚠️ **循环顺序是「外层组数、内层变量」** —— 与源项目一致。
/// 反过来会让随机数消耗顺序不同，样本序列全变。
///
/// `min` / `max` 缺省为 `1.0` / `100.0`（源项目行为）。
/// 若用户把 `min > max` 或 `min == max` 写错，这里不拦（源项目也不拦）——
/// 校验器不是参数校验器，坏区间最多让样本退化，不会误判正确公式。
fn generate_sample_inputs(schema: &FormulaSchema, count: usize) -> Vec<BTreeMap<String, f64>> {
    let mut rng = JavaRandom::new(SAMPLE_SEED);
    let mut samples = Vec::with_capacity(count);

    for _ in 0..count {
        let mut sample = BTreeMap::new();
        for v in &schema.variables {
            let min = v.min.unwrap_or(1.0);
            let max = v.max.unwrap_or(100.0);
            sample.insert(v.symbol.clone(), min + rng.next_double() * (max - min));
        }
        samples.push(sample);
    }

    samples
}

/// `java.util.Random` 的 48 位 LCG。
///
/// 复刻动机见模块文档「关于跨端样本一致」。算法来自 JDK：
///
/// ```text
/// seed = (seed ^ 0x5DEECE66D) & ((1 << 48) - 1)
/// next(bits): seed = (seed * 0x5DEECE66D + 0xB) & ((1 << 48) - 1)
///             return (int)(seed >>> (48 - bits))
/// nextDouble(): ((long)next(26) << 27) + next(27)  再乘 2^-53
/// ```
///
/// 用 `u64` 承载：Java 的 `long` 乘法溢出后取低 48 位，与 `wrapping_mul` 的位模式一致。
#[derive(Debug, Clone)]
pub struct JavaRandom {
    seed: u64,
}

impl JavaRandom {
    const MULTIPLIER: u64 = 0x5DEECE66D;
    const ADDEND: u64 = 0xB;
    const MASK: u64 = (1u64 << 48) - 1;

    pub fn new(seed: i64) -> Self {
        Self {
            seed: ((seed as u64) ^ Self::MULTIPLIER) & Self::MASK,
        }
    }

    /// `next(bits)`：返回高 `bits` 位（`bits <= 32`）。
    pub fn next(&mut self, bits: u32) -> i32 {
        debug_assert!((1..=32).contains(&bits), "bits 必须在 1..=32");
        self.seed = self
            .seed
            .wrapping_mul(Self::MULTIPLIER)
            .wrapping_add(Self::ADDEND)
            & Self::MASK;
        // 右移后 <= 2^32，`as i32` 是符号重解释（与 Java 的 (int) 强转一致）
        (self.seed >> (48 - bits)) as i32
    }

    /// `nextDouble()`：`[0, 1)` 均匀分布。
    pub fn next_double(&mut self) -> f64 {
        let hi = i64::from(self.next(26)) << 27;
        let lo = i64::from(self.next(27));
        (hi + lo) as f64 / (1u64 << 53) as f64
    }
}

// =============================================================================
// 求值（两侧统一回译到本地引擎）
// =============================================================================

/// 合并常量表：内置（`pi`/`e`）+ schema 自定义。
///
/// 自定义**覆盖**内置同名常量（`HashMap::extend` 语义，与源项目的 `+` 一致）。
fn merged_constants(schema: &FormulaSchema) -> HashMap<String, f64> {
    let mut constants = crate::engine::function_table::constants_owned();
    constants.extend(schema.constants.iter().map(|(k, v)| (k.clone(), *v)));
    constants
}

/// 用本地转换器求值（单结果路径）。
fn eval_with_local_converter(
    schema: &FormulaSchema,
    constants: &HashMap<String, f64>,
    inputs: &BTreeMap<String, f64>,
) -> Option<f64> {
    let param_values: HashMap<String, String> =
        inputs.iter().map(|(k, v)| (k.clone(), v.to_string())).collect();
    let result = ExcelFormulaConverter::new().convert(schema, &param_values);
    let value_formula = result.value_formulas.get("主公式")?;
    eval_value_formula(constants, value_formula, inputs)
}

/// 多结果：取第 `index` 个输出、按该组样本代入后的本地公式求值。
fn eval_output_with_local_converter(
    schema: &FormulaSchema,
    constants: &HashMap<String, f64>,
    index: usize,
    param_values: &HashMap<String, String>,
) -> Option<f64> {
    let result = ExcelFormulaConverter::new().convert(schema, param_values);
    let value_formula = result.output_formulas.get(index)?.value_formula.clone();

    // 源项目这里把字符串参数值重新解析回 f64（解析失败按 0）
    let inputs: BTreeMap<String, f64> = param_values
        .iter()
        .map(|(k, v)| (k.clone(), v.parse::<f64>().unwrap_or(0.0)))
        .collect();

    eval_value_formula(constants, &value_formula, &inputs)
}

/// 值公式（已代入数值的 Excel 形态）→ 回译函数名 → 本地引擎求值。
fn eval_value_formula(
    constants: &HashMap<String, f64>,
    value_formula: &str,
    inputs: &BTreeMap<String, f64>,
) -> Option<f64> {
    // 值公式含 Excel 函数名（`EXP(1)` / `LOG10(…)`），同样需要边界回译
    let stripped = value_formula.strip_prefix('=').unwrap_or(value_formula);
    let expr = back_translate_excel_to_local(stripped);
    eval_local_expr(&expr, constants, inputs)
}

/// 用本地引擎求值一个表达式（不代入，变量从 `inputs` 取）。
fn eval_local_expr(
    expr: &str,
    constants: &HashMap<String, f64>,
    inputs: &BTreeMap<String, f64>,
) -> Option<f64> {
    // `eval_multi` 接收 `HashMap`；样本是 `BTreeMap`（顺序可复现），这里转一次
    let input_map: HashMap<String, f64> =
        inputs.iter().map(|(k, v)| (k.clone(), *v)).collect();

    match engine::compile_cached(expr, constants) {
        CompileResult::Ok { expr: compiled } => match engine::eval(&compiled, &input_map) {
            Ok(r) => Some(r.primary),
            Err(e) => {
                crate::log::w(
                    "ExcelValidator",
                    &format!("本地引擎求值失败: {expr} —— {e}"),
                    None,
                );
                None
            }
        },
        CompileResult::Error { position, reason } => {
            crate::log::w(
                "ExcelValidator",
                &format!("本地引擎编译失败: {expr}（位置 {position:?}）—— {reason}"),
                None,
            );
            None
        }
    }
}

/// 求值 AI 生成的 Excel 表达式：**单元格引用换成变量名** → 回译函数名 → 本地引擎求值。
fn eval_ai_excel(
    schema: &FormulaSchema,
    constants: &HashMap<String, f64>,
    excel_expr: &str,
    inputs: &BTreeMap<String, f64>,
) -> Option<f64> {
    let local = ExcelFormulaConverter::new().convert(schema, &HashMap::new());
    let mut ref_map: Vec<(String, String)> = local
        .param_mapping
        .iter()
        .map(|p| (p.cell_ref.clone(), p.variable.clone()))
        .collect();

    // ⚠️ **长引用优先**（源项目用 `sortedByDescending`）。
    //
    // 实测：后边界检查（`A1` 后面不得跟数字）**已经**能防住 `A1` 吃掉 `A10` 的前缀，
    // 所以这个排序在当前边界规则下是**冗余防御**。保留它是为了：
    // 1. 与源项目逐行对应，便于对照排查
    // 2. 万一将来放宽边界规则（如允许 `A1` 后跟数字），排序会成为最后一道防线
    //
    // 稳定排序保证同长度时顺序与变量顺序一致。
    ref_map.sort_by_key(|x| std::cmp::Reverse(x.0.len()));

    let mut expr = excel_expr.strip_prefix('=').unwrap_or(excel_expr).to_string();
    for (cell_ref, variable) in &ref_map {
        // 前边界额外排除 `$`：`$A$1` 这类绝对引用要先被识别成「不是独立引用」
        expr = replace_whole_word(
            &expr,
            cell_ref,
            variable,
            &ref_prev_is_word,
            &ref_next_is_word,
            AfterRule::NotWordChar,
        );
    }

    let expr = back_translate_excel_to_local(&expr);
    eval_local_expr(&expr, constants, inputs)
}

/// 求值单步表达式（先剥赋值前缀）。
fn eval_step_expression(
    constants: &HashMap<String, f64>,
    expr: &str,
    inputs: &BTreeMap<String, f64>,
) -> Option<f64> {
    let clean = engine::splitter::strip_assignment(expr);
    eval_local_expr(&clean, constants, inputs)
}

// =============================================================================
// Excel → 本地 回译（BUG-16）
// =============================================================================

/// Excel 函数名 → 本地函数名。
///
/// ⚠️ **顺序有意义**，不能改成 `HashMap`：
/// - `EXP(1)` 必须排在 `EXP` 之前（否则 `EXP(1)` 会先被换成 `exp(1)`）
/// - `LOG10` 排在 `LN` 之后无影响，但保持与源项目同序便于对照
///
/// 带 `()` 的项按**整词**匹配；不带括号的项要求**后跟 `(`**（避免把同名变量误换）。
const EXCEL_TO_LOCAL: &[(&str, &str)] = &[
    ("PI()", "pi"),
    ("EXP(1)", "e"),
    ("SQRT", "sqrt"),
    ("POWER", "pow"),
    ("LN", "ln"),
    ("LOG10", "log10"),
    ("EXP", "exp"),
    ("SIN", "sin"),
    ("COS", "cos"),
    ("TAN", "tan"),
    ("ASIN", "asin"),
    ("ACOS", "acos"),
    ("ATAN", "atan"),
    ("ABS", "abs"),
    ("MIN", "min"),
    ("MAX", "max"),
    ("FLOOR.MATH", "floor"),
    ("CEILING.MATH", "ceil"),
    ("ROUND", "round"),
];

/// Excel → 本地 回译（**整词 + 括号边界**）。
///
/// 源项目原来用无边界 `String.replace`，会误伤包含相同子串的内容（BUG-16）。
/// 这里要求：前边界不是 `[A-Za-z0-9_.]`，且（视项而定）后边界满足条件。
pub fn back_translate_excel_to_local(expr: &str) -> String {
    let mut result = expr.to_string();
    for (excel, local) in EXCEL_TO_LOCAL {
        let after = if excel.ends_with(')') {
            // `PI()` / `EXP(1)`：整词匹配，后面不能再接标识符字符
            AfterRule::NotWordChar
        } else {
            // `SQRT` / `ROUND`：跳过空白后必须紧跟 `(`
            // （lookahead 不消费括号 —— 替换串里**不要**补 `(`）
            AfterRule::OpenParen
        };
        result = replace_whole_word(&result, excel, local, &func_is_word, &func_is_word, after);
    }
    result
}

// =============================================================================
// 辅助
// =============================================================================

/// 相对误差：`|a-b| / max(|a|, |b|, 1e-12)`。
///
/// 用相对误差而非绝对误差：工程公式的量级跨度极大（`1e-6` 的应变量与 `1e6` 的轴力），
/// 绝对误差阈值无法同时适配。
///
/// `1e-12` 兜底避免两侧都是 0 时除零。
fn relative_error(a: f64, b: f64) -> f64 {
    let scale = a.abs().max(b.abs()).max(1e-12);
    (a - b).abs() / scale
}

/// 单元格引用的**前**边界：`[A-Za-z0-9_$.]`
///
/// 含 `$` 是为了让 `$A$1` 这类绝对引用里的 `A$1` 不被当成独立引用。
fn ref_prev_is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '$'
}

/// 单元格引用的**后**边界：`[A-Za-z0-9_]`（**不含** `.` / `$`）
fn ref_next_is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// 函数名回译的边界：`[A-Za-z0-9_.]`
///
/// 含 `.` 是为了 `FLOOR.MATH` —— 前边界排除 `.` 才不会把 `X.FLOOR.MATH(` 误换。
fn func_is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '.'
}

/// `Option<f64>` 的展示（对齐 Kotlin 字符串模板对 `null` 的输出）。
fn fmt_opt(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("{x:?}"),
        None => "null".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{FormulaSource, FormulaVar, ResultOutput, SourceKind, StepTemplate};

    // ============================================================ 测试助手

    fn var(symbol: &str, min: f64, max: f64) -> FormulaVar {
        FormulaVar {
            symbol: symbol.to_string(),
            desc: symbol.to_string(),
            unit: Some(String::new()),
            default: None,
            min: Some(min),
            max: Some(max),
            required: true,
        }
    }

    fn ai_source() -> FormulaSource {
        FormulaSource {
            kind: SourceKind::Ai,
            ref_: None,
            verified: false,
            model: None,
            created_by: None,
        }
    }

    fn base_schema(expression: &str, symbols: &[&str]) -> FormulaSchema {
        FormulaSchema {
            id: "test:0".to_string(),
            result_name: "t".to_string(),
            result_symbol: "X".to_string(),
            result_unit: Some(String::new()),
            result_outputs: Vec::new(),
            expression: expression.to_string(),
            source_equations: Vec::new(),
            alt_expressions: Vec::new(),
            constants: HashMap::new(),
            variables: symbols.iter().map(|s| var(s, 2.0, 50.0)).collect(),
            domain: String::new(),
            tags: Vec::new(),
            reference_basis: None,
            design_notes: None,
            explanation: None,
            image_ids: Vec::new(),
            revised_from: None,
            source: ai_source(),
            steps_template: None,
            doc_template_id: None,
            excel_expression: None,
            excel_alt_expressions: None,
            excel_steps_template: None,
            excel_function_docs: None,
            schema_version: 3,
            created_at: 0,
            updated_at: 0,
        }
    }

    // ============================================================ JavaRandom

    /// LCG 实现必须与 JDK 一致 —— 用 `new Random(0).nextInt()` 的公开已知值锚定
    #[test]
    fn java_random_matches_jdk_known_value() {
        let mut r = JavaRandom::new(0);
        assert_eq!(r.next(32), -1155484576, "必须与 java.util.Random(0).nextInt() 一致");
    }

    /// 本校验器实际使用的种子（`0x5EED0000`）前 6 个 `nextDouble`
    ///
    /// 期望值由 JDK 算法独立算得（Python 复现），用于防止后续重构改坏样本序列。
    #[test]
    fn java_random_seed_0x5eed0000_sequence() {
        let mut r = JavaRandom::new(SAMPLE_SEED);
        let got: Vec<f64> = (0..6).map(|_| r.next_double()).collect();
        let want = [
            0.1056695280633917,
            0.7518121885709502,
            0.7083117889022367,
            0.9384983213378636,
            0.2628686094909668,
            0.7728962418688827,
        ];
        for (i, (g, w)) in got.iter().zip(want.iter()).enumerate() {
            assert!(
                (g - w).abs() < 1e-15,
                "第 {i} 个 nextDouble 不一致: got={g}, want={w}"
            );
        }
    }

    #[test]
    fn java_random_next_double_in_unit_range() {
        let mut r = JavaRandom::new(12345);
        for _ in 0..1000 {
            let v = r.next_double();
            assert!((0.0..1.0).contains(&v), "nextDouble 必须落在 [0,1)，实际 {v}");
        }
    }

    // ============================================================ 样本生成

    #[test]
    fn sample_inputs_are_deterministic() {
        let s = base_schema("a+b", &["a", "b"]);
        let first = generate_sample_inputs(&s, 3);
        let second = generate_sample_inputs(&s, 3);
        assert_eq!(first, second, "同种子必须产生同样本");
        assert_eq!(first.len(), 3);
    }

    /// 样本必须落在 `[min, max]` 内，且不同组取值不同
    #[test]
    fn sample_inputs_respect_bounds() {
        let s = base_schema("a+b", &["a", "b"]);
        let samples = generate_sample_inputs(&s, 5);
        for sample in &samples {
            for (k, v) in sample {
                assert!((2.0..=50.0).contains(v), "{k}={v} 越界");
            }
        }
        let a_values: Vec<f64> = samples.iter().map(|s| s["a"]).collect();
        assert!(
            a_values.windows(2).any(|w| (w[0] - w[1]).abs() > 1e-9),
            "各组样本不应完全相同"
        );
    }

    /// `min`/`max` 缺省为 1/100
    #[test]
    fn sample_inputs_use_default_bounds_when_absent() {
        let mut s = base_schema("a", &["a"]);
        s.variables[0].min = None;
        s.variables[0].max = None;
        for sample in generate_sample_inputs(&s, 5) {
            let v = sample["a"];
            assert!((1.0..=100.0).contains(&v), "缺省区间应为 [1,100]，实际 {v}");
        }
    }

    /// ⚠️ 变量顺序决定随机数消耗顺序，进而决定样本值
    #[test]
    fn sample_inputs_depend_on_variable_order() {
        let ab = generate_sample_inputs(&base_schema("a+b", &["a", "b"]), 1);
        let ba = generate_sample_inputs(&base_schema("a+b", &["b", "a"]), 1);
        assert!(
            (ab[0]["a"] - ba[0]["a"]).abs() > 1e-12,
            "变量顺序变了样本也该变（若相同说明随机数没按变量逐个消耗）"
        );
    }

    // ============================================================ 回译（BUG-16）

    #[test]
    fn back_translate_maps_all_table_entries() {
        assert_eq!(back_translate_excel_to_local("PI()*2"), "pi*2");
        assert_eq!(back_translate_excel_to_local("EXP(1)+1"), "e+1");
        assert_eq!(back_translate_excel_to_local("SQRT(A1)"), "sqrt(A1)");
        assert_eq!(back_translate_excel_to_local("POWER(A1,2)"), "pow(A1,2)");
        assert_eq!(back_translate_excel_to_local("LN(A1)"), "ln(A1)");
        assert_eq!(back_translate_excel_to_local("LOG10(A1)"), "log10(A1)");
        assert_eq!(back_translate_excel_to_local("EXP(A1)"), "exp(A1)");
        assert_eq!(back_translate_excel_to_local("SIN(A1)"), "sin(A1)");
        assert_eq!(back_translate_excel_to_local("COS(A1)"), "cos(A1)");
        assert_eq!(back_translate_excel_to_local("TAN(A1)"), "tan(A1)");
        assert_eq!(back_translate_excel_to_local("ASIN(A1)"), "asin(A1)");
        assert_eq!(back_translate_excel_to_local("ACOS(A1)"), "acos(A1)");
        assert_eq!(back_translate_excel_to_local("ATAN(A1)"), "atan(A1)");
        assert_eq!(back_translate_excel_to_local("ABS(A1)"), "abs(A1)");
        assert_eq!(back_translate_excel_to_local("MIN(A1,B1)"), "min(A1,B1)");
        assert_eq!(back_translate_excel_to_local("MAX(A1,B1)"), "max(A1,B1)");
        assert_eq!(
            back_translate_excel_to_local("FLOOR.MATH(A1)"),
            "floor(A1)"
        );
        assert_eq!(
            back_translate_excel_to_local("CEILING.MATH(A1)"),
            "ceil(A1)"
        );
        assert_eq!(back_translate_excel_to_local("ROUND(A1,0)"), "round(A1,0)");
    }

    /// `EXP(1)` 必须优先于 `EXP`（顺序敏感）
    #[test]
    fn exp_one_wins_over_exp() {
        assert_eq!(back_translate_excel_to_local("EXP(1)"), "e");
        // 若顺序反了会得到 `exp(1)`（数值等价，但语义不是「自然常数」）
        assert_eq!(back_translate_excel_to_local("EXP(A1)"), "exp(A1)");
    }

    /// 函数名后**不跟括号**时不得替换（那是变量名，不是函数调用）
    #[test]
    fn function_name_without_paren_is_not_replaced() {
        // `MAX` 是变量名，后面是 `+` 不是 `(`
        assert_eq!(back_translate_excel_to_local("MAX+A1"), "MAX+A1");
        // 后跟空白再跟括号仍算函数调用（源项目 `\s*\(`）
        assert_eq!(back_translate_excel_to_local("SQRT (A1)"), "sqrt (A1)");
    }

    /// BUG-16 核心：前边界必须挡住「长标识符的尾巴」
    #[test]
    fn back_translate_respects_leading_boundary() {
        // `XSQRT(` 里的 `SQRT` 不是独立函数名
        assert_eq!(back_translate_excel_to_local("XSQRT(A1)"), "XSQRT(A1)");
        assert_eq!(back_translate_excel_to_local("_SQRT(A1)"), "_SQRT(A1)");
        assert_eq!(back_translate_excel_to_local("A.SQRT(A1)"), "A.SQRT(A1)");
        // 但独立出现时要换
        assert_eq!(back_translate_excel_to_local("1+SQRT(A1)"), "1+sqrt(A1)");
        assert_eq!(back_translate_excel_to_local("(SQRT(A1))"), "(sqrt(A1))");
    }

    /// `FLOOR.MATH` 内含 `.`，前边界排除 `.` 才不会把 `X.FLOOR.MATH(` 误换
    #[test]
    fn floor_math_with_dot_boundary() {
        assert_eq!(
            back_translate_excel_to_local("FLOOR.MATH(A1)"),
            "floor(A1)"
        );
        assert_eq!(
            back_translate_excel_to_local("X.FLOOR.MATH(A1)"),
            "X.FLOOR.MATH(A1)"
        );
    }

    #[test]
    fn back_translate_leaves_unrelated_text_untouched() {
        assert_eq!(back_translate_excel_to_local("A1+B1*2"), "A1+B1*2");
        assert_eq!(back_translate_excel_to_local(""), "");
    }

    // ============================================================ validate：单结果

    #[test]
    fn validate_passes_when_ai_matches_local() {
        let mut s = base_schema("sqrt(a - 1) + b", &["a", "b"]);
        s.excel_expression = Some("=SQRT(A1-1)+B1".to_string());
        let r = validate(&s);
        assert!(r.ok, "应通过，实际: {:?}", r.message);
        assert!(r.message.is_none());
        assert_eq!(r.local_formula.as_deref(), Some("=SQRT(A1-1)+B1"));
        assert_eq!(r.ai_formula.as_deref(), Some("=SQRT(A1-1)+B1"));
        assert_eq!(r.sample_inputs.len(), 3);
    }

    /// BUG-12：固定种子 → 连续校验结果完全一致
    #[test]
    fn validate_is_deterministic() {
        let mut s = base_schema("sqrt(a - 1) + b", &["a", "b"]);
        s.excel_expression = Some("=SQRT(A1-1)+B1".to_string());

        let first = validate(&s);
        for _ in 0..100 {
            let r = validate(&s);
            assert_eq!(first.ok, r.ok);
            assert_eq!(first.message, r.message);
            assert_eq!(first.sample_inputs, r.sample_inputs);
        }
    }

    #[test]
    fn validate_rejects_missing_excel_expression() {
        let s = base_schema("a+b", &["a", "b"]);
        let r = validate(&s);
        assert!(!r.ok);
        assert_eq!(r.message.as_deref(), Some("schema 中 excelExpression 为空"));
    }

    #[test]
    fn validate_rejects_missing_leading_equals() {
        let mut s = base_schema("a+b", &["a", "b"]);
        s.excel_expression = Some("A1+B1".to_string());
        let r = validate(&s);
        assert!(!r.ok);
        assert_eq!(r.message.as_deref(), Some("AI Excel 公式未以 = 开头"));
    }

    /// AI 把变量引错位 → 必须检出
    #[test]
    fn validate_detects_wrong_cell_ref() {
        let mut s = base_schema("a+b", &["a", "b"]);
        // 本地是 =A1+B1；AI 写成 =A1+A1
        s.excel_expression = Some("=A1+A1".to_string());
        let r = validate(&s);
        assert!(!r.ok, "错位引用必须被检出");
        assert!(
            r.message.as_deref().unwrap().contains("不一致"),
            "实际: {:?}",
            r.message
        );
    }

    /// AI 把函数写错 → 必须检出
    #[test]
    fn validate_detects_wrong_function() {
        let mut s = base_schema("sqrt(a)+b", &["a", "b"]);
        // 本地 =SQRT(A1)+B1；AI 用 ABS 冒充
        s.excel_expression = Some("=ABS(A1)+B1".to_string());
        let r = validate(&s);
        assert!(!r.ok, "错误函数必须被检出");
    }

    /// BUG-16 端到端：混合 `EXP(1)`/`LOG10`/`FLOOR.MATH` 回译正确才通过
    #[test]
    fn bug16_mixed_function_back_translate() {
        let mut s = base_schema("e + log10(a) + floor(b)", &["a", "b"]);
        s.excel_expression = Some("=EXP(1)+LOG10(A1)+FLOOR.MATH(B1)".to_string());
        let r = validate(&s);
        assert!(r.ok, "回译失败: {:?}", r.message);
    }

    /// BUG-16 反向：回译不能误伤，导致把正确的 AI 公式判成错误
    #[test]
    fn bug16_no_boundary_misdamage() {
        let mut s = base_schema("e + log10(a) + floor(b)", &["a", "b"]);
        s.excel_expression = Some("=EXP(1)+LOG10(A1)+FLOOR.MATH(B1)".to_string());
        // 连跑多次（源项目同名测试即此意图）
        for _ in 0..20 {
            assert!(validate(&s).ok);
        }
    }

    /// 自定义常量参与求值（两侧一致才通过）
    #[test]
    fn validate_uses_schema_constants() {
        let mut s = base_schema("alpha*a", &["a"]);
        s.constants = HashMap::from([("alpha".to_string(), 1.5)]);
        s.excel_expression = Some("=1.5*A1".to_string());
        let r = validate(&s);
        assert!(r.ok, "自定义常量应参与校验: {:?}", r.message);
    }

    /// 常量为 0 → 样本恒为 0，两侧都应得 0（相对误差兜底不能误判）
    #[test]
    fn validate_handles_zero_result() {
        let mut s = base_schema("a*0", &["a"]);
        s.excel_expression = Some("=A1*0".to_string());
        let r = validate(&s);
        assert!(r.ok, "全零结果不应因除零误判: {:?}", r.message);
    }

    // ============================================================ validate：多结果

    /// 克莱默法则就地展开（不引用步骤符号）
    fn multi_schema(excel_expression: Option<&str>) -> FormulaSchema {
        let mut s = base_schema(
            "x = (b1*a22 - b2*a12)/(a11*a22 - a21*a12); y = (a11*b2 - a21*b1)/(a11*a22 - a21*a12)",
            &["a11", "a12", "a21", "a22", "b1", "b2"],
        );
        s.id = "usr:eq2x".to_string();
        s.result_name = "二元一次方程组解".to_string();
        s.result_symbol = "y".to_string();
        s.result_outputs = vec![
            ResultOutput {
                symbol: "x".to_string(),
                name: "未知数 x".to_string(),
                unit: String::new(),
            },
            ResultOutput {
                symbol: "y".to_string(),
                name: "未知数 y".to_string(),
                unit: String::new(),
            },
        ];
        s.excel_expression = excel_expression.map(str::to_string);
        s
    }

    /// 变量顺序即 A1..F1：a11,a12,a21,a22,b1,b2
    const CORRECT_AI_EXCEL: &str =
        "x = =(E1*D1-F1*B1)/(A1*D1-C1*B1); y = =(A1*F1-C1*E1)/(A1*D1-C1*B1)";

    /// MULTI-V01：每段与本地一致 → 通过（不再因为「不以 = 开头」被误判）
    #[test]
    fn multi_v01_segments_compared_one_by_one() {
        let r = validate(&multi_schema(Some(CORRECT_AI_EXCEL)));
        assert!(r.ok, "多结果校验应通过，实际: {:?}", r.message);
    }

    /// MULTI-V02：第二段整体写错 → 报出该结果的符号，便于定位
    #[test]
    fn multi_v02_mismatch_reported_with_symbol() {
        let broken =
            "x = =(E1*D1-F1*B1)/(A1*D1-C1*B1); y = =(A1*F1-C1*E1)/(A1*D1-C1*B1)*2";
        let r = validate(&multi_schema(Some(broken)));
        assert!(!r.ok, "写错的一段必须被检出");
        let msg = r.message.unwrap_or_default();
        assert!(msg.contains("结果「y」"), "告警应带结果符号: {msg}");
    }

    /// MULTI-V03：AI 少给一段 → 明确报段数不一致（不是静默只校最后一段）
    #[test]
    fn multi_v03_segment_count_mismatch_reported() {
        let r = validate(&multi_schema(Some(
            "x = =(E1*D1-F1*B1)/(A1*D1-C1*B1)",
        )));
        assert!(!r.ok);
        let msg = r.message.unwrap_or_default();
        assert!(msg.contains("段数不一致"), "应报段数不一致: {msg}");
    }

    /// `schema.resultOutputs.len() > 1` 也触发多结果路径 ——
    /// 即使 AI 只给了一段，也要报段数不一致（而不是走单结果路径报「未以 = 开头」）
    #[test]
    fn multi_output_triggered_by_declared_outputs() {
        let r = validate(&multi_schema(Some("=A1+B1")));
        assert!(!r.ok);
        let msg = r.message.unwrap_or_default();
        assert!(
            msg.contains("段数不一致"),
            "应由 resultOutputs 判定为多结果: {msg}"
        );
    }

    // ============================================================ validateSteps

    fn steps_schema(local: &[(&str, &str)], ai: Option<&[(&str, &str)]>) -> FormulaSchema {
        let mut s = base_schema("a+b", &["a", "b"]);
        s.steps_template = Some(
            local
                .iter()
                .map(|(label, expr)| StepTemplate {
                    symbol: label.to_string(),
                    label: (*label).to_string(),
                    group: None,
                    expression: (*expr).to_string(),
                    unit: String::new(),
                    note: None,
                })
                .collect(),
        );
        s.excel_steps_template = ai.map(|list| {
            list.iter()
                .map(|(label, expr)| StepTemplate {
                    symbol: label.to_string(),
                    label: (*label).to_string(),
                    group: None,
                    expression: (*expr).to_string(),
                    unit: String::new(),
                    note: None,
                })
                .collect()
        });
        s
    }

    #[test]
    fn validate_steps_passes_when_equivalent() {
        // 本地 `a+b` 与 Excel 形态 `A1+B1` 在把 A1/B1 视作变量名时无法直接比 ——
        // 但两侧都走同一个本地引擎，只要表达式形态一致就通过
        let s = steps_schema(&[("第一步", "a+b")], Some(&[("第一步", "a+b")]));
        let r = validate_steps(&s);
        assert!(r.ok, "应通过: {:?}", r.message);
    }

    #[test]
    fn validate_steps_rejects_missing_ai_steps() {
        let s = steps_schema(&[("第一步", "a+b")], None);
        let r = validate_steps(&s);
        assert!(!r.ok);
        assert_eq!(r.message.as_deref(), Some("schema 中 excelStepsTemplate 为空"));
    }

    #[test]
    fn validate_steps_rejects_missing_local_steps() {
        let mut s = steps_schema(&[("第一步", "a+b")], Some(&[("第一步", "a+b")]));
        s.steps_template = None;
        let r = validate_steps(&s);
        assert!(!r.ok);
        assert_eq!(r.message.as_deref(), Some("schema 中 stepsTemplate 为空"));
    }

    #[test]
    fn validate_steps_rejects_count_mismatch() {
        let s = steps_schema(
            &[("第一步", "a+b"), ("第二步", "a*b")],
            Some(&[("第一步", "a+b")]),
        );
        let r = validate_steps(&s);
        assert!(!r.ok);
        assert_eq!(r.message.as_deref(), Some("分步数量不一致: local=2, ai=1"));
    }

    #[test]
    fn validate_steps_detects_value_mismatch_with_label() {
        let s = steps_schema(&[("承载力", "a+b")], Some(&[("承载力", "a*b")]));
        let r = validate_steps(&s);
        assert!(!r.ok, "不同的表达式必须被检出");
        let msg = r.message.unwrap_or_default();
        assert!(msg.contains("步骤'承载力'"), "应带步骤名: {msg}");
    }

    /// 单步求值不出来时**跳过**（continue 而非 return）—— 源项目行为
    #[test]
    fn validate_steps_skips_unevaluable_steps() {
        // `@@` 两侧都编译失败 → 全部跳过 → 判定通过
        let s = steps_schema(&[("坏步骤", "@@")], Some(&[("坏步骤", "@@")]));
        let r = validate_steps(&s);
        assert!(r.ok, "求值失败应跳过而非判失败: {:?}", r.message);
    }

    /// 赋值前缀被剥掉（`V = a+b` 与 `V = a+b` 等价）
    #[test]
    fn validate_steps_strips_assignment_prefix() {
        let s = steps_schema(
            &[("第一步", "V = a+b")],
            Some(&[("第一步", "V = a + b")]),
        );
        let r = validate_steps(&s);
        assert!(r.ok, "赋值前缀应被剥离: {:?}", r.message);
    }

    // ============================================================ 辅助函数

    #[test]
    fn relative_error_is_scale_invariant() {
        assert_eq!(relative_error(100.0, 100.0), 0.0);
        assert!((relative_error(1e6, 1.000001e6) - 1e-6).abs() < 1e-12);
        // 同比例的小量级 → 同样的相对误差
        assert!((relative_error(1e-6, 1.000001e-6) - 1e-6).abs() < 1e-12);
        // 两侧都为 0 → 0（不除零）
        assert_eq!(relative_error(0.0, 0.0), 0.0);
        // 一侧为 0 → 误差 1.0
        assert!((relative_error(0.0, 5.0) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn fmt_opt_matches_kotlin_null_rendering() {
        assert_eq!(fmt_opt(None), "null");
        assert_eq!(fmt_opt(Some(3.0)), "3.0");
    }

    #[test]
    fn validation_result_default_is_failure() {
        let d = ValidationResult::default();
        assert!(!d.ok);
        assert!(d.message.is_none());
    }

    /// `merged_constants`：自定义覆盖内置同名常量
    #[test]
    fn merged_constants_override_builtin() {
        let mut s = base_schema("a", &["a"]);
        s.constants = HashMap::from([("pi".to_string(), 3.0)]);
        let c = merged_constants(&s);
        assert_eq!(c["pi"], 3.0, "自定义应覆盖内置 pi");
        assert_eq!(c["e"], std::f64::consts::E, "未覆盖的内置常量保留");
    }
}
