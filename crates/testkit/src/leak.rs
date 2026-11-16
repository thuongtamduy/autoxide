//! Bài rò rỉ giữa khách (bước 33, đặc tả "Test bắt buộc"): sau khi gieo dữ liệu ở khách A, gọi
//! **mọi** endpoint `GET` của app và **mọi** tool MCP chỉ đọc bằng người của khách B (có đủ
//! permission). Vi phạm là một câu trả lời thành công có chứa dấu hiệu của khách A: chuỗi
//! `marker` (gieo trong tiêu đề, nội dung, email…) hay một ID của A.
//!
//! ```no_run
//! # async fn example(app: &autoxide_testkit::TestApp, note_id: String) {
//! let leaks = autoxide_testkit::leak_check(app, "token-b", "MARKER-A", &[("id", note_id)])
//!     .await
//!     .unwrap();
//! assert_eq!(leaks, Vec::<String>::new());
//! # }
//! ```
//!
//! Tham số `{tên}` trong đường dẫn được thay lần lượt bằng mọi ID của A khai cùng tên (không có
//! thì bằng mọi ID của A); tham số tool có tên trong `ids`, hay tên kết thúc `id`, cũng vậy.
//!
//! [`cursor_leak_check`]: con trỏ pagination lấy ở khách A gửi bằng người của khách B không bao giờ
//! cho thấy dữ liệu của A (mọi truy vấn đã lọc theo khách; con trỏ chỉ là vị trí).

use serde_json::{Map, Value};

use crate::app::{TestApp, TestAppError, tool};

/// Bài rò rỉ giữa hai nhánh anh em của cây tổ chức (bước 34, đặc tả "Test bắt buộc"): cùng một
/// khách, dữ liệu gieo ở nhánh A (chuỗi `marker`, các ID trong `ids`), gọi mọi `GET` và mọi tool
/// MCP chỉ đọc bằng `token` của người chỉ có quyền ở nhánh B (vd `.member(token, …, b,
/// &[("notes.read", Scope::Subtree)])`). Vi phạm là câu trả lời thành công có dấu hiệu của A.
///
/// ```no_run
/// # async fn example(app: &autoxide_testkit::TestApp, note_in_a: String) {
/// let leaks = autoxide_testkit::branch_leak_check(app, "south", "MARKER-A", &[("id", note_in_a)])
///     .await
///     .unwrap();
/// assert_eq!(leaks, Vec::<String>::new());
/// # }
/// ```
pub async fn branch_leak_check(
    app: &TestApp,
    token: &str,
    marker: &str,
    ids: &[(&str, String)],
) -> Result<Vec<String>, TestAppError> {
    // Cùng cách dò như giữa hai khách: chỉ khác người gọi (cùng khách, khác nhánh).
    leak_check(app, token, marker, ids).await
}

/// Gọi mọi `GET` và mọi tool MCP chỉ đọc bằng `token` (người của khách B); trả danh sách vi phạm
/// (rỗng là đạt). `ids`: ID đã gieo ở khách A, theo tên tham số (`("id", …)`, `("form_id", …)`).
pub async fn leak_check(
    app: &TestApp,
    token: &str,
    marker: &str,
    ids: &[(&str, String)],
) -> Result<Vec<String>, TestAppError> {
    let leaked =
        |body: &str| body.contains(marker) || ids.iter().any(|(_, id)| body.contains(id.as_str()));
    let mut violations = Vec::new();

    for (path, item) in &app.openapi().paths.paths {
        if item.get.is_none() {
            continue;
        }
        for uri in substitutions(path, ids) {
            // Không kiểm hợp đồng: ID lạ có thể cho câu trả lời mà bài khác đã kiểm.
            let answer = app.get(&uri).token(token).unchecked().send().await?;
            if answer.status().is_success() && leaked(&answer.text()) {
                violations.push(format!(
                    "GET {uri} → {}: shows data the caller must not see",
                    answer.status()
                ));
            }
        }
    }

    let client = app.mcp(token).await?;
    let tools = client
        .list_all_tools()
        .await
        .map_err(|e| TestAppError::new(format!("mcp: list tools: {e}")))?;
    for listed in tools {
        let read_only =
            listed.annotations.as_ref().and_then(|notes| notes.read_only_hint).unwrap_or(false);
        if !read_only {
            continue;
        }
        let properties: Vec<String> = listed
            .input_schema
            .get("properties")
            .and_then(Value::as_object)
            .map(|properties| properties.keys().cloned().collect())
            .unwrap_or_default();
        for arguments in tool_arguments(&properties, ids) {
            let call = client.call_tool(tool(&listed.name, Value::Object(arguments.clone()))).await;
            let Ok(result) = call else { continue };
            if result.is_error == Some(true) {
                continue;
            }
            let text = serde_json::to_string(&result).unwrap_or_default();
            if leaked(&text) {
                violations.push(format!(
                    "tool {} {}: shows data the caller must not see",
                    listed.name,
                    Value::Object(arguments)
                ));
            }
        }
    }
    let _ = client.cancel().await;
    Ok(violations)
}

