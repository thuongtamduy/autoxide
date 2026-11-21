//! Đặt vận đơn với hãng, huỷ, nhãn, hỏi lại hành trình (bước 44 mục 3–4). Gọi hãng giữa hai
//! transaction: đặt `booking` (commit) → gọi hãng → ghi kết quả (commit). Chết giữa chừng thì vận
//! đơn kẹt `booking`; lần chạy lại và lịch `shipping.reconcile_bookings` hỏi `find_by_reference`
//! trước — không bao giờ gọi `create_shipment` lần hai mà chưa hỏi.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::Money;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use jiff::SignedDuration;
use sea_orm::sea_query::{Expr, ExprTrait};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use uuid::Uuid;

use crate::api::{
    CarrierParcel, CarrierRegistry, CarrierShipment, CarrierShipmentRequest, Direction, Label,
    ShipmentStatus, events as names,
};
use crate::carriers::within;
use crate::entity::{parcel, shipment};
use crate::errors;
use crate::permissions::{SHIPMENTS_MANAGE, SHIPMENTS_READ};
use crate::service::{ShippingService, now};
use crate::settings::{readable, writable};
use crate::shipments::{
    Source, StatusUpdate, address_of_shipment, find_shipment, lock_shipment, status_of,
};
use crate::status;

/// Kết quả một lần đặt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Booked {
    /// Đã có mã vận đơn.
    Done,
    /// Hãng từ chối hẳn: vận đơn về `pending`, kèm mã lỗi.
    Failed(String),
    /// Vận đơn không ở trạng thái đặt được (đã đặt, đã huỷ…): không làm gì.
    Skipped,
}

/// Lỗi tạm (hãng không trả lời): chạy lại sau.
fn transient(error: &Error) -> bool {
    matches!(error.kind(), ErrorKind::Unavailable | ErrorKind::Internal)
}

impl ShippingService {
    /// Đặt vận đơn `id` với hãng của phương thức. Lỗi tạm → `Err` (job chạy lại, vận đơn ở
    /// `booking`); lỗi hẳn → `pending` + `shipping.booking_failed`.
    pub(crate) async fn book(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
    ) -> Result<Booked, Error> {
        // Bước 1: `pending → booking` có điều kiện (commit), hay đã kẹt `booking` từ lần trước.
        let transaction = tenant.database().begin_write().await?;
        let before = lock_shipment(&transaction, tenant, id).await?;
        let current = status_of(&before);
        if !matches!(current, ShipmentStatus::Pending | ShipmentStatus::Booking) {
            return Ok(Booked::Skipped);
        }
        let account_id = before.carrier_account_id.ok_or_else(errors::no_carrier)?;
        let resumed = current == ShipmentStatus::Booking;
        if !resumed {
            shipment::Entity::update_many()
                .col_expr(shipment::Column::Status, Expr::value(ShipmentStatus::Booking.as_str()))
                .col_expr(shipment::Column::StatusAt, Expr::value(now()))
                .col_expr(shipment::Column::Attempts, Expr::col(shipment::Column::Attempts).add(1))
                .col_expr(shipment::Column::UpdatedAt, Expr::value(now()))
                .filter(shipment::Column::Id.eq(id))
                .filter(shipment::Column::Status.eq(ShipmentStatus::Pending.as_str()))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
        }
        transaction.commit().await?;

        // Bước 2: gọi hãng, ngoài transaction.
        let (_, account, carrier) = self.account(tenant, account_id).await?;
        let reference = id.to_string();
        let timeout = self.config.carrier_timeout_seconds;
        let found = if resumed {
            within(timeout, carrier.find_by_reference(&account, &reference)).await?
        } else {
            None
        };
        let outcome = match found {
            Some(existing) => Ok(existing),
            None => {
                let request = self.carrier_request(tenant, &before).await?;
                within(timeout, carrier.create_shipment(&account, &request)).await
            }
        };

        // Bước 3: ghi kết quả.
        match outcome {
            Ok(created) => {
                self.record_booking(tenant, actor, id, carrier.code(), &created).await?;
                Ok(Booked::Done)
            }
            Err(error) if transient(&error) => Err(error),
            Err(error) => {
                self.note_carrier_error(tenant, account_id, &error).await;
                self.booking_failed(tenant, actor, id, error.code()).await?;
                Ok(Booked::Failed(error.code().to_owned()))
            }
        }
    }

