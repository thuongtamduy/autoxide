//! Báo khi có hàng lại (D-201): xác nhận hai bước, đồng ý riêng, một lần mỗi đợt có hàng, dừng bằng
//! link, người đã đăng nhập báo vào tài khoản.

use super::CatalogExt as _;
use autoxide_kernel::time::Timestamp;
use autoxide_notifications::api::testing::FakeNotifier;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, root_of};
use autoxide_notifications::api::{Notification, Notifier, Recipient};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Shop, call, drain, shop_with, stock_in};
use crate::api::{Inventory, ReserveLine, ReserveRequest};

const BUYER: u128 = 77;

async fn post(app: &TestApp, uri: &str, token: Option<&str>, body: Value) -> (StatusCode, Value) {
    let mut request =
        app.request(Method::POST, &format!("/api/v1/inventory/public{uri}")).json(body);
    if let Some(token) = token {
        request = request.token(token);
    }
    let answer = request.send().await.unwrap();
    let status = answer.status();
    (status, if answer.body().is_empty() { Value::Null } else { answer.json() })
}

async fn form_token(app: &TestApp) -> String {
    let issued =
        app.get("/api/v1/antispam/token?scope=inventory:restock_alerts").send().await.unwrap();
    assert_eq!(issued.status(), StatusCode::OK, "{}", issued.text());
    issued.json()["data"]["token"].as_str().unwrap().to_owned()
}

/// Token trong link của tin.
fn token_of(sent: &Notification) -> String {
    sent.link.as_deref().unwrap().split("token=").nth(1).unwrap().to_owned()
}

async fn sell_all(shop: &Shop, variant: Uuid, quantity: i64) {
    let api: Arc<dyn Inventory> = shop.api::<dyn Inventory>().unwrap();
    let request = ReserveRequest::new(
        Uuid::now_v7(),
        shop.tree.store,
        Timestamp::now() + SignedDuration::from_mins(30),
        vec![ReserveLine::new("1", variant, quantity)],
    );
    api.reserve(&shop.default_tenant(), &request).await.unwrap();
}

