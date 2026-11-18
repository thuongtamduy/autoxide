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
