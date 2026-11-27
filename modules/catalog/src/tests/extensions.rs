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

/// `KycGate` giả: chỉ các nút trong danh sách đã định danh; ghi lại ai đã bị hỏi.
#[derive(Default)]
struct FakeKyc(std::sync::Mutex<Vec<OrgId>>, std::sync::Mutex<Vec<String>>);

#[async_trait::async_trait]
impl autoxide_compliance::api::KycGate for FakeKyc {
    async fn require_verified(
        &self,
        _tenant: autoxide_kernel::tenancy::TenantId,
        subject: autoxide_compliance::api::Subject,
        activity: autoxide_compliance::api::Activity,
    ) -> Result<(), Error> {
        let (_, id) = subject.columns();
        self.1.lock().unwrap().push(format!("{id}:{}", activity.as_str()));
        let verified = self.0.lock().unwrap().iter().any(|org| org.as_uuid().to_string() == id);
        if verified {
            Ok(())
        } else {
            Err(Error::new(
                autoxide_kernel::error::ErrorKind::Forbidden,
                "compliance.kyc_required",
                "Verify your identity first.",
            ))
        }
    }

    async fn status(
        &self,
        _tenant: autoxide_kernel::tenancy::TenantId,
        _subject: autoxide_compliance::api::Subject,
    ) -> Result<autoxide_compliance::api::KycStatus, Error> {
        Err(Error::internal("not used in this test"))
    }

    async fn start(
        &self,
        _tenant: autoxide_kernel::tenancy::TenantId,
        _subject: autoxide_compliance::api::Subject,
        _party: autoxide_kernel::identity::PartyType,
        _return_url: Option<String>,
    ) -> Result<autoxide_compliance::api::KycSession, Error> {
        Err(Error::internal("not used in this test"))
    }
}

/// Đặt `kernel.business_model` của khách mặc định (trước lần đọc cài đặt đầu tiên: có bộ nhớ đệm).
async fn business_model(app: &autoxide_testkit::TestApp, value: &str) {
    use sea_orm::ConnectionTrait as _;
    let write = app.database().write();
    let backend = write.get_database_backend();
    let sql = "INSERT INTO kernel_tenant_settings (tenant_id, name, value, secret, updated_by, updated_at) \
               VALUES ($1, 'kernel.business_model', $2, FALSE, NULL, $3)";
    let values: Vec<sea_orm::Value> = vec![
        autoxide_kernel::tenancy::DEFAULT_TENANT.as_uuid().into(),
        value.into(),
        autoxide_kernel::time::Timestamp::now().into(),
    ];
    write.execute_raw(autoxide_kernel::database::statement(backend, sql, values)).await.unwrap();
}

async fn publish(app: &autoxide_testkit::TestApp, id: &str) -> (StatusCode, serde_json::Value) {
    let body = Some(json!({"status": "active"}));
    call(app, "POST", &format!("/products/{id}/status"), "boss", body).await
}

/// Dựng app có `KycGate` giả và `kernel.business_model = model`; một sản phẩm của người bán (nút `a`).
async fn kyc_app(
    test: &autoxide_testkit::TestDatabase,
    model: &str,
) -> (autoxide_testkit::TestApp, super::Tree, Arc<FakeKyc>, String) {
    let kyc = Arc::new(FakeKyc::default());
    let gate: Arc<dyn autoxide_compliance::api::KycGate> = kyc.clone();
    let (app, tree) = super::app_with(test, |builder| builder.provide(gate)).await;
    business_model(&app, model).await;
    let (seller, _) = create_product(&app, "boss", tree.a, "K-1", "Hàng người bán").await;
    (app, tree, kyc, seller.to_string())
}

/// Sàn (LG-3): sản phẩm của người bán chưa định danh không mở bán được (`compliance.kyc_required`); hàng của
/// chính sàn (nút gốc) thì được, không hỏi định danh.
#[tokio::test]
async fn san_chan_hang_cua_nguoi_ban_chua_dinh_danh() {
    autoxide_testkit::TestDatabase::run(async |test| {
        let (app, tree, kyc, seller) = kyc_app(test, "marketplace").await;
        let (status, problem) = publish(&app, &seller).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
        assert_eq!(problem["code"], "compliance.kyc_required");
        let (_, still) = call(&app, "GET", &format!("/products/{seller}"), "boss", None).await;
        assert_eq!(still["data"]["status"], "draft");
        assert_eq!(*kyc.1.lock().unwrap(), [format!("{}:sell", tree.a.as_uuid())]);
        let root = autoxide_kernel::tenancy::root_of(autoxide_kernel::tenancy::DEFAULT_TENANT);
        let (own, _) = create_product(&app, "boss", root, "K-2", "Hàng của sàn").await;
        let (status, sold) = publish(&app, &own.to_string()).await;
        assert_eq!(status, StatusCode::OK, "{sold}");
        assert_eq!(kyc.1.lock().unwrap().len(), 1, "hàng của nút gốc không hỏi định danh");
    })
    .await
    .unwrap();
}

/// Người bán đã định danh mở bán được.
#[tokio::test]
async fn san_mo_ban_hang_cua_nguoi_ban_da_dinh_danh() {
    autoxide_testkit::TestDatabase::run(async |test| {
        let (app, tree, kyc, seller) = kyc_app(test, "marketplace").await;
        kyc.0.lock().unwrap().push(tree.a);
        let (status, sold) = publish(&app, &seller).await;
        assert_eq!(status, StatusCode::OK, "{sold}");
    })
    .await
    .unwrap();
}

/// Không phải sàn (cửa hàng tự bán hàng của mình): không qua bước định danh người bán.
#[tokio::test]
async fn khong_phai_san_khong_hoi_dinh_danh() {
    autoxide_testkit::TestDatabase::run(async |test| {
        let (app, _, kyc, seller) = kyc_app(test, "independent").await;
        let (status, sold) = publish(&app, &seller).await;
        assert_eq!(status, StatusCode::OK, "{sold}");
        assert!(kyc.1.lock().unwrap().is_empty());
    })
    .await
    .unwrap();
}
