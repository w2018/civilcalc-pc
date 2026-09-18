//! 存量公式的 Excel 字段迁移（`schemaVersion < 3` → v3）。
//!
//! 源：`FormulaExcelMigrator.kt`（221 行）
//!
//! ## 为什么需要它
//!
//! 契约 v3 改变了单元格映射规则（变量 → 第 1 行横向 `A1`/`B1`/…，步骤结果 → A 列纵向 `A2` 起）。
//! 老数据（`schemaVersion < 3`）里要么没有 `excelExpression`，要么是按**旧规则**生成的 ——
//! 直接导出会给出一份引用错位的 Excel。
//!
//! 迁移**不调用 AI**，纯本地转换：用 [`ExcelFormulaConverter`] 按 v3 契约重算全部 Excel 字段。
//!
//! ## 迁移产物
//!
//! | 字段 | 来源 |
//! |---|---|
//! | `excelExpression` | 单结果 → 主公式；多结果 → `符号 = 公式` 用 `"; "` 拼接 |
//! | `excelAltExpressions` | 逐条转换分支表达式（空表达式原样保留；全空 → `None`） |
//! | `excelStepsTemplate` | 逐条转换步骤表达式，步骤结果符号 → `A(i+2)` |
//! | `excelFunctionDocs` | 主表达式 + 分支用到的函数，按 `name` 排序 |
//! | `schemaVersion` | 置为 `3` |
//!
//! ## 三处实现决策（与源项目的差异）
//!
//! | # | 源项目 | 这里 | 理由 |
//! |---|---|---|---|
//! | 1 | 自己跑词法分析收集函数名（`collectUsedFunctions`） | 直接用 `convert()` 的 `used_functions` | 避免两份函数文档表漂移（`FUNCTION_DOCS` 与源项目的 `functionDocMap` 内容等价） |
//! | 2 | 自己合并常量（`FunctionTable.constants + schema.constants.filterKeys {…}`） | 不合并，交给 `convert()` | `converter` 内部已做同样的「过滤与变量同名的常量」（BUG-09） |
//! | 3 | `catch (e: Exception)` 兜底 | `catch_unwind` 兜底 | Rust 无异常，但 converter 万一 panic 也不该让**整批**迁移中断 —— 迁移发生在启动路径上 |
//!
//! ## 赋值前缀的提取
//!
//! 源项目用正则 `^([A-Za-z_][A-Za-z0-9_']*)\s*=\s*`。这里手写 [`assign_prefix_symbol`]
//! （不引 `regex` 依赖，且行为更可读）。注意标识符允许 `'`（Excel 名称里的合法字符）。

use super::converter::{index_to_cell_ref, ExcelFormulaConverter};
use crate::engine::splitter;
use crate::schema::{AltExpression, FormulaSchema, StepTemplate};
use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};

/// 目标契约版本。`schemaVersion >= 目标` 的公式**不再迁移**。
///
/// 直接引用 [`crate::schema::CURRENT_SCHEMA_VERSION`] 而不是硬编码 `3`：
/// 契约版本升级时迁移目标自动跟随，不会出现「两处版本号不一致」。
/// （若将来需要「v3 → v4」的增量迁移，那时再引入按版本分派的结构。）
pub const TARGET_SCHEMA_VERSION: i32 = crate::schema::CURRENT_SCHEMA_VERSION;

/// 批量迁移统计。
///
/// `failed` 在当前实现下**恒为 0** —— 迁移是纯函数，没有可失败的分支
/// （converter 对编译失败也只是回退原式 + 告警，不报错）。
/// 保留该字段是为了对齐源项目接口，且 `catch_unwind` 兜底时它会真的增长。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MigrationResult {
    /// 实际完成迁移的条数
    pub migrated: usize,
    /// 已是 v3、跳过的条数
    pub skipped: usize,
    /// 迁移失败的条数（当前实现下为 0，见结构体文档）
    pub failed: usize,
}

impl MigrationResult {
    /// 处理过的总条数
    pub fn total(&self) -> usize {
        self.migrated + self.skipped + self.failed
    }
}

