//! `cargo xtask api-diff [--base <ref>]`: chặn thay đổi phá vỡ của `/api/v1` (D-059, đặc tả 31 A1).
//!
//! So `docs/openapi.json` trên đĩa với bản ở tag phát hành gần nhất (tổ tiên của `HEAD`). Mỗi thay
//! đổi phá vỡ có một mã ổn định (`operation.removed`, `response.property_removed`…) và một câu nói
//! cách làm đúng. Thay đổi không phá vỡ in `note:`; thay đổi có thể làm client chặt vỡ in `warning:`.
//!
//! Cách so:
//! - Operation ghép theo (method, đường dẫn với tên tham số thay bằng `{}`): đổi tên `{id}` không
//!   phá client.
//! - Schema so theo **hình dạng**, giải `$ref` hai bên: utoipa đổi tên component không phá client.
//!   `$ref` vòng lặp dừng nhờ tập cặp (ref cũ, ref mới, chiều) đã thăm.
//! - Hai chiều: *request* (bản mới phải nhận mọi thứ bản cũ nhận) và *response* (bản mới chỉ trả thứ
//!   bản cũ đã hứa).
//! - Nullable hai cách viết (`type: [X, "null"]`, `oneOf: [X, {type: null}]`) là một; `allOf` gộp.
//! - Lỗi (4xx, 5xx, `default`): `application/json` và `application/problem+json` là một (B4).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt;
use std::path::Path;
use std::process::Command;

use chrono::NaiveDate;
use serde_json::Value;

/// File ngoại lệ có chủ đích, ở gốc `xtask/`.
const ALLOW_FILE: &str = "xtask/api-breaking-allow.json";

const METHODS: [&str; 8] = ["get", "put", "post", "delete", "options", "head", "patch", "trace"];

/// Mức của một phát hiện.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Breaking,
    Warning,
    Note,
}

/// Một thay đổi tìm thấy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub level: Level,
    /// Mã ổn định, vd `response.property_removed`; rỗng với `note`.
    pub rule: &'static str,
    /// Chỗ thay đổi: `GET /api/v1/x 200 $.data.slug`.
    pub at: String,
    pub message: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.level {
            Level::Breaking => write!(
                f,
                "[api-breaking] {}: {}\n  {}",
                self.rule,
                self.at,
                hint(self.rule, &self.message)
            ),
            Level::Warning => write!(f, "warning: {}: {} — {}", self.rule, self.at, self.message),
            Level::Note => write!(f, "note: {} — {}", self.at, self.message),
        }
    }
}

/// Câu "làm đúng thế nào" cho từng loại phá vỡ.
fn hint(rule: &str, detail: &str) -> String {
    let fix = match rule {
        "operation.removed" => {
            "Keep it working; declare it in Module::deprecations with a sunset date and add the \
             replacement as a new endpoint (CLAUDE.md, architecture rule 6; D-059). It may be \
             removed after the sunset date."
        }
        "parameter.added_required" | "parameter.became_required" => {
            "Clients built for the old version do not send it. Make it optional with a default, or \
             add a new endpoint."
        }
        "parameter.removed" => {
            "The server would silently ignore what clients send. Keep accepting it, or add a new \
             endpoint and deprecate this one."
        }
        "request.body_required" | "request.property_required" => {
            "Clients built for the old version do not send it. Make it optional, or add a new \
             endpoint."
        }
        "request.property_removed" => {
            "The body refuses unknown fields, so clients that send it now fail. Keep accepting it \
             (and ignore it), or add a new endpoint."
        }
        "request.media_type_removed" | "response.media_type_removed" => {
            "Keep serving this media type, or add a new endpoint."
        }
        "request.type_narrowed" | "request.constraint_tightened" => {
            "Values the old version accepted are now refused. Keep accepting them, or add a new \
             endpoint."
        }
        "response.status_removed" => {
            "Clients check this status. Keep returning it, or add a new endpoint."
        }
        "response.property_removed" => {
            "Clients read this field. Keep returning it (mark it deprecated in its description) \
             and add the new field."
        }
        "response.property_optional" => {
            "Clients rely on this field always being there and not null. Keep it required, or add \
             a new field."
        }
        "response.type_changed" => {
            "Clients parse this value as before. Keep its type and format, or add a new field."
        }
        "enum.value_removed" => {
            "Clients send or expect this value. Keep it (mark it deprecated in the description)."
        }
        "schema.composite_changed" => {
            "The variants of a oneOf / anyOf changed in a way that cannot be shown to be safe. \
             Keep the old variants, or add a new endpoint."
        }
        _ => "See docs/steps/31-api-contract.md (A1).",
    };
    format!("{detail}. {fix}")
}

/// Kết quả so hai bản.
#[derive(Debug, Default)]
pub struct Report {
    pub findings: Vec<Finding>,
}

impl Report {
    pub fn breaking(&self) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(|finding| finding.level == Level::Breaking)
    }

    fn push(
        &mut self,
        level: Level,
        rule: &'static str,
        at: impl Into<String>,
        message: impl Into<String>,
    ) {
        let finding = Finding { level, rule, at: at.into(), message: message.into() };
        if !self.findings.contains(&finding) {
            self.findings.push(finding);
        }
    }

    fn breaking_at(
        &mut self,
        rule: &'static str,
        at: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.push(Level::Breaking, rule, at, message);
    }
}

/// Chiều so schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Direction {
    /// Bản mới phải nhận mọi thứ bản cũ nhận.
    Request,
    /// Bản mới chỉ trả thứ bản cũ đã hứa.
    Response,
}

