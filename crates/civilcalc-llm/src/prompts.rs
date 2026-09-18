//! 提示词常量。
//!
//! 源：`civilcalc-android-v2/core/llm/LlmPrompts.kt`（89 行）
//!
//! ## 🔴 铁律：必须是**编译期常量**，禁止运行时拼接
//!
//! 源注释写明：*「编译期常量 prompt —— 禁止运行时拼接动态内容，保证 KV cache 前缀稳定」*。
//! 因此本文件全部用 `const` + [`concat!`] 构造，**不用 `format!`**：
//! 运行时不产生新字符串，前缀逐字节稳定，服务端的 KV cache 才命中。
//!
//! ## 两段式设计（也是为 KV cache）
//!
//! | 常量 | 内容 | 何时发 |
//! |---|---|---|
//! | [`PROMPT_A_CORE`] | 结构 + 规则 + Excel 映射 + 示例 | 总是 |
//! | [`PROMPT_A_EXPLAIN`] | 详解字段规则 | 开了「生成时包含公式详解」才发 |
//! | [`PROMPT_A`] | 两者拼接（默认） | — |
//!
//! 拼起来仍是常量；省掉详解段就形成**另一条独立且同样稳定**的缓存前缀。
//!
//! ## 公共规则只写一份（用宏，不用复制粘贴）
//!
//! 详解规则在 [`PROMPT_A_EXPLAIN`] 与 [`PROMPT_EXPLAIN`] 两处出现。源注释记录过教训：
//! *「同一批规则抄了三遍、其中『分点独占一行』重复了四次」*。这里用
//! [`explain_common_rules!`] 宏把规则收成**一份字面量**，两处 `concat!` 各自内联 ——
//! 既满足编译期常量，又杜绝措辞漂移。
//!
//! > ⚠️ 用 `macro_rules!` 而不是 `const`，是因为 [`concat!`] **只接受字面量**，
//! > 不接受 `const` 标识符；宏在展开后才参与 `concat!`，所以能内联。
//!
//! ## 精简原则（源注释）
//!
//! **契约一个字不能少，格式要求不靠提示词。**
//! 字段表 / 允许函数表 / Excel 映射表 / JSON 示例都是「少一项就出废结果」的硬契约；
//! 而「分点必须换行」这类渲染格式已由渲染端（`civilcalc-report` 的
//! `ExplanationText::normalize`）确定性处理，**不在提示词里反复叮嘱**。

/// 详解链路的公共规则。
///
/// 随公式一起出（[`PROMPT_A_EXPLAIN`]）与旧公式补齐（[`PROMPT_EXPLAIN`]）**共用同一份**。
macro_rules! explain_common_rules {
    () => {
        r#"- 附图标记：附有图片时，在确实需要图示辅助说明的位置（如引用尺寸/构造的那一步）插入 {{img:N}}，N=附图序号（从 1 起，按附图给出顺序）；禁止编造不存在的序号，未附图时禁止出现该标记
- 勤用 Markdown 标记：**加粗** 标关键依据/结论/系数含义，`行内代码` 标变量符号/系数数值/单位，==高亮== 仅标每步最核心的结果或结论（金色荧光底），每步合计 2~5 处，禁整句标记
- 严禁臆造规范条文编号、文献出处或数据，无把握的依据一律写"工程经验"或"通用做法""#
    };
}

