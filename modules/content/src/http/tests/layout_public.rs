//! Đọc công khai trang dựng từ khối (bước 67): `GET /public/resolve` trả bố cục kèm `blocks_data`,
//! mỗi loại khối một lần gọi (không N+1), khối lỗi không hỏng trang, `ETag` đổi theo dữ liệu khối;
//! đo p99 của trang 50 khối. Gate chạy trên cả SQLite và Postgres.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use autoxide_kernel::ai::{Embedder, TextModel};
use autoxide_kernel::blocks::{BlockCategory, BlockData, BlockRef, BlockRequest, BlockType};
use autoxide_kernel::error::Error;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_media::api::Media;
use autoxide_testkit::ai::{FakeEmbedder, FakeTextModel};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{StatusCode, header};
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};

use super::send;
use crate::testing::FakeMedia;

const GRID: &str = r#"{"type": "object", "additionalProperties": false,
    "properties": {"limit": {"type": "integer", "minimum": 1, "maximum": 24}, "fail": {"type": "boolean"}}}"#;

/// Khối có dữ liệu giả: mỗi lần gọi đọc database một lần (như một module thật), trả giá hiện tại.
struct Grid {
    calls: Arc<AtomicUsize>,
    price: Arc<AtomicI64>,
}

#[async_trait::async_trait]
impl BlockData for Grid {
    async fn resolve(
        &self,
        request: &BlockRequest<'_>,
        _block: &str,
        items: &[BlockRef],
    ) -> Result<Vec<Option<Value>>, Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let read = request.tenant.database().read();
        read.query_one_raw(autoxide_kernel::database::statement(
            read.get_database_backend(),
            "SELECT 1",
            Vec::<sea_orm::Value>::new(),
        ))
        .await?;
        if items.iter().any(|item| item.props["fail"] == true) {
            return Err(Error::internal("the shop is down"));
        }
        let price = self.price.load(Ordering::SeqCst);
        Ok(items
            .iter()
            .map(|item| Some(json!({"limit": item.props["limit"], "price": price})))
            .collect())
    }
}

struct Shop {
    calls: Arc<AtomicUsize>,
    price: Arc<AtomicI64>,
}

impl Module for Shop {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", "{}")]
    }

    fn blocks(&self) -> Vec<BlockType> {
        // Nhãn mượn câu đã dịch của `content.section`: `cargo xtask check` đòi bản dịch cho mọi
        // `BlockType::new` trong `src/`, kể cả module thử.
        let grid = BlockType::new(
            "shop.grid",
            BlockCategory::Commerce,
            "Section",
            "Use to group blocks into a band of the page with its own background and spacing.",
            GRID,
        );
        let broken = BlockType::new(
            "shop.broken",
            BlockCategory::Commerce,
            "Section",
            "Use to group blocks into a band of the page with its own background and spacing.",
            GRID,
        );
        vec![grid.with_data(), broken.with_data()]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let grid = Arc::new(Grid { calls: self.calls.clone(), price: self.price.clone() });
        context.block_data("shop.grid", grid.clone())?;
        context.block_data("shop.broken", grid)
    }
}

async fn app(test: &TestDatabase, calls: &Arc<AtomicUsize>, price: &Arc<AtomicI64>) -> TestApp {
    let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
    let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
    let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
    TestApp::builder(test)
        .setting("AUTOXIDE_CONTENT_PREVIEW_SECRET", super::SECRET)
        .user("admin", 1, &["content.read", "content.write", "content.publish"])
        .provide::<dyn Media>(media)
        .provide::<dyn TextModel>(text)
        .provide::<dyn Embedder>(embedder)
        // `content` đăng ký TRƯỚC `shop`: sổ khối vẫn thấy khối của module đăng ký sau.
        .module(crate::module())
        .module(Shop { calls: calls.clone(), price: price.clone() })
        .build()
        .await
        .unwrap()
}

/// Trang 50 khối: 5 phần, mỗi phần một tiêu đề, bốn lưới sản phẩm và năm tiêu đề phụ.
fn fifty_blocks() -> Value {
    let sections: Vec<Value> = (0..5)
        .map(|s| {
            let mut children = vec![json!({"id": format!("h{s}"), "block": "content.heading", "props": {"text": format!("Phần {s}")}})];
            for g in 0..4 {
                children.push(json!({"id": format!("g{s}-{g}"), "block": "shop.grid", "props": {"limit": g + 1}}));
            }
            for t in 0..4 {
                children.push(json!({"id": format!("t{s}-{t}"), "block": "content.heading", "props": {"text": "Ưu đãi", "level": 3}}));
            }
            json!({"id": format!("s{s}"), "block": "content.section", "children": children})
        })
        .collect();
    json!({"layout": {"type": "layout", "schema": 1, "sections": sections}})
}

