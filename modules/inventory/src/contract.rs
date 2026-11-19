//! Cài đặt trait `api::Inventory` (gọi thẳng service — cùng hàm với REST, event và job), job hết
//! hạn chỗ giữ và event `inventory` nghe từ module khác.

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Delivered, EventHandler, Subscription};
use autoxide_kernel::job::{JobContext, JobHandler, JobRegistration, Schedule};
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, TenantId};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use uuid::Uuid;

use crate::api::{
    Availability, FulfilRequest, Inventory, MAX_IDS, ReleaseReason, Reservation, ReservationStatus,
    ReserveRequest, RestockRequest,
};
use crate::entity::{level, reservation, settings};
use crate::public::StockLevel;
use crate::service::{InventoryService, now};

pub(crate) const EXPIRE_RESERVATIONS: &str = "inventory.expire_reservations";
pub(crate) const EXPIRE_LOTS: &str = "inventory.expire_lots";

/// Event của `organizations` cho tỉnh và "nhận tại cửa hàng" của nút.
pub(crate) const PROFILE_UPDATED: &str = "organizations.public_profile_updated";

fn too_many(count: usize) -> Result<(), Error> {
    if count > MAX_IDS {
        return Err(Error::validation(vec![
            autoxide_kernel::error::FieldError::new(
                "variant_ids",
                "length",
                "must hold at most {max} ids",
            )
            .with_param("max", MAX_IDS),
        ]));
    }
    Ok(())
}

#[async_trait::async_trait]
impl Inventory for InventoryService {
    async fn availability(
        &self,
        tenant: &TenantContext,
        selling_org_id: OrgId,
        variant_ids: &[Uuid],
    ) -> Result<Vec<Availability>, Error> {
        too_many(variant_ids.len())?;
        let (orgs, _) = self.candidates(tenant, selling_org_id.as_uuid(), None, None).await?;
        let rows = level::Entity::find()
            .filter(level::Column::OrgId.is_in(orgs.clone()))
            .filter(level::Column::VariantId.is_in(variant_ids.to_vec()))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        let mut found = Vec::with_capacity(rows.len());
        for org in &orgs {
            for row in rows.iter().filter(|row| row.org_id == *org) {
                found.push(Availability {
                    variant_id: row.variant_id,
                    warehouse_org_id: OrgId::from_uuid(row.org_id),
                    on_hand: row.on_hand,
                    available: row.available,
                    incoming: row.incoming,
                });
            }
        }
        Ok(found)
    }

    async fn reserve(
        &self,
        tenant: &TenantContext,
        request: &ReserveRequest,
    ) -> Result<Submitted<Reservation>, Error> {
        InventoryService::reserve(self, tenant, request).await
    }

    async fn confirm(&self, tenant: &TenantContext, order_id: Uuid) -> Result<Reservation, Error> {
        InventoryService::confirm(self, tenant, order_id).await
    }

    async fn release(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
        reason: ReleaseReason,
    ) -> Result<Option<Reservation>, Error> {
        InventoryService::release(self, tenant, order_id, reason, &Actor::system()).await
    }

    async fn fulfil(
        &self,
        tenant: &TenantContext,
        request: &FulfilRequest,
    ) -> Result<Reservation, Error> {
        InventoryService::fulfil(self, tenant, request).await
    }

    async fn restock(&self, tenant: &TenantContext, request: &RestockRequest) -> Result<(), Error> {
        InventoryService::restock(self, tenant, request).await
    }

    async fn in_stock_at(
        &self,
        tenant: &TenantContext,
        store_org_id: OrgId,
        variant_ids: &[Uuid],
    ) -> Result<Vec<Uuid>, Error> {
        too_many(variant_ids.len())?;
        if variant_ids.is_empty() {
            return Ok(Vec::new());
        }
        // Cùng cách tính với tình trạng công khai (kho phục vụ cửa hàng, combo, hàng không theo dõi
        // tồn): "còn hàng" là `in_stock` hay `low`; đặt trước chưa về không tính.
        let rows = self.levels_at(tenant, variant_ids, &[store_org_id.as_uuid()]).await?;
        Ok(rows
            .into_iter()
            .filter(|row| matches!(row.level, StockLevel::InStock | StockLevel::Low))
            .map(|row| row.variant_id)
            .collect())
    }

    async fn restock_alerts_of(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        variant_ids: &[Uuid],
    ) -> Result<Vec<Uuid>, Error> {
        too_many(variant_ids.len())?;
        self.restock_alerts_for(tenant, user_id, variant_ids).await
    }
}

