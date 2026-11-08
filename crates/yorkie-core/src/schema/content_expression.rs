//! ProseMirror-compatible content expressions, mirroring JS
//! `src/document/schema/content-expression.ts`.
//!
//! A content expression says which children a tree node may have:
//!
//! ```text
//! expr       -> sequence ('|' sequence)*
//! sequence   -> element+
//! element    -> atom quantifier?
//! atom       -> name | '(' expr ')'
//! quantifier -> '+' | '*' | '?'
//! ```

use crate::{Result, YorkieError};
use std::collections::BTreeSet;

/// A parsed content expression (JS `ContentExpr`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ContentExpr {
    /// A node type or a group name.
    Node(String),
    Sequence(Vec<ContentExpr>),
    Alternative(Vec<ContentExpr>),
    /// `max` is `None` for no upper bound (JS `Infinity`).
    Repeat {
        expr: Box<ContentExpr>,
        min: usize,
        max: Option<usize>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Name(String),
    Plus,
    Star,
    Question,
    Pipe,
    LParen,
    RParen,
}

impl Token {
    fn value(&self) -> &str {
        match self {
            Self::Name(name) => name,
            Self::Plus => "+",
            Self::Star => "*",
            Self::Question => "?",
            Self::Pipe => "|",
            Self::LParen => "(",
            Self::RParen => ")",
        }
    }
}

fn invalid(message: String) -> YorkieError {
    YorkieError::InvalidArgument(message)
}

/// Splits a content expression into tokens (JS `tokenize`).
fn tokenize(expr: &str) -> Result<Vec<Token>> {
    let chars: Vec<char> = expr.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let token = match chars[i] {
            c if c.is_whitespace() => {
                i += 1;
                continue;
            }
            '+' => Token::Plus,
            '*' => Token::Star,
            '?' => Token::Question,
            '|' => Token::Pipe,
            '(' => Token::LParen,
            ')' => Token::RParen,
            _ => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                if start == i {
                    return Err(invalid(format!(
                        "Unexpected character '{}' at position {i} in content expression",
                        chars[i]
                    )));
                }
                tokens.push(Token::Name(chars[start..i].iter().collect()));
                continue;
            }
        };
        tokens.push(token);
        i += 1;
    }
    Ok(tokens)
}

/// Parses alternatives separated by `|` (JS `parseAlternatives`).
fn parse_alternatives(tokens: &[Token], pos: usize) -> Result<(ContentExpr, usize)> {
    let (first, mut pos) = parse_sequence(tokens, pos)?;
    let mut sequences = vec![first];
    while tokens.get(pos) == Some(&Token::Pipe) {
        let (sequence, next) = parse_sequence(tokens, pos + 1)?;
        sequences.push(sequence);
        pos = next;
    }
    if sequences.len() == 1 {
        return Ok((sequences.remove(0), pos));
    }
    Ok((ContentExpr::Alternative(sequences), pos))
}

/// Parses a sequence of elements (JS `parseSequence`).
fn parse_sequence(tokens: &[Token], mut pos: usize) -> Result<(ContentExpr, usize)> {
    let mut elements = Vec::new();
    while pos < tokens.len() && tokens[pos] != Token::Pipe && tokens[pos] != Token::RParen {
        let (element, next) = parse_element(tokens, pos)?;
        elements.push(element);
        pos = next;
    }
    if elements.len() == 1 {
        return Ok((elements.remove(0), pos));
    }
    Ok((ContentExpr::Sequence(elements), pos))
}

/// Parses an atom and its quantifier, if any (JS `parseElement`).
fn parse_element(tokens: &[Token], pos: usize) -> Result<(ContentExpr, usize)> {
    let (expr, pos) = parse_atom(tokens, pos)?;
    let (min, max) = match tokens.get(pos) {
        Some(Token::Plus) => (1, None),
        Some(Token::Star) => (0, None),
        Some(Token::Question) => (0, Some(1)),
        _ => return Ok((expr, pos)),
    };
    let expr = ContentExpr::Repeat {
        expr: Box::new(expr),
        min,
        max,
    };
    Ok((expr, pos + 1))
}

