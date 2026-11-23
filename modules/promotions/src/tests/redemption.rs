//! Giữ → xác nhận → trả lượt, chạy đồng thời trên database thật (gate chạy cả SQLite và
//! Postgres): không vượt giới hạn, không giữ hai lần, trả đúng bộ đếm đã giữ.

use super::CatalogExt as _;
use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_kernel::reference::Submitted;
use autoxide_testkit::TestDatabase;
use serde_json::json;
use uuid::Uuid;

use super::{Setup, body, call, running, setup};
use crate::api::{
    Cart, GiftSelection, GrantVoucher, RedemptionStatus, ReserveRequest, VoucherSource,
};

fn hold(cart: Cart) -> ReserveRequest {
    ReserveRequest::new(Uuid::now_v7(), cart, Timestamp::now() + SignedDuration::from_hours(1))
}

/// Chạy `count` lần `reserve` đồng thời; trả số lần thành công và mã lỗi của các lần còn lại.
async fn race(setup: &Arc<Setup>, carts: Vec<Cart>) -> (usize, Vec<String>) {
    let mut tasks = Vec::new();
    for cart in carts {
        let setup = setup.clone();
        tasks.push(tokio::spawn(async move {
            let tenant = setup.tenant();
            setup.promotions().reserve(&tenant, &hold(cart)).await
        }));
    }
    let (mut ok, mut errors) = (0, Vec::new());
    for task in tasks {
        match task.await.unwrap() {
            Ok(redemption) => {
                if !redemption.into_inner().evaluation.applied.is_empty() {
                    ok += 1;
                }
            }
            Err(error) => errors.push(error.code().to_owned()),
        }
    }
    (ok, errors)
}

async fn promotion(setup: &Setup, id: Uuid) -> serde_json::Value {
    call(&setup.app, "GET", &format!("/promotions/{id}"), "boss", None).await.1["data"].clone()
}