impl InventoryService {
    /// Trả chỗ giữ `held` quá hạn trong `database` (mọi khách), tối đa `EXPIRE_BATCH` một lượt;
    /// mỗi chỗ một transaction.
    pub(crate) async fn expire_due(&self, database: &Database) -> Result<u64, Error> {
        let due: Vec<(Uuid, Uuid)> = reservation::Entity::find()
            .select_only()
            .column(reservation::Column::TenantId)
            .column(reservation::Column::OrderId)
            .filter(reservation::Column::Status.eq(ReservationStatus::Held.as_str()))
            .filter(reservation::Column::ExpiresAt.lte(now()))
            .order_by_asc(reservation::Column::ExpiresAt)
            .limit(self.config.expire_batch)
            .into_tuple()
            .all(database.read())
            .await?;
        let mut expired = 0;
        for (tenant, order) in due {
            let tenant = self.databases.tenant(TenantId::from_uuid(tenant)).await?;
            match self.release(&tenant, order, ReleaseReason::Expired, &Actor::system()).await {
                Ok(Some(found)) if found.status == ReservationStatus::Expired => expired += 1,
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(order_id = %order, error = %error, "cannot expire a stock reservation");
                }
            }
        }
        Ok(expired)
    }

    /// Ghi tỉnh và cờ "nhận tại cửa hàng" của nút `org` (từ hồ sơ công khai của `organizations`).
    pub(crate) async fn remember_profile(
        &self,
        tenant: &TenantContext,
        org: Uuid,
        province: Option<String>,
        pickup: bool,
    ) -> Result<(), Error> {
        let row = settings::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(org),
            prefer_nearby: Set(false),
            allow_split: Set(false),
            province_code: Set(province),
            pickup: Set(pickup),
            updated_at: Set(now()),
        };
        settings::Entity::insert(row)
            .on_conflict(
                OnConflict::columns([settings::Column::TenantId, settings::Column::OrgId])
                    .update_columns([
                        settings::Column::ProvinceCode,
                        settings::Column::Pickup,
                        settings::Column::UpdatedAt,
                    ])
                    .to_owned(),
            )
            .exec_without_returning(tenant.database().write())
            .await?;
        Ok(())
    }
}

pub(crate) fn registrations(service: &Arc<InventoryService>) -> Vec<JobRegistration> {
    vec![
        JobRegistration::new(EXPIRE_RESERVATIONS, Arc::new(ExpireReservations(service.clone()))),
        JobRegistration::new(EXPIRE_LOTS, Arc::new(ExpireLots(service.clone()))),
        JobRegistration::new(
            crate::preorders::ALLOCATE_PREORDERS,
            Arc::new(crate::preorders::Allocate(service.clone())),
        ),
        JobRegistration::new(
            crate::receipts::POST_RECEIPT,
            Arc::new(PostToLedger(service.clone(), false)),
        ),
        JobRegistration::new(
            crate::receipts::POST_SUPPLIER_RETURN,
            Arc::new(PostToLedger(service.clone(), true)),
        ),
    ]
}

pub(crate) fn schedules() -> Vec<Schedule> {
    vec![
        Schedule::new(EXPIRE_RESERVATIONS, EXPIRE_RESERVATIONS, Duration::from_secs(60)),
        Schedule::new(EXPIRE_LOTS, EXPIRE_LOTS, Duration::from_secs(24 * 60 * 60)),
    ]
}

/// Ghi phiếu nhập (`false`) hay trả hàng nhà cung cấp (`true`) vào sổ cái.
struct PostToLedger(Arc<InventoryService>, bool);

#[async_trait::async_trait]
impl JobHandler for PostToLedger {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let tenant =
            job.tenant.as_ref().ok_or_else(|| Error::internal("ledger job without a tenant"))?;
        let id = job.payload["id"]
            .as_str()
            .and_then(|id| id.parse::<Uuid>().ok())
            .ok_or_else(|| Error::internal("ledger job without a document id"))?;
        if self.1 {
            self.0.post_supplier_return(tenant, id).await
        } else {
            self.0.post_receipt(tenant, id).await
        }
    }
}

struct ExpireLots(Arc<InventoryService>);

#[async_trait::async_trait]
impl JobHandler for ExpireLots {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.expire_lots(&job.database).await.map(|_| ())
    }
}

struct ExpireReservations(Arc<InventoryService>);

#[async_trait::async_trait]
impl JobHandler for ExpireReservations {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.expire_due(&job.database).await.map(|_| ())
    }
}

pub(crate) fn subscriptions(service: &Arc<InventoryService>) -> Vec<Subscription> {
    vec![Subscription::optional(PROFILE_UPDATED, Arc::new(Listener(service.clone())))]
}

struct Listener(Arc<InventoryService>);

#[async_trait::async_trait]
impl EventHandler for Listener {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        if event.name == PROFILE_UPDATED {
            // Chủ thể `organizations.unit:<id>`; nhận lại event chỉ ghi đè cùng giá trị.
            let Some(org) = event
                .subject
                .as_deref()
                .and_then(|subject| subject.strip_prefix("organizations.unit:"))
                .and_then(|id| id.parse::<Uuid>().ok())
            else {
                return Ok(());
            };
            let after = &event.payload["after"];
            let province = after["address"]["province_code"].as_str().map(str::to_owned);
            let pickup = after["pickup"].as_bool().unwrap_or(false);
            return self.0.remember_profile(&event.tenant, org, province, pickup).await;
        }
        Ok(())
    }
}
