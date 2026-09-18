//! 表达式词法分析。
//!
//! 源：`civilcalc-android-v2/core/formula/engine/Lexer.kt`
//!
//! ## 两个易错点（源项目 BUG 修复沉淀，勿简化）
//!
//! ### 1. 科学计数法的**回退**（BUG-07）
//!
//! 读到 `e` / `E` 后先**试探**：只有后面跟「数字」或「正负号 + 数字」才认定为指数；
//! 否则**回退游标到 `e` 之前**。否则变量 `e1`、常量 `e` 会被数字吞掉。
//!
//! ### 2. 一元负号按**前驱 token 类型**判定
//!
//! `-` 是一元还是二元，取决于它前面是什么 token：
//! - 开头（无前驱）、或前驱是运算符 / 左括号 / 逗号 / 一元负号 → **一元负号**（`u-`）
//! - 否则 → 二元减法
//!
//! 判定顺序敏感：`2--3`、`(-3)`、`2*-3`、`a,-3` 都要正确。

use crate::engine::types::OP_UNARY_MINUS;
use std::collections::{HashMap, HashSet};

/// Token 类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenType {
    Number,
    Variable,
    Constant,
    Function,
    Plus,
    Minus,
    Multiply,
    Divide,
    Power,
    Modulo,
    LParen,
    RParen,
    Comma,
    /// 一元负号（由 [`Lexer::process_unary_minus`] 在词法阶段标定）
    UnaryMinus,
    End,
}

/// Token。
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenType,
    /// 原始文本（运算符为符号本身；一元负号为 `"u-"`；数字为字面量文本）
    pub value: String,
    /// 在输入中的字符位置（0 起）
    pub position: usize,
    /// 数字字面量的值（仅 `Number` / `Constant` 有效）
    pub number_value: f64,
}

impl Token {
    fn new(kind: TokenType, value: impl Into<String>, position: usize) -> Self {
        Self {
            kind,
            value: value.into(),
            position,
            number_value: 0.0,
        }
    }

    fn with_number(kind: TokenType, value: impl Into<String>, position: usize, n: f64) -> Self {
        Self {
            kind,
            value: value.into(),
            position,
            number_value: n,
        }
    }

    /// 是否为运算符（含一元负号）
    pub fn is_operator(&self) -> bool {
        matches!(
            self.kind,
            TokenType::Plus
                | TokenType::Minus
                | TokenType::Multiply
                | TokenType::Divide
                | TokenType::Power
                | TokenType::Modulo
                | TokenType::UnaryMinus
        )
    }
}

/// 词法分析器。
///
/// 用 `Vec<char>` 而非字节切片，以对齐 Kotlin 的 `Char` 索引语义
/// （位置信息会出现在用户可见的错误文案里）。
///
/// `constants` / `functions` 用**引用**传入：引擎实例构造时建立一次并持有，
/// 每次编译只传引用，避免重复分配。
pub struct Lexer<'a> {
    input: &'a str,
    chars: Vec<char>,
    pos: usize,
    constants: &'a HashMap<String, f64>,
    functions: &'a HashSet<String>,
}

impl<'a> Lexer<'a> {
    /// `constants` / `functions` 由调用方合并
    /// （通常是 `FunctionTable` 的默认表 + schema 自定义常量）。
    pub fn new(
        input: &'a str,
        constants: &'a HashMap<String, f64>,
        functions: &'a HashSet<String>,
    ) -> Self {
        Self {
            input,
            chars: input.chars().collect(),
            pos: 0,
            constants,
            functions,
        }
    }

