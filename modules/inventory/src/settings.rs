//! Kho phục vụ một cửa hàng (`inventory_sources`) và cách cửa hàng chọn kho
//! (`inventory_settings`); quản trị chỗ giữ (xem, gỡ kẹt bằng tay).

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use uuid::Uuid;

use crate::api::{ReleaseReason, Reservation, ReservationStatus};
use crate::entity::{reservation, settings, source};
use crate::permissions::{ADJUST, RESERVATIONS_READ, SETTINGS_MANAGE};
use crate::reservations::{load, not_found};
use crate::service::{InventoryService, fail_if_any, now, readable, writable};

/// Số kho tối đa phục vụ một cửa hàng.
const MAX_SOURCES: usize = 100;

/// Một kho phục vụ cửa hàng.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceInput {
    pub warehouse_org_id: Uuid,
    pub priority: i32,
    pub active: bool,
}

/// Cách cửa hàng chọn kho.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct StoreSettings {
    pub prefer_nearby: bool,
    pub allow_split: bool,
}

impl InventoryService {
    pub(crate) async fn sources(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        selling: Uuid,
    ) -> Result<Vec<SourceInput>, Error> {
        let read = tenant.database().read();
        principal.require_at(read, &SETTINGS_MANAGE, crate::service::at(selling)).await?;
        let rows = source::Entity::find()
            .filter(source::Column::SellingOrgId.eq(selling))
            .scoped(tenant)
            .order_by_asc(source::Column::Priority)
            .order_by_asc(source::Column::Id)
            .all(read)
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| SourceInput {
                warehouse_org_id: row.warehouse_org_id,
                priority: row.priority,
                active: row.active,
            })
            .collect())
    }

    pub(crate) async fn set_sources(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        selling: Uuid,
        sources: Vec<SourceInput>,
    ) -> Result<Vec<SourceInput>, Error> {
        let mut problems = Vec::new();
        if sources.len() > MAX_SOURCES {
            problems.push(
                FieldError::new("sources", "length", "must hold at most {max} warehouses")
                    .with_param("max", MAX_SOURCES),
            );
        }
        let mut seen = std::collections::BTreeSet::new();
        for (index, item) in sources.iter().enumerate() {
            if !seen.insert(item.warehouse_org_id) {
                problems.push(FieldError::new(
                    format!("sources[{index}].warehouse_org_id"),
                    "duplicate",
                    "lists the same warehouse twice",
                ));
            }
        }
        fail_if_any(problems)?;
        for item in &sources {
            self.check_stock_unit(tenant, item.warehouse_org_id).await?;
        }
        let transaction = tenant.database().begin_write().await?;
        writable(principal, &transaction, &SETTINGS_MANAGE, selling).await?;
        source::Entity::delete_many()
            .filter(source::Column::SellingOrgId.eq(selling))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        for item in &sources {
            source::Entity::insert(source::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                selling_org_id: Set(selling),
                warehouse_org_id: Set(item.warehouse_org_id),
                priority: Set(item.priority),
                active: Set(item.active),
            })
            .exec_without_returning(&transaction)
            .await?;
        }
        transaction.commit().await?;
        self.sources(tenant, principal, selling).await
    }

    pub(crate) async fn store_settings(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Uuid,
    ) -> Result<StoreSettings, Error> {
        let read = tenant.database().read();
        principal.require_at(read, &SETTINGS_MANAGE, crate::service::at(org)).await?;
        let row = settings::Entity::find()
            .filter(settings::Column::OrgId.eq(org))
            .scoped(tenant)
            .one(read)
            .await?;
        Ok(row.map_or_else(StoreSettings::default, |row| StoreSettings {
            prefer_nearby: row.prefer_nearby,
            allow_split: row.allow_split,
        }))
    }

    pub(crate) async fn set_store_settings(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Uuid,
        input: StoreSettings,
    ) -> Result<StoreSettings, Error> {
        let transaction = tenant.database().begin_write().await?;
        writable(principal, &transaction, &SETTINGS_MANAGE, org).await?;
        settings::Entity::insert(settings::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(org),
            prefer_nearby: Set(input.prefer_nearby),
            allow_split: Set(input.allow_split),
            province_code: Set(None),
            pickup: Set(false),
            updated_at: Set(now()),
        })
        .on_conflict(
            OnConflict::columns([settings::Column::TenantId, settings::Column::OrgId])
                .update_columns([
                    settings::Column::PreferNearby,
                    settings::Column::AllowSplit,
                    settings::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec_without_returning(&transaction)
        .await?;
        transaction.commit().await?;
        Ok(input)
    }

    pub(crate) async fn list_reservations(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        order: Option<Uuid>,
        status: Option<ReservationStatus>,
        page: &PageRequest,
    ) -> Result<Page<Reservation>, Error> {
        let scope = principal.scope_condition(
            &RESERVATIONS_READ,
            reservation::Column::SellingOrgId,
            None,
        )?;
        let mut select = reservation::Entity::find().scoped(tenant).filter(scope);
        if let Some(order) = order {
            select = select.filter(reservation::Column::OrderId.eq(order));
        }
        if let Some(status) = status {
            select = select.filter(reservation::Column::Status.eq(status.as_str()));
        }
        let read = tenant.database().read();
        let found = fetch_page(select, reservation::Column::Id, |row| row.id, page, read).await?;
        let mut data = Vec::with_capacity(found.data.len());
        for row in found.data {
            data.push(load(read, tenant, row).await?);
        }
        Ok(Page { data, pagination: found.pagination })
    }

    /// Gỡ chỗ giữ `id` bằng tay (cần `inventory.adjust` ở nút bán); event ghi người gỡ.
    pub(crate) async fn release_by_hand(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Reservation, Error> {
        let read = tenant.database().read();
        let row = reservation::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| not_found(id))?;
        readable(principal, read, &ADJUST, row.selling_org_id, || not_found(id)).await?;
        {
            let transaction = tenant.database().begin_write().await?;
            writable(principal, &transaction, &ADJUST, row.selling_org_id).await?;
        }
        let order = row.order_id;
        self.release(tenant, order, ReleaseReason::Manual, &Actor::from(principal))
            .await?
            .ok_or_else(|| not_found(id))
    }
}
