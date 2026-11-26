//! OpenAPI document: completes the shared parts, serves it over HTTP, and checks its quality.
//!
//! The document is generated from the route declarations themselves (`routes!`), so paths and methods
//! cannot drift from the code. This file adds the things that are identical on every endpoint so module
//! authors need not repeat them:
//!
//! - tag = the module name, so the document groups by module;
//! - `operationId` carries the module prefix, so it does not collide across modules (SDK generators use it
//!   as the function name);
//! - the `default` response = Problem Details: every endpoint can return an error of that form.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::json;
use utoipa::ToSchema;
use utoipa::openapi::header::HeaderBuilder;
use utoipa::openapi::path::{Operation, Parameter, ParameterBuilder, ParameterIn, PathItem};
use utoipa::openapi::schema::{ObjectBuilder, Type};
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::openapi::server::ServerBuilder;
use utoipa::openapi::{
    ContentBuilder, InfoBuilder, OpenApi, OpenApiBuilder, Ref, RefOr, Required, ResponseBuilder,
};

use crate::response::{PROBLEM_CONTENT_TYPE, Problem};

/// The path serving the OpenAPI document as JSON.
pub const OPENAPI_PATH: &str = "/api/v1/openapi.json";
/// The path of the API documentation viewer page (when enabled).
pub const DOCS_PATH: &str = "/api/docs";

/// The name of the "bearer token" authentication scheme in the document. A handler that requires sign-in
/// declares `security((BEARER_AUTH = []))` - write the string `"bearer"` in `#[utoipa::path]`.
pub const BEARER_AUTH: &str = "bearer";

const DESCRIPTION: &str = "\
REST API of Autoxide.

## Conventions

Every endpoint follows the same rules, so learning one endpoint teaches all of them.

- **Success**: a single result is `{\"data\": …}`; a list is \
`{\"data\": […], \"pagination\": {\"limit\", \"next_cursor\"}}`. Creating a record answers \
`201` with a `Location` header pointing to it.
- **Errors**: always RFC 9457 Problem Details (`application/problem+json`). Branch on the \
stable `code` field, never on `detail`. Validation problems list every invalid field of the \
request in `errors`, all at once.
- **Lists**: accept `limit` (1–200, default 50), `order` (`desc` or `asc`, by creation time) and \
`cursor`. Some lists also take `sort` to order by another field, and their own filters. To get the \
next page, pass `pagination.next_cursor` as `cursor` with the same `order`, `sort` and filters; \
`null` means last page.
- **Authentication**: endpoints that need it list the `bearer` security scheme. Send \
`Authorization: Bearer <access token>`. A `401` response carries `WWW-Authenticate: Bearer`; \
its `code` says whether the token is missing, invalid or expired. Public endpoints list an empty \
requirement next to `bearer`: they work without a token, and a sent token must still be valid.
- **Retries**: send `Idempotency-Key: <a random UUID>` with `POST`, `PUT`, `PATCH` and \
`DELETE`. Repeating the same request with the same key within 24 hours returns the first result \
(marked `Idempotent-Replayed: true`) instead of doing it twice; a different request with a used \
key gets `422`, and a repeat while the first is still running gets `409`.
- **Concurrent edits**: reading a single record returns its version in `ETag`. Send it back \
in `If-Match` when changing the record: if someone changed it in between, the answer is `412` \
(code `precondition_failed`) instead of silently overwriting their change. Without `If-Match` \
the change is made anyway, except on endpoints that say `If-Match` is required: those answer \
`428` (code `precondition_required`).
- **Caching**: send the `ETag` you have in `If-None-Match` when reading again; if nothing \
changed the answer is `304 Not Modified` with no body.
- **Deprecation**: an endpoint being retired is marked `deprecated` here and answers with \
`Deprecation` (RFC 9745), `Sunset` (RFC 8594, the removal date) and `Link: <…>; \
rel=\"successor-version\"` headers. Move to the successor before the sunset date.
- **Language**: send `Accept-Language` (e.g. `vi`) to get the `detail` of errors in that \
language; English (`en`) by default and for unsupported languages. Responses carry \
`Content-Language`. Codes never change with the language.
- **Rate limits**: too many requests from one address are answered with `429` (code \
`rate_limited`) and a `Retry-After` header giving the seconds to wait.
- **Which tenant**: on a server with many tenants, a request is served for the tenant of its \
credential (the token or API key); without one, for the tenant named in the header \
`Autoxide-Tenant: <slug or id>`, else for the tenant whose verified domain is in `Host` (behind \
a trusted proxy, `X-Forwarded-Host` or `Forwarded`). A tenant endpoint that cannot tell answers \
`400` with code `tenant_required` (for example a domain not bound to any tenant); a header naming \
no tenant answers `404` with code `tenant_not_found`; a header or domain of another tenant than \
the credential's answers `403` with code `tenant_mismatch`. A server with one tenant ignores \
both.
- **Tenant state**: any endpoint of a tenant may answer `403` with code `tenant_suspended` (the \
tenant is suspended: nothing works, reads included), `403` with code `tenant_read_only` (the \
tenant can only read for now: `GET` works, as do sign-in, exports and links from mails; other \
changes are refused; MCP tools that are not read-only too) or `503` with code \
`tenant_maintenance` and a `Retry-After` header.
- **Request id**: every response carries `x-request-id`. Send your own to correlate calls; quote \
it when reporting a problem.
- **IDs** are UUIDv7 strings; **timestamps** are UTC in RFC 3339; **money** is an integer amount \
in the smallest currency unit plus a currency code.";

