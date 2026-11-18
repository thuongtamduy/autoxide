//! So hình dạng schema hai bản (giải `$ref`, nullable, `allOf`) và so từng operation: tham số,
//! thân request, response.

use super::*;

pub(super) fn shape<'a>(doc: &'a Value, value: &'a Value, depth: usize) -> Shape<'a> {
    let mut out = Shape::default();
    merge(doc, value, &mut out, depth);
    out
}

/// Gộp `value` vào `out` (giải `$ref`, `allOf`, nullable).
pub(super) fn merge<'a>(doc: &'a Value, value: &'a Value, out: &mut Shape<'a>, depth: usize) {
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

pub(super) fn is_null(value: &Value) -> bool {
    value.get("type").and_then(Value::as_str) == Some("null")
}

/// `#/components/schemas/X` → schema `X`.
pub(super) fn resolve<'a>(doc: &'a Value, reference: &str) -> Option<&'a Value> {
    let pointer = reference.strip_prefix('#')?;
    doc.pointer(pointer)
}

/// `true` nếu mọi kiểu trong `inner` nằm trong `outer` (`integer` nằm trong `number`; rỗng là mọi
/// kiểu).
pub(super) fn types_within(inner: &BTreeSet<&str>, outer: &BTreeSet<&str>) -> bool {
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

pub(super) fn describe_types(shape: &Shape<'_>) -> String {
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

/// `content` của thân request / response: media type → schema.
pub(super) fn content(value: &Value) -> BTreeMap<&str, Option<&Value>> {
    value
        .get("content")
        .and_then(Value::as_object)
        .map(|content| {
            content.iter().map(|(media, entry)| (media.as_str(), entry.get("schema"))).collect()
        })
        .unwrap_or_default()
}

/// Một node schema đã giải `$ref`, gộp `allOf`, tách nullable — chỉ một mức (mức con giải khi so).
#[derive(Debug, Default)]
pub(super) struct Shape<'a> {
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

pub(super) const UPPER: [&str; 3] = ["maxLength", "maximum", "maxItems"];

pub(super) const LOWER: [&str; 3] = ["minLength", "minimum", "minItems"];

pub(super) struct Comparison<'a, 'r> {
    pub(super) old_doc: &'a Value,
    pub(super) new_doc: &'a Value,
    pub(super) report: &'r mut Report,
    pub(super) visited: HashSet<(Vec<&'a str>, Vec<&'a str>, Direction)>,
}

impl<'a> Comparison<'a, '_> {
    pub(super) fn operation(&mut self, label: &str, old: &'a Value, new: &'a Value) {
        self.parameters(label, old, new);
        self.request_body(label, old, new);
        self.responses(label, old, new);
    }

    pub(super) fn parameters(&mut self, label: &str, old: &'a Value, new: &'a Value) {
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

    pub(super) fn request_body(&mut self, label: &str, old: &'a Value, new: &'a Value) {
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

    pub(super) fn responses(&mut self, label: &str, old: &'a Value, new: &'a Value) {
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

    pub(super) fn deref(&self, doc: &'a Value, value: &'a Value) -> &'a Value {
        match value.get("$ref").and_then(Value::as_str) {
            Some(reference) => resolve(doc, reference).unwrap_or(value),
            None => value,
        }
    }

    /// So hai schema ở vị trí `at` theo chiều `direction`.
    pub(super) fn schema(
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

    pub(super) fn variants(
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

    pub(super) fn kinds(
        &mut self,
        at: &str,
        old: &Shape<'a>,
        new: &Shape<'a>,
        direction: Direction,
    ) {
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

    pub(super) fn enums(
        &mut self,
        at: &str,
        old: &Shape<'a>,
        new: &Shape<'a>,
        direction: Direction,
    ) {
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

    pub(super) fn constraints(&mut self, at: &str, old: &Shape<'a>, new: &Shape<'a>) {
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

    pub(super) fn properties(
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