/// Parses a name or a parenthesized expression (JS `parseAtom`).
fn parse_atom(tokens: &[Token], pos: usize) -> Result<(ContentExpr, usize)> {
    let Some(token) = tokens.get(pos) else {
        return Err(invalid("Unexpected end of content expression".to_owned()));
    };
    match token {
        Token::LParen => {
            let (expr, pos) = parse_alternatives(tokens, pos + 1)?;
            if tokens.get(pos) != Some(&Token::RParen) {
                return Err(invalid(
                    "Unmatched parenthesis in content expression".to_owned(),
                ));
            }
            Ok((expr, pos + 1))
        }
        Token::Name(name) => Ok((ContentExpr::Node(name.clone()), pos + 1)),
        token => Err(invalid(format!(
            "Expected node type name but got '{}' in content expression",
            token.value()
        ))),
    }
}

/// Parses a content expression (JS `parseContentExpression`). An empty
/// expression allows no children.
pub(crate) fn parse_content_expression(expr: &str) -> Result<ContentExpr> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return Ok(ContentExpr::Sequence(Vec::new()));
    }
    let tokens = tokenize(trimmed)?;
    let (expr, pos) = parse_alternatives(&tokens, 0)?;
    if let Some(token) = tokens.get(pos) {
        return Err(invalid(format!(
            "Unexpected token '{}' at position {pos} in content expression",
            token.value()
        )));
    }
    Ok(expr)
}

/// Returns every position reachable by matching `expr` from any of
/// `positions` (JS `matchExpr`). Keeping all of them is what lets an
/// ambiguous expression such as `a* a` backtrack.
fn match_expr(
    expr: &ContentExpr,
    types: &[&str],
    positions: &BTreeSet<usize>,
    resolver: &dyn Fn(&str) -> Vec<String>,
) -> BTreeSet<usize> {
    match expr {
        ContentExpr::Node(name) => {
            let allowed = resolver(name);
            positions
                .iter()
                .filter(|&&pos| {
                    types
                        .get(pos)
                        .is_some_and(|t| allowed.iter().any(|a| a == t))
                })
                .map(|pos| pos + 1)
                .collect()
        }
        ContentExpr::Sequence(children) => {
            let mut current = positions.clone();
            for child in children {
                current = match_expr(child, types, &current, resolver);
                if current.is_empty() {
                    break;
                }
            }
            current
        }
        ContentExpr::Alternative(children) => children
            .iter()
            .flat_map(|child| match_expr(child, types, positions, resolver))
            .collect(),
        ContentExpr::Repeat { expr, min, max } => {
            let mut current = positions.clone();
            let mut reachable = BTreeSet::new();
            if *min == 0 {
                reachable.extend(current.iter().copied());
            }
            let mut count = 1;
            while max.is_none_or(|max| count <= max) {
                let next: BTreeSet<usize> = match_expr(expr, types, &current, resolver)
                    .into_iter()
                    .filter(|pos| !reachable.contains(pos) || count < *min)
                    .collect();
                if next.is_empty() {
                    break;
                }
                current = next;
                if count >= *min {
                    reachable.extend(current.iter().copied());
                }
                count += 1;
            }
            reachable
        }
    }
}

/// Matches child node types against a content expression and returns why
/// they do not match, if they do not (JS `matchContentExpression`;
/// `None` is `valid: true`). `resolver` turns a group name into its node
/// types and any other name into itself.
pub(crate) fn match_content_expression(
    expr: &ContentExpr,
    child_types: &[&str],
    resolver: &dyn Fn(&str) -> Vec<String>,
) -> Option<String> {
    let positions = match_expr(expr, child_types, &BTreeSet::from([0]), resolver);
    if positions.contains(&child_types.len()) {
        return None;
    }
    match positions.last() {
        None => Some("Children do not match content expression".to_owned()),
        Some(last) => Some(format!("Unexpected child at position {last}")),
    }
}

