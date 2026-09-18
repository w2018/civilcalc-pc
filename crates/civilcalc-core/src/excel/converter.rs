//! Excel 公式转换：本地表达式 / RPN → Excel 公式。
//!
//! 源：`core/formula/export/ExcelFormulaConverter.kt`（627 行）
//!
//! ## 两条不可违反的红线
//!
//! 1. **数值代入必须用单趟词法扫描**（[`substitute_values`]）——
//!    **严禁 `str::replace` 循环**。源项目 P0 缺陷「Excel 数值代入不全」
//!    的根因就是 `replace` 循环：`A1` 替换成 `2` 之后，`A10` 里的 `A1`
//!    也被改掉，且替换结果本身可能再被后续规则命中。
//! 2. **取模输出 `MOD(a,b)`** —— Excel 的 `%` 是**后缀百分比运算符**，
//!    不是取模。直接用 `%` 会得到完全不同的结果。
//!
//! ## 单元格映射契约（schemaVersion = 3）
//!
//! | 对象 | 位置 |
//! |---|---|
//! | 第 n 个变量 | **第 1 行横向**：`A1` / `B1` / … / `Z1` / `AA1` |
//! | 步骤结果 | A 列纵向：第 i 步 → `A(i+2)` |
//!
//! 旧契约是纵向（`A1`,`A2`,…）。[`substitute_values`] 里有**兼容对齐**：
//! 遇到未在映射表里的第 1 行引用或 A 列引用，按「引用首现顺序 ↔ 未认领变量顺序」
//! 认领并告警 —— 兜住历史数据。
//!
//! ## 等号剥离
//!
//! 只用 [`splitter::strip_assignment`]，**不得** `replace("=", "")` ——
//! 那会破坏 `>=` / `<=` / `==`（BUG-08）。

use super::number_formatter;
use crate::engine::types::{CompileResult, RpnToken};
use crate::engine::{compile, expr_utils, function_table, splitter};
use crate::schema::{plain_desc, result_display_name, FormulaSchema, FunctionDoc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::LazyLock;

// =============================================================================
// 输出结构
// =============================================================================

/// 一次转换的完整产物。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExcelFormula {
    /// 单元格引用式：`"主公式"` / 分支 label → `"=A1+B1"`
    ///
    /// ⚠️ 用 `BTreeMap` 而非 `HashMap`：`HashMap` 的序列化顺序不确定，
    /// 前端拿到的键顺序会随机变。展示顺序应由前端显式控制（主公式优先）。
    pub cell_formulas: BTreeMap<String, String>,
    /// 带数值式：同上，但引用被替换为字面量
    pub value_formulas: BTreeMap<String, String>,
    /// 参数对照表（变量 → 单元格）
    pub param_mapping: Vec<ParamCell>,
    /// 告警（**已去重**）
    pub warnings: Vec<String>,
    /// 当前公式用到的 Excel 函数及说明
    #[serde(default)]
    pub used_functions: Vec<FunctionDoc>,
    /// 多结果逐条公式（表达式含多个分号段时才有；保序，单结果为空列表）
    #[serde(default)]
    pub output_formulas: Vec<ExcelOutputFormula>,
}

/// 多结果中的一条输出公式：`symbol` 来自该分号段的赋值目标
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExcelOutputFormula {
    pub symbol: String,
    pub name: String,
    pub cell_formula: String,
    pub value_formula: String,
}

/// 参数对照表的一行
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamCell {
    pub variable: String,
    pub desc: String,
    pub unit: String,
    pub cell_ref: String,
    pub required: bool,
}

/// [`substitute_values`] 的结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExcelSubstituteResult {
    /// 代入后的公式（带前导 `=`）
    pub formula: String,
    /// 无法映射到变量的单元格引用（原样保留）
    pub unresolved_refs: Vec<String>,
    /// 未声明的标识符（非函数、非常量）
    pub unresolved_names: Vec<String>,
    pub warnings: Vec<String>,
}

// =============================================================================
// 映射表
// =============================================================================

/// 本地函数名 → Excel 函数名。
///
/// `CBRTEXCEL` / `HYPOTEXCEL` 是**内部哨兵**：Excel 没有这两个函数，
/// 转换时要展开成等价写法。哨兵名**绝不能外泄到 UI**（BUG-14）——
/// 见 [`excel_display_name`]。
static FUNCTION_MAP: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    HashMap::from([
        ("sqrt", "SQRT"),
        ("cbrt", "CBRTEXCEL"),
        ("abs", "ABS"),
        ("pow", "POWER"),
        ("log", "LN"),
        ("ln", "LN"),
        ("log10", "LOG10"),
        ("exp", "EXP"),
        ("sin", "SIN"),
        ("cos", "COS"),
        ("tan", "TAN"),
        ("asin", "ASIN"),
        ("acos", "ACOS"),
        ("atan", "ATAN"),
        ("min", "MIN"),
        ("max", "MAX"),
        ("floor", "FLOOR.MATH"),
        ("ceil", "CEILING.MATH"),
        ("round", "ROUND"),
        ("hypot", "HYPOTEXCEL"),
    ])
});

/// 本地常量名 → Excel 常量写法
static CONSTANTS_MAP: LazyLock<HashMap<&'static str, &'static str>> =
    LazyLock::new(|| HashMap::from([("pi", "PI()"), ("e", "EXP(1)")]));

/// 函数说明（Excel 面板里给用户看）。
///
/// `log` 的说明里必须写明「本项目 log = 自然对数 LN，常用对数请用 log10」——
/// 这是源项目 BUG-11 的修复点：用户看到 `LN()` 会困惑，说明要解释约定。
static FUNCTION_DOCS: LazyLock<HashMap<&'static str, (&'static str, &'static str, &'static str)>> =
    LazyLock::new(|| {
        HashMap::from([
            ("sqrt", ("SQRT", "返回数值的平方根", "SQRT(number)")),
            (
                "cbrt",
                (
                    "POWER(x,1/3)",
                    "返回数值的立方根（Excel 无 CBRT，用 POWER 代替）",
                    "POWER(number, 1/3)",
                ),
            ),
            ("abs", ("ABS", "返回数值的绝对值", "ABS(number)")),
            ("pow", ("POWER", "返回数值的指定次幂", "POWER(number, power)")),
            (
                "log",
                (
                    "LN",
                    "返回数值的自然对数（约定：本项目 log = 自然对数 LN，常用对数请用 log10）",
                    "LN(number)",
                ),
            ),
            ("ln", ("LN", "返回数值的自然对数（以 e 为底）", "LN(number)")),
            (
                "log10",
                ("LOG10", "返回数值的常用对数（以 10 为底）", "LOG10(number)"),
            ),
            ("exp", ("EXP", "返回 e 的指定次幂", "EXP(number)")),
            ("sin", ("SIN", "返回角度的正弦值（弧度）", "SIN(number)")),
            ("cos", ("COS", "返回角度的余弦值（弧度）", "COS(number)")),
            ("tan", ("TAN", "返回角度的正切值（弧度）", "TAN(number)")),
            ("asin", ("ASIN", "返回数值的反正弦值（结果为弧度）", "ASIN(number)")),
            ("acos", ("ACOS", "返回数值的反余弦值（结果为弧度）", "ACOS(number)")),
            ("atan", ("ATAN", "返回数值的反正切值（结果为弧度）", "ATAN(number)")),
            (
                "min",
                ("MIN", "返回两个数值中的较小值", "MIN(number1, number2)"),
            ),
            (
                "max",
                ("MAX", "返回两个数值中的较大值", "MAX(number1, number2)"),
            ),
            (
                "floor",
                (
                    "FLOOR.MATH",
                    "将数值向下取整到最接近的整数",
                    "FLOOR.MATH(number)",
                ),
            ),
            (
                "ceil",
                (
                    "CEILING.MATH",
                    "将数值向上取整到最接近的整数",
                    "CEILING.MATH(number)",
                ),
            ),
            (
                "round",
                (
                    "ROUND",
                    "将数值四舍五入到指定小数位",
                    "ROUND(number, num_digits)",
                ),
            ),
            (
                "hypot",
                (
                    "SQRT(x^2+y^2)",
                    "返回直角三角形斜边长度（Excel 无 HYPOT）",
                    "SQRT(x^2 + y^2)",
                ),
            ),
        ])
    });

/// 运算符优先级（与源项目 `convertRpnToExcel` 内的 `prec` 一致）
fn op_prec(op: &str) -> i32 {
    match op {
        "^" => 4,
        "*" | "/" | "%" => 3,
        "+" | "-" => 2,
        // 源项目 `prec[token.op] ?: 3`
        _ => 3,
    }
}

