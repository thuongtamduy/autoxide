//! Tài liệu OpenAPI của từng module: `GET /api/v1/openapi/<module>.json`
//! (D-20261118-module-contracts).
//!
//! Tài liệu chung hơn một trăm nghìn dòng: agent chỉ cần làm việc với `notes` không nên phải nạp cả
//! hợp đồng của mọi module. Bản của một module giữ đúng các đường dẫn dưới `/api/v1/<module>` và
//! các schema chúng dùng (đi hết `$ref` lồng nhau), cùng phần đầu (quy ước chung, cách xác thực)
//! của tài liệu chung — đọc một mình là đủ để gọi đúng, và mọi `$ref` trỏ tới mục có thật.
//!
//! Tách một lần lúc dựng router, sau khi tài liệu chung đã hoàn tất (`finish`, deprecation), nên
//! bản của module luôn khớp từng chữ với phần tương ứng của bản chung.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::IntoResponse;
use serde_json::{Map, Value};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::Error;
use crate::response::Problem;

/// Tài liệu của từng module (tên module → JSON), điền sau khi dựng xong tài liệu chung.
pub(super) type ModuleDocuments = Arc<OnceLock<BTreeMap<String, Bytes>>>;

/// Đường dẫn tài liệu của module `module`.
pub(super) fn module_openapi_path(module: &str) -> String {
    format!("{}/openapi/{module}.json", super::API_PREFIX)
}

/// Route `GET /api/v1/openapi/{file}`.
pub(super) fn routes(documents: ModuleDocuments) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(module_openapi)).with_state(documents)
}

/// Tách tài liệu chung thành tài liệu của từng module trong `modules` (module không có đường dẫn
/// nào thì không có tài liệu).
pub(super) fn split(document: &Value, modules: &[&str]) -> BTreeMap<String, Bytes> {
    modules
        .iter()
        .filter_map(|module| {
            let part = module_document(document, module)?;
            let json = serde_json::to_vec(&part).ok()?;
            Some(((*module).to_owned(), Bytes::from(json)))
        })
        .collect()
}

/// Tài liệu của một module; `None` khi module không có đường dẫn nào.
fn module_document(document: &Value, module: &str) -> Option<Value> {
    let prefix = format!("{}/{module}", super::API_PREFIX);
    let paths: Map<String, Value> = document
        .get("paths")?
        .as_object()?
        .iter()
        .filter(|(path, _)| {
            path.as_str() == prefix
                || path.strip_prefix(&prefix).is_some_and(|rest| rest.starts_with('/'))
        })
        .map(|(path, item)| (path.clone(), item.clone()))
        .collect();
    if paths.is_empty() {
        return None;
    }

    // Schema dùng tới: `$ref` trong các đường dẫn, rồi `$ref` trong chính các schema đó.
    let schemas = document.pointer("/components/schemas").and_then(Value::as_object);
    let mut needed = BTreeSet::new();
    let mut pending: Vec<String> = Vec::new();
    collect_schema_refs(&Value::Object(paths.clone()), &mut pending);
    while let Some(name) = pending.pop() {
        if needed.insert(name.clone())
            && let Some(schema) = schemas.and_then(|schemas| schemas.get(&name))
        {
            collect_schema_refs(schema, &mut pending);
        }
    }

    let mut part = Map::new();
    for (key, value) in document.as_object()? {
        match key.as_str() {
            "paths" => {}
            "info" => {
                let mut info = value.clone();
                info["title"] = Value::from(format!("Autoxide API — {module}"));
                let conventions =
                    value.get("description").and_then(Value::as_str).unwrap_or_default();
                info["description"] = Value::from(format!(
                    "Only the endpoints of the `{module}` module, with the schemas they use. The \
                     whole API is at `{}`; every module's document is listed in `/llms.txt`.\n\n\
                     {conventions}",
                    super::OPENAPI_PATH
                ));
                part.insert(key.clone(), info);
            }
            "components" => {
                let mut components = value.clone();
                if let Some(Value::Object(all)) = components.get_mut("schemas") {
                    all.retain(|name, _| needed.contains(name));
                }
                part.insert(key.clone(), components);
            }
            "tags" => {
                let used = used_tags(&paths);
                let tags: Vec<Value> = value
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|tag| {
                        tag.get("name")
                            .and_then(Value::as_str)
                            .is_some_and(|name| used.contains(name))
                    })
                    .cloned()
                    .collect();
                part.insert(key.clone(), Value::Array(tags));
            }
            _ => {
                part.insert(key.clone(), value.clone());
            }
        }
    }
    part.insert("paths".to_owned(), Value::Object(paths));
    Some(Value::Object(part))
}