/// JS reference: `test/unit/document/schema/content_expression_test.ts`.
#[cfg(test)]
mod tests {
    use super::{match_content_expression, parse_content_expression, ContentExpr};

    fn identity(name: &str) -> Vec<String> {
        vec![name.to_owned()]
    }

    fn matches(expr: &str, types: &[&str]) -> bool {
        let expr = parse_content_expression(expr).unwrap();
        match_content_expression(&expr, types, &identity).is_none()
    }

    #[test]
    fn matches_simple_type() {
        assert!(matches("paragraph", &["paragraph"]));
        assert!(!matches("paragraph", &["heading"]));
        assert!(!matches("paragraph", &[]));
    }

    #[test]
    fn matches_one_or_more() {
        assert!(matches("paragraph+", &["paragraph"]));
        assert!(matches("paragraph+", &["paragraph", "paragraph"]));
        assert!(!matches("paragraph+", &[]));
    }

    #[test]
    fn matches_zero_or_more() {
        assert!(matches("text*", &[]));
        assert!(matches("text*", &["text", "text"]));
        assert!(!matches("text*", &["paragraph"]));
    }

    #[test]
    fn matches_zero_or_one() {
        assert!(matches("title?", &[]));
        assert!(matches("title?", &["title"]));
        assert!(!matches("title?", &["title", "title"]));
    }

    #[test]
    fn matches_sequence() {
        assert!(matches("heading paragraph+", &["heading", "paragraph"]));
        assert!(matches(
            "heading paragraph+",
            &["heading", "paragraph", "paragraph"]
        ));
        assert!(!matches("heading paragraph+", &["paragraph"]));
    }

    #[test]
    fn matches_alternatives() {
        assert!(matches("paragraph | heading", &["paragraph"]));
        assert!(matches("paragraph | heading", &["heading"]));
        assert!(!matches("paragraph | heading", &["blockquote"]));
    }

    #[test]
    fn matches_grouped_alternatives_with_quantifier() {
        assert!(matches(
            "(paragraph | heading)+",
            &["paragraph", "heading", "paragraph"]
        ));
        assert!(!matches("(paragraph | heading)+", &[]));
    }

    #[test]
    fn resolves_groups() {
        let resolver = |name: &str| {
            if name == "block" {
                vec!["paragraph".into(), "heading".into(), "blockquote".into()]
            } else {
                vec![name.to_owned()]
            }
        };
        let expr = parse_content_expression("block+").unwrap();
        assert!(match_content_expression(&expr, &["paragraph", "heading"], &resolver).is_none());
        assert!(match_content_expression(&expr, &["inline"], &resolver).is_some());
    }

    #[test]
    fn backtracks_over_ambiguous_repeats() {
        assert!(matches("a* a", &["a", "a"]));
        assert!(!matches("a* a", &[]));
    }

    #[test]
    fn empty_expression_allows_no_children() {
        assert_eq!(
            ContentExpr::Sequence(Vec::new()),
            parse_content_expression("  ").unwrap()
        );
        assert!(matches("", &[]));
        assert!(!matches("", &["text"]));
    }

    #[test]
    fn reports_where_children_stop_matching() {
        let expr = parse_content_expression("heading paragraph?").unwrap();
        assert_eq!(
            Some("Unexpected child at position 1".to_owned()),
            match_content_expression(&expr, &["heading", "heading"], &identity)
        );
        assert_eq!(
            Some("Children do not match content expression".to_owned()),
            match_content_expression(&expr, &["paragraph"], &identity)
        );
    }

    #[test]
    fn rejects_malformed_expressions() {
        for (expr, message) in [
            (
                "a-b",
                "Unexpected character '-' at position 1 in content expression",
            ),
            ("(a | b", "Unmatched parenthesis in content expression"),
            (
                "a )",
                "Unexpected token ')' at position 1 in content expression",
            ),
            (
                "+",
                "Expected node type name but got '+' in content expression",
            ),
        ] {
            let err = parse_content_expression(expr).unwrap_err();
            assert_eq!(format!("invalid argument: {message}"), err.to_string());
        }
    }
}