    /// Yêu cầu tạo vận đơn gửi hãng.
    async fn carrier_request(
        &self,
        tenant: &TenantContext,
        row: &shipment::Model,
    ) -> Result<CarrierShipmentRequest, Error> {
        let parcels = parcel::Entity::find()
            .filter(parcel::Column::ShipmentId.eq(row.id))
            .scoped(tenant)
            .order_by_asc(parcel::Column::Position)
            .all(tenant.database().read())
            .await?
            .into_iter()
            .map(|item| {
                CarrierParcel::new(item.weight_grams).dimensions(
                    item.length_mm,
                    item.width_mm,
                    item.height_mm,
                )
            })
            .collect();
        let declared = Money::from_columns(row.declared_value_minor, &row.currency)?;
        let cod =
            row.cod_minor.map(|minor| Money::from_columns(minor, &row.currency)).transpose()?;
        Ok(CarrierShipmentRequest::new(
            row.id.to_string(),
            (row.recipient_name.clone(), row.recipient_phone.clone()),
            address_of_shipment(row)?,
            parcels,
            declared,
        )
        .service(row.service_code.clone())
        .direction(Direction::parse(&row.direction).unwrap_or(Direction::Outbound))
        .cod(cod)
        .note(row.note.clone()))
    }

    /// Ghi mã vận đơn hãng cấp: `booking → ready_to_ship`.
    async fn record_booking(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
        carrier_code: &str,
        created: &CarrierShipment,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        let before = lock_shipment(&transaction, tenant, id).await?;
        if status_of(&before) != ShipmentStatus::Booking {
            return Ok(());
        }
        let mut after = before.clone();
        after.carrier_code = Some(carrier_code.to_owned());
        after.tracking_number = Some(created.tracking_number.clone());
        after.actual_fee_minor = created.fee.map(Money::minor);
        after.last_error_code = None;
        let saved = shipment::Entity::update(
            sea_orm::IntoActiveModel::into_active_model(after).reset_all(),
        )
        .validate()?
        .scoped(tenant)
        .exec(&transaction)
        .await?;
        let parcels = parcel::Entity::find()
            .filter(parcel::Column::ShipmentId.eq(id))
            .scoped(tenant)
            .order_by_asc(parcel::Column::Position)
            .all(&transaction)
            .await?;
        for (row, tracking) in parcels.iter().zip(&created.parcel_tracking_numbers) {
            parcel::Entity::update_many()
                .col_expr(parcel::Column::TrackingNumber, Expr::value(Some(tracking.clone())))
                .filter(parcel::Column::Id.eq(row.id))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
        }
        self.emit(&transaction, tenant, actor, &[names::SHIPMENT_BOOKED], Some(&before), &saved)
            .await?;
        let update = StatusUpdate {
            event_key: "booked".to_owned(),
            status: ShipmentStatus::ReadyToShip,
            carrier_status: String::new(),
            occurred_at: now(),
            note: created.tracking_number.clone(),
            source: Source::Carrier,
        };
        self.add_event(&transaction, tenant, id, &update).await?;
        self.set_status(&transaction, tenant, actor, &saved, ShipmentStatus::ReadyToShip, now())
            .await?;
        transaction.commit().await?;
        tracing::info!(shipment_id = %id, carrier = carrier_code, "shipment booked with the carrier");
        Ok(())
    }

    /// Hãng từ chối hẳn: `booking → pending`, ghi mã lỗi, event `shipping.booking_failed`.
    async fn booking_failed(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
        code: &str,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        let before = lock_shipment(&transaction, tenant, id).await?;
        if status_of(&before) != ShipmentStatus::Booking {
            return Ok(());
        }
        let mut after = before.clone();
        after.status = ShipmentStatus::Pending.as_str().to_owned();
        after.status_at = now();
        after.last_error_code = Some(code.chars().take(100).collect());
        after.updated_at = now();
        let saved = shipment::Entity::update(
            sea_orm::IntoActiveModel::into_active_model(after).reset_all(),
        )
        .validate()?
        .scoped(tenant)
        .exec(&transaction)
        .await?;
        self.emit(&transaction, tenant, actor, &[names::BOOKING_FAILED], Some(&before), &saved)
            .await?;
        transaction.commit().await?;
        tracing::warn!(shipment_id = %id, error_code = code, "booking with the carrier failed");
        Ok(())
    }