/// The top of the document: name, version, and the API's shared conventions.
pub(super) fn base_document() -> OpenApi {
    OpenApiBuilder::new()
        .info(
            InfoBuilder::new()
                .title("Autoxide API")
                .version(env!("CARGO_PKG_VERSION"))
                .description(Some(DESCRIPTION))
                .build(),
        )
        // Địa chỉ tương đối: tài liệu được phục vụ từ chính máy chủ (`/api/v1/openapi.json`)
        // nên `/` là máy chủ đó; bản trong repo không biết tên miền của người triển khai.
        .servers(Some([ServerBuilder::new()
            .url("/")
            .description(Some(
                "The server that serves this document. When generating a client from a saved \
                 copy, use your deployment's address instead, e.g. `https://api.example.com`.",
            ))
            .build()]))
        .build()
}

/// Tags the document of ONE module before it is merged into the shared document.
pub(super) fn label_module(module: &str, openapi: &mut OpenApi) {
    for item in openapi.paths.paths.values_mut() {
        for (_, operation) in operations_mut(item) {
            // Module tự đặt tag thì giữ; không thì tag là tên module.
            if operation.tags.as_ref().is_none_or(Vec::is_empty) {
                operation.tags = Some(vec![module.to_owned()]);
            }
            if let Some(id) = &operation.operation_id {
                operation.operation_id = Some(format!("{module}_{id}"));
            }
        }
    }
}

/// Records on every operation of a plan-governed module (step 35) that it may return 403 because of the
/// plan: appends a sentence to an existing 403 response, or adds a new 403 response.
pub(super) fn document_plan_gate(openapi: &mut OpenApi) {
    for item in openapi.paths.paths.values_mut() {
        for (method, operation) in operations_mut(item) {
            let writes = matches!(method, "post" | "put" | "patch" | "delete");
            let sentence = if writes {
                "The tenant's plan does not include this module (`module_not_enabled`), or the \
                 subscription is suspended or canceled and data can only be read \
                 (`tenant_read_only`)."
            } else {
                "The tenant's plan does not include this module (`module_not_enabled`)."
            };
            match operation.responses.responses.get_mut("403") {
                Some(RefOr::T(response)) => {
                    response.description = format!("{} {sentence}", response.description);
                }
                Some(RefOr::Ref(_)) => {}
                None => {
                    let response = ResponseBuilder::new()
                        .description(sentence)
                        .content(
                            PROBLEM_CONTENT_TYPE,
                            ContentBuilder::new()
                                .schema(Some(RefOr::Ref(Ref::from_schema_name(Problem::name()))))
                                .build(),
                        )
                        .build();
                    operation.responses.responses.insert("403".to_owned(), RefOr::T(response));
                }
            }
        }
    }
}

