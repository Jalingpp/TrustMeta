//! 布尔表达式解析器
//!
//! 支持大写 AND, OR, NOT 运算符和括号
//!
//! # 语法
//!
//! ```text
//! Expression := Term (OR Term)*
//! Term       := Factor (AND Factor)*
//! Factor     := NOT Factor | Keyword | '(' Expression ')'
//! Keyword    := 任意非布尔运算符短语
//! ```
//!
//! # 示例
//!
//! ```
//! use common::boolean_expr::{BooleanExpr, parse_boolean_expr};
//!
//! // 简单查询
//! let expr = parse_boolean_expr("rust").unwrap();
//!
//! // AND 查询
//! let expr = parse_boolean_expr("rust AND storage").unwrap();
//!
//! // OR 查询
//! let expr = parse_boolean_expr("rust OR python").unwrap();
//!
//! // 复杂查询
//! let expr = parse_boolean_expr("(rust OR python) AND (storage OR database)").unwrap();
//!
//! // NOT 查询
//! let expr = parse_boolean_expr("rust AND NOT python").unwrap();
//! ```

use std::collections::HashSet;

/// 布尔表达式抽象语法树
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BooleanExpr {
    /// 单个关键词
    Keyword(String),
    /// AND 运算: 交集
    And(Box<BooleanExpr>, Box<BooleanExpr>),
    /// OR 运算: 并集
    Or(Box<BooleanExpr>, Box<BooleanExpr>),
    /// NOT 运算: 补集
    Not(Box<BooleanExpr>),
}

impl BooleanExpr {
    /// 获取表达式中所有的关键词
    pub fn get_keywords(&self) -> HashSet<String> {
        match self {
            BooleanExpr::Keyword(kw) => {
                let mut set = HashSet::new();
                set.insert(kw.clone());
                set
            }
            BooleanExpr::And(left, right) | BooleanExpr::Or(left, right) => {
                let mut set = left.get_keywords();
                set.extend(right.get_keywords());
                set
            }
            BooleanExpr::Not(expr) => expr.get_keywords(),
        }
    }

    /// 对结果集合求值
    ///
    /// # 参数
    ///
    /// * `keyword_results` - 每个关键词对应的文件ID集合
    ///
    /// # 返回
    ///
    /// 布尔表达式求值后的文件ID集合
    pub fn evaluate(
        &self,
        keyword_results: &std::collections::HashMap<String, HashSet<String>>,
    ) -> HashSet<String> {
        match self {
            BooleanExpr::Keyword(kw) => keyword_results.get(kw).cloned().unwrap_or_default(),
            BooleanExpr::And(left, right) => {
                let left_result = left.evaluate(keyword_results);
                let right_result = right.evaluate(keyword_results);
                left_result.intersection(&right_result).cloned().collect()
            }
            BooleanExpr::Or(left, right) => {
                let left_result = left.evaluate(keyword_results);
                let right_result = right.evaluate(keyword_results);
                left_result.union(&right_result).cloned().collect()
            }
            BooleanExpr::Not(expr) => {
                // For NOT operation, return empty set
                // (NOT operation is tricky without a universal set)
                let _result = expr.evaluate(keyword_results);
                HashSet::new()
            }
        }
    }

    /// 转换为字符串表示
    pub fn to_string(&self) -> String {
        match self {
            BooleanExpr::Keyword(kw) => kw.clone(),
            BooleanExpr::And(left, right) => {
                format!("({} AND {})", left.to_string(), right.to_string())
            }
            BooleanExpr::Or(left, right) => {
                format!("({} OR {})", left.to_string(), right.to_string())
            }
            BooleanExpr::Not(expr) => {
                format!("(NOT {})", expr.to_string())
            }
        }
    }
}