/// So hai tài liệu OpenAPI. `today`: ngày (UTC) để xét sunset.
pub fn diff(old: &Value, new: &Value, today: NaiveDate) -> Report {
    let mut report = Report::default();
    let old_ops = operations(old);
    let new_ops = operations(new);
    for (key, old_op) in &old_ops {
        let label = format!("{} {}", key.0.to_uppercase(), old_op.path);
        match new_ops.get(key) {
            None => {
                if let Some(sunset) = sunset_of(old_op.operation)
                    && sunset <= today
                {
                    report.push(
                        Level::Note,
                        "",
                        label,
                        format!("removed after its sunset date {sunset}"),
                    );
                } else {
                    report.breaking_at(
                        "operation.removed",
                        label,
                        "This operation of the base version is gone",
                    );
                }
            }
            Some(new_op) => {
                let mut compare = Comparison {
                    old_doc: old,
                    new_doc: new,
                    report: &mut report,
                    visited: HashSet::new(),
                };
                compare.operation(&label, old_op.operation, new_op.operation);
            }
        }
    }
    for (key, new_op) in &new_ops {
        if !old_ops.contains_key(key) {
            report.push(
                Level::Note,
                "",
                format!("{} {}", key.0.to_uppercase(), new_op.path),
                "new operation",
            );
        }
    }
    report
}

struct Op<'a> {
    path: &'a str,
    operation: &'a Value,
}

/// Operation trong phạm vi so (`/api/v1/…`, `/health/…`), theo (method, đường dẫn chuẩn hoá).
fn operations(doc: &Value) -> BTreeMap<(String, String), Op<'_>> {
    let mut found = BTreeMap::new();
    let Some(paths) = doc.get("paths").and_then(Value::as_object) else {
        return found;
    };
    for (path, item) in paths {
        if !(path.starts_with("/api/v1/") || path == "/api/v1" || path.starts_with("/health/")) {
            continue;
        }
        for method in METHODS {
            if let Some(operation) = item.get(method) {
                found.insert((method.to_owned(), normalize_path(path)), Op { path, operation });
            }
        }
    }
    found
}

/// `/a/{id}/b/{name}` → `/a/{}/b/{}`.
fn normalize_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut inside = false;
    for c in path.chars() {
        match c {
            '{' => {
                inside = true;
                out.push_str("{}");
            }
            '}' => inside = false,
            _ if inside => {}
            _ => out.push(c),
        }
    }
    out
}

/// Ngày sunset của operation deprecated: `x-sunset`, hay câu "It will be removed on YYYY-MM-DD."
/// trong `description` (bản cũ chưa có extension).
fn sunset_of(operation: &Value) -> Option<NaiveDate> {
    if operation.get("deprecated").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    if let Some(date) = operation.get("x-sunset").and_then(Value::as_str) {
        return NaiveDate::parse_from_str(date, "%Y-%m-%d").ok();
    }
    let description = operation.get("description").and_then(Value::as_str)?;
    let (_, rest) = description.split_once("removed on ")?;
    NaiveDate::parse_from_str(rest.get(..10)?, "%Y-%m-%d").ok()
}

/// Một node schema đã giải `$ref`, gộp `allOf`, tách nullable — chỉ một mức (mức con giải khi so).
#[derive(Debug, Default)]
struct Shape<'a> {
    /// Các component đã đi qua để tới node này (chống vòng lặp).
    refs: Vec<&'a str>,
    /// Rỗng: không ràng buộc kiểu.
    types: BTreeSet<&'a str>,
    nullable: bool,
    format: Option<&'a str>,
    enum_values: Option<Vec<&'a Value>>,
    properties: BTreeMap<&'a str, &'a Value>,
    required: BTreeSet<&'a str>,
    /// `additionalProperties: false`.
    closed: bool,
    items: Option<&'a Value>,
    additional: Option<&'a Value>,
    variants: Option<(&'static str, Vec<&'a Value>)>,
    numbers: BTreeMap<&'static str, f64>,
    pattern: Option<&'a str>,
}

const UPPER: [&str; 3] = ["maxLength", "maximum", "maxItems"];
const LOWER: [&str; 3] = ["minLength", "minimum", "minItems"];

fn shape<'a>(doc: &'a Value, value: &'a Value, depth: usize) -> Shape<'a> {
    let mut out = Shape::default();
    merge(doc, value, &mut out, depth);
    out
}

/// Gộp `value` vào `out` (giải `$ref`, `allOf`, nullable).
fn merge<'a>(doc: &'a Value, value: &'a Value, out: &mut Shape<'a>, depth: usize) {
    if depth > 64 {
        return;
    }
    let Some(object) = value.as_object() else {
        return;
    };
    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        if out.refs.contains(&reference) {
            return;
        }
        out.refs.push(reference);
        if let Some(target) = resolve(doc, reference) {
            merge(doc, target, out, depth + 1);
        }
    }
    if let Some(parts) = object.get("allOf").and_then(Value::as_array) {
        for part in parts {
            merge(doc, part, out, depth + 1);
        }
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(parts) = object.get(key).and_then(Value::as_array) {
            let (nulls, rest): (Vec<&Value>, Vec<&Value>) =
                parts.iter().partition(|part| is_null(part));
            if !nulls.is_empty() {
                out.nullable = true;
            }
            if rest.len() == 1 {
                merge(doc, rest[0], out, depth + 1);
            } else if !rest.is_empty() {
                out.variants = Some((if key == "oneOf" { "oneOf" } else { "anyOf" }, rest));
            }
        }
    }
    match object.get("type") {
        Some(Value::String(kind)) if kind == "null" => out.nullable = true,
        Some(Value::String(kind)) => {
            out.types.insert(kind);
        }
        Some(Value::Array(kinds)) => {
            for kind in kinds.iter().filter_map(Value::as_str) {
                if kind == "null" {
                    out.nullable = true;
                } else {
                    out.types.insert(kind);
                }
            }
        }
        _ => {}
    }
    if let Some(format) = object.get("format").and_then(Value::as_str) {
        out.format = Some(format);
    }
    if let Some(values) = object.get("enum").and_then(Value::as_array) {
        let values: Vec<&Value> = values.iter().filter(|value| !value.is_null()).collect();
        if values.len() < object.get("enum").and_then(Value::as_array).map_or(0, Vec::len) {
            out.nullable = true;
        }
        out.enum_values = Some(values);
    }
    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        for (name, schema) in properties {
            out.properties.insert(name, schema);
        }
    }
    if let Some(required) = object.get("required").and_then(Value::as_array) {
        out.required.extend(required.iter().filter_map(Value::as_str));
    }
    match object.get("additionalProperties") {
        Some(Value::Bool(false)) => out.closed = true,
        Some(schema @ Value::Object(_)) => out.additional = Some(schema),
        _ => {}
    }
    if let Some(items) = object.get("items") {
        out.items = Some(items);
    }
    for key in UPPER.iter().chain(LOWER.iter()) {
        if let Some(number) = object.get(*key).and_then(Value::as_f64) {
            out.numbers.insert(key, number);
        }
    }
    if let Some(pattern) = object.get("pattern").and_then(Value::as_str) {
        out.pattern = Some(pattern);
    }
}

