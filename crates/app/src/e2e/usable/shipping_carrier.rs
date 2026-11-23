//! Giao qua hãng vận chuyển dùng được như người bán thật (cả SQLite và Postgres), với máy chủ giả của
//! Giao Hàng Nhanh: tài khoản hãng của khách, vận đơn đặt bằng tool, huỷ, bảng đối soát thu hộ lệch số
//! tiền, hỏi lại hãng lịch sử vận đơn.

use autoxide_kernel::tenancy::{DEFAULT_TENANT, root_of};
use autoxide_testkit::{FakeProviderServer, TestDatabase};
use serde_json::{Value, json};
use uuid::Uuid;

use super::kit::{ADMIN, app_with, call_tool, expect, id, import_reference_data};

const CREATE: &str = "/shiip/public-api/v2/shipping-order/create";
const DETAIL: &str = "/shiip/public-api/v2/shipping-order/detail";

fn ok(data: Value) -> Value {
    json!({"code": 200, "message": "Success", "data": data})
}

fn vnd(amount: i64) -> Value {
    json!({"amount": amount.to_string(), "currency": "VND"})
}

#[tokio::test]
async fn van_don_ghn_dat_huy_va_doi_soat_thu_ho() {
    TestDatabase::run(async |test| {
        let ghn = FakeProviderServer::start().await.unwrap();
        let app = app_with(test, |builder| builder.setting("AUTOXIDE_GHN_API_URL", ghn.url())).await;
        import_reference_data(&app).await;
        let root = json!(root_of(DEFAULT_TENANT));
        let account = json!({"org_id": root, "carrier_code": "ghn", "name": "GHN",
            "credentials": {"token": "ghn-token"}, "settings": {"shop_id": 885}});
        let account = id(&expect(&app, "POST", "/api/v1/shipping/carrier-accounts", ADMIN, Some(account), 201).await);
        let method = json!({"org_id": root, "code": "ghn", "kind": "carrier", "currency": "VND",
            "names": {"vi": "Giao Hàng Nhanh", "en": "GHN"}, "carrier_account_id": account, "auto_book": false});
        let method = id(&expect(&app, "POST", "/api/v1/shipping/methods", ADMIN, Some(method), 201).await);

        let shipment = |number: &str| {
            json!({"order_id": Uuid::now_v7(), "order_number": number, "request_key": Uuid::now_v7(),
                "org_id": root, "method_id": method, "recipient_name": "Trần Minh Anh", "recipient_phone": "0987654321",
                "address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "12 Tràng Tiền"},
                "items": [{"order_line_key": "l1", "variant_id": Uuid::now_v7(), "sku": "A", "name": "Áo",
                           "quantity": 2, "weight_grams": 400}],
                "declared_value": vnd(700_000), "cod_amount": vnd(285_000)})
        };
        ghn.reply("POST", CREATE, 200, ok(json!({"order_code": "LADFYR", "total_fee": 20900,
            "expected_delivery_time": "2026-12-15T16:59:59Z"})));
        let first = id(&expect(&app, "POST", "/api/v1/shipping/shipments", ADMIN, Some(shipment("DH1")), 201).await);
        let booked = call_tool(&app, ADMIN, "shipping.book_shipment", json!({"id": first})).await;
        assert_eq!(booked["tracking_number"], "LADFYR", "{booked}");
        call_tool(&app, ADMIN, "shipping.get_shipment", json!({"id": first})).await;

        let second = id(&expect(&app, "POST", "/api/v1/shipping/shipments", ADMIN, Some(shipment("DH2")), 201).await);
        call_tool(&app, ADMIN, "shipping.cancel_shipment", json!({"id": second})).await;

        // Bảng thu hộ của hãng ghi thiếu tiền: một khoản lệch chờ người xử lý.
        let csv = "tracking_number,cod_amount,fee\nLADFYR,200000,20900\n";
        let statement = json!({"carrier_account_id": account, "period_from": "2026-11-01", "period_to": "2026-11-30",
                               "reference": "BK-1", "csv": csv});
        expect(&app, "POST", "/api/v1/shipping/cod/statements", ADMIN, Some(statement), 201).await;
        let found = call_tool(&app, ADMIN, "shipping.list_cod_discrepancies", json!({})).await;
        let discrepancy = found[0]["id"].as_str().unwrap_or_else(|| panic!("{found}")).to_owned();
        ghn.reply("GET", DETAIL, 200, ok(json!({"order_code": "LADFYR", "status": "delivered",
            "updated_date": "2026-11-15T04:49:47.811Z", "cod_amount": 285000, "is_cod_collected": true})));
        let path = format!("/api/v1/shipping/cod/discrepancies/{discrepancy}/refresh-tracking");
        expect(&app, "POST", &path, ADMIN, None, 200).await;
    })
    .await
    .unwrap();
}
