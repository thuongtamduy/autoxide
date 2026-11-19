//! Đặt hàng (mục 3 của bước 45): một chuỗi giữ chỗ có hạn theo `order_id` — đặc tả 45 "Luồng đặt hàng".
//!
//! - Bước 1: `(chủ, Idempotency-Key)` đã có → trả kết quả cũ (khác nội dung → 422).
//! - Bước 2: báo giá lại ở máy chủ; lệch `quote_token` → `orders.quote_changed`.
//! - Bước 3: ghi `orders_checkouts` (`reserving`) — transaction ngắn.
//! - Bước 4–5: giữ tồn (`inventory`) và lượt khuyến mại (`promotions`) NGOÀI transaction; hỏng thì
//!   trả mọi chỗ đã giữ, lượt đặt `failed`.
//! - Bước 7: MỘT transaction — số đơn (`numbering`) → đơn + dòng + thuế + khoản thanh toán + lịch sử
//!   + event, lượt đặt `placed`, giỏ `converted`. Không gọi mạng bên trong (giữ khoá dãy số ngắn).
//! - Bước 8: ngoài transaction, tạo lượt thu ở `ledger` (cổng / VietQR); COD xác nhận ngay ở bước 7.
//!
//! Lượt đặt treo ở `reserving` (máy chết giữa 4 và 7) được job `orders.recover_checkouts` trả chỗ.

use autoxide_inventory::api::{ReleaseReason, ReserveLine, ReserveRequest};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::numbering;
use autoxide_kernel::reference::{self, Submitted};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use autoxide_promotions::api::ReserveRequest as PromotionHold;
use chrono::{Datelike, Duration, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::api::{FundsFlow, OrderStatus};
use crate::carts::{self, CONVERTED, Owner, ShipTo};
use crate::entity::{cart, cart_line, checkout, line, line_tax, order, payment};
use crate::errors;
use crate::lifecycle::{self, Cause, Transition};
use crate::payments::{self, Method};
use crate::pricing::{self, Quote, QuotedLine};
use crate::service::OrdersService;

pub(crate) const RESERVING: &str = "reserving";
pub(crate) const PLACED: &str = "placed";
pub(crate) const FAILED: &str = "failed";

/// Người nhận / liên hệ của đơn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Contact {
    pub name: String,
    pub phone: String,
    pub email: Option<String>,
}

/// Đầu vào của `POST /checkout`.
#[derive(Debug, Clone)]
pub(crate) struct CheckoutInput {
    pub cart_id: Uuid,
    pub quote_token: String,
    pub payment_method: String,
    pub contact: Contact,
    pub note: Option<String>,
    pub accept_terms_version: Option<String>,
    pub invoice_request: Option<serde_json::Value>,
    pub return_url: Option<String>,
}

/// Kết quả đặt hàng: các đơn đã tạo (một người bán: một đơn).
#[derive(Debug, Clone)]
pub(crate) struct Placed {
    pub order_ids: Vec<Uuid>,
}

/// Chỗ đã giữ của một lượt đặt.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct Holds {
    pub inventory: bool,
    pub promotions: bool,
}

fn owner_key(owner: &Owner, cart_id: Uuid) -> String {
    match owner {
        Owner::Buyer(user) => format!("user:{user}"),
        Owner::Guest(_) => format!("cart:{cart_id}"),
    }
}

fn check_contact(contact: &Contact, problems: &mut Vec<FieldError>) {
    if !(1..=200).contains(&contact.name.trim().chars().count()) {
        problems.push(FieldError::new(
            "contact.name",
            "length",
            "must be 1 to 200 characters long",
        ));
    }
    let digits = contact.phone.trim().trim_start_matches('+');
    if !(8..=15).contains(&digits.len()) || !digits.bytes().all(|b| b.is_ascii_digit()) {
        problems.push(FieldError::new(
            "contact.phone",
            "format",
            "must be a phone number: 8 to 15 digits, optionally starting with `+`",
        ));
    }
    if contact.email.as_deref().is_some_and(|email| !email.contains('@') || email.len() > 320) {
        problems.push(FieldError::new("contact.email", "format", "must be an email address"));
    }
}

