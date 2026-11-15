//! Test của module `ai` với một máy chủ HTTP giả chạy ngay trong test: không gọi mạng, không cần
//! key. Kiểm dạng request gửi đi và mọi nhánh của câu trả lời. Test chạm database chạy trên cả
//! SQLite và Postgres.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use autoxide_kernel::ai::{Effort, EmbedKind, Embedder, Part, Prompt, TextModel};
use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
use autoxide_kernel::database::Database;
use autoxide_kernel::migration::{Migrator, ModuleMigrations};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_testkit::TestDatabase;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::config::{EmbeddingConfig, Provider, TextConfig};
use crate::embedding::OpenAiEmbeddings;
use crate::providers::{self, Anthropic, OpenAi};
use crate::service::{Embeddings, Text};
use crate::usage::UsageBook;

/// Một câu trả lời định sẵn của máy chủ giả.
struct Canned {
    status: u16,
    body: Value,
    retry_after: Option<&'static str>,
    delay: Duration,
}

fn ok(body: Value) -> Canned {
    Canned { status: 200, body, retry_after: None, delay: Duration::ZERO }
}

fn failing(status: u16, retry_after: Option<&'static str>) -> Canned {
    let body = json!({"type": "error", "error": {"type": "x", "message": "nope"}});
    Canned { status, body, retry_after, delay: Duration::ZERO }
}

/// Request máy chủ giả đã nhận: header và thân.
#[derive(Clone)]
struct Seen {
    path: String,
    headers: HeaderMap,
    body: Value,
}

#[derive(Default)]
struct Fake {
    answers: Mutex<VecDeque<Canned>>,
    seen: Mutex<Vec<Seen>>,
}

async fn answer(
    State(fake): State<Arc<Fake>>,
    uri: axum::http::Uri,
    headers: HeaderMap,
    body: String,
) -> Response {
    let body = serde_json::from_str(&body).unwrap_or(Value::Null);
    fake.seen.lock().unwrap().push(Seen { path: uri.path().to_owned(), headers, body });
    let canned = fake.answers.lock().unwrap().pop_front();
    let Some(canned) = canned else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "no canned answer").into_response();
    };
    tokio::time::sleep(canned.delay).await;
    let mut response =
        (StatusCode::from_u16(canned.status).unwrap(), axum::Json(canned.body)).into_response();
    if let Some(seconds) = canned.retry_after {
        response.headers_mut().insert("retry-after", seconds.parse().unwrap());
    }
    response
}

/// Máy chủ giả trả lần lượt `answers`; trả địa chỉ gốc và nơi ghi request.
async fn server(answers: Vec<Canned>) -> (String, Arc<Fake>) {
    let fake = Arc::new(Fake { answers: Mutex::new(answers.into()), ..Default::default() });
    // Một handler nhận mọi request, không có router (đây không phải API của Autoxide nên không
    // qua `routes!`); đường dẫn được ghi lại để test kiểm.
    let service = axum::handler::Handler::with_state(answer, fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, service.into_make_service()).await });
    (format!("http://{address}"), fake)
}

fn text_config(provider: Provider, base_url: &str) -> TextConfig {
    TextConfig {
        provider,
        api_key: Some("test-key".to_owned()),
        base_url: base_url.to_owned(),
        model: "model-x".to_owned(),
        images: true,
        timeout: Duration::from_secs(5),
        anthropic_fallback: true,
        openai_strict: true,
        openai_send_effort: false,
        openai_max_tokens_param: "max_completion_tokens".to_owned(),
    }
}

async fn database(test: &TestDatabase) -> Database {
    let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let kernel = Config::read(&mut reader);
    let (kernel, _) = reader.finish(kernel).unwrap();
    let database = Database::connect(&kernel.database).await.unwrap();
    Migrator::new(vec![ModuleMigrations::new("ai", crate::migration::all())])
        .unwrap()
        .up(&database)
        .await
        .unwrap();
    database
}

fn text(provider: Box<dyn providers::Provider>, book: Arc<UsageBook>) -> Text {
    let capabilities =
        autoxide_kernel::ai::Capabilities::enabled("x".into(), "model-x".into()).images(true);
    Text { provider: Some(provider), capabilities, book }
}

fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {"title": {"type": "string"}},
        "required": ["title"],
        "additionalProperties": false
    })
}

fn prompt(input: Vec<Part>) -> Prompt {
    Prompt::new("test.title", "Write a title.".into(), schema())
        .input(input)
        .effort(Effort::Low)
        .max_output_tokens(500)
}

fn anthropic_answer(text: &str, stop: &str) -> Value {
    json!({
        "model": "model-x",
        "stop_reason": stop,
        "content": [{"type": "thinking", "thinking": ""}, {"type": "text", "text": text}],
        "usage": {"input_tokens": 10, "output_tokens": 5, "cache_read_input_tokens": 2}
    })
}

