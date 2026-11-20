//! Danh sách hồ sơ cho nhân viên (bước 47 mục 1): theo phạm vi `customers.read`, tìm theo tên không
//! dấu / số điện thoại / email, lọc theo nhóm, hạng, nhãn, nút; lịch sử mua của một hồ sơ.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::Query;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::entity::{group_member, order_fact, profile};
use crate::permissions::READ;
use crate::profiles::ProfileFilter;
use crate::service::{CUSTOMER, CustomersService};

impl CustomersService {
    /// Một trang hồ sơ trong phạm vi người gọi.
    pub(crate) async fn list(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: &ProfileFilter,
        page: &PageRequest,
    ) -> Result<Page<profile::Model>, Error> {
        let scope = principal.scope_condition(&READ, profile::Column::OrgId, None)?;
        let mut select = profile::Entity::find().filter(scope).scoped(tenant);
        if let Some(q) = filter.q.as_deref().filter(|q| !q.trim().is_empty()) {
            select = select.filter(self.search_condition(tenant, q).await?);
        }
        if let Some(group_id) = filter.group_id {
            let members = Query::select()
                .column(group_member::Column::ProfileId)
                .from(group_member::Entity)
                .and_where(group_member::Column::TenantId.eq(tenant.id().as_uuid()))
                .and_where(group_member::Column::GroupId.eq(group_id))
                .to_owned();
            select = select.filter(profile::Column::Id.in_subquery(members));
        }
        if let Some(tier) = &filter.tier {
            select = select.filter(profile::Column::TierCode.eq(tier.as_str()));
        }
        if !filter.tag_ids.is_empty() {
            select = select.filter(autoxide_kernel::tags::TagIndex.condition(
                tenant,
                CUSTOMER,
                profile::Column::Id,
                &filter.tag_ids,
                &[],
            ));
        }
        if let Some(org) = filter.org_id {
            select = select.filter(profile::Column::OrgId.eq(org));
        }
        fetch_page(select, profile::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Lịch sử mua của hồ sơ `id` (mới nhất trước), từ `customers_order_facts`.
    pub(crate) async fn orders_of(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<order_fact::Model>, Error> {
        let found = self.get(tenant, principal, id).await?;
        self.order_page(tenant, found.id, page).await
    }

    /// Lịch sử mua của hồ sơ `profile_id`, không kiểm quyền (người mua xem của mình).
    pub(crate) async fn order_page(
        &self,
        tenant: &TenantContext,
        profile_id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<order_fact::Model>, Error> {
        let select = order_fact::Entity::find()
            .filter(order_fact::Column::ProfileId.eq(profile_id))
            .scoped(tenant);
        fetch_page(
            select,
            order_fact::Column::OrderId,
            |row| row.order_id,
            page,
            tenant.database().read(),
        )
        .await
    }
}
