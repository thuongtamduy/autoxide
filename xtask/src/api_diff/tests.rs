use serde_json::json;

use super::*;

fn today() -> Date {
    Date::new(2026, 11, 15).unwrap()
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
        operation["parameters"] =
            json!([{"name": name, "in": "path", "required": true, "schema": {"type": "string"}}]);
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
        rules(&field(string.clone(), &["a"]), &field(json!({"type": ["string", "null"]}), &["a"])),
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
    assert_eq!(rules(&request(json!(["a", "b"])), &request(json!(["a"]))), ["enum.value_removed"]);
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
        decision: Some("D-239".to_owned()),
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

// --- Hợp đồng tool MCP (A6) ---

fn tool(name: &str, input: Value) -> Value {
    json!({
        "name": name,
        "description": "x",
        "permissions": ["notes.write"],
        "annotations": {"read_only": false, "destructive": false, "returns_secret": false,
                        "requires_approval_for_agent": false},
        "input_schema": input,
        "output_schema": null,
        "deprecation": null
    })
}

fn manifest(tools: Vec<Value>) -> Value {
    json!({"manifest_version": 1, "tools": tools})
}

fn input(properties: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false})
}

fn tool_rules(old: Vec<Value>, new: Vec<Value>) -> Vec<&'static str> {
    diff_tools(&manifest(old), &manifest(new), today())
        .breaking()
        .map(|finding| finding.rule)
        .collect()
}

fn plain() -> Value {
    tool("notes.add", input(json!({"title": {"type": "string"}}), &[]))
}

#[test]
fn tool_bi_bo() {
    assert_eq!(tool_rules(vec![plain()], vec![]), ["tool.removed"]);
    let report = diff_tools(&manifest(vec![plain()]), &manifest(vec![]), today());
    assert!(report.breaking().next().unwrap().to_string().contains("Deprecation::tool"));
}

#[test]
fn tool_deprecated_qua_sunset_duoc_bo_chua_toi_thi_do() {
    let deprecated = |sunset: &str| {
        let mut old = plain();
        old["deprecation"] =
            json!({"since": "2026-01-01", "sunset": sunset, "successor": "notes.add_v2"});
        old
    };
    assert!(tool_rules(vec![deprecated("2026-11-15")], vec![]).is_empty());
    assert_eq!(tool_rules(vec![deprecated("2026-12-01")], vec![]), ["tool.removed"]);
}

#[test]
fn tool_them_moi_va_them_tham_so_tuy_chon_thi_qua() {
    let wider = tool(
        "notes.add",
        input(json!({"title": {"type": "string"}, "body": {"type": "string"}}), &[]),
    );
    assert!(tool_rules(vec![plain()], vec![wider]).is_empty());
    assert!(tool_rules(vec![], vec![plain()]).is_empty());
    let mut described = plain();
    described["description"] = json!("Another text");
    assert!(tool_rules(vec![plain()], vec![described]).is_empty());
}

#[test]
fn tool_tham_so_bat_buoc_moi_va_tuy_chon_thanh_bat_buoc() {
    let added = tool(
        "notes.add",
        input(
            json!({"title": {"type": "string"}, "folder_id": {"type": "string"}}),
            &["folder_id"],
        ),
    );
    assert_eq!(tool_rules(vec![plain()], vec![added]), ["tool.parameter_added_required"]);
    let became = tool("notes.add", input(json!({"title": {"type": "string"}}), &["title"]));
    assert_eq!(tool_rules(vec![plain()], vec![became]), ["tool.parameter_became_required"]);
    // Lồng sâu cũng tính.
    let nested = |required: &[&str]| {
        tool(
            "notes.add",
            input(
                json!({"page": {"type": "object", "properties": {"limit": {"type": "integer"}}, "required": required}}),
                &[],
            ),
        )
    };
    assert_eq!(
        tool_rules(vec![nested(&[])], vec![nested(&["limit"])]),
        ["tool.parameter_became_required"]
    );
}