/// 函数的 UI 展示名：哨兵函数给出 Excel 等价展开式，**绝不泄漏内部哨兵字符串**。
///
/// 源项目 BUG-14：`POWER(x,1/3)` 这种展开形态曾以 `CBRTEXCEL` 之名出现在
/// Excel 面板的「用到函数」列表里，用户看不懂。
fn excel_display_name(name: &str) -> String {
    match name.to_lowercase().as_str() {
        "cbrt" => "POWER(x,1/3)".to_string(),
        "hypot" => "SQRT(x^2+y^2)".to_string(),
        _ => FUNCTION_MAP
            .get(name)
            .map(|v| v.to_uppercase())
            .filter(|v| !v.ends_with("EXCEL"))
            .unwrap_or_else(|| name.to_uppercase()),
    }
}

// =============================================================================
// 单元格引用
// =============================================================================

/// Excel 列名：`0→A`、`25→Z`、`26→AA`、`52→BA`、`702→AAA`。
///
/// 这是 **26 进制无零位**（Excel 的列名没有 0）：
/// 逐位取模后要 `i = i/26 - 1`，否则 `26` 会算成 `BA` 而不是 `AA`。
pub fn column_name(index: usize) -> String {
    let mut i = index;
    let mut out: Vec<u8> = Vec::new();
    loop {
        out.push(b'A' + (i % 26) as u8);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    out.reverse();
    String::from_utf8(out).expect("A-Z 必然合法 UTF-8")
}

/// 变量序号 → 单元格引用（第 1 行横向，契约 v3）
pub fn index_to_cell_ref(index: usize) -> String {
    format!("{}1", column_name(index))
}

// =============================================================================
// 转换器
// =============================================================================

/// Excel 公式转换器。
///
/// 无状态（所有映射表都是 `static`），构造开销为零。
#[derive(Debug, Default, Clone, Copy)]
pub struct ExcelFormulaConverter;

impl ExcelFormulaConverter {
    pub fn new() -> Self {
        Self
    }

    /// 把公式 Schema 转成 Excel 公式。
    ///
    /// `param_values` 为空时只产出**引用式**（`value_formulas` 里的值会全是 `0`，
    /// 因为空输入按 0 处理 —— 与源项目一致）。
    pub fn convert(
        &self,
        schema: &FormulaSchema,
        param_values: &HashMap<String, String>,
    ) -> ExcelFormula {
        // 变量去重且**保持首现顺序** —— 单元格编号依赖它，不能排序
        let mut variables: Vec<String> = Vec::new();
        for v in &schema.variables {
            if !variables.contains(&v.symbol) {
                variables.push(v.symbol.clone());
            }
        }

        let param_mapping: Vec<ParamCell> = variables
            .iter()
            .enumerate()
            .map(|(index, symbol)| {
                let var = schema.variables.iter().find(|v| &v.symbol == symbol);
                ParamCell {
                    variable: symbol.clone(),
                    desc: var.map(plain_desc).unwrap_or_else(|| symbol.clone()),
                    unit: var.and_then(|v| v.unit.clone()).unwrap_or_default(),
                    cell_ref: index_to_cell_ref(index),
                    required: var.map(|v| v.required).unwrap_or(true),
                }
            })
            .collect();

        let cell_ref_map: HashMap<String, String> = param_mapping
            .iter()
            .map(|p| (p.variable.clone(), p.cell_ref.clone()))
            .collect();

        // BUG-15：主/值/分支分别收集告警，最后去重合并
        let mut warnings_main_cell: Vec<String> = Vec::new();
        let mut warnings_main_value: Vec<String> = Vec::new();
        let mut warnings_alt: Vec<String> = Vec::new();

        // BUG-13：分支 label 计数，重名时追加序号（否则 Map 覆盖会丢分支）
        let mut alt_label_count: HashMap<String, usize> = HashMap::new();

        // BUG-17：代入值统一经 number_formatter 规范化；
        // 非法输入**保留单元格引用并告警**，避免产出半截公式
        let mut value_map: HashMap<String, String> = HashMap::new();
        for pc in &param_mapping {
            let raw = param_values.get(&pc.variable).map(String::as_str).unwrap_or("");
            if raw.trim().is_empty() {
                value_map.insert(pc.variable.clone(), "0".to_string());
                continue;
            }
            match number_formatter::to_excel_literal(raw) {
                Some(lit) => {
                    value_map.insert(pc.variable.clone(), lit);
                }
                None => {
                    value_map.insert(pc.variable.clone(), pc.cell_ref.clone());
                    warnings_main_value.push(format!(
                        "变量 {} 的输入「{}」不是有效数值，已保留单元格引用 {}",
                        pc.variable, raw, pc.cell_ref
                    ));
                }
            }
        }

        // BUG-09：合并 schema.constants（**变量同名优先** —— 同名符号从常量表剔除，
        // 保证它作为变量出现在参数对照表与公式里，而不是被当作常量字面量）
        let variable_names: HashSet<&String> = variables.iter().collect();
        let mut merged_constants = function_table::constants_owned();
        for (k, v) in &schema.constants {
            if !variable_names.contains(k) {
                merged_constants.insert(k.clone(), *v);
            }
        }

        // 多结果（分号多输出）：逐段转换，每段一条 Excel 公式
        let segments = splitter::split(&schema.expression);
        let output_formulas: Vec<ExcelOutputFormula> = if segments.len() > 1 {
            segments
                .iter()
                .enumerate()
                .map(|(index, segment)| {
                    let symbol = segment
                        .symbol
                        .clone()
                        .unwrap_or_else(|| format!("结果{}", index + 1));

                    let mut cell_warnings = Vec::new();
                    let mut value_warnings = Vec::new();
                    let cell =
                        self.convert_expression(&segment.expression, &cell_ref_map, &mut cell_warnings, &merged_constants);
                    let value =
                        self.convert_expression(&segment.expression, &value_map, &mut value_warnings, &merged_constants);

                    warnings_main_cell.extend(
                        cell_warnings.iter().map(|w| format!("结果「{symbol}」：{w}")),
                    );
                    warnings_main_value.extend(
                        value_warnings.iter().map(|w| format!("结果「{symbol}」：{w}")),
                    );

                    // MULTI-02：名称取 resultOutputs，缺失时回退步骤 label / 符号
                    let name = schema
                        .result_outputs
                        .iter()
                        .find(|o| o.symbol == symbol)
                        .map(|o| o.name.clone())
                        .filter(|n| !n.trim().is_empty())
                        .unwrap_or_else(|| result_display_name(schema, &symbol));

                    ExcelOutputFormula {
                        symbol,
                        name,
                        cell_formula: format!("={cell}"),
                        value_formula: format!("={value}"),
                    }
                })
                .collect()
        } else {
            Vec::new()
        };

        let mut cell_formulas: BTreeMap<String, String> = BTreeMap::new();
        let mut value_formulas: BTreeMap<String, String> = BTreeMap::new();
        if let Some(last) = output_formulas.last() {
            // MULTI-03：主公式 = **最后一段**，与引擎 primary「最后一段」口径一致
            cell_formulas.insert("主公式".to_string(), last.cell_formula.clone());
            value_formulas.insert("主公式".to_string(), last.value_formula.clone());
        } else {
            let cell =
                self.convert_expression(&schema.expression, &cell_ref_map, &mut warnings_main_cell, &merged_constants);
            let value =
                self.convert_expression(&schema.expression, &value_map, &mut warnings_main_value, &merged_constants);
            cell_formulas.insert("主公式".to_string(), format!("={cell}"));
            value_formulas.insert("主公式".to_string(), format!("={value}"));
        }

        for alt in &schema.alt_expressions {
            if alt.expression.trim().is_empty() {
                continue;
            }
            let alt_cell =
                self.convert_expression(&alt.expression, &cell_ref_map, &mut warnings_alt, &merged_constants);
            let alt_value =
                self.convert_expression(&alt.expression, &value_map, &mut warnings_alt, &merged_constants);

            // BUG-13：分支 key 必须唯一，重名时追加序号
            let base = if alt.label.trim().is_empty() {
                "分支".to_string()
            } else {
                alt.label.clone()
            };
            let count = alt_label_count.entry(base.clone()).or_insert(0);
            *count += 1;
            let label = if *count == 1 {
                base
            } else {
                format!("{base}{count}")
            };

            cell_formulas.insert(label.clone(), format!("={alt_cell}"));
            value_formulas.insert(label, format!("={alt_value}"));
        }

        let mut warnings = Vec::new();
        warnings.extend(warnings_main_cell);
        warnings.extend(warnings_main_value);
        warnings.extend(warnings_alt);

        // 收集用到的函数：多结果时逐段收集（整条含 `;` 无法一次性编译）
        let mut used_names: HashSet<String> = HashSet::new();
        if segments.len() > 1 {
            for seg in &segments {
                collect_used_functions(&seg.expression, &mut used_names, &merged_constants);
            }
        } else {
            collect_used_functions(&schema.expression, &mut used_names, &merged_constants);
        }
        for alt in &schema.alt_expressions {
            collect_used_functions(&alt.expression, &mut used_names, &merged_constants);
        }

        let mut used_functions: Vec<FunctionDoc> = used_names
            .into_iter()
            .map(|name| match FUNCTION_DOCS.get(name.as_str()) {
                Some((excel, desc, syntax)) => FunctionDoc {
                    name: name.clone(),
                    excel_name: (*excel).to_string(),
                    description: (*desc).to_string(),
                    syntax: (*syntax).to_string(),
                },
                None => FunctionDoc {
                    // BUG-14：哨兵名（如 POWER(x,1/3) 展开形态）不得外泄到 UI
                    excel_name: excel_display_name(&name),
                    description: "Excel 函数".to_string(),
                    syntax: format!("{}(...)", excel_display_name(&name)),
                    name,
                },
            })
            .collect();
        used_functions.sort_by(|a, b| a.name.cmp(&b.name));

        ExcelFormula {
            cell_formulas,
            value_formulas,
            param_mapping,
            warnings: dedup_preserve_order(warnings),
            used_functions,
            output_formulas,
        }
    }

    /// 单条表达式 → Excel 公式（不含前导 `=`）。
    ///
    /// 编译失败时**显式告警并回退原式** —— 不能静默产出半截公式。
    pub fn convert_expression(
        &self,
        expr: &str,
        cell_ref_map: &HashMap<String, String>,
        warnings: &mut Vec<String>,
        constants: &HashMap<String, f64>,
    ) -> String {
        // BUG-08：剥离前导 `=` 与赋值前缀（Lexer 不识别 `=`）
        let clean = splitter::strip_assignment(expr);
        match compile(&clean, constants) {
            CompileResult::Ok { expr: compiled } => {
                convert_rpn_to_excel(&compiled.rpn, cell_ref_map, warnings)
            }
            CompileResult::Error { reason, .. } => {
                warnings.push(format!("表达式编译失败: {reason}"));
                expr.to_string()
            }
        }
    }
}

/// 去重但**保持首次出现顺序**（`HashSet` + `Vec` 的常见写法）。
fn dedup_preserve_order(items: Vec<String>) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        if seen.insert(it.clone()) {
            out.push(it);
        }
    }
    out
}