/// Completes the shared document after every module has been merged. Finalizes the document: the `Problem`
/// schema, authentication, default error responses, and the shared headers declared on each operation (so
/// generated SDKs have the parameters ready). `idempotency`: whether an idempotency key store exists (the
/// `Idempotency-Key` header has an effect).
pub(super) fn finish(openapi: &mut OpenApi, idempotency: bool) {
    // Schema `Problem` (và các schema nó dùng) phải có mặt để `$ref` bên dưới trỏ tới.
    let mut schemas =
        vec![(Problem::name().into_owned(), <Problem as utoipa::PartialSchema>::schema())];
    Problem::schemas(&mut schemas);
    let components = openapi.components.get_or_insert_default();
    for (name, schema) in schemas {
        components.schemas.entry(name).or_insert(schema);
    }
    components.security_schemes.entry(BEARER_AUTH.to_owned()).or_insert_with(|| {
        RefOr::T(SecurityScheme::Http(
            HttpBuilder::new()
                .scheme(HttpAuthScheme::Bearer)
                .bearer_format("JWT")
                .description(Some(
                    "A short-lived access token, sent as `Authorization: Bearer <token>`.",
                ))
                .build(),
        ))
    });

    let problem = ResponseBuilder::new()
        .description(
            "Any error, as RFC 9457 Problem Details. The `code` field identifies the problem.",
        )
        .content(
            PROBLEM_CONTENT_TYPE,
            ContentBuilder::new()
                .schema(Some(RefOr::Ref(Ref::from_schema_name(Problem::name()))))
                .build(),
        )
        .build();
    for item in openapi.paths.paths.values_mut() {
        for (_, operation) in operations_mut(item) {
            operation
                .responses
                .responses
                .entry("default".to_owned())
                .or_insert_with(|| RefOr::T(problem.clone()));
        }
    }
    for item in openapi.paths.paths.values_mut() {
        for (method, operation) in operations_mut(item) {
            // Lỗi thật luôn trả `application/problem+json` (`response::Problem`), nhưng `body =
            // Problem` trong `#[utoipa::path]` khai `application/json`: sửa thành đúng media type để
            // tài liệu không hứa sai (đặc tả 31 B4).
            for response in operation.responses.responses.values_mut() {
                if let RefOr::T(response) = response {
                    problem_media_type(response);
                }
            }
            // Mọi response 201 đến từ `response::Created`, luôn có `Location`.
            if let Some(RefOr::T(created)) = operation.responses.responses.get_mut("201") {
                created.headers.entry("Location".to_owned()).or_insert_with(|| {
                    RefOr::T(
                        HeaderBuilder::new()
                            .schema(Some(ObjectBuilder::new().schema_type(Type::String)))
                            .description(Some(
                                "Where the new record is, e.g. `/api/v1/auth/users/<id>`.",
                            ))
                            .build(),
                    )
                });
            }
            // Lớp idempotency chỉ lưu lệnh ghi của người đã đăng nhập.
            let writes = matches!(method, "post" | "put" | "patch" | "delete");
            let signed_in =
                operation.security.as_ref().is_some_and(|security| !security.is_empty());
            let parameters = operation.parameters.get_or_insert_default();
            // Endpoint tự khai một header chung (vd thao tác hàng loạt bắt buộc `Idempotency-Key`)
            // thì giữ bản của nó: khai hai lần làm tài liệu sai và SDK TypeScript không biên dịch.
            push_header(parameters, accept_language());
            if idempotency && writes && signed_in {
                push_header(parameters, idempotency_key());
            }
        }
    }
}

