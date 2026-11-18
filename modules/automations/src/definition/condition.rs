//! Điều kiện của luật: cây `all` / `any` / `not` trên trường của dữ liệu kích hoạt, toán tử cố định.
//! Không biểu thức tự do, không mã: kiểm được lúc lưu, chạy giống nhau mọi nơi, giải thích được.
//!
//! ```json
//! {"all": [
//!   {"field": "event.total", "op": "gt", "value": 5000000},
//!   {"not": {"field": "event.tags", "op": "contains", "value": "vip"}}
//! ]}
//! ```

use autoxide_kernel::error::FieldError;
use serde_json::Value;

/// Độ sâu lồng tối đa của cây điều kiện.
const MAX_DEPTH: usize = 6;
/// Số nút tối đa của cây (mọi `all` / `any` / `not` / so sánh).
pub(crate) const MAX_NODES: usize = 50;
/// Gốc được phép của đường dẫn trường: dữ liệu kích hoạt, thông tin kích hoạt, kết quả bước trước.
pub(crate) const ROOTS: [&str; 3] = ["event", "trigger", "steps"];

/// Toán tử so sánh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Op {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
    In,
    Contains,
    Exists,
}

impl Op {
    fn parse(text: &str) -> Option<Op> {
        Some(match text {
            "eq" => Op::Eq,
            "ne" => Op::Ne,
            "gt" => Op::Gt,
            "gte" => Op::Gte,
            "lt" => Op::Lt,
            "lte" => Op::Lte,
            "in" => Op::In,
            "contains" => Op::Contains,
            "exists" => Op::Exists,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Condition {
    All(Vec<Condition>),
    Any(Vec<Condition>),
    Not(Box<Condition>),
    Compare { field: String, op: Op, value: Value },
}

/// Đường dẫn trường hợp lệ: `<gốc>.<khoá>…`, khoá là chữ, số, `_`, `-`.
pub(crate) fn valid_path(path: &str) -> bool {
    let mut parts = path.split('.');
    let root_ok = parts.next().is_some_and(|root| ROOTS.contains(&root));
    root_ok
        && path.len() <= 200
        && path.split('.').all(|part| {
            !part.is_empty()
                && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
}

/// Giá trị ở đường dẫn `path` của `context`; khoá là số thì chọn phần tử của mảng.
pub(crate) fn lookup<'a>(context: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(context, |value, key| match value {
        Value::Object(map) => map.get(key),
        Value::Array(items) => key.parse::<usize>().ok().and_then(|index| items.get(index)),
        _ => None,
    })
}

impl Condition {
    /// Đọc cây điều kiện ở `field`; lỗi ghi vào `errors` (gom một lần, D-083). `null` / `{}` là
    /// "luôn đúng".
    pub(crate) fn parse(
        value: &Value,
        field: &str,
        errors: &mut Vec<FieldError>,
    ) -> Option<Condition> {
        // `null` và `{}`: không có điều kiện (luôn đúng).
        if value.is_null() || value.as_object().is_some_and(serde_json::Map::is_empty) {
            return None;
        }
        let mut nodes = 0;
        let condition = parse_node(value, field, 1, &mut nodes, errors);
        if nodes > MAX_NODES {
            errors.push(
                FieldError::new(field, "too_large", "must have at most {max} parts")
                    .with_param("max", MAX_NODES),
            );
        }
        condition
    }

    /// Điều kiện có đúng với dữ liệu `context` không.
    pub(crate) fn holds(&self, context: &Value) -> bool {
        match self {
            Condition::All(items) => items.iter().all(|item| item.holds(context)),
            Condition::Any(items) => items.iter().any(|item| item.holds(context)),
            Condition::Not(inner) => !inner.holds(context),
            Condition::Compare { field, op, value } => compare(lookup(context, field), *op, value),
        }
    }
}

fn parse_node(
    value: &Value,
    field: &str,
    depth: usize,
    nodes: &mut usize,
    errors: &mut Vec<FieldError>,
) -> Option<Condition> {
    *nodes += 1;
    if depth > MAX_DEPTH {
        errors.push(
            FieldError::new(field, "too_deep", "must nest at most {max} levels")
                .with_param("max", MAX_DEPTH),
        );
        return None;
    }
    let Some(object) = value.as_object() else {
        errors.push(invalid(field));
        return None;
    };
    let list = |key: &str, errors: &mut Vec<FieldError>, nodes: &mut usize| {
        let Some(items) =
            object.get(key).and_then(Value::as_array).filter(|items| !items.is_empty())
        else {
            errors.push(FieldError::new(
                format!("{field}.{key}"),
                "invalid",
                "must be a non-empty list of conditions",
            ));
            return None;
        };
        let parsed: Vec<Option<Condition>> = items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                parse_node(item, &format!("{field}.{key}.{index}"), depth + 1, nodes, errors)
            })
            .collect();
        parsed.into_iter().collect::<Option<Vec<_>>>()
    };
    if object.len() == 1 && object.contains_key("all") {
        return list("all", errors, nodes).map(Condition::All);
    }
    if object.len() == 1 && object.contains_key("any") {
        return list("any", errors, nodes).map(Condition::Any);
    }
    if object.len() == 1
        && let Some(inner) = object.get("not")
    {
        return parse_node(inner, &format!("{field}.not"), depth + 1, nodes, errors)
            .map(|inner| Condition::Not(Box::new(inner)));
    }
    let known = ["field", "op", "value"];
    if object.keys().any(|key| !known.contains(&key.as_str())) {
        errors.push(invalid(field));
        return None;
    }
    let path = object.get("field").and_then(Value::as_str).unwrap_or_default();
    let mut ok = true;
    if !valid_path(path) {
        ok = false;
        errors.push(
            FieldError::new(
                format!("{field}.field"),
                "invalid",
                "must be a path such as `event.order.total`, starting with one of {roots}",
            )
            .with_param("roots", ROOTS.join(", ")),
        );
    }
    let op = object.get("op").and_then(Value::as_str).and_then(Op::parse);
    let value = object.get("value").cloned().unwrap_or(Value::Null);
    match op {
        None => {
            ok = false;
            errors.push(FieldError::new(
                format!("{field}.op"),
                "invalid",
                "must be one of eq, ne, gt, gte, lt, lte, in, contains, exists",
            ));
        }
        Some(Op::Exists) if !value.is_null() => {
            ok = false;
            errors.push(FieldError::new(
                format!("{field}.value"),
                "invalid",
                "must be left out for `exists`",
            ));
        }
        Some(Op::In) if !value.is_array() => {
            ok = false;
            errors.push(FieldError::new(format!("{field}.value"), "invalid", "must be a list"));
        }
        Some(Op::Gt | Op::Gte | Op::Lt | Op::Lte) if !(value.is_number() || value.is_string()) => {
            ok = false;
            errors.push(FieldError::new(
                format!("{field}.value"),
                "invalid",
                "must be a number (or a text such as a date)",
            ));
        }
        Some(Op::Eq | Op::Ne | Op::Contains) if value.is_null() => {
            ok = false;
            errors.push(FieldError::new(format!("{field}.value"), "required", "is required"));
        }
        Some(_) => {}
    }
    match (ok, op) {
        (true, Some(op)) => Some(Condition::Compare { field: path.to_owned(), op, value }),
        _ => None,
    }
}

fn invalid(field: &str) -> FieldError {
    FieldError::new(
        field,
        "invalid",
        "must be a condition: an object with `all`, `any` or `not`, or with `field`, `op` and `value`",
    )
}

/// Số của một giá trị: số JSON, hay chữ viết một số (số tiền trong payload là chữ, `"1999"`).
fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// Hai giá trị bằng nhau: so số khi cả hai là số, không thì so JSON, rồi so dạng chữ (event không
/// khai schema thì chỉ so chuỗi được).
fn equal(left: &Value, right: &Value) -> bool {
    if let (Some(left), Some(right)) = (number(left), number(right)) {
        return left == right;
    }
    left == right || text(left) == text(right)
}

fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn order(left: &Value, right: &Value) -> Option<std::cmp::Ordering> {
    match (number(left), number(right)) {
        (Some(left), Some(right)) => left.partial_cmp(&right),
        _ => match (left, right) {
            (Value::String(left), Value::String(right)) => Some(left.cmp(right)),
            _ => None,
        },
    }
}

fn compare(actual: Option<&Value>, op: Op, expected: &Value) -> bool {
    use std::cmp::Ordering;
    let actual = actual.filter(|value| !value.is_null());
    match op {
        Op::Exists => actual.is_some(),
        // Trường vắng thì khác mọi giá trị.
        Op::Ne => actual.is_none_or(|actual| !equal(actual, expected)),
        _ => {
            let Some(actual) = actual else { return false };
            match op {
                Op::Eq => equal(actual, expected),
                Op::Gt => order(actual, expected) == Some(Ordering::Greater),
                Op::Gte => {
                    matches!(order(actual, expected), Some(Ordering::Greater | Ordering::Equal))
                }
                Op::Lt => order(actual, expected) == Some(Ordering::Less),
                Op::Lte => {
                    matches!(order(actual, expected), Some(Ordering::Less | Ordering::Equal))
                }
                Op::In => expected
                    .as_array()
                    .is_some_and(|items| items.iter().any(|item| equal(actual, item))),
                Op::Contains => match actual {
                    Value::Array(items) => items.iter().any(|item| equal(item, expected)),
                    Value::String(text) => text.contains(&self::text(expected)),
                    _ => false,
                },
                Op::Exists | Op::Ne => false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn parse(value: Value) -> (Option<Condition>, Vec<FieldError>) {
        let mut errors = Vec::new();
        let condition = Condition::parse(&value, "conditions", &mut errors);
        (condition, errors)
    }

    #[test]
    fn cay_dieu_kien_tinh_dung() {
        let (condition, errors) = parse(json!({"all": [
            {"field": "event.total", "op": "gt", "value": 5_000_000},
            {"not": {"field": "event.tags", "op": "contains", "value": "vip"}},
            {"any": [
                {"field": "event.channel", "op": "in", "value": ["web", "pos"]},
                {"field": "event.note", "op": "exists"}
            ]}
        ]}));
        assert!(errors.is_empty(), "{errors:?}");
        let condition = condition.unwrap();
        let context = |total: Value, tags: Value| json!({"event": {"total": total, "tags": tags, "channel": "web"}});
        // Số tiền dạng chữ vẫn so như số.
        assert!(condition.holds(&context(json!("6000000"), json!([]))));
        assert!(!condition.holds(&context(json!(4_000_000), json!([]))));
        assert!(!condition.holds(&context(json!(6_000_000), json!(["vip"]))));
    }

    #[test]
    fn truong_vang_chi_dung_voi_ne() {
        let context = json!({"event": {}});
        for (op, expected) in [("eq", false), ("ne", true), ("gt", false), ("contains", false)] {
            let (condition, _) = parse(json!({"field": "event.x", "op": op, "value": 1}));
            assert_eq!(condition.unwrap().holds(&context), expected, "{op}");
        }
        let (condition, _) = parse(json!({"field": "event.x", "op": "exists"}));
        assert!(!condition.unwrap().holds(&context));
    }

    #[test]
    fn loi_gom_mot_lan() {
        let (condition, errors) = parse(json!({"all": [
            {"field": "order.total", "op": "gt", "value": 1},
            {"field": "event.total", "op": "like", "value": 1},
            {"field": "event.x", "op": "in", "value": 1},
            {"script": "1 + 1"}
        ]}));
        assert!(condition.is_none());
        let fields: Vec<&str> = errors.iter().map(|error| error.field.as_str()).collect();
        assert_eq!(
            fields,
            vec![
                "conditions.all.0.field",
                "conditions.all.1.op",
                "conditions.all.2.value",
                "conditions.all.3"
            ]
        );
    }

    #[test]
    fn cay_qua_sau_bi_tu_choi() {
        let mut value = json!({"field": "event.x", "op": "exists"});
        for _ in 0..MAX_DEPTH {
            value = json!({"not": value});
        }
        let (_, errors) = parse(value);
        assert!(errors.iter().any(|error| error.code == "too_deep"));
    }

    #[test]
    fn duong_dan_chon_phan_tu_mang() {
        let context = json!({"steps": [{"result": {"id": "a"}}]});
        assert_eq!(lookup(&context, "steps.0.result.id"), Some(&json!("a")));
        assert_eq!(lookup(&context, "steps.1.result"), None);
    }
}