/// 迁移单个公式。
///
/// - `schemaVersion >= 3` → **原样返回**（不重复转换，避免覆盖用户/AI 手工修正过的 Excel 字段）
/// - 否则按 v3 契约重算全部 Excel 字段，并把 `schemaVersion` 置为 3
///
/// 转换过程中产生的告警（如某段表达式编译失败）**不会**中断迁移 ——
/// 那一段会保留原式，其余字段照常生成。用户在 Excel 面板里能看到告警。
pub fn migrate_schema(schema: &FormulaSchema) -> FormulaSchema {
    if schema.schema_version >= TARGET_SCHEMA_VERSION {
        return schema.clone();
    }

    let converter = ExcelFormulaConverter::new();
    let result = converter.convert(schema, &HashMap::new());

    // 变量 → 第 1 行横向（去重保序；单元格编号依赖顺序）
    let variables = distinct_variables(schema);
    let base_ref_map: HashMap<String, String> = variables
        .iter()
        .enumerate()
        .map(|(idx, sym)| (sym.clone(), index_to_cell_ref(idx)))
        .collect();

    // schema 自定义常量（**排除与变量同名的** —— 变量优先，BUG-09）
    let mut constants = HashMap::new();
    for (k, v) in &schema.constants {
        if !variables.contains(k) {
            constants.insert(k.clone(), *v);
        }
    }

    // ── excelStepsTemplate ──
    // 契约：变量 → 第 1 行横向；**前序**步骤结果 → A 列（步骤 i → A(i+2)，即 A2 起）
    let excel_steps = schema.steps_template.as_ref().map(|steps| {
        let mut step_cells: HashMap<String, String> = HashMap::new();
        let mut out = Vec::with_capacity(steps.len());

        for (step_idx, step) in steps.iter().enumerate() {
            let stripped = step
                .expression
                .strip_prefix('=')
                .unwrap_or(&step.expression)
                .trim()
                .to_string();

            // 步骤含多段时**只取末段**（与引擎口径「步骤结果 = 最后一段」一致）；
            // 旧实现把含 `;` 的整条交给 Lexer，必然编译失败并原样返回。
            let segments = splitter::split(&stripped);
            let last = segments.last();

            // 步骤结果符号 = 表达式左侧赋值目标，缺省回退 `step.symbol`
            let result_symbol = last
                .and_then(|s| s.symbol.clone())
                .or_else(|| assign_prefix_symbol(&stripped))
                .unwrap_or_else(|| step.symbol.clone());

            let clean_expr = if segments.len() > 1 {
                last.map(|s| s.expression.clone()).unwrap_or_default()
            } else {
                splitter::strip_assignment(&stripped)
            };

            // 本步可见的引用表 = 变量（第 1 行）+ 前序步骤结果（A 列）
            let mut ref_map = base_ref_map.clone();
            ref_map.extend(step_cells.iter().map(|(k, v)| (k.clone(), v.clone())));

            let mut warnings = Vec::new();
            let excel_expr =
                converter.convert_expression(&clean_expr, &ref_map, &mut warnings, &constants);

            // ⚠️ 在转换**之后**登记本步结果：本步表达式看不到自己，后续步骤才能看到
            if !result_symbol.trim().is_empty() {
                step_cells.insert(result_symbol, format!("A{}", step_idx + 2));
            }

            out.push(StepTemplate {
                expression: format!("={excel_expr}"),
                ..step.clone()
            });
        }

        out
    });

    // ── excelAltExpressions ──
    // BUG-13：逐条精确转换，不按 `cellFormulas` 下标取值
    //（旧实现依赖 Map 插入顺序，分支空表达式或 label 重名时会串位/丢失）
    let excel_alt: Vec<AltExpression> = schema
        .alt_expressions
        .iter()
        .map(|alt| {
            if alt.expression.trim().is_empty() {
                return alt.clone();
            }
            let mut warnings = Vec::new();
            let formula =
                converter.convert_expression(&alt.expression, &base_ref_map, &mut warnings, &constants);
            AltExpression {
                expression: format!("={formula}"),
                ..alt.clone()
            }
        })
        .collect();

    // ── excelExpression ──
    // 多结果逐段写成「符号 = Excel公式」再拼接，与 AI 生成的 excelExpression 同一格式；
    // 存量公式（内置坐标正/反算、GPS 基线）迁移后不再是一条断式。
    let excel_expression = if result.output_formulas.len() > 1 {
        Some(
            result
                .output_formulas
                .iter()
                .map(|o| format!("{} = {}", o.symbol, o.cell_formula))
                .collect::<Vec<_>>()
                .join("; "),
        )
    } else {
        result.cell_formulas.get("主公式").cloned()
    };

    FormulaSchema {
        excel_expression,
        excel_alt_expressions: if excel_alt.is_empty() {
            None
        } else {
            Some(excel_alt)
        },
        excel_steps_template: excel_steps,
        // 函数清单：直接用转换器收集的结果（含主表达式 + 多结果分段 + 分支），已按 name 排序
        excel_function_docs: Some(result.used_functions),
        schema_version: TARGET_SCHEMA_VERSION,
        ..schema.clone()
    }
}