impl OrdersService {
    /// Đặt hàng từ giỏ `input.cart_id` của `owner`, chống trùng theo `reference` (`Idempotency-Key`).
    pub(crate) async fn checkout(
        &self,
        tenant: &TenantContext,
        owner: &Owner,
        reference: &str,
        input: CheckoutInput,
    ) -> Result<Submitted<Placed>, Error> {
        let reference = reference.trim();
        if reference.is_empty() || reference.len() > 200 {
            return Err(errors::idempotency_key_required());
        }
        let key = owner_key(owner, input.cart_id);
        let fingerprint = reference::fingerprint(&json!({
            "cart_id": input.cart_id, "quote": pricing::token_fingerprint(&input.quote_token),
            "payment_method": input.payment_method, "contact": input.contact, "note": input.note,
        }))?;
        if let Some(existing) = self.find_checkout(tenant, &key, reference).await? {
            return self.replay(existing, &fingerprint, reference);
        }

        // 2. Báo giá lại ở máy chủ, kiểm đầu vào (gom lỗi một lần — D-083).
        let row = self.owned_cart(tenant, input.cart_id, owner).await?;
        if row.status == carts::MERGED || row.status == CONVERTED {
            // Cùng khoá gửi song song: bên kia vừa đặt xong giỏ này — trả kết quả của nó.
            if let Some(existing) = self.find_checkout(tenant, &key, reference).await? {
                return self.replay(existing, &fingerprint, reference);
            }
            return Err(errors::cart_closed());
        }
        let lines = cart_line::Entity::find()
            .filter(cart_line::Column::CartId.eq(row.id))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        if lines.is_empty() {
            return Err(errors::cart_empty());
        }
        let method = Method::parse(&input.payment_method);
        let mut price_input = self.price_input(tenant, &row, owner, &lines).await?;
        price_input.payment_method = Some(input.payment_method.clone());
        let quote =
            pricing::quote(&self.sources, tenant, &price_input, self.config.quote_ttl_minutes)
                .await?;
        let Some((expires, expected)) = pricing::parse_token(&input.quote_token) else {
            return Err(errors::quote_changed());
        };
        if expires < Utc::now() {
            return Err(errors::quote_expired());
        }
        // Dòng hết hàng / ngừng bán báo trước (rõ hơn "giá đổi").
        if let Some(error) = quote.lines.iter().find_map(pricing::line_error) {
            return Err(error);
        }
        if quote.fingerprint() != expected {
            return Err(errors::quote_changed());
        }
        let mut problems = Vec::new();
        check_contact(&input.contact, &mut problems);
        if input.note.as_deref().is_some_and(|note| note.chars().count() > 1000) {
            problems.push(FieldError::new(
                "note",
                "length",
                "must be at most 1000 characters long",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let ship_to: Option<ShipTo> =
            row.ship_to.as_deref().and_then(|raw| serde_json::from_str(raw).ok());
        if quote.requires_shipping
            && (quote.shipping.is_none() || price_input.destination.is_none())
        {
            return Err(errors::delivery_required());
        }
        if method == Method::Cod && !quote.requires_shipping {
            return Err(errors::payment_method_unavailable(&input.payment_method));
        }
        if method != Method::Cod && self.ledger.is_none() {
            return Err(errors::payment_method_unavailable(&input.payment_method));
        }

        // 3. Lượt đặt `reserving`.
        let order_id = Uuid::now_v7();
        let now = Utc::now();
        let started = checkout::ActiveModel {
            id: Set(order_id),
            tenant_id: Set(tenant.id().as_uuid()),
            owner_key: Set(key.clone()),
            reference: Set(reference.to_owned()),
            fingerprint: Set(fingerprint.clone()),
            cart_id: Set(row.id),
            state: Set(RESERVING.to_owned()),
            failure_code: Set(None),
            holds: Set("{}".to_owned()),
            created_at: Set(now),
            updated_at: Set(now),
        };
        if let Err(error) = started.insert(tenant.database().write()).await {
            // Cùng khoá gửi song song: bên kia đã ghi trước.
            return match self.find_checkout(tenant, &key, reference).await? {
                Some(existing) => self.replay(existing, &fingerprint, reference),
                None => Err(error.into()),
            };
        }

        // 4–5. Giữ chỗ ngoài transaction.
        let org = OrgId::from_uuid(row.org_id);
        let hold_until = now + Duration::minutes(self.config.hold_minutes);
        let mut holds = Holds::default();
        let held = self
            .hold_all(tenant, order_id, org, &quote, &price_input, hold_until, &mut holds)
            .await;
        if let Err(error) = held {
            self.release_holds(tenant, order_id, &holds, ReleaseReason::CheckoutAborted).await;
            self.fail_checkout(tenant, order_id, error.code()).await?;
            return Err(error);
        }

        // 7. Tạo đơn trong một transaction.
        let funds_flow = match &self.ledger {
            Some(ledger) => payments::funds_flow(ledger.flow_settings(tenant, org).await?.mode),
            None => FundsFlow::SelfCollect,
        };
        let draft = Draft {
            order_id,
            org,
            buyer: match owner {
                Owner::Buyer(user) => Some(*user),
                Owner::Guest(_) => None,
            },
            cart: row.clone(),
            ship_to,
            quote: &quote,
            input: &input,
            method,
            funds_flow,
            holds: &holds,
            due: now + Duration::minutes(self.config.hold_minutes - self.config.hold_grace_minutes),
        };
        if let Err(error) = self.create_order(tenant, &draft).await {
            self.release_holds(tenant, order_id, &holds, ReleaseReason::CheckoutAborted).await;
            self.fail_checkout(tenant, order_id, error.code()).await?;
            return Err(error);
        }

        // 8. Lượt thu ở đối tác (ngoài transaction). Đối tác lỗi: đơn vẫn chờ thanh toán, người mua
        // mở lại bằng `POST /me/orders/{id}/payments`; hết hạn thì job huỷ.
        if method != Method::Cod
            && let Err(error) = self.start_payment(tenant, order_id, input.return_url.clone()).await
        {
            tracing::warn!(order_id = %order_id, code = error.code(), "payment could not be started at checkout");
        }
        Ok(Submitted::Created(Placed { order_ids: vec![order_id] }))
    }

    async fn find_checkout(
        &self,
        tenant: &TenantContext,
        key: &str,
        reference: &str,
    ) -> Result<Option<checkout::Model>, Error> {
        Ok(checkout::Entity::find()
            .filter(checkout::Column::OwnerKey.eq(key))
            .filter(checkout::Column::Reference.eq(reference))
            .scoped(tenant)
            .one(tenant.database().write())
            .await?)
    }

    fn replay(
        &self,
        existing: checkout::Model,
        fingerprint: &str,
        reference: &str,
    ) -> Result<Submitted<Placed>, Error> {
        if existing.fingerprint != fingerprint {
            return Err(errors::checkout_key_reused(reference));
        }
        match existing.state.as_str() {
            PLACED => Ok(Submitted::Replayed(Placed { order_ids: vec![existing.id] })),
            FAILED => Err(errors::checkout_failed(existing.failure_code.as_deref().unwrap_or(""))),
            _ => Err(errors::checkout_in_progress()),
        }
    }

    /// Giữ tồn rồi lượt khuyến mại; ghi lại chỗ đã giữ vào `holds` để trả khi hỏng.
    #[expect(clippy::too_many_arguments, reason = "một lượt giữ cần chừng đó dữ liệu")]
    async fn hold_all(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
        org: OrgId,
        quote: &Quote,
        input: &pricing::PriceInput,
        until: chrono::DateTime<Utc>,
        holds: &mut Holds,
    ) -> Result<(), Error> {
        let tracked: Vec<ReserveLine> = quote
            .lines
            .iter()
            .filter(|line| line.variant.as_ref().is_some_and(|variant| variant.track_inventory))
            .map(|line| ReserveLine::new(line.key.clone(), line.variant_id, line.quantity))
            .collect();
        if let Some(inventory) = &self.sources.inventory
            && !tracked.is_empty()
        {
            let mut request = ReserveRequest::new(order_id, org, until, tracked);
            if let Some(destination) = &input.destination {
                request = request.ship_to(destination.province_code.clone());
            }
            holds.inventory = true;
            inventory.reserve(tenant, &request).await.map_err(|error| stock_error(error, quote))?;
        }
        if let (Some(promotions), Some(evaluation)) = (&self.sources.promotions, &quote.evaluation)
            && (!evaluation.applied.is_empty() || !input.codes.is_empty())
        {
            let cart = pricing::promotion_cart_of(input, quote);
            let request =
                PromotionHold::new(order_id, cart, until).expecting(&evaluation.fingerprint);
            holds.promotions = true;
            promotions.reserve(tenant, &request).await.map_err(promotion_error)?;
        }
        Ok(())
    }

    /// Trả mọi chỗ đã giữ của lượt đặt hỏng (gọi trực tiếp; lỗi chỉ ghi log — job hết hạn của bên
    /// giữ chỗ là lưới an toàn cuối).
    pub(crate) async fn release_holds(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
        holds: &Holds,
        reason: ReleaseReason,
    ) {
        if holds.inventory
            && let Some(inventory) = &self.sources.inventory
            && let Err(error) = inventory.release(tenant, order_id, reason).await
        {
            tracing::warn!(order_id = %order_id, code = error.code(), "stock hold not released");
        }
        if holds.promotions
            && let Some(promotions) = &self.sources.promotions
            && let Err(error) = promotions.release(tenant, order_id).await
        {
            tracing::warn!(order_id = %order_id, code = error.code(), "promotion hold not released");
        }
    }

    pub(crate) async fn fail_checkout(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        code: &str,
    ) -> Result<(), Error> {
        checkout::Entity::update_many()
            .col_expr(checkout::Column::State, Expr::value(FAILED))
            .col_expr(checkout::Column::FailureCode, Expr::value(code.to_owned()))
            .col_expr(checkout::Column::UpdatedAt, Expr::value(Utc::now()))
            .filter(checkout::Column::Id.eq(id))
            .filter(checkout::Column::State.eq(RESERVING))
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        Ok(())
    }

    /// Bước 7: một transaction ghi đơn, dòng, thuế, khoản thanh toán, lịch sử, event.
    async fn create_order(&self, tenant: &TenantContext, draft: &Draft<'_>) -> Result<(), Error> {
        let database = tenant.database();
        let outbox = &self.outbox;
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let now = Utc::now();
                let year = now.year();
                let sequence =
                    numbering::next(&transaction, tenant.id(), &format!("orders.order:{year}"))
                        .await?;
                let number = format!("{year}-{sequence:06}");
                for (row, taxes) in draft.lines(tenant)? {
                    row.insert(&transaction).await?;
                    for tax in taxes {
                        tax.insert(&transaction).await?;
                    }
                }
                let (status, payment_status) = if draft.method == Method::Cod {
                    (OrderStatus::Confirmed, payments::COD_PENDING)
                } else {
                    (OrderStatus::PendingPayment, payments::UNPAID)
                };
                let transition = Transition::new(
                    status,
                    if draft.buyer.is_some() { Cause::Buyer } else { Cause::System },
                    draft.buyer.map_or_else(Actor::public, Actor::user),
                );
                let created = lifecycle::create(
                    &transaction,
                    tenant,
                    outbox,
                    draft.order_row(tenant, number, payment_status)?,
                    &transition,
                )
                .await?;
                draft.payment_row(tenant, &created).insert(&transaction).await?;
                checkout::Entity::update_many()
                    .col_expr(checkout::Column::State, Expr::value(PLACED))
                    .col_expr(
                        checkout::Column::Holds,
                        Expr::value(serde_json::to_string(draft.holds).map_err(Error::internal)?),
                    )
                    .col_expr(checkout::Column::UpdatedAt, Expr::value(now))
                    .filter(checkout::Column::Id.eq(draft.order_id))
                    .scoped(tenant)
                    .exec(&transaction)
                    .await?;
                cart::Entity::update_many()
                    .col_expr(cart::Column::Status, Expr::value(CONVERTED))
                    .col_expr(cart::Column::ActiveBuyerKey, Expr::value(Option::<String>::None))
                    .col_expr(cart::Column::UpdatedAt, Expr::value(now))
                    .filter(cart::Column::Id.eq(draft.cart.id))
                    .scoped(tenant)
                    .exec(&transaction)
                    .await?;
                transaction.commit().await?;
                Ok(())
            })
            .await
    }
}

/// Dữ liệu của đơn sẽ tạo (bước 7).
struct Draft<'a> {
    order_id: Uuid,
    org: OrgId,
    buyer: Option<Uuid>,
    cart: cart::Model,
    ship_to: Option<ShipTo>,
    quote: &'a Quote,
    input: &'a CheckoutInput,
    method: Method,
    funds_flow: FundsFlow,
    holds: &'a Holds,
    /// Hạn thanh toán tạm, tới khi `ledger` trả hạn thật (D-218).
    due: chrono::DateTime<Utc>,
}

