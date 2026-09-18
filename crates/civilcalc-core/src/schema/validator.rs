//! Schema 校验（12 类规则）。
//!
//! 源：`civilcalc-android-v2/core/schema/SchemaValidator.kt`
//!
//! ## 为什么这是「最高优先级」
//!
//! 源项目规格把它列为**工程安全底线**：建筑公式直接用于工程决策，
//! 错误的系数、符号含义、适用条件会造成实际损失。因此：
//!
//! 1. **内置公式必须逐条可溯源**：`verified = true` 时 `ref` 必须非空且命中白名单
//! 2. **AI 不得冒充权威**：`kind = Ai` 时不得 `verified = true`，且**不得自填 `ref`**
//! 3. 内置库加载时逐条校验，**全部失败则拒绝启动**（见 [`crate::source`]）
//!
//! ## ⚠️ 报错文案是**用户可见**的
//!
//! 本文件所有 `reason` 字符串必须与源项目**逐字一致**，
//! 否则 UI 提示与 Android 端不一致（跨端体验分裂）。

use crate::engine::types::CompileResult;
use crate::engine::{self, function_table, lexer::Lexer, parser, splitter};
use crate::schema::formula::{FormulaSchema, SourceKind};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// 校验失败。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{error_detail}")]
pub struct ValidationError {
    /// 用户可见的失败原因
    pub error_detail: String,
}

impl ValidationError {
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            error_detail: detail.into(),
        }
    }
}

/// 校验结果别名（对齐源项目 `FormulaResult<Unit, ValidationError>`）
pub type ValidationResult = Result<(), ValidationError>;

/// 规范来源白名单（**21 项**）。
///
/// 内置公式 `source.ref` 须落在下列规范集方可标 `verified = true`。
/// 匹配方式为 **`contains`（包含）**，非精确匹配 —— 允许 `"GB 50010-2010 第6.2.10条"` 这类完整表述。
///
/// 白名单之外需经人工评审追加。
pub const WHITELIST_REFS: &[&str] = &[
    // 混凝土结构
    "GB 50010",
    "GB 50204",
    // 钢结构
    "GB 50017",
    "GB 50755",
    // 建筑荷载 / 抗震 / 地基
    "GB 50009",
    "GB 50011",
    "GB 50007",
    // 砌体 / 木结构
    "GB 50003",
    "GB 50005",
    // 桩基 / 基坑
    "JGJ 94",
    "JGJ 120",
    "JGJ 79",
    // 脚手架 / 施工安全
    "JGJ 130",
    "JGJ 59",
    "JGJ 101",
    // 混凝土配合比 / 工程测量
    "JGJ 55",
    "GB 50026",
    "CJJ/T 8",
    // 计价 / 标准写作
    "GB 50500",
    "GB/T 50854",
    "GB/T 1.1",
];

/// `ref` 是否命中白名单（`contains` 匹配）
pub fn matches_whitelist(ref_: &str) -> bool {
    WHITELIST_REFS.iter().any(|w| ref_.contains(w))
}

