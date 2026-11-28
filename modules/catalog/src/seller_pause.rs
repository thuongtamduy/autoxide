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

use std::collections::BTreeMap;

use autoxide_kernel::error::Error;
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, Site, TenantContext, TenantId, root_of};
use sea_orm::sea_query::{Expr, ExprTrait};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QuerySelect};
use uuid::Uuid;

use crate::entity::product;
use crate::service::CatalogService;

/// Job hằng ngày kiểm lại người bán của mọi sàn (mọi database).
pub(crate) const SELLER_KYC_SWEEP: &str = "catalog.seller_kyc_sweep";

/// (sản phẩm, nút bán) của hàng `active` thuộc nút khác gốc của khách, đọc qua `connection` người gọi đưa (đã lọc
/// theo khách).
pub(crate) async fn on_sale<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
) -> Result<Vec<(Uuid, Uuid)>, Error> {
    let root = root_of(tenant.id()).as_uuid();
    Ok(product::Entity::find()
        .select_only()
        .column(product::Column::Id)
        .column(product::Column::OrgId)
        .filter(product::Column::Status.eq("active"))
        .filter(product::Column::OrgId.ne(root))
        .scoped(tenant)
        .into_tuple()
        .all(connection)
        .await?)
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
    /// Kiểm lại người bán của hàng đang bán `on_sale` ((sản phẩm, nút bán) — trong nhánh `within`, hay cả khách khi
    /// `None`) và dừng hàng của người bán không còn qua được cổng định danh. Khách không phải sàn: không làm gì. Trả
    /// số sản phẩm đã dừng.
    pub(crate) async fn pause_unverified_sellers(
        &self,
        tenant: &TenantContext,
        within: Option<OrgId>,
        on_sale: Vec<(Uuid, Uuid)>,
    ) -> Result<u64, Error> {
        if on_sale.is_empty() || !self.is_marketplace(tenant).await? {
            return Ok(0);
        }
        let mut by_seller: BTreeMap<Uuid, Vec<Uuid>> = BTreeMap::new();
        for (id, org) in on_sale {
            by_seller.entry(org).or_default().push(id);
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
        // ghi qua `TenantContext` của chính nó
        let rows: Vec<(Uuid, Uuid, Uuid)> = product::Entity::find()
            .select_only()
            .column(product::Column::TenantId)
            .column(product::Column::Id)
            .column(product::Column::OrgId)
            .filter(product::Column::Status.eq("active"))
            .filter(Expr::col(product::Column::OrgId).not_equals(product::Column::TenantId))
            .into_tuple()
            .all(job.database.read())
            .await?;
        let mut tenants: BTreeMap<Uuid, Vec<(Uuid, Uuid)>> = BTreeMap::new();
        for (tenant, id, org) in rows {
            tenants.entry(tenant).or_default().push((id, org));
        }
        // Lỗi của một khách không chặn việc kiểm các khách khác; job vẫn báo lỗi để chạy lại.
        let mut failed = None;
        for (tenant, on_sale) in tenants {
            let result = async {
                let context = self.0.databases.tenant(TenantId::from_uuid(tenant)).await?;
                self.0.pause_unverified_sellers(&context, None, on_sale).await
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
