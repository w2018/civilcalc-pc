//! 表达式引擎（自研，ADR-013 —— 不引入第三方解析库）。
//!
//! 源：`civilcalc-android-v2/core/formula/engine/`（9 文件约 940 行）
//!
//! ## 为什么自研（ADR-013）
//!
//! 1. **行为一致性**：源项目引擎承载 27 个 BUG 修复沉淀（隐含乘号、科学计数法回退、
//!    一元负号判定顺序、4 位小数舍入、NaN/Inf 显式报错、除零报错文案）
//! 2. **RPN → Excel 是内建需求**：第三方库不暴露 RPN，无法做 Excel 转换
//! 3. **错误文案用户可见**：第三方库错误信息无法对齐
//! 4. **函数白名单与定义域需精确控制**
//!
//! ## 移植顺序（严格依赖）
//!
//! ```text
//! types.rs → function_table.rs → lexer.rs → parser.rs → evaluator.rs
//!   → splitter.rs → implicit_mul.rs → expr_utils.rs → mod.rs
//! ```
//!
//! | 文件 | 源文件 | 任务 |
//! |---|---|---|
//! | `types.rs` | `FormulaEngine.kt` | P0-4 |
//! | `function_table.rs` | `FunctionTable.kt` | P0-4 |
//! | `lexer.rs` | `Lexer.kt` | P0-5 |
//! | `parser.rs` | `Parser.kt` | P0-6 |
//! | `evaluator.rs` | `Evaluator.kt` | P0-7 |
//! | `splitter.rs` | `ExpressionSplitter.kt` | P0-8 |
//! | `expr_utils.rs` | `ExprUtils.kt` | P0-8 |
//! | `implicit_mul.rs` | `ImplicitMultiplication.kt` | P0-9（ADR-016 手写单趟扫描） |
//! | `mod.rs` | `FormulaEngineImpl.kt` | P0-10 |
//! | `cache.rs` | —（**PC 端新增**） | P2-2 |

pub mod cache;
pub mod evaluator;
pub mod expr_utils;
pub mod function_table;
pub mod implicit_mul;
pub mod lexer;
pub mod parser;
pub mod splitter;
pub mod steps;
pub mod types;

pub use steps::eval_steps;

use crate::schema::{EvalError, EvalOutput, EvalResult, FormulaSchema, FormulaVar};
use cache::CachedCompile;
use std::collections::HashMap;
use types::{CompileResult, CompiledExpr, FormulaResult};

/// 编译表达式。
///
/// 对齐源项目 `FormulaEngineImpl.compile`：
/// - 常量表 = `FunctionTable.constants` ∪ `constants`（**调用方自定义常量优先**）
/// - 表达式先经 [`normalize`] 取**最后一段**（多输出公式的单表达式视图）
/// - 任何异常（含词法错误）→ `CompileResult::Error { position: None, .. }`
///   —— 源项目此处丢弃位置信息，只保留文案
pub fn compile(expr: &str, constants: &HashMap<String, f64>) -> CompileResult {
    let mut merged = function_table::constants_owned();
    for (k, v) in constants {
        merged.insert(k.clone(), *v);
    }
    let functions = function_table::function_name_set();

    let normalized = normalize(expr);
    let mut lx = lexer::Lexer::new(&normalized, &merged, &functions);
    let tokens = match lx.tokenize() {
        Ok(t) => t,
        Err(msg) => {
            return CompileResult::Error {
                position: None,
                reason: msg,
            }
        }
    };
    parser::parse(&tokens)
}

/// 取最后一个分号段并剥离赋值前缀（`compile` 的单表达式视图）。
///
/// 多输出求值走 [`eval_multi`]。
fn normalize(expr: &str) -> String {
    splitter::split(expr)
        .last()
        .map(|seg| seg.expression.clone())
        .unwrap_or_else(|| expr.to_string())
}

