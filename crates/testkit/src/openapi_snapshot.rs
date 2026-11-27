//! The OpenAPI contract of one module, pinned by a file in the module: a change to an endpoint's description fails a
//! test until the file is updated on purpose.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{Map, Value};

/// The part of `openapi` that belongs to module `module`: its paths (`/api/v1/<module>…`) and every schema they reach
/// through `$ref`, as JSON with keys sorted by name (the order `autoxide openapi` writes, stable between runs).
pub fn module_openapi(openapi: &utoipa::openapi::OpenApi, module: &str) -> Value {
    let document = serde_json::to_value(openapi).unwrap_or_default();
    let prefix = format!("/api/v1/{module}");
    let mut paths = Map::new();
    if let Some(all) = document.get("paths").and_then(Value::as_object) {
        for (path, item) in all {
            if path == &prefix || path.starts_with(&format!("{prefix}/")) {
                paths.insert(path.clone(), item.clone());
            }
        }
    }
    let schemas = document
        .pointer("/components/schemas")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut reached = BTreeSet::new();
    let mut pending: Vec<String> = Vec::new();
    collect_refs(&Value::Object(paths.clone()), &mut pending);
    while let Some(name) = pending.pop() {
        if reached.insert(name.clone())
            && let Some(schema) = schemas.get(&name)
        {
            collect_refs(schema, &mut pending);
        }
    }
    let reached: Map<String, Value> = reached
        .into_iter()
        .filter_map(|name| schemas.get(&name).map(|schema| (name, schema.clone())))
        .collect();
    serde_json::json!({ "paths": paths, "components": { "schemas": reached } })
}

fn collect_refs(value: &Value, found: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(target)) = map.get("$ref")
                && let Some(name) = target.strip_prefix("#/components/schemas/")
            {
                found.push(name.to_owned());
            }
            map.values().for_each(|inner| collect_refs(inner, found));
        }
        Value::Array(items) => items.iter().for_each(|inner| collect_refs(inner, found)),
        _ => {}
    }
}

/// Compares `actual` with the JSON file `path` (pretty-printed, keys sorted). On a difference it writes the new
/// version next to the file as `<path>.new` and returns an error naming both: review the change, then replace the
/// file with the `.new` one.
pub fn check_openapi_snapshot(actual: &Value, path: &Path) -> Result<(), String> {
    // Qua `Value` lần nữa để khoá luôn sắp theo tên, kể cả khi bản chụp được sửa tay.
    let canonical = |value: &Value| serde_json::to_string_pretty(value).unwrap_or_default() + "\n";
    let actual = canonical(actual);
    let expected = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .map(|value| canonical(&value));
    if expected.as_deref() == Some(actual.as_str()) {
        return Ok(());
    }
    let new = path.with_extension("json.new");
    std::fs::write(&new, &actual).map_err(|error| format!("{}: {error}", new.display()))?;
    Err(format!(
        "the OpenAPI contract differs from {}; the new version is in {}: review it, then replace the file with it",
        path.display(),
        new.display()
    ))
}
