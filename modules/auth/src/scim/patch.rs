//! PATCH theo RFC 7644 §3.5.2: áp các lệnh `add` / `replace` / `remove` lên bản JSON của tài
//! nguyên (khoá chữ thường), rồi bên gọi đọc bản kết quả như thân của `PUT`. Một cách cho mọi
//! thuộc tính — không viết riêng từng đường dẫn.
//!
//! Đường dẫn hỗ trợ: `attr`, `attr.sub`, `attr[bộ lọc]`, `attr[bộ lọc].sub` (tiền tố URN lõi bỏ
//! được). Tương thích với cách Entra ID / Okta gọi:
//!
//! - `op` không phân biệt hoa thường (`Replace`, `Add`);
//! - không có `path` thì `value` là object, mỗi khoá một lệnh (`{"active": false}`), kể cả khoá
//!   `a.b` (`{"name.givenName": "Ada"}`);
//! - `remove` đường dẫn nhiều giá trị kèm `value` là mảng: chỉ bỏ các phần tử có cùng `value`
//!   (`{"op": "Remove", "path": "members", "value": [{"value": "…"}]}`);
//! - đặt `attr[type eq "work"].value` khi chưa có phần tử khớp thì thêm phần tử mới.
//!
//! Thuộc tính của lược đồ mở rộng (`urn:…:extension:…`) mà Autoxide không lưu: bỏ qua.

use serde_json::{Map, Value};

use super::error::{ScimError, ScimType};
use super::filter::{self, Filter, Literal, Op};

/// Lược đồ của thân PATCH.
pub(crate) const PATCH_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:PatchOp";
/// Nhiều lệnh nhất trong một PATCH.
const MAX_OPERATIONS: usize = 1000;

/// Đổi mọi khoá object (mọi cấp) sang chữ thường: tên thuộc tính SCIM không phân biệt hoa
/// thường (RFC 7643 §2.1).
pub(crate) fn lowercase_keys(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (filter::attribute_name(&key), lowercase_keys(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(lowercase_keys).collect()),
        other => other,
    }
}

/// Áp thân PATCH `body` (JSON thô) lên `resource` (khoá chữ thường). `immutable`: thuộc tính
/// không được đổi (`id`) — lệnh chạm tới là `mutability`.
pub(crate) fn apply(
    resource: &mut Value,
    body: &Value,
    immutable: &[&str],
) -> Result<(), ScimError> {
    let body = lowercase_keys(body.clone());
    let schemas_ok = body
        .get("schemas")
        .and_then(Value::as_array)
        .is_some_and(|schemas| schemas.iter().any(|schema| schema.as_str() == Some(PATCH_SCHEMA)));
    if !schemas_ok {
        return Err(ScimError::bad(
            ScimType::InvalidSyntax,
            format!("`schemas` must contain `{PATCH_SCHEMA}`."),
        ));
    }
    let Some(operations) = body.get("operations").and_then(Value::as_array) else {
        return Err(ScimError::bad(ScimType::InvalidSyntax, "`Operations` must be an array."));
    };
    if operations.len() > MAX_OPERATIONS {
        return Err(ScimError::bad(
            ScimType::InvalidSyntax,
            format!("At most {MAX_OPERATIONS} operations per request."),
        ));
    }
    for (index, operation) in operations.iter().enumerate() {
        let kind =
            operation.get("op").and_then(Value::as_str).map(str::to_ascii_lowercase).ok_or_else(
                || {
                    ScimError::bad(
                        ScimType::InvalidSyntax,
                        format!("Operations[{index}].op is missing."),
                    )
                },
            )?;
        let path =
            operation.get("path").and_then(Value::as_str).filter(|path| !path.trim().is_empty());
        let value = operation.get("value").cloned();
        match (kind.as_str(), path) {
            ("add" | "replace", None) => {
                let Some(Value::Object(map)) = value else {
                    return Err(ScimError::bad(
                        ScimType::InvalidValue,
                        format!("Operations[{index}]: without `path`, `value` must be an object."),
                    ));
                };
                for (key, value) in map {
                    let path = Path::parse(&key)?;
                    apply_one(resource, &kind, &path, Some(value), immutable)?;
                }
            }
            ("remove", None) => {
                return Err(ScimError::bad(
                    ScimType::NoTarget,
                    format!("Operations[{index}]: `remove` needs a `path`."),
                ));
            }
            ("add" | "replace" | "remove", Some(path)) => {
                let path = Path::parse(path)?;
                if kind != "remove" && value.is_none() {
                    return Err(ScimError::invalid_value(format!(
                        "Operations[{index}]: `{kind}` needs a `value`."
                    )));
                }
                apply_one(resource, &kind, &path, value, immutable)?;
            }
            _ => {
                return Err(ScimError::bad(
                    ScimType::InvalidSyntax,
                    format!("Operations[{index}].op must be `add`, `replace` or `remove`."),
                ));
            }
        }
    }
    Ok(())
}

