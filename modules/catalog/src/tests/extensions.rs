//! Điểm cắm cho module đăng ký sau `catalog` (`CatalogExtensions`): điều kiện mở bán của
//! `marketplace` (bước 46).

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use axum::http::StatusCode;
use serde_json::json;

use super::{app, call, create_product};
use crate::api::{CatalogExtensions, ListingCandidate, ListingDecision, ListingGate};

/// Cấm mở bán sản phẩm của một nút (như người bán chưa xác thực).
struct BlockOrg(OrgId);

#[async_trait::async_trait]
impl ListingGate for BlockOrg {
    async fn can_publish(
        &self,
        _tenant: &TenantContext,
        candidate: &ListingCandidate,
    ) -> Result<ListingDecision, Error> {
        Ok(if candidate.org_id == self.0 {
            ListingDecision::Deny {
                code: "marketplace.seller_not_verified".to_owned(),
                message: "verify the seller first".to_owned(),
            }
        } else {
            ListingDecision::Allow
        })
    }
}

#[tokio::test]
async fn dieu_kien_mo_ban_chan_san_pham_cua_nguoi_ban() {
    autoxide_testkit::TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let extensions = app.api::<dyn CatalogExtensions>().unwrap();
        assert!(extensions.set_listing_gate(Arc::new(BlockOrg(tree.a))));
        assert!(!extensions.set_listing_gate(Arc::new(BlockOrg(tree.b))), "one gate only");

        let (blocked, _) = create_product(&app, "boss", tree.a, "L-1", "Hàng người bán").await;
        let (allowed, _) = create_product(&app, "boss", tree.chain, "L-2", "Hàng chuỗi").await;
        let active = json!({"status": "active"});
        let (status, problem) = call(
            &app,
            "POST",
            &format!("/products/{blocked}/status"),
            "boss",
            Some(active.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["code"], "catalog.listing_denied");
        assert!(
            problem["detail"]
                .as_str()
                .unwrap_or_default()
                .contains("marketplace.seller_not_verified"),
            "{problem}"
        );
        let (_, still) = call(&app, "GET", &format!("/products/{blocked}"), "boss", None).await;
        assert_eq!(still["data"]["status"], "draft");
        let (status, sold) = call(
            &app,
            "POST",
            &format!("/products/{allowed}/status"),
            "boss",
            Some(active.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{sold}");
        // Ngoài phạm vi: báo không có sản phẩm, không lộ lý do của điều kiện mở bán.
        let (status, _) =
            call(&app, "POST", &format!("/products/{blocked}/status"), "store_b", Some(active))
                .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}