/// 批量迁移。
///
/// 返回迁移后的列表（**顺序与输入一致**，长度不变）与统计。
///
/// ⚠️ 单条迁移 panic **不会**中断整批 —— 迁移发生在启动路径上，
/// 一条坏数据不该让应用起不来。panic 的那条按「失败」计入并保留原样。
pub fn migrate_all(schemas: &[FormulaSchema]) -> (Vec<FormulaSchema>, MigrationResult) {
    let mut stats = MigrationResult::default();

    let out = schemas
        .iter()
        .map(|schema| {
            if schema.schema_version >= TARGET_SCHEMA_VERSION {
                stats.skipped += 1;
                return schema.clone();
            }

            match catch_unwind(AssertUnwindSafe(|| migrate_schema(schema))) {
                Ok(new_schema) => {
                    if new_schema.schema_version >= TARGET_SCHEMA_VERSION {
                        stats.migrated += 1;
                    } else {
                        stats.failed += 1;
                    }
                    new_schema
                }
                Err(_) => {
                    crate::log::e(
                        "ExcelMigrator",
                        &format!(
                            "schema 迁移 panic，保持 v{}: {}",
                            schema.schema_version, schema.id
                        ),
                        None,
                    );
                    stats.failed += 1;
                    schema.clone()
                }
            }
        })
        .collect();

    (out, stats)
}

/// 去重但**保持首次出现顺序**（单元格编号依赖它）。
fn distinct_variables(schema: &FormulaSchema) -> Vec<String> {
    let mut seen = HashSet::new();
    schema
        .variables
        .iter()
        .map(|v| v.symbol.clone())
        .filter(|s| seen.insert(s.clone()))
        .collect()
}

