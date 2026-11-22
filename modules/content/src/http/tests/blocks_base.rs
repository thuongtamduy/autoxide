//! Khối nền của `content` (bước 67): mọi khối lưu được với props đúng; luật riêng của video, cột,
//! bản ghi nhúng; vùng banner trả banner đang chạy theo lô; chân trang pháp lý tự gắn dưới trang và
//! không đặt được trong bố cục. Gate chạy trên cả SQLite và Postgres.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use autoxide_kernel::ai::{Embedder, TextModel};
use autoxide_kernel::blocks::{BlockCategory, BlockData, BlockRef, BlockRequest, BlockType};
use autoxide_kernel::error::Error;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_media::api::Media;
use autoxide_testkit::ai::{FakeEmbedder, FakeTextModel};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{StatusCode, header};
use serde_json::{Value, json};

use super::send;
use crate::testing::FakeMedia;

/// `compliance` giả: chỉ khai chân trang pháp lý và đếm số lần được hỏi.
struct Compliance {
    calls: Arc<AtomicUsize>,
}

struct Footer(Arc<AtomicUsize>);

#[async_trait::async_trait]
impl BlockData for Footer {
    async fn resolve(
        &self,
        _request: &BlockRequest<'_>,
        _block: &str,
        items: &[BlockRef],
    ) -> Result<Vec<Option<Value>>, Error> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(items.iter().map(|_| Some(json!({"owner": {"legal_name": "Cửa hàng A"}}))).collect())
    }
}

impl Module for Compliance {
    fn name(&self) -> &'static str {
        "compliance"
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", "{}")]
    }

    fn blocks(&self) -> Vec<BlockType> {
        // Nhãn mượn câu đã dịch của `content.section` (`cargo xtask check` đòi bản dịch).
        let footer = BlockType::new(
            "compliance.legal_footer",
            BlockCategory::Legal,
            "Section",
            "Use to group blocks into a band of the page with its own background and spacing.",
            r#"{"type": "object", "additionalProperties": false, "properties": {}}"#,
        );
        vec![footer.with_data()]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.block_data("compliance.legal_footer", Arc::new(Footer(self.calls.clone())))
    }
}

async fn app(test: &TestDatabase, calls: &Arc<AtomicUsize>) -> TestApp {
    let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
    let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
    let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
    TestApp::builder(test)
        .user(
            "admin",
            1,
            &[
                "content.read",
                "content.write",
                "content.publish",
                "content.manage",
                "content.banners",
            ],
        )
        .provide::<dyn Media>(media)
        .provide::<dyn TextModel>(text)
        .provide::<dyn Embedder>(embedder)
        .module(crate::module())
        .module(Compliance { calls: calls.clone() })
        .build()
        .await
        .unwrap()
}

fn layout(sections: Value) -> Value {
    json!({"layout": {"type": "layout", "schema": 1, "sections": sections}})
}

async fn page(app: &TestApp, title: &str) -> (String, String) {
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
    let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
    let read = send(app, "GET", &format!("/entries/{id}"), Some("admin"), &[], None).await;
    (id, read.header(header::ETAG).to_owned())
}