/// Tên schema của mọi `$ref` `#/components/schemas/<tên>` trong `node`.
fn collect_schema_refs(node: &Value, names: &mut Vec<String>) {
    match node {
        Value::Object(map) => {
            for (key, value) in map {
                match value {
                    Value::String(reference) if key == "$ref" => {
                        if let Some(name) = reference.strip_prefix("#/components/schemas/") {
                            names.push(name.to_owned());
                        }
                    }
                    _ => collect_schema_refs(value, names),
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|item| collect_schema_refs(item, names)),
        _ => {}
    }
}

/// Tag của mọi operation trong `paths`.
fn used_tags(paths: &Map<String, Value>) -> BTreeSet<String> {
    paths
        .values()
        .filter_map(Value::as_object)
        .flat_map(|item| item.values())
        .filter_map(|operation| operation.get("tags").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

/// Lỗi khi không có module (có đường dẫn) tên đó.
fn not_found(module: &str) -> Error {
    Error::not_found(
        "openapi_module_not_found",
        "No module named `{module}` has endpoints on this server. The installed modules, with \
         links to their documents, are listed at `/llms.txt`.",
    )
    .with_param("module", module.to_owned())
}

/// OpenAPI document of one module.
///
/// Only the endpoints under `/api/v1/<module>` and the schemas they use, with the same
/// conventions and security schemes as the whole document at `/api/v1/openapi.json`. Lets an agent
/// load just the part of the API it works with; `/llms.txt` links every module's document.
#[utoipa::path(
    get,
    path = "/api/v1/openapi/{file}",
    extensions(("x-public" = json!(true))),
    tag = "health",
    operation_id = "module_openapi",
    params(("file" = String, Path, description = "The module name followed by `.json`, e.g. `notes.json`.")),
    responses(
        (status = 200, description = "The module's OpenAPI 3.1 document.", content_type = "application/json", body = Object),
        (status = 404, description = "No installed module with endpoints has this name, or the name does not end in `.json`; code `openapi_module_not_found`.", body = Problem),
    )
)]
async fn module_openapi(
    State(documents): State<ModuleDocuments>,
    Path(file): Path<String>,
) -> Result<impl IntoResponse, Error> {
    let module = file.strip_suffix(".json").ok_or_else(|| not_found(&file))?;
    let document = documents
        .get()
        .and_then(|documents| documents.get(module))
        .cloned()
        .ok_or_else(|| not_found(module))?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "public, max-age=300"),
        ],
        document,
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn chi_giu_duong_dan_va_schema_cua_module() {
        let document = json!({
            "openapi": "3.1.0",
            "info": {"title": "Autoxide API", "version": "1", "description": "Conventions."},
            "tags": [{"name": "notes"}, {"name": "auth"}],
            "paths": {
                "/api/v1/notes": {"get": {"tags": ["notes"], "responses": {"200": {"content": {"application/json": {"schema": {"$ref": "#/components/schemas/Page"}}}}}}},
                "/api/v1/notes/{id}": {"get": {"tags": ["notes"]}},
                "/api/v1/notesx": {"get": {"tags": ["notes"]}},
                "/api/v1/auth/me": {"get": {"tags": ["auth"], "responses": {"200": {"content": {"application/json": {"schema": {"$ref": "#/components/schemas/User"}}}}}}},
            },
            "components": {
                "schemas": {
                    "Page": {"properties": {"data": {"items": {"$ref": "#/components/schemas/Note"}}}},
                    "Note": {"properties": {"author": {"$ref": "#/components/schemas/Author"}}},
                    "Author": {"type": "object"},
                    "User": {"type": "object"},
                },
                "securitySchemes": {"bearer": {"type": "http"}},
            },
        });
        let part = module_document(&document, "notes").unwrap();
        let paths: Vec<&String> = part["paths"].as_object().unwrap().keys().collect();
        assert_eq!(paths, ["/api/v1/notes", "/api/v1/notes/{id}"], "`notesx` là module khác");
        let schemas: Vec<&String> =
            part["components"]["schemas"].as_object().unwrap().keys().collect();
        assert_eq!(schemas, ["Author", "Note", "Page"], "đi hết `$ref` lồng nhau, bỏ `User`");
        assert_eq!(
            part["components"]["securitySchemes"],
            document["components"]["securitySchemes"]
        );
        assert_eq!(part["tags"], json!([{"name": "notes"}]));
        assert_eq!(part["info"]["title"], "Autoxide API — notes");
        assert!(part["info"]["description"].as_str().unwrap().ends_with("Conventions."));
        assert!(module_document(&document, "orders").is_none());
        assert_eq!(split(&document, &["notes", "orders"]).len(), 1);
    }
}
