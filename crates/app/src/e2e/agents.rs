//! Bề mặt cho AI agent của cả binary (bước 40 P2): `/llms.txt` nêu đủ module và không có link chết,
//! resource schema khớp OpenAPI, prompt của `content` / `comments` theo permission.

use autoxide_kernel::mcp::SCHEMA_MIME;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use rmcp::model::{GetPromptRequestParams, ReadResourceRequestParams, ResourceContents};
use serde_json::Value;

/// App với mọi module của binary; `agent` có quyền đọc / viết nháp nội dung và kiểm duyệt bình
/// luận, `reader` chỉ đọc nội dung.
async fn app(test: &TestDatabase) -> TestApp {
    let dir = std::env::temp_dir().join(format!("autoxide-agents-{}", uuid::Uuid::now_v7()));
    let mut builder = TestApp::builder(test)
        .setting("AUTOXIDE_FILES_DIR", &dir.join("files").display().to_string())
        .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string())
        .setting("AUTOXIDE_HTTP_API_DOCS", "true")
        .user(
            "agent",
            1,
            &["content.read", "content.write_own", "comments.read", "comments.moderate"],
        )
        .user("reader", 2, &["content.read"]);
    for module in crate::modules::all() {
        builder = builder.boxed_module(module);
    }
    builder.build().await.unwrap()
}

fn text(contents: &[ResourceContents]) -> String {
    match contents {
        [ResourceContents::TextResourceContents { text, .. }] => text.clone(),
        other => panic!("one text content expected: {other:?}"),
    }
}

#[test]
fn moi_module_co_mo_ta() {
    let missing: Vec<&str> = crate::modules::all()
        .iter()
        .filter(|module| module.description().trim().is_empty())
        .map(|module| module.name())
        .collect();
    assert!(missing.is_empty(), "modules without `Module::description`: {missing:?}");
}

#[tokio::test]
async fn llms_txt_co_moi_module_va_link() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let response = app.get("/llms.txt").send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let text = response.text();
        for (name, description) in app.modules().descriptions() {
            let line = format!("- [{name}](/api/v1/kernel/modules/{name}): {description}");
            assert!(text.contains(&line), "missing `{line}` in:\n{text}");
        }
        // Mỗi module: link tới tài liệu OpenAPI và endpoint MCP riêng, cùng tên tool.
        for expected in [
            "  - [OpenAPI](/api/v1/openapi/content.json) · [MCP](/mcp/content) · tools: ",
            "`content.create_draft`",
            "/api/v1/openapi/<module>.json",
        ] {
            assert!(text.contains(expected), "missing {expected} in:\n{text}");
        }
        for expected in [
            "`content.draft_post`",
            "`comments.moderate_queue`",
            "`autoxide://content/guide`",
            "`autoxide://content/schema/entry`",
            "`autoxide://comments/guide`",
            "`autoxide://kernel/modules`",
            "POST /api/v1/auth/api-keys",
        ] {
            assert!(text.contains(expected), "missing {expected} in:\n{text}");
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn moi_link_trong_llms_txt_khong_404() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let text = app.get("/llms.txt").send().await.unwrap().text();
        let links: Vec<String> = text
            .split("](")
            .skip(1)
            .filter_map(|rest| rest.split(')').next())
            .map(str::to_owned)
            .collect();
        assert!(links.len() > 20, "{links:?}");
        for link in links {
            // Không kiểm hợp đồng: `/mcp` trả 401 kiểu Problem nhưng `GET` không có trong OpenAPI.
            let response = app.get(&link).unchecked().send().await.unwrap();
            assert!(
                matches!(response.status().as_u16(), 200 | 401),
                "{link}: {}",
                response.status()
            );
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn openapi_rieng_cua_module_khop_ban_chung_va_mcp_rieng() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let full = serde_json::to_value(app.openapi()).unwrap();
        let mut documented = 0;
        for (name, _) in app.modules().descriptions() {
            let prefix = format!("/api/v1/{name}");
            let expected: Vec<&String> = full["paths"]
                .as_object()
                .unwrap()
                .keys()
                .filter(|path| *path == &prefix || path.starts_with(&format!("{prefix}/")))
                .collect();
            let response = app.get(&format!("/api/v1/openapi/{name}.json")).send().await.unwrap();
            if expected.is_empty() {
                assert_eq!(response.status(), StatusCode::NOT_FOUND, "{name}");
                continue;
            }
            assert_eq!(response.status(), StatusCode::OK, "{name}");
            let document = response.json();
            let paths: Vec<&String> = document["paths"].as_object().unwrap().keys().collect();
            assert_eq!(paths, expected, "{name}");
            for path in paths {
                assert_eq!(document["paths"][path], full["paths"][path], "{name} {path}");
            }
            // Mọi `$ref` trỏ tới schema có trong chính tài liệu, cùng nội dung như bản chung.
            let schemas = document["components"]["schemas"].as_object().unwrap();
            for (schema, value) in schemas {
                assert_eq!(value, &full["components"]["schemas"][schema], "{name} {schema}");
            }
            let mut refs = Vec::new();
            collect_refs(&document, &mut refs);
            for reference in refs {
                let target = reference.trim_start_matches("#/components/schemas/");
                assert!(schemas.contains_key(target), "{name}: dangling {reference}");
            }
            documented += 1;
        }
        assert!(documented > 15, "only {documented} modules have a document");

        let missing = app.get("/api/v1/openapi/orders.json").send().await.unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(missing.json()["code"], "openapi_module_not_found");
        let no_suffix = app.get("/api/v1/openapi/content").send().await.unwrap();
        assert_eq!(no_suffix.status(), StatusCode::NOT_FOUND);

        // Endpoint MCP riêng: chỉ tool, resource, prompt của module đó, vẫn theo permission.
        let agent = app.mcp_at("agent", "/mcp/content").await.unwrap();
        let tools = agent.list_all_tools().await.unwrap();
        assert!(!tools.is_empty());
        assert!(tools.iter().all(|tool| tool.name.starts_with("content.")), "{tools:?}");
        let resources = agent.list_all_resources().await.unwrap();
        assert!(resources.iter().all(|resource| resource.uri.starts_with("autoxide://content/")));
        let prompts = agent.list_all_prompts().await.unwrap();
        assert_eq!(
            prompts.iter().map(|prompt| prompt.name.as_str()).collect::<Vec<_>>(),
            ["content.draft_post"]
        );
        agent.cancel().await.unwrap();
    })
    .await
    .unwrap();
}

