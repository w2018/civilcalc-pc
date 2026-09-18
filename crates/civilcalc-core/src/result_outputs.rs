//! 结果输出的统一解析。
//!
//! 源：`core/formula/ResultOutputs.kt`（102 行）
//!
//! ## 为什么单独一个模块
//!
//! 「结果」有三种形态，展示层（结果区 / 历史卡片 / Excel 逐条公式 / DOCX 计算书 /
//! HTML 预览）都要用同一套口径。三处各写一份兜底逻辑，迟早会不一致 ——
//! 源项目把它抽成 `ResultOutputs` 正是这个原因。
//!
//! ## 三种形态的收敛规则（源注释原样保留）
//!
//! 1. `schema.result_outputs` **有声明**（方程组 x/y 等）→ 按声明顺序取，
//!    **未声明的段视为中间量不展示**
//! 2. **无声明但表达式是多段**（历史公式、内置坐标公式）→ 按段输出
//! 3. **单段** → 老的单值口径（`primary`）

use crate::number_format::format_number;
use crate::engine::splitter;
use crate::schema::{
    result_display_name, result_unit_of, EvalResult, FormulaSchema, ResultOutput,
};

/// 一个已解析的展示用结果。
///
/// `value` 为 `None` 表示该输出在本次求值结果里**找不到**
/// （声明了却没算出来）—— 展示层应显示占位（`—`）而不是静默隐藏，
/// 否则用户会以为公式没有这个结果。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedResult {
    pub symbol: String,
    pub name: String,
    pub value: Option<f64>,
    pub unit: String,
}

impl ResolvedResult {
    /// 展示标签：`X（新点 X 坐标）`。
    ///
    /// 名称为空、或名称就等于符号时只显示符号 —— 否则会出现 `x（x）` 这种废话。
    pub fn label(&self) -> String {
        if self.name.is_empty() || self.name == self.symbol {
            self.symbol.clone()
        } else {
            format!("{}（{}）", self.symbol, self.name)
        }
    }

    /// 数值文本；无值时是占位符 `—`
    pub fn value_text(&self) -> String {
        match self.value {
            Some(v) => format_number(v),
            None => "—".to_string(),
        }
    }

    /// 数值 + 单位（单位空则不补空格）
    pub fn value_with_unit(&self) -> String {
        let v = self.value_text();
        if self.unit.is_empty() {
            v
        } else {
            format!("{v} {}", self.unit)
        }
    }

    /// 单行文本 `符号 = 数值 单位`（复制按钮用）
    pub fn one_line(&self) -> String {
        let value = self.value_text();
        if self.unit.is_empty() {
            format!("{} = {value}", self.symbol)
        } else {
            format!("{} = {value} {}", self.symbol, self.unit)
        }
    }
}

/// 把三种结果形态收敛成一条展示路径。见模块文档的收敛规则。
pub fn resolve(schema: &FormulaSchema, result: &EvalResult) -> Vec<ResolvedResult> {
    // 规则 1：有声明清单 → 按声明顺序取
    let declared: Vec<&ResultOutput> = schema
        .result_outputs
        .iter()
        .filter(|o| !o.symbol.trim().is_empty())
        .collect();

    if !declared.is_empty() {
        return declared
            .into_iter()
            .map(|d| ResolvedResult {
                symbol: d.symbol.clone(),
                // 声明里的 name 为空时回落到「步骤 label → 符号本身」
                name: if d.name.trim().is_empty() {
                    result_display_name(schema, &d.symbol)
                } else {
                    d.name.clone()
                },
                value: result
                    .outputs
                    .iter()
                    .find(|o| o.symbol.as_deref() == Some(d.symbol.as_str()))
                    .map(|o| o.value),
                unit: if d.unit.trim().is_empty() {
                    result_unit_of(schema, &d.symbol)
                } else {
                    d.unit.clone()
                },
            })
            .collect();
    }

    // 规则 2：无声明但多段 → 按段输出
    if result.outputs.len() > 1 {
        return result
            .outputs
            .iter()
            .map(|o| {
                // 段没有赋值符号时（`a+b; c+d`）回落到公式级 result_symbol
                let symbol = o.symbol.clone().unwrap_or_else(|| schema.result_symbol.clone());
                ResolvedResult {
                    name: result_display_name(schema, &symbol),
                    unit: result_unit_of(schema, &symbol),
                    symbol,
                    value: Some(o.value),
                }
            })
            .collect();
    }

    // 规则 3：单段 → 单值口径
    vec![ResolvedResult {
        symbol: schema.result_symbol.clone(),
        name: schema.result_name.clone(),
        value: Some(result.primary),
        unit: schema.result_unit.clone().unwrap_or_default(),
    }]
}