/// A response whose body is `Problem` declared under `application/json` is switched to
/// `application/problem+json`.
fn problem_media_type(response: &mut utoipa::openapi::Response) {
    let problem_ref = format!("#/components/schemas/{}", Problem::name());
    let is_problem = response.content.get("application/json").is_some_and(|content| {
        matches!(
            content,
            RefOr::T(content) if matches!(&content.schema, Some(RefOr::Ref(reference)) if reference.ref_location == problem_ref)
        )
    });
    if is_problem && let Some(content) = response.content.shift_remove("application/json") {
        response.content.insert(PROBLEM_CONTENT_TYPE.to_owned(), content);
    }
}

/// The machine-readable permission contract (D-205, spec 54 P1, D-245): each operation declares exactly one
/// of:
///
/// - `x-public: true` - callable without signing in;
/// - `x-permissions: [..]` - requires sign-in and one of the permissions (an empty list: only sign-in is
///   required); the HTTP layer refuses a caller with none of them before the handler runs
///   (D-20261126-permission-gate). With `x-buyers: true` as well, a buyer account (which has no permissions)
///   may also call the operation; staff still need one of the permissions.
///
/// Each declared permission must be mentioned in the operation's description or 403 response, so the
/// human-readable text does not drift from the declaration.
fn access_problems(endpoint: &str, operation: &utoipa::openapi::path::Operation) -> Vec<String> {
    let extensions = operation.extensions.as_ref();
    let public = extensions.and_then(|extensions| extensions.get("x-public"));
    let permissions = extensions.and_then(|extensions| extensions.get("x-permissions"));
    match (public, permissions) {
        (Some(serde_json::Value::Bool(true)), None) => Vec::new(),
        (None, Some(serde_json::Value::Array(names))) => {
            let mut text = format!(
                "{} {}",
                operation.summary.as_deref().unwrap_or_default(),
                operation.description.as_deref().unwrap_or_default()
            );
            if let Some(RefOr::T(forbidden)) = operation.responses.responses.get("403") {
                text.push_str(&forbidden.description);
            }
            names
                .iter()
                .filter_map(|name| match name.as_str() {
                    Some(name) if text.contains(&format!("`{name}`")) => None,
                    Some(name) => Some(format!(
                        "{endpoint}: `x-permissions` lists `{name}`, but neither the description nor \
                         the 403 response mentions it"
                    )),
                    None => Some(format!("{endpoint}: `x-permissions` must list permission names")),
                })
                .collect()
        }
        _ => vec![format!(
            "{endpoint}: declare who may call it — add `extensions((\"x-permissions\" = \
             json!([\"<module>.<permission>\"])))` (`[]`: any signed-in caller) or \
             `extensions((\"x-public\" = json!(true)))` to `#[utoipa::path]` (D-245)"
        )],
    }
}