fn is_null(value: &Value) -> bool {
    value.get("type").and_then(Value::as_str) == Some("null")
}

/// `#/components/schemas/X` → schema `X`.
fn resolve<'a>(doc: &'a Value, reference: &str) -> Option<&'a Value> {
    let pointer = reference.strip_prefix('#')?;
    doc.pointer(pointer)
}

/// `true` nếu mọi kiểu trong `inner` nằm trong `outer` (`integer` nằm trong `number`; rỗng là mọi
/// kiểu).
fn types_within(inner: &BTreeSet<&str>, outer: &BTreeSet<&str>) -> bool {
    if outer.is_empty() {
        return true;
    }
    if inner.is_empty() {
        return false;
    }
    inner
        .iter()
        .all(|kind| outer.contains(kind) || (*kind == "integer" && outer.contains("number")))
}

fn describe_types(shape: &Shape<'_>) -> String {
    let mut kinds: Vec<&str> = shape.types.iter().copied().collect();
    if kinds.is_empty() {
        kinds.push("any");
    }
    if shape.nullable {
        kinds.push("null");
    }
    let mut text = kinds.join(" | ");
    if let Some(format) = shape.format {
        text.push_str(&format!(" ({format})"));
    }
    text
}

struct Comparison<'a, 'r> {
    old_doc: &'a Value,
    new_doc: &'a Value,
    report: &'r mut Report,
    visited: HashSet<(Vec<&'a str>, Vec<&'a str>, Direction)>,
}