/// 从 `V = expr` 里取出 `V`。
///
/// 对齐源项目正则 `^([A-Za-z_][A-Za-z0-9_']*)\s*=\s*`：
/// - 首字符必须是 ASCII 字母或 `_`
/// - 后续允许 ASCII 字母 / 数字 / `_` / `'`（`'` 是 Excel 名称里的合法字符）
/// - 之后允许任意空白，然后必须是 `=`
///
/// 这是 [`splitter::split`] 没能识别出符号时的兜底。
fn assign_prefix_symbol(expr: &str) -> Option<String> {
    let bytes = expr.as_bytes();

    if !matches!(bytes.first(), Some(c) if c.is_ascii_alphabetic() || *c == b'_') {
        return None;
    }

    let mut i = 1;
    while i < bytes.len()
        && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'\'')
    {
        i += 1;
    }

    let symbol = &expr[..i];
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }

    if bytes.get(i) == Some(&b'=') {
        Some(symbol.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{FormulaSource, FormulaVar, ResultOutput, SourceKind};

    // ============================================================ 测试助手

    fn var(symbol: &str) -> FormulaVar {
        FormulaVar {
            symbol: symbol.to_string(),
            desc: symbol.to_string(),
            unit: Some(String::new()),
            default: None,
            min: None,
            max: None,
            required: true,
        }
    }

    fn schema(expression: &str, symbols: &[&str], version: i32) -> FormulaSchema {
        FormulaSchema {
            id: "usr:legacy".to_string(),
            result_name: "老公式".to_string(),
            result_symbol: "X".to_string(),
            result_unit: Some(String::new()),
            result_outputs: Vec::new(),
            expression: expression.to_string(),
            source_equations: Vec::new(),
            alt_expressions: Vec::new(),
            constants: HashMap::new(),
            variables: symbols.iter().map(|s| var(s)).collect(),
            domain: String::new(),
            tags: Vec::new(),
            reference_basis: None,
            design_notes: None,
            explanation: None,
            image_ids: Vec::new(),
            revised_from: None,
            source: FormulaSource {
                kind: SourceKind::Standard,
                ref_: None,
                verified: true,
                model: None,
                created_by: None,
            },
            steps_template: None,
            doc_template_id: None,
            excel_expression: None,
            excel_alt_expressions: None,
            excel_steps_template: None,
            excel_function_docs: None,
            schema_version: version,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn step(symbol: &str, expression: &str) -> StepTemplate {
        StepTemplate {
            symbol: symbol.to_string(),
            label: symbol.to_string(),
            group: None,
            expression: expression.to_string(),
            unit: String::new(),
            note: None,
        }
    }

    // ============================================================ 跳过

    #[test]
    fn already_v3_is_returned_untouched() {
        let mut s = schema("a+b", &["a", "b"], 3);
        s.excel_expression = Some("=A1+B1".to_string());
        let out = migrate_schema(&s);
        assert_eq!(out, s, "v3 公式必须原样返回（不覆盖手工修正的字段）");
    }

    #[test]
    fn higher_version_is_also_skipped() {
        let s = schema("a+b", &["a", "b"], 9);
        assert_eq!(migrate_schema(&s), s);
    }

    // ============================================================ 单结果

    #[test]
    fn migrates_single_result_to_main_formula() {
        let out = migrate_schema(&schema("sqrt(a)+b", &["a", "b"], 1));
        assert_eq!(out.excel_expression.as_deref(), Some("=SQRT(A1)+B1"));
        assert_eq!(out.schema_version, 3);
        assert!(out.excel_steps_template.is_none(), "无步骤模板 → None");
        assert!(
            out.excel_alt_expressions.is_none(),
            "无分支 → None（不是空列表）"
        );
    }

    /// 迁移不碰非 Excel 字段
    #[test]
    fn migration_preserves_other_fields() {
        let mut s = schema("a+b", &["a", "b"], 1);
        s.domain = "结构".to_string();
        s.tags = vec!["受弯".to_string()];
        s.design_notes = Some("备注".to_string());

        let out = migrate_schema(&s);
        assert_eq!(out.domain, "结构");
        assert_eq!(out.tags, vec!["受弯"]);
        assert_eq!(out.design_notes.as_deref(), Some("备注"));
        assert_eq!(out.id, "usr:legacy");
        assert_eq!(out.expression, "a+b", "原表达式不动");
    }

    // ============================================================ 多结果

    #[test]
    fn migrates_multi_result_to_semicolon_joined() {
        let mut s = schema("x = a+b; y = a-b", &["a", "b"], 2);
        s.result_outputs = vec![
            ResultOutput {
                symbol: "x".to_string(),
                name: "x".to_string(),
                unit: String::new(),
            },
            ResultOutput {
                symbol: "y".to_string(),
                name: "y".to_string(),
                unit: String::new(),
            },
        ];

        let out = migrate_schema(&s);
        assert_eq!(
            out.excel_expression.as_deref(),
            Some("x = =A1+B1; y = =A1-B1"),
            "多结果应写成「符号 = 公式」并用 ; 拼接"
        );
    }

    // ============================================================ 分支

    #[test]
    fn migrates_alt_expressions_one_by_one() {
        let mut s = schema("a+b", &["a", "b"], 1);
        s.alt_expressions = vec![
            AltExpression {
                label: "分支1".to_string(),
                expression: "a-b".to_string(),
                condition: Some("a>b".to_string()),
            },
            AltExpression {
                label: "分支2".to_string(),
                expression: "a*b".to_string(),
                condition: None,
            },
        ];

        let out = migrate_schema(&s);
        let alts = out.excel_alt_expressions.expect("应有分支");
        assert_eq!(alts.len(), 2);
        assert_eq!(alts[0].expression, "=A1-B1");
        assert_eq!(alts[1].expression, "=A1*B1");
        // label 与 condition 原样保留
        assert_eq!(alts[0].label, "分支1");
        assert_eq!(alts[0].condition.as_deref(), Some("a>b"));
    }

    /// 空白分支表达式**原样保留**（不生成 `=` 这种空公式）
    #[test]
    fn blank_alt_expression_is_left_alone() {
        let mut s = schema("a+b", &["a", "b"], 1);
        s.alt_expressions = vec![AltExpression {
            label: "空分支".to_string(),
            expression: "   ".to_string(),
            condition: None,
        }];

        let out = migrate_schema(&s);
        let alts = out.excel_alt_expressions.expect("应有分支");
        assert_eq!(alts[0].expression, "   ", "空白分支不应被改写");
    }

    // ============================================================ 步骤

    #[test]
    fn migrates_steps_with_column_a_mapping() {
        let mut s = schema("V = a+b", &["a", "b"], 1);
        s.steps_template = Some(vec![step("V1", "a+b"), step("V2", "a*b")]);

        let out = migrate_schema(&s);
        let steps = out.excel_steps_template.expect("应有步骤");
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].expression, "=A1+B1");
        assert_eq!(steps[1].expression, "=A1*B1");
        // label / symbol / unit 保留
        assert_eq!(steps[0].symbol, "V1");
    }

    /// **后续步骤可引用前序步骤结果**（步骤 i → `A(i+2)`）
    #[test]
    fn later_steps_can_reference_earlier_step_results() {
        let mut s = schema("V1 = a+b", &["a", "b"], 1);
        s.steps_template = Some(vec![step("V1", "V1 = a+b"), step("V2", "V1*a")]);

        let out = migrate_schema(&s);
        let steps = out.excel_steps_template.expect("应有步骤");
        assert_eq!(steps[0].expression, "=A1+B1", "第一步：变量在 A1/B1");
        assert_eq!(
            steps[1].expression, "=A2*A1",
            "第二步：V1 → A2（前序步骤结果），a → A1"
        );
    }

    /// 步骤 i 的结果单元格是 `A(i+2)`（即第 0 步 → A2）
    #[test]
    fn step_result_cells_start_at_a2() {
        let mut s = schema("V1 = a", &["a"], 1);
        s.steps_template = Some(vec![
            step("V1", "V1 = a"),
            step("V2", "V1*2"),
            step("V3", "V2+1"),
        ]);

        let out = migrate_schema(&s);
        let steps = out.excel_steps_template.expect("应有步骤");
        assert_eq!(steps[0].expression, "=A1", "V1 = a → a 在 A1");
        assert_eq!(steps[1].expression, "=A2*2", "V1 → A2");
        assert_eq!(steps[2].expression, "=A3+1", "V2 → A3");
    }

    /// 步骤结果符号缺省回退 `step.symbol`
    #[test]
    fn step_result_symbol_falls_back_to_step_symbol() {
        let mut s = schema("a+b", &["a", "b"], 1);
        // 表达式无赋值前缀 → 用 step.symbol 登记
        s.steps_template = Some(vec![step("V1", "a+b"), step("V2", "V1*2")]);

        let out = migrate_schema(&s);
        let steps = out.excel_steps_template.expect("应有步骤");
        assert_eq!(steps[1].expression, "=A2*2", "V1 应回退到 step.symbol 并映射到 A2");
    }

    /// 步骤含多段时只取末段
    #[test]
    fn step_with_multiple_segments_takes_last() {
        let mut s = schema("V = a+b", &["a", "b"], 1);
        s.steps_template = Some(vec![step("V", "t = a*b; V = t+b")]);

        let out = migrate_schema(&s);
        let steps = out.excel_steps_template.expect("应有步骤");
        // 末段是 `V = t+b`；`t` 不在任何映射表里 → 保留原名
        assert_eq!(steps[0].expression, "=t+B1");
    }

    /// 步骤表达式带前导 `=` 也能处理
    #[test]
    fn step_expression_with_leading_equals() {
        let mut s = schema("V = a+b", &["a", "b"], 1);
        s.steps_template = Some(vec![step("V", "= V = a+b")]);

        let out = migrate_schema(&s);
        let steps = out.excel_steps_template.expect("应有步骤");
        assert_eq!(steps[0].expression, "=A1+B1");
    }

    // ============================================================ 函数文档

    #[test]
    fn collects_function_docs_sorted() {
        let out = migrate_schema(&schema("sqrt(a)+abs(b)+max(a,b)", &["a", "b"], 1));
        let docs = out.excel_function_docs.expect("应有函数文档");
        let names: Vec<&str> = docs.iter().map(|d| d.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted, "函数文档应按 name 排序");
        assert_eq!(names, ["abs", "max", "sqrt"]);
    }

    /// BUG-11：`log` 的约定必须在文档里可见
    #[test]
    fn log_doc_explains_ln_convention() {
        let out = migrate_schema(&schema("log(a)+b", &["a", "b"], 1));
        let docs = out.excel_function_docs.expect("应有函数文档");
        let log_doc = docs.iter().find(|d| d.name == "log").expect("应有 log");
        assert_eq!(log_doc.excel_name, "LN");
        assert!(
            log_doc.description.contains("自然对数") && log_doc.description.contains("log10"),
            "应解释约定: {}",
            log_doc.description
        );
    }

    /// 分支里的函数也要收集
    #[test]
    fn collects_functions_from_alt_expressions_too() {
        let mut s = schema("a+b", &["a", "b"], 1);
        s.alt_expressions = vec![AltExpression {
            label: "分支".to_string(),
            expression: "sqrt(a)".to_string(),
            condition: None,
        }];

        let out = migrate_schema(&s);
        let docs = out.excel_function_docs.expect("应有函数文档");
        assert!(
            docs.iter().any(|d| d.name == "sqrt"),
            "分支里的函数应被收集: {:?}",
            docs.iter().map(|d| &d.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn no_functions_yields_empty_doc_list() {
        let out = migrate_schema(&schema("a+b", &["a", "b"], 1));
        assert_eq!(
            out.excel_function_docs,
            Some(Vec::new()),
            "无函数时是 Some(空列表)，不是 None"
        );
    }

    // ============================================================ 常量

    /// 自定义常量（不与变量同名）参与转换 → 输出字面数值
    #[test]
    fn custom_constant_emits_literal() {
        let mut s = schema("alpha*a", &["a"], 1);
        s.constants = HashMap::from([("alpha".to_string(), 1.5)]);
        let out = migrate_schema(&s);
        assert_eq!(out.excel_expression.as_deref(), Some("=1.5*A1"));
    }

    /// BUG-09：与变量同名的常量被过滤（变量优先）
    #[test]
    fn constant_shadowed_by_variable_is_filtered() {
        let mut s = schema("k*b", &["k", "b"], 1);
        s.constants = HashMap::from([("k".to_string(), 9.0)]);
        let out = migrate_schema(&s);
        assert_eq!(
            out.excel_expression.as_deref(),
            Some("=A1*B1"),
            "k 是变量 → 输出单元格引用，不是字面量 9"
        );
    }

    // ============================================================ 变量去重与单元格

    #[test]
    fn duplicate_variables_map_to_single_cell() {
        let out = migrate_schema(&schema("a+a", &["a", "a"], 1));
        assert_eq!(out.excel_expression.as_deref(), Some("=A1+A1"));
    }

    #[test]
    fn variables_map_across_columns() {
        let symbols: Vec<String> = (0..28).map(|i| format!("v{i}")).collect();
        let refs: Vec<&str> = symbols.iter().map(String::as_str).collect();
        let expr = symbols.join("+");
        let out = migrate_schema(&schema(&expr, &refs, 1));
        let formula = out.excel_expression.expect("应有公式");
        assert!(formula.contains("A1"), "第 1 个变量 → A1");
        assert!(formula.contains("Z1"), "第 26 个变量 → Z1");
        assert!(formula.contains("AB1"), "第 28 个变量 → AB1");
    }

    // ============================================================ 编译失败

    /// 表达式编译失败 → 回退原式（不 panic，不中断迁移）
    #[test]
    fn compile_failure_falls_back_to_original() {
        let out = migrate_schema(&schema("a + (b", &["a", "b"], 1));
        assert_eq!(out.excel_expression.as_deref(), Some("=a + (b"));
        assert_eq!(out.schema_version, 3, "编译失败也照样升级版本号");
    }

    // ============================================================ migrate_all

    #[test]
    fn migrate_all_counts_migrated_and_skipped() {
        let schemas = vec![
            schema("a+b", &["a", "b"], 1),
            schema("a*b", &["a", "b"], 3),
            schema("a-b", &["a", "b"], 2),
        ];
        let (out, stats) = migrate_all(&schemas);

        assert_eq!(stats.migrated, 2);
        assert_eq!(stats.skipped, 1);
        assert_eq!(stats.failed, 0);
        assert_eq!(stats.total(), 3);
        assert_eq!(out.len(), 3, "长度不变");
        assert!(out.iter().all(|s| s.schema_version >= 3));
    }

    /// 顺序与输入一致
    #[test]
    fn migrate_all_preserves_order() {
        let schemas = vec![
            schema("a+1", &["a"], 1),
            schema("a+2", &["a"], 1),
            schema("a+3", &["a"], 1),
        ];
        let (out, _) = migrate_all(&schemas);
        let exprs: Vec<&str> = out
            .iter()
            .map(|s| s.excel_expression.as_deref().unwrap_or(""))
            .collect();
        assert_eq!(exprs, ["=A1+1", "=A1+2", "=A1+3"]);
    }

    #[test]
    fn migrate_all_on_empty_input() {
        let (out, stats) = migrate_all(&[]);
        assert!(out.is_empty());
        assert_eq!(stats, MigrationResult::default());
    }

    #[test]
    fn migrate_all_leaves_v3_fields_untouched() {
        let mut v3 = schema("a+b", &["a", "b"], 3);
        v3.excel_expression = Some("=手工修正过的".to_string());
        let (out, stats) = migrate_all(&[v3.clone()]);

        assert_eq!(stats.skipped, 1);
        assert_eq!(
            out[0].excel_expression.as_deref(),
            Some("=手工修正过的"),
            "v3 公式的 Excel 字段不能被覆盖"
        );
    }

    // ============================================================ assign_prefix_symbol

    #[test]
    fn assign_prefix_symbol_basic() {
        assert_eq!(assign_prefix_symbol("V = a+b").as_deref(), Some("V"));
        assert_eq!(assign_prefix_symbol("V=a+b").as_deref(), Some("V"));
        assert_eq!(assign_prefix_symbol("V   =   a+b").as_deref(), Some("V"));
        assert_eq!(assign_prefix_symbol("V1 = a").as_deref(), Some("V1"));
        assert_eq!(assign_prefix_symbol("_x = a").as_deref(), Some("_x"));
    }

    /// `'` 是 Excel 名称里的合法字符，允许出现在标识符中
    #[test]
    fn assign_prefix_symbol_allows_apostrophe() {
        assert_eq!(assign_prefix_symbol("V' = a").as_deref(), Some("V'"));
    }

    #[test]
    fn assign_prefix_symbol_rejects_non_assignment() {
        assert_eq!(assign_prefix_symbol("a+b"), None, "无等号");
        assert_eq!(assign_prefix_symbol("1 = a"), None, "首字符不能是数字");
        assert_eq!(assign_prefix_symbol("= a+b"), None, "以 = 开头");
        assert_eq!(assign_prefix_symbol(""), None);
        assert_eq!(assign_prefix_symbol("  V = a"), None, "前导空白不匹配 ^");
        assert_eq!(assign_prefix_symbol("V + a"), None, "跳过空白后不是 =");
    }

    /// `V == a` 里 `==` 不是赋值 —— 我们的判定只看「跳过空白后是否 `=`」，
    /// 所以会取出 `V`。这是与源项目正则一致的**宽松**行为（正则也只匹配一个 `=`）。
    #[test]
    fn assign_prefix_symbol_is_lenient_like_source_regex() {
        assert_eq!(assign_prefix_symbol("V == a").as_deref(), Some("V"));
    }

    // ============================================================ distinct_variables

    #[test]
    fn distinct_variables_keeps_first_occurrence_order() {
        let s = schema("a+b+a", &["b", "a", "b"], 1);
        assert_eq!(distinct_variables(&s), ["b", "a"]);
    }
}
