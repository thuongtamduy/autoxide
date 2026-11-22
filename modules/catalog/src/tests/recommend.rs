//! Bán chạy, điểm đánh giá, thường mua cùng (từ event của `orders`) và sản phẩm tương tự (D-201,
//! B-c14). Module `orders` thật đăng ký sau `catalog` và phụ thuộc nó, nên ở đây một module giả
//! tên `orders` phát đúng event và payload mà `orders` phát.

use std::sync::{Arc, Mutex};

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, EventInfo, Outbox};
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_testkit::ai::FakeEmbedder;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use super::public::{names, on_sale, price_list, public};
use super::{app, app_with, call};
use crate::api::events::{ORDERS_ORDER_COMPLETED, ORDERS_REVIEW_PUBLISHED};

/// Module giả tên `orders`: khai event, giữ `Outbox` để test ghi event như module thật.
struct FakeOrders(Arc<Mutex<Option<Outbox>>>);

impl Module for FakeOrders {
    fn name(&self) -> &'static str {
        "orders"
    }
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            autoxide_kernel::event::outbox_migration("m0001_events", "orders"),
            autoxide_kernel::event::outbox_actor_migration("m0002_event_actor", "orders"),
            autoxide_kernel::tenancy::migration::adopt(
                "m0003_tenancy_events",
                vec![autoxide_kernel::event::tenant_table("orders")],
            ),
            autoxide_kernel::event::outbox_access_migration("k0001_event_access", "orders"),
        ]
    }
    fn events(&self) -> Vec<EventInfo> {
        vec![
            EventInfo::new(ORDERS_ORDER_COMPLETED, "Test event."),
            EventInfo::new(ORDERS_REVIEW_PUBLISHED, "Test event."),
        ]
    }
    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(context.outbox()?);
        Ok(())
    }
}

#[derive(Debug, serde::Serialize)]
#[serde(transparent)]
struct Completed(Value);

impl autoxide_kernel::event::Event for Completed {
    const NAME: &'static str = ORDERS_ORDER_COMPLETED;
}

#[derive(Debug, serde::Serialize)]
#[serde(transparent)]
struct Reviewed(Value);

impl autoxide_kernel::event::Event for Reviewed {
    const NAME: &'static str = ORDERS_REVIEW_PUBLISHED;
}

async fn emit<E: autoxide_kernel::event::Event>(
    app: &TestApp,
    outbox: &Arc<Mutex<Option<Outbox>>>,
    event: &E,
) {
    let outbox = outbox.lock().unwrap().clone().unwrap();
    let transaction = app.database().begin_write().await.unwrap();
    let tenant = app.default_tenant();
    outbox.record(&transaction, &tenant, Actor::system(), None, event).await.unwrap();
    transaction.commit().await.unwrap();
}

/// Chạy hết event và job đang chờ.
async fn settle(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap().blocked_delay(std::time::Duration::ZERO);
    let dispatcher = app.modules().dispatcher().unwrap();
    for _ in 0..10 {
        let events = dispatcher.run_once().await.unwrap();
        if worker.run_once().await.unwrap() + events == 0 {
            break;
        }
    }
}

/// Thân `orders.order_completed` như module `orders` ghi (chỉ các trường `catalog` đọc).
fn completed(order: Uuid, lines: &[(&str, i64)]) -> Completed {
    let lines: Vec<Value> = lines
        .iter()
        .map(|(product, quantity)| json!({"product_id": product, "quantity": quantity}))
        .collect();
    Completed(json!({"after": {"order_id": order, "status": "completed", "lines": lines}}))
}