/// 校验 Schema（12 类规则）。
///
/// 校验顺序与源项目**严格一致** —— 因为用户看到的是**第一条**失败原因。
pub fn validate_schema(schema: &FormulaSchema) -> ValidationResult {
    // ---- 1. id 非空 ----
    if schema.id.trim().is_empty() {
        return Err(ValidationError::new("id 不能为空"));
    }

    // ---- 2. id 前缀 ----
    if !schema.id.starts_with("builtin:") && !schema.id.starts_with("usr:") {
        return Err(ValidationError::new(format!(
            "id 必须以 builtin: 或 usr: 开头: {}",
            schema.id
        )));
    }

    // ---- 3-5. 必填字段 ----
    if schema.result_name.trim().is_empty() {
        return Err(ValidationError::new("resultName 不能为空"));
    }
    if schema.result_symbol.trim().is_empty() {
        return Err(ValidationError::new("resultSymbol 不能为空"));
    }
    if schema.expression.trim().is_empty() {
        return Err(ValidationError::new("expression 不能为空"));
    }

    // ---- 6-8. 主表达式按分号分段逐段编译 + 符号声明检查 ----
    //
    // BUG-01：多输出公式每段都必须合法
    let segments = splitter::split(&schema.expression);

    // BUG-02：已声明符号 = 变量 + 自定义常量 + 白名单常量 + 步骤结果符号
    let mut declared: HashSet<String> = HashSet::new();
    for v in &schema.variables {
        declared.insert(v.symbol.clone());
    }
    for k in schema.constants.keys() {
        declared.insert(k.clone());
    }
    for k in function_table::CONSTANTS.keys() {
        declared.insert((*k).to_string());
    }
    if let Some(steps) = &schema.steps_template {
        for st in steps {
            declared.insert(st.symbol.clone());
            // 赋值目标也是结果符号（如步骤写 `x = ...` 时主表达式可引用 x）
            for seg in splitter::split(&st.expression) {
                if let Some(sym) = seg.symbol {
                    declared.insert(sym);
                }
            }
        }
    }

    for (seg_idx, seg) in segments.iter().enumerate() {
        let compiled = match engine::compile(&seg.expression, &schema.constants) {
            CompileResult::Ok { expr } => expr,
            CompileResult::Error { reason, .. } => {
                return Err(ValidationError::new(format!(
                    "主表达式第{}段编译失败: {}",
                    seg_idx + 1,
                    reason
                )));
            }
        };

        let referenced = &compiled.variables;

        // ---- 多结果自引用闸门 ----
        //
        // 方程/方程组必须给出**闭式解**：右侧再出现本段输出符号说明根本没解出来
        // （此时该符号会落到 variables 里当输入用，静默算出一个错值）。
        // 仅对新格式（有 resultOutputs）生效。
        if !schema.result_outputs.is_empty() {
            if let Some(sym) = &seg.symbol {
                if referenced.contains(sym) {
                    return Err(ValidationError::new(format!(
                        "主表达式第{}段「{} = …」右侧含符号 {}，方程未解出（须写成闭式解）",
                        seg_idx + 1,
                        sym,
                        sym
                    )));
                }
            }
        }

        let undeclared: Vec<&String> = referenced
            .iter()
            .filter(|v| !declared.contains(*v))
            .collect();
        if !undeclared.is_empty() {
            let list = undeclared
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(ValidationError::new(format!(
                "主表达式第{}段引用了未声明符号: {}",
                seg_idx + 1,
                list
            )));
        }
    }

    // ---- 9. 多结果清单自洽性（仅 resultOutputs 非空时生效）----
    //
    // 存量公式与内置库完全不受影响。
    if !schema.result_outputs.is_empty() {
        let mut seen_output_symbols: HashSet<String> = HashSet::new();
        for (out_idx, output) in schema.result_outputs.iter().enumerate() {
            if output.symbol.trim().is_empty() {
                return Err(ValidationError::new(format!(
                    "resultOutputs[{out_idx}] symbol 不能为空"
                )));
            }
            if !seen_output_symbols.insert(output.symbol.clone()) {
                return Err(ValidationError::new(format!(
                    "resultOutputs 中符号 '{}' 重复",
                    output.symbol
                )));
            }
            if !segments.iter().any(|s| s.symbol.as_deref() == Some(output.symbol.as_str())) {
                return Err(ValidationError::new(format!(
                    "resultOutputs 的符号 '{}' 未出现在 expression 的分号段赋值目标中",
                    output.symbol
                )));
            }
        }
    }

    // ---- 10. 分支表达式逐个编译 ----
    for alt in &schema.alt_expressions {
        if !alt.expression.trim().is_empty() {
            if let CompileResult::Error { reason, .. } =
                engine::compile(&alt.expression, &schema.constants)
            {
                return Err(ValidationError::new(format!(
                    "分支表达式编译失败 [{}]: {}",
                    alt.label, reason
                )));
            }
        }
    }

    // ---- 11. 变量 ----
    for variable in &schema.variables {
        if variable.symbol.trim().is_empty() {
            return Err(ValidationError::new("变量 symbol 不能为空"));
        }
        if variable.desc.trim().is_empty() {
            return Err(ValidationError::new(format!(
                "变量 {} 缺少 desc (中文释义+单位)",
                variable.symbol
            )));
        }
        // 内置公式（verified=true）的变量必须填 unit（无量纲系数可留空字符串）
        if schema.source.verified && variable.unit.is_none() {
            return Err(ValidationError::new(format!(
                "内置公式变量 {} 必须填写 unit（无量纲系数可留空字符串）",
                variable.symbol
            )));
        }
    }

    // ---- 12. 内置来源校验 ----
    if schema.source.verified {
        let ref_ok = schema
            .source
            .ref_
            .as_ref()
            .is_some_and(|r| !r.trim().is_empty());
        if !ref_ok {
            return Err(ValidationError::new(format!(
                "内置公式必须填写 source.ref: {}",
                schema.id
            )));
        }
        let r = schema.source.ref_.as_deref().unwrap_or_default();
        if !matches_whitelist(r) {
            return Err(ValidationError::new(format!(
                "source.ref 不在白名单，需人工评审: {} -> {}",
                schema.id, r
            )));
        }
    }

    // ---- AI 红线 ----
    if schema.source.kind == SourceKind::Ai && schema.source.verified {
        return Err(ValidationError::new(format!(
            "AI 产出公式不得标记 verified=true: {}",
            schema.id
        )));
    }
    if schema.source.kind == SourceKind::Ai && schema.source.ref_.is_some() {
        return Err(ValidationError::new(format!(
            "AI 产出公式不得自填 source.ref: {}",
            schema.id
        )));
    }

    // ---- 13. stepsTemplate ----
    if let Some(steps) = &schema.steps_template {
        if !steps.is_empty() {
            validate_steps(schema, steps)?;
        }
    }

    Ok(())
}