#[tokio::test]
async fn bao_co_hang_lai_mot_lan_moi_dot() {
    TestDatabase::run(async |test| {
        let notifier = Arc::new(FakeNotifier::default());
        let shop = shop_with(test, |builder, _| {
            builder
                .setting("AUTOXIDE_ANTISPAM_MIN_FILL_SECONDS", "0")
                .setting(
                    "AUTOXIDE_INVENTORY_RESTOCK_CONFIRM_URL",
                    "https://shop.example/restock?token={token}",
                )
                .provide::<dyn Notifier>(notifier.clone())
                .principal("buyer", Principal::customer(Uuid::from_u128(BUYER)))
        })
        .await;
        let a = shop.tree.a.as_uuid();
        let store = shop.tree.store.as_uuid();
        let root = root_of(DEFAULT_TENANT).as_uuid();
        // Cửa hàng S và kho online (nút gốc) đều bán từ kho A.
        for selling in [store, root] {
            let sources =
                json!({"sources": [{"warehouse_org_id": a, "priority": 1, "active": true}]});
            let (status, _) =
                call(&shop, "PUT", &format!("/sources/{selling}"), "boss", Some(sources)).await;
            assert_eq!(status, StatusCode::OK);
        }
        let item = shop.catalog.item();

        // Thiếu đồng ý, thiếu liên lạc: một lỗi gom cả hai.
        let (status, problem) = post(
            &shop,
            "/restock-alerts",
            None,
            json!({"variant_id": item, "consent": false, "form_token": form_token(&shop).await}),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["errors"].as_array().unwrap().len(), 2, "{problem}");

        // Khách vãng lai: 202, tin xác nhận có link; chưa xác nhận thì không được báo.
        let guest = json!({"variant_id": item, "store_id": store, "email": "Ada@Example.com",
                           "consent": true, "consent_text": "Báo tôi khi có hàng"});
        let mut body = guest.clone();
        body["form_token"] = json!(form_token(&shop).await);
        let (status, answer) = post(&shop, "/restock-alerts", None, body).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{answer}");
        assert_eq!(answer["data"]["accepted"], true);
        let confirms = notifier.sent_of("inventory.restock_confirm");
        assert_eq!(confirms.len(), 1);
        assert_eq!(confirms[0].recipient, Recipient::Email("ada@example.com".into()));
        // Biến thể không có: cũng 202, không lưu, không gửi.
        let mut ghost = guest.clone();
        ghost["variant_id"] = json!(Uuid::now_v7());
        ghost["form_token"] = json!(form_token(&shop).await);
        let (status, _) = post(&shop, "/restock-alerts", None, ghost).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(notifier.sent_of("inventory.restock_confirm").len(), 1);

        let token = token_of(&confirms[0]);
        for _ in 0..2 {
            let (status, answer) =
                post(&shop, "/restock-alerts/confirm", None, json!({"token": token})).await;
            assert_eq!(status, StatusCode::OK, "{answer}");
            assert_eq!(answer["data"]["status"], "confirmed");
        }
        let (status, _) =
            post(&shop, "/restock-alerts/confirm", None, json!({"token": "nope"})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Người đã đăng nhập: báo vào tài khoản, không cần xác nhận; `restock_alerts_of` thấy.
        let (status, answer) = post(
            &shop,
            "/restock-alerts",
            Some("buyer"),
            json!({"variant_id": item, "consent": true}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{answer}");
        let api: Arc<dyn Inventory> = shop.api::<dyn Inventory>().unwrap();
        let tenant = shop.default_tenant();
        let mine = api.restock_alerts_of(&tenant, Uuid::from_u128(BUYER), &[item]).await.unwrap();
        assert_eq!(mine, vec![item]);

        // Hàng về: mỗi người một tin; hàng về thêm (không qua 0) không báo lại.
        stock_in(&shop, shop.tree.a, item, 3, None).await;
        drain(&shop).await;
        let back = notifier.sent_of("inventory.back_in_stock");
        assert_eq!(back.len(), 2, "{back:?}");
        assert!(back.iter().any(|sent| sent.recipient == Recipient::User(Uuid::from_u128(BUYER))));
        stock_in(&shop, shop.tree.a, item, 2, None).await;
        drain(&shop).await;
        assert_eq!(notifier.sent_of("inventory.back_in_stock").len(), 2);

        // Bán hết rồi về lại: đợt mới, mỗi người thêm một tin.
        sell_all(&shop, item, 5).await;
        drain(&shop).await;
        stock_in(&shop, shop.tree.a, item, 4, None).await;
        drain(&shop).await;
        let back = notifier.sent_of("inventory.back_in_stock");
        assert_eq!(back.len(), 4);

        // Khách vãng lai dừng bằng link trong tin: đợt sau chỉ còn người đăng nhập.
        let guest_last = back
            .iter()
            .rev()
            .find(|sent| sent.recipient == Recipient::Email("ada@example.com".into()))
            .unwrap();
        let (status, answer) =
            post(&shop, "/restock-alerts/stop", None, json!({"token": token_of(guest_last)})).await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        sell_all(&shop, item, 4).await;
        drain(&shop).await;
        stock_in(&shop, shop.tree.a, item, 1, None).await;
        drain(&shop).await;
        let back = notifier.sent_of("inventory.back_in_stock");
        assert_eq!(back.len(), 5);
        assert_eq!(back[4].recipient, Recipient::User(Uuid::from_u128(BUYER)));

        // Dữ liệu cá nhân (rà soát bảo mật 2026-11): bản xuất của người mua có đăng ký của họ; tài
        // khoản bị xoá thì đăng ký bị xoá (nhận lại event cũng không sao).
        let buyer = Uuid::from_u128(BUYER);
        let exported = shop.modules().personal_data().export(&tenant, buyer).await.unwrap();
        let alerts = exported["inventory"]["restock_alerts"].as_array().unwrap();
        assert_eq!(alerts.len(), 1, "{exported:?}");
        assert_eq!(alerts[0]["variant_id"], json!(item));
        let deleted = autoxide_kernel::event::Delivered::new(
            tenant.clone(),
            "auth",
            autoxide_kernel::privacy::USER_DELETED.to_owned(),
            json!({"before": {"id": buyer}}),
        );
        use autoxide_kernel::event::EventHandler as _;
        crate::alerts::AlertPrivacy.handle(&deleted).await.unwrap();
        crate::alerts::AlertPrivacy.handle(&deleted).await.unwrap();
        let exported = shop.modules().personal_data().export(&tenant, buyer).await.unwrap();
        assert_eq!(exported["inventory"]["restock_alerts"], json!([]));
        assert!(api.restock_alerts_of(&tenant, buyer, &[item]).await.unwrap().is_empty());
    })
    .await
    .unwrap();
}