/// Đường dẫn đã phân tích: thuộc tính, bộ lọc giá trị, thuộc tính con.
#[derive(Debug)]
struct Path {
    attribute: String,
    filter: Option<Filter>,
    sub: Option<String>,
    /// Thuộc tính của lược đồ mở rộng: bỏ qua.
    extension: bool,
}

impl Path {
    fn parse(raw: &str) -> Result<Path, ScimError> {
        let raw = raw.trim();
        let lower = raw.to_ascii_lowercase();
        if lower.starts_with("urn:") && filter::attribute_name(raw) == lower {
            // Không phải lược đồ lõi (vd `urn:…:extension:enterprise:2.0:User:department`).
            return Ok(Path { attribute: lower, filter: None, sub: None, extension: true });
        }
        let name = filter::attribute_name(raw);
        let (head, filter, rest) = match name.find('[') {
            Some(open) => {
                let close = name.rfind(']').filter(|close| *close > open).ok_or_else(|| {
                    ScimError::bad(ScimType::InvalidPath, format!("`{raw}`: `[` is not closed."))
                })?;
                // Bộ lọc giữ nguyên hoa thường của giá trị: cắt từ chuỗi gốc theo cùng vị trí
                // (tên thuộc tính chỉ ASCII; tiền tố URN bỏ đi có cùng độ dài ở cả hai).
                let offset = raw.len() - name.len();
                let inner = raw.get(offset + open + 1..offset + close).unwrap_or_default();
                let rest = name.get(close + 1..).unwrap_or_default();
                (name[..open].to_owned(), Some(filter::parse(inner)?), rest.to_owned())
            }
            None => match name.split_once('.') {
                Some((head, sub)) => (head.to_owned(), None, format!(".{sub}")),
                None => (name.clone(), None, String::new()),
            },
        };
        let sub = match rest.as_str() {
            "" => None,
            rest => match rest.strip_prefix('.') {
                Some(sub) if !sub.is_empty() && !sub.contains('.') => Some(sub.to_owned()),
                _ => {
                    return Err(ScimError::bad(
                        ScimType::InvalidPath,
                        format!("`{raw}` is not a supported path."),
                    ));
                }
            },
        };
        if head.is_empty() {
            return Err(ScimError::bad(
                ScimType::InvalidPath,
                format!("`{raw}` has no attribute."),
            ));
        }
        Ok(Path { attribute: head, filter, sub, extension: false })
    }
}

fn apply_one(
    resource: &mut Value,
    kind: &str,
    path: &Path,
    value: Option<Value>,
    immutable: &[&str],
) -> Result<(), ScimError> {
    if path.extension || path.attribute == "schemas" || path.attribute == "meta" {
        return Ok(());
    }
    if immutable.contains(&path.attribute.as_str()) {
        return Err(ScimError::bad(
            ScimType::Mutability,
            format!("`{}` cannot be changed.", path.attribute),
        ));
    }
    let value = value.map(lowercase_keys);
    let Some(object) = resource.as_object_mut() else {
        return Err(ScimError::invalid_value("The resource is not an object."));
    };
    match &path.filter {
        None => apply_plain(object, kind, &path.attribute, path.sub.as_deref(), value),
        Some(filter) => {
            apply_filtered(object, kind, &path.attribute, filter, path.sub.as_deref(), value)
        }
    }
}