#[tokio::test]
async fn doc_cong_khai_tra_du_lieu_khoi_theo_lo() {
    TestDatabase::run(async |test| {
        let calls = Arc::new(AtomicUsize::new(0));
        let price = Arc::new(AtomicI64::new(100_000));
        let app = app(test, &calls, &price).await;
        let created = send(
            &app,
            "POST",
            "/entries",
            Some("admin"),
            &[],
            Some(json!({"kind": "page", "title": "Sale"})),
        )
        .await;
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let etag = send(&app, "GET", &format!("/entries/{id}"), Some("admin"), &[], None)
            .await
            .header(header::ETAG)
            .to_owned();
        let saved = send(
            &app,
            "PUT",
            &format!("/entries/{id}/layout"),
            Some("admin"),
            &[("if-match", &etag)],
            Some(fifty_blocks()),
        )
        .await;
        assert_eq!(saved.status(), StatusCode::OK, "{}", saved.json());
        let etag = saved.header(header::ETAG).to_owned();
        let published = send(
            &app,
            "POST",
            &format!("/entries/{id}/publish"),
            Some("admin"),
            &[("if-match", &etag)],
            None,
        )
        .await;
        assert_eq!(published.status(), StatusCode::OK, "{}", published.json());

        // Hai mươi khối lưới: MỘT lần gọi `BlockData`.
        let path = "/public/resolve?locale=en&path=/sale";
        calls.store(0, Ordering::SeqCst);
        let found = send(&app, "GET", path, None, &[], None).await;
        assert_eq!(found.status(), StatusCode::OK, "{}", found.json());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let data = found.json()["data"]["entry"]["blocks_data"].clone();
        assert_eq!(data.as_object().unwrap().len(), 20);
        assert_eq!(data["g0-3"], json!({"limit": 4, "price": 100_000}));
        assert!(data.get("h0").is_none(), "headings carry no data");

        // ETag theo cả dữ liệu khối: giá đổi thì ETag đổi, không đổi thì 304.
        let first = found.header(header::ETAG).to_owned();
        let same = send(&app, "GET", path, None, &[("if-none-match", &first)], None).await;
        assert_eq!(same.status(), StatusCode::NOT_MODIFIED);
        price.store(90_000, Ordering::SeqCst);
        let cheaper = send(&app, "GET", path, None, &[("if-none-match", &first)], None).await;
        assert_eq!(cheaper.status(), StatusCode::OK);
        assert_eq!(cheaper.json()["data"]["entry"]["blocks_data"]["g0-0"]["price"], 90_000);

        // Đo p99 của trang 50 khối (ghi vào D-20261122-layout-public; không chặn theo máy).
        let mut times = Vec::new();
        for _ in 0..200 {
            let started = Instant::now();
            let answer = send(&app, "GET", path, None, &[], None).await;
            times.push(started.elapsed());
            assert_eq!(answer.status(), StatusCode::OK);
        }
        times.sort();
        let p99 = times[197];
        eprintln!("content public resolve, 50 blocks: p50 = {:?}, p99 = {p99:?}", times[100]);
        assert!(p99 < Duration::from_secs(2), "p99 = {p99:?}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khoi_loi_khong_lam_hong_trang() {
    TestDatabase::run(async |test| {
        let calls = Arc::new(AtomicUsize::new(0));
        let price = Arc::new(AtomicI64::new(1));
        let app = app(test, &calls, &price).await;
        let created = send(
            &app,
            "POST",
            "/entries",
            Some("admin"),
            &[],
            Some(json!({"kind": "page", "title": "Mix"})),
        )
        .await;
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let etag = send(&app, "GET", &format!("/entries/{id}"), Some("admin"), &[], None)
            .await
            .header(header::ETAG)
            .to_owned();
        let layout = json!({"layout": {"type": "layout", "schema": 1, "sections": [
            {"id": "ok", "block": "shop.grid", "props": {"limit": 2}},
            {"id": "down", "block": "shop.broken", "props": {"limit": 1, "fail": true}},
            {"id": "title", "block": "content.heading", "props": {"text": "Xin chào"}}
        ]}});
        let saved = send(
            &app,
            "PUT",
            &format!("/entries/{id}/layout"),
            Some("admin"),
            &[("if-match", &etag)],
            Some(layout),
        )
        .await;
        assert_eq!(saved.status(), StatusCode::OK, "{}", saved.json());

        // Xem trước bản nháp: cùng dữ liệu khối như trang công khai.
        let token =
            send(&app, "POST", &format!("/entries/{id}/preview-token"), Some("admin"), &[], None)
                .await
                .json();
        let token = token["data"]["token"].as_str().unwrap().to_owned();
        let preview =
            send(&app, "GET", &format!("/public/preview?token={token}"), None, &[], None).await;
        assert_eq!(preview.json()["data"]["blocks_data"]["ok"]["limit"], 2);

        let etag = saved.header(header::ETAG).to_owned();
        send(
            &app,
            "POST",
            &format!("/entries/{id}/publish"),
            Some("admin"),
            &[("if-match", &etag)],
            None,
        )
        .await;
        let found = send(&app, "GET", "/public/resolve?locale=en&path=/mix", None, &[], None).await;
        assert_eq!(found.status(), StatusCode::OK, "{}", found.json());
        let entry = &found.json()["data"]["entry"];
        assert_eq!(entry["blocks_data"]["ok"]["limit"], 2);
        assert_eq!(entry["blocks_data"]["down"], Value::Null);
        assert_eq!(entry["body"]["sections"].as_array().unwrap().len(), 3);
    })
    .await
    .unwrap();
}