    /// 当前字符（越界为 `None`）
    fn current(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn advance(&mut self) {
        self.pos += 1;
    }

    /// 从 `start` 到当前 `pos` 的子串
    fn slice(&self, start: usize, end: usize) -> String {
        self.chars[start..end].iter().collect()
    }

    /// 词法分析。失败时返回**用户可见的错误文案**（含位置）。
    ///
    /// 对齐源项目：错误文案形如 `未知字符: 'x' at position 5`。
    pub fn tokenize(&mut self) -> Result<Vec<Token>, String> {
        let mut tokens: Vec<Token> = Vec::new();

        while let Some(c) = self.current() {
            if c.is_whitespace() {
                self.advance();
                continue;
            }
            if c.is_ascii_digit() || c == '.' {
                tokens.push(self.read_number()?);
                continue;
            }
            if c.is_alphabetic() || c == '_' {
                tokens.push(self.read_identifier());
                continue;
            }

            let pos = self.pos;
            let tok = match c {
                '+' => Token::new(TokenType::Plus, "+", pos),
                '-' => Token::new(TokenType::Minus, "-", pos),
                '*' => Token::new(TokenType::Multiply, "*", pos),
                '/' => Token::new(TokenType::Divide, "/", pos),
                '^' => Token::new(TokenType::Power, "^", pos),
                '%' => Token::new(TokenType::Modulo, "%", pos),
                '(' => Token::new(TokenType::LParen, "(", pos),
                ')' => Token::new(TokenType::RParen, ")", pos),
                ',' => Token::new(TokenType::Comma, ",", pos),
                other => return Err(format!("未知字符: '{other}' at position {pos}")),
            };
            tokens.push(tok);
            self.advance();
        }

        tokens.push(Token::new(TokenType::End, "", self.pos));
        Ok(process_unary_minus(tokens))
    }

    /// 读数字字面量（含科学计数法，带**回退**）。
    fn read_number(&mut self) -> Result<Token, String> {
        let start = self.pos;
        let mut has_dot = false;

        while let Some(c) = self.current() {
            if c.is_ascii_digit() {
                self.advance();
            } else if c == '.' && !has_dot {
                has_dot = true;
                self.advance();
            } else {
                break;
            }
        }

        // ---- 科学计数法（BUG-07）：`1e3` / `1E-3` / `1.5e+10` ----
        if matches!(self.current(), Some('e') | Some('E')) {
            let save_pos = self.pos;
            self.advance();

            if matches!(self.current(), Some('+') | Some('-')) {
                self.advance();
            }

            if self.current().is_some_and(|c| c.is_ascii_digit()) {
                while self.current().is_some_and(|c| c.is_ascii_digit()) {
                    self.advance();
                }
            } else {
                // 不是科学计数法（如变量 `e1`、常量 `e`）→ **回退游标到 e 之前**
                self.pos = save_pos;
            }
        }

        let text = self.slice(start, self.pos);
        let value: f64 = text
            .parse()
            .map_err(|_| format!("非法数字字面量: '{text}' at position {start}"))?;
        Ok(Token::with_number(TokenType::Number, text, start, value))
    }

    /// 读标识符 → 函数 / 常量 / 变量（**函数与常量名大小写不敏感**）。
    fn read_identifier(&mut self) -> Token {
        let start = self.pos;
        while self
            .current()
            .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '\'')
        {
            self.advance();
        }

        let text = self.slice(start, self.pos);
        let lower = text.to_lowercase();

        if self.functions.contains(&lower) {
            Token::new(TokenType::Function, lower, start)
        } else if let Some(v) = self.constants.get(&lower) {
            Token::with_number(TokenType::Constant, lower, start, *v)
        } else {
            // 变量保留**原始大小写**（`X1` 与 `x1` 是不同变量）
            Token::new(TokenType::Variable, text, start)
        }    }

    /// 原始输入（供错误文案使用）
    pub fn input(&self) -> &'a str {
        self.input
    }
}