fn codes(body: &Value) -> Vec<(String, String)> {
    body["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["field"].as_str().unwrap().to_owned(), e["code"].as_str().unwrap().to_owned()))
        .collect()
}

#[tokio::test]
async fn khoi_nen_luu_duoc_va_luat_rieng_cua_khoi() {
    TestDatabase::run(async |test| {
        let calls = Arc::new(AtomicUsize::new(0));
        let app = app(test, &calls).await;
        let listed = app.get("/api/v1/kernel/blocks").token("admin").send().await.unwrap().json();
        let names: Vec<&str> =
            listed["data"].as_array().unwrap().iter().map(|b| b["name"].as_str().unwrap()).collect();
        for name in [
            "content.columns", "content.spacer", "content.divider", "content.text", "content.button",
            "content.faq", "content.image", "content.gallery", "content.video", "content.hero",
            "content.announcement_bar", "content.countdown", "content.popup", "content.embed",
            "content.search_box", "content.placement",
        ] {
            assert!(names.contains(&name), "{name} in {names:?}");
        }

        let (id, etag) = page(&app, "Ra mắt").await;
        let uri = format!("/entries/{id}/layout");
        let good = layout(json!([
            {"id": "bar", "block": "content.announcement_bar", "props": {"text": "Miễn phí giao hàng", "link": "/ship", "tone": "primary"}},
            {"id": "hero", "block": "content.hero", "props": {"heading": "Bộ sưu tập hè", "button_label": "Xem", "button_link": "/he", "height": {"base": "medium", "lg": "screen"}}},
            {"id": "s1", "block": "content.section", "children": [
                {"id": "cols", "block": "content.columns", "props": {"widths": "equal"}, "children": [
                    {"id": "t1", "block": "content.text", "props": {"text": "Đoạn một.\n\nĐoạn hai."}},
                    {"id": "b1", "block": "content.button", "props": {"label": "Mua", "link": "https://example.com/a", "style": "primary"}}
                ]},
                {"id": "sp", "block": "content.spacer", "props": {"size": "large"}},
                {"id": "dv", "block": "content.divider"},
                {"id": "faq", "block": "content.faq", "props": {"items": [{"question": "Giao bao lâu?", "answer": "2 ngày."}]}},
                {"id": "img", "block": "content.image", "props": {"alt": "Chưa chọn ảnh"}},
                {"id": "vid", "block": "content.video", "props": {"embed_url": "https://www.youtube.com/embed/x"}},
                {"id": "cd", "block": "content.countdown", "props": {"ends_at": "2030-01-01T00:00:00Z", "label": "Còn"}},
                {"id": "pop", "block": "content.popup", "props": {"heading": "Nhận tin", "consent_text": "Tôi đồng ý nhận email.", "policy_link": "/privacy", "frequency": "weekly"}},
                {"id": "q", "block": "content.search_box"}
            ]}
        ]));
        let saved = send(&app, "PUT", &uri, Some("admin"), &[("if-match", &etag)], Some(good)).await;
        assert_eq!(saved.status(), StatusCode::OK, "{}", saved.json());
        let etag = saved.header(header::ETAG).to_owned();
        assert!(saved.json()["data"]["plain_text"].as_str().is_none_or(|text| !text.contains("<")));

        // Luật riêng, mọi lỗi một lần.
        let bad = layout(json!([
            {"id": "v1", "block": "content.video", "props": {"embed_url": "https://evil.example/x"}},
            {"id": "v2", "block": "content.video", "props": {}},
            {"id": "c1", "block": "content.columns", "children": [{"id": "only", "block": "content.spacer"}]},
            {"id": "pop", "block": "content.popup", "props": {"heading": "Nhận tin"}},
            {"id": "b", "block": "content.button", "props": {"label": "x", "link": "javascript:alert(1)"}},
            {"id": "lf", "block": "compliance.legal_footer"}
        ]));
        let refused = send(&app, "PUT", &uri, Some("admin"), &[("if-match", &etag)], Some(bad)).await;
        assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY, "{}", refused.json());
        let found = codes(&refused.json());
        for (field, code) in [
            ("body.sections[0].props.embed_url", "host_not_allowed"),
            ("body.sections[1].props", "invalid"),
            ("body.sections[2].children", "range"),
            ("body.sections[3].props.consent_text", "required"),
            ("body.sections[3].props.policy_link", "required"),
            ("body.sections[5].block", "not_allowed"),
        ] {
            assert!(found.contains(&(field.to_owned(), code.to_owned())), "{field} {code} in {found:?}");
        }
        assert!(found.iter().any(|(field, _)| field == "body.sections[4].props.link"), "{found:?}");

        // Bản ghi nhúng: loại không có trong sổ đối tượng bị từ chối như node `embed` của thân `rich`.
        let embed = layout(json!([{"id": "e", "block": "content.embed", "props": {"kind": "nope.thing", "id": "1"}}]));
        let refused = send(&app, "PUT", &uri, Some("admin"), &[("if-match", &etag)], Some(embed)).await;
        assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY, "{}", refused.json());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn vung_banner_va_chan_trang_phap_ly_khi_doc_cong_khai() {
    TestDatabase::run(async |test| {
        let calls = Arc::new(AtomicUsize::new(0));
        let app = app(test, &calls).await;
        let presets = send(&app, "POST", "/placements/presets", Some("admin"), &[], None).await;
        assert_eq!(presets.status(), StatusCode::OK, "{}", presets.json());
        let banner = json!({"placement": "top_strip", "kind": "text", "title": "Sale hè", "status": "active", "link": {"url": "/sale"}});
        let created = send(&app, "POST", "/banners", Some("admin"), &[], Some(banner)).await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());

        let (id, etag) = page(&app, "Khuyến mại").await;
        let body = layout(json!([
            {"id": "top", "block": "content.placement", "props": {"key": "top_strip"}},
            {"id": "again", "block": "content.placement", "props": {"key": "top_strip"}},
            {"id": "none", "block": "content.placement", "props": {"key": "missing_key"}}
        ]));
        let saved = send(&app, "PUT", &format!("/entries/{id}/layout"), Some("admin"), &[("if-match", &etag)], Some(body)).await;
        assert_eq!(saved.status(), StatusCode::OK, "{}", saved.json());
        let etag = saved.header(header::ETAG).to_owned();
        let published = send(&app, "POST", &format!("/entries/{id}/publish"), Some("admin"), &[("if-match", &etag)], None).await;
        assert_eq!(published.status(), StatusCode::OK, "{}", published.json());

        let read = send(&app, "GET", "/public/resolve?locale=en&path=/khuyen-mai", None, &[], None).await;
        assert_eq!(read.status(), StatusCode::OK, "{}", read.json());
        let entry = &read.json()["data"]["entry"];
        assert_eq!(entry["blocks_data"]["top"]["banners"][0]["title"], "Sale hè", "{entry}");
        assert_eq!(entry["blocks_data"]["again"], entry["blocks_data"]["top"]);
        assert_eq!(entry["blocks_data"]["none"], Value::Null);
        // Chân trang pháp lý tự gắn ngoài bố cục, một lần hỏi `compliance`.
        assert_eq!(entry["legal_footer"]["owner"]["legal_name"], "Cửa hàng A", "{entry}");
        assert!(entry["blocks_data"].get("$legal_footer").is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    })
    .await
    .unwrap();
}
