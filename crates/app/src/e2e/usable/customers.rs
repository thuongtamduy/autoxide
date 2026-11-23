//! Khách hàng dùng được như nhân viên cửa hàng thật (cả SQLite và Postgres): đăng ký khách tại quầy,
//! xem hồ sơ và điểm, nhóm khách, quy tắc tích điểm, hạng, đổi điểm lấy voucher, giới thiệu, nhắc giỏ.

use autoxide_kernel::tenancy::{DEFAULT_TENANT, root_of};
use autoxide_testkit::TestDatabase;
use serde_json::json;

use super::kit::{ADMIN, app, call_tool, expect, get, id, import_reference_data, promotion_window};

#[tokio::test]
async fn khach_tai_quay_diem_nhom_va_doi_diem_lay_voucher() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        import_reference_data(&app).await;
        let root = json!(root_of(DEFAULT_TENANT));
        let customer = json!({"phone": "0912345678", "full_name": "Trần Bình"});
        let customer = id(&expect(&app, "POST", "/api/v1/customers", ADMIN, Some(customer), 201).await);
        call_tool(&app, ADMIN, "customers.get_customer", json!({"id": customer})).await;
        get(&app, &format!("/api/v1/customers/{customer}/points")).await;
        let group = json!({"name": "Khách quen", "org_id": root});
        expect(&app, "POST", "/api/v1/customers/groups", ADMIN, Some(group), 201).await;
        call_tool(&app, ADMIN, "customers.list_groups", json!({})).await;

        get(&app, "/api/v1/customers/loyalty/rules").await;
        get(&app, "/api/v1/customers/loyalty/tiers").await;
        get(&app, "/api/v1/customers/referral-rules").await;
        get(&app, "/api/v1/customers/reminder-rules").await;

        // Đổi 100 điểm lấy voucher của một chương trình đang chạy; tạm tắt phần thưởng.
        let (starts, ends) = promotion_window();
        let promotion = json!({"promotion": {"org_id": root, "kind": "amount_off", "currency": "VND",
            "value": {"amount": "20000", "currency": "VND"}, "trigger": "voucher", "locale": "vi", "title": "Giảm 20.000đ",
            "terms": "Đổi bằng điểm", "starts_at": starts, "ends_at": ends}});
        let promotion = call_tool(&app, ADMIN, "promotions.create_promotion", promotion).await;
        let promotion = promotion["id"].as_str().unwrap_or_else(|| panic!("{promotion}")).to_owned();
        super::orders::activate(&app, &promotion).await;
        let reward = json!({"kind": "points", "points_cost": 100, "promotion_id": promotion});
        let reward = id(&expect(&app, "POST", "/api/v1/customers/loyalty/voucher-rewards", ADMIN, Some(reward), 201).await);
        get(&app, "/api/v1/customers/loyalty/voucher-rewards").await;
        let path = format!("/api/v1/customers/loyalty/voucher-rewards/{reward}");
        expect(&app, "PUT", &path, ADMIN, Some(json!({"active": false})), 200).await;
    })
    .await
    .unwrap();
}
