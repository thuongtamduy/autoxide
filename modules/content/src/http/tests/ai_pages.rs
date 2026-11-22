//! AI tạo và sửa trang (bước 67, D-109): `FakeTextModel` trả lời theo kịch bản. Kết quả chỉ là bản
//! nháp `ai_assisted`; khối lạ thì gọi lại kèm lỗi rồi báo `content.ai_result_invalid`; sửa bằng JSON
//! Patch; có người sửa xen giữa thì không ghi đè; lịch sử trò chuyện giữ revision để hoàn tác; mô hình
//! chỉ thấy lời yêu cầu và bố cục (không dữ liệu người mua). Gate chạy trên cả SQLite và Postgres.

use std::sync::Mutex;

use super::*;
use autoxide_kernel::ai::{Part, Prompt};

/// Câu trả lời lần lượt theo kịch bản; hết kịch bản thì lặp câu cuối.
fn scripted(answers: Vec<Value>) -> FakeTextModel {
    let answers = Mutex::new(answers);
    FakeTextModel::new(move |_prompt: &Prompt| {
        let mut list = answers.lock().unwrap();
        Ok(if list.len() > 1 { list.remove(0) } else { list[0].clone() })
    })
}

fn layout_answer(sections: Value) -> Value {
    json!({"layout_json": json!({"type": "layout", "schema": 1, "sections": sections}).to_string()})
}

async fn ai_app(test: &TestDatabase, model: Arc<FakeTextModel>) -> TestApp {
    let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
    let text: Arc<dyn TextModel> = model;
    let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
    TestApp::builder(test)
        .user("admin", 1, &["content.read", "content.write", "content.publish"])
        .user("reader", 2, &["content.read"])
        .provide::<dyn Media>(media)
        .provide::<dyn TextModel>(text)
        .provide::<dyn Embedder>(embedder)
        .module(crate::module())
        .build()
        .await
        .unwrap()
}

async fn run_jobs(app: &TestApp) {
    let worker = app.modules().worker(2).unwrap();
    while worker.run_once().await.unwrap() > 0 {}
}