/// Prompt A 主体（结构 + 规则 + Excel 映射 + 示例），**不含**详解段。
macro_rules! prompt_a_core_body {
    () => {
        r#"工程公式解析器。仅输出 JSON，禁止解释/Markdown。给出最优公式，严禁补全/臆造系数。

输出字段（顺序固定，结构见末尾示例）：
id,resultName,resultSymbol,resultUnit,resultOutputs,expression,sourceEquations,altExpressions,constants,variables,domain,referenceBasis,tags,source,stepsTemplate,excelExpression,excelAltExpressions,excelStepsTemplate,excelFunctionDocs,designNotes,schemaVersion

规则：
1. expression 仅含允许函数与四则运算，可纯函数求值。允许函数：sqrt/abs/pow/log/ln/log10/exp/sin/cos/tan/asin/acos/atan/min/max/floor/ceil/round（log=自然对数）
2. variables 覆盖 expression 全部符号，symbol/desc/unit 必填，unit∈{m,mm,kN,N,MPa,""}，required=true/false。default 仅在数值可靠时填：用户明确给出、工程通用系数（材料密度/安全系数等公认值）、用户要求示例数据；臆造或不确定的仍留符号不填
3. expression 单结果时是一条表达式；同一组输入同时得出多个并列量（如方程组的 x、y，最多 4 个）时用分号分段、每段必须以「符号 =」开头（如 "x = …; y = …"），并在 resultOutputs 按展示顺序列出 {symbol,name,unit}，symbol 与各段赋值符号一一对应；单结果 resultOutputs=[]。方程/方程组只写代数展开后的闭式解（求根公式/克莱默法则），系数入 variables 供用户改，禁把系数写死进 constants；任一段右侧不得再出现本段输出符号（否则等于没解出）；中间量放 stepsTemplate。resultUnit 无量纲填""；referenceBasis 不确定写"经验公式/教材通用式"；source 恒为 {kind:"AI",ref:null,verified:false}；id 以 usr: 开头
4. stepsTemplate 拆 2~12 步（按需求复杂度取，简单 2~4 步、复杂最多 12 步；无需拆解填[]）：symbol=本步结果符号（后步可引用），label=中文名，group=可选分组，expression 只引用前序 symbol+variables+constants（禁循环引用），unit=本步单位；按依赖拓扑序，末步即主结果（多输出时=最后一个输出）、每步尽量单段（步骤含多段时 Excel 只取末段）；粒度以"独立工程含义"为准，禁为凑步数拆分无意义中间算术（宁少勿滥）
5. designNotes ≤100 字中文，归纳工程逻辑与推导要点（系数含义、适用前提），供续写微调引用
6. desc 与 designNotes 可用少量 Markdown 标关键含义：**加粗**、`行内代码`
7. schemaVersion=2
8. sourceEquations：需求里给出的原始方程/条件式逐条原样抄录（如 "x+y+z=6"、"2x-y+z=3"），一条一项、符号与 expression 一致、乘号写显式形式（2x 写作 2*x）；需求没给方程（纯描述或查表类工程公式）填 []。该字段只用于回显与代入验算，不参与计算

Excel 转换（固定映射，禁止自创）：
- excelExpression：expression→Excel。变量按 variables 顺序横向排在第 1 行（第 n 个变量→列名(n)+1：第1个→A1、第26个→Z1、第27个→AA1），禁止放进其它行；常量 pi→PI()/e→EXP(1)；函数映射 sqrt→SQRT,pow→POWER,log/ln→LN,log10→LOG10,exp→EXP,sin→SIN,cos→COS,tan→TAN,asin→ASIN,acos→ACOS,atan→ATAN,abs→ABS,min→MIN,max→MAX,floor→FLOOR.MATH,ceil→CEILING.MATH,round→ROUND(,0)，无对应：cbrt→POWER(,1/3),hypot→SQRT(x^2+y^2)；幂 x^y→POWER(x,y)；必须以 = 开头；多输出时同样用分号分段、每段写「符号 = Excel 公式」，段数与符号跟 expression 一一对应
- excelStepsTemplate：步骤结果统一存 A 列——第 i 步结果=A(i+1)（第1步→A2），后续步骤引用前序结果直接写该单元格；变量仍按第1行契约
- excelAltExpressions：同结构，expression 为 Excel 格式（变量映射同上）
- excelFunctionDocs：仅列实际用到的函数 {name,excelName,description,syntax}

示例：圆柱体积 V=π·r²·h（r 无确定值不预设，h 为示例数据）
{"id":"usr:cylinder","resultName":"圆柱体积","resultSymbol":"V","resultUnit":"m³","resultOutputs":[],"expression":"pi*r^2*h","sourceEquations":[],"altExpressions":[],"constants":{"pi":3.1415926},"variables":[{"symbol":"r","desc":"**圆柱底面**半径","unit":"m","required":true},{"symbol":"h","desc":"柱体高度（示例 `10.0m`）","unit":"m","default":10.0,"required":true}],"domain":"圆柱","referenceBasis":"几何","tags":[],"source":{"kind":"AI","ref":null,"verified":false},"stepsTemplate":[{"symbol":"A","label":"底面积","expression":"pi*r^2","unit":"m²"},{"symbol":"V","label":"体积","expression":"A*h","unit":"m³"}],"excelExpression":"=PI()*A1^2*B1","excelAltExpressions":[],"excelStepsTemplate":[{"symbol":"A","label":"底面积","expression":"=PI()*A1^2","unit":"m²"},{"symbol":"V","label":"体积","expression":"=A2*B1","unit":"m³"}],"excelFunctionDocs":[{"name":"pow","excelName":"POWER","description":"幂运算","syntax":"POWER(n,p)"}],"designNotes":"底面圆面积乘柱高即体积，pi 取圆周率","schemaVersion":2}"#
    };
}