/// 从表达式中收集用到的函数名。
///
/// 用 [`compile`] 的 `functions` 字段（它本身就是「去重、保持首现顺序」）——
/// 比源项目再跑一遍词法分析简单，结果等价。
///
/// 差异：源项目单独跑词法分析，所以**语法失败但词法成功**时仍能收集到函数；
/// 这里编译失败就收集不到。该差异只影响「编译失败时 Excel 面板的函数清单」，
/// 而那种情况下面板本来就会显示编译失败告警，用户不会依赖函数清单。
fn collect_used_functions(
    expr: &str,
    out: &mut HashSet<String>,
    constants: &HashMap<String, f64>,
) {
    let clean = splitter::strip_assignment(expr);
    if let CompileResult::Ok { expr: compiled } = compile(&clean, constants) {
        for f in compiled.functions {
            out.insert(f.to_lowercase());
        }
    }
}

/// RPN → Excel 表达式（按优先级补括号）。
fn convert_rpn_to_excel(
    rpn: &[RpnToken],
    cell_ref_map: &HashMap<String, String>,
    warnings: &mut Vec<String>,
) -> String {
    let mut stack: Vec<String> = Vec::new();

    for token in rpn {
        match token {
            RpnToken::Number { value } => {
                stack.push(number_formatter::format(*value).unwrap_or_else(|| value.to_string()));
            }
            RpnToken::Variable { name } => {
                // 查不到映射表时**保留原名**（不是静默丢弃）
                stack.push(
                    cell_ref_map
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| name.clone()),
                );
            }
            RpnToken::Constant { name, value } => {
                let lower = name.to_lowercase();
                // BUG-09：pi/e 之外的常量（含 schema.constants）输出**字面数值**
                let excel = CONSTANTS_MAP
                    .get(lower.as_str())
                    .map(|s| (*s).to_string())
                    .or_else(|| number_formatter::format(*value))
                    .unwrap_or_else(|| name.clone());
                stack.push(excel);
            }
            RpnToken::Operator { op, .. } => {
                let right = stack.pop().unwrap_or_default();
                // 一元负号没有左操作数
                let left = if op == "u-" {
                    String::new()
                } else {
                    stack.pop().unwrap_or_default()
                };
                let cur_prec = op_prec(op);
                let is_right_assoc = op == "^";

                let left_str = if !left.is_empty() && expr_utils::needs_parens_left(&left, cur_prec) {
                    format!("({left})")
                } else {
                    left
                };
                let right_str =
                    if expr_utils::needs_parens_right(&right, cur_prec, is_right_assoc) {
                        format!("({right})")
                    } else {
                        right
                    };

                let result = match op.as_str() {
                    "+" => format!("{left_str}+{right_str}"),
                    "-" => {
                        if left_str.is_empty() {
                            format!("-{right_str}")
                        } else {
                            format!("{left_str}-{right_str}")
                        }
                    }
                    "*" => format!("{left_str}*{right_str}"),
                    "/" => format!("{left_str}/{right_str}"),
                    "^" => format!("{left_str}^{right_str}"),
                    "%" => {
                        // BUG-05：Excel 的 `%` 是后缀百分比运算符，取模必须用 MOD()
                        warnings.push(
                            "取模已转换为 MOD()；注意本地 % 与 Excel MOD 对负数符号定义不同（本地结果跟随被除数，Excel MOD 跟随除数）"
                                .to_string(),
                        );
                        format!("MOD({left_str},{right_str})")
                    }
                    "u-" => format!("-{right_str}"),
                    other => {
                        warnings.push(format!("未知运算符: {other}"));
                        format!("{left_str}{other}{right_str}")
                    }
                };
                stack.push(result);
            }
            RpnToken::Function { name, arg_count } => {
                let mut args: Vec<String> = Vec::new();
                for _ in 0..*arg_count {
                    // 插到 0 位：栈是后进先出，这样参数顺序才正
                    args.insert(0, stack.pop().unwrap_or_default());
                }

                let lower = name.to_lowercase();
                let excel_func = FUNCTION_MAP
                    .get(lower.as_str())
                    .map(|s| (*s).to_string())
                    .unwrap_or_else(|| name.to_uppercase());

                let joined = args.join(",");
                let result = match excel_func.as_str() {
                    "CBRTEXCEL" => {
                        warnings.push("Excel 无 CBRT，使用 ^(1/3) 代替".to_string());
                        format!("POWER({},1/3)", args.first().cloned().unwrap_or_default())
                    }
                    "HYPOTEXCEL" => {
                        warnings.push("Excel 无 HYPOT，展开为 SQRT(x^2+y^2)".to_string());
                        format!(
                            "SQRT({}^2+{}^2)",
                            args.first().cloned().unwrap_or_default(),
                            args.get(1).cloned().unwrap_or_default()
                        )
                    }
                    "ROUND" => {
                        warnings.push("Excel ROUND 需第二参数，默认 0".to_string());
                        format!("ROUND({},0)", args.first().cloned().unwrap_or_default())
                    }
                    // 其余一律「函数名(参数)」形式
                    _ => format!("{excel_func}({joined})"),
                };
                stack.push(result);
            }
        }
    }

    stack.last().cloned().unwrap_or_default()
}

// =============================================================================
// 数值代入（单趟词法扫描）
// =============================================================================

/// 单元格引用形态：`[$]?[A-Za-z]{1,3}[$]?[0-9]+`
///
/// 用**手写匹配**而不是正则：需要与字符索引交互（前后边界判断），
/// 正则的字节索引与 `Vec<char>` 索引混用极易出错。
fn match_ref(chars: &[char], start: usize) -> Option<usize> {
    let n = chars.len();
    let mut p = start;

    if p < n && chars[p] == '$' {
        p += 1;
    }

    let letters_start = p;
    while p < n && chars[p].is_ascii_alphabetic() && p - letters_start < 3 {
        p += 1;
    }
    if p == letters_start {
        return None;
    }
    // 第 4 个还是字母 → 不是引用（如 `ABCD1`）
    if p < n && chars[p].is_ascii_alphabetic() {
        return None;
    }

    if p < n && chars[p] == '$' {
        p += 1;
    }

    let digits_start = p;
    while p < n && chars[p].is_ascii_digit() {
        p += 1;
    }
    if p == digits_start {
        return None;
    }

    Some(p - start)
}

/// 标识符/引用边界字符（字母、数字、`_`、`.`）
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.'
}

/// 引用**后**不能跟的字符（比 [`is_word_char`] 多一个 `(`）
fn is_ref_tail_char(c: char) -> bool {
    is_word_char(c) || c == '('
}

