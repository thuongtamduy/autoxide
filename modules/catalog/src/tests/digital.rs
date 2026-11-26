//! Hàng số: gắn file (chép từ `files`), chặn mở bán khi chưa có file, phát quyền tải từ event của
//! `orders` (nhận lại không phát thêm), trừ lượt đồng thời không vượt giới hạn, địa chỉ ký hết hạn
//! hay bị thu hồi, địa chỉ ký của khách này không dùng được ở khách khác.

use autoxide_kernel::time::Timestamp;
use std::sync::{Arc, Mutex};

use autoxide_files::api::{FileInfo, Files, OpenedFile};
use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, EventInfo, Outbox};
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::storage::Storage;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_testkit::{MemoryStorage, TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use bytes::Bytes;
use futures_util::StreamExt as _;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{app_with, call, product_body};
use crate::api::events::{ORDERS_ORDER_CANCELLED, ORDERS_ORDER_CONFIRMED};

const CONTENT: &[u8] = b"0123456789 hello digital world";
/// File người bán đã tải lên qua `files`.
const FILE_ID: Uuid = Uuid::from_u128(0x0193_0000_0000_7000_8000_0000_0000_0042);
/// Người mua có tài khoản.
const BUYER: u128 = 9;

/// `dyn Files` giả: một file, ai có `catalog.products.write` cũng được xem.
struct FakeFiles;

#[async_trait::async_trait]
impl Files for FakeFiles {
    async fn find(
        &self,
        _tenant: &TenantContext,
        _principal: &Principal,
        id: Uuid,
    ) -> Result<FileInfo, Error> {
        if id != FILE_ID {
            return Err(Error::internal("the fake holds one file only"));
        }
        Ok(FileInfo::new(id, "Sách điện tử.pdf", "application/pdf", 30, "ab", Timestamp::now()))
    }

    async fn open(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<OpenedFile, Error> {
        let info = self.find(tenant, principal, id).await?;
        let content = futures_util::stream::iter(vec![Ok(Bytes::from_static(CONTENT))]).boxed();
        Ok(OpenedFile::new(info, content))
    }
}

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
            EventInfo::new(ORDERS_ORDER_CONFIRMED, "Test event."),
            EventInfo::new(ORDERS_ORDER_CANCELLED, "Test event."),
        ]
    }
    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(context.outbox()?);
        Ok(())
    }
}

#[derive(Debug, serde::Serialize)]
#[serde(transparent)]
struct Confirmed(Value);

impl autoxide_kernel::event::Event for Confirmed {
    const NAME: &'static str = ORDERS_ORDER_CONFIRMED;
}

#[derive(Debug, serde::Serialize)]
#[serde(transparent)]
struct Cancelled(Value);

impl autoxide_kernel::event::Event for Cancelled {
    const NAME: &'static str = ORDERS_ORDER_CANCELLED;
}

struct Shop {
    app: TestApp,
    outbox: Arc<Mutex<Option<Outbox>>>,
    variant: String,
    product: String,
    chain: autoxide_kernel::tenancy::OrgId,
}