/// [`CompileResult`] → 缓存表示（`Ok` 包成 `Arc` 以便命中时零克隆）
fn to_cached(r: CompileResult) -> CachedCompile {
    match r {
        CompileResult::Ok { expr } => CachedCompile::Ok(std::sync::Arc::new(expr)),
        CompileResult::Error { position, reason } => CachedCompile::Err { position, reason },
    }
}

/// 编译表达式，**走全局 LRU 缓存**（P2-2）。
///
/// 与 [`compile`] 的区别只是「先查缓存」。缓存键是
/// **`(表达式, 常量表)`** —— 因为常量值会被烤进 RPN，
/// 只用表达式做键会静默算错（详见 [`cache`] 模块文档）。
///
/// 热路径（`eval_multi`）用内部的 `Arc` 版本避免克隆；
/// 这个函数返回 [`CompileResult`]（命中时克隆一次 RPN），给外部调用方用。
pub fn compile_cached(expr: &str, constants: &HashMap<String, f64>) -> CompileResult {
    compile_shared(expr, constants).to_result()
}

/// 编译表达式，返回**零克隆**的缓存表示（热路径专用）。
fn compile_shared(expr: &str, constants: &HashMap<String, f64>) -> CachedCompile {
    let key = cache::cache_key(expr, constants);
    cache::global_cache().get_or_compile(&key, || to_cached(compile(expr, constants)))
}

/// 清空全局编译缓存（"重置软件" / 排障用）
pub fn clear_compile_cache() {
    cache::global_cache().clear();
}

/// 全局编译缓存统计（排障用）
pub fn compile_cache_stats() -> cache::CacheStats {
    cache::global_cache().stats()
}

/// 对已编译表达式求值
pub fn eval(
    compiled: &CompiledExpr,
    inputs: &HashMap<String, f64>,
) -> FormulaResult<EvalResult, EvalError> {
    evaluator::eval(compiled, inputs)
}

/// 多输出求值（BUG-01）。
///
/// 按分号拆段逐段编译求值，**每段输出绑定其赋值符号**，
/// 后续段可引用前序段结果；`primary` = **最后一段**（与旧行为一致）。
pub fn eval_multi(
    expr: &str,
    constants: &HashMap<String, f64>,
    inputs: &HashMap<String, f64>,
) -> FormulaResult<EvalResult, EvalError> {
    let segments = splitter::split(expr);
    if segments.is_empty() {
        return Err(EvalError::compile("空表达式"));
    }

    let mut outputs: Vec<EvalOutput> = Vec::with_capacity(segments.len());
    let mut steps = Vec::new();
    // 逐段累加的作用域：段结果注入后，后续段可引用
    let mut scope: HashMap<String, f64> = inputs.clone();

    for (index, seg) in segments.iter().enumerate() {
        // 走缓存（P2-2）：热路径是「表达式不变、只改输入值」，
        // 用 `Arc` 版本可让命中时**零克隆**。
        let compiled = match compile_shared(&seg.expression, constants) {
            CachedCompile::Ok(arc) => arc,
            CachedCompile::Err { position, reason } => {
                let sym_part = seg
                    .symbol
                    .as_ref()
                    .map(|s| format!("({s})"))
                    .unwrap_or_default();
                return Err(EvalError::Compile {
                    position,
                    msg: format!("第{}段{}编译失败: {}", index + 1, sym_part, reason),
                });
            }
        };

        let result = evaluator::eval(&compiled, &scope)?;
        outputs.push(EvalOutput {
            symbol: seg.symbol.clone(),
            value: result.primary,
        });
        steps.extend(result.steps);

        // 段结果绑定赋值符号，供后续段引用
        if let Some(sym) = &seg.symbol {
            scope.insert(sym.clone(), result.primary);
        }
    }

    let primary = outputs
        .last()
        .map(|o| o.value)
        .ok_or_else(|| EvalError::compile("空表达式"))?;

    Ok(EvalResult {
        primary,
        outputs,
        branches: Vec::new(),
        steps,
        step_results: Vec::new(),
        warnings: Vec::new(),
    })
}