impl Draft<'_> {
    fn order_row(
        &self,
        tenant: &TenantContext,
        number: String,
        payment_status: &str,
    ) -> Result<order::ActiveModel, Error> {
        let quote = self.quote;
        let now = Utc::now();
        let json = |value: &dyn erased::Json| value.to_json();
        let mut ship_to = self.ship_to.clone();
        if let Some(ship_to) = &mut ship_to
            && ship_to.address.is_none()
        {
            ship_to.address = None;
        }
        Ok(order::ActiveModel {
            id: Set(self.order_id),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(self.org.as_uuid()),
            checkout_id: Set(Some(self.order_id)),
            number: Set(number),
            channel: Set("web".to_owned()),
            external_ref: Set(None),
            buyer_user_id: Set(self.buyer),
            buyer_key: Set(None),
            contact: Set(json(&self.input.contact)?),
            ship_to: Set(self.ship_to.as_ref().map(|ship_to| json(ship_to)).transpose()?),
            shipping_option: Set(quote.shipping.as_ref().map(|option| {
                serde_json::json!({"method_id": option.method_id, "option_key": option.option_key,
                    "method_code": option.method_code, "name": option.name,
                    "carrier_code": option.carrier_code, "amount": option.amount,
                    "pickup_org_id": option.pickup_org_id, "eta": option.eta})
                .to_string()
            })),
            pickup_org_id: Set(quote
                .shipping
                .as_ref()
                .and_then(|option| option.pickup_org_id)
                .map(|org| org.as_uuid())),
            status: sea_orm::ActiveValue::NotSet,
            payment_status: Set(payment_status.to_owned()),
            fulfillment_status: Set("unfulfilled".to_owned()),
            payment_method: Set(self.input.payment_method.clone()),
            funds_flow: Set(self.funds_flow.as_str().to_owned()),
            currency: Set(self.cart.currency.clone()),
            subtotal_minor: Set(quote.subtotal.minor()),
            discount_minor: Set(quote.discount.minor()),
            shipping_fee_minor: Set(quote.shipping_fee.minor()),
            shipping_discount_minor: Set(quote.shipping_discount.minor()),
            tax_minor: Set(quote.tax.minor()),
            total_minor: Set(quote.total.minor()),
            paid_minor: Set(0),
            refunded_minor: Set(0),
            payment_due_at: Set((self.method != Method::Cod).then_some(self.due)),
            attribution: Set(self.cart.attribution.clone()),
            promotions: Set(quote
                .evaluation
                .as_ref()
                .map(|evaluation| json(evaluation))
                .transpose()?),
            stock_shortage: Set(false),
            invoice_request: Set(self.input.invoice_request.as_ref().map(ToString::to_string)),
            terms_version: Set(self.input.accept_terms_version.clone()),
            note: Set(self.input.note.clone()),
            locale: Set(self.cart.locale.clone()),
            placed_at: Set(now),
            confirmed_at: Set(None),
            delivered_at: Set(None),
            completed_at: Set(None),
            cancelled_at: Set(None),
            cancel_reason: Set(None),
            version: Set(1),
            created_at: Set(now),
            updated_at: Set(now),
        })
    }

    /// Dòng đơn và thuế của dòng.
    fn lines(
        &self,
        tenant: &TenantContext,
    ) -> Result<Vec<(line::ActiveModel, Vec<line_tax::ActiveModel>)>, Error> {
        let evaluation = self.quote.evaluation.as_ref();
        self.quote
            .lines
            .iter()
            .map(|quoted| {
                let id = Uuid::now_v7();
                let discounts: Vec<_> = evaluation
                    .map(|evaluation| {
                        evaluation
                            .lines
                            .iter()
                            .filter(|d| d.line_key == quoted.key)
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                let row = line_row(tenant, self.order_id, id, quoted, &discounts)?;
                let taxes = quoted
                    .taxes
                    .iter()
                    .map(|tax| line_tax::ActiveModel {
                        tenant_id: Set(tenant.id().as_uuid()),
                        line_id: Set(id),
                        tax_code: Set(tax.tax_kind.clone()),
                        rate: Set(tax.rate_percent.clone()),
                        exempt: Set(tax.exempt),
                        base_minor: Set(quoted.net.minor()),
                        amount_minor: Set(tax.amount.minor()),
                    })
                    .collect();
                Ok((row, taxes))
            })
            .collect()
    }

    fn payment_row(&self, tenant: &TenantContext, order: &order::Model) -> payment::ActiveModel {
        let now = Utc::now();
        payment::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            order_id: Set(order.id),
            purpose: Set(payments::PURPOSE_ORDER.to_owned()),
            method: Set(self.input.payment_method.clone()),
            gateway: Set(None),
            ledger_payment_id: Set(None),
            external_id: Set(None),
            reference: Set(payments::reference(order.id, payments::PURPOSE_ORDER, 1)),
            amount_minor: Set(order.total_minor),
            currency: Set(order.currency.clone()),
            tendered_minor: Set(None),
            status: Set(payments::PENDING.to_owned()),
            action: Set(None),
            expires_at: Set(order.payment_due_at),
            succeeded_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
    }
}

fn line_row(
    tenant: &TenantContext,
    order_id: Uuid,
    id: Uuid,
    quoted: &QuotedLine,
    discounts: &[autoxide_promotions::api::LineDiscount],
) -> Result<line::ActiveModel, Error> {
    let variant =
        quoted.variant.as_ref().ok_or_else(|| Error::internal("quoted line without variant"))?;
    let snapshot = json!({
        "variant_label": variant.variant_label, "image_media_id": variant.image_media_id,
        "warranty_months": variant.warranty_months, "serial_tracked": variant.serial_tracked,
        "weight_grams": variant.weight_grams, "barcode": variant.barcode,
        "category_ids": variant.category_ids, "tax_category": variant.tax_category,
    });
    let kind = serde_json::to_value(variant.kind).map_err(Error::internal)?;
    let sale_mode = serde_json::to_value(variant.sale_mode).map_err(Error::internal)?;
    Ok(line::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant.id().as_uuid()),
        order_id: Set(order_id),
        line_key: Set(quoted.key.clone()),
        parent_line_id: Set(None),
        variant_id: Set(quoted.variant_id),
        product_id: Set(variant.product_id),
        category_id: Set(variant.category_ids.first().copied()),
        seller_org_id: Set(variant.org_id.as_uuid()),
        kind: Set(kind.as_str().unwrap_or("physical").to_owned()),
        sku: Set(variant.sku.clone()),
        name: Set(variant.name.clone()),
        snapshot: Set(snapshot.to_string()),
        sale_mode: Set(sale_mode.as_str().unwrap_or("in_stock").to_owned()),
        preorder_available_on: Set(variant.preorder_available_on),
        requires_shipping: Set(variant.requires_shipping),
        tax_inclusive: Set(quoted.tax_inclusive),
        quantity: Set(quoted.quantity),
        unit_price_minor: Set(quoted.unit_price.minor()),
        subtotal_minor: Set(quoted.subtotal.minor()),
        discount_minor: Set(quoted.discount.minor()),
        net_minor: Set(quoted.net.minor()),
        tax_minor: Set(quoted.tax.minor()),
        total_minor: Set(quoted.gross.minor()),
        compare_at_minor: Set(quoted.compare_at.map(|price| price.minor())),
        discounts: Set(serde_json::to_string(discounts).map_err(Error::internal)?),
        gift_of: Set(quoted.gift_of),
        warehouse_org_id: Set(None),
        returned_quantity: Set(0),
        created_at: Set(Utc::now()),
    })
}