/// 是否多结果（结果区据此在**表格**与**单值大字号**布局之间切换）。
pub fn is_multi(items: &[ResolvedResult]) -> bool {
    items.len() > 1
}

/// 多行复制文本：每行「符号 = 数值 单位」。
pub fn build_copy_text(items: &[ResolvedResult]) -> String {
    items
        .iter()
        .map(ResolvedResult::one_line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// 单行摘要（历史卡片等窄位场景）：`符号=数值, 符号=数值`。
pub fn build_inline_text(items: &[ResolvedResult]) -> String {
    items
        .iter()
        .map(ResolvedResult::one_line)
        .collect::<Vec<_>>()
        .join(", ")
}

/// 声明清单的摘要文本（续写微调 / 按需补详解的上下文用）：
/// `x（未知数x，—）；y（未知数y，—）`。
///
/// ⚠️ 这里**不查值** —— 它描述的是「公式声明了哪些输出」，
/// 用于给模型补上下文，与本次计算结果无关。
pub fn build_declared_summary(outputs: &[ResultOutput]) -> String {
    outputs
        .iter()
        .map(|o| {
            let desc: Vec<&str> = [o.name.as_str(), o.unit.as_str()]
                .into_iter()
                .filter(|s| !s.trim().is_empty())
                .collect();
            if desc.is_empty() {
                o.symbol.clone()
            } else {
                format!("{}（{}）", o.symbol, desc.join("，"))
            }
        })
        .collect::<Vec<_>>()
        .join("；")
}

/// 修复 AI 输出的结果声明：剔除**符号对不上任何分号段**的项与重复项。
///
/// 让「模型多填了几项」退化成能用的公式，而不是整条生成失败 ——
/// 与校验器的硬闸门互为松紧两道。
///
/// ⚠️ 调用时机在 `SchemaValidator` **之前**（见 P4-4）。
pub fn sanitize(schema: &FormulaSchema) -> Vec<ResultOutput> {
    let segment_symbols: std::collections::HashSet<String> = splitter::split(&schema.expression)
        .into_iter()
        .filter_map(|seg| seg.symbol)
        .collect();

    let mut seen = std::collections::HashSet::new();
    schema
        .result_outputs
        .iter()
        .filter(|o| {
            !o.symbol.trim().is_empty()
                && segment_symbols.contains(&o.symbol)
                && seen.insert(o.symbol.clone())
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{EvalOutput, ResultOutput};

    fn schema(json: &str) -> FormulaSchema {
        serde_json::from_str(json).expect("schema 应能解析")
    }

    /// 单段、单值
    fn single() -> FormulaSchema {
        schema(
            r#"{
                "id":"s","resultName":"面积","resultSymbol":"A",
                "expression":"b*h","source":{"kind":"CUSTOM"}
            }"#,
        )
    }

    fn primary(v: f64) -> EvalResult {
        EvalResult::primary_only(v)
    }

    // ------------------------------------------------------------ 规则 3

    #[test]
    fn single_segment_uses_primary() {
        let s = single();
        let items = resolve(&s, &primary(6.0));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].symbol, "A");
        assert_eq!(items[0].name, "面积");
        assert_eq!(items[0].value, Some(6.0));
        assert!(!is_multi(&items));
    }

    /// 单段公式的 `outputs` 有一个元素（`symbol = None`），
    /// 但**仍走单值口径**（`outputs.len() == 1` 不满足 `> 1`）
    #[test]
    fn single_segment_ignores_lone_output_entry() {
        let s = single();
        let mut r = primary(6.0);
        r.outputs = vec![EvalOutput {
            symbol: None,
            value: 999.0,
        }];
        let items = resolve(&s, &r);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].value, Some(6.0), "应用 primary 而不是 outputs[0]");
    }

    #[test]
    fn result_unit_used_for_single() {
        let s = schema(
            r#"{
                "id":"s","resultName":"面积","resultSymbol":"A","resultUnit":"m2",
                "expression":"b*h","source":{"kind":"CUSTOM"}
            }"#,
        );
        let items = resolve(&s, &primary(6.0));
        assert_eq!(items[0].unit, "m2");
        assert_eq!(items[0].value_with_unit(), "6 m2");
    }

    // ------------------------------------------------------------ 规则 2

    #[test]
    fn multi_segment_without_declaration_uses_outputs() {
        let s = schema(
            r#"{
                "id":"s","resultName":"坐标","resultSymbol":"V",
                "expression":"x = a+1; y = b+2","source":{"kind":"CUSTOM"}
            }"#,
        );
        let mut r = primary(3.0);
        r.outputs = vec![
            EvalOutput { symbol: Some("x".into()), value: 1.0 },
            EvalOutput { symbol: Some("y".into()), value: 2.0 },
        ];
        let items = resolve(&s, &r);
        assert_eq!(items.len(), 2);
        assert!(is_multi(&items));
        assert_eq!(items[0].symbol, "x");
        assert_eq!(items[1].symbol, "y");
        assert_eq!(items[0].value, Some(1.0));
    }

    /// 段没有赋值符号时回落到公式级 `resultSymbol`
    #[test]
    fn symbol_less_segments_fall_back_to_result_symbol() {
        let s = schema(
            r#"{
                "id":"s","resultName":"合计","resultSymbol":"S",
                "expression":"a+1; b+2","source":{"kind":"CUSTOM"}
            }"#,
        );
        let mut r = primary(3.0);
        r.outputs = vec![
            EvalOutput { symbol: None, value: 1.0 },
            EvalOutput { symbol: None, value: 2.0 },
        ];
        let items = resolve(&s, &r);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].symbol, "S");
        assert_eq!(items[1].symbol, "S");
    }

    // ------------------------------------------------------------ 规则 1

    #[test]
    fn declared_outputs_win_and_keep_order() {
        let s = schema(
            r#"{
                "id":"s","resultName":"坐标","resultSymbol":"V",
                "expression":"x = a+1; y = b+2; t = 9",
                "resultOutputs":[{"symbol":"y","name":"Y 坐标","unit":"m"},
                                 {"symbol":"x","name":"X 坐标","unit":"m"}],
                "source":{"kind":"CUSTOM"}
            }"#,
        );
        let mut r = primary(3.0);
        r.outputs = vec![
            EvalOutput { symbol: Some("x".into()), value: 1.0 },
            EvalOutput { symbol: Some("y".into()), value: 2.0 },
            EvalOutput { symbol: Some("t".into()), value: 9.0 },
        ];
        let items = resolve(&s, &r);
        assert_eq!(items.len(), 2, "未声明的段 t 不展示");
        assert_eq!(items[0].symbol, "y", "顺序按声明，不是按表达式");
        assert_eq!(items[1].symbol, "x");
        assert_eq!(items[0].name, "Y 坐标");
    }

    /// 声明的符号在结果里找不到 → `value = None`（展示占位，不静默隐藏）
    #[test]
    fn declared_but_missing_value_is_none() {
        let s = schema(
            r#"{
                "id":"s","resultName":"坐标","resultSymbol":"V",
                "expression":"x = a+1; y = b+2",
                "resultOutputs":[{"symbol":"x"},{"symbol":"y"}],
                "source":{"kind":"CUSTOM"}
            }"#,
        );
        let mut r = primary(1.0);
        r.outputs = vec![EvalOutput { symbol: Some("x".into()), value: 1.0 }];
        let items = resolve(&s, &r);
        assert_eq!(items[0].value, Some(1.0));
        assert_eq!(items[1].value, None, "声明了却没算出来 → None");
        assert_eq!(items[1].value_text(), "—");
    }

    /// 声明里 name/unit 为空 → 回落到步骤 label / 公式级 resultUnit
    #[test]
    fn declared_blank_name_and_unit_fall_back() {
        let s = schema(
            r#"{
                "id":"s","resultName":"坐标","resultSymbol":"V","resultUnit":"m",
                "expression":"x = a+1",
                "resultOutputs":[{"symbol":"x"}],
                "stepsTemplate":[{"symbol":"x","label":"X 坐标","unit":"","expression":"a+1"}],
                "source":{"kind":"CUSTOM"}
            }"#,
        );
        let mut r = primary(1.0);
        r.outputs = vec![EvalOutput { symbol: Some("x".into()), value: 1.0 }];
        let items = resolve(&s, &r);
        assert_eq!(items[0].name, "X 坐标", "name 回落到步骤 label");
        assert_eq!(items[0].unit, "m", "unit 回落到公式级 resultUnit");
    }

    /// 声明里全空白的符号被忽略（`filter { symbol.isNotBlank() }`）
    #[test]
    fn blank_declared_symbols_are_ignored() {
        let s = schema(
            r#"{
                "id":"s","resultName":"面积","resultSymbol":"A",
                "expression":"b*h",
                "resultOutputs":[{"symbol":"  "}],
                "source":{"kind":"CUSTOM"}
            }"#,
        );
        let items = resolve(&s, &primary(6.0));
        assert_eq!(items.len(), 1, "空白声明被忽略 → 退回单值口径");
        assert_eq!(items[0].symbol, "A");
    }

    // ------------------------------------------------------------ 展示文本

    #[test]
    fn label_avoids_tautology() {
        let mut r = ResolvedResult {
            symbol: "x".into(),
            name: "x".into(),
            value: Some(1.0),
            unit: String::new(),
        };
        assert_eq!(r.label(), "x", "名称等于符号时不显示括号");

        r.name = String::new();
        assert_eq!(r.label(), "x", "名称空时只显示符号");

        r.name = "X 坐标".into();
        assert_eq!(r.label(), "x（X 坐标）");
    }

    #[test]
    fn value_text_and_unit() {
        let r = ResolvedResult {
            symbol: "A".into(),
            name: "面积".into(),
            value: Some(6.0),
            unit: "m2".into(),
        };
        assert_eq!(r.value_text(), "6");
        assert_eq!(r.value_with_unit(), "6 m2");
        assert_eq!(r.one_line(), "A = 6 m2");

        let no_unit = ResolvedResult {
            unit: String::new(),
            ..r.clone()
        };
        assert_eq!(no_unit.value_with_unit(), "6");
        assert_eq!(no_unit.one_line(), "A = 6");
    }

    #[test]
    fn copy_text_multiline() {
        let items = vec![
            ResolvedResult { symbol: "x".into(), name: "X".into(), value: Some(1.0), unit: "m".into() },
            ResolvedResult { symbol: "y".into(), name: "Y".into(), value: Some(2.0), unit: "m".into() },
        ];
        assert_eq!(build_copy_text(&items), "x = 1 m\ny = 2 m");
        assert_eq!(build_inline_text(&items), "x = 1 m, y = 2 m");
    }

    #[test]
    fn copy_text_of_single() {
        let s = single();
        let items = resolve(&s, &primary(6.0));
        assert_eq!(build_copy_text(&items), "A = 6");
    }

    /// 无值时复制文本用占位符，不写 `NaN`
    #[test]
    fn copy_text_uses_placeholder_for_missing() {
        let items = vec![ResolvedResult {
            symbol: "x".into(),
            name: "x".into(),
            value: None,
            unit: String::new(),
        }];
        assert_eq!(build_copy_text(&items), "x = —");
    }

    // ------------------------------------------------------------ 声明摘要

    #[test]
    fn declared_summary_format() {
        let outs = vec![
            ResultOutput { symbol: "x".into(), name: "未知数x".into(), unit: String::new() },
            ResultOutput { symbol: "y".into(), name: String::new(), unit: "m".into() },
            ResultOutput { symbol: "z".into(), name: String::new(), unit: String::new() },
        ];
        assert_eq!(
            build_declared_summary(&outs),
            "x（未知数x）；y（m）；z"
        );
    }

    #[test]
    fn declared_summary_joins_name_and_unit() {
        let outs = vec![ResultOutput {
            symbol: "x".into(),
            name: "坐标".into(),
            unit: "m".into(),
        }];
        assert_eq!(build_declared_summary(&outs), "x（坐标，m）");
    }

    // ------------------------------------------------------------ sanitize

    #[test]
    fn sanitize_drops_unknown_symbols_and_duplicates() {
        let s = schema(
            r#"{
                "id":"s","resultName":"坐标","resultSymbol":"V",
                "expression":"x = a+1; y = b+2",
                "resultOutputs":[
                    {"symbol":"x"},{"symbol":"x"},
                    {"symbol":"y"},{"symbol":"ghost"},{"symbol":"  "}
                ],
                "source":{"kind":"CUSTOM"}
            }"#,
        );
        let fixed = sanitize(&s);
        let symbols: Vec<&str> = fixed.iter().map(|o| o.symbol.as_str()).collect();
        assert_eq!(symbols, ["x", "y"], "应剔除 ghost、空白与重复项");
    }

    #[test]
    fn sanitize_keeps_declared_order() {
        let s = schema(
            r#"{
                "id":"s","resultName":"坐标","resultSymbol":"V",
                "expression":"x = 1; y = 2; z = 3",
                "resultOutputs":[{"symbol":"z"},{"symbol":"x"}],
                "source":{"kind":"CUSTOM"}
            }"#,
        );
        let fixed = sanitize(&s);
        assert_eq!(
            fixed.iter().map(|o| o.symbol.as_str()).collect::<Vec<_>>(),
            ["z", "x"],
            "保持声明顺序"
        );
    }

    #[test]
    fn sanitize_on_single_segment_drops_all() {
        let s = schema(
            r#"{
                "id":"s","resultName":"面积","resultSymbol":"A",
                "expression":"b*h",
                "resultOutputs":[{"symbol":"x"}],
                "source":{"kind":"CUSTOM"}
            }"#,
        );
        assert!(sanitize(&s).is_empty(), "单段公式没有分号段符号");
    }
}