/// 去掉 `$` 并把列名大写（`$a$1` → `A1`）
fn normalize_ref(r: &str) -> String {
    let clean: String = r.chars().filter(|c| *c != '$').collect();
    let row = ref_row_str(&clean);
    let col = &clean[..clean.len() - row.len()];
    format!("{}{}", col.to_uppercase(), row)
}

/// 取引用末尾的数字串（行号）
fn ref_row_str(r: &str) -> &str {
    let bytes = r.as_bytes();
    let mut i = bytes.len();
    while i > 0 && bytes[i - 1].is_ascii_digit() {
        i -= 1;
    }
    &r[i..]
}

/// 归一化引用 → (列名, 行号)；形态不合法返回 `None`
fn parse_ref(r: &str) -> Option<(String, i32)> {
    let row = ref_row_str(r);
    if row.is_empty() {
        return None;
    }
    let row_num: i32 = row.parse().ok()?;
    let col = &r[..r.len() - row.len()];
    Some((col.to_string(), row_num))
}

/// 词法扫描产出的 token
#[derive(Debug, Clone, PartialEq)]
enum ScanToken {
    /// 单元格引用
    Ref { text: String, normalized: String },
    /// 标识符（可能是函数调用）
    Ident {
        text: String,
        name: String,
        is_call: bool,
    },
    /// 原样文本（字符串字面量、工作表名、数字、运算符）
    Text(String),
}

