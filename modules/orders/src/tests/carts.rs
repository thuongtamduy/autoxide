//! Giỏ hàng: giá tính lại ở máy chủ, vãng lai và gộp giỏ, tồn, mã giảm giá, phí giao, giỏ bỏ dở.

use super::CatalogExt as _;
use super::money;
use autoxide_kernel::tenancy::{OrgId, ScopeExt};
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use jiff::SignedDuration;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::json;

use super::{Call, drain, minor, shop};
use crate::entity::cart;

/// Tạo giỏ ở cửa hàng `org`; trả (id, cart_token nếu vãng lai).
async fn new_cart(call: &Call<'_>, org: OrgId) -> (String, Option<String>) {
    let (status, created) =
        call.send("POST", "/carts", Some(json!({"org_id": org.as_uuid()}))).await;
    assert!(status == StatusCode::CREATED || status == StatusCode::OK, "{created}");
    let id = created["data"]["id"].as_str().unwrap().to_owned();
    let token = created["data"]["cart_token"].as_str().map(str::to_owned);
    (id, token)
}

#[tokio::test]
async fn cart_prices_are_recomputed_ignoring_client_numbers() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let shirt = shop.catalog.add(shop.tree.a, 150_000);
        shop.inventory.stock(shop.tree.a, shirt, 10);
        let guest = Call::as_guest(&shop, None);
        let (id, token) = new_cart(&guest, shop.tree.a).await;
        let token = token.unwrap();
        let guest = Call::as_guest(&shop, Some(&token));
        // Client gửi giá và tổng sai: bị bỏ qua, máy chủ tính lại.
        let (status, cart) = guest
            .send(
                "PUT",
                &format!("/carts/{id}/lines/{shirt}"),
                Some(json!({"quantity": 2, "unit_price": {"amount": "1", "currency": "VND"}, "total": "2"})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{cart}");
        let line = &cart["data"]["lines"][0];
        assert_eq!(minor(&line["unit_price"]), 150_000);
        assert_eq!(minor(&line["total"]), 300_000);
        assert_eq!(minor(&cart["data"]["totals"]["total"]), 300_000);
        // Giá đổi ở `catalog`: đọc lại giỏ thấy giá mới, `quote_token` đổi.
        let before = cart["data"]["quote_token"].as_str().unwrap().to_owned();
        shop.catalog.set_price(shirt, money(140_000));
        let (_, cart) = guest.send("GET", &format!("/carts/{id}"), None).await;
        assert_eq!(minor(&cart["data"]["totals"]["total"]), 280_000);
        assert_ne!(cart["data"]["quote_token"].as_str().unwrap(), before);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn guest_cart_merges_into_buyer_cart_on_login() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let (shirt, hat) =
            (shop.catalog.add(shop.tree.a, 100_000), shop.catalog.add(shop.tree.a, 50_000));
        shop.inventory.stock(shop.tree.a, shirt, 50);
        shop.inventory.stock(shop.tree.a, hat, 50);
        let buyer = Call::as_user(&shop, "buyer1");
        let (mine, _) = new_cart(&buyer, shop.tree.a).await;
        buyer
            .send("PUT", &format!("/carts/{mine}/lines/{shirt}"), Some(json!({"quantity": 1})))
            .await;
        // Tạo lại khi đã có giỏ: trả giỏ cũ (200).
        let (status, again) =
            buyer.send("POST", "/carts", Some(json!({"org_id": shop.tree.a.as_uuid()}))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(again["data"]["id"].as_str().unwrap(), mine);

        let (guest_id, token) = new_cart(&Call::as_guest(&shop, None), shop.tree.a).await;
        let token = token.unwrap();
        let guest = Call::as_guest(&shop, Some(&token));
        guest
            .send("PUT", &format!("/carts/{guest_id}/lines/{shirt}"), Some(json!({"quantity": 2})))
            .await;
        guest
            .send("PUT", &format!("/carts/{guest_id}/lines/{hat}"), Some(json!({"quantity": 1})))
            .await;
        guest
            .send("PUT", &format!("/carts/{guest_id}/codes"), Some(json!({"codes": ["sale"]})))
            .await;

        // Người mua khác không đọc được giỏ vãng lai bằng ID.
        let (status, _) =
            Call::as_user(&shop, "buyer2").send("GET", &format!("/carts/{guest_id}"), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let merge = Call { app: &shop, who: Some("buyer1"), cart_token: Some(&token) };
        let (status, merged) = merge.send("POST", &format!("/carts/{guest_id}/merge"), None).await;
        assert_eq!(status, StatusCode::OK, "{merged}");
        assert_eq!(merged["data"]["id"].as_str().unwrap(), mine);
        let quantities: Vec<i64> = merged["data"]["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| line["quantity"].as_i64().unwrap())
            .collect();
        assert_eq!(quantities, vec![3, 1]);
        assert_eq!(merged["data"]["codes"], json!(["SALE"]));
        // Gộp lại lần hai không cộng thêm.
        let (status, again) = merge.send("POST", &format!("/carts/{guest_id}/merge"), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(again["data"]["lines"][0]["quantity"], 3);
        // Giỏ vãng lai đã gộp không sửa được nữa.
        let (status, closed) = guest
            .send("PUT", &format!("/carts/{guest_id}/lines/{hat}"), Some(json!({"quantity": 5})))
            .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(closed["code"], "orders.cart_closed");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cart_token_from_other_tenant_is_rejected() {
    TestDatabase::run(async |test| {
        let catalog = std::sync::Arc::new(super::FakeCatalog::default());
        let app = TestApp::builder(test)
            .multi_tenant()
            .tenant("a")
            .tenant("b")
            .provide::<dyn autoxide_catalog::api::Catalog>(catalog.clone())
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let created = app
            .post("/api/v1/orders/carts")
            .header("Autoxide-Tenant", "a")
            .json(json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.text());
        let body = created.json();
        let (id, token) =
            (body["data"]["id"].as_str().unwrap(), body["data"]["cart_token"].as_str().unwrap());
        let read = |tenant: &'static str| {
            app.get(&format!("/api/v1/orders/carts/{id}"))
                .header("Autoxide-Tenant", tenant)
                .header("Cart-Token", token)
        };
        assert_eq!(read("a").send().await.unwrap().status(), StatusCode::OK);
        assert_eq!(read("b").send().await.unwrap().status(), StatusCode::NOT_FOUND);
        // Sai token ở đúng khách: 401.
        let wrong = app
            .get(&format!("/api/v1/orders/carts/{id}"))
            .header("Autoxide-Tenant", "a")
            .header("Cart-Token", "not-the-token")
            .send()
            .await
            .unwrap();
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(wrong.json()["code"], "orders.cart_token_invalid");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn het_hang_va_so_luong_sai_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let shirt = shop.catalog.add(shop.tree.a, 100_000);
        let stopped =
            shop.catalog.add_with(shop.tree.a, 100_000, |variant| variant.variant_active = false);
        let backorder = shop.catalog.add_with(shop.tree.a, 100_000, |variant| {
            variant.sale_mode = autoxide_catalog::api::SaleMode::Backorder;
        });
        shop.inventory.stock(shop.tree.a, shirt, 2);
        let buyer = Call::as_user(&shop, "buyer1");
        let (id, _) = new_cart(&buyer, shop.tree.a).await;
        let line = |variant: uuid::Uuid| format!("/carts/{id}/lines/{variant}");
        let (status, error) = buyer.send("PUT", &line(shirt), Some(json!({"quantity": 3}))).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(error["code"], "orders.out_of_stock");
        assert!(error["detail"].as_str().unwrap().contains("(2 left)"), "{error}");
        let (status, error) =
            buyer.send("PUT", &line(shirt), Some(json!({"quantity": 1000}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error["code"], "orders.quantity_invalid");
        let (status, error) = buyer.send("PUT", &line(stopped), Some(json!({"quantity": 1}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error["code"], "orders.variant_unavailable");
        // Hàng cho đặt khi hết (backorder) không cần tồn.
        let (status, cart) =
            buyer.send("PUT", &line(backorder), Some(json!({"quantity": 5}))).await;
        assert_eq!(status, StatusCode::OK, "{cart}");
        let (status, cart) = buyer.send("PUT", &line(shirt), Some(json!({"quantity": 2}))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(cart["data"]["lines"].as_array().unwrap().len(), 2);
        // 0 xoá dòng.
        let (_, cart) = buyer.send("PUT", &line(backorder), Some(json!({"quantity": 0}))).await;
        assert_eq!(cart["data"]["lines"].as_array().unwrap().len(), 1);
        // Hết hàng sau khi đã bỏ vào giỏ: dòng báo vấn đề, không tính tiền.
        shop.catalog.edit_variant(shirt, |variant| variant.variant_active = false);
        let (_, cart) = buyer.send("GET", &format!("/carts/{id}"), None).await;
        assert_eq!(cart["data"]["lines"][0]["problem"]["code"], "orders.variant_unavailable");
        assert_eq!(minor(&cart["data"]["totals"]["total"]), 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ma_giam_gia_phi_giao_va_mien_phi_giao_con_thieu() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let shirt = shop.catalog.add(shop.tree.a, 200_000);
        shop.inventory.stock(shop.tree.a, shirt, 10);
        shop.promotions.add_code("GIAM10", 1000);
        let buyer = Call::as_user(&shop, "buyer1");
        let (id, _) = new_cart(&buyer, shop.tree.a).await;
        buyer.send("PUT", &format!("/carts/{id}/lines/{shirt}"), Some(json!({"quantity": 1}))).await;
        let delivery = json!({"address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "1 Trang Tien"},
            "recipient_name": "An", "phone": "0901234567", "shipping_option": "standard"});
        let (status, cart) = buyer.send("PUT", &format!("/carts/{id}/delivery"), Some(delivery)).await;
        assert_eq!(status, StatusCode::OK, "{cart}");
        let data = &cart["data"];
        assert_eq!(data["shipping"]["selected"], "standard");
        assert_eq!(minor(&data["totals"]["shipping_fee"]), 30_000);
        assert_eq!(minor(&data["free_shipping_remaining"]), 300_000);
        assert_eq!(minor(&data["totals"]["total"]), 230_000);
        // Phí giao do `shipping` tính, đổi ở đó thì giỏ đổi theo.
        *shop.shipping.fee.lock().unwrap() = 35_000;
        let (_, cart) = buyer.send("GET", &format!("/carts/{id}"), None).await;
        assert_eq!(minor(&cart["data"]["totals"]["total"]), 235_000);
        *shop.shipping.fee.lock().unwrap() = 30_000;

        let (_, cart) = buyer.send("PUT", &format!("/carts/{id}/codes"), Some(json!({"codes": ["giam10", "KHONGCO"]}))).await;
        let data = &cart["data"];
        assert_eq!(minor(&data["totals"]["discount"]), 20_000, "{data}");
        assert_eq!(minor(&data["lines"][0]["total"]), 180_000);
        assert_eq!(minor(&data["totals"]["total"]), 210_000);
        assert_eq!(data["promotions"][0]["code"], "GIAM10");
        assert_eq!(data["rejected_codes"][0]["code"], "KHONGCO");
        assert_eq!(data["rejected_codes"][0]["reason"]["code"], "promotions.reason.code_invalid");
        // Mã và voucher dùng được cho giỏ.
        let (status, offers) = buyer.send("GET", &format!("/carts/{id}/promotions"), None).await;
        assert_eq!(status, StatusCode::OK, "{offers}");
        // Voucher: chỉ người đăng nhập.
        let (guest_id, token) = new_cart(&Call::as_guest(&shop, None), shop.tree.a).await;
        let token = token.unwrap();
        let guest = Call::as_guest(&shop, Some(&token));
        let (status, _) = guest
            .send("PUT", &format!("/carts/{guest_id}/vouchers"), Some(json!({"voucher_ids": [uuid::Uuid::now_v7()]})))
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gio_bo_do_phat_event_mot_lan_va_nhan_vien_xem_theo_pham_vi() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let shirt = shop.catalog.add(shop.tree.a, 100_000);
        let hat = shop.catalog.add(shop.tree.b, 100_000);
        shop.inventory.stock(shop.tree.a, shirt, 10);
        shop.inventory.stock(shop.tree.b, hat, 10);
        let buyer = Call::as_user(&shop, "buyer1");
        let (in_a, _) = new_cart(&buyer, shop.tree.a).await;
        buyer
            .send("PUT", &format!("/carts/{in_a}/lines/{shirt}"), Some(json!({"quantity": 1})))
            .await;
        let (in_b, _) = new_cart(&buyer, shop.tree.b).await;
        buyer
            .send("PUT", &format!("/carts/{in_b}/lines/{hat}"), Some(json!({"quantity": 1})))
            .await;
        // Vãng lai không có người mua để nhắc: không tính.
        let (guest_id, token) = new_cart(&Call::as_guest(&shop, None), shop.tree.a).await;
        Call::as_guest(&shop, token.as_deref())
            .send("PUT", &format!("/carts/{guest_id}/lines/{shirt}"), Some(json!({"quantity": 1})))
            .await;
        let tenant = shop.default_tenant();
        cart::Entity::update_many()
            .col_expr(
                cart::Column::UpdatedAt,
                Expr::value(Timestamp::now() - SignedDuration::from_hours(3)),
            )
            .filter(cart::Column::Status.eq("active"))
            .scoped(&tenant)
            .exec(shop.database().write())
            .await
            .unwrap();
        let marked = run_mark_abandoned(&shop).await;
        assert_eq!(marked, 2);
        assert_eq!(run_mark_abandoned(&shop).await, 0, "only once");
        drain(&shop).await;
        let staff = Call::as_user(&shop, "staff_a");
        let (status, page) = staff.send("GET", "/carts?status=abandoned", None).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        let ids: Vec<&str> = page["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|cart| cart["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec![in_a.as_str()]);
        let (status, _) = Call::as_user(&shop, "buyer1").send("GET", "/carts", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        // Sửa giỏ: giỏ hoạt động lại.
        let (_, cart) = buyer
            .send("PUT", &format!("/carts/{in_a}/lines/{shirt}"), Some(json!({"quantity": 2})))
            .await;
        assert_eq!(cart["data"]["status"], "active");
    })
    .await
    .unwrap();
}

/// Chạy job `orders.mark_abandoned_carts` một lượt; trả số event `orders.cart_abandoned` mới.
async fn run_mark_abandoned(shop: &super::Shop) -> u64 {
    let before = count_abandoned_events(shop).await;
    super::run_job(shop, crate::jobs::MARK_ABANDONED_CARTS).await;
    count_abandoned_events(shop).await - before
}

async fn count_abandoned_events(shop: &super::Shop) -> u64 {
    use sea_orm::{ConnectionTrait, Statement};
    let read = shop.database().read();
    let sql = "SELECT COUNT(*) FROM orders_events WHERE name = 'orders.cart_abandoned'";
    let row = read
        .query_one_raw(Statement::from_string(read.get_database_backend(), sql))
        .await
        .unwrap()
        .unwrap();
    u64::try_from(row.try_get_by_index::<i64>(0).unwrap()).unwrap()
}