/// 详解字段规则（可按设置整段省掉）。
macro_rules! prompt_a_explain_body {
    () => {
        concat!(
            r#"补充字段 explanation（计算公式详解，供用户核对公式是否符合预期）：
- summary：需求理解，≤200 字中文，用 ①②③… 分点（每点一句话）：先说明要解决的问题与已知条件/约束，再说明总体计算思路
- solution：解决方式，≤200 字中文，用 ①②③… 分点（每点一句话）：逐条对应 summary 提出的诉求给出解法（采用哪条公式或推导路径、系数取值依据、边界与特例如何处理、还需用户补充哪些参数）
- steps：逐步拆解，步数必须与 stepsTemplate 严格相等且顺序一一对应（stepsTemplate 为空时才把主表达式拆 1~3 步）；每步 title≤10字、expression 为该步表达式、detail 同时说清「计算依据」（数学原理/工程通用做法/系数含义）与「预期达到的结果」
"#,
            explain_common_rules!(),
            "\n\n",
            r#"格式示例（节选，仅示意写法）：
"explanation":{"summary":"①需按图纸核对**底面半径**与**柱高**两个尺寸；②柱高取净高，不含底板厚度。","solution":"①按底圆面积乘柱高求解；②`pi` 取 `3.1415926`；③半径未给出时需实测补入。","steps":[{"title":"底面积","expression":"pi*r^2","detail":"底面为圆，按 `πr²` 求**底面积**，作为乘柱高的基准量。"}]}"#
        )
    };
}

/// 旧公式补齐详解用的专项提醒词。
macro_rules! prompt_explain_body {
    () => {
        concat!(
            r#"工程公式讲解员。仅输出 JSON，禁止解释/Markdown。
输入为已验证公式的关键信息（名称/表达式/变量/常量/步骤/依据），可能附有用户上传的图片。请站在使用者角度总结该公式要解决的问题，再逐项拆解，供用户核对公式是否符合预期。
输出 Schema：
{"summary":"需求理解：≤200字中文，用 ①②③… 分点","solution":"解决方式：≤200字中文，用 ①②③… 分点","steps":[{"title":"简短中文步骤名","expression":"该步对应表达式","detail":"本步计算依据（数学原理/工程通用做法/系数含义）与预期达到的结果"}]}
规则：
1. summary 先说明要解决的问题与已知条件/约束，再说明总体计算思路，每点一句话
2. solution 逐条对应 summary 提出的诉求给出解法（采用哪条公式或推导路径、系数取值依据、边界与特例如何处理、还需用户补充哪些参数）
3. steps 按计算顺序拆解：有 stepsTemplate 时与其步数严格相等、顺序一一对应，无则把主表达式拆为 1~3 个有工程含义的部分；每步 title≤10字，expression 原样引用输入中的符号
4. detail 必须同时说清「依据」与「预期结果」
5. 全部中文输出；不新增输入中不存在的符号或系数
"#,
            explain_common_rules!()
        )
    };
}