/// App có nơi lưu, `dyn Files` giả, `orders` giả, khách `b` trên tên miền `b.example`; một sản
/// phẩm hàng số nháp của chuỗi.
async fn shop(test: &TestDatabase) -> Shop {
    let outbox = Arc::new(Mutex::new(None));
    let fake = FakeOrders(outbox.clone());
    let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
    let (app, tree) = app_with(test, move |builder| {
        builder
            .provide::<dyn Storage>(storage)
            .provide::<dyn Files>(Arc::new(FakeFiles))
            .module(fake)
            .user(&BUYER.to_string(), BUYER, &[])
            .domain("default", "a.example")
            .tenant("b")
            .domain("b", "b.example")
    })
    .await;
    let mut body = product_body(tree.chain, "EBOOK", "Sách điện tử");
    body["kind"] = json!("digital");
    body["variant"] = json!({"sku": "EBOOK"});
    let (status, made) = call(&app, "POST", "/products", "boss", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    let product = made["data"]["id"].as_str().unwrap().to_owned();
    let variant = made["data"]["variants"][0]["id"].as_str().unwrap().to_owned();
    Shop { app, outbox, variant, product, chain: tree.chain }
}

async fn emit<E: autoxide_kernel::event::Event>(shop: &Shop, event: &E) {
    let outbox = shop.outbox.lock().unwrap().clone().unwrap();
    let transaction = shop.app.database().begin_write().await.unwrap();
    let tenant = shop.app.default_tenant();
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

/// Gắn file vào biến thể với `limit` lượt.
async fn attach(shop: &Shop, limit: i64) -> Value {
    let uri = format!("/variants/{}/digital-assets", shop.variant);
    let body = json!({"file_id": FILE_ID, "download_limit": limit, "grant_days": 7});
    let (status, made) = call(&shop.app, "POST", &uri, "boss", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    made["data"].clone()
}

/// Đơn đã trả tiền mua biến thể hàng số: (ID đơn, event).
fn paid_order(shop: &Shop, buyer: Option<u128>) -> (Uuid, Confirmed) {
    let order = Uuid::now_v7();
    let lines = json!([{"line_id": Uuid::now_v7(), "variant_id": shop.variant, "quantity": 1}]);
    let buyer = buyer.map(Uuid::from_u128);
    let after = json!({"order_id": order, "payment_status": "paid", "buyer_user_id": buyer, "lines": lines});
    (order, Confirmed(json!({"after": after})))
}

/// Token mới của đơn (người quản lý cấp lại).
async fn token_of(shop: &Shop, order: Uuid) -> String {
    let uri = format!("/orders/{order}/downloads/reissue");
    let (status, issued) = call(&shop.app, "POST", &uri, "boss", None).await;
    assert_eq!(status, StatusCode::OK, "{issued}");
    let path = issued["data"][0]["token_path"].as_str().unwrap();
    path.rsplit('/').next().unwrap().to_owned()
}

async fn spend(app: &TestApp, token: &str, host: Option<&str>) -> (StatusCode, Value) {
    let mut request =
        app.request(Method::POST, &format!("/api/v1/catalog/public/downloads/{token}"));
    request = request.host(host.unwrap_or("a.example"));
    let answer = request.send().await.unwrap();
    let status = answer.status();
    (status, if answer.body().is_empty() { Value::Null } else { answer.json() })
}

#[tokio::test]
async fn hang_so_can_file_truoc_khi_mo_ban_va_phat_quyen_tai_goi_lai_khong_phat_them() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let status_uri = format!("/products/{}/status", shop.product);
        let (status, problem) =
            call(&shop.app, "POST", &status_uri, "boss", Some(json!({"status": "active"}))).await;
        assert_eq!(status, StatusCode::CONFLICT, "{problem}");
        assert_eq!(problem["code"], "catalog.digital_asset_missing");

        // Hàng vật lý không gắn file được.
        let (_, physical) =
            super::create_product(&shop.app, "boss", shop.chain, "BOOK", "Sách giấy").await;
        let uri = format!("/variants/{physical}/digital-assets");
        let (status, problem) =
            call(&shop.app, "POST", &uri, "boss", Some(json!({"file_id": FILE_ID}))).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("catalog.not_digital"))
        );

        let asset = attach(&shop, 3).await;
        assert_eq!(asset["file_name"], "Sách điện tử.pdf");
        assert_eq!(asset["size"], CONTENT.len());
        let (status, made) =
            call(&shop.app, "POST", &status_uri, "boss", Some(json!({"status": "active"}))).await;
        assert_eq!(status, StatusCode::OK, "{made}");

        // Đơn chưa trả (thu khi giao) không phát gì; đơn đã trả phát một quyền; nhận lại không thêm.
        let (unpaid, mut cod) = paid_order(&shop, Some(BUYER));
        cod.0["after"]["payment_status"] = json!("unpaid");
        emit(&shop, &cod).await;
        let (order, paid) = paid_order(&shop, Some(BUYER));
        emit(&shop, &paid).await;
        emit(&shop, &paid).await;
        settle(&shop.app).await;
        let (status, _) =
            call(&shop.app, "GET", &format!("/orders/{unpaid}/downloads"), "boss", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, grants) =
            call(&shop.app, "GET", &format!("/orders/{order}/downloads"), "boss", None).await;
        assert_eq!(status, StatusCode::OK, "{grants}");
        assert_eq!(grants["data"].as_array().unwrap().len(), 1);
        assert_eq!(grants["data"][0]["remaining"], 3);
        assert!(grants["data"][0].get("token_path").is_none());

        // Người mua đăng nhập thấy quyền của mình và tải được; người không quản lý không xem được.
        let buyer = BUYER.to_string();
        let (status, mine) = call(&shop.app, "GET", "/downloads", &buyer, None).await;
        assert_eq!(status, StatusCode::OK, "{mine}");
        let grant = mine["data"][0]["id"].as_str().unwrap().to_owned();
        let (status, link) =
            call(&shop.app, "POST", &format!("/downloads/{grant}"), &buyer, None).await;
        assert_eq!(status, StatusCode::OK, "{link}");
        assert_eq!(link["data"]["remaining"], 2);
        let answer = shop
            .app
            .get(link["data"]["url"].as_str().unwrap())
            .host("a.example")
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), StatusCode::OK);
        assert_eq!(answer.body(), CONTENT);
        // `Range`: tải tiếp phần còn lại, không trừ lượt.
        let answer = shop
            .app
            .get(link["data"]["url"].as_str().unwrap())
            .host("a.example")
            .header("range", "bytes=11-")
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(answer.body(), &CONTENT[11..]);
        let (status, _) =
            call(&shop.app, "GET", &format!("/orders/{order}/downloads"), &buyer, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // Còn quyền tải dùng được thì không gỡ file được.
        let asset_id = asset["id"].as_str().unwrap();
        let (status, problem) =
            call(&shop.app, "DELETE", &format!("/digital-assets/{asset_id}"), "boss", None).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::CONFLICT, Some("catalog.digital_asset_in_use"))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tai_dong_thoi_khong_vuot_luot() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        attach(&shop, 5).await;
        let (order, paid) = paid_order(&shop, None);
        emit(&shop, &paid).await;
        settle(&shop.app).await;
        let token = token_of(&shop, order).await;
        let requests = (0..20).map(|_| spend(&shop.app, &token, None));
        let answers = futures_util::future::join_all(requests).await;
        let ok = answers.iter().filter(|(status, _)| *status == StatusCode::OK).count();
        let used_up = answers
            .iter()
            .filter(|(status, body)| {
                *status == StatusCode::CONFLICT && body["code"] == "catalog.download_limit_reached"
            })
            .count();
        assert_eq!((ok, used_up), (5, 15), "{answers:?}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn token_het_han_hay_thu_hoi() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        attach(&shop, 5).await;
        let (order, paid) = paid_order(&shop, None);
        emit(&shop, &paid).await;
        settle(&shop.app).await;
        let token = token_of(&shop, order).await;
        let (status, link) = spend(&shop.app, &token, None).await;
        assert_eq!(status, StatusCode::OK, "{link}");
        let url = link["data"]["url"].as_str().unwrap().to_owned();

        // Chữ ký hay hạn bị sửa: như link sai.
        let forged = url.replace("exp=", "exp=1");
        let answer = shop.app.get(&forged).host("a.example").send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::NOT_FOUND);
        assert_eq!(answer.json()["code"], "catalog.download_not_found");
        // Token cũ hết dùng sau khi cấp lại.
        let fresh = token_of(&shop, order).await;
        let (status, _) = spend(&shop.app, &token, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // Đơn huỷ: thu hồi; token mới và địa chỉ ký đã phát cùng hết dùng. Nhận lại không đổi gì.
        let cancelled = Cancelled(json!({"after": {"order_id": order, "status": "cancelled"}}));
        emit(&shop, &cancelled).await;
        emit(&shop, &cancelled).await;
        settle(&shop.app).await;
        let (status, body) = spend(&shop.app, &fresh, None).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("catalog.download_not_found"))
        );
        let answer = shop.app.get(&url).host("a.example").send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::NOT_FOUND);
        let (_, grants) =
            call(&shop.app, "GET", &format!("/orders/{order}/downloads"), "boss", None).await;
        assert!(grants["data"][0]["revoked_at"].is_string(), "{grants}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn url_ky_cua_khach_a_bi_tu_choi_o_khach_b() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        attach(&shop, 5).await;
        let (order, paid) = paid_order(&shop, None);
        emit(&shop, &paid).await;
        settle(&shop.app).await;
        let token = token_of(&shop, order).await;
        // Token của khách mặc định không có ở khách `b`.
        let (status, _) = spend(&shop.app, &token, Some("b.example")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, link) = spend(&shop.app, &token, None).await;
        assert_eq!(status, StatusCode::OK, "{link}");
        let url = link["data"]["url"].as_str().unwrap();
        let answer = shop.app.get(url).host("b.example").send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::NOT_FOUND);
        assert_eq!(answer.json()["code"], "catalog.download_not_found");
        let answer = shop.app.get(url).host("a.example").send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::OK);
    })
    .await
    .unwrap();
}