fn collect_refs(node: &Value, refs: &mut Vec<String>) {
    match node {
        Value::Object(map) => {
            for (key, value) in map {
                match value {
                    Value::String(reference) if key == "$ref" => refs.push(reference.clone()),
                    _ => collect_refs(value, refs),
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|item| collect_refs(item, refs)),
        _ => {}
    }
}

#[tokio::test]
async fn schema_resource_khop_openapi() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let agent = app.mcp("agent").await.unwrap();
        let listed = agent.list_all_resources().await.unwrap();
        let entry = listed
            .iter()
            .find(|resource| resource.uri == "autoxide://content/schema/entry")
            .unwrap();
        assert_eq!(entry.mime_type.as_deref(), Some(SCHEMA_MIME));
        let read = agent
            .read_resource(ReadResourceRequestParams::new("autoxide://content/schema/entry"))
            .await
            .unwrap();
        let mut schema: Value = serde_json::from_str(&text(&read.contents)).unwrap();
        let contract = serde_json::to_value(app.openapi()).unwrap();
        let components = &contract["components"]["schemas"];
        // Kiểu con nhúng trong `$defs` bằng đúng schema cùng tên trong OpenAPI.
        let defs = schema.as_object_mut().unwrap().remove("$defs").unwrap();
        for (name, nested) in defs.as_object().unwrap() {
            let mut nested = nested.clone();
            back_to_components(&mut nested);
            assert_eq!(&nested, &components[name], "nested schema {name}");
        }
        let object = schema.as_object_mut().unwrap();
        object.remove("$schema");
        assert_eq!(object.remove("title"), Some(Value::from("EntryResponse")));
        back_to_components(&mut schema);
        assert_eq!(schema, components["EntryResponse"]);
        agent.cancel().await.unwrap();
    })
    .await
    .unwrap();
}

/// `#/$defs/X` của resource về lại `#/components/schemas/X` của OpenAPI.
fn back_to_components(node: &mut Value) {
    match node {
        Value::Object(object) => {
            for (key, value) in object.iter_mut() {
                match value {
                    Value::String(target) if key == "$ref" => {
                        *target = target.replace("#/$defs/", "#/components/schemas/");
                    }
                    _ => back_to_components(value),
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(back_to_components),
        _ => {}
    }
}

#[tokio::test]
async fn prompt_cua_module_theo_permission() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let agent = app.mcp("agent").await.unwrap();
        let names: Vec<String> =
            agent.list_all_prompts().await.unwrap().into_iter().map(|prompt| prompt.name).collect();
        assert_eq!(names, ["content.draft_post", "comments.moderate_queue"]);
        let Value::Object(arguments) =
            serde_json::json!({"topic": "Đà Lạt mùa hoa", "locale": "vi"})
        else {
            unreachable!()
        };
        let prompt = agent
            .get_prompt(GetPromptRequestParams::new("content.draft_post").with_arguments(arguments))
            .await
            .unwrap();
        let message = serde_json::to_value(&prompt.messages[0]).unwrap();
        let body = message["content"]["text"].as_str().unwrap();
        assert!(body.contains("<topic>\nĐà Lạt mùa hoa\n</topic>"), "{body}");
        assert!(body.contains("`content.create_draft`"), "{body}");
        let guide = agent
            .read_resource(ReadResourceRequestParams::new("autoxide://comments/guide"))
            .await
            .unwrap();
        assert!(text(&guide.contents).starts_with("# Moderating comments"));

        // Chỉ đọc nội dung: không viết nháp được nên không thấy prompt; không đọc được quy tắc kiểm
        // duyệt.
        let reader = app.mcp("reader").await.unwrap();
        assert!(reader.list_all_prompts().await.unwrap().is_empty());
        let denied = reader
            .read_resource(ReadResourceRequestParams::new("autoxide://comments/guide"))
            .await
            .unwrap_err()
            .to_string();
        assert!(denied.contains("Unknown resource"), "{denied}");
        agent.cancel().await.unwrap();
        reader.cancel().await.unwrap();
    })
    .await
    .unwrap();
}
