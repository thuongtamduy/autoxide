//! Cross-tenant leak test (step 33, spec "Mandatory tests"): after seeding data in tenant A, call
//! **every** `GET` endpoint of the app and **every** read-only MCP tool as a user of tenant B (with all
//! permissions). A violation is a successful answer containing a sign of tenant A: the string
//! `marker` (seeded in a title, content, email…) or an ID of A.
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
//! A `{name}` parameter in a path is replaced in turn by every A ID declared under the same name (if none,
//! by every A ID); tool parameters named in `ids`, or whose name ends in `id`, work the same way.
//!
//! [`cursor_leak_check`]: a pagination cursor obtained in tenant A and sent as a tenant B user never
//! reveals A's data (every query is already filtered by tenant; a cursor is only a position).

use serde_json::{Map, Value};

use crate::app::{TestApp, TestAppError, tool};

/// Leak test between two sibling branches of the organization tree (step 34, spec "Mandatory tests"): within
/// one tenant, data seeded in branch A (the string `marker`, the IDs in `ids`); call every `GET` and every
/// read-only MCP tool with the `token` of a user who has permission only in branch B (e.g.
/// `.member(token, …, b, &[("notes.read", Scope::Subtree)])`). A violation is a successful answer containing
/// a sign of A.
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

/// Calls every `GET` and every read-only MCP tool with `token` (a user of tenant B); returns the list of
/// violations (empty means pass). `ids`: IDs seeded in tenant A, by parameter name (`("id", …)`,
/// `("form_id", …)`).
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

/// A pagination cursor from tenant A used in tenant B (step 33, D-259 item 5): for every `GET` that returns a
/// page (`pagination.next_cursor`), fetch the first page with `limit=1` as `owner` (a user of A) — once per
/// value of the `sort` parameter if the endpoint has one, to try column-based cursors too — then send that
/// cursor as `other` (a user of B). A violation is a successful answer containing a sign of A (`marker` or an
/// ID in `ids`). Returns (violations, number of cursors tried): the caller checks that number is > 0 to know
/// the test really ran (A needs at least two rows in the list to get a cursor).
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

/// Values of the `sort` query parameter of operation `operation` (an enum, including via `$ref`).
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

/// Every way to replace the `{param}` placeholders of `path` with IDs of tenant A.
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

/// IDs declared for parameter `name`; if none, all IDs (the path is still tried).
fn candidates(name: &str, ids: &[(&str, String)]) -> Vec<String> {
    let named: Vec<String> =
        ids.iter().filter(|(key, _)| *key == name).map(|(_, id)| id.clone()).collect();
    if named.is_empty() { ids.iter().map(|(_, id)| id.clone()).collect() } else { named }
}

/// The parameter sets to try for one tool: each parameter named in `ids` or ending in `id` receives the A IDs
/// in turn; a tool with no such parameter is called once with `{}`.
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
