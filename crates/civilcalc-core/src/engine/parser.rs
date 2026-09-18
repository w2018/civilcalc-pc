//! 表达式语法分析（Shunting-yard → 逆波兰）。
//!
//! 源：`civilcalc-android-v2/core/formula/engine/Parser.kt`
//!
//! ## 输出为什么是 RPN 而不是 AST
//!
//! 源项目的 **Excel 转换直接消费 RPN**（`convertRpnToExcelWithRefs`）——
//! 保留 RPN 是该能力的前提（见 ADR-013）。
//!
//! ## 优先级与结合性（与 [`crate::engine::types`] 保持一致）
//!
//! | 运算符 | 优先级 | 结合性 |
//! |---|---|---|
//! | `+` `-` | 1 | 左 |
//! | `*` `/` `%` | 2 | 左 |
//! | `^` | 3 | **右** |
//! | `u-`（一元负号） | 4 | **右** |
//!
//! ## 与源项目的一处简化
//!
//! 源项目 `Parser(functions, constants)` 的构造参数**实际未被使用**
//! （函数元数直接从 `FunctionTable` 全局表取）。Rust 版去掉这两个死参数，
//! 避免未使用参数告警。

use crate::engine::function_table;
use crate::engine::lexer::{Token, TokenType};
use crate::engine::types::{
    precedence_of, CompileResult, CompiledExpr, RpnToken, OP_UNARY_MINUS,
};

/// 解析 token 序列为 RPN。
///
/// 失败时返回带**字符位置**的错误（对齐源项目：解析错误带位置，
/// 词法错误经 `compile()` 包装后位置为 `None`）。
pub fn parse(tokens: &[Token]) -> CompileResult {
    let mut output: Vec<RpnToken> = Vec::new();
    let mut op_stack: Vec<&Token> = Vec::new();

    for token in tokens {
        match token.kind {
            TokenType::Number => output.push(RpnToken::Number {
                value: token.number_value,
            }),
            TokenType::Variable => output.push(RpnToken::Variable {
                name: token.value.clone(),
            }),
            TokenType::Constant => output.push(RpnToken::Constant {
                name: token.value.clone(),
                value: token.number_value,
            }),

            TokenType::Function => op_stack.push(token),

            TokenType::Comma => {
                // 弹出栈顶运算符，直到遇到左括号为止（左括号保留在栈上）
                while let Some(op) = op_stack.pop_if(|t| t.kind != TokenType::LParen) {
                    match pop_operator(op) {
                        Ok(t) => output.push(t),
                        Err(reason) => {
                            return CompileResult::Error {
                                position: Some(op.position),
                                reason,
                            }
                        }
                    }
                }
                if op_stack.is_empty() {
                    return CompileResult::Error {
                        position: Some(token.position),
                        reason: "逗号前缺少左括号".to_string(),
                    };
                }
            }

            TokenType::LParen => op_stack.push(token),

            TokenType::RParen => {
                // 弹出运算符直到遇到左括号（左括号本身也要消费掉）
                while let Some(op) = op_stack.pop_if(|t| t.kind != TokenType::LParen) {
                    match pop_operator(op) {
                        Ok(t) => output.push(t),
                        Err(reason) => {
                            return CompileResult::Error {
                                position: Some(op.position),
                                reason,
                            }
                        }
                    }
                }

                if !matches!(op_stack.last().map(|t| t.kind), Some(TokenType::LParen)) {
                    return CompileResult::Error {
                        position: Some(token.position),
                        reason: "缺少左括号".to_string(),
                    };
                }
                op_stack.pop(); // 消费左括号

                // 左括号之前若是函数名 → 收尾为函数调用
                if let Some(func) = op_stack.pop_if(|t| t.kind == TokenType::Function) {
                    let arg_count = function_table::get_function(&func.value)
                        .map(|d| d.arg_count)
                        .unwrap_or(1);
                    output.push(RpnToken::Function {
                        name: func.value.clone(),
                        arg_count,
                    });
                }
            }

            TokenType::UnaryMinus => {
                // 右结合、优先级最高：弹出所有优先级 > 4 的（实际没有），
                // 以及优先级 == 4 且非右结合的（实际没有）→ 基本不弹
                while should_pop_before_push(op_stack.last(), token) {
                    let op = op_stack.pop().expect("已确认非空");
                    match pop_operator(op) {
                        Ok(t) => output.push(t),
                        Err(reason) => {
                            return CompileResult::Error {
                                position: Some(op.position),
                                reason,
                            }
                        }
                    }
                }
                op_stack.push(token);
            }

            TokenType::Plus
            | TokenType::Minus
            | TokenType::Multiply
            | TokenType::Divide
            | TokenType::Power
            | TokenType::Modulo => {
                while should_pop_before_push(op_stack.last(), token) {
                    let op = op_stack.pop().expect("已确认非空");
                    match pop_operator(op) {
                        Ok(t) => output.push(t),
                        Err(reason) => {
                            return CompileResult::Error {
                                position: Some(op.position),
                                reason,
                            }
                        }
                    }
                }
                op_stack.push(token);
            }

            TokenType::End => {}
        }
    }

    // 收尾：弹出剩余运算符
    while let Some(op) = op_stack.pop() {
        if matches!(op.kind, TokenType::LParen | TokenType::RParen) {
            return CompileResult::Error {
                position: Some(op.position),
                reason: "括号不匹配".to_string(),
            };
        }
        match pop_operator(op) {
            Ok(t) => output.push(t),
            Err(reason) => {
                return CompileResult::Error {
                    position: Some(op.position),
                    reason,
                }
            }
        }
    }

    // 变量名与函数名去重（保持首现顺序）
    let mut variables: Vec<String> = Vec::new();
    let mut functions: Vec<String> = Vec::new();
    for t in &output {
        match t {
            RpnToken::Variable { name } if !variables.contains(name) => {
                variables.push(name.clone());
            }
            RpnToken::Function { name, .. } if !functions.contains(name) => {
                functions.push(name.clone());
            }
            _ => {}
        }
    }

    CompileResult::Ok {
        expr: CompiledExpr {
            rpn: output,
            variables,
            functions,
        },
    }
}

