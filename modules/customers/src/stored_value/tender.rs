//! `Tender` thẻ quà tặng / số dư cắm vào `orders`, và kiểm tra của `autoxide doctor`.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::money::Money;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_orders::api::Tender;
use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::service::{CustomersService, now};

use super::{CREDIT, GIFT_CARD, STORE_CREDIT, card_inactive, insufficient, need_account};

/// `Tender` loại `gift_card`: `input = {"code": "…"}`.
pub(crate) struct GiftCardTender(pub Arc<CustomersService>);

#[async_trait::async_trait]
impl Tender for GiftCardTender {
    fn kind(&self) -> &'static str {
        GIFT_CARD
    }

    async fn hold(
        &self,
        tenant: &TenantContext,
        reference: &str,
        buyer: Option<Uuid>,
        input: &Value,
        max: Money,
        expires_at: DateTime<Utc>,
    ) -> Result<Money, Error> {
        // Cần tài khoản: bộ đếm tra sai theo người; khách vãng lai dùng chung một bộ đếm thì người
        // đoán mã sẽ chặn mọi khách vãng lai khác.
        let who = buyer.ok_or_else(need_account)?;
        let code = input["code"].as_str().unwrap_or_default();
        let card = self.0.lookup_gift_card(tenant, who, code).await?;
        let expired = card.expires_at.is_some_and(|at| at <= now());
        if card.status != "active" || expired {
            return Err(card_inactive());
        }
        if card.currency != max.currency().code() {
            return Err(card_inactive());
        }
        let wanted = Money::new(Ord::min(card.balance_minor, max.minor()), max.currency());
        self.0.hold_value(tenant, GIFT_CARD, card.id, reference, wanted, expires_at).await
    }

    async fn release(&self, tenant: &TenantContext, reference: &str) -> Result<(), Error> {
        self.0.settle_value_holds(tenant, reference, false).await
    }
}

/// `Tender` loại `store_credit`: số dư của chính người mua; `input = {"amount": <tối đa>}` (tuỳ chọn).
pub(crate) struct StoreCreditTender(pub Arc<CustomersService>);

#[async_trait::async_trait]
impl Tender for StoreCreditTender {
    fn kind(&self) -> &'static str {
        CREDIT
    }

    async fn hold(
        &self,
        tenant: &TenantContext,
        reference: &str,
        buyer: Option<Uuid>,
        input: &Value,
        max: Money,
        expires_at: DateTime<Utc>,
    ) -> Result<Money, Error> {
        self.0.stored_value_allowed(tenant, &STORE_CREDIT).await?;
        let buyer = buyer.ok_or_else(need_account)?;
        let profile = self.0.me(tenant, buyer).await?;
        let balance = self.0.credit_balance(tenant.database().read(), tenant, profile.id).await?;
        let cap = input["amount"].as_i64().unwrap_or(i64::MAX);
        let wanted =
            Money::new(Ord::max(Ord::min(Ord::min(balance, max.minor()), cap), 0), max.currency());
        if !wanted.is_positive() {
            return Err(insufficient(balance));
        }
        self.0.hold_value(tenant, CREDIT, profile.id, reference, wanted, expires_at).await
    }

    async fn release(&self, tenant: &TenantContext, reference: &str) -> Result<(), Error> {
        self.0.settle_value_holds(tenant, reference, false).await
    }
}

/// `autoxide doctor`: cảnh báo khi thẻ quà tặng / số dư đã bật (cần đối chiếu NĐ 52/2024).
pub(crate) struct StoredValueCheck(pub bool);

#[async_trait::async_trait]
impl autoxide_kernel::doctor::Probe for StoredValueCheck {
    async fn probe(&self) -> autoxide_kernel::doctor::Finding {
        if self.0 {
            autoxide_kernel::doctor::Finding::warning(
                "gift cards and store credit are turned on (AUTOXIDE_CUSTOMERS_STORED_VALUE_ENABLED)",
            )
            .with_hint(
                "Keep them only after a qualified person checked Decree 52/2024/ND-CP on prepaid cards and e-wallets: \
                 usable only in the issuing shop, no cash out, no transfer.",
            )
        } else {
            autoxide_kernel::doctor::Finding::ok("gift cards and store credit are off")
        }
    }
}