// =============================================================================
// 求值输入准备（供命令层复用）
// =============================================================================

/// 构建求值上下文：常量 ∪ 输入 ∪ 变量的 default 兜底。
///
/// 对齐源项目 `EvalUseCase` 的行为：未提供值但有 `default` 的变量用默认值补齐。
pub fn build_eval_context(
    schema: &FormulaSchema,
    inputs: &HashMap<String, f64>,
) -> HashMap<String, f64> {
    let mut ctx = schema.constants.clone();
    for (k, v) in inputs {
        ctx.insert(k.clone(), *v);
    }
    for var in &schema.variables {
        if let Some(default) = var.default {
            ctx.entry(var.symbol.clone()).or_insert(default);
        }
    }
    ctx
}

/// 定义域检查：返回越界告警文案（`变量标签: 不能小于/大于 X`）。
///
/// 对齐源项目：越界只产生 **warning**（不阻断求值），
/// 真正的定义域错误由引擎（如 `sqrt` 负数）以 `EvalError::Domain` 抛出。
pub fn check_domain(var: &FormulaVar, value: f64) -> Option<String> {
    if let Some(min) = var.min {
        if value < min {
            return Some(format!("{} 不能小于 {}", var.symbol, min));
        }
    }
    if let Some(max) = var.max {
        if value > max {
            return Some(format!("{} 不能大于 {}", var.symbol, max));
        }
    }
    None
}

