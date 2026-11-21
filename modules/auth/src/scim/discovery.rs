//! Ba endpoint khám phá của RFC 7644 §4: `ServiceProviderConfig`, `ResourceTypes`, `Schemas`.
//! Nội dung cố định (giống nhau cho mọi khách), nói đúng mức Autoxide hỗ trợ — client SCIM đọc
//! để biết có PATCH, bộ lọc, Bulk hay không (D-20261121-scim).

use serde_json::{Value, json};

use super::{GROUP_SCHEMA, LIST_SCHEMA, USER_SCHEMA};

/// Nhiều kết quả nhất một trang (`count`) và cũng là `filter.maxResults`.
pub(crate) const MAX_RESULTS: u64 = 200;

/// `ServiceProviderConfig` (RFC 7643 §5). `base`: đường dẫn gốc SCIM của máy chủ.
pub(crate) fn service_provider_config(base: &str) -> Value {
    json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig"],
        "documentationUri": "https://github.com/thuongtamduy/autoxide/blob/main/docs/guides/scim.md",
        "patch": {"supported": true},
        "bulk": {"supported": false, "maxOperations": 0, "maxPayloadSize": 0},
        "filter": {"supported": true, "maxResults": MAX_RESULTS},
        "changePassword": {"supported": false},
        "sort": {"supported": false},
        "etag": {"supported": false},
        "authenticationSchemes": [{
            "type": "oauthbearertoken",
            "name": "SCIM token",
            "description": "A SCIM token of the tenant (`axscim1_…`), created with POST /api/v1/auth/scim/tokens, sent as `Authorization: Bearer <token>`.",
            "primary": true,
        }],
        "meta": {
            "resourceType": "ServiceProviderConfig",
            "location": format!("{base}/ServiceProviderConfig"),
        },
    })
}

/// Hai loại tài nguyên: `User`, `Group`.
pub(crate) fn resource_types(base: &str) -> Vec<Value> {
    let one = |id: &str, endpoint: &str, schema: &str, description: &str| {
        json!({
            "schemas": ["urn:ietf:params:scim:schemas:core:2.0:ResourceType"],
            "id": id,
            "name": id,
            "endpoint": endpoint,
            "description": description,
            "schema": schema,
            "meta": {"resourceType": "ResourceType", "location": format!("{base}/ResourceTypes/{id}")},
        })
    };
    vec![
        one("User", "/Users", USER_SCHEMA, "A staff account of the tenant."),
        one(
            "Group",
            "/Groups",
            GROUP_SCHEMA,
            "A group of the identity provider; mapping rules turn it into roles at organization units.",
        ),
    ]
}

/// Lược đồ `User` và `Group`, chỉ các thuộc tính Autoxide lưu.
pub(crate) fn schemas(base: &str) -> Vec<Value> {
    let attribute = |name: &str,
                     kind: &str,
                     required: bool,
                     case_exact: bool,
                     mutability: &str,
                     uniqueness: &str| {
        json!({
            "name": name,
            "type": kind,
            "multiValued": false,
            "required": required,
            "caseExact": case_exact,
            "mutability": mutability,
            "returned": "default",
            "uniqueness": uniqueness,
        })
    };
    let text = |name: &str| attribute(name, "string", false, false, "readWrite", "none");
    let mut emails = attribute("emails", "complex", false, false, "readWrite", "none");
    emails["multiValued"] = json!(true);
    emails["subAttributes"] = json!([
        text("value"),
        text("type"),
        attribute("primary", "boolean", false, false, "readWrite", "none")
    ]);
    let mut name = attribute("name", "complex", false, false, "readWrite", "none");
    name["subAttributes"] = json!([text("formatted"), text("givenName"), text("familyName")]);
    let mut groups = attribute("groups", "complex", false, false, "readOnly", "none");
    groups["multiValued"] = json!(true);
    groups["subAttributes"] = json!([text("value"), text("display")]);
    let mut members = attribute("members", "complex", false, false, "readWrite", "none");
    members["multiValued"] = json!(true);
    members["subAttributes"] = json!([text("value"), text("display")]);
    let schema = |id: &str, name: &str, attributes: Vec<Value>| {
        json!({
            "schemas": ["urn:ietf:params:scim:schemas:core:2.0:Schema"],
            "id": id,
            "name": name,
            "attributes": attributes,
            "meta": {"resourceType": "Schema", "location": format!("{base}/Schemas/{id}")},
        })
    };
    vec![
        schema(
            USER_SCHEMA,
            "User",
            vec![
                attribute("userName", "string", true, false, "readWrite", "server"),
                attribute("externalId", "string", false, true, "readWrite", "server"),
                text("displayName"),
                name,
                emails,
                attribute("active", "boolean", false, false, "readWrite", "none"),
                groups,
            ],
        ),
        schema(
            GROUP_SCHEMA,
            "Group",
            vec![
                attribute("displayName", "string", true, false, "readWrite", "server"),
                attribute("externalId", "string", false, true, "readWrite", "server"),
                members,
            ],
        ),
    ]
}

/// Danh sách (`ListResponse`) của các tài nguyên cố định, trả nguyên một trang.
pub(crate) fn list(resources: Vec<Value>) -> Value {
    json!({
        "schemas": [LIST_SCHEMA],
        "totalResults": resources.len(),
        "startIndex": 1,
        "itemsPerPage": resources.len(),
        "Resources": resources,
    })
}