/// `attr` hay `attr.sub`, không bộ lọc.
fn apply_plain(
    object: &mut Map<String, Value>,
    kind: &str,
    attribute: &str,
    sub: Option<&str>,
    value: Option<Value>,
) -> Result<(), ScimError> {
    if let Some(sub) = sub {
        let parent =
            object.entry(attribute.to_owned()).or_insert_with(|| Value::Object(Map::new()));
        if parent.is_null() {
            *parent = Value::Object(Map::new());
        }
        // `emails.value` trên thuộc tính nhiều giá trị: áp cho mọi phần tử (RFC 7644 §3.5.2).
        if let Value::Array(items) = parent {
            for item in items.iter_mut().filter_map(Value::as_object_mut) {
                set_or_remove(item, kind, sub, value.clone());
            }
            return Ok(());
        }
        let Some(parent) = parent.as_object_mut() else {
            return Err(ScimError::bad(
                ScimType::InvalidPath,
                format!("`{attribute}` has no sub-attributes."),
            ));
        };
        set_or_remove(parent, kind, sub, value);
        return Ok(());
    }
    match (kind, object.get_mut(attribute), value) {
        // Thêm vào thuộc tính nhiều giá trị: nối, bỏ phần tử đã có.
        ("add", Some(Value::Array(items)), Some(Value::Array(new))) => {
            for item in new {
                if !items.iter().any(|existing| same_item(existing, &item)) {
                    items.push(item);
                }
            }
        }
        ("add", Some(Value::Array(items)), Some(item)) => {
            if !items.iter().any(|existing| same_item(existing, &item)) {
                items.push(item);
            }
        }
        // Thêm / thay object con: gộp từng khoá (RFC 7644 §3.5.2.1 với thuộc tính phức hợp).
        ("add" | "replace", Some(Value::Object(current)), Some(Value::Object(new))) => {
            for (key, value) in new {
                current.insert(key, value);
            }
        }
        // Bỏ các phần tử có cùng `value` (Entra ID gửi kiểu này cho `members`).
        ("remove", Some(Value::Array(items)), Some(Value::Array(gone))) => {
            items.retain(|item| !gone.iter().any(|gone| same_item(item, gone)));
        }
        ("remove", _, _) => {
            object.remove(attribute);
        }
        (_, _, Some(value)) => {
            object.insert(attribute.to_owned(), value);
        }
        (_, _, None) => {}
    }
    Ok(())
}

/// `attr[bộ lọc]` hay `attr[bộ lọc].sub`.
fn apply_filtered(
    object: &mut Map<String, Value>,
    kind: &str,
    attribute: &str,
    filter: &Filter,
    sub: Option<&str>,
    value: Option<Value>,
) -> Result<(), ScimError> {
    let items = object.entry(attribute.to_owned()).or_insert_with(|| Value::Array(Vec::new()));
    if items.is_null() {
        *items = Value::Array(Vec::new());
    }
    let Value::Array(items) = items else {
        return Err(ScimError::bad(
            ScimType::InvalidPath,
            format!("`{attribute}` is not multi-valued; a value filter needs a list."),
        ));
    };
    let matched: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| filter.matches(item))
        .map(|(index, _)| index)
        .collect();
    match (kind, sub) {
        ("remove", None) => {
            let mut index = 0;
            items.retain(|_| {
                let keep = !matched.contains(&index);
                index += 1;
                keep
            });
        }
        (_, Some(sub)) => {
            if matched.is_empty() {
                if kind == "remove" {
                    return Ok(());
                }
                // Chưa có phần tử khớp: thêm phần tử dựng từ các điều kiện `eq` của bộ lọc.
                let mut item = Map::new();
                seed(filter, &mut item);
                if let Some(value) = value {
                    item.insert(sub.to_owned(), value);
                }
                items.push(Value::Object(item));
                return Ok(());
            }
            for index in matched {
                if let Some(item) = items.get_mut(index).and_then(Value::as_object_mut) {
                    set_or_remove(item, kind, sub, value.clone());
                }
            }
        }
        (_, None) => {
            if matched.is_empty() {
                if kind == "add" {
                    if let Some(value) = value {
                        items.push(value);
                    }
                    return Ok(());
                }
                return Err(ScimError::bad(
                    ScimType::NoTarget,
                    format!("No `{attribute}` matches the filter."),
                ));
            }
            for index in matched {
                if let (Some(slot), Some(value)) = (items.get_mut(index), value.clone()) {
                    *slot = value;
                }
            }
        }
    }
    Ok(())
}

fn set_or_remove(item: &mut Map<String, Value>, kind: &str, key: &str, value: Option<Value>) {
    match (kind, value) {
        ("remove", _) => {
            item.remove(key);
        }
        (_, Some(value)) => {
            item.insert(key.to_owned(), value);
        }
        (_, None) => {}
    }
}

/// Điều kiện `eq` (nối bằng `and`) của bộ lọc thành các trường của phần tử mới.
fn seed(filter: &Filter, item: &mut Map<String, Value>) {
    match filter {
        Filter::And(left, right) => {
            seed(left, item);
            seed(right, item);
        }
        Filter::Compare { attribute, op: Op::Eq, value } => {
            let value = match value {
                Literal::Text(text) => Value::String(text.clone()),
                Literal::Bool(flag) => Value::Bool(*flag),
                Literal::Number(_) | Literal::Null => return,
            };
            item.insert(attribute.clone(), value);
        }
        _ => {}
    }
}

