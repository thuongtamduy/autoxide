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
//! Người bán không qua được cổng (`compliance.kyc_required`, hay sàn thiếu `compliance`) thì mọi sản phẩm
//! `active` của nút đó chuyển `inactive`, mỗi sản phẩm phát `catalog.product_status_changed` với tác nhân hệ
//! thống. Gọi lại vô hại (D-054): chỉ dòng còn `active` bị đổi, lần sau không còn gì để đổi. Định danh lại xong
//! thì người bán tự mở bán lại — không tự bật lại hàng (người bán có thể đã muốn tắt).

use std::collections::BTreeSet;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::tenancy::{OrgId, OrgTree, ScopeExt, TenantContext, TenantId, root_of};
use sea_orm::sea_query::{Expr, ExprTrait};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use uuid::Uuid;

use crate::entity::product;
use crate::events::{self, Change};
use crate::products::snapshot;
use crate::service::{CatalogService, now};

/// Job hằng ngày kiểm lại người bán của mọi sàn (mọi database).
pub(crate) const SELLER_KYC_SWEEP: &str = "catalog.seller_kyc_sweep";

/// Mã lỗi của cổng nghĩa là "người bán này không được bán" (khác lỗi tạm thời như database).
fn refused(error: &Error) -> bool {
    matches!(error.code(), "compliance.kyc_required" | "catalog.seller_verification_unavailable")
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
        let root = root_of(tenant.id());
        let mut select = product::Entity::find()
            .select_only()
            .column(product::Column::OrgId)
            .distinct()
            .filter(product::Column::Status.eq("active"))
            .filter(product::Column::OrgId.ne(root.as_uuid()))
            .scoped(tenant);
        if let Some(node) = within {
            select = select.filter(OrgTree::subtree_condition(
                tenant.id(),
                &[node],
                product::Column::OrgId,
            ));
        }
        let sellers: Vec<Uuid> = select.into_tuple().all(tenant.database().read()).await?;
        let mut paused = 0;
        for seller in sellers {
            let org = OrgId::from_uuid(seller);
            match self.check_seller_identity(tenant, org).await {
                Ok(()) => {}
                Err(error) if refused(&error) => {
                    paused += self.pause_products_of(tenant, org, error.code()).await?;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(paused)
    }

    /// Chuyển mọi sản phẩm `active` của đúng nút `org` sang `inactive` (một transaction ghi ngắn mỗi sản phẩm).
    async fn pause_products_of(
        &self,
        tenant: &TenantContext,
        org: OrgId,
        reason: &str,
    ) -> Result<u64, Error> {
        let ids: Vec<Uuid> = product::Entity::find()
            .select_only()
            .column(product::Column::Id)
            .filter(product::Column::OrgId.eq(org.as_uuid()))
            .filter(product::Column::Status.eq("active"))
            .scoped(tenant)
            .into_tuple()
            .all(tenant.database().read())
            .await?;
        let mut paused = 0;
        for id in ids {
            let transaction = tenant.database().begin_write().await?;
            let Some(before) =
                product::Entity::find_by_id(id).scoped(tenant).one(&transaction).await?
            else {
                transaction.rollback().await?;
                continue;
            };
            let at = now();
            // Đổi có điều kiện `status = 'active'`: người khác vừa đổi (hay event đến lại) thì không đổi gì.
            let changed = product::Entity::update_many()
                .col_expr(product::Column::Status, Expr::value("inactive"))
                .col_expr(product::Column::UpdatedAt, Expr::value(at))
                .filter(product::Column::Id.eq(id))
                .filter(product::Column::Status.eq("active"))
                .scoped(tenant)
                .exec(&transaction)
                .await?
                .rows_affected;
            if changed == 0 {
                transaction.rollback().await?;
                continue;
            }
            let after =
                product::Model { status: "inactive".to_owned(), updated_at: at, ..before.clone() };
            let change = Change { before: Some(snapshot(&before)), after: Some(snapshot(&after)) };
            let event = events::ProductStatusChanged(change);
            let subject = events::product_subject(id);
            self.outbox.record(&transaction, tenant, Actor::system(), subject, &event).await?;
            self.queue_products(&transaction, tenant, vec![id]).await?;
            transaction.commit().await?;
            paused += 1;
        }
        if paused > 0 {
            tracing::warn!(
                org_id = %org.as_uuid(),
                paused,
                reason,
                "catalog paused the goods of a marketplace seller without a valid identity verification"
            );
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
        for tenant in tenants {
            let context = self.0.databases.tenant(TenantId::from_uuid(tenant)).await?;
            self.0.pause_unverified_sellers(&context, None).await?;
        }
        Ok(())
    }
}
