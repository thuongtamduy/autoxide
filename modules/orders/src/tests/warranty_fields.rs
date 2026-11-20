//! Ô người mua nhập lúc đặt hàng (D-235), trả góp qua đối tác, bảo hành theo serial / theo dòng,
//! hạn huỷ / đổi trả và cách trả trên giỏ cho website.

use autoxide_kernel::access::Scope;
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use chrono::{Duration, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};
use uuid::Uuid;

use super::checkout::{body, checkout, ready_cart};
use super::lifecycle::{etag, transition};
use super::{Call, Shop, drain, run_job, shop_with};
use crate::entity::warranty;

async fn world(test: &TestDatabase) -> Shop {
    shop_with(test, |builder, tree| {
        builder.member("tech_a", 8, tree.a, &[("orders.warranty.manage", Scope::Unit)])
    })
    .await
}

/// Định nghĩa trường tự định nghĩa của `orders.order` (bằng `boss`).
async fn define(shop: &Shop, field: Value) {
    let answer =
        shop.post("/api/v1/kernel/custom-fields").token("boss").json(field).send().await.unwrap();
    assert_eq!(answer.status(), StatusCode::CREATED, "{}", answer.json());
}

fn error_fields(problem: &Value) -> Vec<String> {
    problem["errors"]
        .as_array()
        .map(|errors| errors.iter().map(|e| e["field"].as_str().unwrap().to_owned()).collect())
        .unwrap_or_default()
}

