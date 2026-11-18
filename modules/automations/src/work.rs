//! Hộp việc (bước 51 P2): lần chạy có bước rủi ro chờ người duyệt, luật tự tạm dừng.

use std::sync::Arc;

use autoxide_kernel::access::{OrgTarget, Permission, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Order, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::work::{Severity, WorkItem, WorkSource};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

use crate::entity::{rule, run};
use crate::permissions::{AUTOMATIONS_MANAGE, AUTOMATIONS_RUN};
use crate::runner::AWAITING_APPROVAL;
use crate::service::rules::PAUSED;

/// Màn hình của module trong sổ điều hướng.
pub(crate) const SCREEN: &str = "settings.automations";

/// Lần chạy chờ duyệt, trong phạm vi `automations.run` của người gọi.
pub(crate) struct Approvals;

#[async_trait::async_trait]
impl WorkSource for Approvals {
    fn id(&self) -> &'static str {
        "automations.approvals"
    }

    fn permission(&self) -> Permission {
        AUTOMATIONS_RUN
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        _: OrgTarget,
    ) -> Result<u64, Error> {
        let scope = principal.scope_condition(&AUTOMATIONS_RUN, run::Column::OrgId, None)?;
        Ok(run::Entity::find()
            .filter(scope)
            .filter(run::Column::Status.eq(AWAITING_APPROVAL))
            .scoped(tenant)
            .count(tenant.database().read())
            .await?)
    }

    async fn items(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        _: OrgTarget,
        limit: u16,
    ) -> Result<Vec<WorkItem>, Error> {
        let scope = principal.scope_condition(&AUTOMATIONS_RUN, run::Column::OrgId, None)?;
        let select = run::Entity::find()
            .filter(scope)
            .filter(run::Column::Status.eq(AWAITING_APPROVAL))
            .scoped(tenant);
        // Cũ nhất trước: chờ lâu nhất.
        let page = PageRequest::first(u32::from(limit), Order::Asc);
        let rows =
            fetch_page(select, run::Column::Id, |row| row.id, &page, tenant.database().read())
                .await?
                .data;
        Ok(rows
            .into_iter()
            .map(|row| {
                WorkItem::new(
                    row.id.to_string(),
                    Severity::Attention,
                    "Approve or reject a step of an automation run",
                    SCREEN,
                )
            })
            .collect())
    }
}

/// Luật tự tạm dừng (chạy quá nhiều, người chạy mất quyền), trong phạm vi `automations.manage`.
pub(crate) struct Paused;

#[async_trait::async_trait]
impl WorkSource for Paused {
    fn id(&self) -> &'static str {
        "automations.paused"
    }

    fn permission(&self) -> Permission {
        AUTOMATIONS_MANAGE
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        _: OrgTarget,
    ) -> Result<u64, Error> {
        let scope = principal.scope_condition(&AUTOMATIONS_MANAGE, rule::Column::OrgId, None)?;
        Ok(rule::Entity::find()
            .filter(scope)
            .filter(rule::Column::Status.eq(PAUSED))
            .scoped(tenant)
            .count(tenant.database().read())
            .await?)
    }

    async fn items(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        _: OrgTarget,
        limit: u16,
    ) -> Result<Vec<WorkItem>, Error> {
        let scope = principal.scope_condition(&AUTOMATIONS_MANAGE, rule::Column::OrgId, None)?;
        let select = rule::Entity::find()
            .filter(scope)
            .filter(rule::Column::Status.eq(PAUSED))
            .scoped(tenant);
        let page = PageRequest::first(u32::from(limit), Order::Asc);
        let rows =
            fetch_page(select, rule::Column::Id, |row| row.id, &page, tenant.database().read())
                .await?
                .data;
        Ok(rows
            .into_iter()
            .map(|row| {
                WorkItem::new(
                    row.id.to_string(),
                    Severity::Attention,
                    "The automation rule {name} paused itself",
                    SCREEN,
                )
                .param("name", row.name)
            })
            .collect())
    }
}

pub(crate) fn sources() -> Vec<Arc<dyn WorkSource>> {
    vec![Arc::new(Approvals), Arc::new(Paused)]
}
