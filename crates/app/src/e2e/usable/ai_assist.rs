//! Tính năng AI dùng được như người dùng cấu hình thật (cả SQLite và Postgres): nhà cung cấp dạng OpenAI
//! (`AUTOXIDE_AI_PROVIDER=openai`, `AUTOXIDE_EMBEDDING_PROVIDER=openai`) trỏ tới máy chủ giả trả lời đúng
//! định dạng Chat Completions và Embeddings. Gợi ý SEO, dịch bài, tìm theo nghĩa công khai, gợi ý alt ảnh.

use autoxide_testkit::{FakeProviderServer, TestApp, TestDatabase};
use serde_json::{Value, json};

use super::kit::{ADMIN, ANONYMOUS, app_with, call_tool, drain, expect, get, id};

/// Số chiều vector của máy chủ giả.
const DIMENSIONS: usize = 4;

/// App có AI trỏ tới `server`, và cấu hình thêm `settings`.
pub(crate) async fn app_with_ai(
    test: &TestDatabase,
    server: &FakeProviderServer,
    settings: &[(&str, &str)],
) -> TestApp {
    let url = server.url().to_owned();
    let settings: Vec<(String, String)> =
        settings.iter().map(|(name, value)| ((*name).to_owned(), (*value).to_owned())).collect();
    app_with(test, move |mut builder| {
        for (name, value) in [
            ("AUTOXIDE_AI_PROVIDER", "openai"),
            ("AUTOXIDE_AI_BASE_URL", url.as_str()),
            ("AUTOXIDE_AI_MODEL", "test-model"),
            ("AUTOXIDE_AI_API_KEY", "test-key"),
            ("AUTOXIDE_EMBEDDING_PROVIDER", "openai"),
            ("AUTOXIDE_EMBEDDING_BASE_URL", url.as_str()),
            ("AUTOXIDE_EMBEDDING_MODEL", "test-embedding"),
            ("AUTOXIDE_EMBEDDING_DIMENSIONS", "4"),
            ("AUTOXIDE_EMBEDDING_BATCH_SIZE", "1"),
        ] {
            builder = builder.setting(name, value);
        }
        for (name, value) in &settings {
            builder = builder.setting(name, value);
        }
        builder
    })
    .await
}

/// Câu trả lời kế tiếp của mô hình sinh chữ: `answer` (JSON theo schema của tính năng).
pub(crate) fn ai_answers(server: &FakeProviderServer, answer: Value) {
    server.reply(
        "POST",
        "/chat/completions",
        200,
        json!({"model": "test-model", "usage": {"prompt_tokens": 20, "completion_tokens": 10},
               "choices": [{"index": 0, "finish_reason": "stop",
                            "message": {"role": "assistant", "content": answer.to_string()}}]}),
    );
}

/// Máy chủ embedding trả một vector cho mỗi lời gọi (lô 1).
pub(crate) fn embeddings(server: &FakeProviderServer) {
    let vector: Vec<f32> = (0..DIMENSIONS).map(|n| 0.1 * (n as f32 + 1.0)).collect();
    server.reply(
        "POST",
        "/embeddings",
        200,
        json!({"model": "test-embedding", "usage": {"prompt_tokens": 3},
               "data": [{"index": 0, "object": "embedding", "embedding": vector}]}),
    );
}

async fn post_with_body(app: &TestApp, title: &str) -> String {
    let body = json!({"kind": "post", "title": title, "locale": "vi",
        "body": {"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Nội dung"}]}]}});
    id(&expect(app, "POST", "/api/v1/content/entries", ADMIN, Some(body), 201).await)
}

async fn if_match(
    app: &TestApp,
    method: &str,
    path: &str,
    body: Option<Value>,
    expected: u16,
) -> Value {
    let method = axum::http::Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method.clone(), path).token(ADMIN).header("if-match", "*");
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let json: Value = serde_json::from_slice(answer.body()).unwrap_or(Value::Null);
    assert_eq!(answer.status().as_u16(), expected, "{method} {path}: {json}");
    json
}

#[tokio::test]
async fn goi_y_seo_dich_bai_tim_theo_nghia_va_alt_anh() {
    TestDatabase::run(async |test| {
        let server = FakeProviderServer::start().await.unwrap();
        embeddings(&server);
        let app = app_with_ai(test, &server, &[("AUTOXIDE_CONTENT_PUBLIC_SEMANTIC_SEARCH", "true")]).await;

        let post = post_with_body(&app, "Ra mắt").await;
        if_match(&app, "POST", &format!("/api/v1/content/entries/{post}/publish"), None, 200).await;
        drain(&app).await;
        expect(&app, "GET", "/api/v1/content/public/search?q=ra%20m%E1%BA%AFt", ANONYMOUS, None, 200).await;

        let seo = json!({"meta_title": "Ra mắt sản phẩm mới", "meta_description": "Tin ra mắt", "excerpt": "Tóm tắt",
                         "term_ids": []});
        ai_answers(&server, seo.clone());
        expect(&app, "POST", &format!("/api/v1/content/entries/{post}/seo-suggestions"), ADMIN, None, 200).await;
        ai_answers(&server, seo);
        call_tool(&app, ADMIN, "content.suggest_seo", json!({"id": post})).await;

        // Dịch sang tiếng Anh: mẩu `f.title` (tiêu đề) và `b1` (đoạn đầu của thân bài).
        let translated = json!({"segments": [{"id": "f.title", "text": "Launch"}, {"id": "b1", "text": "Content"}]});
        ai_answers(&server, translated.clone());
        let requested =
            expect(&app, "POST", &format!("/api/v1/content/entries/{post}/translate"), ADMIN, Some(json!({"locale": "en"})), 202).await;
        drain(&app).await;
        get(&app, &format!("/api/v1/content/entries/{post}/translation-requests")).await;
        let done = get(&app, &format!("/api/v1/content/translation-requests/{}", id(&requested))).await;
        assert_eq!(done["data"]["status"], "succeeded", "{done}");
        let second = post_with_body(&app, "Tin vui").await;
        ai_answers(&server, json!({"segments": [{"id": "f.title", "text": "Good news"}, {"id": "b1", "text": "Content"}]}));
        let request = call_tool(&app, ADMIN, "content.translate", json!({"id": second, "locale": "en"})).await;
        drain(&app).await;
        call_tool(&app, ADMIN, "content.get_translation_request", json!({"id": request["id"]})).await;

        // Gợi ý alt cho ảnh.
        let png = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/guides/files/photo.png")).unwrap();
        let uploaded = app.post("/api/v1/media?name=photo.png").token(ADMIN).header("content-type", "image/png").bytes(png).send().await.unwrap();
        assert_eq!(uploaded.status(), 201, "{}", uploaded.text());
        let item = uploaded.json()["data"]["id"].as_str().unwrap().to_owned();
        // Máy chủ không gửi ảnh riêng tư cho nhà cung cấp AI: ảnh dùng trên website thì công khai.
        if_match(&app, "PATCH", &format!("/api/v1/media/{item}"), Some(json!({"visibility": "public"})), 200).await;
        let alt = json!({"alt": "Ảnh minh hoạ sản phẩm", "decorative": false});
        ai_answers(&server, alt.clone());
        expect(&app, "POST", &format!("/api/v1/media/{item}/alt-suggestions"), ADMIN, Some(json!({"locale": "vi"})), 200).await;
        ai_answers(&server, alt);
        call_tool(&app, ADMIN, "media.suggest_alt", json!({"id": item, "locale": "vi"})).await;
    })
    .await
    .unwrap();
}
