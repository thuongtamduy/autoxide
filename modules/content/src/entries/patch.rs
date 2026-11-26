//! JSON Patch (RFC 6902) trên JSON Pointer (RFC 6901) để sửa từng phần thân `layout`
//! (`PATCH /entries/{id}/layout`, bước 67). Giao diện kéo thả và AI cùng gửi patch; patch áp lên
//! bản sao, một thao tác hỏng là cả patch hỏng (nguyên tử), rồi bố cục kết quả được kiểm lại toàn
//! bộ như khi thay cả thân.
//!
//! Tự viết (~150 dòng) thay vì thêm thư viện: sáu thao tác, không phụ thuộc gì ngoài `serde_json`.

use autoxide_kernel::error::FieldError;
use autoxide_spike_macros::Schema as ToSchema;
use serde::Deserialize;
use serde_json::Value;

/// Số thao tác tối đa của một patch.
pub(crate) const MAX_OPERATIONS: usize = 200;

/// One JSON Patch operation (RFC 6902).
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Operation {
    /// `add`, `remove`, `replace`, `move`, `copy` or `test`.
    #[schema(example = "replace")]
    pub op: String,
    /// JSON Pointer (RFC 6901) of the target, e.g. `/sections/0/props/text`; `-` appends to a list.
    #[schema(example = "/sections/0/props/text")]
    pub path: String,
    /// Source pointer of `move` and `copy`.
    #[serde(default)]
    pub from: Option<String>,
    /// Value of `add`, `replace` and `test`.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub value: Option<Value>,
}

/// Áp `operations` lên bản sao của `document`. Lỗi đầu tiên dừng cả patch; lỗi có đường dẫn
/// `operations[i]`.
pub(crate) fn apply(document: &Value, operations: &[Operation]) -> Result<Value, FieldError> {
    if operations.len() > MAX_OPERATIONS {
        return Err(FieldError::new("operations", "too_large", "has more than {max} operations")
            .with_param("max", MAX_OPERATIONS));
    }
    let mut document = document.clone();
    for (index, operation) in operations.iter().enumerate() {
        apply_one(&mut document, operation)
            .map_err(|failure| failure.error(format!("operations[{index}]"), &operation.path))?;
    }
    Ok(document)
}

/// Vì sao một thao tác hỏng.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failure {
    Missing,
    NotPosition,
    NotPointer,
    WholeDocument,
    IntoItself,
    NoValue,
    NoFrom,
    TestFailed,
    UnknownOp,
}

impl Failure {
    // Mỗi câu một lời gọi `FieldError::new` với chuỗi viết sẵn: `cargo xtask check` cần câu mẫu
    // để kiểm có bản dịch.
    fn error(self, at: String, path: &str) -> FieldError {
        let error = match self {
            Failure::Missing => {
                FieldError::new(at, "not_found", "`{path}` does not exist in the layout")
            }
            Failure::NotPosition => {
                FieldError::new(at, "not_found", "`{path}` is not a position in the list")
            }
            Failure::NotPointer => FieldError::new(at, "invalid", "`{path}` is not a JSON Pointer"),
            Failure::WholeDocument => {
                FieldError::new(at, "invalid", "`{path}` cannot replace the whole layout")
            }
            Failure::IntoItself => {
                FieldError::new(at, "invalid", "cannot move a value into itself at `{path}`")
            }
            Failure::NoValue => FieldError::new(at, "required", "`value` is required"),
            Failure::NoFrom => FieldError::new(at, "required", "`from` is required"),
            Failure::TestFailed => {
                FieldError::new(at, "test_failed", "`{path}` does not hold the tested value")
            }
            Failure::UnknownOp => FieldError::new(
                at,
                "invalid",
                "`op` must be add, remove, replace, move, copy or test",
            ),
        };
        error.with_param("path", path)
    }
}

fn apply_one(document: &mut Value, operation: &Operation) -> Result<(), Failure> {
    let value = || operation.value.clone().ok_or(Failure::NoValue);
    let from = || operation.from.as_deref().ok_or(Failure::NoFrom);
    match operation.op.as_str() {
        "add" => add(document, &operation.path, value()?),
        "remove" => remove(document, &operation.path).map(drop),
        "replace" => {
            let target = pointer_mut(document, &operation.path)?;
            *target = value()?;
            Ok(())
        }
        "move" => {
            let from = from()?;
            if operation.path.starts_with(&format!("{from}/")) {
                return Err(Failure::IntoItself);
            }
            let moved = remove(document, from)?;
            add(document, &operation.path, moved)
        }
        "copy" => {
            let copied =
                document.pointer(&escape_check(from()?)?).cloned().ok_or(Failure::Missing)?;
            add(document, &operation.path, copied)
        }
        "test" => {
            let current =
                document.pointer(&escape_check(&operation.path)?).ok_or(Failure::Missing)?;
            if *current == value()? { Ok(()) } else { Err(Failure::TestFailed) }
        }
        _ => Err(Failure::UnknownOp),
    }
}