/// 递归下降解析器
struct Parser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn peek_char(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn advance_char(&mut self) -> Option<char> {
        let ch = self.peek_char()?;
        self.pos += ch.len_utf8();
        Some(ch)
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek_char(), Some(ch) if ch.is_whitespace()) {
            self.advance_char();
        }
    }

    fn prev_char(&self) -> Option<char> {
        self.input[..self.pos].chars().next_back()
    }

    fn is_operator_boundary(ch: char) -> bool {
        ch.is_whitespace() || ch == '(' || ch == ')' || ch == ',' || ch == ';'
    }

    fn matches_operator_at(&self, op: &str) -> bool {
        if !self.input[self.pos..].starts_with(op) {
            return false;
        }

        let before_ok = self
            .prev_char()
            .map(Self::is_operator_boundary)
            .unwrap_or(true);
        let after_pos = self.pos + op.len();
        let after_ok = self
            .input
            .get(after_pos..)
            .and_then(|rest| rest.chars().next())
            .map(Self::is_operator_boundary)
            .unwrap_or(true);

        before_ok && after_ok
    }

    fn consume_operator(&mut self, op: &str) -> bool {
        if self.matches_operator_at(op) {
            self.pos += op.len();
            true
        } else {
            false
        }
    }

    fn find_matching_paren(&self) -> Option<usize> {
        let mut depth = 0usize;
        let mut index = self.pos;

        while index < self.input.len() {
            let ch = self.input[index..].chars().next()?;
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some(index);
                    }
                }
                _ => {}
            }
            index += ch.len_utf8();
        }

        None
    }

    fn contains_top_level_operator(&self, start: usize, end: usize) -> bool {
        let mut index = start;
        while index < end {
            let ch = self.input[index..].chars().next().unwrap();
            if ch.is_whitespace() || ch == '(' || ch == ')' || ch == ',' || ch == ';' {
                index += ch.len_utf8();
                continue;
            }

            for op in ["AND", "OR", "NOT"] {
                if self.input[index..end].starts_with(op) {
                    let before_ok = if index == start {
                        true
                    } else {
                        self.input[..index]
                            .chars()
                            .next_back()
                            .map(Self::is_operator_boundary)
                            .unwrap_or(true)
                    };
                    let after_pos = index + op.len();
                    let after_ok = if after_pos >= end {
                        true
                    } else {
                        self.input[after_pos..end]
                            .chars()
                            .next()
                            .map(Self::is_operator_boundary)
                            .unwrap_or(true)
                    };

                    if before_ok && after_ok {
                        return true;
                    }
                }
            }

            index += ch.len_utf8();
        }

        false
    }

    fn looks_like_group(&self) -> bool {
        if self.peek_char() != Some('(') {
            return false;
        }

        let Some(close_pos) = self.find_matching_paren() else {
            return false;
        };
        self.contains_top_level_operator(self.pos + 1, close_pos)
    }

    /// Expression := Term (OR Term)*
    fn parse_expression(&mut self, allow_right_paren: bool) -> Result<BooleanExpr, String> {
        let mut left = self.parse_term(allow_right_paren)?;

        loop {
            self.skip_whitespace();
            if allow_right_paren && self.peek_char() == Some(')') {
                break;
            }

            if self.consume_operator("OR") {
                let right = self.parse_term(allow_right_paren)?;
                left = BooleanExpr::Or(Box::new(left), Box::new(right));
            } else {
                break;
            }
        }

        Ok(left)
    }

    /// Term := Factor (AND Factor)*
    fn parse_term(&mut self, allow_right_paren: bool) -> Result<BooleanExpr, String> {
        let mut left = self.parse_factor(allow_right_paren)?;

        loop {
            self.skip_whitespace();
            if allow_right_paren && self.peek_char() == Some(')') {
                break;
            }

            if self.consume_operator("AND") {
                let right = self.parse_factor(allow_right_paren)?;
                left = BooleanExpr::And(Box::new(left), Box::new(right));
            } else {
                break;
            }
        }

        Ok(left)
    }

    fn parse_keyword_phrase(&mut self, allow_right_paren: bool) -> Result<BooleanExpr, String> {
        let start = self.pos;
        while let Some(ch) = self.peek_char() {
            if allow_right_paren && ch == ')' {
                break;
            }
            if self.matches_operator_at("AND")
                || self.matches_operator_at("OR")
                || self.matches_operator_at("NOT")
            {
                break;
            }
            self.advance_char();
        }

        let keyword = self.input[start..self.pos].trim();
        if keyword.is_empty() {
            Err("Expected keyword".to_string())
        } else {
            Ok(BooleanExpr::Keyword(keyword.to_string()))
        }
    }

    /// Factor := NOT Factor | Keyword | '(' Expression ')'
    fn parse_factor(&mut self, allow_right_paren: bool) -> Result<BooleanExpr, String> {
        self.skip_whitespace();

        if self.consume_operator("NOT") {
            let expr = self.parse_factor(allow_right_paren)?;
            return Ok(BooleanExpr::Not(Box::new(expr)));
        }

        if self.peek_char() == Some('(') && self.looks_like_group() {
            self.advance_char();
            let expr = self.parse_expression(true)?;
            self.skip_whitespace();
            if self.peek_char() != Some(')') {
                return Err("Expected ')'".to_string());
            }
            self.advance_char();
            return Ok(expr);
        }

        self.parse_keyword_phrase(allow_right_paren)
    }

    fn parse(&mut self) -> Result<BooleanExpr, String> {
        self.skip_whitespace();
        let expr = self.parse_expression(false)?;
        self.skip_whitespace();
        if !self.is_eof() {
            return Err(format!(
                "Unexpected token after expression: {:?}",
                self.peek_char()
            ));
        }
        Ok(expr)
    }
}

