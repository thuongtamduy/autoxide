//! Webhook trạng thái của hãng (bước 44 mục 3): kiểm chữ ký trên thân thô → ghi nguyên văn vào
//! `shipping_webhook_inbox` (chỉ trả 2xx sau khi đã ghi) → job `shipping.process_webhook` áp từng
//! cập nhật (chống trùng theo `event_key`, trạng thái theo bảng chuyển). Chữ ký sai → 401, không
//! lưu thân. Hai đường vào: `/webhooks/{carrier_code}/{hook}/{token}` tìm khách từ `{hook}` (định danh
//! của nền `autoxide_kernel::provider`, hãng không cần gửi header hay gọi đúng tên miền của khách)
//! rồi tìm tài khoản từ `{token}`; đường cũ `/webhooks/{carrier_code}/{token}` lấy khách của request
//! (header / tên miền). Token của khách A gửi vào khách B không khớp tài khoản nào → 404.

use std::collections::BTreeMap;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::EnqueueOptions;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use axum::http::HeaderMap;
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, Condition, EntityTrait, QueryFilter, Set};
use uuid::Uuid;

use crate::entity::{parcel, shipment, webhook_inbox};
use crate::errors;
use crate::jobs::ProcessWebhook;
use crate::service::{ShippingService, now};
use crate::shipments::{Source, StatusUpdate};

/// Header không lưu (bí mật của phiên, không phải của hãng).
const DROPPED: &[&str] = &["authorization", "cookie", "proxy-authorization", "x-api-key"];

fn headers_json(headers: &HeaderMap) -> String {
    let mut out = BTreeMap::new();
    for (name, value) in headers {
        let name = name.as_str().to_ascii_lowercase();
        if DROPPED.contains(&name.as_str()) {
            continue;
        }
        if let Ok(value) = value.to_str() {
            out.insert(name, value.to_owned());
        }
    }
    serde_json::to_string(&out).unwrap_or_else(|_| "{}".to_owned())
}

fn headers_of(raw: &str) -> HeaderMap {
    let values: BTreeMap<String, String> = serde_json::from_str(raw).unwrap_or_default();
    let mut headers = HeaderMap::new();
    for (name, value) in values {
        if let (Ok(name), Ok(value)) = (
            axum::http::HeaderName::from_bytes(name.as_bytes()),
            axum::http::HeaderValue::from_str(&value),
        ) {
            headers.insert(name, value);
        }
    }
    headers
}

impl ShippingService {
    /// Nhận một webhook. `Ok(id)` khi đã ghi nguyên văn và xếp job.
    pub(crate) async fn receive_webhook(
        &self,
        tenant: &TenantContext,
        carrier_code: &str,
        token: &str,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<Uuid, Error> {
        if body.len() > self.config.webhook_max_bytes {
            return Err(errors::webhook_too_large(self.config.webhook_max_bytes));
        }
        let row = self
            .account_by_token(tenant.database().read(), tenant, carrier_code, token)
            .await?
            .ok_or_else(errors::webhook_unknown)?;
        let (_, account, carrier) = self.account(tenant, row.id).await?;
        // Kiểm chữ ký trước khi lưu: sai → 401, không lưu thân.
        carrier.parse_webhook(&account, headers, body)?;
        let text = String::from_utf8(body.to_vec()).map_err(|_| errors::webhook_body())?;
        let id = Uuid::now_v7();
        let transaction = tenant.database().begin_write().await?;
        webhook_inbox::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            carrier_account_id: Set(row.id),
            received_at: Set(now()),
            headers: Set(headers_json(headers)),
            body: Set(text),
            status: Set("received".to_owned()),
            error: Set(None),
        }
        .insert(&transaction)
        .await?;
        self.jobs
            .enqueue(
                &transaction,
                tenant,
                &ProcessWebhook { inbox_id: id },
                EnqueueOptions::default(),
            )
            .await?;
        transaction.commit().await?;
        Ok(id)
    }

    /// Nhận webhook mà khách nằm trong đường dẫn: `hook` → khách (sai → `provider.webhook_unknown`,
    /// 404), rồi như [`receive_webhook`](Self::receive_webhook) với khách đó.
    pub(crate) async fn receive_webhook_by_hook(
        &self,
        carrier_code: &str,
        hook: &str,
        token: &str,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<Uuid, Error> {
        let tenant = self
            .providers
            .resolve_webhook(&crate::carriers::webhook_base(carrier_code), hook)
            .await?;
        self.receive_webhook(&tenant, carrier_code, token, headers, body).await
    }

    /// Job: áp các cập nhật của một webhook đã lưu. Chạy lại không sao (chống trùng `event_key`).
    pub(crate) async fn process_webhook(
        &self,
        tenant: &TenantContext,
        inbox_id: Uuid,
    ) -> Result<(), Error> {
        let Some(row) = webhook_inbox::Entity::find_by_id(inbox_id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
        else {
            return Ok(());
        };
        if row.status == "processed" {
            return Ok(());
        }
        let outcome = self.apply_webhook(tenant, &row).await;
        let (status, error) = match &outcome {
            Ok(()) => ("processed", None),
            Err(error) => ("failed", Some(error.code().chars().take(500).collect::<String>())),
        };
        webhook_inbox::Entity::update_many()
            .col_expr(webhook_inbox::Column::Status, Expr::value(status))
            .col_expr(webhook_inbox::Column::Error, Expr::value(error))
            .filter(webhook_inbox::Column::Id.eq(inbox_id))
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        outcome
    }

    async fn apply_webhook(
        &self,
        tenant: &TenantContext,
        row: &webhook_inbox::Model,
    ) -> Result<(), Error> {
        let (account_row, account, carrier) = self.account(tenant, row.carrier_account_id).await?;
        let updates =
            carrier.parse_webhook(&account, &headers_of(&row.headers), row.body.as_bytes())?;
        for update in updates {
            let Some(shipment_id) = self
                .shipment_by_tracking(tenant, &account_row.carrier_code, &update.tracking_number)
                .await?
            else {
                tracing::warn!(carrier = %account_row.carrier_code, "webhook update for an unknown tracking number");
                continue;
            };
            self.apply_update(
                tenant,
                Actor::system(),
                shipment_id,
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

    /// Vận đơn có mã `tracking` (của vận đơn hay của một kiện) ở hãng `carrier_code`.
    pub(crate) async fn shipment_by_tracking(
        &self,
        tenant: &TenantContext,
        carrier_code: &str,
        tracking: &str,
    ) -> Result<Option<Uuid>, Error> {
        let read = tenant.database().read();
        let direct = shipment::Entity::find()
            .filter(
                Condition::all()
                    .add(shipment::Column::CarrierCode.eq(carrier_code))
                    .add(shipment::Column::TrackingNumber.eq(tracking)),
            )
            .scoped(tenant)
            .one(read)
            .await?;
        if let Some(row) = direct {
            return Ok(Some(row.id));
        }
        let by_parcel = parcel::Entity::find()
            .filter(parcel::Column::TrackingNumber.eq(tracking))
            .scoped(tenant)
            .one(read)
            .await?;
        Ok(by_parcel.map(|row| row.shipment_id))
    }
}