/// Đơn hoàn tất đếm bán chạy và "thường mua cùng"; nhận lại event không đếm hai lần; tổng đánh
/// giá từ event đánh giá sắp được `sort=rating`.
#[tokio::test]
async fn ban_chay_mua_cung_va_diem_tu_event_cua_orders() {
    TestDatabase::run(async |test| {
        let outbox = Arc::new(Mutex::new(None));
        let fake = FakeOrders(outbox.clone());
        let (app, tree) = app_with(test, |builder| builder.module(fake)).await;
        let list = price_list(&app, tree.chain).await;
        let (a, _) = on_sale(&app, &list, tree.chain, "R-A", "Bàn chải", 30_000).await;
        let (b, _) = on_sale(&app, &list, tree.chain, "R-B", "Kem đánh răng", 40_000).await;
        let (c, _) = on_sale(&app, &list, tree.chain, "R-C", "Chỉ nha khoa", 20_000).await;
        settle(&app).await;

        let first = completed(Uuid::now_v7(), &[(&a, 2), (&b, 1)]);
        emit(&app, &outbox, &first).await;
        emit(&app, &outbox, &completed(Uuid::now_v7(), &[(&a, 1), (&c, 1), (&a, 1)])).await;
        // Nhận lại cùng đơn (cùng `order_id`): không đếm thêm.
        emit(&app, &outbox, &first).await;
        settle(&app).await;

        let site = tree.chain.as_uuid();
        let uri = format!("/products?locale=vi&site={site}&sort=best_selling");
        let (status, best, _) = public(&app, &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{best}");
        assert_eq!(names(&best)[0], "Bàn chải");
        let sold: Vec<i64> = crate::entity::product_stat::Entity::find_units(&app).await;
        assert_eq!(sold, vec![1, 1, 4]);

        let uri = format!("/products/{a}/frequently-bought-together?locale=vi&site={site}");
        let (status, together, _) = public(&app, &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{together}");
        let mut found = names(&together);
        found.sort();
        assert_eq!(found, vec!["Chỉ nha khoa", "Kem đánh răng"]);
        let uri = format!("/products/{b}/frequently-bought-together?locale=vi&site={site}");
        let (_, together, _) = public(&app, &uri, None).await;
        assert_eq!(names(&together), vec!["Bàn chải"]);

        // Tổng đánh giá của "Chỉ nha khoa": 2 đánh giá, 9 sao.
        let review = json!({"after": {"product_id": c, "product_rating": {"count": 2, "sum": 9}}});
        emit(&app, &outbox, &Reviewed(review)).await;
        settle(&app).await;
        let uri = format!("/products?locale=vi&site={site}&sort=rating");
        let (_, rated, _) = public(&app, &uri, None).await;
        assert_eq!(names(&rated)[0], "Chỉ nha khoa");
        assert_eq!(rated["data"][0]["rating"], json!(4.5));
        assert_eq!(rated["data"][0]["rating_count"], 2);

        // Lọc theo số sao tối thiểu dùng cùng cột điểm.
        let (_, rebuilt, _) =
            public(&app, &format!("/products?locale=vi&site={site}&min_rating=4"), None).await;
        assert_eq!(names(&rebuilt), vec!["Chỉ nha khoa"]);
    })
    .await
    .unwrap();
}

/// Sản phẩm tương tự theo embedding: hàng cùng nghĩa trước, hàng không liên quan không có; thôi
/// bán thì rời danh sách. Sản phẩm không hiện ở website: 404.
#[tokio::test]
async fn san_pham_tuong_tu_theo_y_nghia() {
    TestDatabase::run(async |test| {
        let embedder = Arc::new(FakeEmbedder::new(64));
        let provided = embedder.clone();
        let (app, tree) = app_with(test, |builder| {
            builder.provide::<dyn autoxide_kernel::ai::Embedder>(provided)
        })
        .await;
        let list = price_list(&app, tree.chain).await;
        let (white, _) = on_sale(&app, &list, tree.chain, "T-1", "Áo thun cotton trắng", 1).await;
        let (black, _) = on_sale(&app, &list, tree.chain, "T-2", "Áo thun cotton đen", 1).await;
        on_sale(&app, &list, tree.chain, "T-3", "Nồi cơm điện", 1).await;
        settle(&app).await;
        assert!(embedder.calls() > 0);

        let site = tree.chain.as_uuid();
        let uri = format!("/products/{white}/similar?locale=vi&site={site}");
        let (status, similar, _) = public(&app, &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{similar}");
        assert_eq!(names(&similar), vec!["Áo thun cotton đen"]);

        // Thôi bán "đen": rời danh sách của "trắng".
        let body = json!({"status": "inactive"});
        let (status, _) =
            call(&app, "POST", &format!("/products/{black}/status"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        settle(&app).await;
        let (_, similar, _) = public(&app, &uri, None).await;
        assert!(names(&similar).is_empty(), "{similar}");

        let uri = format!("/products/{black}/similar?locale=vi&site={site}");
        let (status, problem, _) = public(&app, &uri, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{problem}");
        let uri = format!("/products/{white}/similar?locale=vi&site={site}&limit=50");
        let (status, _, _) = public(&app, &uri, None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

/// Không có embedding: sản phẩm cùng danh mục, đang hiện ở website.
#[tokio::test]
async fn tuong_tu_khong_embedding_thi_cung_danh_muc() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let body = json!({"texts": {"vi": {"name": "Đồ bếp"}}});
        let (_, made) = call(&app, "POST", "/categories", "boss", Some(body)).await;
        let kitchen = made["data"]["id"].as_str().unwrap().to_owned();
        let list = price_list(&app, tree.chain).await;
        let (pot, _) = on_sale(&app, &list, tree.chain, "K-1", "Nồi", 1).await;
        let (pan, _) = on_sale(&app, &list, tree.chain, "K-2", "Chảo", 1).await;
        on_sale(&app, &list, tree.chain, "K-3", "Gối", 1).await;
        for product in [&pot, &pan] {
            let body = json!({"category_ids": [kitchen]});
            let uri = format!("/products/{product}/categories");
            let (status, _) = call(&app, "PUT", &uri, "boss", Some(body)).await;
            assert_eq!(status, StatusCode::OK);
        }
        settle(&app).await;
        let site = tree.chain.as_uuid();
        let uri = format!("/products/{pot}/similar?locale=vi&site={site}");
        let (status, similar, _) = public(&app, &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{similar}");
        assert_eq!(names(&similar), vec!["Chảo"]);
    })
    .await
    .unwrap();
}

impl crate::entity::product_stat::Entity {
    /// Số bán của mọi sản phẩm của khách mặc định, tăng dần.
    async fn find_units(app: &TestApp) -> Vec<i64> {
        use sea_orm::EntityTrait as _;
        let mut units: Vec<i64> = Self::find()
            .all(app.database().read())
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.units_sold)
            .collect();
        units.sort_unstable();
        units
    }
}

/// `colors.len()` sản phẩm giống nhau; mỗi sản phẩm thêm `extra` job lập index trùng (nhiều thay đổi
/// cùng sản phẩm); worker 8 slot chạy các job đan xen. Sau đó mỗi sản phẩm phải thấy đủ mọi hàng
/// còn lại.
async fn index_in_parallel(test: &TestDatabase, colors: &[&str], extra: usize) {
    let embedder = Arc::new(FakeEmbedder::new(64));
    let provided = embedder.clone();
    let (app, tree) =
        app_with(test, |builder| builder.provide::<dyn autoxide_kernel::ai::Embedder>(provided))
            .await;
    let list = price_list(&app, tree.chain).await;
    let mut products = Vec::new();
    for (index, color) in colors.iter().enumerate() {
        let name = format!("Áo thun cotton màu {color}");
        let (id, _) = on_sale(&app, &list, tree.chain, &format!("P-{index}"), &name, 1).await;
        products.push((id, name));
    }
    if extra > 0 {
        let tenant = app.default_tenant();
        let queue = autoxide_kernel::job::JobQueue::new("catalog");
        for (id, _) in &products {
            let job = crate::similar::IndexJob { products: vec![id.parse().unwrap()] };
            for _ in 0..extra {
                let options = autoxide_kernel::job::EnqueueOptions::default();
                queue.enqueue(tenant.database().write(), &tenant, &job, options).await.unwrap();
            }
        }
    }
    let worker = app.modules().worker(8).unwrap().blocked_delay(std::time::Duration::ZERO);
    let dispatcher = app.modules().dispatcher().unwrap();
    for _ in 0..10 {
        let events = dispatcher.run_once().await.unwrap();
        if worker.run_once().await.unwrap() + events == 0 {
            break;
        }
    }
    let site = tree.chain.as_uuid();
    for (id, name) in &products {
        let uri = format!("/products/{id}/similar?locale=vi&site={site}");
        let (status, similar, _) = public(&app, &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{similar}");
        let mut got = names(&similar);
        got.sort();
        let mut want: Vec<String> =
            products.iter().map(|(_, other)| other.clone()).filter(|o| o != name).collect();
        want.sort();
        assert_eq!(got, want, "{name}");
    }
}

/// Nhiều job lập index chạy song song (worker nhiều slot): mỗi sản phẩm vẫn thấy mọi hàng giống nó.
/// Trước đây job đọc embedding qua kết nối đọc rồi mới mở transaction ghi, nên job chạy sau có thể
/// xoá dòng đối xứng job kia vừa ghi — danh sách trả `[]` ngẫu nhiên.
#[tokio::test]
async fn tuong_tu_dung_khi_nhieu_job_chay_song_song() {
    TestDatabase::run(async |test| {
        let colors = ["đỏ", "xanh", "vàng", "tím", "hồng", "nâu", "xám", "cam"];
        index_in_parallel(test, &colors, 0).await;
    })
    .await
    .unwrap();
}

/// Hai (ba) job của CÙNG một sản phẩm chạy cùng lúc: ghi lại vector là xoá + chèn (đổi ID dòng), nên
/// khoá theo dòng embedding có thể trượt — danh sách vẫn phải đủ.
#[tokio::test]
async fn tuong_tu_dung_khi_hai_job_cung_san_pham() {
    TestDatabase::run(async |test| {
        let colors = ["đỏ", "xanh", "vàng", "tím", "hồng", "nâu"];
        index_in_parallel(test, &colors, 2).await;
    })
    .await
    .unwrap();
}

/// Bảng embedding còn rỗng: hai sản phẩm đầu tiên lập index cùng lúc (chưa có dòng nào để khoá)
/// vẫn thấy nhau.
#[tokio::test]
async fn tuong_tu_dung_khi_bang_rong() {
    TestDatabase::run(async |test| {
        index_in_parallel(test, &["đỏ", "xanh"], 0).await;
    })
    .await
    .unwrap();
}
