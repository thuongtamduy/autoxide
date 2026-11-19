//! Hộp việc (bước 51): mỗi yêu cầu gọi lại đang mở là MỘT mục, đếm thẳng từ bảng yêu cầu — không có
//! bảng việc riêng để lệch.

use autoxide_kernel::access::{OrgTarget, Permission, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Order, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::work::{Severity, WorkItem, WorkSource};
use sea_orm::sea_query::Condition;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

use crate::entity::callback;
use crate::permissions::CALL;

pub(crate) struct Callbacks;

impl Callbacks {
    fn condition(principal: &Principal) -> Result<Condition, Error> {
        let scope = principal.scope_condition(
            &CALL,
            callback::Column::OrgId,
            Some(callback::Column::AssignedUserId),
        )?;
        Ok(Condition::all().add(scope).add(callback::Column::Status.eq("open")))
    }
}

#[async_trait::async_trait]
impl WorkSource for Callbacks {
    fn id(&self) -> &'static str {
        "conversations.callbacks"
    }

    fn permission(&self) -> Permission {
        CALL
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        _: OrgTarget,
    ) -> Result<u64, Error> {
        Ok(callback::Entity::find()
            .filter(Self::condition(principal)?)
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
        let page = PageRequest::first(u32::from(limit), Order::Asc);
        let select = callback::Entity::find().filter(Self::condition(principal)?).scoped(tenant);
        let rows =
            fetch_page(select, callback::Column::Id, |row| row.id, &page, tenant.database().read())
                .await?
                .data;
        Ok(rows
            .into_iter()
            .map(|row| {
                WorkItem::new(
                    row.id.to_string(),
                    Severity::Attention,
                    "Call back customer {phone}",
                    "conversations.callbacks",
                )
                .param("phone", row.phone_masked)
                .due_at(Some(row.window_end))
            })
            .collect())
    }
}
