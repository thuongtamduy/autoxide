//! Tính giỏ qua service: trần pháp luật với giá tham chiếu thật của catalog, sàn giá, mã sai, lý
//! do theo ngôn ngữ, dấu vân tay, REST / MCP / `dyn Promotions`.

use autoxide_catalog::api::FloorAction;
use autoxide_testkit::{TestDatabase, tool};
use axum::http::StatusCode;
use chrono::{Duration, Utc};
use serde_json::json;
use uuid::Uuid;

use super::{body, call, money, running, setup};

#[tokio::test]
async fn cat_ve_tran_khi_tinh_va_san_gia() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let category = Uuid::now_v7();
        let cheap = setup.catalog.add(setup.tree.a, 50_000, &[category]);
        let dear = setup.catalog.add(setup.tree.a, 300_000, &[category]);
        // Giảm 40 000 mỗi sản phẩm của danh mục: vượt 50% giá tham chiếu của món 50 000.
        let id = super::create(
            &setup,
            body(setup.tree.a, json!({"kind": "amount_off", "value_bp": null, "value": {"amount": "40000", "currency": "VND"}})),
        )
        .await;
        call(
            &setup.app,
            "PUT",
            &format!("/promotions/{id}/targets"),
            "boss",
            Some(json!({"targets": [{"target_type": "category", "target_id": category}]})),
        )
        .await;
        let (status, activated) = super::activate(&setup, &id).await;
        assert_eq!(status, StatusCode::OK, "{activated}");
        let tenant = setup.tenant();
        let promotions = setup.promotions();
        let result = promotions.evaluate(&tenant, &setup.cart(&[(cheap, 1), (dear, 1)])).await.unwrap();
        let line = |key: &str| result.lines.iter().find(|line| line.line_key == key).unwrap().clone();
        assert_eq!((line("l0").amount.minor(), line("l0").capped_by_law), (25_000, true));
        assert_eq!((line("l1").amount.minor(), line("l1").capped_by_law), (40_000, false));
        // Sàn giá `block` của catalog cắt tiếp.
        setup.catalog.floors.lock().unwrap().insert(dear, (money(280_000), FloorAction::Block));
        let result = promotions.evaluate(&tenant, &setup.cart(&[(dear, 1)])).await.unwrap();
        assert_eq!(result.lines[0].amount.minor(), 20_000);
        assert!(result.lines[0].capped_by_floor);
        // Sản phẩm mới chưa có giá trước khuyến mại: giữ lại khoản giảm, có lý do.
        let fresh = setup.catalog.add(setup.tree.a, 100_000, &[category]);
        setup.catalog.prices.lock().unwrap().insert(fresh, vec![(Utc::now() - Duration::minutes(5), money(100_000))]);
        let result = promotions.evaluate(&tenant, &setup.cart(&[(fresh, 1)])).await.unwrap();
        assert!(result.lines.is_empty());
        assert_eq!(result.withheld[0].reason.code, "promotions.reason.no_reference_price");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ma_sai_khong_chan_ca_gio_va_ly_do_theo_ngon_ngu() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let variant = setup.catalog.add(setup.tree.a, 200_000, &[]);
        let id = running(
            &setup,
            body(setup.tree.a, json!({"trigger": "code", "scope": "order", "kind": "amount_off", "value_bp": null,
                "value": {"amount": "50000", "currency": "VND"}, "min_order": {"amount": "500000", "currency": "VND"}})),
        )
        .await;
        let (status, codes) = call(
            &setup.app,
            "POST",
            &format!("/promotions/{id}/codes"),
            "boss",
            Some(json!({"purpose": "redeem", "code": "GIAM50K"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{codes}");
        let (status, _) =
            call(&setup.app, "POST", &format!("/promotions/{id}/codes"), "boss", Some(json!({"purpose": "redeem", "code": "giam50k"}))).await;
        assert_eq!(status, StatusCode::CONFLICT);
        let mut cart = setup.cart(&[(variant, 2)]);
        cart.codes = vec!["giam 50k".into(), "KHONGCO".into()];
        cart.locale = "vi".into();
        let tenant = setup.tenant();
        let result = setup.promotions().evaluate(&tenant, &cart).await.unwrap();
        assert!(result.applied.is_empty());
        let reason = |code: &str| {
            result.rejected_codes.iter().find(|found| found.code.as_deref() == Some(code)).unwrap().reason.clone()
        };
        assert_eq!(reason("KHONGCO").code, "promotions.reason.code_invalid");
        assert_eq!(reason("GIAM50K").text, "Thêm 100,000 VND để dùng (đơn từ 500,000 VND)");
        cart.lines[0].quantity = 3;
        cart.locale = "en".into();
        let result = setup.promotions().evaluate(&tenant, &cart).await.unwrap();
        assert_eq!(result.total_discount.minor(), 50_000);
        assert_eq!(result.lines[0].reason.text, "50,000 VND off orders from 500,000 VND");
        assert_eq!(result.applied[0].code.as_deref(), Some("GIAM50K"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cung_gio_cung_ket_qua_khi_xao_tron() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let variants: Vec<Uuid> = (0..4).map(|i| setup.catalog.add(setup.tree.a, 50_000 + i * 10_000, &[])).collect();
        running(&setup, body(setup.tree.a, json!({"kind": "buy_x_get_y", "value_bp": null, "buy_quantity": 1, "get_quantity": 1, "reward_value_bp": 4000}))).await;
        running(
            &setup,
            body(setup.tree.a, json!({"scope": "order", "kind": "amount_off", "value_bp": null, "value": {"amount": "33333", "currency": "VND"}})),
        )
        .await;
        let tenant = setup.tenant();
        let mut cart = setup.cart(&variants.iter().map(|variant| (*variant, 2)).collect::<Vec<_>>());
        let first = setup.promotions().evaluate(&tenant, &cart).await.unwrap();
        assert!(first.total_discount.minor() > 33_333);
        for round in 0..20 {
            cart.lines.rotate_left(1 + round % 3);
            let again = setup.promotions().evaluate(&tenant, &cart).await.unwrap();
            assert_eq!(again.fingerprint, first.fingerprint);
            assert_eq!(again.total_discount, first.total_discount);
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn evaluate_qua_rest_va_mcp_cung_ket_qua() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let variant = setup.catalog.add(setup.tree.a, 100_000, &[]);
        running(&setup, body(setup.tree.a, json!({}))).await;
        let cart = serde_json::to_value(setup.cart(&[(variant, 1)])).unwrap();
        let (status, rest) =
            call(&setup.app, "POST", "/evaluate", "boss", Some(cart.clone())).await;
        assert_eq!(status, StatusCode::OK, "{rest}");
        assert_eq!(rest["data"]["total_discount"]["amount"], "10000");
        let client = setup.app.mcp("boss").await.unwrap();
        let answer = client
            .call_tool(tool("promotions.evaluate_cart", json!({"cart": cart})))
            .await
            .unwrap();
        let mcp = answer.structured_content.unwrap();
        assert_eq!(mcp["data"]["fingerprint"], rest["data"]["fingerprint"], "{mcp}");
        client.cancel().await.unwrap();
    })
    .await
    .unwrap();
}