#[tokio::test]
async fn ma_gioi_han_10_luot_100_nguoi() {
    TestDatabase::run(async |test| {
        let setup = Arc::new(setup(test, true).await);
        let variant = setup.catalog.add(setup.tree.a, 100_000, &[]);
        let id = running(&setup, body(setup.tree.a, json!({"trigger": "code", "usage_limit": 10})))
            .await;
        call(
            &setup.app,
            "POST",
            &format!("/promotions/{id}/codes"),
            "boss",
            Some(json!({"purpose": "redeem", "code": "TEN10"})),
        )
        .await;
        let carts = (0..100)
            .map(|_| {
                let mut cart = setup.cart(&[(variant, 1)]);
                cart.codes = vec!["TEN10".into()];
                cart
            })
            .collect();
        let (ok, errors) = race(&setup, carts).await;
        assert_eq!(ok, 10, "{errors:?}");
        assert!(errors.iter().all(|code| code == "promotions.usage_exhausted"), "{errors:?}");
        assert_eq!(promotion(&setup, id).await["usage_count"], 10);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn flash_sale_5_suat_100_nguoi() {
    TestDatabase::run(async |test| {
        let setup = Arc::new(setup(test, true).await);
        let variant = setup.catalog.add(setup.tree.a, 200_000, &[]);
        let id = super::create(&setup, body(setup.tree.a, json!({"kind": "flash_sale", "value_bp": null}))).await;
        call(
            &setup.app,
            "PUT",
            &format!("/promotions/{id}/flash-items"),
            "boss",
            Some(json!({"items": [{"variant_id": variant, "sale_price_minor": 120000, "quantity_limit": 5}]})),
        )
        .await;
        super::activate(&setup, &id).await;
        let carts = (0..100).map(|_| setup.cart(&[(variant, 1)])).collect();
        let (ok, errors) = race(&setup, carts).await;
        assert_eq!(ok, 5, "{errors:?}");
        let found = promotion(&setup, id.parse().unwrap()).await;
        assert_eq!(found["flash_items"][0]["quantity_used"], 5);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn moi_nguoi_mot_luot_gui_dong_thoi_va_cung_so_dien_thoai_hai_tai_khoan() {
    TestDatabase::run(async |test| {
        let setup = Arc::new(setup(test, true).await);
        let variant = setup.catalog.add(setup.tree.a, 100_000, &[]);
        running(&setup, body(setup.tree.a, json!({"per_customer_limit": 1}))).await;
        let carts = (0..10)
            .map(|_| {
                let mut cart = setup.cart(&[(variant, 1)]);
                cart.buyer_keys = vec!["user:1".into(), "phone:abc".into()];
                cart
            })
            .collect();
        let (ok, errors) = race(&setup, carts).await;
        assert_eq!(ok, 1, "{errors:?}");
        assert!(errors.iter().all(|code| code == "promotions.customer_limit_reached"));
        // Tài khoản mới, cùng số điện thoại (băm): khoá chung chặn.
        let mut cart = setup.cart(&[(variant, 1)]);
        cart.buyer_keys = vec!["user:2".into(), "phone:abc".into()];
        let error = setup.promotions().reserve(&setup.tenant(), &hold(cart)).await.unwrap_err();
        assert_eq!(error.code(), "promotions.customer_limit_reached");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gioi_han_moi_nguoi_ma_khong_co_khoa_nguoi_mua_thi_tu_choi() {
    TestDatabase::run(async |test| {
        let setup = Arc::new(setup(test, true).await);
        let variant = setup.catalog.add(setup.tree.a, 100_000, &[]);
        let id = running(&setup, body(setup.tree.a, json!({"per_customer_limit": 1}))).await;
        // Không khoá nào: không đếm được theo người → từ chối, không giữ lượt nào.
        let cart = setup.cart(&[(variant, 1)]);
        let error = setup.promotions().reserve(&setup.tenant(), &hold(cart)).await.unwrap_err();
        assert_eq!(error.code(), "promotions.buyer_unidentified");
        assert_eq!(promotion(&setup, id).await["usage_count"], 0);
        // Có khoá thì giữ được như thường.
        let mut cart = setup.cart(&[(variant, 1)]);
        cart.buyer_keys = vec!["phone:abc".into()];
        setup.promotions().reserve(&setup.tenant(), &hold(cart)).await.unwrap();
        assert_eq!(promotion(&setup, id).await["usage_count"], 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn giu_lai_cung_order_id_va_tra_lai_dung_bo_dem() {
    TestDatabase::run(async |test| {
        let setup = Arc::new(setup(test, true).await);
        let variant = setup.catalog.add(setup.tree.a, 100_000, &[]);
        let id = running(&setup, body(setup.tree.a, json!({"usage_limit": 100}))).await;
        let request = hold(setup.cart(&[(variant, 1)]));
        let mut tasks = Vec::new();
        for _ in 0..16 {
            let (setup, request) = (setup.clone(), request.clone());
            tasks.push(tokio::spawn(async move {
                setup.promotions().reserve(&setup.tenant(), &request).await
            }));
        }
        let mut created = 0;
        for task in tasks {
            if matches!(task.await.unwrap().unwrap(), Submitted::Created(_)) {
                created += 1;
            }
        }
        assert_eq!(created, 1);
        assert_eq!(promotion(&setup, id).await["usage_count"], 1);
        // Cùng đơn, giỏ khác: không giữ thêm.
        let mut other = request.clone();
        other.cart.lines[0].quantity = 2;
        let error = setup.promotions().reserve(&setup.tenant(), &other).await.unwrap_err();
        assert_eq!(error.code(), "promotions.reference_reused");
        // Hai lần trả đồng thời chỉ cộng lại một lần.
        let order = request.order_id;
        let (promotions, tenant) = (setup.promotions(), setup.tenant());
        let (a, b) =
            tokio::join!(promotions.release(&tenant, order), promotions.release(&tenant, order));
        assert!(a.unwrap().is_some() && b.unwrap().is_some());
        assert_eq!(promotion(&setup, id).await["usage_count"], 0);
        assert!(
            setup.promotions().release(&setup.tenant(), Uuid::now_v7()).await.unwrap().is_none()
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn xac_nhan_va_het_han_dua_nhau() {
    TestDatabase::run(async |test| {
        let setup = Arc::new(setup(test, true).await);
        let variant = setup.catalog.add(setup.tree.a, 100_000, &[]);
        let id = running(&setup, body(setup.tree.a, json!({"usage_limit": 5}))).await;
        let request = hold(setup.cart(&[(variant, 1)]));
        setup.promotions().reserve(&setup.tenant(), &request).await.unwrap();
        let tenant = setup.tenant();
        let promotions = setup.promotions();
        // Xác nhận đua với job hết hạn (giả như đã quá hạn): đúng một bên thắng.
        let (confirmed, expired) =
            tokio::join!(promotions.confirm(&tenant, request.order_id), async {
                crate::tests::expire_now(&setup, request.order_id).await
            });
        let usage = promotion(&setup, id).await["usage_count"].as_i64().unwrap();
        match confirmed {
            Ok(found) => {
                assert_eq!(found.status, RedemptionStatus::Confirmed);
                assert_eq!(usage, 1);
                assert!(!expired);
            }
            Err(error) => {
                assert_eq!(error.code(), "promotions.redemption_expired");
                assert_eq!(usage, 0);
            }
        }
        // Gọi lại `confirm` trả như cũ hay báo hết hạn — không đổi bộ đếm.
        let _ = promotions.confirm(&tenant, request.order_id).await;
        assert_eq!(promotion(&setup, id).await["usage_count"].as_i64().unwrap(), usage);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ngan_sach_khong_vuot_khi_100_don_va_luot_theo_ngay() {
    TestDatabase::run(async |test| {
        let setup = Arc::new(setup(test, true).await);
        let variant = setup.catalog.add(setup.tree.a, 100_000, &[]);
        let budget = running(
            &setup,
            body(
                setup.tree.a,
                json!({"budget": {"amount": "75000", "currency": "VND"}, "daily_usage_limit": 6}),
            ),
        )
        .await;
        let carts = (0..100).map(|_| setup.cart(&[(variant, 1)])).collect();
        let (ok, errors) = race(&setup, carts).await;
        // Ngân sách 75 000 / 10 000 mỗi đơn → 7, nhưng lượt theo ngày 6 chặn trước.
        assert_eq!(ok, 6, "{errors:?}");
        let found = promotion(&setup, budget).await;
        assert_eq!(found["budget_used"]["amount"], "60000");
        let (_, list) = call(&setup.app, "GET", "/redemptions?status=held", "boss", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 6);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn mot_voucher_hai_don_dong_thoi_va_huy_don_tra_voucher() {
    TestDatabase::run(async |test| {
        let setup = Arc::new(setup(test, true).await);
        let variant = setup.catalog.add(setup.tree.a, 100_000, &[]);
        let id = running(&setup, body(setup.tree.a, json!({"trigger": "voucher"}))).await;
        let buyer = Uuid::from_u128(100);
        let tenant = setup.tenant();
        let voucher = setup
            .promotions()
            .grant_voucher(
                &tenant,
                &GrantVoucher::new(id, buyer, VoucherSource::Birthday, "birthday:2026"),
            )
            .await
            .unwrap()
            .into_inner();
        let carts = (0..2)
            .map(|_| {
                let mut cart = setup.cart(&[(variant, 1)]);
                cart.buyer_id = Some(buyer);
                cart.voucher_ids = vec![voucher.id];
                cart
            })
            .collect();
        let (ok, _) = race(&setup, carts).await;
        assert_eq!(ok, 1);
        let held = setup.promotions().vouchers_of(&tenant, buyer, None).await.unwrap();
        assert_eq!(held[0].status, crate::api::VoucherStatus::Held);
        let order = held[0].used_order_id.unwrap();
        setup.promotions().confirm(&tenant, order).await.unwrap();
        let used = setup.promotions().vouchers_of(&tenant, buyer, None).await.unwrap();
        assert_eq!(used[0].status, crate::api::VoucherStatus::Used);
        // Huỷ trọn đơn: voucher còn hạn về ví.
        setup.promotions().release(&tenant, order).await.unwrap();
        let back = setup.promotions().vouchers_of(&tenant, buyer, None).await.unwrap();
        assert_eq!(back[0].status, crate::api::VoucherStatus::Available);
        assert_eq!(back[0].used_order_id, None);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn qua_het_suat() {
    TestDatabase::run(async |test| {
        let setup = Arc::new(setup(test, true).await);
        let variant = setup.catalog.add(setup.tree.a, 600_000, &[]);
        let gift = Uuid::now_v7();
        let id = running(
            &setup,
            body(
                setup.tree.a,
                json!({"kind": "gift_choice", "value_bp": null,
                "gift_options": [{"variant_id": gift, "stock_limit": 3}], "gift_quantity": 1}),
            ),
        )
        .await;
        let carts = (0..20)
            .map(|_| {
                let mut cart = setup.cart(&[(variant, 1)]);
                cart.gift_selections =
                    vec![GiftSelection { promotion_id: id, variant_id: gift, quantity: 1 }];
                cart
            })
            .collect();
        let (ok, errors) = race(&setup, carts).await;
        assert_eq!(ok, 3, "{errors:?}");
        assert!(errors.iter().all(|code| code == "promotions.gift_out_of_stock"));
    })
    .await
    .unwrap();
}

/// Nhân viên trả bằng tay lượt giữ của một đơn kẹt (`POST /redemptions/{id}/release`), trên cả hai
/// database — trên Postgres lần kiểm quyền ghi từng chạy trên kết nối đọc và trả 500.
#[tokio::test]
async fn nhan_vien_tra_luot_giu_bang_tay() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let shirt = setup.catalog.add(setup.tree.a, 200_000, &[]);
        let id = running(&setup, body(setup.tree.a, json!({}))).await;
        let tenant = setup.tenant();
        let held =
            setup.promotions().reserve(&tenant, &hold(setup.cart(&[(shirt, 1)]))).await.unwrap();
        let held = held.into_inner();
        assert_eq!(held.status, RedemptionStatus::Held);
        let (status, released) =
            call(&setup.app, "POST", &format!("/redemptions/{}/release", held.id), "boss", None)
                .await;
        assert_eq!(status, axum::http::StatusCode::OK, "{released}");
        assert_eq!(released["data"]["status"], "released");
        let (status, _) =
            call(&setup.app, "POST", &format!("/redemptions/{}/release", held.id), "store_b", None)
                .await;
        assert_eq!(status, axum::http::StatusCode::NOT_FOUND, "another store does not see it");
        assert!(promotion(&setup, id).await["id"].is_string());
    })
    .await
    .unwrap();
}
