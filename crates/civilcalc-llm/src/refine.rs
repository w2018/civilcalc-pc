//! 公式续写微调辅助（**纯逻辑，无 IO**）。
//!
//! 源：`civilcalc-android-v2/core/llm/FormulaRefine.kt`（83 行）
//!
//! 两件事：
//!
//! 1. [`next_version_name`] —— 为新公式生成版本化名称（`原名-v1.0`、`原名-v2.0`…）
//! 2. [`build_revise_request`] —— 把「既有公式的结构摘要 + 微调需求」拼成续写请求文本
//!
//! ## 为什么摘要要列这么全
//!
//! 源注释：*「让模型看清变量/单位/默认值/步骤/分支后再改，避免微调后丢步骤、丢单位、丢分支」*。
//! 实测少一项就会退化：漏 `resultOutputs` → 微调后被改回单结果；漏单位 → 单位丢失。
//!
//! ## ⚠️ 与源的一处偏差：`constants` 按键排序
//!
//! 源用 Kotlin `Map`（JSON 反序列化后保留**键的插入顺序**），摘要里常量按原顺序输出，
//! 以保证提示词前缀稳定（命中 KV cache）。PC 的 `FormulaSchema.constants` 是
//! `HashMap`，**顺序本就不确定** —— 直接把 `HashMap` 迭代结果拼进去会让每次请求的
//! 提示词都不同，**彻底破坏缓存**。
//!
//! 因此这里**按键排序**：牺牲「与源完全同序」，换回「同一条公式每次拼出的文本完全相同」。
//! 其余字段（变量/分支/步骤）在 PC 侧都是 `Vec`，天然保序，与源一致。

use civilcalc_core::result_outputs::build_declared_summary;
use civilcalc_core::schema::{image_marker, FormulaSchema};

/// 续写请求里 `constants` 的排序键。
///
/// 见模块文档：`HashMap` 无序，必须显式排序才能保证提示词前缀稳定。
fn sorted_constants(schema: &FormulaSchema) -> Vec<(&str, f64)> {
    let mut items: Vec<(&str, f64)> = schema
        .constants
        .iter()
        .map(|(k, v)| (k.as_str(), *v))
        .collect();
    items.sort_by(|a, b| a.0.cmp(b.0));
    items
}

/// 计算续写公式名称：剥掉原有 `-vN.0` 后缀取基础名，版本号 +1。
///
/// - `圆柱体积` → `圆柱体积-v1.0`
/// - `圆柱体积-v1.0` → `圆柱体积-v2.0`
///
/// 名称为空或**全部被剥掉**时回退基础名 `微调公式`。
#[must_use]
pub fn next_version_name(result_name: &str) -> String {
    let trimmed = result_name.trim();
    let (base, major) = match_version(trimmed);
    let safe_base = if base.trim().is_empty() {
        "微调公式"
    } else {
        base.as_str()
    };
    format!("{safe_base}-v{}.0", major + 1)
}

/// 返回 `(基础名, 当前主版本号)`。
///
/// 无 `-vN.0` 后缀时主版本号记 0；形如 `-vx.0` 的**非法**后缀一并剥离。
///
/// 等价于源的两个正则：
/// - `^(.*?)-v(\d+)\.0$`（惰性：取**最左**的合法 `-v<数字>.0`）
/// - `^(.*)-v.*\.0$`（贪婪：取**最右**的 `-v….0`）
///
/// 不引 `regex` crate —— 本机离线，且这两个模式用字符串切分足够表达。
#[must_use]
pub fn match_version(name: &str) -> (String, i32) {
    // ① 合法后缀：`-v<digits>.0` 且**恰好**收尾
    for (i, _) in name.match_indices("-v") {
        let rest = &name[i + 2..];
        if let Some(digits) = rest.strip_suffix(".0") {
            if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
                let major = digits.parse::<i32>().unwrap_or(0);
                return (name[..i].to_string(), major);
            }
        }
    }
    // ② 非法后缀（如 `-vx.0`）：贪婪取最右
    let mut last: Option<usize> = None;
    for (i, _) in name.match_indices("-v") {
        if name[i + 2..].ends_with(".0") {
            last = Some(i);
        }
    }
    if let Some(i) = last {
        return (name[..i].to_string(), 0);
    }
    (name.to_string(), 0)
}