impl<'a> Comparison<'a, '_> {
    fn operation(&mut self, label: &str, old: &'a Value, new: &'a Value) {
        self.parameters(label, old, new);
        self.request_body(label, old, new);
        self.responses(label, old, new);
    }

    fn parameters(&mut self, label: &str, old: &'a Value, new: &'a Value) {
        let collect = |doc: &'a Value, operation: &'a Value| {
            let mut found: BTreeMap<(String, String), &'a Value> = BTreeMap::new();
            let mut path_index = 0;
            for parameter in
                operation.get("parameters").and_then(Value::as_array).into_iter().flatten()
            {
                let parameter = match parameter.get("$ref").and_then(Value::as_str) {
                    Some(reference) => resolve(doc, reference).unwrap_or(parameter),
                    None => parameter,
                };
                let place = parameter.get("in").and_then(Value::as_str).unwrap_or_default();
                let name = parameter.get("name").and_then(Value::as_str).unwrap_or_default();
                // Tham số đường dẫn ghép theo vị trí (đổi tên không phá); header không phân biệt hoa
                // thường.
                let key = match place {
                    "path" => {
                        path_index += 1;
                        format!("#{path_index}")
                    }
                    "header" => name.to_ascii_lowercase(),
                    _ => name.to_owned(),
                };
                found.insert((place.to_owned(), key), parameter);
            }
            found
        };
        let old_params = collect(self.old_doc, old);
        let new_params = collect(self.new_doc, new);
        let required =
            |parameter: &Value| parameter.get("required").and_then(Value::as_bool) == Some(true);
        let name = |parameter: &Value| {
            parameter.get("name").and_then(Value::as_str).unwrap_or_default().to_owned()
        };
        for (key, old_param) in &old_params {
            let at = format!("{label} {} {}", key.0, name(old_param));
            match new_params.get(key) {
                None if key.0 == "path" => {}
                None => self.report.breaking_at(
                    "parameter.removed",
                    at,
                    "This parameter of the base version is gone",
                ),
                Some(new_param) => {
                    if required(new_param) && !required(old_param) {
                        self.report.breaking_at(
                            "parameter.became_required",
                            at.clone(),
                            "This optional parameter is now required",
                        );
                    }
                    if let (Some(old_schema), Some(new_schema)) =
                        (old_param.get("schema"), new_param.get("schema"))
                    {
                        self.schema(&at, old_schema, new_schema, Direction::Request, 0);
                    }
                }
            }
        }
        for (key, new_param) in &new_params {
            if !old_params.contains_key(key) && required(new_param) {
                self.report.breaking_at(
                    "parameter.added_required",
                    format!("{label} {} {}", key.0, name(new_param)),
                    "A new required parameter",
                );
            }
        }
    }

    fn request_body(&mut self, label: &str, old: &'a Value, new: &'a Value) {
        let body = |doc: &'a Value, operation: &'a Value| {
            operation.get("requestBody").map(|body| {
                match body.get("$ref").and_then(Value::as_str) {
                    Some(reference) => resolve(doc, reference).unwrap_or(body),
                    None => body,
                }
            })
        };
        let (old_body, new_body) = (body(self.old_doc, old), body(self.new_doc, new));
        let required = |body: Option<&Value>| {
            body.and_then(|body| body.get("required")).and_then(Value::as_bool) == Some(true)
        };
        if required(new_body) && !required(old_body) {
            self.report.breaking_at(
                "request.body_required",
                format!("{label} body"),
                "The request body is now required",
            );
        }
        let (Some(old_body), Some(new_body)) = (old_body, new_body) else {
            return;
        };
        let (old_content, new_content) = (content(old_body), content(new_body));
        for (media, old_schema) in &old_content {
            match new_content.get(media) {
                None => self.report.breaking_at(
                    "request.media_type_removed",
                    format!("{label} body {media}"),
                    "The request body no longer accepts this media type",
                ),
                Some(new_schema) => {
                    if let (Some(old_schema), Some(new_schema)) = (old_schema, new_schema) {
                        self.schema(
                            &format!("{label} body $"),
                            old_schema,
                            new_schema,
                            Direction::Request,
                            0,
                        );
                    }
                }
            }
        }
    }

    fn responses(&mut self, label: &str, old: &'a Value, new: &'a Value) {
        let (Some(old_responses), Some(new_responses)) = (
            old.get("responses").and_then(Value::as_object),
            new.get("responses").and_then(Value::as_object),
        ) else {
            return;
        };
        for (status, old_response) in old_responses {
            let success = status.starts_with('2') || status.starts_with('3');
            let at = format!("{label} {status}");
            let Some(new_response) = new_responses.get(status) else {
                if success {
                    self.report.breaking_at(
                        "response.status_removed",
                        at,
                        "This status of the base version is no longer returned",
                    );
                }
                continue;
            };
            let old_response = self.deref(self.old_doc, old_response);
            let new_response = self.deref(self.new_doc, new_response);
            let (old_content, new_content) = (content(old_response), content(new_response));
            if success {
                for (media, old_schema) in &old_content {
                    match new_content.get(media) {
                        None => self.report.breaking_at(
                            "response.media_type_removed",
                            format!("{at} {media}"),
                            "This media type is no longer returned",
                        ),
                        Some(new_schema) => {
                            if let (Some(old_schema), Some(new_schema)) = (old_schema, new_schema) {
                                self.schema(
                                    &format!("{at} $"),
                                    old_schema,
                                    new_schema,
                                    Direction::Response,
                                    0,
                                );
                            }
                        }
                    }
                }
            } else {
                // Lỗi luôn là Problem: hai media type JSON là một (B4).
                let first = |content: &BTreeMap<&'a str, Option<&'a Value>>| {
                    ["application/problem+json", "application/json"]
                        .iter()
                        .find_map(|media| content.get(media).copied().flatten())
                };
                if let (Some(old_schema), Some(new_schema)) =
                    (first(&old_content), first(&new_content))
                {
                    self.schema(&format!("{at} $"), old_schema, new_schema, Direction::Response, 0);
                }
            }
        }
    }

    fn deref(&self, doc: &'a Value, value: &'a Value) -> &'a Value {
        match value.get("$ref").and_then(Value::as_str) {
            Some(reference) => resolve(doc, reference).unwrap_or(value),
            None => value,
        }
    }

    /// So hai schema ở vị trí `at` theo chiều `direction`.
    fn schema(
        &mut self,
        at: &str,
        old: &'a Value,
        new: &'a Value,
        direction: Direction,
        depth: usize,
    ) {
        if depth > 64 {
            return;
        }
        let old_shape = shape(self.old_doc, old, 0);
        let new_shape = shape(self.new_doc, new, 0);
        if !old_shape.refs.is_empty() || !new_shape.refs.is_empty() {
            let key = (old_shape.refs.clone(), new_shape.refs.clone(), direction);
            if !self.visited.insert(key) {
                return;
            }
        }
        self.variants(at, &old_shape, &new_shape, direction, depth);
        self.kinds(at, &old_shape, &new_shape, direction);
        self.enums(at, &old_shape, &new_shape, direction);
        if direction == Direction::Request {
            self.constraints(at, &old_shape, &new_shape);
        }
        self.properties(at, &old_shape, &new_shape, direction, depth);
        if let (Some(old_items), Some(new_items)) = (old_shape.items, new_shape.items) {
            self.schema(&format!("{at}[]"), old_items, new_items, direction, depth + 1);
        }
        if let (Some(old_values), Some(new_values)) = (old_shape.additional, new_shape.additional) {
            self.schema(&format!("{at}{{*}}"), old_values, new_values, direction, depth + 1);
        }
    }

    fn variants(
        &mut self,
        at: &str,
        old: &Shape<'a>,
        new: &Shape<'a>,
        direction: Direction,
        depth: usize,
    ) {
        match (&old.variants, &new.variants) {
            (None, None) => {}
            (Some((old_kind, old_parts)), Some((new_kind, new_parts)))
                if old_kind == new_kind && old_parts.len() == new_parts.len() =>
            {
                for (index, (old_part, new_part)) in old_parts.iter().zip(new_parts).enumerate() {
                    self.schema(
                        &format!("{at}<{old_kind}[{index}]>"),
                        old_part,
                        new_part,
                        direction,
                        depth + 1,
                    );
                }
            }
            _ => self.report.breaking_at(
                "schema.composite_changed",
                at.to_owned(),
                "The oneOf / anyOf variants differ",
            ),
        }
    }

    fn kinds(&mut self, at: &str, old: &Shape<'a>, new: &Shape<'a>, direction: Direction) {
        let format_changed =
            old.format != new.format && !(old.format.is_none() && direction == Direction::Response);
        match direction {
            Direction::Request => {
                let narrowed =
                    !types_within(&old.types, &new.types) || (old.nullable && !new.nullable);
                // Thêm `format` cho trường request cũ không có: cũng là thu hẹp.
                if narrowed || (format_changed && !(new.format.is_none())) {
                    self.report.breaking_at(
                        "request.type_narrowed",
                        at.to_owned(),
                        format!("Was {}, now {}", describe_types(old), describe_types(new)),
                    );
                }
            }
            Direction::Response => {
                if new.nullable && !old.nullable {
                    self.report.breaking_at(
                        "response.property_optional",
                        at.to_owned(),
                        "This value may now be null",
                    );
                }
                if !types_within(&new.types, &old.types) || format_changed {
                    self.report.breaking_at(
                        "response.type_changed",
                        at.to_owned(),
                        format!("Was {}, now {}", describe_types(old), describe_types(new)),
                    );
                }
            }
        }
    }

    fn enums(&mut self, at: &str, old: &Shape<'a>, new: &Shape<'a>, direction: Direction) {
        match (&old.enum_values, &new.enum_values) {
            (Some(old_values), Some(new_values)) => {
                for value in old_values.iter().filter(|value| !new_values.contains(value)) {
                    self.report.breaking_at(
                        "enum.value_removed",
                        at.to_owned(),
                        format!("The value {value} is gone"),
                    );
                }
                for value in new_values.iter().filter(|value| !old_values.contains(value)) {
                    match direction {
                        Direction::Request => self.report.push(
                            Level::Note,
                            "",
                            at.to_owned(),
                            format!("new accepted value {value}"),
                        ),
                        Direction::Response => self.report.push(
                            Level::Warning,
                            "enum.value_added",
                            at.to_owned(),
                            format!("may now return {value}; strict clients may fail on it"),
                        ),
                    }
                }
            }
            (None, Some(_)) if direction == Direction::Request => self.report.breaking_at(
                "request.constraint_tightened",
                at.to_owned(),
                "Only a fixed list of values is accepted now",
            ),
            (Some(_), None) if direction == Direction::Response => self.report.push(
                Level::Warning,
                "enum.removed",
                at.to_owned(),
                "may now return values outside the old list",
            ),
            _ => {}
        }
    }

    fn constraints(&mut self, at: &str, old: &Shape<'a>, new: &Shape<'a>) {
        for key in UPPER {
            if let Some(limit) = new.numbers.get(key)
                && old.numbers.get(key).is_none_or(|old_limit| limit < old_limit)
            {
                self.report.breaking_at(
                    "request.constraint_tightened",
                    at.to_owned(),
                    format!("{key} is now {limit}"),
                );
            }
        }
        for key in LOWER {
            if let Some(limit) = new.numbers.get(key)
                && old.numbers.get(key).is_none_or(|old_limit| limit > old_limit)
            {
                self.report.breaking_at(
                    "request.constraint_tightened",
                    at.to_owned(),
                    format!("{key} is now {limit}"),
                );
            }
        }
        if new.pattern.is_some() && new.pattern != old.pattern {
            self.report.breaking_at(
                "request.constraint_tightened",
                at.to_owned(),
                "The pattern changed",
            );
        }
    }

    fn properties(
        &mut self,
        at: &str,
        old: &Shape<'a>,
        new: &Shape<'a>,
        direction: Direction,
        depth: usize,
    ) {
        for (name, old_schema) in &old.properties {
            let here = format!("{at}.{name}");
            match new.properties.get(name) {
                None => match direction {
                    Direction::Response => self.report.breaking_at(
                        "response.property_removed",
                        here,
                        "This field is no longer returned",
                    ),
                    Direction::Request if new.closed => self.report.breaking_at(
                        "request.property_removed",
                        here,
                        "This field is no longer accepted",
                    ),
                    Direction::Request => {}
                },
                Some(new_schema) => {
                    match direction {
                        Direction::Request
                            if new.required.contains(name) && !old.required.contains(name) =>
                        {
                            self.report.breaking_at(
                                "request.property_required",
                                here.clone(),
                                "This optional field is now required",
                            );
                        }
                        Direction::Response
                            if old.required.contains(name) && !new.required.contains(name) =>
                        {
                            self.report.breaking_at(
                                "response.property_optional",
                                here.clone(),
                                "This field may now be missing",
                            );
                        }
                        _ => {}
                    }
                    self.schema(&here, old_schema, new_schema, direction, depth + 1);
                }
            }
        }
        for name in new.properties.keys().filter(|name| !old.properties.contains_key(*name)) {
            let here = format!("{at}.{name}");
            if direction == Direction::Request && new.required.contains(name) {
                self.report.breaking_at("request.property_required", here, "A new required field");
            } else {
                self.report.push(Level::Note, "", here, "new field");
            }
        }
    }
}