#[tokio::test]
async fn o_nguoi_mua_nhap_luc_dat_hang_va_han_cua_nguoi_mua() {
    TestDatabase::run(async |test| {
        let shop = world(test).await;
        define(&shop, json!({"target": "orders.order", "key": "delivery_time", "data_type": "text",
            "labels": {"en": "Preferred delivery time"}, "required": true, "visibility": "buyer_input"}))
        .await;
        define(&shop, json!({"target": "orders.order", "key": "staff_note", "data_type": "text",
            "labels": {"en": "Staff note"}, "required": true, "visibility": "internal"}))
        .await;
        let item = shop.catalog.add(shop.tree.a, 100_000);
        shop.inventory.stock(shop.tree.a, item, 5);
        let buyer = Call::as_user(&shop, "buyer1");
        let (cart, _, quote) = ready_cart(&buyer, shop.tree.a, item, 1).await;

        // Thiếu ô bắt buộc: báo cùng lỗi khác của checkout, một lần.
        let mut request = body(&cart, &quote, "cod");
        request["contact"]["phone"] = json!("");
        let (status, problem) = checkout(&buyer, Some("cf-1"), request).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        let fields = error_fields(&problem);
        assert!(fields.contains(&"custom_fields.delivery_time".to_owned()), "{problem}");
        assert!(fields.iter().any(|field| field.starts_with("contact")), "{problem}");

        // Người mua không ghi được trường nội bộ.
        let mut request = body(&cart, &quote, "cod");
        request["custom_fields"] = json!({"delivery_time": "18:00", "staff_note": "x"});
        let (status, problem) = checkout(&buyer, Some("cf-2"), request).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert!(error_fields(&problem).contains(&"custom_fields.staff_note".to_owned()));

        let mut request = body(&cart, &quote, "cod");
        request["custom_fields"] = json!({"delivery_time": "18:00-20:00"});
        let (status, placed) = checkout(&buyer, Some("cf-3"), request).await;
        assert_eq!(status, StatusCode::CREATED, "{placed}");
        let id = placed["data"]["orders"][0]["id"].as_str().unwrap();
        let (status, mine) = buyer.send("GET", &format!("/me/orders/{id}"), None).await;
        assert_eq!(status, StatusCode::OK, "{mine}");
        assert_eq!(mine["data"]["custom_fields"]["delivery_time"], "18:00-20:00");
        // Website hiện sẵn hạn huỷ; chưa giao thì chưa có hạn đổi trả.
        assert_eq!(mine["data"]["buyer_cancel_until"], "confirmed");
        assert_eq!(mine["data"]["buyer_can_cancel"], true);
        assert!(mine["data"].get("return_until").is_none(), "{mine}");

        let id: Uuid = id.parse().unwrap();
        deliver(&shop, id).await;
        let (_, mine) = buyer.send("GET", &format!("/me/orders/{id}"), None).await;
        assert_eq!(mine["data"]["buyer_can_cancel"], false);
        let until: chrono::DateTime<Utc> =
            serde_json::from_value(mine["data"]["return_until"].clone()).unwrap();
        let days = (until - Utc::now()).num_hours();
        assert!((6 * 24..=7 * 24).contains(&days), "hạn đổi trả mặc định 7 ngày: {until}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tra_gop_qua_doi_tac_va_cach_tra_cua_gio() {
    TestDatabase::run(async |test| {
        let shop = world(test).await;
        let item = shop.catalog.add(shop.tree.a, 3_000_000);
        shop.inventory.stock(shop.tree.a, item, 5);
        let buyer = Call::as_user(&shop, "buyer1");
        let (cart, _, quote) = ready_cart(&buyer, shop.tree.a, item, 1).await;
        let (_, found) = buyer.send("GET", &format!("/carts/{cart}"), None).await;
        let methods = found["data"]["payment_methods"].clone();
        assert_eq!(methods, json!(["cod", "bank_transfer", "gateway", "installment"]));

        // Trả góp: thiếu kế hoạch, số kỳ lạ, kế hoạch kèm cách trả khác — gom lỗi.
        let (status, problem) =
            checkout(&buyer, Some("i-1"), body(&cart, &quote, "installment")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert!(error_fields(&problem).contains(&"installment".to_owned()));
        let mut request = body(&cart, &quote, "installment");
        request["installment"] = json!({"partner": "", "months": 5});
        let (status, problem) = checkout(&buyer, Some("i-2"), request).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        let fields = error_fields(&problem);
        assert!(fields.contains(&"installment.partner".to_owned()), "{problem}");
        assert!(fields.contains(&"installment.months".to_owned()), "{problem}");
        let mut request = body(&cart, &quote, "cod");
        request["installment"] = json!({"partner": "fe_credit", "months": 6});
        let (status, _) = checkout(&buyer, Some("i-3"), request).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

        // Đúng: lượt thu đi qua đối tác người mua chọn — core không giữ tiền (D-187).
        let mut request = body(&cart, &quote, "installment");
        request["installment"] = json!({"partner": "fe_credit", "months": 6});
        let (status, placed) = checkout(&buyer, Some("i-4"), request).await;
        assert_eq!(status, StatusCode::CREATED, "{placed}");
        assert_eq!(placed["data"]["orders"][0]["status"], "pending_payment");
        assert_eq!(*shop.ledger.providers.lock().unwrap(), vec![Some("fe_credit".to_owned())]);

        // Cửa hàng tắt COD: giỏ không còn đưa COD.
        shop.orgs.set(shop.tree.a, "orders.cod_enabled", json!(false));
        let (cart, _, _) = ready_cart(&buyer, shop.tree.a, item, 1).await;
        let (_, found) = buyer.send("GET", &format!("/carts/{cart}"), None).await;
        assert_eq!(found["data"]["payment_methods"][0], "bank_transfer");
    })
    .await
    .unwrap();
}

/// Đơn `id` đi giao tới nơi.
async fn deliver(shop: &Shop, id: Uuid) {
    for to in ["preparing", "shipping", "delivered"] {
        let tag = etag(shop, id).await;
        let (status, moved) = transition(shop, id, to, Some(&tag)).await;
        assert_eq!(status, StatusCode::OK, "{to}: {moved}");
    }
    drain(shop).await;
}

/// Đơn COD của `buyer1`: `quantity` cái `item`; trả (ID đơn, ID dòng).
async fn order_of(shop: &Shop, item: Uuid, quantity: i64, key: &str) -> (Uuid, Uuid) {
    let buyer = Call::as_user(shop, "buyer1");
    let (cart, _, quote) = ready_cart(&buyer, shop.tree.a, item, quantity).await;
    let (status, placed) = checkout(&buyer, Some(key), body(&cart, &quote, "cod")).await;
    assert_eq!(status, StatusCode::CREATED, "{placed}");
    let id = placed["data"]["orders"][0]["id"].as_str().unwrap().parse().unwrap();
    let (_, order) = Call::as_user(shop, "boss").send("GET", &format!("/{id}"), None).await;
    let line = order["data"]["lines"][0]["id"].as_str().unwrap().parse().unwrap();
    (id, line)
}

#[tokio::test]
async fn bao_hanh_theo_serial_va_theo_dong_phieu_bao_hanh() {
    TestDatabase::run(async |test| {
        let shop = world(test).await;
        let phone = shop.catalog.add_with(shop.tree.a, 5_000_000, |variant| {
            variant.serial_tracked = true;
            variant.warranty_months = Some(12);
        });
        let case = shop.catalog.add_with(shop.tree.a, 100_000, |variant| {
            variant.warranty_months = Some(3);
        });
        let plain = shop.catalog.add(shop.tree.a, 50_000);
        for item in [phone, case, plain] {
            shop.inventory.stock(shop.tree.a, item, 10);
        }
        let boss = Call::as_user(&shop, "boss");

        // Serial: không quá số lượng, không trùng, không rỗng — gom một lần.
        let (order, line) = order_of(&shop, phone, 2, "w-1").await;
        let uri = format!("/{order}/lines/{line}/serials");
        let (status, problem) =
            boss.send("POST", &uri, Some(json!({"serials": ["A1", "A1", " ", "A3"]}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        let fields = error_fields(&problem);
        assert!(fields.contains(&"serials[0]".to_owned()) && fields.contains(&"serials".to_owned()));
        let (status, saved) =
            boss.send("POST", &uri, Some(json!({"serials": ["SN-001", "SN-002"]}))).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        // Gửi lại: không thêm.
        let (_, again) = boss.send("POST", &uri, Some(json!({"serials": []}))).await;
        assert_eq!(again["data"]["serials"], json!(["SN-001", "SN-002"]));
        let (status, _) = Call::as_user(&shop, "staff_a")
            .send("POST", &uri, Some(json!({"serials": ["SN-003"]})))
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // Giao xong: mỗi serial một bảo hành 12 tháng; event giao lại không mở thêm.
        deliver(&shop, order).await;
        let (other, _) = order_of(&shop, case, 1, "w-2").await;
        deliver(&shop, other).await;
        let (none, _) = order_of(&shop, plain, 1, "w-3").await;
        deliver(&shop, none).await;
        let tech = Call::as_user(&shop, "tech_a");
        let (status, found) = tech.send("GET", "/warranties?serial=SN-002", None).await;
        assert_eq!(status, StatusCode::OK, "{found}");
        let rows = found["data"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{found}");
        assert_eq!(rows[0]["status"], "active");
        let ends: chrono::DateTime<Utc> = serde_json::from_value(rows[0]["ends_at"].clone()).unwrap();
        assert!(ends > Utc::now() + Duration::days(360) && ends < Utc::now() + Duration::days(370));
        let (_, by_phone) = tech.send("GET", "/warranties?phone=0901234567", None).await;
        assert_eq!(by_phone["data"].as_array().unwrap().len(), 3, "2 serial + 1 dòng: {by_phone}");
        let (_, mine) = Call::as_user(&shop, "buyer1").send("GET", "/me/warranties", None).await;
        assert_eq!(mine["data"].as_array().unwrap().len(), 3);
        let (status, _) = tech.send("GET", "/warranties", None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (status, _) = Call::as_user(&shop, "staff_a").send("GET", "/warranties?serial=SN-002", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // Phiếu bảo hành: số phiếu, hạn xử lý, chuyển theo bảng.
        let warranty_id = rows[0]["id"].as_str().unwrap();
        let (status, claim) = tech
            .send(
                "POST",
                "/warranty-claims",
                Some(json!({"warranty_id": warranty_id, "org_id": shop.tree.a.as_uuid(),
                    "issue": "Màn hình không lên"})),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{claim}");
        assert!(claim["data"]["number"].as_str().unwrap().starts_with("BH"));
        assert_eq!(claim["data"]["status"], "received");
        let claim_id = claim["data"]["id"].as_str().unwrap();
        let move_to = async |to: &str| {
            tech.send(
                "POST",
                &format!("/warranty-claims/{claim_id}/transitions"),
                Some(json!({"to": to})),
            )
            .await
        };
        let (status, problem) = move_to("ready").await;
        assert_eq!(status, StatusCode::CONFLICT, "{problem}");
        assert_eq!(problem["code"], "orders.warranty_claim_transition");
        for to in ["inspecting", "inspecting", "repairing", "ready", "returned"] {
            let (status, moved) = move_to(to).await;
            assert_eq!(status, StatusCode::OK, "{to}: {moved}");
            assert_eq!(moved["data"]["status"], to);
        }
        // Tiếp nhận ở cửa hàng ngoài phạm vi: 403.
        let (status, _) = tech
            .send(
                "POST",
                "/warranty-claims",
                Some(json!({"warranty_id": warranty_id, "org_id": shop.tree.b.as_uuid(), "issue": "x"})),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // Hết hạn: job đánh dấu, tiếp nhận bị từ chối.
        warranty::Entity::update_many()
            .col_expr(warranty::Column::EndsAt, Expr::value(Utc::now() - Duration::days(1)))
            .filter(warranty::Column::Id.eq(warranty_id.parse::<Uuid>().unwrap()))
            .exec(shop.database().write())
            .await
            .unwrap();
        run_job(&shop, crate::jobs::EXPIRE_WARRANTIES).await;
        let (_, found) = tech.send("GET", "/warranties?serial=SN-002", None).await;
        assert_eq!(found["data"][0]["status"], "expired");
        let (status, problem) = tech
            .send(
                "POST",
                "/warranty-claims",
                Some(json!({"warranty_id": warranty_id, "org_id": shop.tree.a.as_uuid(), "issue": "x"})),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{problem}");
        assert_eq!(problem["code"], "orders.warranty_expired");
    })
    .await
    .unwrap();
}