/// 校验 `stepsTemplate`（含逐段编译、引用检查、symbol 重复检查）。
fn validate_steps(
    schema: &FormulaSchema,
    steps: &[crate::schema::formula::StepTemplate],
) -> ValidationResult {
    // 合并常量与函数（步骤表达式可引用 schema 自定义常量）
    let mut merged_constants: HashMap<String, f64> = function_table::constants_owned();
    for (k, v) in &schema.constants {
        merged_constants.insert(k.clone(), *v);
    }
    let merged_functions: HashSet<String> = function_table::function_name_set();

    let mut seen_symbols: HashSet<String> = HashSet::new();

    for (idx, step) in steps.iter().enumerate() {
        if step.symbol.trim().is_empty() {
            return Err(ValidationError::new(format!(
                "stepsTemplate[{idx}] symbol 不能为空"
            )));
        }
        if step.label.trim().is_empty() {
            return Err(ValidationError::new(format!(
                "stepsTemplate[{idx}] label 不能为空"
            )));
        }
        if step.expression.trim().is_empty() {
            return Err(ValidationError::new(format!(
                "stepsTemplate[{idx}] expression 不能为空"
            )));
        }

        // BUG-03：步骤可含多段子表达式，赋值目标是结果符号
        let sub_segments = splitter::split(&step.expression);
        for seg in &sub_segments {
            let mut lx = Lexer::new(&seg.expression, &merged_constants, &merged_functions);
            let tokens = match lx.tokenize() {
                Ok(t) => t,
                Err(msg) => {
                    return Err(ValidationError::new(format!(
                        "stepsTemplate[{idx}] '{}' 编译失败: {}",
                        step.label, msg
                    )));
                }
            };

            let compiled = match parser::parse(&tokens) {
                CompileResult::Ok { expr } => expr,
                CompileResult::Error { reason, .. } => {
                    return Err(ValidationError::new(format!(
                        "stepsTemplate[{idx}] '{}' 编译失败: {}",
                        step.label, reason
                    )));
                }
            };

            // 引用检查：引用的变量必须是 schema 变量、前序步骤 symbol/赋值目标、或已声明常量
            let mut available: HashSet<String> =
                schema.variables.iter().map(|v| v.symbol.clone()).collect();
            available.extend(seen_symbols.iter().cloned());

            for v in &compiled.variables {
                if !available.contains(v) && !merged_constants.contains_key(v) {
                    return Err(ValidationError::new(format!(
                        "stepsTemplate[{idx}] '{}' 引用了未定义符号 '{}'",
                        step.label, v
                    )));
                }
            }
        }

        // symbol 重复检查 + 绑定本步所有结果符号
        if seen_symbols.contains(&step.symbol) {
            return Err(ValidationError::new(format!(
                "stepsTemplate[{idx}] symbol '{}' 重复定义",
                step.symbol
            )));
        }
        seen_symbols.insert(step.symbol.clone());
        for seg in &sub_segments {
            if let Some(sym) = &seg.symbol {
                seen_symbols.insert(sym.clone());
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::formula::{
        AltExpression, FormulaSource, FormulaVar, ResultOutput, StepTemplate,
        CURRENT_SCHEMA_VERSION,
    };

    /// 构造一个「合法的最小 Schema」（内置来源）
    fn builtin_schema() -> FormulaSchema {
        FormulaSchema {
            id: "builtin:test".to_string(),
            result_name: "测试".to_string(),
            result_symbol: "y".to_string(),
            result_unit: Some("m".to_string()),
            result_outputs: vec![],
            expression: "a+b".to_string(),
            source_equations: vec![],
            alt_expressions: vec![],
            constants: HashMap::new(),
            variables: vec![
                FormulaVar {
                    symbol: "a".to_string(),
                    desc: "参数a".to_string(),
                    unit: Some("m".to_string()),
                    default: None,
                    min: None,
                    max: None,
                    required: true,
                },
                FormulaVar {
                    symbol: "b".to_string(),
                    desc: "参数b".to_string(),
                    unit: Some("m".to_string()),
                    default: None,
                    min: None,
                    max: None,
                    required: true,
                },
            ],
            domain: "通用".to_string(),
            tags: vec![],
            reference_basis: None,
            design_notes: None,
            explanation: None,
            image_ids: vec![],
            revised_from: None,
            source: FormulaSource::standard("GB 50010-2010 第6.2.10条"),
            steps_template: None,
            doc_template_id: None,
            excel_expression: None,
            excel_alt_expressions: None,
            excel_steps_template: None,
            excel_function_docs: None,
            schema_version: CURRENT_SCHEMA_VERSION,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn ai_schema() -> FormulaSchema {
        let mut s = builtin_schema();
        s.id = "usr:test".to_string();
        s.source = FormulaSource::ai(Some("deepseek".to_string()));
        s
    }

    fn err_msg(s: &FormulaSchema) -> String {
        validate_schema(s).expect_err("应校验失败").error_detail
    }

    // ================= 白名单 =================

    #[test]
    fn whitelist_has_21_entries() {
        // 已核对源项目 SchemaValidator.kt：白名单为 21 项
        assert_eq!(WHITELIST_REFS.len(), 21, "白名单应为 21 项");
    }

    #[test]
    fn whitelist_matching_is_contains_based() {
        assert!(matches_whitelist("GB 50010-2010 第6.2.10条"));
        assert!(matches_whitelist("GB 50010"));
        assert!(matches_whitelist("《建筑施工手册》GB 50204 §5.2.1"));
        assert!(!matches_whitelist("GB 99999"));
        assert!(!matches_whitelist("随便写的条款"));
    }

    // ================= 合法用例 =================

    #[test]
    fn valid_builtin_schema_passes() {
        assert!(validate_schema(&builtin_schema()).is_ok());
    }

    #[test]
    fn valid_ai_schema_passes() {
        assert!(validate_schema(&ai_schema()).is_ok());
    }

    // ================= 规则 1-2：id =================

    #[test]
    fn rule1_id_not_blank() {
        let mut s = builtin_schema();
        s.id = "  ".to_string();
        assert_eq!(err_msg(&s), "id 不能为空");
    }

    #[test]
    fn rule2_id_prefix() {
        let mut s = builtin_schema();
        s.id = "test:1".to_string();
        assert_eq!(err_msg(&s), "id 必须以 builtin: 或 usr: 开头: test:1");
    }

    // ================= 规则 3-5：必填 =================

    #[test]
    fn rule3_result_name_not_blank() {
        let mut s = builtin_schema();
        s.result_name = "".to_string();
        assert_eq!(err_msg(&s), "resultName 不能为空");
    }

    #[test]
    fn rule4_result_symbol_not_blank() {
        let mut s = builtin_schema();
        s.result_symbol = "".to_string();
        assert_eq!(err_msg(&s), "resultSymbol 不能为空");
    }

    #[test]
    fn rule5_expression_not_blank() {
        let mut s = builtin_schema();
        s.expression = "".to_string();
        assert_eq!(err_msg(&s), "expression 不能为空");
    }

    // ================= 规则 6-8：编译与符号声明 =================

    #[test]
    fn rule6_expression_must_compile() {
        let mut s = builtin_schema();
        // `a +` 能编译（见 engine 测试：错误在求值阶段才暴露），
        // 因此这里用真正的语法错误
        s.expression = "(a+b".to_string();
        let msg = err_msg(&s);
        assert!(msg.starts_with("主表达式第1段编译失败:"), "实际: {msg}");
    }

    #[test]
    fn rule6_reports_segment_index() {
        let mut s = builtin_schema();
        s.expression = "a+b; (c".to_string();
        let msg = err_msg(&s);
        assert!(msg.starts_with("主表达式第2段编译失败:"), "实际: {msg}");
    }

    #[test]
    fn rule8_undeclared_symbol_reported() {
        let mut s = builtin_schema();
        s.expression = "a+b+c".to_string();
        assert_eq!(err_msg(&s), "主表达式第1段引用了未声明符号: c");
    }

    #[test]
    fn rule8_constants_count_as_declared() {
        let mut s = builtin_schema();
        s.expression = "a+b+k".to_string();
        s.constants.insert("k".to_string(), 2.0);
        assert!(validate_schema(&s).is_ok(), "自定义常量应视为已声明");
    }

    #[test]
    fn rule8_whitelist_constants_count_as_declared() {
        let mut s = builtin_schema();
        s.expression = "a+b+pi".to_string();
        assert!(validate_schema(&s).is_ok(), "pi 是白名单常量");
    }

    // ================= 多结果闸门 =================

    #[test]
    fn multi_result_self_reference_gate() {
        let mut s = builtin_schema();
        // `X = X + 1` —— 方程未解出（右侧含本段输出符号）
        s.expression = "X = X + 1".to_string();
        s.variables.push(FormulaVar {
            symbol: "X".to_string(),
            desc: "未知数".to_string(),
            unit: Some("".to_string()),
            default: None,
            min: None,
            max: None,
            required: true,
        });
        s.result_outputs = vec![ResultOutput {
            symbol: "X".to_string(),
            name: "X".to_string(),
            unit: "".to_string(),
        }];
        let msg = err_msg(&s);
        assert!(
            msg.contains("方程未解出（须写成闭式解）"),
            "应触发自引用闸门，实际: {msg}"
        );
    }

    #[test]
    fn multi_result_closed_form_passes() {
        let mut s = builtin_schema();
        s.expression = "X = 2*a".to_string();
        s.result_outputs = vec![ResultOutput {
            symbol: "X".to_string(),
            name: "X".to_string(),
            unit: "".to_string(),
        }];
        assert!(validate_schema(&s).is_ok(), "闭式解应通过");
    }

    #[test]
    fn multi_result_symbol_must_appear_in_segments() {
        let mut s = builtin_schema();
        s.expression = "a+b".to_string();
        s.result_outputs = vec![ResultOutput {
            symbol: "Z".to_string(),
            name: "Z".to_string(),
            unit: "".to_string(),
        }];
        assert_eq!(
            err_msg(&s),
            "resultOutputs 的符号 'Z' 未出现在 expression 的分号段赋值目标中"
        );
    }

    #[test]
    fn multi_result_symbol_must_be_unique() {
        let mut s = builtin_schema();
        s.expression = "X = 2*a".to_string();
        s.result_outputs = vec![
            ResultOutput {
                symbol: "X".to_string(),
                name: "X".to_string(),
                unit: "".to_string(),
            },
            ResultOutput {
                symbol: "X".to_string(),
                name: "X2".to_string(),
                unit: "".to_string(),
            },
        ];
        assert_eq!(err_msg(&s), "resultOutputs 中符号 'X' 重复");
    }

    #[test]
    fn multi_result_blank_symbol_rejected() {
        let mut s = builtin_schema();
        s.expression = "X = 2*a".to_string();
        s.result_outputs = vec![ResultOutput {
            symbol: "".to_string(),
            name: "".to_string(),
            unit: "".to_string(),
        }];
        assert_eq!(err_msg(&s), "resultOutputs[0] symbol 不能为空");
    }

    /// 存量公式（无 resultOutputs）不受闸门影响
    #[test]
    fn self_reference_allowed_without_result_outputs() {
        let mut s = builtin_schema();
        s.expression = "a+b".to_string();
        s.variables.push(FormulaVar {
            symbol: "X".to_string(),
            desc: "X".to_string(),
            unit: Some("".to_string()),
            default: None,
            min: None,
            max: None,
            required: true,
        });
        // 无 resultOutputs → 闸门不生效
        assert!(validate_schema(&s).is_ok());
    }

    // ================= 规则 10：分支 =================

    #[test]
    fn rule10_branch_must_compile() {
        let mut s = builtin_schema();
        s.alt_expressions = vec![AltExpression {
            label: "分支一".to_string(),
            expression: "(a+b".to_string(),
            condition: None,
        }];
        let msg = err_msg(&s);
        assert_eq!(msg, "分支表达式编译失败 [分支一]: 括号不匹配");
    }

    #[test]
    fn rule10_blank_branch_expression_skipped() {
        let mut s = builtin_schema();
        s.alt_expressions = vec![AltExpression {
            label: "空分支".to_string(),
            expression: "  ".to_string(),
            condition: None,
        }];
        assert!(validate_schema(&s).is_ok(), "空分支表达式跳过编译检查");
    }

    // ================= 规则 11：变量 =================

    #[test]
    fn rule11_variable_symbol_not_blank() {
        let mut s = builtin_schema();
        // 注意：变量 symbol 检查在「未声明符号检查」**之后**，
        // 因此表达式不能引用被清空的符号，否则会先命中规则 8。
        s.variables[0].symbol = String::new();
        s.expression = "b".to_string();
        assert_eq!(err_msg(&s), "变量 symbol 不能为空");
    }

    #[test]
    fn rule11_variable_desc_required() {
        let mut s = builtin_schema();
        s.variables[0].desc = "  ".to_string();
        assert_eq!(err_msg(&s), "变量 a 缺少 desc (中文释义+单位)");
    }

    #[test]
    fn rule11_builtin_variable_unit_required() {
        let mut s = builtin_schema();
        s.variables[0].unit = None;
        assert_eq!(
            err_msg(&s),
            "内置公式变量 a 必须填写 unit（无量纲系数可留空字符串）"
        );
    }

    #[test]
    fn rule11_ai_variable_unit_optional() {
        let mut s = ai_schema();
        s.variables[0].unit = None;
        assert!(validate_schema(&s).is_ok(), "AI 公式变量 unit 可空");
    }

    #[test]
    fn rule11_empty_string_unit_is_allowed_for_dimensionless() {
        let mut s = builtin_schema();
        s.variables[0].unit = Some(String::new());
        assert!(validate_schema(&s).is_ok(), "空字符串表示无量纲，合法");
    }

    // ================= 规则 12：内置来源 =================

    #[test]
    fn rule12_verified_requires_ref() {
        let mut s = builtin_schema();
        s.source.ref_ = None;
        assert_eq!(err_msg(&s), "内置公式必须填写 source.ref: builtin:test");
    }

    #[test]
    fn rule12_blank_ref_rejected() {
        let mut s = builtin_schema();
        s.source.ref_ = Some("   ".to_string());
        assert_eq!(err_msg(&s), "内置公式必须填写 source.ref: builtin:test");
    }

    #[test]
    fn rule12_ref_must_hit_whitelist() {
        let mut s = builtin_schema();
        s.source.ref_ = Some("GB 99999-2020".to_string());
        assert_eq!(
            err_msg(&s),
            "source.ref 不在白名单，需人工评审: builtin:test -> GB 99999-2020"
        );
    }

    // ================= AI 红线 =================

    #[test]
    fn ai_must_not_be_verified() {
        let mut s = ai_schema();
        s.source.verified = true;
        s.source.ref_ = Some("GB 50010".to_string()); // 先绕过 ref 检查
        let msg = err_msg(&s);
        // 注意顺序：verified 且有 ref 时先过白名单，然后命中 AI 红线
        assert!(msg.contains("AI 产出公式不得标记 verified=true"), "实际: {msg}");
    }

    #[test]
    fn ai_must_not_self_assign_ref() {
        let mut s = ai_schema();
        s.source.verified = false;
        s.source.ref_ = Some("GB 50010-2010 第6.2.10条".to_string());
        assert_eq!(
            err_msg(&s),
            "AI 产出公式不得自填 source.ref: usr:test"
        );
    }

    // ================= 规则 13：stepsTemplate =================

    fn step(symbol: &str, label: &str, expression: &str) -> StepTemplate {
        StepTemplate {
            symbol: symbol.to_string(),
            label: label.to_string(),
            group: None,
            expression: expression.to_string(),
            unit: String::new(),
            note: None,
        }
    }

    #[test]
    fn rule13_valid_steps_pass() {
        let mut s = builtin_schema();
        s.steps_template = Some(vec![step("s1", "第一步", "a*b"), step("s2", "第二步", "s1+1")]);
        assert!(validate_schema(&s).is_ok());
    }

    #[test]
    fn rule13_step_symbol_not_blank() {
        let mut s = builtin_schema();
        s.steps_template = Some(vec![step("", "第一步", "a*b")]);
        assert_eq!(err_msg(&s), "stepsTemplate[0] symbol 不能为空");
    }

    #[test]
    fn rule13_step_label_not_blank() {
        let mut s = builtin_schema();
        s.steps_template = Some(vec![step("s1", "", "a*b")]);
        assert_eq!(err_msg(&s), "stepsTemplate[0] label 不能为空");
    }

    #[test]
    fn rule13_step_expression_not_blank() {
        let mut s = builtin_schema();
        s.steps_template = Some(vec![step("s1", "第一步", "  ")]);
        assert_eq!(err_msg(&s), "stepsTemplate[0] expression 不能为空");
    }

    #[test]
    fn rule13_step_expression_must_compile() {
        let mut s = builtin_schema();
        s.steps_template = Some(vec![step("s1", "第一步", "(a+b")]);
        let msg = err_msg(&s);
        assert_eq!(msg, "stepsTemplate[0] '第一步' 编译失败: 括号不匹配");
    }

    #[test]
    fn rule13_step_undefined_symbol_rejected() {
        let mut s = builtin_schema();
        s.steps_template = Some(vec![step("s1", "第一步", "a*zzz")]);
        assert_eq!(
            err_msg(&s),
            "stepsTemplate[0] '第一步' 引用了未定义符号 'zzz'"
        );
    }

    #[test]
    fn rule13_later_step_can_reference_earlier_symbol() {
        let mut s = builtin_schema();
        s.steps_template = Some(vec![
            step("s1", "第一步", "a*b"),
            step("s2", "第二步", "s1*2"),
        ]);
        assert!(validate_schema(&s).is_ok());
    }

    #[test]
    fn rule13_duplicate_step_symbol_rejected() {
        let mut s = builtin_schema();
        s.steps_template = Some(vec![
            step("s1", "第一步", "a*b"),
            step("s1", "第二步", "a+b"),
        ]);
        assert_eq!(err_msg(&s), "stepsTemplate[1] symbol 's1' 重复定义");
    }

    #[test]
    fn rule13_assignment_target_counts_as_defined() {
        let mut s = builtin_schema();
        // 第二步引用第一步的赋值目标 `x`
        s.steps_template = Some(vec![
            step("s1", "第一步", "x = a*b"),
            step("s2", "第二步", "x*2"),
        ]);
        assert!(validate_schema(&s).is_ok());
    }

    #[test]
    fn rule13_empty_steps_template_is_skipped() {
        let mut s = builtin_schema();
        s.steps_template = Some(vec![]);
        assert!(validate_schema(&s).is_ok());
    }

    #[test]
    fn rule13_step_can_use_schema_constants() {
        let mut s = builtin_schema();
        s.constants.insert("k".to_string(), 1.5);
        s.steps_template = Some(vec![step("s1", "第一步", "k*a")]);
        assert!(validate_schema(&s).is_ok());
    }
}