/// `content` của thân request / response: media type → schema.
fn content(value: &Value) -> BTreeMap<&str, Option<&Value>> {
    value
        .get("content")
        .and_then(Value::as_object)
        .map(|content| {
            content.iter().map(|(media, entry)| (media.as_str(), entry.get("schema"))).collect()
        })
        .unwrap_or_default()
}

/// Một ngoại lệ có chủ đích (`xtask/api-breaking-allow.json`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Allowed {
    pub base: String,
    pub rule: String,
    pub at: String,
    pub decision: Option<String>,
}

/// Bỏ phát hiện phá vỡ có ngoại lệ cho đúng `base`; lỗi khi ngoại lệ thiếu `decision`.
pub fn apply_allowlist(
    report: &mut Report,
    allowed: &[Allowed],
    base: &str,
) -> Result<usize, String> {
    if let Some(entry) =
        allowed.iter().find(|entry| entry.decision.as_deref().is_none_or(str::is_empty))
    {
        return Err(format!(
            "{ALLOW_FILE}: the entry for `{} {}` has no `decision` (D-NNN explaining why the break is intended)",
            entry.rule, entry.at
        ));
    }
    let before = report.findings.len();
    report.findings.retain(|finding| {
        !(finding.level == Level::Breaking
            && allowed.iter().any(|entry| {
                entry.base == base && entry.rule == finding.rule && entry.at == finding.at
            }))
    });
    Ok(before - report.findings.len())
}