/// `inventory.insufficient_stock` → `orders.out_of_stock` với SKU của dòng.
fn stock_error(error: Error, quote: &Quote) -> Error {
    if error.code() != "inventory.insufficient_stock" {
        return error;
    }
    let variant = error
        .params()
        .find(|(name, _)| *name == "variant_id")
        .and_then(|(_, value)| value.parse::<Uuid>().ok());
    let available = error
        .params()
        .find(|(name, _)| *name == "available")
        .and_then(|(_, value)| value.parse::<i64>().ok())
        .unwrap_or(0);
    let sku = quote
        .lines
        .iter()
        .find(|line| Some(line.variant_id) == variant)
        .and_then(|line| line.variant.as_ref().map(|v| v.sku.clone()))
        .unwrap_or_default();
    errors::out_of_stock(&sku, available)
}

/// Lỗi giữ lượt khuyến mại → lỗi của `orders`.
fn promotion_error(error: Error) -> Error {
    match (error.code(), error.kind()) {
        ("promotions.changed", _) => errors::quote_changed(),
        (_, ErrorKind::Conflict) => errors::promotion_unavailable(),
        _ => error,
    }
}

/// `Serialize` → chuỗi JSON cho cột `text`.
mod erased {
    use autoxide_kernel::error::Error;

    pub(super) trait Json {
        fn to_json(&self) -> Result<String, Error>;
    }

    impl<T: serde::Serialize> Json for T {
        fn to_json(&self) -> Result<String, Error> {
            serde_json::to_string(self).map_err(Error::internal)
        }
    }
}