/// Checks whether the document is enough to call the API correctly without reading the source code. Returns
/// the list of problems; empty means it passes. The gate runs this function on the document of the whole
/// program.
pub fn lint(openapi: &OpenApi) -> Vec<String> {
    let mut problems = Vec::new();
    let mut operation_ids = BTreeSet::new();
    for (path, item) in &openapi.paths.paths {
        for (method, operation) in operations(item) {
            let endpoint = format!("{} {path}", method.to_uppercase());
            if operation.summary.as_deref().is_none_or(|summary| summary.trim().is_empty()) {
                problems.push(format!(
                    "{endpoint}: no summary — add a `///` doc comment to the handler"
                ));
            }
            if operation.tags.as_ref().is_none_or(Vec::is_empty) {
                problems.push(format!("{endpoint}: no tag"));
            }
            match &operation.operation_id {
                None => problems.push(format!("{endpoint}: no operationId")),
                Some(id) => {
                    if !operation_ids.insert(id.clone()) {
                        problems.push(format!("{endpoint}: operationId `{id}` is used twice"));
                    }
                }
            }
            // Cùng tên và vị trí khai hai lần thì tài liệu mâu thuẫn (một bản bắt buộc, một bản
            // không) và SDK TypeScript sinh ra không biên dịch được. Tên header không phân biệt hoa
            // thường (RFC 9110), tên tham số khác thì có.
            let mut seen = BTreeSet::new();
            for parameter in operation.parameters.iter().flatten() {
                if let RefOr::T(parameter) = parameter {
                    let place = match parameter.parameter_in {
                        ParameterIn::Header => "header",
                        ParameterIn::Path => "path",
                        ParameterIn::Query => "query",
                        ParameterIn::QueryString => "querystring",
                        ParameterIn::Cookie => "cookie",
                    };
                    let name = if place == "header" {
                        parameter.name.to_ascii_lowercase()
                    } else {
                        parameter.name.clone()
                    };
                    if !seen.insert((place, name)) {
                        problems.push(format!(
                            "{endpoint}: {place} parameter `{}` is declared twice",
                            parameter.name
                        ));
                    }
                }
            }
            let responses = &operation.responses.responses;
            // `101`: bắt tay WebSocket thành công (bước 69 P3) cũng là trả lời thành công.
            if !responses
                .keys()
                .any(|status| status.starts_with('2') || status.starts_with('3') || status == "101")
            {
                problems.push(format!(
                    "{endpoint}: no success response — declare what the endpoint returns"
                ));
            }
            for (status, response) in responses {
                if let RefOr::T(response) = response
                    && response.description.trim().is_empty()
                {
                    problems.push(format!("{endpoint}: response {status} has no description"));
                }
            }
            problems.extend(access_problems(&endpoint, operation));
        }
        problems.extend(head_problems(path, item));
    }
    match serde_json::to_value(openapi) {
        Ok(document) => {
            problems.extend(dangling_refs(&document));
            problems.extend(untranslated_text(&document, "OpenAPI"));
        }
        Err(error) => problems.push(format!("the document cannot be serialized: {error}")),
    }
    problems
}

/// A `HEAD` declared next to a `GET` follows its own `x-permissions` in the permission layer
/// (D-20261126-permission-gate), so it must not be more open than that `GET`: when the `GET` lists permissions, the
/// `HEAD` lists permissions too, each of them also opening the `GET`. Both public is fine.
fn head_problems(path: &str, item: &PathItem) -> Option<String> {
    let (Some(get), Some(head)) = (&item.get, &item.head) else { return None };
    let listed = |operation: &Operation| -> Vec<String> {
        operation
            .extensions
            .as_ref()
            .and_then(|extensions| extensions.get("x-permissions"))
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_str().map(str::to_owned))
            .collect()
    };
    let (get, head) = (listed(get), listed(head));
    if get.is_empty() {
        return None;
    }
    let wider: Vec<&String> = head.iter().filter(|name| !get.contains(name)).collect();
    if head.is_empty() || !wider.is_empty() {
        return Some(format!(
            "HEAD {path}: is more open than GET {path} — list in `x-permissions` only permissions of the GET \
             ({get:?}), at least one"
        ));
    }
    None
}

/// The names of schemas that two places in `documents` (place name, that place's document) declare with
/// different content. The same name with the same content (e.g. the kernel's `Problem`, used by every
/// module) is fine.
pub(super) fn schema_conflicts<'a>(
    documents: impl IntoIterator<Item = (&'a str, &'a OpenApi)>,
) -> Vec<String> {
    let mut seen: BTreeMap<&str, (&str, serde_json::Value)> = BTreeMap::new();
    let mut problems = Vec::new();
    for (owner, document) in documents {
        let Some(components) = &document.components else { continue };
        for (name, schema) in &components.schemas {
            let value = serde_json::to_value(schema).unwrap_or_default();
            match seen.get(name.as_str()) {
                Some((first, known)) if *known != value => problems.push(format!(
                    "schema `{name}` is declared differently by `{first}` and `{owner}` — one \
                     would silently replace the other; rename one with `#[schema(as = …)]`"
                )),
                Some(_) => {}
                None => {
                    seen.insert(name, (owner, value));
                }
            }
        }
    }
    problems
}