#[test]
fn tool_bo_tham_so_doi_kieu_va_rang_buoc() {
    let empty = tool("notes.add", input(json!({}), &[]));
    assert_eq!(tool_rules(vec![plain()], vec![empty]), ["tool.parameter_removed"]);
    let integer = tool("notes.add", input(json!({"title": {"type": "integer"}}), &[]));
    assert_eq!(tool_rules(vec![plain()], vec![integer]), ["tool.parameter_type_changed"]);
    let short = tool("notes.add", input(json!({"title": {"type": "string", "maxLength": 5}}), &[]));
    assert_eq!(tool_rules(vec![plain()], vec![short]), ["tool.constraint_tightened"]);
    let fixed = |values: Value| {
        tool("notes.add", input(json!({"title": {"type": "string", "enum": values}}), &[]))
    };
    assert_eq!(
        tool_rules(vec![fixed(json!(["a", "b"]))], vec![fixed(json!(["a"]))]),
        ["tool.constraint_tightened"]
    );
    assert!(tool_rules(vec![fixed(json!(["a"]))], vec![fixed(json!(["a", "b"]))]).is_empty());
}

#[test]
fn tool_khong_con_chi_doc_va_them_duyet() {
    let mut read_only = plain();
    read_only["annotations"]["read_only"] = json!(true);
    assert_eq!(tool_rules(vec![read_only.clone()], vec![plain()]), ["tool.no_longer_read_only"]);
    assert!(tool_rules(vec![plain()], vec![read_only]).is_empty(), "becoming read-only is fine");
    let mut approval = plain();
    approval["annotations"]["requires_approval_for_agent"] = json!(true);
    assert_eq!(tool_rules(vec![plain()], vec![approval.clone()]), ["tool.approval_added"]);
    assert!(tool_rules(vec![approval], vec![plain()]).is_empty());
}

#[test]
fn tool_thanh_pha_huy_hay_doi_permission_chi_canh_bao() {
    let mut destructive = plain();
    destructive["annotations"]["destructive"] = json!(true);
    destructive["permissions"] = json!(["notes.manage"]);
    let report = diff_tools(&manifest(vec![plain()]), &manifest(vec![destructive]), today());
    assert_eq!(report.breaking().count(), 0);
    let warnings: Vec<&str> = report
        .findings
        .iter()
        .filter(|finding| finding.level == Level::Warning)
        .map(|finding| finding.rule)
        .collect();
    assert_eq!(warnings, ["tool.now_destructive", "tool.permissions_changed"]);
}

#[test]
fn tool_ket_qua_bo_truong_hay_doi() {
    let with_output = |output: Value| {
        let mut tool = plain();
        tool["output_schema"] = output;
        tool
    };
    let full = with_output(
        json!({"type": "object", "properties": {"id": {"type": "string"}}, "required": ["id"]}),
    );
    let without = with_output(json!({"type": "object", "properties": {}}));
    let nullable = with_output(
        json!({"type": "object", "properties": {"id": {"type": ["string", "null"]}}, "required": ["id"]}),
    );
    assert_eq!(tool_rules(vec![full.clone()], vec![without]), ["tool.result_property_removed"]);
    assert_eq!(tool_rules(vec![full.clone()], vec![nullable]), ["tool.result_changed"]);
    assert_eq!(
        tool_rules(vec![full.clone()], vec![plain()]),
        ["tool.result_changed"],
        "schema dropped"
    );
    assert!(tool_rules(vec![plain()], vec![full]).is_empty(), "adding a result schema is fine");
}

#[test]
fn so_docs_mcp_tools_voi_chinh_no_khong_co_gi() {
    let text =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/mcp-tools.json"))
            .unwrap();
    let document: Value = serde_json::from_str(&text).unwrap();
    let report = diff_tools(&document, &document, today());
    assert!(report.findings.is_empty(), "{:?}", report.findings.first());
}