async fn new_page(app: &TestApp, title: &str) -> String {
    let created = send(
        app,
        "POST",
        "/entries",
        Some("admin"),
        &[],
        Some(json!({"kind": "page", "title": title})),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
    created.json()["data"]["id"].as_str().unwrap().to_owned()
}

async fn thread(app: &TestApp, id: &str) -> Value {
    let answer =
        send(app, "GET", &format!("/entries/{id}/ai/thread"), Some("reader"), &[], None).await;
    assert_eq!(answer.status(), StatusCode::OK, "{}", answer.json());
    answer.json()["data"][0].clone()
}

#[tokio::test]
async fn ai_tao_trang_chi_la_ban_nhap_va_sua_tiep_bang_json_patch() {
    TestDatabase::run(async |test| {
        let model = Arc::new(scripted(vec![
            layout_answer(json!([
                {"id": "hero", "block": "content.hero", "props": {"heading": "Sale hè", "text": "Đồ mới mỗi tuần."}},
                {"id": "faq", "block": "content.faq", "props": {"items": [{"question": "Giao bao lâu?", "answer": "2 ngày."}]}}
            ])),
            json!({"patch_json": json!([{"op": "replace", "path": "/sections/0/props/heading", "value": "Sale tháng 7"}]).to_string()}),
        ]));
        let app = ai_app(test, model.clone()).await;
        let id = new_page(&app, "Sale").await;
        let refused = send(&app, "POST", &format!("/entries/{id}/ai/compose"), Some("reader"), &[], Some(json!({"instruction": "x"}))).await;
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);
        let empty = send(&app, "POST", &format!("/entries/{id}/ai/compose"), Some("admin"), &[], Some(json!({"instruction": " "}))).await;
        assert_eq!(empty.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let queued = send(&app, "POST", &format!("/entries/{id}/ai/compose"), Some("admin"), &[], Some(json!({"instruction": "Trang sale hè có hỏi đáp"}))).await;
        assert_eq!(queued.status(), StatusCode::ACCEPTED, "{}", queued.json());
        assert_eq!(queued.json()["data"]["status"], "queued");
        run_jobs(&app).await;

        let turn = thread(&app, &id).await;
        assert_eq!((turn["status"].as_str(), turn["tries"].as_i64()), (Some("succeeded"), Some(1)), "{turn}");
        let page = send(&app, "GET", &format!("/entries/{id}"), Some("admin"), &[], None).await.json();
        assert_eq!(page["data"]["status"], "draft", "never published by itself");
        assert_eq!(page["data"]["ai_assisted"], true);
        assert_eq!(page["data"]["body"]["sections"][0]["props"]["heading"], "Sale hè");
        // Mô hình chỉ thấy luật + danh mục khối ở `instructions`, lời yêu cầu ở `input`.
        let prompt = model.prompts.lock().unwrap()[0].clone();
        assert!(prompt.instructions.contains("content.hero") && prompt.instructions.contains("Never write prices"));
        assert!(!prompt.instructions.contains("compliance.legal_footer"));
        let Part::Text(request) = &prompt.input[0] else { panic!() };
        assert!(request.contains("sale hè"));

        // Sửa tiếp: JSON Patch trên bố cục hiện tại; lượt giữ revision trước để hoàn tác.
        let queued = send(&app, "POST", &format!("/entries/{id}/ai/edit"), Some("admin"), &[], Some(json!({"instruction": "Đổi tiêu đề thành tháng 7", "selection": ["hero"]}))).await;
        assert_eq!(queued.status(), StatusCode::ACCEPTED, "{}", queued.json());
        run_jobs(&app).await;
        let turn = thread(&app, &id).await;
        assert_eq!(turn["status"], "succeeded", "{turn}");
        let page = send(&app, "GET", &format!("/entries/{id}"), Some("admin"), &[], None).await;
        assert_eq!(page.json()["data"]["body"]["sections"][0]["props"]["heading"], "Sale tháng 7");
        let previous = turn["previous_revision_id"].as_str().unwrap();
        let etag = page.header(header::ETAG).to_owned();
        let undone = send(&app, "POST", &format!("/entries/{id}/revisions/{previous}/restore"), Some("admin"), &[("if-match", &etag)], None).await;
        assert_eq!(undone.status(), StatusCode::OK, "{}", undone.json());
        assert_eq!(undone.json()["data"]["body"]["sections"][0]["props"]["heading"], "Sale hè");
        let prompt = model.prompts.lock().unwrap()[1].clone();
        assert!(prompt.input.iter().any(|part| matches!(part, Part::Text(text) if text.contains("Selected blocks"))));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ai_dung_khoi_la_thi_thu_lai_roi_bao_loi_va_khong_ghi_de() {
    TestDatabase::run(async |test| {
        let bad = layout_answer(json!([{"id": "x", "block": "shop.unknown", "props": {}}]));
        let model = Arc::new(scripted(vec![bad]));
        let app = ai_app(test, model.clone()).await;
        let id = new_page(&app, "Lỗi").await;
        send(&app, "POST", &format!("/entries/{id}/ai/compose"), Some("admin"), &[], Some(json!({"instruction": "Trang lạ"}))).await;
        run_jobs(&app).await;
        let turn = thread(&app, &id).await;
        assert_eq!((turn["status"].as_str(), turn["error_code"].as_str(), turn["tries"].as_i64()), (Some("failed"), Some("content.ai_result_invalid"), Some(3)), "{turn}");
        assert_eq!(model.calls(), 3, "one call and two retries");
        let second = model.prompts.lock().unwrap()[1].clone();
        assert!(second.input.iter().any(|part| matches!(part, Part::Text(text) if text.contains("refused") && text.contains("shop.unknown"))));
        let page = send(&app, "GET", &format!("/entries/{id}"), Some("admin"), &[], None).await.json();
        assert_eq!(page["data"]["ai_assisted"], false, "nothing written");

        // Có người sửa trong lúc AI làm: kết quả không ghi đè.
        let id = new_page(&app, "Xung đột").await;
        send(&app, "POST", &format!("/entries/{id}/ai/compose"), Some("admin"), &[], Some(json!({"instruction": "Bất kỳ"}))).await;
        let etag = send(&app, "GET", &format!("/entries/{id}"), Some("admin"), &[], None).await.header(header::ETAG).to_owned();
        let edited = send(&app, "PATCH", &format!("/entries/{id}"), Some("admin"), &[("if-match", &etag)], Some(json!({"excerpt": "người sửa"}))).await;
        assert_eq!(edited.status(), StatusCode::OK);
        run_jobs(&app).await;
        let turn = thread(&app, &id).await;
        assert_eq!(turn["status"], "failed", "{turn}");
        assert!(matches!(turn["error_code"].as_str(), Some("content.ai_conflict" | "content.ai_result_invalid")), "{turn}");
    })
    .await
    .unwrap();
}
