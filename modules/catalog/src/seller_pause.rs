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
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use uuid::Uuid;

use crate::entity::product;
use crate::service::CatalogService;

/// Job hằng ngày kiểm lại người bán của mọi sàn (mọi database).
pub(crate) const SELLER_KYC_SWEEP: &str = "catalog.seller_kyc_sweep";

/// Số sản phẩm đọc mỗi lượt: một sàn lớn không bị nạp hết vào bộ nhớ. Test dùng trang nhỏ để đi qua nhiều trang.
const PAGE: u64 = if cfg!(test) { 2 } else { 500 };

/// Một trang (sản phẩm, nút bán) của hàng `active` thuộc nút khác gốc của khách, theo ID tăng dần sau `after`, đọc
/// qua `connection` người gọi đưa (đã lọc theo khách).
async fn on_sale_page<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    after: Option<Uuid>,
) -> Result<Vec<(Uuid, Uuid)>, Error> {
    let root = root_of(tenant.id()).as_uuid();
    let mut select = product::Entity::find()
        .select_only()
        .column(product::Column::Id)
        .column(product::Column::OrgId)
        .filter(product::Column::Status.eq("active"))
        .filter(product::Column::OrgId.ne(root))
        .scoped(tenant);
    if let Some(after) = after {
        select = select.filter(product::Column::Id.gt(after));
    }
    Ok(select.order_by_asc(product::Column::Id).limit(PAGE).into_tuple().all(connection).await?)
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
    /// Kiểm lại người bán có hàng `active` (trong nhánh `within`, hay cả khách khi `None`) và dừng hàng của người bán
    /// không còn qua được cổng định danh. Khách không phải sàn: không đọc gì. Đọc theo trang qua `connection` (đã lọc
    /// theo khách); mỗi người bán chỉ hỏi cổng một lần. Trả số sản phẩm đã dừng.
    pub(crate) async fn pause_unverified_sellers<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        within: Option<OrgId>,
    ) -> Result<u64, Error> {
        if !self.is_marketplace(tenant).await? {
            return Ok(0);
        }
        // Người bán đã xét: `Some(lý do)` khi phải dừng, `None` khi được bán hay ngoài nhánh.
        let mut decided: BTreeMap<Uuid, Option<&'static str>> = BTreeMap::new();
        let (mut after, mut paused) = (None, 0);
        loop {
            let page = on_sale_page(connection, tenant, after).await?;
            let Some(&(last, _)) = page.last() else { break };
            after = Some(last);
            for (id, seller) in page {
                let verdict = match decided.get(&seller) {
                    Some(verdict) => *verdict,
                    None => {
                        let verdict = self.seller_verdict(tenant, seller, within).await?;
                        decided.insert(seller, verdict);
                        verdict
                    }
                };
                let Some(reason) = verdict else { continue };
                if self.pause_as_system(tenant, id).await? {
                    paused += 1;
                    tracing::warn!(
                        org_id = %seller,
                        product_id = %id,
                        reason,
                        "catalog paused goods of a marketplace seller without a valid identity verification"
                    );
                }
            }
        }
        Ok(paused)
    }

    /// Hàng của người bán `seller` phải dừng không: `Some(mã lý do)` khi người bán (trong nhánh `within`) không qua
    /// được cổng định danh; lỗi tạm thời trả ra để thử lại.
    async fn seller_verdict(
        &self,
        tenant: &TenantContext,
        seller: Uuid,
        within: Option<OrgId>,
    ) -> Result<Option<&'static str>, Error> {
        let org = OrgId::from_uuid(seller);
        if let Some(node) = within {
            // Chỉ người bán trong nhánh của nút vừa đổi định danh.
            let inside = match Site::of(tenant, org).await {
                Ok(site) => site.lineage().contains(&node),
                Err(error) if refused(&error) => true,
                Err(error) => return Err(error),
            };
            if !inside {
                return Ok(None);
            }
        }
        match self.check_seller_identity(tenant, org).await {
            Ok(()) => Ok(None),
            Err(error) if error.code() == "compliance.kyc_required" => {
                Ok(Some("compliance.kyc_required"))
            }
            Err(error) if error.code() == "catalog.seller_verification_unavailable" => {
                Ok(Some("catalog.seller_verification_unavailable"))
            }
            Err(error) if error.code() == "site_not_found" => Ok(Some("site_not_found")),
            Err(error) => Err(error),
        }
    }
}

/// Job `catalog.seller_kyc_sweep`: mọi khách có hàng `active` của nút khác gốc (mọi database).
pub(crate) struct SellerKycSweep(pub std::sync::Arc<CatalogService>);

#[async_trait::async_trait]
impl JobHandler for SellerKycSweep {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        // tenant-scope: việc nền tìm khách có hàng đang bán của nút khác gốc (chỉ ID khách, không nạp sản phẩm); mỗi
        // khách sau đó đọc theo trang, lọc theo chính khách đó
        let tenants: Vec<Uuid> = product::Entity::find()
            .select_only()
            .column(product::Column::TenantId)
            .distinct()
            .filter(product::Column::Status.eq("active"))
            .filter(Expr::col(product::Column::OrgId).not_equals(product::Column::TenantId))
            .into_tuple()
            .all(job.database.read())
            .await?;
        // Lỗi của một khách không chặn việc kiểm các khách khác; job vẫn báo lỗi để chạy lại.
        let mut failed = None;
        for tenant in tenants {
            let result = async {
                let context = self.0.databases.tenant(TenantId::from_uuid(tenant)).await?;
                // Khách không phải sàn: `pause_unverified_sellers` thôi ngay, không đọc sản phẩm.
                self.0.pause_unverified_sellers(job.database.read(), &context, None).await
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