/// Kiểm dạng con trỏ: rỗng (cả tài liệu) hay bắt đầu bằng `/`; `~` chỉ trong `~0` / `~1`.
fn escape_check(pointer: &str) -> Result<String, Failure> {
    let valid = pointer.is_empty()
        || (pointer.starts_with('/')
            && pointer
                .split('~')
                .skip(1)
                .all(|rest| rest.starts_with('0') || rest.starts_with('1')));
    if valid { Ok(pointer.to_owned()) } else { Err(Failure::NotPointer) }
}

fn unescape(token: &str) -> String {
    token.replace("~1", "/").replace("~0", "~")
}

/// Tách con trỏ thành (cha, khoá cuối).
fn split(pointer: &str) -> Result<(String, String), Failure> {
    escape_check(pointer)?;
    let (parent, last) = pointer.rsplit_once('/').ok_or(Failure::WholeDocument)?;
    Ok((parent.to_owned(), unescape(last)))
}

fn pointer_mut<'a>(document: &'a mut Value, pointer: &str) -> Result<&'a mut Value, Failure> {
    escape_check(pointer)?;
    if pointer.is_empty() {
        return Err(Failure::WholeDocument);
    }
    document.pointer_mut(pointer).ok_or(Failure::Missing)
}

fn index(list: &[Value], key: &str, inserting: bool) -> Result<usize, Failure> {
    let bound = if inserting { list.len() } else { list.len().saturating_sub(1) };
    if inserting && key == "-" {
        return Ok(list.len());
    }
    let valid = !key.is_empty()
        && key.bytes().all(|b| b.is_ascii_digit())
        && (key == "0" || !key.starts_with('0'));
    match key.parse::<usize>() {
        Ok(position) if valid && position <= bound && (inserting || !list.is_empty()) => {
            Ok(position)
        }
        _ => Err(Failure::NotPosition),
    }
}

fn add(document: &mut Value, pointer: &str, value: Value) -> Result<(), Failure> {
    let (parent, key) = split(pointer)?;
    match document.pointer_mut(&parent).ok_or(Failure::Missing)? {
        Value::Object(object) => {
            object.insert(key, value);
            Ok(())
        }
        Value::Array(list) => {
            let position = index(list, &key, true)?;
            list.insert(position, value);
            Ok(())
        }
        _ => Err(Failure::Missing),
    }
}

fn remove(document: &mut Value, pointer: &str) -> Result<Value, Failure> {
    let (parent, key) = split(pointer)?;
    match document.pointer_mut(&parent).ok_or(Failure::Missing)? {
        Value::Object(object) => object.remove(&key).ok_or(Failure::Missing),
        Value::Array(list) => {
            let position = index(list, &key, false)?;
            Ok(list.remove(position))
        }
        _ => Err(Failure::Missing),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn op(value: Value) -> Operation {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn sau_thao_tac_theo_rfc_6902() {
        let document = json!({"sections": [{"id": "a"}, {"id": "b"}], "m~n": {"x/y": 1}});
        let patched = apply(
            &document,
            &[
                op(json!({"op": "add", "path": "/sections/-", "value": {"id": "c"}})),
                op(json!({"op": "move", "from": "/sections/0", "path": "/sections/2"})),
                op(json!({"op": "replace", "path": "/m~0n/x~1y", "value": 2})),
                op(json!({"op": "copy", "from": "/sections/0", "path": "/sections/0"})),
                op(json!({"op": "remove", "path": "/sections/1"})),
                op(json!({"op": "test", "path": "/m~0n/x~1y", "value": 2})),
            ],
        )
        .unwrap();
        assert_eq!(
            patched,
            json!({"sections": [{"id": "b"}, {"id": "c"}, {"id": "a"}], "m~n": {"x/y": 2}})
        );
    }

    #[test]
    fn patch_hong_thi_khong_doi_gi() {
        let document = json!({"sections": [{"id": "a"}]});
        let failures = [
            (json!({"op": "remove", "path": "/sections/5"}), "not_found"),
            (json!({"op": "replace", "path": "/nope/x", "value": 1}), "not_found"),
            (json!({"op": "test", "path": "/sections/0/id", "value": "b"}), "test_failed"),
            (json!({"op": "add", "path": "sections", "value": 1}), "invalid"),
            (json!({"op": "add", "path": "/sections/01", "value": 1}), "not_found"),
            (json!({"op": "move", "from": "/sections", "path": "/sections/0"}), "invalid"),
            (json!({"op": "replace", "path": "/sections/0"}), "required"),
            (json!({"op": "drop", "path": "/sections"}), "invalid"),
            (json!({"op": "replace", "path": "", "value": {}}), "invalid"),
        ];
        for (operation, code) in failures {
            let error = apply(
                &document,
                &[op(json!({"op": "add", "path": "/x", "value": 1})), op(operation.clone())],
            )
            .unwrap_err();
            assert_eq!(
                (error.field.as_str(), error.code.as_str()),
                ("operations[1]", code),
                "{operation}"
            );
        }
    }
}