/// Every `$ref` must point to an entry that exists in the document itself: a missing entry makes the
/// document invalid - standard validators and SDK generators reject the whole file.
fn dangling_refs(document: &serde_json::Value) -> Vec<String> {
    let mut refs = BTreeSet::new();
    collect_refs(document, &mut refs);
    refs.into_iter()
        .filter(|reference| {
            let target = reference
                .strip_prefix("#/")
                .map(|path| path.split('/').try_fold(document, |node, key| node.get(key)));
            !matches!(target, Some(Some(_)))
        })
        .map(|reference| {
            format!(
                "`$ref` `{reference}` points to nothing in the document — the type is used but \
                 not registered (derive `ToSchema` and list it, or mark the field `inline`)"
            )
        })
        .collect()
}

fn collect_refs(node: &serde_json::Value, refs: &mut BTreeSet<String>) {
    match node {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                match value {
                    serde_json::Value::String(reference) if key == "$ref" => {
                        refs.insert(reference.clone());
                    }
                    _ => collect_refs(value, refs),
                }
            }
        }
        serde_json::Value::Array(items) => items.iter().for_each(|item| collect_refs(item, refs)),
        _ => {}
    }
}

/// Descriptions sent outward (OpenAPI, MCP tools) must be in English: `///` on a type with `ToSchema` /
/// `IntoParams` is copied straight into them. Reports every `summary` / `description` containing Vietnamese
/// text; `what` is the document's name in the message.
pub(crate) fn untranslated_text(document: &serde_json::Value, what: &str) -> Vec<String> {
    let mut problems = Vec::new();
    collect_untranslated(document, what, &mut problems);
    problems
}

fn collect_untranslated(node: &serde_json::Value, what: &str, problems: &mut Vec<String>) {
    match node {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                match value {
                    serde_json::Value::String(text)
                        if matches!(key.as_str(), "summary" | "description" | "title")
                            && looks_vietnamese(text) =>
                    {
                        problems.push(format!(
                            "{what}: `{text}` is not English — `///` comments on handlers and on \
                             types with `ToSchema`/`IntoParams` are published; write them in \
                             English and keep notes for readers of the code in `//` comments"
                        ));
                    }
                    _ => collect_untranslated(value, what, problems),
                }
            }
        }
        serde_json::Value::Array(items) => {
            items.iter().for_each(|item| collect_untranslated(item, what, problems));
        }
        _ => {}
    }
}

/// Contains letters specific to Vietnamese (`ă â đ ê ô ơ ư`, or vowels with tone marks).
pub(crate) fn looks_vietnamese(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c, 'ă' | 'Ă' | 'đ' | 'Đ' | 'ơ' | 'Ơ' | 'ư' | 'Ư')
            || ('\u{1EA0}'..='\u{1EF9}').contains(&c)
    })
}

/// Adds a shared header to the parameter list, unless the operation already declares a header of the same
/// name.
fn push_header(parameters: &mut Vec<RefOr<Parameter>>, header: Parameter) {
    let declared = parameters.iter().any(|parameter| {
        matches!(parameter, RefOr::T(parameter)
            if parameter.parameter_in == ParameterIn::Header
                && parameter.name.eq_ignore_ascii_case(&header.name))
    });
    if !declared {
        parameters.push(RefOr::T(header));
    }
}

/// The `Accept-Language` header, present on every operation.
fn accept_language() -> Parameter {
    ParameterBuilder::new()
        .name("Accept-Language")
        .parameter_in(ParameterIn::Header)
        .required(Required::False)
        .description(Some(
            "Language of the `detail` of errors: `en` (the default) or `vi`. Codes never change \
             with the language.",
        ))
        .schema(Some(ObjectBuilder::new().schema_type(Type::String).examples([json!("vi")])))
        .build()
}