/// 一元负号标定：按**前驱 token 类型**判定。
///
/// 判定规则（对齐源项目）：前驱为 `None`（开头）或
/// `+ - * / ^ % ( , u-` 时，`-` 是一元负号。
fn process_unary_minus(tokens: Vec<Token>) -> Vec<Token> {
    let mut result: Vec<Token> = Vec::with_capacity(tokens.len());
    let mut prev: Option<TokenType> = None;

    for token in tokens {
        if token.kind == TokenType::Minus {
            let is_unary = match prev {
                None => true,
                Some(t) => matches!(
                    t,
                    TokenType::Plus
                        | TokenType::Minus
                        | TokenType::Multiply
                        | TokenType::Divide
                        | TokenType::Power
                        | TokenType::Modulo
                        | TokenType::LParen
                        | TokenType::Comma
                        | TokenType::UnaryMinus
                ),
            };

            if is_unary {
                result.push(Token::new(
                    TokenType::UnaryMinus,
                    OP_UNARY_MINUS,
                    token.position,
                ));
            } else {
                result.push(token.clone());
            }
        } else {
            result.push(token.clone());
        }

        if token.kind != TokenType::End {
            prev = Some(token.kind);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::function_table;

    fn lex(input: &str) -> Vec<Token> {
        let constants = function_table::constants_owned();
        let functions = function_table::function_name_set();
        let mut lx = Lexer::new(input, &constants, &functions);
        lx.tokenize().expect("词法分析应成功")
    }

    fn lex_err(input: &str) -> String {
        let constants = function_table::constants_owned();
        let functions = function_table::function_name_set();
        let mut lx = Lexer::new(input, &constants, &functions);
        lx.tokenize().expect_err("词法分析应失败")
    }

    fn kinds(input: &str) -> Vec<TokenType> {
        lex(input).into_iter().map(|t| t.kind).collect()
    }

    fn numbers(input: &str) -> Vec<f64> {
        lex(input)
            .into_iter()
            .filter(|t| t.kind == TokenType::Number)
            .map(|t| t.number_value)
            .collect()
    }

    #[test]
    fn basic_arithmetic() {
        assert_eq!(
            kinds("1+2*3"),
            vec![
                TokenType::Number,
                TokenType::Plus,
                TokenType::Number,
                TokenType::Multiply,
                TokenType::Number,
                TokenType::End
            ]
        );
    }

    #[test]
    fn whitespace_is_ignored() {
        assert_eq!(kinds(" 1 + 2 "), kinds("1+2"));
    }

    // ---------------- BUG-07 科学计数法 ----------------

    #[test]
    fn bug07_scientific_notation_basic() {
        assert_eq!(numbers("1e3"), vec![1000.0]);
        assert_eq!(numbers("1e3*2"), vec![1000.0, 2.0]);
    }

    #[test]
    fn bug07_negative_exponent() {
        assert_eq!(numbers("2e-3"), vec![0.002]);
    }

    #[test]
    fn bug07_positive_sign_exponent() {
        assert_eq!(numbers("1.5e+10"), vec![1.5e10]);
    }

    #[test]
    fn bug07_euler_constant_is_constant_not_number() {
        // `e` 应识别为常量，而不是被当成数字的一部分
        let ts = lex("e");
        assert_eq!(ts[0].kind, TokenType::Constant);
        assert!((ts[0].number_value - std::f64::consts::E).abs() < f64::EPSILON);
    }

    #[test]
    fn bug07_variable_e1_after_operator() {
        // `2*e1`：`e1` 是变量，不能被科学计数法吞掉
        let ts = lex("2*e1");
        assert_eq!(
            ts.iter().map(|t| t.kind).collect::<Vec<_>>(),
            vec![
                TokenType::Number,
                TokenType::Multiply,
                TokenType::Variable,
                TokenType::End
            ]
        );
        assert_eq!(ts[2].value, "e1");
    }

    #[test]
    fn bug07_rollback_leaves_e_for_identifier() {
        // `e` 后面不是数字 → 回退，`e` 作为标识符重新读
        let ts = lex("e+1");
        assert_eq!(ts[0].kind, TokenType::Constant);
        assert_eq!(ts[1].kind, TokenType::Plus);
    }

    // ---------------- 一元负号 ----------------

    #[test]
    fn leading_minus_is_unary() {
        assert_eq!(kinds("-3")[0], TokenType::UnaryMinus);
    }

    #[test]
    fn binary_minus_stays_binary() {
        assert_eq!(kinds("1-2")[1], TokenType::Minus);
    }

    #[test]
    fn minus_after_operator_is_unary() {
        // 2*-3
        assert_eq!(kinds("2*-3")[2], TokenType::UnaryMinus);
        // 2^-3
        assert_eq!(kinds("2^-3")[2], TokenType::UnaryMinus);
        // 2%-3
        assert_eq!(kinds("2%-3")[2], TokenType::UnaryMinus);
    }

    #[test]
    fn minus_after_lparen_is_unary() {
        assert_eq!(kinds("(-3)")[1], TokenType::UnaryMinus);
    }

    #[test]
    fn minus_after_comma_is_unary() {
        assert_eq!(kinds("min(1,-3)")[4], TokenType::UnaryMinus);
    }

    #[test]
    fn double_minus_is_unary_then_unary() {
        // 2--3 → 数字、减号、一元负号、数字
        assert_eq!(
            kinds("2--3"),
            vec![
                TokenType::Number,
                TokenType::Minus,
                TokenType::UnaryMinus,
                TokenType::Number,
                TokenType::End
            ]
        );
    }

    // ---------------- 标识符 ----------------

    #[test]
    fn function_names_are_case_insensitive_and_lowercased() {
        let ts = lex("SQRT(4)");
        assert_eq!(ts[0].kind, TokenType::Function);
        assert_eq!(ts[0].value, "sqrt");
    }

    #[test]
    fn variable_keeps_original_case() {
        let ts = lex("X1+x1");
        assert_eq!(ts[0].kind, TokenType::Variable);
        assert_eq!(ts[0].value, "X1");
        assert_eq!(ts[2].value, "x1");
    }

    #[test]
    fn identifier_allows_underscore_and_quote() {
        let ts = lex("x_1 + alpha'");
        assert_eq!(ts[0].value, "x_1");
        assert_eq!(ts[2].value, "alpha'");
    }

    #[test]
    fn constant_names_are_case_insensitive() {
        let ts = lex("PI");
        assert_eq!(ts[0].kind, TokenType::Constant);
        assert!((ts[0].number_value - std::f64::consts::PI).abs() < f64::EPSILON);
    }

    // ---------------- 错误 ----------------

    #[test]
    fn unknown_char_reports_position() {
        assert_eq!(lex_err("1 @ 2"), "未知字符: '@' at position 2");
    }

    #[test]
    fn trailing_dot_is_accepted_by_number_parser() {
        // Kotlin `"1.".toDouble()` 合法；Rust 的 f64 解析同样接受
        assert_eq!(numbers("1."), vec![1.0]);
    }

    #[test]
    fn leading_dot_is_accepted() {
        assert_eq!(numbers(".5"), vec![0.5]);
    }

    #[test]
    fn end_token_is_appended() {
        assert_eq!(lex("1").last().unwrap().kind, TokenType::End);
    }
}
