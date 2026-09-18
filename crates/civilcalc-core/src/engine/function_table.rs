//! 函数白名单与常量表（**冻结**）。
//!
//! 源：`civilcalc-android-v2/core/formula/engine/FunctionTable.kt`
//!
//! ## 白名单（20 个函数 + 2 个常量）
//!
//! 这是**冻结集合** —— 新增函数需同步修改：本表、Prompt 常量、Excel 函数映射表
//! （`excel/converter.rs`）、以及前端函数说明。
//!
//! ## ⚠️ 舍入语义的两处差异（已实测确认，勿"顺手简化"）
//!
//! 源项目用 Kotlin 标准库，其舍入语义与 Rust 默认不同：
//!
//! | Kotlin | Java 等价 | 行为 | Rust 等价 |
//! |---|---|---|---|
//! | `kotlin.math.round(x)` | `Math.rint` | **ties-to-even** | `f64::round_ties_even()` |
//! | `Double.roundToLong()` | `Math.round` | **ties toward +∞** | `(x + 0.5).floor()` |
//!
//! Rust 的 `f64::round()` 是 *away from zero*，**两者都不匹配**。
//! 实测：`rint(2.5) = 2.0`、`rint(-2.5) = -2.0`；`Math.round(-2.5) = -2`。

use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};
use std::f64::consts::{E, PI};

/// 函数定义。
///
/// `eval` 返回 `Err(msg)` 表示**定义域错误**，`msg` 为用户可见文案
/// （对齐源项目抛 `IllegalArgumentException` 的行为，由 [`crate::engine::evaluator`]
/// 转换为 `EvalError::Domain`）。
pub struct FunctionDef {
    /// 参数个数（源项目只支持固定元数）
    pub arg_count: usize,
    /// 求值函数
    pub eval: fn(&[f64]) -> Result<f64, &'static str>,
}

impl std::fmt::Debug for FunctionDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FunctionDef")
            .field("arg_count", &self.arg_count)
            .finish_non_exhaustive()
    }
}

/// 函数白名单（20 个）
pub static FUNCTIONS: Lazy<HashMap<&'static str, FunctionDef>> = Lazy::new(|| {
    let mut m = HashMap::new();

    // ---- 根式 ----
    m.insert(
        "sqrt",
        FunctionDef {
            arg_count: 1,
            eval: |a| {
                if a[0] < 0.0 {
                    return Err("sqrt 参数必须 >= 0");
                }
                Ok(a[0].sqrt())
            },
        },
    );
    m.insert(
        "cbrt",
        FunctionDef {
            arg_count: 1,
            eval: |a| Ok(a[0].cbrt()),
        },
    );

    // ---- 通用 ----
    m.insert(
        "abs",
        FunctionDef {
            arg_count: 1,
            eval: |a| Ok(a[0].abs()),
        },
    );
    m.insert(
        "pow",
        FunctionDef {
            arg_count: 2,
            eval: |a| Ok(a[0].powf(a[1])),
        },
    );

    // ---- 对数（注意：log 即自然对数）----
    m.insert(
        "log",
        FunctionDef {
            arg_count: 1,
            eval: |a| {
                if a[0] <= 0.0 {
                    return Err("log 参数必须 > 0");
                }
                Ok(a[0].ln())
            },
        },
    );
    m.insert(
        "ln",
        FunctionDef {
            arg_count: 1,
            eval: |a| {
                if a[0] <= 0.0 {
                    return Err("ln 参数必须 > 0");
                }
                Ok(a[0].ln())
            },
        },
    );
    m.insert(
        "log10",
        FunctionDef {
            arg_count: 1,
            eval: |a| {
                if a[0] <= 0.0 {
                    return Err("log10 参数必须 > 0");
                }
                Ok(a[0].log10())
            },
        },
    );
    m.insert(
        "exp",
        FunctionDef {
            arg_count: 1,
            eval: |a| Ok(a[0].exp()),
        },
    );

    // ---- 三角 ----
    m.insert(
        "sin",
        FunctionDef {
            arg_count: 1,
            eval: |a| Ok(a[0].sin()),
        },
    );
    m.insert(
        "cos",
        FunctionDef {
            arg_count: 1,
            eval: |a| Ok(a[0].cos()),
        },
    );
    m.insert(
        "tan",
        FunctionDef {
            arg_count: 1,
            eval: |a| Ok(a[0].tan()),
        },
    );
    m.insert(
        "asin",
        FunctionDef {
            arg_count: 1,
            eval: |a| {
                if !(-1.0..=1.0).contains(&a[0]) {
                    return Err("asin 参数必须在 [-1, 1] 内");
                }
                Ok(a[0].asin())
            },
        },
    );
    m.insert(
        "acos",
        FunctionDef {
            arg_count: 1,
            eval: |a| {
                if !(-1.0..=1.0).contains(&a[0]) {
                    return Err("acos 参数必须在 [-1, 1] 内");
                }
                Ok(a[0].acos())
            },
        },
    );
    m.insert(
        "atan",
        FunctionDef {
            arg_count: 1,
            eval: |a| Ok(a[0].atan()),
        },
    );

    // ---- 比较 / 取整 ----
    m.insert(
        "min",
        FunctionDef {
            arg_count: 2,
            eval: |a| Ok(a[0].min(a[1])),
        },
    );
    m.insert(
        "max",
        FunctionDef {
            arg_count: 2,
            eval: |a| Ok(a[0].max(a[1])),
        },
    );
    m.insert(
        "floor",
        FunctionDef {
            arg_count: 1,
            eval: |a| Ok(a[0].floor()),
        },
    );
    m.insert(
        "ceil",
        FunctionDef {
            arg_count: 1,
            eval: |a| Ok(a[0].ceil()),
        },
    );
    m.insert(
        "round",
        FunctionDef {
            arg_count: 1,
            // ⚠️ 必须用 round_ties_even（对齐 Kotlin round = Math.rint），
            //    不能用 Rust 默认的 round()（away from zero）
            eval: |a| Ok(a[0].round_ties_even()),
        },
    );

    // ---- 几何 ----
    m.insert(
        "hypot",
        FunctionDef {
            arg_count: 2,
            eval: |a| Ok(a[0].hypot(a[1])),
        },
    );

    m
});