/// Lệnh `cargo xtask api-diff`.
pub fn run(root: &Path, base: Option<&str>, today: NaiveDate) -> Result<bool, String> {
    let base = match base {
        Some(base) => base.to_owned(),
        None => {
            let shallow = git(root, &["rev-parse", "--is-shallow-repository"])?;
            if shallow.trim() == "true" {
                return Err(
                    "the repository is a shallow clone, so the last release tag cannot be \
                            found; fetch the full history (`git fetch --unshallow --tags`, or \
                            `fetch-depth: 0` in actions/checkout)"
                        .to_owned(),
                );
            }
            match git(
                root,
                &["describe", "--tags", "--abbrev=0", "--match", "v[0-9]*.[0-9]*.[0-9]*", "HEAD"],
            ) {
                Ok(tag) => tag.trim().to_owned(),
                Err(_) => {
                    println!("api-diff: no release tag found; nothing to compare");
                    return Ok(true);
                }
            }
        }
    };
    let Ok(old_text) = git(root, &["show", &format!("{base}:docs/openapi.json")]) else {
        println!("api-diff: no docs/openapi.json in {base}; nothing to compare");
        return Ok(true);
    };
    let old: Value = serde_json::from_str(&old_text)
        .map_err(|error| format!("docs/openapi.json in {base} is not JSON: {error}"))?;
    let new_text = std::fs::read_to_string(root.join("docs/openapi.json"))
        .map_err(|error| format!("cannot read docs/openapi.json: {error}"))?;
    let new: Value = serde_json::from_str(&new_text)
        .map_err(|error| format!("docs/openapi.json is not JSON: {error}"))?;
    let mut report = diff(&old, &new, today);
    let allowed = read_allowlist(root)?;
    let skipped = apply_allowlist(&mut report, &allowed, &base)?;
    for finding in report.findings.iter().filter(|finding| finding.level != Level::Breaking) {
        println!("{finding}");
    }
    let breaking: Vec<&Finding> = report.breaking().collect();
    for finding in &breaking {
        eprintln!("{finding}");
    }
    if breaking.is_empty() {
        println!(
            "api-diff: no breaking change to /api/v1 since {base}{}",
            if skipped > 0 {
                format!(" ({skipped} allowed in {ALLOW_FILE})")
            } else {
                String::new()
            }
        );
        Ok(true)
    } else {
        eprintln!(
            "api-diff: {} breaking change(s) to /api/v1 since {base}. An intended break needs an \
             entry in {ALLOW_FILE} with the decision that explains it.",
            breaking.len()
        );
        Ok(false)
    }
}