/// Hai phần tử là "một": cùng `value` khi cả hai có (thành viên, email), không thì bằng nhau.
fn same_item(left: &Value, right: &Value) -> bool {
    match (left.get("value"), right.get("value")) {
        (Some(Value::String(left)), Some(Value::String(right))) => left.eq_ignore_ascii_case(right),
        (Some(left), Some(right)) => left == right,
        _ => left == right,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn patch(operations: Value) -> Value {
        json!({"schemas": [PATCH_SCHEMA], "Operations": operations})
    }

    fn user() -> Value {
        lowercase_keys(json!({
            "id": "1",
            "userName": "ada@example.com",
            "active": true,
            "name": {"givenName": "Ada", "familyName": "Lovelace"},
            "emails": [{"value": "ada@example.com", "type": "work", "primary": true}],
        }))
    }

    #[test]
    fn replace_active_kieu_entra_va_okta() {
        let mut resource = user();
        // Entra ID: `Replace`, path, giá trị chuỗi "False".
        let body = patch(json!([{"op": "Replace", "path": "active", "value": "False"}]));
        apply(&mut resource, &body, &["id"]).unwrap();
        assert_eq!(resource["active"], "False");
        // Okta: không path, value là object.
        let body = patch(json!([{"op": "replace", "value": {"active": true}}]));
        apply(&mut resource, &body, &["id"]).unwrap();
        assert_eq!(resource["active"], true);
    }

    #[test]
    fn duong_dan_co_bo_loc_va_thuoc_tinh_con() {
        let mut resource = user();
        let body = patch(json!([
            {"op": "replace", "path": "emails[type eq \"work\"].value", "value": "ADA@new.example"},
            {"op": "add", "path": "name.givenName", "value": "Augusta"},
            {"op": "add", "value": {"name.familyName": "King", "externalId": "e-1"}},
            {"op": "add", "path": "emails[type eq \"home\"].value", "value": "home@example.com"},
        ]));
        apply(&mut resource, &body, &["id"]).unwrap();
        assert_eq!(resource["emails"][0]["value"], "ADA@new.example");
        assert_eq!(resource["emails"][1], json!({"type": "home", "value": "home@example.com"}));
        assert_eq!(resource["name"]["givenname"], "Augusta");
        assert_eq!(resource["name"]["familyname"], "King");
        assert_eq!(resource["externalid"], "e-1");
        let body = patch(json!([{"op": "remove", "path": "emails[type eq \"home\"]"}]));
        apply(&mut resource, &body, &["id"]).unwrap();
        assert_eq!(resource["emails"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn them_va_bo_thanh_vien() {
        let mut group = lowercase_keys(json!({"displayName": "Sales", "members": []}));
        let body = patch(json!([
            {"op": "add", "path": "members", "value": [{"value": "a"}, {"value": "b"}]},
            {"op": "add", "path": "members", "value": [{"value": "a"}]},
        ]));
        apply(&mut group, &body, &["id"]).unwrap();
        assert_eq!(group["members"].as_array().unwrap().len(), 2);
        // Entra ID: remove kèm value.
        let body = patch(json!([{"op": "Remove", "path": "members", "value": [{"value": "a"}]}]));
        apply(&mut group, &body, &["id"]).unwrap();
        assert_eq!(group["members"], json!([{"value": "b"}]));
        // Chuẩn: remove theo bộ lọc.
        let body = patch(json!([{"op": "remove", "path": "members[value eq \"b\"]"}]));
        apply(&mut group, &body, &["id"]).unwrap();
        assert_eq!(group["members"], json!([]));
    }

    #[test]
    fn loi_theo_rfc() {
        let mut resource = user();
        let wrong_schema = json!({"schemas": ["x"], "Operations": []});
        assert_eq!(
            apply(&mut resource, &wrong_schema, &[]).unwrap_err().body()["scimType"],
            "invalidSyntax"
        );
        let body = patch(json!([{"op": "replace", "path": "id", "value": "2"}]));
        assert_eq!(
            apply(&mut resource, &body, &["id"]).unwrap_err().body()["scimType"],
            "mutability"
        );
        let body = patch(json!([{"op": "remove"}]));
        assert_eq!(
            apply(&mut resource, &body, &["id"]).unwrap_err().body()["scimType"],
            "noTarget"
        );
        let body = patch(json!([{"op": "move", "path": "active"}]));
        assert_eq!(
            apply(&mut resource, &body, &["id"]).unwrap_err().body()["scimType"],
            "invalidSyntax"
        );
        let body = patch(json!([{"op": "replace", "path": "emails[type eq \"x\"]", "value": {}}]));
        assert_eq!(
            apply(&mut resource, &body, &["id"]).unwrap_err().body()["scimType"],
            "noTarget"
        );
        // Lược đồ mở rộng: bỏ qua, không lỗi.
        let body = patch(
            json!([{"op": "add", "path": "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User:department", "value": "R&D"}]),
        );
        apply(&mut resource, &body, &["id"]).unwrap();
        assert!(resource.get("department").is_none());
    }
}