/// Prompt A 主体（不含详解段）。
///
/// ⚠️ 内容必须与源 `LlmPrompts.PROMPT_A_CORE` **逐字一致** ——
/// 这是「契约一个字不能少」的部分，改一个字都可能让模型输出废结果。
pub const PROMPT_A_CORE: &str = prompt_a_core_body!();

/// 详解字段规则（可按设置省掉）。
pub const PROMPT_A_EXPLAIN: &str = prompt_a_explain_body!();

/// 默认提示词 = 主体 + 详解规则。
pub const PROMPT_A: &str = concat!(prompt_a_core_body!(), "\n\n", prompt_a_explain_body!());

/// 旧公式补齐详解的提醒词。
pub const PROMPT_EXPLAIN: &str = prompt_explain_body!();

/// 详解公共规则原文（供测试比对，验证两处内联一致）。
pub const EXPLAIN_COMMON_RULES: &str = explain_common_rules!();

/// 按设置拼接 Prompt A：关掉「生成时包含公式详解」则省掉详解段。
///
/// 返回 `&'static str` —— 是上面两个常量之一，**不会新建字符串**（KV cache 前缀稳定）。
#[must_use]
pub fn prompt_a(generate_explanation: bool) -> &'static str {
    if generate_explanation {
        PROMPT_A
    } else {
        PROMPT_A_CORE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------
    // 结构：三段关系必须成立
    // ---------------------------------------------------------------------

    /// `PROMPT_A` 必须是 `CORE + "\n\n" + EXPLAIN`（与源 `"$A\n\n$B"` 一致）
    #[test]
    fn prompt_a_is_core_plus_explain() {
        let expected = format!("{PROMPT_A_CORE}\n\n{PROMPT_A_EXPLAIN}");
        assert_eq!(PROMPT_A, expected);
    }

    /// `prompt_a(true)` 走默认，`prompt_a(false)` 省掉详解段
    #[test]
    fn prompt_a_switches_explain_segment() {
        assert_eq!(prompt_a(true), PROMPT_A);
        assert_eq!(prompt_a(false), PROMPT_A_CORE);
        assert!(
            !prompt_a(false).contains("补充字段 explanation"),
            "关掉详解后不得残留详解字段规则"
        );
    }

    /// 两段拼接处**恰好**是两个换行，不多不少
    #[test]
    fn separator_is_exactly_two_newlines() {
        assert!(PROMPT_A.contains(&format!("{PROMPT_A_CORE}\n\n{PROMPT_A_EXPLAIN}")));
        // CORE 末字符是 `}`，EXPLAIN 首字符是 `补`
        assert!(PROMPT_A_CORE.ends_with('}'));
        assert!(PROMPT_A_EXPLAIN.starts_with("补充字段"));
        assert!(PROMPT_A_EXPLAIN.ends_with('}'));
    }

    // ---------------------------------------------------------------------
    // 契约完整性：这些「少一项就出废结果」的内容必须都在
    // ---------------------------------------------------------------------

    /// 允许函数表完整（少一个函数 → 模型可能输出引擎不支持的名字）
    #[test]
    fn core_lists_all_allowed_functions() {
        for f in [
            "sqrt", "abs", "pow", "log", "ln", "log10", "exp", "sin", "cos", "tan", "asin",
            "acos", "atan", "min", "max", "floor", "ceil", "round",
        ] {
            assert!(
                PROMPT_A_CORE.contains(f),
                "允许函数表缺 `{f}` —— 契约不能少项"
            );
        }
        // `log=自然对数` 这条容易误解的说明必须保留
        assert!(PROMPT_A_CORE.contains("log=自然对数"));
    }

    /// Excel 映射表关键项（少一项 → 用户粘到 Excel 里报错）
    #[test]
    fn core_lists_excel_mappings() {
        for m in [
            "sqrt→SQRT",
            "pow→POWER",
            "log/ln→LN",
            "log10→LOG10",
            "floor→FLOOR.MATH",
            "ceil→CEILING.MATH",
            "cbrt→POWER(,1/3)",
            "hypot→SQRT(x^2+y^2)",
            "pi→PI()",
        ] {
            assert!(
                PROMPT_A_CORE.contains(m),
                "Excel 映射表缺 `{m}` —— 契约不能少项"
            );
        }
    }

    /// 输出字段顺序表必须完整（顺序固定，模型按此顺序出 JSON）
    #[test]
    fn core_lists_output_field_order() {
        let order = "id,resultName,resultSymbol,resultUnit,resultOutputs,expression,sourceEquations,altExpressions,constants,variables,domain,referenceBasis,tags,source,stepsTemplate,excelExpression,excelAltExpressions,excelStepsTemplate,excelFunctionDocs,designNotes,schemaVersion";
        assert!(PROMPT_A_CORE.contains(order), "输出字段顺序表被改动或缺失");
    }

    /// 示例 JSON 必须还在（模型靠它对齐结构）
    #[test]
    fn core_keeps_json_example() {
        assert!(PROMPT_A_CORE.contains(r#""id":"usr:cylinder""#));
        assert!(PROMPT_A_CORE.contains(r#""schemaVersion":2"#));
    }

    /// 两条硬约束的原文措辞（改了就丢语义）
    #[test]
    fn core_keeps_hard_constraints() {
        assert!(PROMPT_A_CORE.contains("严禁补全/臆造系数"));
        assert!(
            PROMPT_A_CORE.contains(r#"source 恒为 {kind:"AI",ref:null,verified:false}"#),
            "AI 来源必须强制 verified=false / ref=null"
        );
        assert!(PROMPT_A_CORE.contains("禁把系数写死进 constants"));
    }

    /// 步骤上限 12、输出上限 4 —— 与引擎的硬限制对齐
    #[test]
    fn core_keeps_numeric_limits() {
        assert!(PROMPT_A_CORE.contains("2~12 步"));
        assert!(PROMPT_A_CORE.contains("最多 4 个"));
    }

    // ---------------------------------------------------------------------
    // 公共规则：两处必须一致（防措辞漂移）
    // ---------------------------------------------------------------------

    /// 🔴 核心防漂移断言：`PROMPT_A_EXPLAIN` 与 `PROMPT_EXPLAIN` 内联的规则**逐字相同**
    #[test]
    fn common_rules_are_identical_in_both_prompts() {
        assert!(
            PROMPT_A_EXPLAIN.contains(EXPLAIN_COMMON_RULES),
            "PROMPT_A_EXPLAIN 里的公共规则与单一定义不一致"
        );
        assert!(
            PROMPT_EXPLAIN.contains(EXPLAIN_COMMON_RULES),
            "PROMPT_EXPLAIN 里的公共规则与单一定义不一致"
        );
    }

    /// 公共规则三条都在（附图标记 / Markdown / 禁臆造）
    #[test]
    fn common_rules_have_three_bullets() {
        assert!(EXPLAIN_COMMON_RULES.contains("{{img:N}}"));
        assert!(EXPLAIN_COMMON_RULES.contains("==高亮=="));
        assert!(EXPLAIN_COMMON_RULES.contains("严禁臆造规范条文编号"));
        assert_eq!(
            EXPLAIN_COMMON_RULES.lines().count(),
            3,
            "公共规则应恰好 3 条"
        );
    }

    /// 公共规则**只出现一次**在各自 prompt 中（没被复制粘贴两份）
    #[test]
    fn common_rules_appear_once_each() {
        assert_eq!(PROMPT_A_EXPLAIN.matches("- 附图标记：").count(), 1);
        assert_eq!(PROMPT_EXPLAIN.matches("- 附图标记：").count(), 1);
    }

    // ---------------------------------------------------------------------
    // 补齐 prompt
    // ---------------------------------------------------------------------

    /// `PROMPT_EXPLAIN` 自带输出 Schema 与 5 条规则
    #[test]
    fn prompt_explain_has_schema_and_five_rules() {
        assert!(PROMPT_EXPLAIN.starts_with("工程公式讲解员"));
        assert!(PROMPT_EXPLAIN.contains("输出 Schema："));
        for i in 1..=5 {
            assert!(
                PROMPT_EXPLAIN.contains(&format!("\n{i}. ")),
                "缺少第 {i} 条规则"
            );
        }
        assert!(PROMPT_EXPLAIN.ends_with(EXPLAIN_COMMON_RULES));
    }

    // ---------------------------------------------------------------------
    // 格式卫生：无首尾空白、无 trimIndent 残留
    // ---------------------------------------------------------------------

    /// 四个常量都不得有首尾空白（源用 `trimIndent()`，等价物）
    #[test]
    fn no_leading_or_trailing_whitespace() {
        for (name, s) in [
            ("PROMPT_A_CORE", PROMPT_A_CORE),
            ("PROMPT_A_EXPLAIN", PROMPT_A_EXPLAIN),
            ("PROMPT_A", PROMPT_A),
            ("PROMPT_EXPLAIN", PROMPT_EXPLAIN),
            ("EXPLAIN_COMMON_RULES", EXPLAIN_COMMON_RULES),
        ] {
            assert_eq!(s.trim(), s, "{name} 有首尾空白");
            assert!(!s.is_empty(), "{name} 为空");
        }
    }

    /// 不得有行首缩进（Kotlin `trimIndent()` 会去掉；漏掉会让 prompt 多出空格）
    #[test]
    fn no_leading_indent_on_any_line() {
        for (name, s) in [
            ("PROMPT_A_CORE", PROMPT_A_CORE),
            ("PROMPT_A_EXPLAIN", PROMPT_A_EXPLAIN),
            ("PROMPT_A", PROMPT_A),
            ("PROMPT_EXPLAIN", PROMPT_EXPLAIN),
        ] {
            for line in s.lines() {
                assert!(
                    !line.starts_with(' ') && !line.starts_with('\t'),
                    "{name} 有行首缩进：{line:?}"
                );
            }
        }
    }

    /// 不得出现 `\r`（Windows 换行混入会污染缓存前缀）
    #[test]
    fn no_carriage_returns() {
        for s in [PROMPT_A_CORE, PROMPT_A_EXPLAIN, PROMPT_A, PROMPT_EXPLAIN] {
            assert!(!s.contains('\r'), "prompt 含 CR");
        }
    }

    /// 返回 `&'static str` 且**不新建字符串**
    ///
    /// ⚠️ 这里**不能**用 `std::ptr::eq` 比对：`const` 会在每个使用点内联，
    /// 编译器允许为 `PROMPT_A_CORE` 与 `prompt_a(false)` 的返回值各生成一份匿名分配，
    /// 地址不保证相同（改用 `static` 才会）。有意义的不变量是「内容与常量逐字相等」。
    #[test]
    fn prompt_a_returns_constants_without_allocating() {
        assert_eq!(prompt_a(true), PROMPT_A);
        assert_eq!(prompt_a(false), PROMPT_A_CORE);
        // 返回类型是 `&'static str`（编译期已由签名钉住，运行时无法伪造）
        let s: &'static str = prompt_a(true);
        assert_eq!(s.len(), PROMPT_A.len());
    }
}