/// **单趟词法扫描**：引用 / 标识符 / 原样文本。
///
/// 识别的形态：
/// - 字符串字面量 `"..."`（`""` 为转义）→ 原样
/// - 工作表名 `'...'!` → 原样
/// - 单元格引用（含 `$`）→ [`ScanToken::Ref`]
/// - 标识符（字母开头，可含数字/`_`/`.`）→ [`ScanToken::Ident`]；
///   紧跟 `(` 则 `is_call = true`
/// - 数字字面量（含科学计数法）→ 原样
/// - 其他单字符 → 原样
fn scan(expr: &str) -> Vec<ScanToken> {
    let chars: Vec<char> = expr.chars().collect();
    let n = chars.len();
    let mut out: Vec<ScanToken> = Vec::new();
    let mut i = 0;

    while i < n {
        let c = chars[i];

        if c == '"' {
            // 字符串字面量：`""` 是转义的双引号
            let start = i;
            i += 1;
            while i < n {
                if chars[i] == '"' {
                    if i + 1 < n && chars[i + 1] == '"' {
                        i += 2;
                    } else {
                        i += 1;
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            out.push(ScanToken::Text(chars[start..i].iter().collect()));
            continue;
        }

        if c == '\'' {
            // 工作表名 `'My Sheet'!` —— 连后面的 `!` 一起原样保留
            let start = i;
            i += 1;
            while i < n {
                if chars[i] == '\'' {
                    if i + 1 < n && chars[i + 1] == '\'' {
                        i += 2;
                    } else {
                        i += 1;
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            if i < n && chars[i] == '!' {
                i += 1;
            }
            out.push(ScanToken::Text(chars[start..i].iter().collect()));
            continue;
        }

        if c.is_alphabetic() || c == '$' {
            let matched = match_ref(&chars, i);
            let is_ref_candidate = match matched {
                Some(len) => {
                    let end = i + len;
                    let prev_ok = i == 0 || !is_word_char(chars[i - 1]);
                    let next_ok = end >= n || !is_ref_tail_char(chars[end]);
                    prev_ok && next_ok
                }
                None => false,
            };

            if let (true, Some(len)) = (is_ref_candidate, matched) {
                let text: String = chars[i..i + len].iter().collect();
                out.push(ScanToken::Ref {
                    normalized: normalize_ref(&text),
                    text,
                });
                i += len;
            } else if c.is_alphabetic() {
                // 标识符：字母开头的连续字母/数字/`_`/`.`
                let start = i;
                while i < n && is_word_char(chars[i]) {
                    i += 1;
                }
                let name: String = chars[start..i].iter().collect();
                // `LOG10(` 这类「字母+数字+(」已被引用匹配排除，落到这里的字母串
                // 若紧跟 `(` 就是函数调用
                let is_call = i < n && chars[i] == '(' && name.chars().any(|x| x.is_alphabetic());
                out.push(ScanToken::Ident {
                    text: name.clone(),
                    name,
                    is_call,
                });
            } else {
                // 单独的 `$`（不成引用）
                out.push(ScanToken::Text("$".to_string()));
                i += 1;
            }
            continue;
        }

        if c.is_ascii_digit() {
            // 数字字面量（含科学计数法）—— 原样保留，不参与替换
            let start = i;
            while i < n {
                let ch = chars[i];
                if ch.is_ascii_digit() || ch == '.' {
                    i += 1;
                } else if (ch == 'e' || ch == 'E')
                    && i + 1 < n
                    && (chars[i + 1].is_ascii_digit()
                        || ((chars[i + 1] == '+' || chars[i + 1] == '-')
                            && i + 2 < n
                            && chars[i + 2].is_ascii_digit()))
                {
                    i += 1;
                    if chars[i] == '+' || chars[i] == '-' {
                        i += 1;
                    }
                } else {
                    break;
                }
            }
            out.push(ScanToken::Text(chars[start..i].iter().collect()));
            continue;
        }

        out.push(ScanToken::Text(c.to_string()));
        i += 1;
    }

    out
}

/// 本地/Excel 双侧都认识的函数（用于残留标识符判定）
static KNOWN_FUNCTIONS: LazyLock<HashSet<String>> = LazyLock::new(|| {
    let mut s: HashSet<String> = HashSet::new();
    for name in function_table::FUNCTIONS.keys() {
        s.insert((*name).to_string());
        s.insert(name.to_uppercase());
    }
    for extra in [
        "PI",
        "MOD",
        "POWER",
        "SQRT",
        "LN",
        "LOG10",
        "EXP",
        "SIN",
        "COS",
        "TAN",
        "ASIN",
        "ACOS",
        "ATAN",
        "ABS",
        "MIN",
        "MAX",
        "FLOOR.MATH",
        "CEILING.MATH",
        "ROUND",
        "IF",
    ] {
        s.insert(extra.to_string());
    }
    s
});

/// **Excel 词法扫描 + 单趟数值代入**。
///
/// 这是「带数值式」Excel 公式的产出路径。要点：
///
/// - 只识别 `[A-Za-z]{1,3}[0-9]+` 形态的引用（跳过函数名 `LOG10(`、
///   字符串字面量、工作表名）
/// - 命中引用先查 `cell_ref_map`；查不到时按「引用首现顺序 ↔ 未认领变量顺序」
///   **兼容对齐**（兜住历史纵向 `A1/A2/…` 数据），对齐产生 warning
/// - 数值经 [`number_formatter`] 规范化（负数加括号、千分位、科学计数）；
///   非法值**保留原引用**并告警
/// - 扫描结束后的残留标识符（非函数、非常量）进 `unresolvedNames`
///
/// ⚠️ **严禁**改成 `replace` 循环 —— 见本模块文档的红线说明。
pub fn substitute_values(
    excel_expr: &str,
    cell_ref_map: &HashMap<String, String>,
    param_values: &HashMap<String, String>,
) -> ExcelSubstituteResult {
    let expr = excel_expr.trim().strip_prefix('=').unwrap_or(excel_expr.trim());
    let mut warnings: Vec<String> = Vec::new();
    let mut unresolved_refs: Vec<String> = Vec::new();
    let mut unresolved_names: Vec<String> = Vec::new();
    // 归一化引用 → 变量（兼容对齐用）
    let mut aligned: HashMap<String, String> = HashMap::new();

    let mut ref_to_var: HashMap<String, String> = HashMap::new();
    for (variable, r) in cell_ref_map {
        ref_to_var.insert(normalize_ref(r), variable.clone());
    }

    let tokens = scan(expr);

    // ── 第 1 趟：收集引用出现顺序（兼容对齐用）──
    //
    // 只需要引用顺序：标识符的判定在第 2 趟做（那里本来就要遍历 tokens），
    // 没必要提前收一遍。
    let mut ref_order: Vec<String> = Vec::new();
    for t in &tokens {
        if let ScanToken::Ref { normalized, .. } = t {
            ref_order.push(normalized.clone());
        }
    }

    // 兼容对齐：未在映射表里的第 1 行引用，按首现顺序认领「公式中未出现」的变量
    let refs_in_formula: Vec<String> = dedup_preserve_order(ref_order.clone());
    let mut matched_vars: HashSet<String> = refs_in_formula
        .iter()
        .filter_map(|r| ref_to_var.get(r).cloned())
        .collect();
    let unclaimed_vars: Vec<String> = cell_ref_map
        .keys()
        .filter(|v| !matched_vars.contains(*v))
        .cloned()
        .collect();

    for r in &refs_in_formula {
        if ref_to_var.contains_key(r) {
            continue;
        }
        // 只对齐「疑似契约内」的引用：第 1 行（新契约横向）或 A 列（旧契约纵向）
        let Some((col, row)) = parse_ref(r) else {
            continue;
        };
        if row != 1 && col != "A" {
            continue;
        }
        if let Some(candidate) = unclaimed_vars.iter().find(|v| !matched_vars.contains(*v)) {
            aligned.insert(r.clone(), candidate.clone());
            matched_vars.insert(candidate.clone());
            warnings.push(format!(
                "单元格引用 {r} 未在映射表中，已按首现顺序对齐到变量 {candidate}（历史契约兼容），请核对参数表"
            ));
        }
    }

    // 归一化引用 → 变量名（映射表优先，其次兼容对齐结果）
    let lookup_var = |normalized: &str| -> Option<String> {
        ref_to_var
            .get(normalized)
            .or_else(|| aligned.get(normalized))
            .cloned()
    };

    // 变量 → Excel 数值字面量。空输入按 `0`；非法输入返回 `None`（调用方保留原引用）
    let literal_of = |variable: &str| -> Option<String> {
        let raw = param_values.get(variable).map(String::as_str).unwrap_or("");
        if raw.trim().is_empty() {
            return Some("0".to_string());
        }
        number_formatter::to_excel_literal(raw)
    };

    // ── 第 2 趟：实际替换 ──
    let mut out = String::with_capacity(expr.len());
    for t in &tokens {
        match t {
            ScanToken::Ref { text, normalized } => match lookup_var(normalized) {
                // 引用映射不到任何变量：原样保留 + 告警
                None => {
                    warnings.push(format!("单元格引用 {text} 无法映射到任何变量，已原样保留"));
                    unresolved_refs.push(text.clone());
                    out.push_str(text);
                }
                // 映射到了，但输入值不是合法数值：同样保留原引用
                Some(v) => match literal_of(&v) {
                    Some(lit) => out.push_str(&lit),
                    None => {
                        let raw = param_values.get(&v).map(String::as_str).unwrap_or("");
                        warnings.push(format!(
                            "变量 {v} 的输入「{raw}」不是有效数值，已保留原引用 {text}"
                        ));
                        unresolved_refs.push(text.clone());
                        out.push_str(text);
                    }
                },
            },
            ScanToken::Ident { text, name, is_call } => {
                if *is_call {
                    let known = KNOWN_FUNCTIONS.contains(&name.to_lowercase())
                        || KNOWN_FUNCTIONS.contains(&name.to_uppercase());
                    if !known {
                        unresolved_names.push(name.clone());
                        warnings.push(format!(
                            "公式中的「{name}()」不是支持的函数，Excel 中将无法解析"
                        ));
                    }
                } else {
                    let lower = name.to_lowercase();
                    if lower != "pi" && lower != "e" {
                        unresolved_names.push(name.clone());
                        warnings.push(format!(
                            "公式中存在未声明的符号「{name}」，代入后请人工核对"
                        ));
                    }
                }
                out.push_str(text);
            }
            ScanToken::Text(t) => out.push_str(t),
        }
    }

    ExcelSubstituteResult {
        formula: format!("={out}"),
        unresolved_refs: dedup_preserve_order(unresolved_refs),
        unresolved_names: dedup_preserve_order(unresolved_names),
        warnings: dedup_preserve_order(warnings),
    }
}

/// 拆分多结果的 Excel 公式串。
///
/// AI 生成的 `excelExpression` 形如 `"x = =(A1+B1); y = =(A1-B1)"`，
/// 也容忍 `"x = (A1+B1)"` 这种省略内层 `=` 的写法。
///
/// 单段、或每段都取不出符号时返回空列表 —— 由调用方回退到单结果路径。
pub fn split_output_formula(excel_expression: &str) -> Vec<(String, String)> {
    let cleaned = excel_expression.trim().strip_prefix('=').unwrap_or(excel_expression.trim());
    let segments = splitter::split(cleaned.trim());
    if segments.len() <= 1 {
        return Vec::new();
    }
    segments
        .iter()
        .filter_map(|seg| {
            let symbol = seg.symbol.clone()?;
            let body = seg.expression.trim().strip_prefix('=').unwrap_or(seg.expression.trim());
            Some((symbol, format!("={body}")))
        })
        .collect()
}


// =============================================================================
// 残留符号扫描（诊断）
// =============================================================================

/// 公式文本里残留的「未解析符号」。
///
/// ## 为什么需要它
///
/// [`ExcelFormulaConverter::convert`] 对**未声明的变量**会保留原名 ——
/// 既不静默丢弃（那样公式会少一项，更难发现），也不报错
/// （见 `convert_rpn_to_excel` 的 `RpnToken::Variable` 分支）。
/// 于是生成的 Excel 公式里会出现一个用户看不懂的裸标识符。
///
/// 这个扫描把那些符号找出来，供前端提示
/// （「表达式中 `xyz` 未在参数表中声明」）。
///
/// ⚠️ **不要**用 [`substitute_values`] 的 `unresolved_*` 代替：
/// 那个只覆盖「数值代入」这一条路径，而本函数对**任意**公式文本都适用。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResidualScan {
    /// 残留的单元格引用。
    ///
    /// **值模式下不该出现** —— 出现了说明 `inputs` 里缺这个变量的值
    /// （`convert` 会回退成单元格引用并告警）。
    pub cell_refs: Vec<String>,
    /// 残留的标识符（既不是函数调用，也不是数字字面量的一部分）。
    pub names: Vec<String>,
}

impl ResidualScan {
    pub fn is_empty(&self) -> bool {
        self.cell_refs.is_empty() && self.names.is_empty()
    }
}

/// 扫描公式里的残留符号（去重、保持首现顺序）。
///
/// 会跳过：
/// - 字符串字面量 `"…"`（`""` 为转义双引号）
/// - 工作表名 `'…'`（`''` 为转义撇号）
/// - 函数调用（标识符后面跳过空白紧跟 `(`）
/// - 科学计数法的指数部分（`1E+300` 里的 `E`）
pub fn scan_residuals(formula: &str) -> ResidualScan {
    let body = formula.strip_prefix('=').unwrap_or(formula);
    let chars: Vec<char> = body.chars().collect();

    let mut scan = ResidualScan::default();
    let mut seen_refs: HashSet<String> = HashSet::new();
    let mut seen_names: HashSet<String> = HashSet::new();

    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];

        // 字符串字面量
        if c == '"' {
            i += 1;
            while i < chars.len() {
                if chars[i] == '"' {
                    if chars.get(i + 1) == Some(&'"') {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }

        // 工作表名
        if c == '\'' {
            i += 1;
            while i < chars.len() {
                if chars[i] == '\'' {
                    if chars.get(i + 1) == Some(&'\'') {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }

        // 词：以字母 / `_` / `$` 开头
        if c.is_ascii_alphabetic() || c == '_' || c == '$' {
            // ⚠️ 前一个字符是数字 → 科学计数法的指数部分（`1E+300` 里的 `E`），不是标识符
            let prev_is_digit = i > 0 && chars[i - 1].is_ascii_digit();

            if !prev_is_digit {
                let start = i;
                while i < chars.len()
                    && (chars[i].is_ascii_alphanumeric()
                        || chars[i] == '_'
                        || chars[i] == '.'
                        || chars[i] == '$')
                {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                let bare: String = word.chars().filter(|c| *c != '$').collect();

                if is_cell_ref_shape(&bare) {
                    if seen_refs.insert(word.clone()) {
                        scan.cell_refs.push(word);
                    }
                } else {
                    // 跳过空白后紧跟 `(` → 函数调用，不算残留
                    let mut j = i;
                    while j < chars.len() && chars[j].is_whitespace() {
                        j += 1;
                    }
                    if chars.get(j) != Some(&'(') && seen_names.insert(word.clone()) {
                        scan.names.push(word);
                    }
                }
                continue;
            }
        }

        i += 1;
    }

    scan
}

/// 形如 `A1` / `AA10` / `XYZ99`（1~3 个字母 + 至少一位数字）。
///
/// 4 个及以上字母（如 `ABCD1`）**不是**合法 Excel 列名，按标识符处理 ——
/// 与 [`substitute_values`] 的判定保持一致。
fn is_cell_ref_shape(bare: &str) -> bool {
    let letters_len = bare.chars().take_while(|c| c.is_ascii_alphabetic()).count();
    if letters_len == 0 || letters_len > 3 {
        return false;
    }
    let digits = &bare[letters_len..];
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{AltExpression, FormulaSource, FormulaVar, ResultOutput, SourceKind};

    // ============================================================ 测试助手

    /// 构造一个测试用 Schema（变量单位固定 `m`，AI 来源未核验）
    fn schema(expression: &str, symbols: &[&str]) -> FormulaSchema {
        FormulaSchema {
            id: "test:0".to_string(),
            result_name: "测试".to_string(),
            result_symbol: "X".to_string(),
            result_unit: Some("m".to_string()),
            result_outputs: Vec::new(),
            expression: expression.to_string(),
            source_equations: Vec::new(),
            alt_expressions: Vec::new(),
            constants: HashMap::new(),
            variables: symbols
                .iter()
                .map(|s| FormulaVar {
                    symbol: (*s).to_string(),
                    desc: (*s).to_string(),
                    unit: Some("m".to_string()),
                    default: None,
                    min: None,
                    max: None,
                    required: true,
                })
                .collect(),
            domain: String::new(),
            tags: Vec::new(),
            reference_basis: None,
            design_notes: None,
            explanation: None,
            image_ids: Vec::new(),
            revised_from: None,
            source: FormulaSource {
                kind: SourceKind::Ai,
                ref_: None,
                verified: false,
                model: None,
                created_by: None,
            },
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

    /// 变量 → 单元格引用（按序号，契约 v3 第 1 行横向）
    fn ref_map(pairs: &[(&str, usize)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(name, idx)| ((*name).to_string(), index_to_cell_ref(*idx)))
            .collect()
    }

    /// 变量 → 输入值
    fn vals(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn convert(expression: &str, symbols: &[&str]) -> ExcelFormula {
        ExcelFormulaConverter::new().convert(&schema(expression, symbols), &HashMap::new())
    }

    fn main_formula(r: &ExcelFormula) -> String {
        r.cell_formulas.get("主公式").cloned().unwrap_or_default()
    }

    // ============================================================ 基础运算

    #[test]
    fn arithmetic_operators() {
        assert_eq!(main_formula(&convert("r+h", &["r", "h"])), "=A1+B1");
        assert_eq!(main_formula(&convert("a-b", &["a", "b"])), "=A1-B1");
        assert_eq!(main_formula(&convert("a*b", &["a", "b"])), "=A1*B1");
        assert_eq!(main_formula(&convert("a/b", &["a", "b"])), "=A1/B1");
        assert_eq!(main_formula(&convert("r^2", &["r"])), "=A1^2");
    }

    #[test]
    fn unary_minus() {
        assert_eq!(main_formula(&convert("-x", &["x"])), "=-A1");
    }

    // ============================================================ 常量

    #[test]
    fn pi_constant_maps_to_pi_function() {
        assert_eq!(
            main_formula(&convert("pi*r^2*h", &["r", "h"])),
            "=PI()*A1^2*B1"
        );
    }

    #[test]
    fn e_constant_maps_to_exp_one() {
        assert_eq!(main_formula(&convert("e^x", &["x"])), "=EXP(1)^A1");
    }

    // ============================================================ 函数映射

    #[test]
    fn basic_function_mapping() {
        assert_eq!(main_formula(&convert("sqrt(x)", &["x"])), "=SQRT(A1)");
        assert_eq!(main_formula(&convert("abs(x)", &["x"])), "=ABS(A1)");
        assert_eq!(
            main_formula(&convert("pow(x,y)", &["x", "y"])),
            "=POWER(A1,B1)"
        );
        assert_eq!(main_formula(&convert("sin(x)", &["x"])), "=SIN(A1)");
        assert_eq!(main_formula(&convert("cos(x)", &["x"])), "=COS(A1)");
        assert_eq!(main_formula(&convert("tan(x)", &["x"])), "=TAN(A1)");
        assert_eq!(main_formula(&convert("exp(x)", &["x"])), "=EXP(A1)");
        assert_eq!(main_formula(&convert("log10(x)", &["x"])), "=LOG10(A1)");
        assert_eq!(
            main_formula(&convert("floor(x)", &["x"])),
            "=FLOOR.MATH(A1)"
        );
        assert_eq!(
            main_formula(&convert("ceil(x)", &["x"])),
            "=CEILING.MATH(A1)"
        );
        assert_eq!(
            main_formula(&convert("min(x,y)", &["x", "y"])),
            "=MIN(A1,B1)"
        );
        assert_eq!(
            main_formula(&convert("max(x,y)", &["x", "y"])),
            "=MAX(A1,B1)"
        );
    }

    /// 本项目约定 `log` = 自然对数（映射到 `LN`），不是常用对数
    #[test]
    fn log_maps_to_ln() {
        assert_eq!(main_formula(&convert("log(x)", &["x"])), "=LN(A1)");
        assert_eq!(main_formula(&convert("ln(x)", &["x"])), "=LN(A1)");
    }

    #[test]
    fn round_gets_zero_second_arg() {
        let r = convert("round(x)", &["x"]);
        assert_eq!(main_formula(&r), "=ROUND(A1,0)");
        assert!(r.warnings.iter().any(|w| w.contains("ROUND")));
    }

    #[test]
    fn cbrt_expands_to_power() {
        let r = convert("cbrt(x)", &["x"]);
        assert_eq!(main_formula(&r), "=POWER(A1,1/3)");
        assert!(r.warnings.iter().any(|w| w.contains("CBRT")));
    }

    #[test]
    fn hypot_expands_to_sqrt() {
        let r = convert("hypot(x,y)", &["x", "y"]);
        assert_eq!(main_formula(&r), "=SQRT(A1^2+B1^2)");
        assert!(r.warnings.iter().any(|w| w.contains("HYPOT")));
    }

    #[test]
    fn function_name_case_is_preserved() {
        assert_eq!(main_formula(&convert("ROUND(x)", &["x"])), "=ROUND(A1,0)");
    }

    // ============================================================ 嵌套

    #[test]
    fn nested_functions() {
        assert_eq!(
            main_formula(&convert("sqrt(pi*r^2)", &["r"])),
            "=SQRT(PI()*A1^2)"
        );
        assert_eq!(
            main_formula(&convert("sqrt(pi*r^2*h/3)", &["r", "h"])),
            "=SQRT(PI()*A1^2*B1/3)"
        );
    }

    // ============================================================ 无变量 / 编译失败

    #[test]
    fn no_variables() {
        let r = convert("pi*2", &[]);
        assert_eq!(main_formula(&r), "=PI()*2");
        assert!(r.param_mapping.is_empty());
    }

    /// 编译失败时**回退原式并显式告警**（不静默产出半截公式）。
    ///
    /// ⚠️ 源项目这里用的是 `"+++"`，但 PC 端 parser 更宽松：
    /// `+++` / `a +` / `*a` / `a ** b` 都能编译通过，只有**括号不匹配**
    /// 等结构性错误才会失败（实测见 P3-1 记录）。所以用 `"a + (b"`。
    #[test]
    fn compile_error_returns_original_with_warning() {
        let r = convert("a + (b", &["x"]);
        assert!(
            r.warnings.iter().any(|w| w.contains("编译失败")),
            "应告警: {:?}",
            r.warnings
        );
        assert_eq!(main_formula(&r), "=a + (b");
    }

    // ============================================================ 参数对照表

    #[test]
    fn param_mapping_order_and_fields() {
        let r = convert("a+b*c", &["a", "b", "c"]);
        assert_eq!(r.param_mapping.len(), 3);
        assert_eq!(r.param_mapping[0].variable, "a");
        assert_eq!(r.param_mapping[0].cell_ref, "A1");
        assert_eq!(r.param_mapping[0].unit, "m");
        assert!(r.param_mapping[0].required);
        assert_eq!(r.param_mapping[1].cell_ref, "B1");
        assert_eq!(r.param_mapping[2].cell_ref, "C1");
    }

    #[test]
    fn multi_column_cell_ref() {
        let symbols: Vec<String> = (0..27).map(|i| format!("v{i}")).collect();
        let refs: Vec<&str> = symbols.iter().map(String::as_str).collect();
        let expr = symbols.join("+");
        let r = convert(&expr, &refs);

        assert_eq!(r.param_mapping[0].cell_ref, "A1");
        assert_eq!(r.param_mapping[25].cell_ref, "Z1");
        assert_eq!(r.param_mapping[26].cell_ref, "AA1");
    }

    #[test]
    fn column_name_beyond_z() {
        assert_eq!(column_name(0), "A");
        assert_eq!(column_name(25), "Z");
        assert_eq!(column_name(26), "AA");
        assert_eq!(column_name(52), "BA");
        assert_eq!(column_name(702), "AAA");
    }

    /// 变量去重且保持首现顺序（单元格编号依赖它）
    #[test]
    fn duplicate_variables_are_deduped() {
        let r = convert("a+a", &["a", "a"]);
        assert_eq!(r.param_mapping.len(), 1);
        assert_eq!(main_formula(&r), "=A1+A1");
    }

    // ============================================================ 分支

    #[test]
    fn alt_expressions_become_extra_entries() {
        let mut s = schema("a+b", &["a", "b"]);
        s.alt_expressions = vec![AltExpression {
            label: "分支1".to_string(),
            expression: "a-b".to_string(),
            condition: Some("a>b".to_string()),
        }];
        let r = ExcelFormulaConverter::new().convert(&s, &HashMap::new());

        assert_eq!(main_formula(&r), "=A1+B1");
        assert_eq!(r.cell_formulas.get("分支1").map(String::as_str), Some("=A1-B1"));
    }

    /// BUG-13：分支 label 重名时追加序号，否则 Map 覆盖会丢分支
    #[test]
    fn duplicate_alt_labels_get_suffix() {
        let mut s = schema("a+b", &["a", "b"]);
        s.alt_expressions = vec![
            AltExpression {
                label: "分支".to_string(),
                expression: "a-b".to_string(),
                condition: None,
            },
            AltExpression {
                label: "分支".to_string(),
                expression: "a*b".to_string(),
                condition: None,
            },
        ];
        let r = ExcelFormulaConverter::new().convert(&s, &HashMap::new());

        assert_eq!(r.cell_formulas.get("分支").map(String::as_str), Some("=A1-B1"));
        assert_eq!(r.cell_formulas.get("分支2").map(String::as_str), Some("=A1*B1"));
    }

    /// 空白 label 用「分支」兜底
    #[test]
    fn blank_alt_label_falls_back() {
        let mut s = schema("a+b", &["a", "b"]);
        s.alt_expressions = vec![AltExpression {
            label: "   ".to_string(),
            expression: "a-b".to_string(),
            condition: None,
        }];
        let r = ExcelFormulaConverter::new().convert(&s, &HashMap::new());
        assert!(r.cell_formulas.contains_key("分支"));
    }

    // ============================================================ 多结果（分号）

    fn multi_schema() -> FormulaSchema {
        let mut s = schema(
            "X2 = X1 + D*cos(alpha); Y2 = Y1 + D*sin(alpha)",
            &["X1", "Y1", "D", "alpha"],
        );
        s.result_outputs = vec![
            ResultOutput {
                symbol: "X2".to_string(),
                name: "新点 X 坐标".to_string(),
                unit: "m".to_string(),
            },
            ResultOutput {
                symbol: "Y2".to_string(),
                name: "新点 Y 坐标".to_string(),
                unit: "m".to_string(),
            },
        ];
        s
    }

    #[test]
    fn multi_output_converted_per_segment() {
        let r = ExcelFormulaConverter::new().convert(&multi_schema(), &HashMap::new());

        assert_eq!(r.output_formulas.len(), 2);
        let symbols: Vec<&str> = r.output_formulas.iter().map(|o| o.symbol.as_str()).collect();
        assert_eq!(symbols, ["X2", "Y2"]);
        assert_eq!(r.output_formulas[0].cell_formula, "=A1+C1*COS(D1)");
        assert_eq!(r.output_formulas[1].cell_formula, "=B1+C1*SIN(D1)");
        // 旧实现把含 ';' 的整条交给 Lexer → 必然编译失败 → 导出断式
        assert!(
            !r.warnings.iter().any(|w| w.contains("编译失败")),
            "不应出现编译失败告警: {:?}",
            r.warnings
        );
    }

    #[test]
    fn multi_output_names_come_from_declared_list() {
        let r = ExcelFormulaConverter::new().convert(&multi_schema(), &HashMap::new());
        let names: Vec<&str> = r.output_formulas.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["新点 X 坐标", "新点 Y 坐标"]);
    }

    /// 主公式 = 最后一段（与引擎 `primary`「最后一段」口径一致）
    #[test]
    fn multi_output_main_formula_is_last_segment() {
        let r = ExcelFormulaConverter::new().convert(&multi_schema(), &HashMap::new());
        assert_eq!(main_formula(&r), r.output_formulas.last().unwrap().cell_formula);
        assert_eq!(
            r.value_formulas.get("主公式").map(String::as_str),
            Some(r.output_formulas.last().unwrap().value_formula.as_str())
        );
    }

    /// 某段编译失败时告警带符号前缀，方便定位是哪个结果出了问题
    ///
    /// （用括号不匹配而非 `+++` —— 见 [`compile_error_returns_original_with_warning`]）
    #[test]
    fn multi_output_warnings_carry_symbol() {
        let r = convert("x = 1+1; y = a + (b", &["a", "b"]);
        assert!(
            r.warnings.iter().any(|w| w.contains("结果「y」")),
            "告警应带符号前缀: {:?}",
            r.warnings
        );
    }

    #[test]
    fn single_segment_keeps_output_formulas_empty() {
        assert!(convert("a+b", &["a", "b"]).output_formulas.is_empty());
    }

    // ============================================================ split_output_formula

    #[test]
    fn split_output_formula_handles_both_shapes() {
        assert_eq!(
            split_output_formula("x = =(A1+B1)/D; y = =(A1-B1)/D"),
            vec![
                ("x".to_string(), "=(A1+B1)/D".to_string()),
                ("y".to_string(), "=(A1-B1)/D".to_string()),
            ]
        );
        assert_eq!(
            split_output_formula("x = A1+B1; y = A1-B1"),
            vec![
                ("x".to_string(), "=A1+B1".to_string()),
                ("y".to_string(), "=A1-B1".to_string()),
            ]
        );
    }

    #[test]
    fn split_output_formula_ignores_single_or_symbol_less() {
        assert!(split_output_formula("=A1+B1").is_empty());
        assert!(split_output_formula("A1+B1; A1-B1").is_empty());
    }

    // ============================================================ 缺陷回归

    /// BUG-05：Excel 的 `%` 是后缀百分比运算符，取模必须用 `MOD()`
    #[test]
    fn bug05_modulo_maps_to_mod() {
        let r = convert("a % b", &["a", "b"]);
        assert_eq!(main_formula(&r), "=MOD(A1,B1)");
        assert!(
            r.warnings.iter().any(|w| w.contains("MOD") && w.contains("负数")),
            "应说明 MOD 与本地 % 的负数差异: {:?}",
            r.warnings
        );
    }

    /// BUG-08：赋值前缀被剥离，且**不吞掉表达式内部的等号**
    #[test]
    fn bug08_assignment_prefix_stripped() {
        let mut warnings = Vec::new();
        let out = ExcelFormulaConverter::new().convert_expression(
            "V = pi*r^2*h",
            &ref_map(&[("r", 0), ("h", 1)]),
            &mut warnings,
            &HashMap::new(),
        );
        assert_eq!(out, "PI()*A1^2*B1");
        assert!(warnings.is_empty(), "不应有告警: {warnings:?}");
    }

    #[test]
    fn bug08_inner_equals_fails_loudly() {
        let mut warnings = Vec::new();
        // 中段 `=` 不是赋值前缀：词法报未知字符，转换失败显式告警并回退原式
        let out = ExcelFormulaConverter::new().convert_expression(
            "sin(a) = 1",
            &ref_map(&[("a", 0)]),
            &mut warnings,
            &HashMap::new(),
        );
        assert_eq!(out, "sin(a) = 1");
        assert!(warnings.iter().any(|w| w.contains("编译失败")));
    }

    /// BUG-09：`schema.constants` 里的常量输出**字面数值**，不再是未知符号
    #[test]
    fn bug09_schema_constant_emits_literal() {
        let mut s = schema("alpha1*b", &["b"]);
        s.constants = HashMap::from([("alpha1".to_string(), 1.0)]);
        let r = ExcelFormulaConverter::new().convert(&s, &HashMap::new());
        assert_eq!(main_formula(&r), "=1*A1");
    }

    /// BUG-09：变量与常量同名时**变量优先**
    #[test]
    fn bug09_variable_shadows_constant() {
        let mut s = schema("k*b", &["k", "b"]);
        s.constants = HashMap::from([("k".to_string(), 9.0)]);
        let r = ExcelFormulaConverter::new().convert(&s, &HashMap::new());
        // k 是变量 → 输出单元格引用，不是 9
        assert_eq!(main_formula(&r), "=A1*B1");
    }

    /// BUG-14：内部哨兵名（`CBRTEXCEL` / `HYPOTEXCEL`）绝不外泄
    #[test]
    fn bug14_sentinel_never_leaks() {
        let r = convert("cbrt(a)+hypot(a,b)", &["a", "b"]);

        for doc in &r.used_functions {
            assert!(
                !doc.excel_name.contains("EXCEL"),
                "哨兵名外泄到 UI: {}",
                doc.excel_name
            );
        }
        let cbrt_doc = r.used_functions.iter().find(|d| d.name == "cbrt").unwrap();
        assert_eq!(cbrt_doc.excel_name, "POWER(x,1/3)");

        for f in r.cell_formulas.values() {
            assert!(
                !f.contains("CBRTEXCEL") && !f.contains("HYPOTEXCEL"),
                "公式本体含哨兵: {f}"
            );
        }
    }

    /// BUG-15：告警去重（同一表达式按 cell/value/分支多路转换）
    #[test]
    fn bug15_warnings_deduplicated() {
        let mut s = schema("a+b", &["a", "b"]);
        s.alt_expressions = vec![AltExpression {
            label: "f1".to_string(),
            expression: "a+b".to_string(),
            condition: None,
        }];
        let r = ExcelFormulaConverter::new().convert(&s, &HashMap::new());
        let mut deduped = r.warnings.clone();
        deduped.dedup();
        assert_eq!(r.warnings, deduped);
        assert_eq!(r.warnings.len(), deduped.len());
    }

    /// BUG-11：`log` 的语义约定必须在函数说明里可见
    #[test]
    fn bug11_log_doc_explains_ln_convention() {
        let r = convert("log(a)", &["a"]);
        let doc = r.used_functions.iter().find(|d| d.name == "log").unwrap();
        assert!(
            doc.description.contains("自然对数") && doc.description.contains("log10"),
            "说明应解释约定: {}",
            doc.description
        );
    }

    #[test]
    fn used_functions_sorted_by_name() {
        let r = convert("sqrt(a)+abs(b)+max(a,b)", &["a", "b"]);
        let names: Vec<&str> = r.used_functions.iter().map(|d| d.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
    }

    // ============================================================ 数值代入（单趟扫描）

    #[test]
    fn p0_001_basic_substitution() {
        let r = substitute_values(
            "=PI()*A1^2*B1",
            &ref_map(&[("r", 0), ("h", 1)]),
            &vals(&[("r", "2"), ("h", "3")]),
        );
        assert_eq!(r.formula, "=PI()*2^2*3");
        assert!(r.unresolved_refs.is_empty());
        assert!(r.unresolved_names.is_empty());
        assert!(r.warnings.is_empty(), "不应有告警: {:?}", r.warnings);
    }

    /// 引用出现顺序与变量顺序不同时也要全部替换
    #[test]
    fn p0_002_six_vars_all_substituted() {
        let r = substitute_values(
            "=F1*D1*A1*(B1-C1/2)",
            &ref_map(&[
                ("b", 0),
                ("h0", 1),
                ("As", 2),
                ("fc", 3),
                ("fy", 4),
                ("alpha1", 5),
            ]),
            &vals(&[
                ("b", "200"),
                ("h0", "500"),
                ("As", "1250"),
                ("fc", "14.3"),
                ("fy", "360"),
                ("alpha1", "1"),
            ]),
        );
        assert_eq!(r.formula, "=1*14.3*200*(500-1250/2)");
        assert!(r.unresolved_refs.is_empty());
    }

    /// 历史纵向契约（`A1`,`A2`…）走兼容对齐 + 告警
    #[test]
    fn p0_003_legacy_column_a_alignment() {
        let r = substitute_values(
            "=A1*A2",
            &ref_map(&[("b", 0), ("h0", 1)]),
            &vals(&[("b", "3"), ("h0", "4")]),
        );
        assert_eq!(r.formula, "=3*4");
        assert_eq!(r.warnings.len(), 1);
        assert!(r.warnings[0].contains("对齐"));
    }

    /// 三种绝对引用形态归一化后是同一个引用
    #[test]
    fn p0_004_absolute_ref_variants() {
        let r = substitute_values(
            "=$A$1+A$1+$A1",
            &ref_map(&[("x", 0), ("y", 1), ("z", 2)]),
            &vals(&[("x", "1"), ("y", "2"), ("z", "3")]),
        );
        assert_eq!(r.formula, "=1+1+1");
    }

    #[test]
    fn p0_005_blank_input_becomes_zero() {
        let r = substitute_values(
            "=A1*B1",
            &ref_map(&[("r", 0), ("h", 1)]),
            &vals(&[("r", ""), ("h", "3")]),
        );
        assert_eq!(r.formula, "=0*3");
    }

    #[test]
    fn p0_006_thousands_and_negative_parens() {
        let r = substitute_values(
            "=A1*B1^2",
            &ref_map(&[("r", 0), ("h", 1)]),
            &vals(&[("r", "1,000"), ("h", "-5")]),
        );
        assert_eq!(r.formula, "=1000*(-5)^2");
    }

    /// 科学计数法要输出 Excel 能识别的形式（不允许 `1.0E7`）
    #[test]
    fn p0_007_scientific_notation() {
        let r = substitute_values("=A1*2", &ref_map(&[("r", 0)]), &vals(&[("r", "1e7")]));
        assert_eq!(r.formula, "=10000000*2");

        let big = substitute_values("=A1", &ref_map(&[("r", 0)]), &vals(&[("r", "1e300")]));
        assert_eq!(big.formula, "=1E+300");
    }

    #[test]
    fn p0_008_unresolved_identifier_reported() {
        let r = substitute_values("=A1*x", &ref_map(&[("b", 0)]), &vals(&[("b", "5")]));
        assert_eq!(r.formula, "=5*x");
        assert_eq!(r.unresolved_names, vec!["x"]);
        assert!(!r.warnings.is_empty());
    }

    /// 函数名 `LOG10(` 不能被误判为单元格引用
    #[test]
    fn p0_008b_function_names_not_treated_as_refs() {
        let r = substitute_values("=LOG10(A1)+PI()", &ref_map(&[("x", 0)]), &vals(&[("x", "100")]));
        assert_eq!(r.formula, "=LOG10(100)+PI()");
        assert!(r.unresolved_names.is_empty());
    }

    /// 非法输入值：保留原引用并告警
    #[test]
    fn p0_008c_invalid_value_keeps_ref() {
        let r = substitute_values(
            "=A1+B1",
            &ref_map(&[("a", 0), ("b", 1)]),
            &vals(&[("a", "abc"), ("b", "2")]),
        );
        assert_eq!(r.formula, "=A1+2");
        assert!(r.warnings.iter().any(|w| w.contains('a')));
        assert_eq!(r.unresolved_refs, vec!["A1"]);
    }

    /// 全角数字与全角逗号
    #[test]
    fn p0_008d_full_width_input() {
        let r = substitute_values("=A1", &ref_map(&[("a", 0)]), &vals(&[("a", "１，２００")]));
        assert_eq!(r.formula, "=1200");
    }

    /// 负数括号保护：`-5^2` 在 Excel 里语义错误，必须写成 `(-5)^2`
    #[test]
    fn p0_010b_negative_parens_protect_power() {
        let r = substitute_values("=A1^2", &ref_map(&[("h", 0)]), &vals(&[("h", "-5")]));
        assert_eq!(r.formula, "=(-5)^2");
    }

    /// 字符串字面量里的 `A1` 不能被替换
    #[test]
    fn string_literals_are_not_substituted() {
        let r = substitute_values(
            "=IF(A1>0,\"A1\",B1)",
            &ref_map(&[("a", 0), ("b", 1)]),
            &vals(&[("a", "1"), ("b", "2")]),
        );
        assert_eq!(r.formula, "=IF(1>0,\"A1\",2)");
        assert!(r.unresolved_names.is_empty(), "IF 是已知函数");
    }

    /// 工作表名 `'Sheet'!A1` 里的名字部分原样保留，引用照常替换
    #[test]
    fn sheet_qualified_ref_is_substituted() {
        let r = substitute_values("='My Sheet'!A1", &ref_map(&[("a", 0)]), &vals(&[("a", "7")]));
        assert_eq!(r.formula, "='My Sheet'!7");
    }

    /// 数字字面量不参与替换（`10` 里的 `1` 不是引用）
    #[test]
    fn numeric_literals_are_left_alone() {
        let r = substitute_values("=A1*10", &ref_map(&[("a", 0)]), &vals(&[("a", "3")]));
        assert_eq!(r.formula, "=3*10");
    }

    /// 四字母开头（`ABCD1`）不是合法引用形态，按标识符处理
    #[test]
    fn four_letter_name_is_not_a_ref() {
        let r = substitute_values("=ABCD1+A1", &ref_map(&[("a", 0)]), &vals(&[("a", "1")]));
        assert_eq!(r.formula, "=ABCD1+1");
        assert_eq!(r.unresolved_names, vec!["ABCD1"]);
    }

    /// 无法映射的引用原样保留并告警
    #[test]
    fn unmapped_ref_is_kept_with_warning() {
        let r = substitute_values("=Z9+A1", &ref_map(&[("a", 0)]), &vals(&[("a", "1")]));
        assert_eq!(r.formula, "=Z9+1");
        assert_eq!(r.unresolved_refs, vec!["Z9"]);
        assert!(r.warnings.iter().any(|w| w.contains("无法映射")));
    }
}