    /// Nhân viên bấm "Đặt với hãng".
    pub(crate) async fn book_by_staff(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<crate::shipments::ShipmentView, Error> {
        let row = find_shipment(tenant.database().read(), tenant, id).await?;
        writable(principal, tenant.database().write(), &SHIPMENTS_MANAGE, row.org_id, || {
            errors::shipment_not_found(id)
        })
        .await?;
        if row.tracking_number.is_some() {
            return Err(errors::already_booked());
        }
        match self.book(tenant, Actor::user(principal.user_id()), id).await? {
            Booked::Failed(code) => return Err(errors::booking_rejected(&code)),
            Booked::Skipped => {
                return Err(errors::invalid_transition(status_of(&row), ShipmentStatus::Booking));
            }
            Booked::Done => {}
        }
        self.get_shipment(tenant, principal, id).await
    }

    /// Vận đơn kẹt `booking` quá 10 phút: hỏi hãng theo `client_reference`. Có → ghi; không →
    /// về `pending` (`shipping.booking_lost`) để nhân viên đặt lại.
    pub(crate) async fn reconcile_booking(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<(), Error> {
        let row = find_shipment(tenant.database().read(), tenant, id).await?;
        if status_of(&row) != ShipmentStatus::Booking
            || row.status_at > now() - SignedDuration::from_mins(10)
        {
            return Ok(());
        }
        let Some(account_id) = row.carrier_account_id else { return Ok(()) };
        let (_, account, carrier) = self.account(tenant, account_id).await?;
        let found = within(
            self.config.carrier_timeout_seconds,
            carrier.find_by_reference(&account, &id.to_string()),
        )
        .await?;
        match found {
            Some(created) => {
                self.record_booking(tenant, Actor::system(), id, carrier.code(), &created).await
            }
            None => self.booking_failed(tenant, Actor::system(), id, "shipping.booking_lost").await,
        }
    }

    /// Huỷ vận đơn. Đã có mã vận đơn → gọi hãng huỷ (ngoài transaction); hãng đã lấy hàng →
    /// `shipping.cancel_too_late`.
    pub(crate) async fn cancel(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
    ) -> Result<bool, Error> {
        let row = find_shipment(tenant.database().read(), tenant, id).await?;
        let current = status_of(&row);
        if current == ShipmentStatus::Cancelled {
            return Ok(false);
        }
        if status::picked_up(current) || !status::allowed(current, ShipmentStatus::Cancelled) {
            return Err(crate::api::errors::cancel_too_late());
        }
        if let (Some(tracking), Some(account_id)) = (&row.tracking_number, row.carrier_account_id) {
            let (_, account, carrier) = self.account(tenant, account_id).await?;
            within(self.config.carrier_timeout_seconds, carrier.cancel(&account, tracking)).await?;
        }
        let update = StatusUpdate {
            event_key: "cancelled".to_owned(),
            status: ShipmentStatus::Cancelled,
            carrier_status: String::new(),
            occurred_at: now(),
            note: String::new(),
            source: Source::System,
        };
        self.apply_update(tenant, actor, id, update).await
    }

    pub(crate) async fn cancel_by_staff(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<crate::shipments::ShipmentView, Error> {
        let row = find_shipment(tenant.database().read(), tenant, id).await?;
        writable(principal, tenant.database().write(), &SHIPMENTS_MANAGE, row.org_id, || {
            errors::shipment_not_found(id)
        })
        .await?;
        self.cancel(tenant, Actor::user(principal.user_id()), id).await?;
        self.get_shipment(tenant, principal, id).await
    }

    /// File nhãn từ hãng (gọi hãng mỗi lần; không lưu đệm).
    pub(crate) async fn label(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Label, Error> {
        let row = find_shipment(tenant.database().read(), tenant, id).await?;
        readable(principal, tenant.database().read(), &SHIPMENTS_READ, row.org_id, || {
            errors::shipment_not_found(id)
        })
        .await?;
        let (Some(tracking), Some(account_id)) = (&row.tracking_number, row.carrier_account_id)
        else {
            return Err(errors::not_booked());
        };
        let (_, account, carrier) = self.account(tenant, account_id).await?;
        within(self.config.carrier_timeout_seconds, carrier.label(&account, tracking)).await
    }

    /// Hỏi lại toàn bộ hành trình từ hãng và áp (bước đã có bị bỏ qua theo `event_key`).
    pub(crate) async fn refresh_tracking(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<(), Error> {
        let row = find_shipment(tenant.database().read(), tenant, id).await?;
        let (Some(tracking), Some(account_id)) = (&row.tracking_number, row.carrier_account_id)
        else {
            return Ok(());
        };
        let (_, account, carrier) = self.account(tenant, account_id).await?;
        let updates =
            within(self.config.carrier_timeout_seconds, carrier.track(&account, tracking)).await?;
        for update in updates {
            self.apply_update(
                tenant,
                Actor::system(),
                id,
                StatusUpdate {
                    event_key: update.event_id,
                    status: update.status,
                    carrier_status: update.carrier_status,
                    occurred_at: update.occurred_at,
                    note: update.note,
                    source: Source::Carrier,
                },
            )
            .await?;
        }
        Ok(())
    }

    /// Hãng của vận đơn không gửi webhook: lịch `shipping.poll_tracking` hỏi định kỳ.
    pub(crate) async fn polls(&self, tenant: &TenantContext, id: Uuid) -> Result<bool, Error> {
        let row = find_shipment(tenant.database().read(), tenant, id).await?;
        let Some(code) = row.carrier_code else { return Ok(false) };
        Ok(self.carriers.get(&code).is_some_and(|carrier| !carrier.capabilities().webhooks))
    }
}