/// 组装续写请求的 user 消息。
///
/// 给出既有公式的结构摘要（字段顺序固定，便于命中缓存）；
/// **不带** Excel 字段（本地可重算）与历史版本（避免前缀膨胀）。
///
/// 既有详解只取**纯文本**：附图标记是渲染指令，带进续写上下文只会让模型照抄旧序号。
#[must_use]
pub fn build_revise_request(schema: &FormulaSchema, user_input: &str) -> String {
    let mut out = String::new();

    out.push_str(&format!(
        "既有公式：{}（结果符号 {}，单位 {}）\n",
        schema.result_name,
        schema.result_symbol,
        schema.result_unit.as_deref().unwrap_or("")
    ));
    out.push_str(&format!("主表达式：{}\n", schema.expression));

    // 多结果（方程组等）：把并列输出一并交代，否则微调后会被改回单结果、单位也会丢
    if !schema.result_outputs.is_empty() {
        out.push_str(&format!(
            "结果输出：{}\n",
            build_declared_summary(&schema.result_outputs)
        ));
    }

    if !schema.alt_expressions.is_empty() {
        let joined = schema
            .alt_expressions
            .iter()
            .map(|alt| {
                let cond = alt
                    .condition
                    .as_ref()
                    .map(|c| format!("（条件：{c}）"))
                    .unwrap_or_default();
                format!("{}:{}{}", alt.label, alt.expression, cond)
            })
            .collect::<Vec<_>>()
            .join("；");
        out.push_str(&format!("分支公式：{joined}\n"));
    }

    if !schema.constants.is_empty() {
        let joined = sorted_constants(schema)
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("；");
        out.push_str(&format!("常量：{joined}\n"));
    }

    if !schema.variables.is_empty() {
        let joined = schema
            .variables
            .iter()
            .map(|v| {
                let unit = v
                    .unit
                    .as_deref()
                    .filter(|u| !u.trim().is_empty())
                    .map(|u| format!("，{u}"))
                    .unwrap_or_default();
                let default = v
                    .default
                    .map(|d| format!("，默认{d}"))
                    .unwrap_or_default();
                format!("{}({}{unit}{default})", v.symbol, v.desc)
            })
            .collect::<Vec<_>>()
            .join("；");
        out.push_str(&format!("变量：{joined}\n"));
    }

    if let Some(steps) = schema.steps_template.as_ref().filter(|s| !s.is_empty()) {
        let joined = steps
            .iter()
            .map(|s| format!("{}={}（{}）", s.symbol, s.expression, s.label))
            .collect::<Vec<_>>()
            .join("；");
        out.push_str(&format!("步骤：{joined}\n"));
    }

    if !schema.domain.trim().is_empty() {
        out.push_str(&format!("适用条件：{}\n", schema.domain));
    }
    if let Some(b) = schema.reference_basis.as_deref().filter(|b| !b.trim().is_empty()) {
        out.push_str(&format!("依据：{b}\n"));
    }
    if let Some(n) = schema.design_notes.as_deref().filter(|n| !n.trim().is_empty()) {
        out.push_str(&format!("设计要点：{n}\n"));
    }

    if let Some(exp) = schema.explanation.as_ref() {
        // 逐段输出，并记下「到底输出了没有」—— 三段全空时不追加收尾说明，
        // 否则一条空详解会凭空多出一行提示
        let mut emitted = false;
        if !exp.summary.trim().is_empty() {
            out.push_str(&format!(
                "既有需求理解：{}\n",
                image_marker::strip_all(&exp.summary)
            ));
            emitted = true;
        }
        if !exp.solution.trim().is_empty() {
            out.push_str(&format!(
                "既有解决方式：{}\n",
                image_marker::strip_all(&exp.solution)
            ));
            emitted = true;
        }
        // 分步依据（需求 4）：让微调时能看到「上一次是怎么一步步想下来的」，
        // 否则模型只拿到公式结构摘要，会把用户已经确认过的推理重新猜一遍 ——
        // 表现为「同一个需求，微调前后对不上」。
        if !exp.steps.is_empty() {
            let joined = exp
                .steps
                .iter()
                .map(|s| {
                    let expr = s
                        .expression
                        .as_deref()
                        .map(str::trim)
                        .filter(|e| !e.is_empty())
                        .map(|e| format!("（{e}）"))
                        .unwrap_or_default();
                    format!(
                        "{}：{}{expr}",
                        s.title,
                        image_marker::strip_all(&s.detail)
                    )
                })
                .collect::<Vec<_>>()
                .join("；");
            out.push_str(&format!("既有分步依据：{joined}\n"));
            emitted = true;
        }
        if emitted {
            out.push_str("（以上「既有…」是上一次已确认的解读，请在同一套思路下修改，不要推翻它。）\n");
        }
    }

    out.push_str("请保持上述变量符号与单位，按微调需求在该公式基础上修改，输出要求与系统提示词一致。\n");
    out.push_str(&format!("微调需求：{}", user_input.trim()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use civilcalc_core::schema::{
        AltExpression, ExplanationStep, FormulaExplanation, FormulaSource, FormulaVar, ResultOutput,
        SourceKind, StepTemplate,
    };
    use std::collections::HashMap;

    // ---------------------------------------------------------------------
    // next_version_name / match_version
    // ---------------------------------------------------------------------

    #[test]
    fn first_refine_appends_v1() {
        assert_eq!(next_version_name("圆柱体积"), "圆柱体积-v1.0");
    }

    #[test]
    fn second_refine_increments() {
        assert_eq!(next_version_name("圆柱体积-v1.0"), "圆柱体积-v2.0");
        assert_eq!(next_version_name("圆柱体积-v9.0"), "圆柱体积-v10.0");
    }

    /// 多段版本号：取**最左**的合法后缀（惰性正则语义）
    #[test]
    fn multi_suffix_takes_leftmost_valid() {
        assert_eq!(next_version_name("a-v1.0-v2.0"), "a-v1.0-v3.0");
    }

    #[test]
    fn trims_before_parsing() {
        assert_eq!(next_version_name("  圆柱体积  "), "圆柱体积-v1.0");
        assert_eq!(next_version_name("  圆柱体积-v1.0  "), "圆柱体积-v2.0");
    }

    #[test]
    fn empty_or_blank_falls_back() {
        assert_eq!(next_version_name(""), "微调公式-v1.0");
        assert_eq!(next_version_name("   "), "微调公式-v1.0");
    }

    /// 基础名被全部剥掉（如 `-v1.0`）→ 回退「微调公式」
    #[test]
    fn stripped_to_empty_falls_back() {
        assert_eq!(next_version_name("-v1.0"), "微调公式-v2.0");
    }

    /// 非法后缀 `-vx.0` 被剥离，版本号记 0 → 变 `-v1.0`
    #[test]
    fn malformed_suffix_is_stripped() {
        assert_eq!(next_version_name("abc-vx.0"), "abc-v1.0");
        assert_eq!(next_version_name("abc-v.0"), "abc-v1.0");
    }

    /// `-v1.0` 中间出现（不是结尾）→ 不匹配版本后缀
    #[test]
    fn suffix_must_be_at_end() {
        assert_eq!(next_version_name("a-v1.0x"), "a-v1.0x-v1.0");
    }

    /// 没有 `-v` → 原样做基础名
    #[test]
    fn no_suffix_uses_name_as_base() {
        assert_eq!(match_version("abc"), ("abc".to_string(), 0));
    }

    #[test]
    fn match_version_returns_major() {
        assert_eq!(match_version("x-v7.0"), ("x".to_string(), 7));
        assert_eq!(match_version("x"), ("x".to_string(), 0));
        assert_eq!(match_version("x-vz.0"), ("x".to_string(), 0));
    }

    /// 版本号超 i32 范围不 panic（回退 0）
    #[test]
    fn huge_major_does_not_panic() {
        let (base, major) = match_version("x-v99999999999999999999.0");
        assert_eq!(base, "x");
        assert_eq!(major, 0, "解析失败回退 0");
        assert_eq!(next_version_name("x-v99999999999999999999.0"), "x-v1.0");
    }

    /// 中文基础名（多字节）切片正确
    #[test]
    fn multibyte_base_name() {
        assert_eq!(next_version_name("梁正截面受弯承载力-v3.0"), "梁正截面受弯承载力-v4.0");
    }

    // ---------------------------------------------------------------------
    // build_revise_request
    // ---------------------------------------------------------------------

    fn base_schema() -> FormulaSchema {
        FormulaSchema {
            id: "usr:1".into(),
            result_name: "圆柱体积".into(),
            result_symbol: "V".into(),
            result_unit: Some("m³".into()),
            result_outputs: Vec::new(),
            expression: "pi*r^2*h".into(),
            source_equations: Vec::new(),
            alt_expressions: Vec::new(),
            constants: HashMap::new(),
            variables: Vec::new(),
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

    fn var(symbol: &str, desc: &str, unit: Option<&str>, default: Option<f64>) -> FormulaVar {
        FormulaVar {
            symbol: symbol.into(),
            desc: desc.into(),
            unit: unit.map(String::from),
            default,
            min: None,
            max: None,
            required: true,
        }
    }

    #[test]
    fn request_has_header_and_footer() {
        let s = base_schema();
        let text = build_revise_request(&s, "  把半径换成直径  ");
        assert!(text.starts_with("既有公式：圆柱体积（结果符号 V，单位 m³）\n"));
        assert!(text.contains("\n主表达式：pi*r^2*h\n"));
        assert!(text.ends_with("微调需求：把半径换成直径"), "需求应 trim 后收尾");
        assert!(
            text.contains("请保持上述变量符号与单位，按微调需求在该公式基础上修改，输出要求与系统提示词一致。\n"),
            "固定尾句必须在需求之前"
        );
    }

    /// 单位为空时括号内留空（`orEmpty()`）
    #[test]
    fn empty_unit_renders_blank() {
        let mut s = base_schema();
        s.result_unit = None;
        let text = build_revise_request(&s, "x");
        assert!(text.starts_with("既有公式：圆柱体积（结果符号 V，单位 ）\n"));
    }

    #[test]
    fn variables_with_unit_and_default() {
        let mut s = base_schema();
        s.variables = vec![
            var("r", "半径", Some("m"), None),
            var("h", "高", Some("m"), Some(10.0)),
            var("k", "系数", None, None),
        ];
        let text = build_revise_request(&s, "x");
        assert!(
            text.contains("变量：r(半径，m)；h(高，m，默认10)；k(系数)\n"),
            "实际：{text}"
        );
    }

    /// 空白单位不进摘要（`unit?.takeIf { isNotBlank() }`）
    #[test]
    fn blank_unit_omitted_in_variables() {
        let mut s = base_schema();
        s.variables = vec![var("r", "半径", Some("   "), None)];
        let text = build_revise_request(&s, "x");
        assert!(text.contains("变量：r(半径)\n"), "实际：{text}");
    }

    #[test]
    fn constants_sorted_by_key_for_cache_stability() {
        let mut s = base_schema();
        s.constants = HashMap::from([("beta".to_string(), 2.5), ("alpha".to_string(), 206000.0)]);
        let a = build_revise_request(&s, "x");
        let b = build_revise_request(&s, "x");
        assert_eq!(a, b, "同一条公式两次拼装必须逐字相同（否则破坏 KV cache）");
        assert!(a.contains("常量：alpha=206000；beta=2.5\n"), "按键排序，实际：{a}");
    }

    #[test]
    fn alt_expressions_with_and_without_condition() {
        let mut s = base_schema();
        s.alt_expressions = vec![
            AltExpression {
                label: "短柱".into(),
                expression: "a+b".into(),
                condition: Some("h/b<=4".into()),
            },
            AltExpression {
                label: "长柱".into(),
                expression: "a-b".into(),
                condition: None,
            },
        ];
        let text = build_revise_request(&s, "x");
        assert!(text.contains("分支公式：短柱:a+b（条件：h/b<=4）；长柱:a-b\n"), "实际：{text}");
    }

    #[test]
    fn steps_template_rendered() {
        let mut s = base_schema();
        s.steps_template = Some(vec![
            StepTemplate {
                symbol: "A".into(),
                label: "底面积".into(),
                group: None,
                expression: "pi*r^2".into(),
                unit: "m²".into(),
                note: None,
            },
            StepTemplate {
                symbol: "V".into(),
                label: "体积".into(),
                group: None,
                expression: "A*h".into(),
                unit: "m³".into(),
                note: None,
            },
        ]);
        let text = build_revise_request(&s, "x");
        assert!(text.contains("步骤：A=pi*r^2（底面积）；V=A*h（体积）\n"), "实际：{text}");
    }

    #[test]
    fn declared_outputs_summary_included() {
        let mut s = base_schema();
        s.expression = "x = a+b; y = a-b".into();
        s.result_outputs = vec![
            ResultOutput { symbol: "x".into(), name: "X 坐标".into(), unit: "m".into() },
            ResultOutput { symbol: "y".into(), name: "Y 坐标".into(), unit: "m".into() },
        ];
        let text = build_revise_request(&s, "x");
        assert!(text.contains("结果输出："), "实际：{text}");
        assert!(text.contains("X 坐标"));
        assert!(text.contains("Y 坐标"));
    }

    #[test]
    fn optional_lines_omitted_when_blank() {
        let s = base_schema();
        let text = build_revise_request(&s, "x");
        for absent in ["结果输出：", "分支公式：", "常量：", "变量：", "步骤：", "适用条件：", "依据：", "设计要点：", "既有需求理解：", "既有解决方式：", "既有分步依据：", "（以上「既有…」"] {
            assert!(!text.contains(absent), "空字段不该出现 `{absent}`：{text}");
        }
    }

    #[test]
    fn domain_and_basis_and_notes_rendered() {
        let mut s = base_schema();
        s.domain = "矩形截面".into();
        s.reference_basis = Some("GB 50010-2010".into());
        s.design_notes = Some("按净高取值".into());
        let text = build_revise_request(&s, "x");
        assert!(text.contains("适用条件：矩形截面\n"));
        assert!(text.contains("依据：GB 50010-2010\n"));
        assert!(text.contains("设计要点：按净高取值\n"));
    }

    /// 🔴 既有详解**必须剥掉 `{{img:N}}`**（否则模型会照抄旧序号）
    #[test]
    fn explanation_strips_image_markers() {
        let mut s = base_schema();
        s.explanation = Some(FormulaExplanation {
            summary: "①按{{img:1}}核对尺寸".into(),
            solution: "②见{{img:2}}的做法".into(),
            steps: vec![ExplanationStep {
                title: "t".into(),
                expression: None,
                detail: "d".into(),
            }],
        });
        let text = build_revise_request(&s, "x");
        assert!(text.contains("既有需求理解：①按核对尺寸\n"), "实际：{text}");
        assert!(text.contains("既有解决方式：②见的做法\n"), "实际：{text}");
        assert!(!text.contains("{{img:"), "不得把渲染指令带进续写上下文");
    }

    /// 详解为空串时不输出该行
    #[test]
    fn blank_explanation_lines_omitted() {
        let mut s = base_schema();
        s.explanation = Some(FormulaExplanation::default());
        let text = build_revise_request(&s, "x");
        assert!(!text.contains("既有需求理解"));
        assert!(!text.contains("既有解决方式"));
        assert!(!text.contains("既有分步依据"));
        // 三段全空时连收尾说明也不该出现（否则一条空详解会凭空多一行）
        assert!(!text.contains("（以上「既有…」"), "实际：{text}");
    }

    /// 需求 4：微调上下文要带上详解的**分步依据**，让思维过程对齐
    #[test]
    fn explanation_steps_included() {
        let mut s = base_schema();
        s.explanation = Some(FormulaExplanation {
            summary: "求圆柱体积".into(),
            solution: "底面积乘高".into(),
            steps: vec![
                ExplanationStep {
                    title: "算底面积".into(),
                    expression: Some("pi*r^2".into()),
                    detail: "半径为 r 的圆面积".into(),
                },
                ExplanationStep {
                    title: "乘高".into(),
                    expression: None,
                    detail: "得到体积".into(),
                },
            ],
        });
        let text = build_revise_request(&s, "把单位换成 mm");
        assert!(
            text.contains("既有分步依据：算底面积：半径为 r 的圆面积（pi*r^2）；乘高：得到体积\n"),
            "实际：{text}"
        );
        assert!(text.contains("请在同一套思路下修改"), "要有对齐提示：{text}");
        // 分步依据必须排在固定尾句之前（缓存友好：字段顺序固定）
        let idx = |n: &str| text.find(n).unwrap_or_else(|| panic!("缺 {n}"));
        assert!(idx("既有分步依据：") < idx("微调需求："));
    }

    /// 分步依据里的 `{{img:N}}` 同样要剥掉（与 summary/solution 一致）
    #[test]
    fn explanation_steps_strip_image_markers() {
        let mut s = base_schema();
        s.explanation = Some(FormulaExplanation {
            summary: String::new(),
            solution: String::new(),
            steps: vec![ExplanationStep {
                title: "看图".into(),
                expression: None,
                detail: "对照{{img:3}}的尺寸".into(),
            }],
        });
        let text = build_revise_request(&s, "x");
        assert!(!text.contains("{{img:"), "实际：{text}");
        assert!(text.contains("既有分步依据：看图：对照的尺寸"), "实际：{text}");
    }

    /// **不带** Excel 字段与历史版本
    #[test]
    fn excludes_excel_fields() {
        let mut s = base_schema();
        s.excel_expression = Some("=PI()*A1^2".into());
        s.doc_template_id = Some("tpl".into());
        let text = build_revise_request(&s, "x");
        assert!(!text.contains("PI()"), "Excel 字段不进续写请求");
        assert!(!text.contains("tpl"));
    }

    /// 字段顺序固定（缓存友好）：逐行顺序断言
    #[test]
    fn field_order_is_stable() {
        let mut s = base_schema();
        s.variables = vec![var("r", "半径", Some("m"), None)];
        s.constants = HashMap::from([("alpha".to_string(), 2.5)]);
        s.steps_template = Some(vec![StepTemplate {
            symbol: "A".into(),
            label: "底面积".into(),
            group: None,
            expression: "pi*r^2".into(),
            unit: "m²".into(),
            note: None,
        }]);
        s.domain = "圆柱".into();
        let text = build_revise_request(&s, "x");
        let idx = |needle: &str| text.find(needle).unwrap_or_else(|| panic!("缺 {needle}"));
        assert!(idx("既有公式：") < idx("主表达式："));
        assert!(idx("主表达式：") < idx("常量："));
        assert!(idx("常量：") < idx("变量："));
        assert!(idx("变量：") < idx("步骤："));
        assert!(idx("步骤：") < idx("适用条件："));
        assert!(idx("适用条件：") < idx("微调需求："));
    }
}