/// 是否应在压入 `incoming` 之前弹出栈顶运算符。
///
/// 规则（对齐源项目）：
/// - 栈顶必须是运算符
/// - 栈顶优先级 **>** 入栈优先级 → 弹
/// - 优先级相等且入栈运算符**左结合** → 弹（左结合先算左边）
fn should_pop_before_push(top: Option<&&Token>, incoming: &Token) -> bool {
    let Some(top) = top else { return false };
    if !top.is_operator() {
        return false;
    }
    let top_prec = token_precedence(top);
    let inc_prec = token_precedence(incoming);
    top_prec > inc_prec || (top_prec == inc_prec && !token_right_assoc(incoming))
}

/// token 对应的运算符优先级
fn token_precedence(t: &Token) -> i32 {
    if t.kind == TokenType::UnaryMinus {
        return precedence_of(OP_UNARY_MINUS);
    }
    precedence_of(&t.value)
}

/// token 对应的运算符是否右结合
fn token_right_assoc(t: &Token) -> bool {
    if t.kind == TokenType::UnaryMinus {
        return true;
    }
    crate::engine::types::is_right_associative(&t.value)
}

/// 运算符 token → RPN 运算符节点（含冗余的优先级与结合性，供 Excel 转换直接使用）
fn pop_operator(token: &Token) -> Result<RpnToken, String> {
    let (op, prec, right) = match token.kind {
        TokenType::Plus => ("+", 1, false),
        TokenType::Minus => ("-", 1, false),
        TokenType::Multiply => ("*", 2, false),
        TokenType::Divide => ("/", 2, false),
        TokenType::Power => ("^", 3, true),
        TokenType::Modulo => ("%", 2, false),
        TokenType::UnaryMinus => (OP_UNARY_MINUS, 4, true),
        other => return Err(format!("非运算符: {other:?}")),
    };
    Ok(RpnToken::Operator {
        op: op.to_string(),
        precedence: prec,
        right_associative: right,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::lexer::Lexer;

    fn parse_expr(input: &str) -> CompileResult {
        let constants = function_table::constants_owned();
        let functions = function_table::function_name_set();
        let mut lx = Lexer::new(input, &constants, &functions);
        let tokens = lx.tokenize().expect("词法分析应成功");
        parse(&tokens)
    }

    /// RPN 的紧凑表示（便于断言）
    fn rpn(input: &str) -> String {
        let cr = parse_expr(input);
        let expr = cr.expr().expect("解析应成功");
        expr.rpn
            .iter()
            .map(|t| match t {
                RpnToken::Number { value } => format!("{value}"),
                RpnToken::Variable { name } => name.clone(),
                RpnToken::Constant { name, .. } => name.clone(),
                RpnToken::Operator { op, .. } => op.clone(),
                RpnToken::Function { name, .. } => format!("{name}()"),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn precedence_mul_before_add() {
        assert_eq!(rpn("1+2*3"), "1 2 3 * +");
    }

    #[test]
    fn left_associativity_of_subtraction() {
        // 1-2-3 → (1-2)-3
        assert_eq!(rpn("1-2-3"), "1 2 - 3 -");
    }

    #[test]
    fn right_associativity_of_power() {
        // 2^3^2 → 2^(3^2)
        assert_eq!(rpn("2^3^2"), "2 3 2 ^ ^");
    }

    #[test]
    fn parentheses_override_precedence() {
        assert_eq!(rpn("(1+2)*3"), "1 2 + 3 *");
    }

    #[test]
    fn unary_minus_binds_tightest() {
        // -3^2 → (-3)^2（一元负号优先级 4 > ^ 的 3）
        assert_eq!(rpn("-3^2"), "3 u- 2 ^");
    }

    #[test]
    fn function_call_becomes_rpn_function() {
        assert_eq!(rpn("sqrt(4)"), "4 sqrt()");
    }

    #[test]
    fn two_arg_function() {
        assert_eq!(rpn("pow(2,10)"), "2 10 pow()");
    }

    #[test]
    fn nested_function_calls() {
        assert_eq!(rpn("sqrt(pow(3,2))"), "3 2 pow() sqrt()");
    }

    #[test]
    fn constants_are_inlined() {
        // 常量在编译期即替换为字面量
        let cr = parse_expr("pi");
        let expr = cr.expr().unwrap();
        assert_eq!(expr.rpn.len(), 1);
        match &expr.rpn[0] {
            RpnToken::Constant { name, value } => {
                assert_eq!(name, "pi");
                assert!((value - std::f64::consts::PI).abs() < f64::EPSILON);
            }
            other => panic!("应为常量，实际 {other:?}"),
        }
    }

    #[test]
    fn variables_and_functions_are_deduped_in_first_occurrence_order() {
        let cr = parse_expr("a+b*a+sqrt(c)+sqrt(d)");
        let expr = cr.expr().unwrap();
        assert_eq!(expr.variables, vec!["a", "b", "c", "d"]);
        assert_eq!(expr.functions, vec!["sqrt"]);
    }

    #[test]
    fn missing_lparen_reports_position() {
        let cr = parse_expr("1+2)");
        match cr {
            CompileResult::Error { position, reason } => {
                assert_eq!(reason, "缺少左括号");
                assert_eq!(position, Some(3));
            }
            other => panic!("应报错，实际 {other:?}"),
        }
    }

    #[test]
    fn unclosed_lparen_reports_mismatch() {
        let cr = parse_expr("(1+2");
        match cr {
            CompileResult::Error { reason, .. } => assert_eq!(reason, "括号不匹配"),
            other => panic!("应报错，实际 {other:?}"),
        }
    }

    #[test]
    fn comma_without_lparen_reports() {
        // `1,2` —— 逗号在栈空时出现
        let cr = parse_expr("1,2");
        match cr {
            CompileResult::Error { reason, .. } => assert_eq!(reason, "逗号前缺少左括号"),
            other => panic!("应报错，实际 {other:?}"),
        }
    }

    /// 源项目 ExprUtilsParensTest 的括号场景在 RPN 层的对应验证
    #[test]
    fn bug06_operator_order_preserved_in_rpn() {
        // k/((a+b)*(c+d))
        assert_eq!(rpn("k/((a+b)*(c+d))"), "k a b + c d + * /");
        // x-(y+z)
        assert_eq!(rpn("x-(y+z)"), "x y z + -");
    }
}