/// Con trỏ pagination của khách A dùng ở khách B (bước 33, D-259 mục 5): với mọi `GET` trả trang
/// (`pagination.next_cursor`), lấy trang đầu `limit=1` bằng `owner` (người của A) — mỗi giá trị của
/// tham số `sort` nếu endpoint có, để thử cả con trỏ theo cột — rồi gửi con trỏ đó bằng `other`
/// (người của B). Vi phạm là câu trả lời thành công chứa dấu hiệu của A (`marker` hay một ID trong
/// `ids`). Trả (vi phạm, số con trỏ đã thử): bên gọi kiểm số đó > 0 để biết bài thật sự chạy (A
/// phải có ít nhất hai dòng trong danh sách để có con trỏ).
pub async fn cursor_leak_check(
    app: &TestApp,
    owner: &str,
    other: &str,
    marker: &str,
    ids: &[(&str, String)],
) -> Result<(Vec<String>, usize), TestAppError> {
    let leaked =
        |body: &str| body.contains(marker) || ids.iter().any(|(_, id)| body.contains(id.as_str()));
    let contract = serde_json::to_value(app.openapi())
        .map_err(|e| TestAppError::new(format!("openapi: {e}")))?;
    let mut violations = Vec::new();
    let mut tried = 0;
    let paths = contract["paths"].as_object().cloned().unwrap_or_default();
    for (path, item) in paths {
        let Some(get) = item.get("get") else { continue };
        // Phần query chung của hai lượt: không gì, hay `sort=<giá trị>`.
        let mut sorts = vec![String::new()];
        sorts.extend(sort_values(&contract, get).iter().map(|sort| format!("&sort={sort}")));
        for uri in substitutions(&path, ids) {
            for sort in &sorts {
                let first = app
                    .get(&format!("{uri}?limit=1{sort}"))
                    .token(owner)
                    .unchecked()
                    .send()
                    .await?;
                if !first.status().is_success() {
                    continue;
                }
                let Some(cursor) =
                    first.json()["pagination"]["next_cursor"].as_str().map(str::to_owned)
                else {
                    continue;
                };
                tried += 1;
                let next = format!("{uri}?limit=200{sort}&cursor={cursor}");
                let answer = app.get(&next).token(other).unchecked().send().await?;
                if answer.status().is_success() && leaked(&answer.text()) {
                    violations.push(format!(
                        "GET {next} → {}: a cursor of the other tenant shows its data",
                        answer.status()
                    ));
                }
            }
        }
    }
    Ok((violations, tried))
}

/// Giá trị của tham số query `sort` của thao tác `operation` (enum, kể cả qua `$ref`).
fn sort_values(contract: &Value, operation: &Value) -> Vec<String> {
    let Some(parameters) = operation["parameters"].as_array() else { return Vec::new() };
    let Some(sort) = parameters.iter().find(|parameter| parameter["name"] == "sort") else {
        return Vec::new();
    };
    let mut schema = &sort["schema"];
    // `oneOf: [null, {$ref}]` (tham số tuỳ chọn) hay `$ref` thẳng.
    if let Some(choices) = schema["oneOf"].as_array()
        && let Some(found) = choices.iter().find(|choice| choice.get("$ref").is_some())
    {
        schema = found;
    }
    if let Some(reference) = schema["$ref"].as_str() {
        let name = reference.rsplit('/').next().unwrap_or_default();
        schema = &contract["components"]["schemas"][name];
    }
    schema["enum"]
        .as_array()
        .map(|values| values.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default()
}

/// Mọi cách thay `{tham số}` của `path` bằng ID của khách A.
fn substitutions(path: &str, ids: &[(&str, String)]) -> Vec<String> {
    let mut uris = vec![String::new()];
    let mut rest = path;
    while let Some(start) = rest.find('{') {
        let Some(length) = rest[start..].find('}') else { break };
        let name = &rest[start + 1..start + length];
        let values = candidates(name, ids);
        let prefix = &rest[..start];
        uris = uris
            .into_iter()
            .flat_map(|uri| values.iter().map(move |value| format!("{uri}{prefix}{value}")))
            .collect();
        rest = &rest[start + length + 1..];
    }
    uris.into_iter().map(|uri| format!("{uri}{rest}")).collect()
}

/// ID khai cho tham số `name`; không có thì mọi ID (đường dẫn vẫn được thử).
fn candidates(name: &str, ids: &[(&str, String)]) -> Vec<String> {
    let named: Vec<String> =
        ids.iter().filter(|(key, _)| *key == name).map(|(_, id)| id.clone()).collect();
    if named.is_empty() { ids.iter().map(|(_, id)| id.clone()).collect() } else { named }
}

/// Bộ tham số thử cho một tool: mỗi tham số có tên trong `ids` hay kết thúc bằng `id` nhận lần
/// lượt các ID của A; tool không có tham số như vậy thì gọi một lần với `{}`.
fn tool_arguments(properties: &[String], ids: &[(&str, String)]) -> Vec<Map<String, Value>> {
    let mut sets = vec![Map::new()];
    for property in properties {
        let wanted = ids.iter().any(|(key, _)| key == property) || property.ends_with("id");
        if !wanted || ids.is_empty() {
            continue;
        }
        let values = candidates(property, ids);
        sets = sets
            .into_iter()
            .flat_map(|set| {
                values.iter().map(move |value| {
                    let mut set = set.clone();
                    set.insert(property.clone(), Value::String(value.clone()));
                    set
                })
            })
            .collect();
    }
    sets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thay_tham_so_bang_moi_id_hop_ten() {
        let ids = [("id", "A1".to_owned()), ("id", "A2".to_owned()), ("form_id", "F".to_owned())];
        assert_eq!(
            substitutions("/api/v1/forms/{form_id}/items/{id}", &ids),
            ["/api/v1/forms/F/items/A1", "/api/v1/forms/F/items/A2"]
        );
        assert_eq!(substitutions("/api/v1/x/{slug}", &ids[2..]), ["/api/v1/x/F"]);
        assert_eq!(substitutions("/api/v1/notes", &ids), ["/api/v1/notes"]);

        let properties = vec!["id".to_owned(), "limit".to_owned()];
        let sets = tool_arguments(&properties, &ids[..2]);
        assert_eq!(sets.len(), 2);
        assert_eq!(sets[1]["id"], "A2");
        assert!(tool_arguments(&["q".to_owned()], &ids)[0].is_empty());
    }
}
