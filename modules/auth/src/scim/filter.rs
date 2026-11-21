//! Bộ lọc SCIM (RFC 7644 §3.4.2.2), mức Autoxide hỗ trợ (D-20261121-scim): so sánh `eq`, `ne`,
//! `co`, `sw`, `ew`, `pr`; nối bằng `and` / `or` (`and` chặt hơn), ngoặc `( … )`. Không có `not`,
//! `gt` / `ge` / `lt` / `le` và bộ lọc giá trị lồng `attr[…]` ở tham số `filter` — gặp thì
//! `invalidFilter`. Bộ lọc giá trị trong đường dẫn PATCH (`emails[type eq "work"]`) dùng cùng bộ
//! phân tích, đánh giá trên JSON bằng [`Filter::matches`].
//!
//! Tên thuộc tính không phân biệt hoa thường (RFC 7643 §2.1): đưa về chữ thường, bỏ tiền tố URN
//! của lược đồ lõi.

use serde_json::Value;

use super::error::ScimError;

/// Toán tử so sánh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Op {
    Eq,
    Ne,
    Co,
    Sw,
    Ew,
}

/// Giá trị bên phải của phép so sánh.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Literal {
    Text(String),
    Bool(bool),
    Null,
    Number(String),
}

/// Cây bộ lọc đã phân tích.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Filter {
    And(Box<Filter>, Box<Filter>),
    Or(Box<Filter>, Box<Filter>),
    Compare { attribute: String, op: Op, value: Literal },
    Present { attribute: String },
}

/// Tiền tố URN của hai lược đồ lõi: `urn:…:core:2.0:User:userName` cũng là `username`.
const CORE_PREFIXES: [&str; 2] =
    ["urn:ietf:params:scim:schemas:core:2.0:user:", "urn:ietf:params:scim:schemas:core:2.0:group:"];

/// Tên thuộc tính chuẩn hoá: chữ thường, bỏ tiền tố URN lõi.
pub(crate) fn attribute_name(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    for prefix in CORE_PREFIXES {
        if let Some(rest) = lower.strip_prefix(prefix) {
            return rest.to_owned();
        }
    }
    lower
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Word(String),
    Text(String),
    Open,
    Close,
}

fn tokenize(input: &str) -> Result<Vec<Token>, ScimError> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(&character) = chars.peek() {
        match character {
            ' ' | '\t' | '\n' | '\r' => {
                chars.next();
            }
            '(' => {
                chars.next();
                tokens.push(Token::Open);
            }
            ')' => {
                chars.next();
                tokens.push(Token::Close);
            }
            '"' => {
                chars.next();
                let mut text = String::new();
                let mut closed = false;
                while let Some(next) = chars.next() {
                    match next {
                        '"' => {
                            closed = true;
                            break;
                        }
                        // Chuỗi JSON (RFC 7644 §3.4.2.2): `\"`, `\\` và các ký tự thoát khác.
                        '\\' => match chars.next() {
                            Some('n') => text.push('\n'),
                            Some('t') => text.push('\t'),
                            Some('r') => text.push('\r'),
                            Some(other) => text.push(other),
                            None => break,
                        },
                        other => text.push(other),
                    }
                }
                if !closed {
                    return Err(ScimError::invalid_filter("A quoted value is not closed."));
                }
                tokens.push(Token::Text(text));
            }
            '[' | ']' => {
                return Err(ScimError::invalid_filter(
                    "Value filters (`attribute[...]`) are not supported in `filter`; compare \
                     `attribute.subAttribute` instead.",
                ));
            }
            _ => {
                let mut word = String::new();
                while let Some(&next) = chars.peek() {
                    if matches!(next, ' ' | '\t' | '\n' | '\r' | '(' | ')' | '"' | '[' | ']') {
                        break;
                    }
                    word.push(next);
                    chars.next();
                }
                tokens.push(Token::Word(word));
            }
        }
    }
    Ok(tokens)
}

/// Phân tích `input` thành cây bộ lọc.
pub(crate) fn parse(input: &str) -> Result<Filter, ScimError> {
    if input.len() > 2048 {
        return Err(ScimError::invalid_filter("The filter is longer than 2048 characters."));
    }
    let tokens = tokenize(input)?;
    let mut parser = Parser { tokens, position: 0, depth: 0 };
    let filter = parser.or()?;
    if parser.position != parser.tokens.len() {
        return Err(ScimError::invalid_filter("Unexpected text after the end of the filter."));
    }
    Ok(filter)
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
    depth: usize,
}