/// 常量表（2 个）
pub static CONSTANTS: Lazy<HashMap<&'static str, f64>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("pi", PI);
    m.insert("e", E);
    m
});

/// 是否为白名单函数
pub fn is_function(name: &str) -> bool {
    FUNCTIONS.contains_key(name)
}

/// 是否为白名单常量
pub fn is_constant(name: &str) -> bool {
    CONSTANTS.contains_key(name)
}

/// 取函数定义
pub fn get_function(name: &str) -> Option<&'static FunctionDef> {
    FUNCTIONS.get(name)
}

/// 取常量值
pub fn get_constant(name: &str) -> Option<f64> {
    CONSTANTS.get(name).copied()
}

/// 白名单函数名集合（供 `Lexer` 使用）
pub fn function_names() -> Vec<&'static str> {
    FUNCTIONS.keys().copied().collect()
}

/// 常量名集合（供 `Lexer` 使用）
pub fn constant_names() -> Vec<&'static str> {
    CONSTANTS.keys().copied().collect()
}

/// 拥有所有权的常量表（`String` 键），供 [`crate::engine::lexer::Lexer`] 使用。
///
/// 引擎实例在构造时调用一次并持有，之后每次编译只传引用，避免重复分配。
pub fn constants_owned() -> HashMap<String, f64> {
    CONSTANTS.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
}

