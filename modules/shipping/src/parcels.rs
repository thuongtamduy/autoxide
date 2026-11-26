//! Chia kiện và tách vận đơn (giao một phần) — chỉ khi vận đơn còn `pending` (chưa đặt hãng).

use std::collections::BTreeMap;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::secret::sha256;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use uuid::Uuid;

use crate::api::{ShipmentStatus, events as names};
use crate::entity::{parcel, shipment, shipment_item};
use crate::errors;
use crate::permissions::SHIPMENTS_MANAGE;
use crate::service::{ShippingService, now};
use crate::settings::writable;
use crate::shipments::{
    ParcelInput, ShipmentView, Source, StatusUpdate, invalid, lock_shipment, status_of,
};

impl ShippingService {
    /// Chia lại kiện (chỉ khi `pending`): mỗi kiện cân nặng, kích thước và các dòng nằm trong nó.
    pub(crate) async fn put_parcels(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        parcels: Vec<ParcelInput>,
    ) -> Result<ShipmentView, Error> {
        let mut problems = Vec::new();
        if parcels.is_empty() {
            problems.push(FieldError::new("parcels", "required", "list at least one parcel"));
        }
        for (index, input) in parcels.iter().enumerate() {
            if input.weight_grams <= 0 {
                problems.push(FieldError::new(
                    format!("parcels[{index}].weight_grams"),
                    "range",
                    "must be more than 0",
                ));
            }
        }
        invalid(problems)?;
        let transaction = tenant.database().begin_write().await?;
        let row = lock_shipment(&transaction, tenant, id).await?;
        writable(principal, &transaction, &SHIPMENTS_MANAGE, row.org_id, || {
            errors::shipment_not_found(id)
        })
        .await?;
        if status_of(&row) != ShipmentStatus::Pending {
            return Err(errors::already_booked());
        }
        let items = shipment_item::Entity::find()
            .filter(shipment_item::Column::ShipmentId.eq(id))
            .scoped(tenant)
            .all(&transaction)
            .await?;
        let mut placed: BTreeMap<String, Uuid> = BTreeMap::new();
        parcel::Entity::delete_many()
            .filter(parcel::Column::ShipmentId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let mut first = None;
        for (index, input) in parcels.iter().enumerate() {
            let parcel_id = Uuid::now_v7();
            first.get_or_insert(parcel_id);
            parcel::ActiveModel {
                id: Set(parcel_id),
                tenant_id: Set(tenant.id().as_uuid()),
                shipment_id: Set(id),
                position: Set(i32::try_from(index + 1).unwrap_or(i32::MAX)),
                weight_grams: Set(input.weight_grams),
                length_mm: Set(input.length_mm),
                width_mm: Set(input.width_mm),
                height_mm: Set(input.height_mm),
                tracking_number: Set(None),
            }
            .insert(&transaction)
            .await?;
            for key in &input.items {
                placed.insert(key.clone(), parcel_id);
            }
        }
        for item in items {
            let parcel = placed.get(&item.order_line_key).copied().or(first);
            shipment_item::Entity::update_many()
                .col_expr(shipment_item::Column::ParcelId, Expr::value(parcel))
                .filter(shipment_item::Column::Id.eq(item.id))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
        }
        let view = self.view_of(&transaction, tenant, row).await?;
        transaction.commit().await?;
        Ok(view)
    }

    /// Tách các dòng `lines` (khoá dòng → số lượng) của vận đơn `pending` sang vận đơn mới (giao
    /// một phần). Vận đơn mới có `request_key` = `<khoá cũ>/split-<n>`.
    pub(crate) async fn split(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        lines: BTreeMap<String, i64>,
    ) -> Result<ShipmentView, Error> {
        let transaction = tenant.database().begin_write().await?;
        let row = lock_shipment(&transaction, tenant, id).await?;
        writable(principal, &transaction, &SHIPMENTS_MANAGE, row.org_id, || {
            errors::shipment_not_found(id)
        })
        .await?;
        if status_of(&row) != ShipmentStatus::Pending {
            return Err(errors::already_booked());
        }
        let items = shipment_item::Entity::find()
            .filter(shipment_item::Column::ShipmentId.eq(id))
            .scoped(tenant)
            .all(&transaction)
            .await?;
        let mut problems = Vec::new();
        let mut moving_total = 0i64;
        let total: i64 = items.iter().map(|item| item.quantity).sum();
        for (key, quantity) in &lines {
            let have: i64 = items
                .iter()
                .filter(|item| &item.order_line_key == key)
                .map(|item| item.quantity)
                .sum();
            if *quantity <= 0 || *quantity > have {
                problems.push(
                    FieldError::new(format!("lines.{key}"), "range", "must be 1 to {max}")
                        .with_param("max", have),
                );
            } else {
                // Chỉ cộng số hợp lệ (≤ số đang có): số ngoài khoảng do client gửi làm tràn (PT-5).
                moving_total = moving_total.saturating_add(*quantity);
            }
        }
        if lines.is_empty() || moving_total >= total {
            problems.push(FieldError::new("lines", "split", "move some, not all, of the items"));
        }
        invalid(problems)?;
        let siblings = shipment::Entity::find()
            .filter(shipment::Column::OrderId.eq(row.order_id))
            .scoped(tenant)
            .all(&transaction)
            .await?
            .len();
        let at = now();
        let mut new_row = row.clone();
        new_row.id = Uuid::now_v7();
        new_row.request_key =
            format!("{}/split-{siblings}", row.request_key).chars().take(100).collect();
        new_row.request_fingerprint = sha256(&format!("split:{}:{siblings}", row.id));
        new_row.created_at = at;
        new_row.updated_at = at;
        new_row.status_at = at;
        // COD ở lại vận đơn gốc: số tiền thu hộ do `orders` quyết, tách hàng không chia tiền.
        new_row.cod_minor = None;
        new_row.cod_status = "none".to_owned();
        let new_row = sea_orm::IntoActiveModel::into_active_model(new_row)
            .reset_all()
            .insert(&transaction)
            .await?;
        let parcel_id = Uuid::now_v7();
        let mut weight = 0i64;
        for item in items {
            let Some(moving) = lines.get(&item.order_line_key).copied() else { continue };
            let take = moving.min(item.quantity);
            weight += item.weight_grams.unwrap_or(0).saturating_mul(take);
            let mut copy = item.clone();
            copy.id = Uuid::now_v7();
            copy.shipment_id = new_row.id;
            copy.parcel_id = Some(parcel_id);
            copy.quantity = take;
            sea_orm::IntoActiveModel::into_active_model(copy)
                .reset_all()
                .insert(&transaction)
                .await?;
            if take == item.quantity {
                shipment_item::Entity::delete_by_id(item.id)
                    .scoped(tenant)
                    .exec(&transaction)
                    .await?;
            } else {
                shipment_item::Entity::update_many()
                    .col_expr(shipment_item::Column::Quantity, Expr::value(item.quantity - take))
                    .filter(shipment_item::Column::Id.eq(item.id))
                    .scoped(tenant)
                    .exec(&transaction)
                    .await?;
            }
        }
        parcel::ActiveModel {
            id: Set(parcel_id),
            tenant_id: Set(tenant.id().as_uuid()),
            shipment_id: Set(new_row.id),
            position: Set(1),
            weight_grams: Set(weight),
            length_mm: Set(None),
            width_mm: Set(None),
            height_mm: Set(None),
            tracking_number: Set(None),
        }
        .insert(&transaction)
        .await?;
        self.add_event(
            &transaction,
            tenant,
            new_row.id,
            &StatusUpdate {
                event_key: "created".to_owned(),
                status: ShipmentStatus::Pending,
                carrier_status: String::new(),
                occurred_at: at,
                note: format!("split from {}", row.id),
                source: Source::Staff,
            },
        )
        .await?;
        self.emit(
            &transaction,
            tenant,
            Actor::user(principal.user_id()),
            &[names::SHIPMENT_CREATED],
            None,
            &new_row,
        )
        .await?;
        let view = self.view_of(&transaction, tenant, new_row).await?;
        transaction.commit().await?;
        Ok(view)
    }
}