/// 解析布尔表达式
///
/// # 示例
///
/// ```
/// use common::boolean_expr::parse_boolean_expr;
///
/// let expr = parse_boolean_expr("rust AND storage").unwrap();
/// println!("{}", expr.to_string());
/// ```
pub fn parse_boolean_expr(input: &str) -> Result<BooleanExpr, String> {
    let mut parser = Parser::new(input);
    parser.parse()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_single_keyword() {
        let expr = parse_boolean_expr("rust").unwrap();
        assert_eq!(expr, BooleanExpr::Keyword("rust".to_string()));
    }

    #[test]
    fn test_parse_and() {
        let expr = parse_boolean_expr("rust AND storage").unwrap();
        assert!(matches!(expr, BooleanExpr::And(_, _)));
    }

    #[test]
    fn test_parse_or() {
        let expr = parse_boolean_expr("rust OR python").unwrap();
        assert!(matches!(expr, BooleanExpr::Or(_, _)));
    }

    #[test]
    fn test_parse_not() {
        let expr = parse_boolean_expr("NOT rust").unwrap();
        assert!(matches!(expr, BooleanExpr::Not(_)));
    }

    #[test]
    fn test_parse_complex() {
        let expr = parse_boolean_expr("(rust OR python) AND storage").unwrap();
        assert!(matches!(expr, BooleanExpr::And(_, _)));
    }

    #[test]
    fn test_parse_with_parens() {
        let expr = parse_boolean_expr("(rust AND storage) OR (python AND database)").unwrap();
        assert!(matches!(expr, BooleanExpr::Or(_, _)));
    }

    #[test]
    fn test_get_keywords() {
        let expr = parse_boolean_expr("(rust OR python) AND storage").unwrap();
        let keywords = expr.get_keywords();
        assert_eq!(keywords.len(), 3);
        assert!(keywords.contains("rust"));
        assert!(keywords.contains("python"));
        assert!(keywords.contains("storage"));
    }

    #[test]
    fn test_evaluate_and() {
        let expr = parse_boolean_expr("rust AND storage").unwrap();
        let mut results = std::collections::HashMap::new();

        let mut rust_files = HashSet::new();
        rust_files.insert("file1".to_string());
        rust_files.insert("file2".to_string());

        let mut storage_files = HashSet::new();
        storage_files.insert("file2".to_string());
        storage_files.insert("file3".to_string());

        results.insert("rust".to_string(), rust_files);
        results.insert("storage".to_string(), storage_files);

        let result = expr.evaluate(&results);
        assert_eq!(result.len(), 1);
        assert!(result.contains("file2"));
    }

    #[test]
    fn test_evaluate_or() {
        let expr = parse_boolean_expr("rust OR python").unwrap();
        let mut results = std::collections::HashMap::new();

        let mut rust_files = HashSet::new();
        rust_files.insert("file1".to_string());

        let mut python_files = HashSet::new();
        python_files.insert("file2".to_string());

        results.insert("rust".to_string(), rust_files);
        results.insert("python".to_string(), python_files);

        let result = expr.evaluate(&results);
        assert_eq!(result.len(), 2);
        assert!(result.contains("file1"));
        assert!(result.contains("file2"));
    }

    #[test]
    fn test_case_insensitive_operators() {
        let expr = parse_boolean_expr("rust and storage").unwrap();
        assert_eq!(expr, BooleanExpr::Keyword("rust and storage".to_string()));

        let expr = parse_boolean_expr("rust or python").unwrap();
        assert_eq!(expr, BooleanExpr::Keyword("rust or python".to_string()));

        let expr = parse_boolean_expr("not rust").unwrap();
        assert_eq!(expr, BooleanExpr::Keyword("not rust".to_string()));
    }

    #[test]
    fn test_parse_multiword_keyword() {
        let expr = parse_boolean_expr("Language Integrated Query(Linq)").unwrap();
        assert_eq!(
            expr,
            BooleanExpr::Keyword("Language Integrated Query(Linq)".to_string())
        );
    }

    #[test]
    fn test_parse_relaxed_query_line() {
        let expr =
            parse_boolean_expr("ADO.Net AND 存储过程 OR Language Integrated Query(Linq)").unwrap();
        assert!(matches!(expr, BooleanExpr::Or(_, _)));
        assert_eq!(expr.get_keywords().len(), 3);
    }
}