impl Parser {
    fn peek_word(&self, word: &str) -> bool {
        matches!(self.tokens.get(self.position), Some(Token::Word(found)) if found.eq_ignore_ascii_case(word))
    }

    fn or(&mut self) -> Result<Filter, ScimError> {
        let mut left = self.and()?;
        while self.peek_word("or") {
            self.position += 1;
            let right = self.and()?;
            left = Filter::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Filter, ScimError> {
        let mut left = self.term()?;
        while self.peek_word("and") {
            self.position += 1;
            let right = self.term()?;
            left = Filter::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn term(&mut self) -> Result<Filter, ScimError> {
        match self.tokens.get(self.position).cloned() {
            Some(Token::Open) => {
                self.depth += 1;
                if self.depth > 16 {
                    return Err(ScimError::invalid_filter("The filter nests too deep."));
                }
                self.position += 1;
                let inner = self.or()?;
                if self.tokens.get(self.position) != Some(&Token::Close) {
                    return Err(ScimError::invalid_filter("A `(` is not closed."));
                }
                self.position += 1;
                self.depth -= 1;
                Ok(inner)
            }
            Some(Token::Word(word)) if word.eq_ignore_ascii_case("not") => {
                Err(ScimError::invalid_filter("`not` is not supported."))
            }
            Some(Token::Word(attribute)) => {
                self.position += 1;
                let attribute = attribute_name(&attribute);
                let Some(Token::Word(op)) = self.tokens.get(self.position).cloned() else {
                    return Err(ScimError::invalid_filter(format!(
                        "Expected an operator after `{attribute}`."
                    )));
                };
                self.position += 1;
                let op = match op.to_ascii_lowercase().as_str() {
                    "pr" => return Ok(Filter::Present { attribute }),
                    "eq" => Op::Eq,
                    "ne" => Op::Ne,
                    "co" => Op::Co,
                    "sw" => Op::Sw,
                    "ew" => Op::Ew,
                    "gt" | "ge" | "lt" | "le" => {
                        return Err(ScimError::invalid_filter(format!(
                            "Operator `{op}` is not supported; use eq, ne, co, sw, ew or pr."
                        )));
                    }
                    _ => {
                        return Err(ScimError::invalid_filter(format!("Unknown operator `{op}`.")));
                    }
                };
                let value = match self.tokens.get(self.position).cloned() {
                    Some(Token::Text(text)) => Literal::Text(text),
                    Some(Token::Word(word)) => match word.to_ascii_lowercase().as_str() {
                        "true" => Literal::Bool(true),
                        "false" => Literal::Bool(false),
                        "null" => Literal::Null,
                        _ if word.parse::<f64>().is_ok() => Literal::Number(word),
                        _ => {
                            return Err(ScimError::invalid_filter(format!(
                                "`{word}` is not a value; quote strings with `\"`."
                            )));
                        }
                    },
                    _ => {
                        return Err(ScimError::invalid_filter(format!(
                            "Expected a value after `{attribute}`."
                        )));
                    }
                };
                self.position += 1;
                Ok(Filter::Compare { attribute, op, value })
            }
            _ => Err(ScimError::invalid_filter("Expected an attribute or `(`.")),
        }
    }
}

impl Filter {
    /// Phần tử JSON `item` (một phần tử của thuộc tính nhiều giá trị, khoá chữ thường) có khớp
    /// không. Chuỗi so không phân biệt hoa thường (`type`, `value` của email đều caseExact=false).
    pub(crate) fn matches(&self, item: &Value) -> bool {
        match self {
            Filter::And(left, right) => left.matches(item) && right.matches(item),
            Filter::Or(left, right) => left.matches(item) || right.matches(item),
            Filter::Present { attribute } => {
                lookup(item, attribute).is_some_and(|value| !value.is_null() && value != "")
            }
            Filter::Compare { attribute, op, value } => {
                let found = lookup(item, attribute);
                match (found, value) {
                    (None | Some(Value::Null), Literal::Null) => *op == Op::Eq,
                    (Some(Value::Bool(found)), Literal::Bool(wanted)) => match op {
                        Op::Eq => found == wanted,
                        Op::Ne => found != wanted,
                        _ => false,
                    },
                    (Some(Value::String(found)), Literal::Text(wanted)) => {
                        compare_text(&found.to_lowercase(), *op, &wanted.to_lowercase())
                    }
                    (Some(Value::Number(found)), Literal::Number(wanted)) => match op {
                        Op::Eq => found.to_string() == *wanted,
                        Op::Ne => found.to_string() != *wanted,
                        _ => false,
                    },
                    (_, _) => *op == Op::Ne,
                }
            }
        }
    }
}

/// So hai chuỗi đã chuẩn hoá.
pub(crate) fn compare_text(found: &str, op: Op, wanted: &str) -> bool {
    match op {
        Op::Eq => found == wanted,
        Op::Ne => found != wanted,
        Op::Co => found.contains(wanted),
        Op::Sw => found.starts_with(wanted),
        Op::Ew => found.ends_with(wanted),
    }
}

/// Giá trị của `attribute` (có thể `a.b`) trong `item` có khoá chữ thường.
fn lookup<'a>(item: &'a Value, attribute: &str) -> Option<&'a Value> {
    attribute.split('.').try_fold(item, |current, part| current.get(part))
}

/// Mẫu `LIKE … ESCAPE '\'` cho `op` (`co`, `sw`, `ew`); `None` với `eq` / `ne`.
pub(crate) fn like_pattern(op: Op, text: &str) -> Option<String> {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if matches!(character, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    match op {
        Op::Co => Some(format!("%{escaped}%")),
        Op::Sw => Some(format!("{escaped}%")),
        Op::Ew => Some(format!("%{escaped}")),
        Op::Eq | Op::Ne => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn phan_tich_eq_va_and_or() {
        let filter = parse(r#"userName eq "Ada@Example.com""#).unwrap();
        assert_eq!(
            filter,
            Filter::Compare {
                attribute: "username".to_owned(),
                op: Op::Eq,
                value: Literal::Text("Ada@Example.com".to_owned())
            }
        );
        let filter = parse(r#"userName sw "a" and (externalId pr or active eq False)"#).unwrap();
        let Filter::And(_, right) = filter else { panic!("and") };
        assert!(matches!(*right, Filter::Or(_, _)));
        let filter =
            parse(r#"urn:ietf:params:scim:schemas:core:2.0:User:userName eq "a\"b""#).unwrap();
        assert_eq!(
            filter,
            Filter::Compare {
                attribute: "username".to_owned(),
                op: Op::Eq,
                value: Literal::Text("a\"b".to_owned())
            }
        );
    }

    #[test]
    fn bo_loc_sai_la_invalid_filter() {
        for bad in [
            "userName eq",
            "userName xx \"a\"",
            "userName gt \"a\"",
            "not (userName eq \"a\")",
            "emails[type eq \"work\"]",
            "userName eq \"a",
            "(userName eq \"a\"",
            "userName eq \"a\" extra",
            "userName eq a",
        ] {
            let error = parse(bad).unwrap_err();
            assert_eq!(error.body()["scimType"], "invalidFilter", "{bad}");
        }
    }

    #[test]
    fn danh_gia_tren_json() {
        let item = json!({"type": "work", "value": "ada@example.com", "primary": true});
        assert!(parse(r#"type eq "Work""#).unwrap().matches(&item));
        assert!(parse(r#"primary eq true"#).unwrap().matches(&item));
        assert!(parse(r#"value ew "@example.com" and type ne "home""#).unwrap().matches(&item));
        assert!(!parse(r#"type eq "home""#).unwrap().matches(&item));
        assert!(parse(r#"display pr or value pr"#).unwrap().matches(&item));
    }

    #[test]
    fn mau_like_thoat_ky_tu() {
        assert_eq!(like_pattern(Op::Co, "50%_a"), Some(r"%50\%\_a%".to_owned()));
        assert_eq!(like_pattern(Op::Sw, "a"), Some("a%".to_owned()));
        assert_eq!(like_pattern(Op::Eq, "a"), None);
    }
}
