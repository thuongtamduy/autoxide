//! Sàn dừng hàng đang bán của người bán không còn định danh (LG-3b, D-20261128-seller-kyc-pause).
//!
//! Cổng mở bán (`products.rs`, `check_seller_identity`) chỉ chặn lúc chuyển sang `active`; hàng đã bán trước đó
//! phải được kiểm lại khi:
//! - hồ sơ định danh của một nút hết hạn hay bị thu hồi (`compliance.kyc_expired` / `kyc_revoked`) — kiểm lại
//!   người bán trong nhánh của nút đó;
//! - nút đổi chỗ trong cây (`organizations.unit_moved`: cửa hàng con chuyển sang dưới người bán chưa định danh)
//!   — kiểm lại mọi người bán của khách;
//! - mỗi ngày (job `catalog.seller_kyc_sweep`): khách vừa chuyển thành sàn (`kernel.business_model`), quy tắc
//!   `kyc.required_activities` bắt đầu áp dụng, khách gộp vào sàn.
//!
//! Người bán không qua được cổng (`compliance.kyc_required`, sàn thiếu `compliance`, nút không còn trong cây) thì
//! mọi sản phẩm `active` của nút đó chuyển `inactive` qua đường đổi trạng thái của `products.rs`, phát
//! `catalog.product_status_changed` với tác nhân hệ thống. Gọi lại vô hại (D-054): chỉ dòng còn `active` bị đổi. Định danh lại xong
//! thì người bán tự mở bán lại — không tự bật lại hàng (người bán có thể đã muốn tắt).

use std::collections::{BTreeMap, BTreeSet};

use autoxide_kernel::db::{Queries as _, Record, TenantKey};
use autoxide_kernel::error::Error;
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::tenancy::{OrgId, Site, TenantContext, TenantId, root_of};
use sea_orm::sea_query::{Expr, ExprTrait};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use uuid::Uuid;

use crate::entity::product;
use crate::service::CatalogService;

/// Job hằng ngày kiểm lại người bán của mọi sàn (mọi database).
pub(crate) const SELLER_KYC_SWEEP: &str = "catalog.seller_kyc_sweep";

/// Sản phẩm và nút bán của nó — chỉ các cột việc kiểm lại cần, đọc qua lớp `db` của kernel (lọc theo khách).
#[derive(Record, Clone, Debug)]
#[record(table = "catalog_products", scope = tenant)]
pub(crate) struct OnSale {
    #[record(primary_key)]
    id: Uuid,
    tenant_id: TenantKey,
    org_id: Uuid,
    status: String,
}

/// Mã lỗi nghĩa là "người bán này không được bán" (khác lỗi tạm thời như database). `site_not_found`: nút không
/// còn trong cây — không xác định được người bán nên không coi là đã định danh.
fn refused(error: &Error) -> bool {
    matches!(
        error.code(),
        "compliance.kyc_required" | "catalog.seller_verification_unavailable" | "site_not_found"
    )
}

impl CatalogService {
    /// Kiểm lại người bán có hàng `active` (trong nhánh `within`, hay cả khách khi `None`) và dừng hàng của người
    /// bán không còn qua được cổng định danh. Khách không phải sàn: không làm gì. Trả số sản phẩm đã dừng.
    pub(crate) async fn pause_unverified_sellers(
        &self,
        tenant: &TenantContext,
        within: Option<OrgId>,
    ) -> Result<u64, Error> {
        if !self.is_marketplace(tenant).await? {
            return Ok(0);
        }
        let root = root_of(tenant.id()).as_uuid();
        let rows = tenant
            .db()
            .read()
            .find::<OnSale>()
            .filter(OnSale::STATUS.eq("active".to_owned()))
            .filter(OnSale::ORG_ID.ne(root))
            .all()
            .await?;
        let mut by_seller: BTreeMap<Uuid, Vec<Uuid>> = BTreeMap::new();
        for row in rows {
            by_seller.entry(row.org_id).or_default().push(row.id);
        }
        let mut paused = 0;
        for (seller, products) in by_seller {
            let org = OrgId::from_uuid(seller);
            if let Some(node) = within {
                // Chỉ người bán trong nhánh của nút vừa đổi định danh.
                let inside = match Site::of(tenant, org).await {
                    Ok(site) => site.lineage().contains(&node),
                    Err(error) if refused(&error) => true,
                    Err(error) => return Err(error),
                };
                if !inside {
                    continue;
                }
            }
            match self.check_seller_identity(tenant, org).await {
                Ok(()) => {}
                Err(error) if refused(&error) => {
                    let mut count = 0;
                    for id in products {
                        count += u64::from(self.pause_as_system(tenant, id).await?);
                    }
                    if count > 0 {
                        tracing::warn!(
                            org_id = %seller,
                            paused = count,
                            reason = error.code(),
                            "catalog paused the goods of a marketplace seller without a valid identity verification"
                        );
                    }
                    paused += count;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(paused)
    }
}

/// Job `catalog.seller_kyc_sweep`: mọi khách có hàng `active` của nút khác gốc (mọi database).
pub(crate) struct SellerKycSweep(pub std::sync::Arc<CatalogService>);

#[async_trait::async_trait]
impl JobHandler for SellerKycSweep {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        // tenant-scope: việc nền quét mọi khách của database để tìm sàn có hàng của người bán; mỗi khách sau đó
        // đọc và ghi qua `TenantContext` của chính nó
        let rows: Vec<(Uuid, Uuid)> = product::Entity::find()
            .select_only()
            .column(product::Column::TenantId)
            .column(product::Column::OrgId)
            .distinct()
            .filter(product::Column::Status.eq("active"))
            .filter(Expr::col(product::Column::OrgId).not_equals(product::Column::TenantId))
            .into_tuple()
            .all(job.database.read())
            .await?;
        let tenants: BTreeSet<Uuid> = rows.into_iter().map(|(tenant, _)| tenant).collect();
        // Lỗi của một khách không chặn việc kiểm các khách khác; job vẫn báo lỗi để chạy lại.
        let mut failed = None;
        for tenant in tenants {
            let result = async {
                let context = self.0.databases.tenant(TenantId::from_uuid(tenant)).await?;
                self.0.pause_unverified_sellers(&context, None).await
            }
            .await;
            if let Err(error) = result {
                tracing::error!(tenant_id = %tenant, code = error.code(), "seller identity sweep failed for a tenant");
                failed.get_or_insert(error);
            }
        }
        failed.map_or(Ok(()), Err)
    }
}
