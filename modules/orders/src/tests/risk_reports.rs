//! Chống bom hàng COD (D-194): quy tắc buộc trả trước khi rủi ro cao, danh sách chặn có lý do và
//! hạn, dữ liệu chỉ trong khách. Báo cáo bán hàng theo phạm vi quyền.

use autoxide_kernel::access::Scope;
use autoxide_kernel::secret::SecretKey;
use autoxide_kernel::sms::PhoneNumber;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, TenantId};
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use jiff::SignedDuration;
use serde_json::{Value, json};
use uuid::Uuid;

use super::checkout::{body, checkout, ready_cart};
use super::lifecycle::{etag, transition};
use super::{Call, Shop, drain, minor, shop_with};

async fn world(test: &TestDatabase) -> Shop {
    shop_with(test, |builder, tree| {
        builder.member("reporter_a", 7, tree.a, &[("orders.reports.read", Scope::Unit)])
    })
    .await
}

/// Một đơn COD của `buyer1` ở nút `org` (giá 100.000, 1 đơn vị); trả (trạng thái, thân trả lời).
async fn cod_at(shop: &Shop, org: OrgId, phone: &str) -> (StatusCode, Value) {
    let item = shop.catalog.add(org, 100_000);
    shop.inventory.stock(org, item, 5);
    let buyer = Call::as_user(shop, "buyer1");
    let (cart, _, quote) = ready_cart(&buyer, org, item, 1).await;
    let mut request = body(&cart, &quote, "cod");
    request["contact"]["phone"] = json!(phone);
    checkout(&buyer, Some(&Uuid::now_v7().to_string()), request).await
}

/// Đơn `id` đi giao rồi bị trả về (huỷ sau khi giao không thành).
async fn refuse(shop: &Shop, id: Uuid) {
    for to in ["preparing", "shipping", "delivery_failed", "cancelled"] {
        let tag = etag(shop, id).await;
        let (status, moved) = transition(shop, id, to, Some(&tag)).await;
        assert_eq!(status, StatusCode::OK, "{to}: {moved}");
    }
    drain(shop).await;
}