// `FormulaVar` 的便捷访问（Kotlin 侧字段名就是 symbol）

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    fn no_constants() -> HashMap<String, f64> {
        HashMap::new()
    }

    fn compile_ok(expr: &str) -> CompiledExpr {
        match compile(expr, &no_constants()) {
            CompileResult::Ok { expr } => expr,
            CompileResult::Error { reason, .. } => panic!("编译应成功: {reason}"),
        }
    }

    // ---------------- compile ----------------

    #[test]
    fn compile_succeeds_for_basic_expression() {
        assert!(compile("a+b", &no_constants()).is_ok());
    }

    #[test]
    fn compile_lexer_error_has_no_position() {
        // 对齐源项目：`compile` 只捕获**词法异常**，此时位置信息被丢弃（文案里带位置）
        match compile("1 @ 2", &no_constants()) {
            CompileResult::Error { position, reason } => {
                assert_eq!(position, None);
                assert_eq!(reason, "未知字符: '@' at position 2");
            }
            other => panic!("应报错，实际 {other:?}"),
        }
    }

    #[test]
    fn compile_parser_error_keeps_position() {
        // 解析器返回的错误**保留位置**（与源项目一致：parser 不抛异常，直接返回 Error）
        match compile("(1+2", &no_constants()) {
            CompileResult::Error { position, reason } => {
                assert_eq!(reason, "括号不匹配");
                assert_eq!(position, Some(0));
            }
            other => panic!("应报错，实际 {other:?}"),
        }
    }

    /// 源项目行为：`1+` 能**通过编译**（RPN 为 `1 +`），错误在求值阶段才暴露
    /// （栈不足 → `EvalError::Compile`）。这不是移植缺陷。
    #[test]
    fn incomplete_expression_compiles_but_fails_at_eval() {
        let compiled = compile_ok("1+");
        assert_eq!(compiled.rpn.len(), 2);
        let r = eval(&compiled, &HashMap::new());
        assert!(r.is_err(), "求值阶段应报错");
    }

    #[test]
    fn compile_merges_custom_constants() {
        // 自定义常量应被识别（不是变量）
        let mut c = HashMap::new();
        c.insert("k".to_string(), 2.5);
        let compiled = match compile("k*x", &c) {
            CompileResult::Ok { expr } => expr,
            other => panic!("应成功: {other:?}"),
        };
        // k 是常量 → 只剩 x 是变量
        assert_eq!(compiled.variables, vec!["x"]);
    }

    #[test]
    fn compile_normalizes_to_last_segment() {
        // `a = 1; b = a + 2` 的单表达式视图 = 最后一段 `a + 2`
        let compiled = compile_ok("a = 1; b = a + 2");
        assert_eq!(compiled.variables, vec!["a"]);
    }

    // ---------------- eval_multi（源项目 FormulaEngineMultiOutputTest） ----------------

    /// BUG-01：多输出公式所有段全部输出（此前只算最后一段）
    #[test]
    fn bug01_multi_output_all_segments_evaluated() {
        let r = eval_multi(
            "X2 = X1 + D * cos(alpha); Y2 = Y1 + D * sin(alpha)",
            &no_constants(),
            &inputs(&[("X1", 100.0), ("Y1", 200.0), ("D", 50.0), ("alpha", 0.0)]),
        )
        .expect("求值应成功");

        assert_eq!(r.outputs.len(), 2);
        assert_eq!(r.outputs[0].symbol.as_deref(), Some("X2"));
        assert!((r.outputs[0].value - 150.0).abs() < 1e-9);
        assert_eq!(r.outputs[1].symbol.as_deref(), Some("Y2"));
        assert!((r.outputs[1].value - 200.0).abs() < 1e-9);
        // primary = 最后一段
        assert!((r.primary - 200.0).abs() < 1e-9);
    }

    /// BUG-01：跨段引用 —— 后续段可引用前序段输出
    #[test]
    fn bug01_cross_segment_reference() {
        let r = eval_multi("s = 2 * 3; area = s * 5", &no_constants(), &HashMap::new())
            .expect("求值应成功");
        assert_eq!(r.outputs.len(), 2);
        assert!((r.outputs[0].value - 6.0).abs() < 1e-9);
        assert!((r.outputs[1].value - 30.0).abs() < 1e-9);
    }

    /// BUG-01：单输出表达式 outputs 恰 1 项，行为不变
    #[test]
    fn bug01_single_expression_unchanged() {
        let r = eval_multi("a+b", &no_constants(), &inputs(&[("a", 1.0), ("b", 2.0)]))
            .expect("求值应成功");
        assert_eq!(r.outputs.len(), 1);
        assert_eq!(r.outputs[0].symbol, None);
        assert!((r.primary - 3.0).abs() < 1e-9);
    }

    /// BUG-01：中段 `=`（等式型）编译失败并向上报错，不再静默
    #[test]
    fn bug01_mid_equality_fails_loudly() {
        let r = eval_multi("sin(a) = 1", &no_constants(), &inputs(&[("a", 0.5)]));
        assert!(r.is_err(), "应失败");
    }

    /// BUG-01：某段求值失败（缺变量）时报错指向该段
    #[test]
    fn bug01_missing_variable_reported() {
        let r = eval_multi("a = 1; b = c * 2", &no_constants(), &HashMap::new());
        match r {
            Err(EvalError::Compile { .. }) => {}
            other => panic!("应为 Compile 错误，实际 {other:?}"),
        }
    }

    #[test]
    fn eval_multi_empty_expression_fails() {
        let r = eval_multi("", &no_constants(), &HashMap::new());
        match r {
            Err(EvalError::Compile { msg, .. }) => assert_eq!(msg, "空表达式"),
            other => panic!("应为 Compile 错误，实际 {other:?}"),
        }
    }

    // ---------------- 定义域检查 ----------------

    #[test]
    fn check_domain_reports_bounds() {
        let var = FormulaVar {
            symbol: "fc".to_string(),
            desc: "混凝土强度".to_string(),
            unit: Some("N/mm²".to_string()),
            default: None,
            min: Some(0.0),
            max: Some(100.0),
            required: true,
        };
        assert_eq!(check_domain(&var, -1.0).as_deref(), Some("fc 不能小于 0"));
        assert_eq!(check_domain(&var, 200.0).as_deref(), Some("fc 不能大于 100"));
        assert_eq!(check_domain(&var, 50.0), None);
    }

    #[test]
    fn build_eval_context_merges_defaults() {
        let schema = FormulaSchema {
            id: "usr:t".to_string(),
            result_name: "t".to_string(),
            result_symbol: "y".to_string(),
            result_unit: None,
            result_outputs: vec![],
            expression: "a+b".to_string(),
            source_equations: vec![],
            alt_expressions: vec![],
            constants: HashMap::new(),
            variables: vec![
                FormulaVar {
                    symbol: "a".to_string(),
                    desc: "a".to_string(),
                    unit: None,
                    default: Some(1.0),
                    min: None,
                    max: None,
                    required: true,
                },
                FormulaVar {
                    symbol: "b".to_string(),
                    desc: "b".to_string(),
                    unit: None,
                    default: Some(2.0),
                    min: None,
                    max: None,
                    required: true,
                },
            ],
            domain: String::new(),
            tags: vec![],
            reference_basis: None,
            design_notes: None,
            explanation: None,
            image_ids: vec![],
            revised_from: None,
            source: crate::schema::FormulaSource::custom(),
            steps_template: None,
            doc_template_id: None,
            excel_expression: None,
            excel_alt_expressions: None,
            excel_steps_template: None,
            excel_function_docs: None,
            schema_version: crate::schema::CURRENT_SCHEMA_VERSION,
            created_at: 0,
            updated_at: 0,
        };

        // 无输入 → 用 default 补齐
        let ctx = build_eval_context(&schema, &HashMap::new());
        assert_eq!(ctx.get("a"), Some(&1.0));
        assert_eq!(ctx.get("b"), Some(&2.0));

        // 显式输入覆盖 default
        let ctx2 = build_eval_context(&schema, &inputs(&[("a", 9.0)]));
        assert_eq!(ctx2.get("a"), Some(&9.0));
        assert_eq!(ctx2.get("b"), Some(&2.0));
    }

    /// **接线回归**：`eval_multi` 第二次求值应命中编译缓存（P2-2）
    ///
    /// 用**唯一表达式**避免与其他测试互相干扰；断言只看 hits 单调增长
    /// （其他测试只会增加 hits，不会减少）。
    #[test]
    fn eval_multi_hits_compile_cache_on_second_call() {
        let constants = HashMap::from([("kk_cache".to_string(), 2.0)]);
        let inputs = HashMap::from([("aa_cache".to_string(), 1.0)]);
        let expr = "aa_cache * kk_cache + 1";

        let r1 = eval_multi(expr, &constants, &inputs).unwrap();
        let s1 = compile_cache_stats();

        let r2 = eval_multi(expr, &constants, &inputs).unwrap();
        let s2 = compile_cache_stats();

        assert_eq!(r1.primary, r2.primary, "两次结果应一致");
        assert_eq!(r1.primary, 3.0);
        assert!(
            s2.hits > s1.hits,
            "第二次求值应命中编译缓存（hits {} → {}）",
            s1.hits,
            s2.hits
        );
    }

    /// 缓存**不得**跨常量表串味（同一表达式、不同常量 → 不同结果）
    #[test]
    fn cache_does_not_leak_between_constant_tables() {
        let inputs = HashMap::from([("aa_leak".to_string(), 10.0)]);
        let expr = "aa_leak * kk_leak";

        let c1 = HashMap::from([("kk_leak".to_string(), 2.0)]);
        let c2 = HashMap::from([("kk_leak".to_string(), 3.0)]);

        let r1 = eval_multi(expr, &c1, &inputs).unwrap();
        let r2 = eval_multi(expr, &c2, &inputs).unwrap();
        let r1_again = eval_multi(expr, &c1, &inputs).unwrap();

        assert_eq!(r1.primary, 20.0);
        assert_eq!(r2.primary, 30.0, "常量不同必须算出不同结果（缓存键含常量表）");
        assert_eq!(r1_again.primary, 20.0, "回到 c1 应仍得 20");
    }
}