fn read_allowlist(root: &Path) -> Result<Vec<Allowed>, String> {
    let path = root.join(ALLOW_FILE);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {ALLOW_FILE}: {error}"))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("{ALLOW_FILE} is not a list of entries: {error}"))
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|error| format!("cannot run git: {error}"))?;
    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|error| format!("git printed non-UTF-8: {error}"))
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 15).unwrap()
    }

    /// Tài liệu một operation `method path`, kèm `components.schemas`.
    fn doc(path: &str, method: &str, operation: Value, schemas: Value) -> Value {
        json!({"openapi": "3.1.0", "paths": {path: {method: operation}}, "components": {"schemas": schemas}})
    }

    /// Operation GET trả 200 với thân `schema`.
    fn returns(schema: Value) -> Value {
        json!({"responses": {"200": {"description": "ok", "content": {"application/json": {"schema": schema}}}}})
    }

    /// Operation POST nhận thân `schema` (bắt buộc), trả 201.
    fn accepts(schema: Value) -> Value {
        json!({
            "requestBody": {"required": true, "content": {"application/json": {"schema": schema}}},
            "responses": {"201": {"description": "created"}}
        })
    }

    fn rules(old: &Value, new: &Value) -> Vec<&'static str> {
        diff(old, new, today()).breaking().map(|finding| finding.rule).collect()
    }

    fn get(schema: Value) -> Value {
        doc("/api/v1/x", "get", returns(schema), json!({}))
    }

    fn post(schema: Value) -> Value {
        doc("/api/v1/x", "post", accepts(schema), json!({}))
    }

    fn object(properties: Value, required: &[&str]) -> Value {
        json!({"type": "object", "properties": properties, "required": required})
    }

    #[test]
    fn so_tai_lieu_trong_repo_voi_chinh_no_khong_co_gi() {
        let text =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/openapi.json"))
                .unwrap();
        let document: Value = serde_json::from_str(&text).unwrap();
        let report = diff(&document, &document, today());
        assert!(report.findings.is_empty(), "{:?}", report.findings.first());
    }

    #[test]
    fn bo_operation() {
        let old = get(json!({"type": "string"}));
        let new = doc("/api/v1/y", "get", returns(json!({"type": "string"})), json!({}));
        assert_eq!(rules(&old, &new), ["operation.removed"]);
        let report = diff(&old, &new, today());
        let message = report.breaking().next().unwrap().to_string();
        assert!(message.contains("Module::deprecations"), "{message}");
    }

    #[test]
    fn operation_deprecated_qua_sunset_duoc_bo_chua_toi_thi_do() {
        let deprecated = |sunset: &str| {
            let mut operation = returns(json!({"type": "string"}));
            operation["deprecated"] = json!(true);
            operation["x-sunset"] = json!(sunset);
            doc("/api/v1/x", "get", operation, json!({}))
        };
        let new = doc("/api/v1/y", "get", returns(json!({"type": "string"})), json!({}));
        assert!(rules(&deprecated("2026-11-15"), &new).is_empty(), "sunset today");
        assert_eq!(rules(&deprecated("2026-11-16"), &new), ["operation.removed"]);
        // Bản cũ chưa có `x-sunset`: đọc câu trong mô tả.
        let mut operation = returns(json!({"type": "string"}));
        operation["deprecated"] = json!(true);
        operation["description"] = json!("Old. It will be removed on 2026-01-01. Use y.");
        let old = doc("/api/v1/x", "get", operation, json!({}));
        assert!(rules(&old, &new).is_empty());
    }

    #[test]
    fn doi_ten_tham_so_duong_dan_khong_pha() {
        let operation = |name: &str| {
            let mut operation = returns(json!({"type": "string"}));
            operation["parameters"] = json!([{"name": name, "in": "path", "required": true, "schema": {"type": "string"}}]);
            operation
        };
        let old = doc("/api/v1/x/{id}", "get", operation("id"), json!({}));
        let new = doc("/api/v1/x/{item_id}", "get", operation("item_id"), json!({}));
        assert!(rules(&old, &new).is_empty());
    }

    fn with_params(params: Value) -> Value {
        let mut operation = returns(json!({"type": "string"}));
        operation["parameters"] = params;
        doc("/api/v1/x", "get", operation, json!({}))
    }

    #[test]
    fn tham_so_bat_buoc_moi_tuy_chon_thanh_bat_buoc_va_bo_tham_so() {
        let optional =
            json!([{"name": "q", "in": "query", "required": false, "schema": {"type": "string"}}]);
        let required =
            json!([{"name": "q", "in": "query", "required": true, "schema": {"type": "string"}}]);
        assert_eq!(
            rules(&with_params(json!([])), &with_params(required.clone())),
            ["parameter.added_required"]
        );
        assert_eq!(
            rules(&with_params(optional.clone()), &with_params(required)),
            ["parameter.became_required"]
        );
        assert_eq!(
            rules(&with_params(optional.clone()), &with_params(json!([]))),
            ["parameter.removed"]
        );
        assert!(
            rules(&with_params(json!([])), &with_params(optional)).is_empty(),
            "new optional is fine"
        );
        // Header: không phân biệt hoa thường.
        let upper = json!([{"name": "If-Match", "in": "header", "schema": {"type": "string"}}]);
        let lower = json!([{"name": "if-match", "in": "header", "schema": {"type": "string"}}]);
        assert!(rules(&with_params(upper), &with_params(lower)).is_empty());
    }

    #[test]
    fn than_request_thanh_bat_buoc_va_bo_media_type() {
        let body = |required: bool, media: &str| {
            doc(
                "/api/v1/x",
                "post",
                json!({"requestBody": {"required": required, "content": {media: {"schema": {"type": "object"}}}},
                       "responses": {"201": {"description": "x"}}}),
                json!({}),
            )
        };
        assert_eq!(
            rules(&body(false, "application/json"), &body(true, "application/json")),
            ["request.body_required"]
        );
        assert_eq!(
            rules(&body(true, "application/json"), &body(true, "application/octet-stream")),
            ["request.media_type_removed"]
        );
    }

    #[test]
    fn truong_request_bat_buoc_moi_va_tuy_chon_thanh_bat_buoc() {
        let old = post(object(json!({"a": {"type": "string"}}), &[]));
        let became = post(object(json!({"a": {"type": "string"}}), &["a"]));
        let added = post(object(json!({"a": {"type": "string"}, "b": {"type": "string"}}), &["b"]));
        let optional = post(object(json!({"a": {"type": "string"}, "b": {"type": "string"}}), &[]));
        assert_eq!(rules(&old, &became), ["request.property_required"]);
        assert_eq!(rules(&old, &added), ["request.property_required"]);
        assert!(rules(&old, &optional).is_empty());
    }

    #[test]
    fn bo_truong_request_khi_than_tu_choi_truong_la() {
        let old = post(
            json!({"type": "object", "properties": {"a": {"type": "string"}}, "additionalProperties": false}),
        );
        let new = post(json!({"type": "object", "properties": {}, "additionalProperties": false}));
        assert_eq!(rules(&old, &new), ["request.property_removed"]);
        let open = post(json!({"type": "object", "properties": {}}));
        assert!(rules(&old, &open).is_empty(), "unknown fields are ignored");
    }

    #[test]
    fn kieu_request_hep_lai_va_rang_buoc_chat_hon() {
        let field = |schema: Value| post(object(json!({"a": schema}), &[]));
        assert_eq!(
            rules(&field(json!({"type": ["string", "null"]})), &field(json!({"type": "string"}))),
            ["request.type_narrowed"]
        );
        assert_eq!(
            rules(&field(json!({"type": "integer"})), &field(json!({"type": "string"}))),
            ["request.type_narrowed"]
        );
        assert!(
            rules(&field(json!({"type": "integer"})), &field(json!({"type": "number"}))).is_empty(),
            "widened"
        );
        assert_eq!(
            rules(
                &field(json!({"type": "string"})),
                &field(json!({"type": "string", "format": "uuid"}))
            ),
            ["request.type_narrowed"]
        );
        assert_eq!(
            rules(
                &field(json!({"type": "string", "maxLength": 10})),
                &field(json!({"type": "string", "maxLength": 5}))
            ),
            ["request.constraint_tightened"]
        );
        assert_eq!(
            rules(
                &field(json!({"type": "integer", "minimum": 0})),
                &field(json!({"type": "integer", "minimum": 1}))
            ),
            ["request.constraint_tightened"]
        );
        assert_eq!(
            rules(
                &field(json!({"type": "string"})),
                &field(json!({"type": "string", "pattern": "^a"}))
            ),
            ["request.constraint_tightened"]
        );
        assert!(
            rules(
                &field(json!({"type": "string", "maxLength": 5})),
                &field(json!({"type": "string", "maxLength": 10}))
            )
            .is_empty(),
            "loosened"
        );
    }

    #[test]
    fn response_bo_ma_bo_media_type() {
        let old = doc(
            "/api/v1/x",
            "get",
            json!({"responses": {"200": {"description": "x", "content": {"application/json": {"schema": {"type": "string"}}}}}}),
            json!({}),
        );
        let moved = doc(
            "/api/v1/x",
            "get",
            json!({"responses": {"201": {"description": "x", "content": {"application/json": {"schema": {"type": "string"}}}}}}),
            json!({}),
        );
        let csv = doc(
            "/api/v1/x",
            "get",
            json!({"responses": {"200": {"description": "x", "content": {"text/csv": {"schema": {"type": "string"}}}}}}),
            json!({}),
        );
        assert_eq!(rules(&old, &moved), ["response.status_removed"]);
        assert_eq!(rules(&old, &csv), ["response.media_type_removed"]);
    }

    #[test]
    fn response_bo_truong_tuy_chon_hoa_va_doi_kieu() {
        let field = |schema: Value, required: &[&str]| get(object(json!({"a": schema}), required));
        let string = json!({"type": "string"});
        assert_eq!(
            rules(&field(string.clone(), &["a"]), &get(object(json!({}), &[]))),
            ["response.property_removed"]
        );
        assert_eq!(
            rules(&field(string.clone(), &["a"]), &field(string.clone(), &[])),
            ["response.property_optional"]
        );
        assert_eq!(
            rules(
                &field(string.clone(), &["a"]),
                &field(json!({"type": ["string", "null"]}), &["a"])
            ),
            ["response.property_optional"]
        );
        assert_eq!(
            rules(&field(string.clone(), &["a"]), &field(json!({"type": "integer"}), &["a"])),
            ["response.type_changed"]
        );
        assert_eq!(
            rules(
                &field(json!({"type": "string", "format": "uuid"}), &["a"]),
                &field(string.clone(), &["a"])
            ),
            ["response.type_changed"]
        );
        // Thêm trường response: không phá.
        let more = get(object(json!({"a": string, "b": {"type": "string"}}), &["a", "b"]));
        assert!(rules(&field(json!({"type": "string"}), &["a"]), &more).is_empty());
    }

    #[test]
    fn enum_bo_gia_tri_va_them_gia_tri() {
        let response = |values: Value| get(json!({"type": "string", "enum": values}));
        let request =
            |values: Value| post(object(json!({"a": {"type": "string", "enum": values}}), &[]));
        assert_eq!(
            rules(&response(json!(["a", "b"])), &response(json!(["a"]))),
            ["enum.value_removed"]
        );
        assert_eq!(
            rules(&request(json!(["a", "b"])), &request(json!(["a"]))),
            ["enum.value_removed"]
        );
        assert!(rules(&request(json!(["a"])), &request(json!(["a", "b"]))).is_empty());
        let report = diff(&response(json!(["a"])), &response(json!(["a", "b"])), today());
        assert_eq!(report.breaking().count(), 0);
        assert!(report.findings.iter().any(|finding| finding.level == Level::Warning));
    }

    #[test]
    fn one_of_doi_bien_the_khong_ro_an_toan() {
        let variants = |parts: Value| get(json!({"oneOf": parts}));
        let two = variants(json!([{"type": "string"}, {"type": "integer"}]));
        let three = variants(json!([{"type": "string"}, {"type": "integer"}, {"type": "boolean"}]));
        assert_eq!(rules(&two, &three), ["schema.composite_changed"]);
        assert!(rules(&two, &two).is_empty());
    }

    #[test]
    fn doi_ten_component_cung_hinh_dang_khong_pha() {
        let old = doc(
            "/api/v1/x",
            "get",
            returns(json!({"$ref": "#/components/schemas/Page_Note"})),
            json!({"Page_Note": object(json!({"id": {"type": "string"}}), &["id"])}),
        );
        let new = doc(
            "/api/v1/x",
            "get",
            returns(json!({"$ref": "#/components/schemas/NotePage"})),
            json!({"NotePage": object(json!({"id": {"type": "string"}}), &["id"])}),
        );
        assert!(rules(&old, &new).is_empty());
    }

    #[test]
    fn nullable_hai_cach_viet_va_all_of_tuong_duong() {
        let list = get(object(
            json!({"a": {"type": ["object", "null"], "properties": {"x": {"type": "string"}}, "required": ["x"]}}),
            &["a"],
        ));
        let one_of = doc(
            "/api/v1/x",
            "get",
            returns(object(
                json!({"a": {"oneOf": [{"$ref": "#/components/schemas/A"}, {"type": "null"}]}}),
                &["a"],
            )),
            json!({"A": {"allOf": [{"type": "object", "properties": {"x": {"type": "string"}}}, {"required": ["x"]}]}}),
        );
        assert!(rules(&list, &one_of).is_empty(), "{:?}", diff(&list, &one_of, today()).findings);
        assert!(rules(&one_of, &list).is_empty());
    }

    #[test]
    fn ref_vong_lap_khong_treo() {
        let tree = |name: &str| {
            doc(
                "/api/v1/x",
                "get",
                returns(json!({"$ref": format!("#/components/schemas/{name}")})),
                json!({name: object(json!({"children": {"type": "array", "items": {"$ref": format!("#/components/schemas/{name}")}}}), &["children"])}),
            )
        };
        assert!(rules(&tree("A"), &tree("B")).is_empty());
    }

    #[test]
    fn loi_hai_media_type_json_la_mot() {
        let error = |media: &str| {
            doc(
                "/api/v1/x",
                "get",
                json!({"responses": {"404": {"description": "x", "content": {media: {"schema": {"$ref": "#/components/schemas/Problem"}}}}}}),
                json!({"Problem": object(json!({"code": {"type": "string"}}), &["code"])}),
            )
        };
        assert!(rules(&error("application/json"), &error("application/problem+json")).is_empty());
    }

    #[test]
    fn ngoai_le_chi_hieu_luc_dung_ban_goc_va_can_quyet_dinh() {
        let old = get(object(json!({"a": {"type": "string"}}), &["a"]));
        let new = get(object(json!({}), &[]));
        let entry = |base: &str| Allowed {
            base: base.to_owned(),
            rule: "response.property_removed".to_owned(),
            at: "GET /api/v1/x 200 $.a".to_owned(),
            decision: Some("D-999".to_owned()),
        };
        let mut report = diff(&old, &new, today());
        assert_eq!(
            apply_allowlist(&mut report, &[entry("v0.1.0")], "v0.2.0").unwrap(),
            0,
            "other base"
        );
        assert_eq!(report.breaking().count(), 1);
        assert_eq!(apply_allowlist(&mut report, &[entry("v0.2.0")], "v0.2.0").unwrap(), 1);
        assert_eq!(report.breaking().count(), 0);
        let mut missing = entry("v0.2.0");
        missing.decision = None;
        assert!(apply_allowlist(&mut report, &[missing], "v0.2.0").is_err());
    }

    #[test]
    fn duong_dan_ngoai_api_v1_khong_so() {
        let old = doc("/metrics", "get", returns(json!({"type": "string"})), json!({}));
        let new = doc("/other", "get", returns(json!({"type": "string"})), json!({}));
        assert!(rules(&old, &new).is_empty());
    }
}
