//! Ví voucher: cấp không nhân đôi, quota khi nhiều người nhận cùng lúc, chỉ chủ ví dùng được,
//! lưu / nhập mã nhận qua REST, dữ liệu cá nhân, xoá tài khoản.

use std::sync::Arc;

use autoxide_kernel::reference::Submitted;
use autoxide_testkit::{TestDatabase, tool};
use axum::http::StatusCode;
use serde_json::json;
use uuid::Uuid;

use super::{body, call, running, setup};
use crate::api::{GrantVoucher, VoucherSource, VoucherStatus};

#[tokio::test]
async fn voucher_quota_10_nguoi_100_va_cap_lai_khong_nhan_doi() {
    TestDatabase::run(async |test| {
        let setup = Arc::new(setup(test, true).await);
        let id =
            running(&setup, body(setup.tree.a, json!({"trigger": "voucher", "voucher_quota": 10})))
                .await;
        let mut tasks = Vec::new();
        for user in 0..100u128 {
            let setup = setup.clone();
            tasks.push(tokio::spawn(async move {
                let request = GrantVoucher::new(
                    id,
                    Uuid::from_u128(1000 + user),
                    VoucherSource::Referral,
                    "friend",
                );
                setup.promotions().grant_voucher(&setup.tenant(), &request).await
            }));
        }
        let mut issued = 0;
        for task in tasks {
            match task.await.unwrap() {
                Ok(_) => issued += 1,
                Err(error) => assert_eq!(error.code(), "promotions.voucher_quota_exhausted"),
            }
        }
        assert_eq!(issued, 10);
        let (_, found) = call(&setup.app, "GET", &format!("/promotions/{id}"), "boss", None).await;
        assert_eq!(found["data"]["issued_count"], 10);
        // Gọi lại cùng `source` + `source_ref`: voucher cũ, không cấp thêm.
        let tenant = setup.tenant();
        let wallet =
            setup.promotions().vouchers_of(&tenant, Uuid::from_u128(1000), None).await.unwrap();
        if let Some(first) = wallet.first() {
            let again = setup
                .promotions()
                .grant_voucher(
                    &tenant,
                    &GrantVoucher::new(
                        id,
                        Uuid::from_u128(1000),
                        VoucherSource::Referral,
                        "friend",
                    ),
                )
                .await
                .unwrap();
            assert!(matches!(&again, Submitted::Replayed(found) if found.id == first.id));
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chi_chu_vi_dung_duoc_va_luu_nhap_ma_qua_rest() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let variant = setup.catalog.add(setup.tree.a, 100_000, &[]);
        let id = running(
            &setup,
            body(setup.tree.a, json!({"trigger": "voucher", "claimable": true, "public": true, "per_user_claim_limit": 1})),
        )
        .await;
        let (status, saved) = call(&setup.app, "POST", &format!("/public/promotions/{id}/claim"), "buyer0", None).await;
        assert_eq!(status, StatusCode::CREATED, "{saved}");
        let (status, _) = call(&setup.app, "POST", &format!("/public/promotions/{id}/claim"), "buyer0", None).await;
        assert_eq!(status, StatusCode::OK);
        let voucher: Uuid = saved["data"]["id"].as_str().unwrap().parse().unwrap();
        // Mã nhận voucher.
        call(&setup.app, "POST", &format!("/promotions/{id}/codes"), "boss", Some(json!({"purpose": "claim", "code": "NHAN-QUA"}))).await;
        let (status, claimed) =
            call(&setup.app, "POST", "/me/vouchers/claim-code", "buyer1", Some(json!({"code": "nhan-qua"}))).await;
        assert_eq!(status, StatusCode::CREATED, "{claimed}");
        let (status, wrong) =
            call(&setup.app, "POST", "/me/vouchers/claim-code", "buyer1", Some(json!({"code": "KHONG"}))).await;
        assert_eq!((status, wrong["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("promotions.code_invalid")));
        let (_, wallet) = call(&setup.app, "GET", "/me/vouchers", "buyer0", None).await;
        assert_eq!(wallet["data"].as_array().unwrap().len(), 1);
        // Voucher của buyer0 trong giỏ của buyer1: bị từ chối, giỏ vẫn tính được.
        let tenant = setup.tenant();
        let mut cart = setup.cart(&[(variant, 1)]);
        cart.buyer_id = Some(Uuid::from_u128(101));
        cart.voucher_ids = vec![voucher];
        let result = setup.promotions().evaluate(&tenant, &cart).await.unwrap();
        assert!(result.applied.is_empty());
        assert_eq!(result.rejected_codes[0].reason.code, "promotions.reason.voucher_not_owned");
        cart.buyer_id = Some(Uuid::from_u128(100));
        let result = setup.promotions().evaluate(&tenant, &cart).await.unwrap();
        assert_eq!(result.total_discount.minor(), 10_000);
        assert_eq!(result.applied[0].voucher_id, Some(voucher));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn xoa_tai_khoan_thu_hoi_voucher_va_co_trong_xuat_du_lieu() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let id = running(&setup, body(setup.tree.a, json!({"trigger": "voucher"}))).await;
        let user = Uuid::from_u128(100);
        let tenant = setup.tenant();
        setup
            .promotions()
            .grant_voucher(
                &tenant,
                &GrantVoucher::new(id, user, VoucherSource::Compensation, "ticket:1"),
            )
            .await
            .unwrap();
        let exported = setup.app.modules().personal_data().export(&tenant, user).await.unwrap();
        assert_eq!(exported["promotions"]["vouchers"].as_array().unwrap().len(), 1);
        crate::vouchers::erase_user(&tenant, user).await.unwrap();
        assert!(setup.promotions().vouchers_of(&tenant, user, None).await.unwrap().is_empty());
        let (status, granted) = call(
            &setup.app,
            "POST",
            &format!("/promotions/{id}/vouchers"),
            "boss",
            Some(json!({"user_id": user, "reason": "Bù lỗi giao hàng", "reference": "case-9"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{granted}");
        let wallet = setup
            .promotions()
            .vouchers_of(&tenant, user, Some(VoucherStatus::Available))
            .await
            .unwrap();
        assert_eq!(wallet.len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tool_vi_voucher_cua_nguoi_dung_chi_tra_voucher_trong_pham_vi() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let user = Uuid::from_u128(4242);
        for org in [setup.tree.a, setup.tree.b] {
            let id = running(&setup, body(org, json!({"trigger": "voucher"}))).await;
            let request = GrantVoucher::new(id, user, VoucherSource::Referral, format!("r-{id}"));
            setup.promotions().grant_voucher(&setup.tenant(), &request).await.unwrap();
        }
        let count = async |who: &str| {
            let client = setup.app.mcp(who).await.unwrap();
            let answer = client
                .call_tool(tool("promotions.list_vouchers_of_user", json!({"user_id": user})))
                .await
                .unwrap();
            let found = answer.structured_content.unwrap();
            client.cancel().await.unwrap();
            found["data"]["vouchers"].as_array().map_or(0, Vec::len)
        };
        // Cả khách thấy hai; cửa hàng A chỉ thấy voucher của chương trình của mình.
        assert_eq!(count("boss").await, 2);
        assert_eq!(count("store_a").await, 1);
        assert_eq!(count("store_b").await, 1);
    })
    .await
    .unwrap();
}