fn openai_answer(content: &str, finish: &str) -> Value {
    json!({
        "model": "model-x",
        "choices": [{"finish_reason": finish, "message": {"role": "assistant", "content": content, "refusal": null}}],
        "usage": {"prompt_tokens": 7, "completion_tokens": 3}
    })
}

/// Anthropic: header, thân request (schema, effort, fallback, ảnh trước chữ), kết quả trong khối
/// `text` sau khối `thinking`, token cộng vào hạn mức.
#[tokio::test]
async fn anthropic_dang_request_va_ket_qua() {
    TestDatabase::run(async |test| {
        let (url, fake) =
            server(vec![ok(anthropic_answer(r#"{"title": "Đà Lạt"}"#, "end_turn"))]).await;
        let database = database(test).await;
        let tenant = TenantContext::legacy_default(&database);
        let book = Arc::new(UsageBook::new(database, 0));
        let model = text(
            Box::new(Anthropic::new(&text_config(Provider::Anthropic, &url)).unwrap()),
            book.clone(),
        );
        let image = Part::Image { media_type: "image/png".into(), bytes: vec![1, 2, 3] };
        let done = model
            .generate(&tenant, prompt(vec![Part::Text("Một bài về Đà Lạt".into()), image]))
            .await
            .unwrap();
        assert_eq!(done.output, json!({"title": "Đà Lạt"}));
        assert_eq!((done.usage.input_tokens, done.usage.output_tokens), (12, 5));
        assert_eq!(book.used_today().await.unwrap(), 17);

        let seen = fake.seen.lock().unwrap()[0].clone();
        assert_eq!(seen.path, "/v1/messages");
        assert_eq!(seen.headers["x-api-key"], "test-key");
        assert_eq!(seen.headers["anthropic-version"], providers::ANTHROPIC_VERSION);
        assert_eq!(seen.headers["anthropic-beta"], providers::ANTHROPIC_FALLBACK_BETA);
        let body = seen.body;
        assert_eq!(body["model"], "model-x");
        assert_eq!(body["system"], "Write a title.");
        assert_eq!(body["max_tokens"], 500);
        assert_eq!(body["fallbacks"], "default");
        assert_eq!(body["output_config"]["effort"], "low");
        assert_eq!(body["output_config"]["format"]["schema"], schema());
        let content = body["messages"][0]["content"].as_array().unwrap();
        assert_eq!(
            (content[0]["type"].as_str(), content[1]["type"].as_str()),
            (Some("image"), Some("text"))
        );
        assert_eq!(content[0]["source"]["data"], "AQID");
        assert!(body.get("temperature").is_none());
    })
    .await
    .unwrap();
}

/// Mọi nhánh lỗi: từ chối, bị cắt, sai dạng rồi đúng (thử lại một lần), sai dạng hai lần, sai
/// key, rate limit có `retry-after`, quá tải, hết giờ chờ.
#[tokio::test]
async fn anthropic_cac_nhanh_loi() {
    TestDatabase::run(async |test| {
        let database = database(test).await;
        let tenant = TenantContext::legacy_default(&database);
        let book = Arc::new(UsageBook::new(database, 0));
        let answers = vec![
            ok(anthropic_answer("", "refusal")),
            ok(anthropic_answer(r#"{"title": "x"#, "max_tokens")),
            ok(anthropic_answer("not json", "end_turn")),
            ok(anthropic_answer(r#"{"title": "second try"}"#, "end_turn")),
            ok(anthropic_answer(r#"{"wrong": 1}"#, "end_turn")),
            ok(anthropic_answer(r#"{"title": 5}"#, "end_turn")),
            failing(401, None),
            failing(429, Some("7")),
            failing(529, None),
            failing(400, None),
            Canned { delay: Duration::from_secs(3), ..ok(anthropic_answer("{}", "end_turn")) },
        ];
        let (url, fake) = server(answers).await;
        let mut config = text_config(Provider::Anthropic, &url);
        config.anthropic_fallback = false;
        config.timeout = Duration::from_secs(1);
        let model = text(Box::new(Anthropic::new(&config).unwrap()), book);
        let run = || model.generate(&tenant, prompt(vec![Part::Text("x".into())]));
        assert_eq!(run().await.unwrap_err().code(), "ai.refused");
        assert_eq!(run().await.unwrap_err().code(), "ai.output_truncated");
        assert_eq!(run().await.unwrap().output, json!({"title": "second try"}));
        assert_eq!(run().await.unwrap_err().code(), "ai.output_invalid");
        assert_eq!(run().await.unwrap_err().code(), "ai.misconfigured");
        let busy = run().await.unwrap_err();
        assert_eq!((busy.code(), busy.retry_after()), ("ai.provider_unavailable", Some(7)));
        assert_eq!(run().await.unwrap_err().code(), "ai.provider_unavailable");
        assert_eq!(run().await.unwrap_err().code(), "ai.request_rejected");
        assert_eq!(run().await.unwrap_err().code(), "ai.provider_unavailable");
        // Không có header beta khi tắt fallback.
        assert!(fake.seen.lock().unwrap()[0].headers.get("anthropic-beta").is_none());
    })
    .await
    .unwrap();
}

/// Chat Completions: `response_format` chặt, `max_completion_tokens`, không gửi
/// `reasoning_effort`, key là `Bearer`; chế độ `json_object` đưa schema vào lời dặn và đọc được
/// JSON bọc trong ```` ``` ````; `refusal`, `length`, `content_filter`.
#[tokio::test]
async fn openai_dang_request_va_cac_nhanh() {
    TestDatabase::run(async |test| {
        let database = database(test).await;
        let tenant = TenantContext::legacy_default(&database);
        let book = Arc::new(UsageBook::new(database, 0));
        let refusal = json!({"model": "m", "choices": [{"finish_reason": "stop", "message": {"content": null, "refusal": "I can't."}}]});
        let answers = vec![
            ok(openai_answer(r#"{"title": "A"}"#, "stop")),
            ok(openai_answer("```json\n{\"title\": \"B\"}\n```", "stop")),
            ok(refusal),
            ok(openai_answer("{", "length")),
            ok(openai_answer("", "content_filter")),
        ];
        let (url, fake) = server(answers).await;
        let config = text_config(Provider::OpenAi, &url);
        let strict = text(Box::new(OpenAi::new(&config).unwrap()), book.clone());
        let image = Part::Image { media_type: "image/jpeg".into(), bytes: vec![255] };
        let done = strict.generate(&tenant, prompt(vec![Part::Text("x".into()), image])).await.unwrap();
        assert_eq!(done.output, json!({"title": "A"}));
        assert_eq!(book.used_today().await.unwrap(), 10);
        let seen = fake.seen.lock().unwrap()[0].clone();
        assert_eq!(seen.path, "/chat/completions");
        assert_eq!(seen.headers["authorization"], "Bearer test-key");
        let body = seen.body;
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(body["response_format"]["json_schema"]["name"], "test_title");
        assert_eq!(body["response_format"]["json_schema"]["strict"], true);
        assert_eq!(body["max_completion_tokens"], 500);
        assert!(body.get("reasoning_effort").is_none());
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"][1]["image_url"]["url"], "data:image/jpeg;base64,/w==");

        let mut loose = config.clone();
        loose.openai_strict = false;
        loose.openai_send_effort = true;
        loose.openai_max_tokens_param = "max_tokens".into();
        loose.api_key = None;
        let loose = text(Box::new(OpenAi::new(&loose).unwrap()), book);
        let run = || loose.generate(&tenant, prompt(vec![Part::Text("x".into())]));
        assert_eq!(run().await.unwrap().output, json!({"title": "B"}));
        let seen = fake.seen.lock().unwrap()[1].clone();
        assert!(seen.headers.get("authorization").is_none());
        assert_eq!(seen.body["response_format"], json!({"type": "json_object"}));
        assert!(seen.body["messages"][0]["content"].as_str().unwrap().contains("JSON Schema"));
        assert_eq!((seen.body["max_tokens"].as_u64(), seen.body["reasoning_effort"].as_str()), (Some(500), Some("low")));
        assert_eq!(run().await.unwrap_err().code(), "ai.refused");
        assert_eq!(run().await.unwrap_err().code(), "ai.output_truncated");
        assert_eq!(run().await.unwrap_err().code(), "ai.refused");
    })
    .await
    .unwrap();
}

/// Embedding: chia lô, xếp theo `index`, gửi `dimensions` khi bật, sai số chiều bị từ chối.
#[tokio::test]
async fn embedding_chia_lo_va_kiem_so_chieu() {
    TestDatabase::run(async |test| {
        let database = database(test).await;
        let tenant = TenantContext::legacy_default(&database);
        let book = Arc::new(UsageBook::new(database, 0));
        let answers = vec![
            ok(json!({"data": [{"index": 1, "embedding": [0.0, 1.0]}, {"index": 0, "embedding": [1.0, 0.0]}], "usage": {"prompt_tokens": 4}})),
            ok(json!({"data": [{"index": 0, "embedding": [0.5, 0.5]}], "usage": {"total_tokens": 2}})),
            ok(json!({"data": [{"index": 0, "embedding": [0.5]}]})),
        ];
        let (url, fake) = server(answers).await;
        let config = EmbeddingConfig {
            base_url: url,
            api_key: None,
            model: "bge-m3".into(),
            dimensions: 2,
            batch_size: 2,
            send_dimensions: true,
            timeout: Duration::from_secs(5),
        };
        let embeddings = Embeddings { client: Some(OpenAiEmbeddings::new(&config).unwrap()), book: book.clone() };
        assert!(embeddings.enabled());
        assert_eq!((embeddings.model_id(), embeddings.dimensions()), ("bge-m3", 2));
        let texts: Vec<String> = ["a", "b", "c"].map(str::to_owned).to_vec();
        let vectors = embeddings.embed(&tenant, "test.embed", &texts, EmbedKind::Document).await.unwrap();
        assert_eq!(vectors, [vec![1.0, 0.0], vec![0.0, 1.0], vec![0.5, 0.5]]);
        assert_eq!(book.used_today().await.unwrap(), 6);
        let seen = fake.seen.lock().unwrap().clone();
        assert_eq!(seen[0].path, "/embeddings");
        assert_eq!(seen[0].body, json!({"model": "bge-m3", "input": ["a", "b"], "dimensions": 2}));
        assert_eq!(seen[1].body["input"], json!(["c"]));
        let wrong = embeddings.embed(&tenant, "test.embed", &["d".to_owned()], EmbedKind::Query).await;
        assert_eq!(wrong.unwrap_err().code(), "ai.output_invalid");
    })
    .await
    .unwrap();
}

/// Tắt: sinh chữ trả `ai.disabled`, embedding `ai.embedding_disabled`; mô hình không đọc ảnh
/// thì từ chối ảnh; hết hạn mức thì `ai.quota_exceeded` kèm `Retry-After`.
#[tokio::test]
async fn tat_khong_doc_anh_va_het_han_muc() {
    TestDatabase::run(async |test| {
        let database = database(test).await;
        let tenant = TenantContext::legacy_default(&database);
        let book = Arc::new(UsageBook::new(database.clone(), 0));
        let off = Text { provider: None, capabilities: Default::default(), book: book.clone() };
        assert!(!off.capabilities().enabled);
        assert_eq!(
            off.generate(&tenant, prompt(Vec::new())).await.unwrap_err().code(),
            "ai.disabled"
        );
        let no_embeddings = Embeddings { client: None, book: book.clone() };
        assert!(!no_embeddings.enabled());
        let error =
            no_embeddings.embed(&tenant, "test.embed", &["a".to_owned()], EmbedKind::Query).await;
        assert_eq!(error.unwrap_err().code(), "ai.embedding_disabled");

        let (url, _) = server(vec![ok(anthropic_answer(r#"{"title": "x"}"#, "end_turn"))]).await;
        let mut blind =
            text(Box::new(Anthropic::new(&text_config(Provider::Anthropic, &url)).unwrap()), book);
        blind.capabilities.images = false;
        let image = Part::Image { media_type: "image/png".into(), bytes: vec![1] };
        assert_eq!(
            blind.generate(&tenant, prompt(vec![image])).await.unwrap_err().code(),
            "ai.unsupported_input"
        );

        let limited = Arc::new(UsageBook::new(database, 15));
        let model = text(
            Box::new(Anthropic::new(&text_config(Provider::Anthropic, &url)).unwrap()),
            limited.clone(),
        );
        model.generate(&tenant, prompt(vec![Part::Text("x".into())])).await.unwrap();
        let error =
            model.generate(&tenant, prompt(vec![Part::Text("x".into())])).await.unwrap_err();
        assert_eq!(error.code(), "ai.quota_exceeded");
        assert!(error.retry_after().is_some_and(|seconds| seconds <= 86_400));
    })
    .await
    .unwrap();
}

/// Nhiều tiến trình cùng cộng token: không mất số, một dòng mỗi (ngày, việc); dọn dòng cũ.
#[tokio::test]
async fn cong_token_dong_thoi() {
    TestDatabase::run(async |test| {
        let database = database(test).await;
        let book = Arc::new(UsageBook::new(database.clone(), 0));
        let tasks: Vec<_> = (0..12)
            .map(|_| {
                let book = book.clone();
                tokio::spawn(async move {
                    let usage = autoxide_kernel::ai::Usage::new(3, 2);
                    book.record("test.parallel", usage).await
                })
            })
            .collect();
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        assert_eq!(book.used_today().await.unwrap(), 60);
        use sea_orm::{EntityTrait, PaginatorTrait};
        let rows = crate::usage::entity::Entity::find().count(database.read()).await.unwrap();
        assert_eq!(rows, 1);
        let row = crate::usage::entity::Entity::find().one(database.read()).await.unwrap().unwrap();
        assert_eq!(row.requests, 12);
        assert_eq!(book.prune(1).await.unwrap(), 0);
    })
    .await
    .unwrap();
}