#[tokio::test]
async fn quy_tac_buoc_tra_truoc_khi_rui_ro_cao_va_danh_sach_chan() {
    TestDatabase::run(async |test| {
        let shop = world(test).await;
        let phone = "0911222333";
        for _ in 0..2 {
            let (status, placed) = cod_at(&shop, shop.tree.a, phone).await;
            assert_eq!(status, StatusCode::CREATED, "{placed}");
            let id = placed["data"]["orders"][0]["id"].as_str().unwrap().parse().unwrap();
            refuse(&shop, id).await;
        }
        let boss = Call::as_user(&shop, "boss");
        let (status, risk) = boss.send("GET", "/risk/buyers?phone=0911%20222%20333", None).await;
        assert_eq!(status, StatusCode::OK, "{risk}");
        assert_eq!(risk["data"]["refused"], 2);
        assert_eq!(risk["data"]["fail_ratio_bp"], 10_000);

        // Chưa có quy tắc nào: COD vẫn được (mặc định không bật sẵn).
        let (status, placed) = cod_at(&shop, shop.tree.a, phone).await;
        assert_eq!(status, StatusCode::CREATED, "{placed}");

        // Quy tắc của chuỗi... của cửa hàng A: ≥ 2 đơn, không thành ≥ 50% → phải trả trước.
        let (status, rule) = boss
            .send(
                "POST",
                "/risk/cod-rules",
                Some(json!({"org_id": shop.tree.a.as_uuid(), "min_orders": 2, "max_fail_ratio_bp": 5000})),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{rule}");
        let (status, refused) = cod_at(&shop, shop.tree.a, phone).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        assert_eq!(refused["code"], "orders.cod_not_allowed");
        // Người mua khác (số khác) không bị ảnh hưởng.
        let (status, other) = cod_at(&shop, shop.tree.a, "0977000111").await;
        assert_eq!(status, StatusCode::CREATED, "{other}");
        // Kết thúc quy tắc: COD lại được.
        let rule_id = rule["data"]["id"].as_str().unwrap();
        let (status, ended) = boss.send("DELETE", &format!("/risk/cod-rules/{rule_id}"), None).await;
        assert_eq!(status, StatusCode::OK, "{ended}");
        assert!(!ended["data"]["effective_to"].is_null());
        let (status, _) = cod_at(&shop, shop.tree.a, phone).await;
        assert_eq!(status, StatusCode::CREATED);

        // Danh sách chặn: luôn có lý do trong danh sách và hạn ≤ 365 ngày.
        let far = Timestamp::now() + SignedDuration::from_hours(24 * 400);
        let (status, invalid) = boss
            .send(
                "POST",
                "/risk/cod-blocks",
                Some(json!({"phone": "0977000111", "reason_code": "ghét", "expires_at": far})),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{invalid}");
        let fields: Vec<&str> =
            invalid["errors"].as_array().unwrap().iter().map(|e| e["field"].as_str().unwrap()).collect();
        assert!(fields.contains(&"reason_code") && fields.contains(&"expires_at"), "{invalid}");
        let soon = Timestamp::now() + SignedDuration::from_hours(24 * 30);
        let (status, blocked) = boss
            .send(
                "POST",
                "/risk/cod-blocks",
                Some(json!({"phone": "0977000111", "reason_code": "fake_orders", "expires_at": soon})),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{blocked}");
        assert!(!blocked.to_string().contains("0977000111"), "chỉ lưu khoá, không trả số");
        let (status, _) = cod_at(&shop, shop.tree.a, "0977000111").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let block = blocked["data"]["id"].as_str().unwrap();
        let (status, _) = boss.send("DELETE", &format!("/risk/cod-blocks/{block}"), None).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = cod_at(&shop, shop.tree.a, "0977000111").await;
        assert_eq!(status, StatusCode::CREATED);
        // Nhân viên không có `orders.risk.manage`.
        let (status, _) = Call::as_user(&shop, "staff_a").send("GET", "/risk/cod-blocks", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khoa_lien_he_khac_nhau_giua_cac_khach() {
    TestDatabase::run(async |test| {
        let shop = world(test).await;
        let secret = shop.api::<SecretKey>().unwrap();
        let phone = PhoneNumber::parse("0911222333", "84").unwrap();
        let here = secret.phone_key(DEFAULT_TENANT, &phone).await.unwrap();
        let again = secret
            .phone_key(DEFAULT_TENANT, &PhoneNumber::parse("+84 911 222 333", "84").unwrap())
            .await
            .unwrap();
        let elsewhere =
            secret.phone_key(TenantId::from_uuid(Uuid::now_v7()), &phone).await.unwrap();
        assert_eq!(here, again, "cùng số, cùng khách: cùng khoá");
        assert_ne!(here, elsewhere, "hai khách: hai khoá, không so được");
        assert_ne!(here, secret.email_key(DEFAULT_TENANT, "+84911222333").await.unwrap());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bao_cao_ban_hang_theo_pham_vi() {
    TestDatabase::run(async |test| {
        let shop = world(test).await;
        for org in [shop.tree.a, shop.tree.a, shop.tree.b] {
            let (status, placed) = cod_at(&shop, org, "0911222333").await;
            assert_eq!(status, StatusCode::CREATED, "{placed}");
        }
        let from = (Timestamp::now() - SignedDuration::from_hours(24)).to_rfc3339();
        let to = (Timestamp::now() + SignedDuration::from_hours(24)).to_rfc3339();
        let uri = |group: &str| {
            format!(
                "/reports/sales?group_by={group}&from={}&to={}",
                urlencode(&from),
                urlencode(&to)
            )
        };
        let (status, by_org) = Call::as_user(&shop, "boss").send("GET", &uri("org"), None).await;
        assert_eq!(status, StatusCode::OK, "{by_org}");
        let rows = by_org["data"].as_array().unwrap();
        assert_eq!(rows.len(), 2, "{by_org}");
        assert_eq!(rows[0]["key"], shop.tree.a.as_uuid().to_string());
        assert_eq!(rows[0]["orders"], 2);
        // Mỗi đơn 100.000 + phí giao 30.000.
        assert_eq!(minor(&rows[0]["gross"]), 260_000);
        // Nhân viên cửa hàng A chỉ thấy số của A.
        let (status, scoped) =
            Call::as_user(&shop, "reporter_a").send("GET", &uri("org"), None).await;
        assert_eq!(status, StatusCode::OK, "{scoped}");
        assert_eq!(scoped["data"].as_array().unwrap().len(), 1);
        assert_eq!(scoped["data"][0]["key"], shop.tree.a.as_uuid().to_string());
        let (_, by_day) = Call::as_user(&shop, "reporter_a").send("GET", &uri("day"), None).await;
        assert_eq!(by_day["data"].as_array().unwrap().len(), 1);
        assert_eq!(by_day["data"][0]["orders"], 2);
        let (_, by_product) = Call::as_user(&shop, "boss").send("GET", &uri("product"), None).await;
        assert_eq!(by_product["data"].as_array().unwrap().len(), 3);
        assert_eq!(by_product["data"][0]["quantity"], 1);
        let (_, by_channel) = Call::as_user(&shop, "boss").send("GET", &uri("channel"), None).await;
        assert_eq!(by_channel["data"][0]["key"], "web");
        assert_eq!(by_channel["data"][0]["orders"], 3);
        // Không có quyền báo cáo: 403; khoảng quá dài: 422.
        let (status, _) = Call::as_user(&shop, "staff_a").send("GET", &uri("org"), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let long = format!(
            "/reports/sales?group_by=org&from={}&to={}",
            urlencode(&(Timestamp::now() - SignedDuration::from_hours(24 * 400)).to_rfc3339()),
            urlencode(&to)
        );
        let (status, _) = Call::as_user(&shop, "boss").send("GET", &long, None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

fn urlencode(text: &str) -> String {
    text.replace('+', "%2B").replace(':', "%3A")
}

#[tokio::test]
async fn don_xac_nhan_va_hoan_tien_sinh_so_lieu_cho_bao_cao_gop() {
    TestDatabase::run(async |test| {
        let shop = world(test).await;
        let facts = crate::events::facts();
        let org = shop.tree.a.as_uuid();
        let confirmed = autoxide_kernel::event::Delivered::new(
            shop.default_tenant(),
            "orders",
            crate::api::events::ORDER_CONFIRMED,
            json!({"after": {"order_id": Uuid::now_v7(), "org_id": org, "channel": "pos",
                "total": {"amount": "150000", "currency": "VND"}}}),
        );
        let found = facts[0].extract(&confirmed).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].amount.unwrap().minor(), 150_000);
        assert_eq!(found[0].dimensions["channel"], "pos");
        assert_eq!(found[1].quantity, Some(1));
        let refunded = autoxide_kernel::event::Delivered::new(
            shop.default_tenant(),
            "orders",
            crate::api::events::REFUND_COMPLETED,
            json!({"after": {"order_id": Uuid::now_v7(), "org_id": org, "channel": "web",
                "amount": {"amount": "20000", "currency": "VND"}}}),
        );
        let found = facts[1].extract(&refunded).unwrap();
        assert_eq!(found[0].amount.unwrap().minor(), 20_000);
    })
    .await
    .unwrap();
}