/// 拥有所有权的函数名集合（`String`），供 [`crate::engine::lexer::Lexer`] 使用。
pub fn function_name_set() -> HashSet<String> {
    FUNCTIONS.keys().map(|k| (*k).to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitelist_has_exactly_20_functions() {
        // 冻结集合：数量变化必须是有意为之
        assert_eq!(FUNCTIONS.len(), 20, "函数白名单应为 20 个");
    }

    #[test]
    fn whitelist_has_exactly_2_constants() {
        assert_eq!(CONSTANTS.len(), 2);
        assert!((get_constant("pi").unwrap() - PI).abs() < f64::EPSILON);
        assert!((get_constant("e").unwrap() - E).abs() < f64::EPSILON);
    }

    #[test]
    fn expected_function_names_present() {
        for name in [
            "sqrt", "cbrt", "abs", "pow", "log", "ln", "log10", "exp", "sin", "cos", "tan",
            "asin", "acos", "atan", "min", "max", "floor", "ceil", "round", "hypot",
        ] {
            assert!(is_function(name), "缺少函数: {name}");
        }
    }

    #[test]
    fn domain_errors_match_source_messages() {
        let f = |name: &str, args: &[f64]| (FUNCTIONS[name].eval)(args);

        assert_eq!(f("sqrt", &[-1.0]), Err("sqrt 参数必须 >= 0"));
        assert_eq!(f("log", &[0.0]), Err("log 参数必须 > 0"));
        assert_eq!(f("log", &[-1.0]), Err("log 参数必须 > 0"));
        assert_eq!(f("ln", &[0.0]), Err("ln 参数必须 > 0"));
        assert_eq!(f("log10", &[0.0]), Err("log10 参数必须 > 0"));
        assert_eq!(f("asin", &[1.5]), Err("asin 参数必须在 [-1, 1] 内"));
        assert_eq!(f("acos", &[-1.5]), Err("acos 参数必须在 [-1, 1] 内"));
    }

    #[test]
    fn domain_boundaries_are_inclusive() {
        let f = |name: &str, args: &[f64]| (FUNCTIONS[name].eval)(args);

        assert!(f("sqrt", &[0.0]).is_ok(), "sqrt(0) 合法");
        assert!(f("asin", &[1.0]).is_ok(), "asin(1) 合法");
        assert!(f("asin", &[-1.0]).is_ok(), "asin(-1) 合法");
        assert!(f("acos", &[1.0]).is_ok());
        assert!(f("log", &[f64::MIN_POSITIVE]).is_ok(), "极小正数合法");
    }

    #[test]
    fn log_is_natural_log() {
        // 源项目约定：log = ln（自然对数）
        let l = (FUNCTIONS["log"].eval)(&[E]).unwrap();
        let n = (FUNCTIONS["ln"].eval)(&[E]).unwrap();
        assert!((l - 1.0).abs() < 1e-12, "log(e) 应为 1");
        assert!((n - 1.0).abs() < 1e-12, "ln(e) 应为 1");
        assert!((l - n).abs() < f64::EPSILON);
    }

    #[test]
    fn log10_is_common_log() {
        assert!(((FUNCTIONS["log10"].eval)(&[1000.0]).unwrap() - 3.0).abs() < 1e-12);
    }

    /// ⚠️ 关键差异：Kotlin round = Math.rint = ties-to-even
    #[test]
    fn round_uses_ties_to_even() {
        let r = |x: f64| (FUNCTIONS["round"].eval)(&[x]).unwrap();

        assert_eq!(r(0.5), 0.0, "rint(0.5) = 0");
        assert_eq!(r(1.5), 2.0, "rint(1.5) = 2");
        assert_eq!(r(2.5), 2.0, "rint(2.5) = 2（ties-to-even，不是 3）");
        assert_eq!(r(-2.5), -2.0, "rint(-2.5) = -2（不是 -3）");
        assert_eq!(r(2.4), 2.0);
        assert_eq!(r(2.6), 3.0);
    }

    #[test]
    fn other_functions_behave() {
        let f = |name: &str, args: &[f64]| (FUNCTIONS[name].eval)(args).unwrap();

        assert!((f("cbrt", &[27.0]) - 3.0).abs() < 1e-12);
        assert!((f("abs", &[-2.5]) - 2.5).abs() < f64::EPSILON);
        assert!((f("pow", &[2.0, 10.0]) - 1024.0).abs() < 1e-9);
        assert!((f("exp", &[0.0]) - 1.0).abs() < f64::EPSILON);
        assert!((f("min", &[3.0, 1.0]) - 1.0).abs() < f64::EPSILON);
        assert!((f("max", &[3.0, 1.0]) - 3.0).abs() < f64::EPSILON);
        assert!((f("floor", &[2.9]) - 2.0).abs() < f64::EPSILON);
        assert!((f("ceil", &[2.1]) - 3.0).abs() < f64::EPSILON);
        assert!((f("hypot", &[3.0, 4.0]) - 5.0).abs() < 1e-12);
        assert!((f("sin", &[0.0])).abs() < f64::EPSILON);
        assert!((f("cos", &[0.0]) - 1.0).abs() < f64::EPSILON);
        assert!((f("tan", &[0.0])).abs() < f64::EPSILON);
        assert!((f("atan", &[0.0])).abs() < f64::EPSILON);
    }
}