/// The `Idempotency-Key` header, present on every write operation that requires sign-in.
fn idempotency_key() -> Parameter {
    ParameterBuilder::new()
        .name("Idempotency-Key")
        .parameter_in(ParameterIn::Header)
        .required(Required::False)
        .description(Some(
            "A random UUID that makes this request safe to retry: a repeat with the same key \
             within 24 hours returns the first result (with `Idempotent-Replayed: true`) instead \
             of doing it twice.",
        ))
        .schema(Some(
            ObjectBuilder::new()
                .schema_type(Type::String)
                .max_length(Some(255))
                .examples([json!("0193e3a4-7b1c-7f00-9a51-6f2b8c0d4e11")]),
        ))
        .build()
}

/// The operations of one path, with the method name.
fn operations(item: &PathItem) -> impl Iterator<Item = (&'static str, &Operation)> {
    [
        ("get", &item.get),
        ("put", &item.put),
        ("post", &item.post),
        ("delete", &item.delete),
        ("options", &item.options),
        ("head", &item.head),
        ("patch", &item.patch),
        ("trace", &item.trace),
    ]
    .into_iter()
    .filter_map(|(method, operation)| operation.as_ref().map(|operation| (method, operation)))
}

fn operations_mut(item: &mut PathItem) -> impl Iterator<Item = (&'static str, &mut Operation)> {
    [
        ("get", &mut item.get),
        ("put", &mut item.put),
        ("post", &mut item.post),
        ("delete", &mut item.delete),
        ("options", &mut item.options),
        ("head", &mut item.head),
        ("patch", &mut item.patch),
        ("trace", &mut item.trace),
    ]
    .into_iter()
    .filter_map(|(method, operation)| operation.as_mut().map(|operation| (method, operation)))
}

#[cfg(test)]
mod tests {

    fn item(get: serde_json::Value, head: serde_json::Value) -> PathItem {
        let operation = |permissions: serde_json::Value| {
            let mut operation = json!({"responses": {}});
            if !permissions.is_null() {
                operation["x-permissions"] = permissions;
            }
            operation
        };
        serde_json::from_value(json!({"get": operation(get), "head": operation(head)})).unwrap()
    }

    #[test]
    fn head_khong_mo_hon_get() {
        // Cả hai công khai, hay HEAD hẹp bằng / hẹp hơn GET: được.
        assert!(head_problems("/p", &item(json!(null), json!(null))).is_none());
        assert!(
            head_problems("/p", &item(json!(["m.read", "m.list"]), json!(["m.read"]))).is_none()
        );
        // GET cần permission mà HEAD công khai, hay HEAD nhận permission GET không nhận: lỗi.
        assert!(head_problems("/p", &item(json!(["m.read"]), json!(null))).is_some());
        assert!(head_problems("/p", &item(json!(["m.read"]), json!([]))).is_some());
        assert!(head_problems("/p", &item(json!(["m.read"]), json!(["m.peek"]))).is_some());
    }

    use serde_json::json;

    use super::*;

    #[test]
    fn bat_ref_tro_vao_khoang_khong() {
        let document = json!({
            "paths": {"/x": {"get": {"parameters": [
                {"schema": {"$ref": "#/components/schemas/Order"}},
                {"schema": {"$ref": "#/components/schemas/Page"}},
            ]}}},
            "components": {"schemas": {"Page": {"type": "object"}}},
        });
        let problems = dangling_refs(&document);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("#/components/schemas/Order"), "{problems:?}");
    }

    #[test]
    fn bat_mo_ta_tieng_viet() {
        let document = json!({"description": "Đầu vào của tool", "properties": {
            "a": {"description": "Maximum number of items, café"},
            "b": {"title": "Tên người dùng"},
            "description": {"type": "string"},
        }});
        let problems = untranslated_text(&document, "tool `x`");
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(!looks_vietnamese("Resume naïve café über"));
    }
}