/// Theo `next_cursor` của `uri` (đã có `?limit=…`) tới hết với `boss`; trả id của mọi dòng theo thứ tự, mỗi trang
/// không quá 2 dòng.
async fn walk_ids(app: &TestApp, uri: &str) -> Vec<Value> {
    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page_uri = match &cursor {
            Some(cursor) => format!("{uri}&cursor={cursor}"),
            None => uri.to_owned(),
        };
        let (status, page) = call(app, "GET", &page_uri, "boss", None).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        let rows = page["data"].as_array().unwrap();
        assert!(rows.len() <= 2, "{page}");
        seen.extend(rows.iter().map(|row| row["id"].clone()));
        match page["pagination"]["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => return seen,
        }
    }
}

/// File của biến thể hàng số (keyset trong database) và quyền tải của một đơn (`page_of`) là `Page<T>` (bước 70
/// P9 mục 3): năm file, một đơn có năm quyền tải, `limit=2` — đi hết bằng `next_cursor` ra đủ năm, không trùng
/// không sót, hai chiều.
#[tokio::test]
async fn file_hang_so_va_quyen_tai_cua_don_theo_trang() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let mut assets = Vec::new();
        for _ in 0..5 {
            assets.push(attach(&shop, 3).await["id"].clone());
        }
        let uri = format!("/variants/{}/digital-assets?limit=2&order=asc", shop.variant);
        assert_eq!(walk_ids(&shop.app, &uri).await, assets);
        assets.reverse();
        let uri = format!("/variants/{}/digital-assets?limit=2", shop.variant);
        assert_eq!(walk_ids(&shop.app, &uri).await, assets);

        let status_uri = format!("/products/{}/status", shop.product);
        let (status, made) =
            call(&shop.app, "POST", &status_uri, "boss", Some(json!({"status": "active"}))).await;
        assert_eq!(status, StatusCode::OK, "{made}");
        let (order, paid) = paid_order(&shop, Some(BUYER));
        emit(&shop, &paid).await;
        settle(&shop.app).await;
        let asc =
            walk_ids(&shop.app, &format!("/orders/{order}/downloads?limit=2&order=asc")).await;
        assert_eq!(asc.len(), 5, "một quyền tải cho mỗi file: {asc:?}");
        let mut unique = asc.clone();
        unique.sort_by_key(|id| id.as_str().unwrap().to_owned());
        unique.dedup();
        assert_eq!(unique.len(), 5, "không trùng");
        let mut desc = walk_ids(&shop.app, &format!("/orders/{order}/downloads?limit=2")).await;
        desc.reverse();
        assert_eq!(desc, asc, "hai chiều ngược nhau");
    })
    .await
    .unwrap();
}
