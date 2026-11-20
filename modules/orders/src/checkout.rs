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
    /// Người nhận hộ khi nhận tại cửa hàng (D-201): tên và số điện thoại, đối chiếu lúc giao.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_phone: Option<String>,
}

/// Số điện thoại hợp lệ: 8–15 chữ số, có thể bắt đầu bằng `+`.
pub(crate) fn phone_valid(phone: &str) -> bool {
    let digits = phone.trim().trim_start_matches('+');
    (8..=15).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit())
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
    /// Người mua đồng ý điều kiện đặt trước (bắt buộc khi giỏ có hàng đặt trước, D-204).
    pub accept_preorder_terms: bool,
    pub invoice_request: Option<serde_json::Value>,
    pub return_url: Option<String>,
    /// Ô người mua nhập (trường tự định nghĩa mức `buyer_input` của `orders.order`, D-235).
    pub custom_fields: Option<serde_json::Map<String, serde_json::Value>>,
    /// Trả góp qua đối tác: đối tác và số kỳ (chỉ với `payment_method = installment`).
    pub installment: Option<Installment>,
}

/// Trả góp qua đối tác có giấy phép (bước 36): `orders` không cho vay, không giữ tiền (D-187) —
/// đối tác trả đủ cho người bán, người mua trả góp cho đối tác.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Installment {
    /// Mã bộ nối đối tác trả góp (cổng thanh toán của `ledger`).
    pub partner: String,
    /// Số kỳ.
    pub months: i32,
}

/// Loại bản ghi của sổ đối tượng cho trường tự định nghĩa của đơn (bước 51 P19, D-235).
pub(crate) const ORDER_TARGET: &str = "orders.order";

/// Số kỳ trả góp nhận được.
pub(crate) const INSTALLMENT_MONTHS: [i32; 6] = [3, 6, 9, 12, 18, 24];

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
    if !phone_valid(&contact.phone) {
        problems.push(FieldError::new(
            "contact.phone",
            "format",
            "must be a phone number: 8 to 15 digits, optionally starting with `+`",
        ));
    }
    match (&contact.proxy_name, &contact.proxy_phone) {
        (None, None) => {}
        (Some(name), Some(phone)) => {
            if !(1..=200).contains(&name.trim().chars().count()) {
                problems.push(FieldError::new(
                    "contact.proxy_name",
                    "length",
                    "must be 1 to 200 characters long",
                ));
            }
            if !phone_valid(phone) {
                problems.push(FieldError::new(
                    "contact.proxy_phone",
                    "format",
                    "must be a phone number: 8 to 15 digits, optionally starting with `+`",
                ));
            }
        }
        _ => problems.push(FieldError::new(
            "contact.proxy_name",
            "required",
            "give both `proxy_name` and `proxy_phone` of the person collecting, or neither",
        )),
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
            "custom_fields": input.custom_fields, "installment": input.installment,
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
        if expires < crate::service::now() {
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
        if quote.has_preorder() && !input.accept_preorder_terms {
            problems.push(FieldError::new(
                "accept_preorder_terms",
                "required",
                "the cart has pre-order items: show their terms and expected dates, then send `true`",
            ));
        }
        if input.note.as_deref().is_some_and(|note| note.chars().count() > 1000) {
            problems.push(FieldError::new(
                "note",
                "length",
                "must be at most 1000 characters long",
            ));
        }
        match (input.payment_method.as_str(), &input.installment) {
            ("installment", None) => problems.push(FieldError::new(
                "installment",
                "required",
                "is required with the payment method `installment`",
            )),
            ("installment", Some(plan)) => {
                if !(1..=40).contains(&plan.partner.trim().chars().count()) {
                    problems.push(FieldError::new(
                        "installment.partner",
                        "length",
                        "must be 1 to 40 characters",
                    ));
                }
                if !INSTALLMENT_MONTHS.contains(&plan.months) {
                    problems.push(
                        FieldError::new("installment.months", "invalid", "must be one of {values}")
                            .with_param(
                                "values",
                                INSTALLMENT_MONTHS.map(|months| months.to_string()).join(", "),
                            ),
                    );
                }
            }
            (_, Some(_)) => problems.push(FieldError::new(
                "installment",
                "not_allowed",
                "is only for the payment method `installment`",
            )),
            _ => {}
        }
        // Ô người mua nhập: kiểm cùng bộ gom lỗi của lượt đặt (D-235).
        let custom_fields = match self
            .custom_fields
            .validate_buyer_input(tenant, ORDER_TARGET, input.custom_fields.as_ref())
            .await
        {
            Ok(values) => (!values.is_empty())
                .then(|| serde_json::to_string(&values).map_err(Error::internal))
                .transpose()?,
            Err(error) if !error.fields().is_empty() => {
                problems.extend(error.fields().iter().cloned());
                None
            }
            Err(error) => return Err(error),
        };
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        // Bản chụp nơi giao: địa chỉ thật (kể cả địa chỉ lấy từ sổ của người mua) và người nhận —
        // đơn không phụ thuộc sổ địa chỉ đổi sau này.
        let chosen: Option<ShipTo> =
            row.ship_to.as_deref().and_then(|raw| serde_json::from_str(raw).ok());
        let ship_to = price_input.destination.clone().map(|address| ShipTo {
            address_id: chosen.as_ref().and_then(|chosen| chosen.address_id),
            recipient_name: Some(
                chosen
                    .as_ref()
                    .and_then(|chosen| chosen.recipient_name.clone())
                    .unwrap_or_else(|| input.contact.name.clone()),
            ),
            phone: Some(
                chosen
                    .as_ref()
                    .and_then(|chosen| chosen.phone.clone())
                    .unwrap_or_else(|| input.contact.phone.clone()),
            ),
            address: Some(address),
        });
        if quote.requires_shipping
            && (quote.shipping.is_none() || price_input.destination.is_none())
        {
            return Err(errors::delivery_required());
        }
        if method == Method::Cod && !quote.requires_shipping {
            return Err(errors::payment_method_unavailable(&input.payment_method));
        }
        if method == Method::Cod
            && !self.policy(tenant, OrgId::from_uuid(row.org_id)).await?.cod_enabled
        {
            return Err(errors::cod_not_allowed());
        }
        // Chống bom hàng (D-194): người mua bị chặn hay rủi ro cao theo quy tắc → phải trả trước.
        if method == Method::Cod {
            self.check_cod_risk(
                tenant,
                OrgId::from_uuid(row.org_id),
                &input.contact.phone,
                quote.total,
            )
            .await?;
        }
        if method != Method::Cod && self.ledger.is_none() {
            return Err(errors::payment_method_unavailable(&input.payment_method));
        }

        // 3. Lượt đặt `reserving`.
        let order_id = Uuid::now_v7();
        let now = crate::service::now();
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

        // Khoá liên hệ tính trước khi giữ chỗ: lỗi ở đây không để lại chỗ giữ nào.
        let buyer_key = match self.buyer_key(tenant, &input.contact.phone).await {
            Ok(key) => key,
            Err(error) => {
                self.fail_checkout(tenant, order_id, error.code()).await?;
                return Err(error);
            }
        };

        // 4–5. Giữ chỗ ngoài transaction.
        let org = OrgId::from_uuid(row.org_id);
        let hold_until = now + Duration::minutes(self.config.hold_minutes);
        let mut holds = Holds::default();
        let held = self
            .hold_all(tenant, order_id, org, None, &quote, &price_input, hold_until, &mut holds)
            .await;
        if let Err(error) = held {
            self.release_holds(tenant, order_id, &holds, ReleaseReason::CheckoutAborted).await;
            self.fail_checkout(tenant, order_id, error.code()).await?;
            return Err(error);
        }

        // Trả trước một phần cho hàng đặt trước (D-232): chỉ khi trả online (COD thu khi giao).
        let advance =
            if method == Method::Cod { None } else { self.plan_for(tenant, org, &quote).await? };

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
            buyer_key,
            custom_fields,
            advance,
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
    /// `fixed`: chỉ lấy hàng từ kho này (bán tại quầy: kho của cửa hàng).
    #[expect(clippy::too_many_arguments, reason = "một lượt giữ cần chừng đó dữ liệu")]
    pub(crate) async fn hold_all(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
        org: OrgId,
        fixed: Option<OrgId>,
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
            request.fixed_org_id = fixed;
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
            .col_expr(checkout::Column::UpdatedAt, Expr::value(crate::service::now()))
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
                let now = crate::service::now();
                let year = now.year();
                let sequence =
                    numbering::next(&transaction, tenant.id(), &format!("orders.order:{year}"))
                        .await?;
                let number = format!("{year}-{sequence:06}");
                insert_lines(&transaction, draft.lines(tenant)?).await?;
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
                if let Some(planned) = &draft.advance {
                    crate::advance_flow::plan_row(tenant, created.id, planned)?
                        .insert(&transaction)
                        .await?;
                }
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
    /// Khoá liên hệ của số điện thoại người mua (tra đơn, gắn đơn, chống bom hàng).
    buyer_key: Option<String>,
    /// Ô người mua nhập đã kiểm (JSON).
    custom_fields: Option<String>,
    /// Kế hoạch trả trước một phần (D-232).
    advance: Option<crate::advance_flow::Planned>,
}

impl Draft<'_> {
    fn order_row(
        &self,
        tenant: &TenantContext,
        number: String,
        payment_status: &str,
    ) -> Result<order::ActiveModel, Error> {
        let quote = self.quote;
        let now = crate::service::now();
        let json = |value: &dyn erased::Json| value.to_json();
        Ok(order::ActiveModel {
            id: Set(self.order_id),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(self.org.as_uuid()),
            checkout_id: Set(Some(self.order_id)),
            number: Set(number),
            channel: Set("web".to_owned()),
            external_ref: Set(None),
            buyer_user_id: Set(self.buyer),
            buyer_key: Set(self.buyer_key.clone()),
            contact: Set(json(&self.input.contact)?),
            ship_to: Set(self.ship_to.as_ref().map(|ship_to| json(ship_to)).transpose()?),
            shipping_option: Set(quote.shipping.as_ref().map(|option| {
                serde_json::json!({"method_id": option.method_id, "option_key": option.option_key,
                    "method_code": option.method_code, "name": option.name,
                    "carrier_code": option.carrier_code, "amount": option.amount,
                    "pickup_org_id": option.pickup_org_id, "eta": option.eta,
                    "tax_category": option.tax_category,
                    "hold_days": option.hold_days})
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
            preorder_accepted_at: Set(self.quote.has_preorder().then_some(now)),
            note: Set(self.input.note.clone()),
            locale: Set(self.cart.locale.clone()),
            placed_at: Set(now),
            confirmed_at: Set(None),
            delivered_at: Set(None),
            completed_at: Set(None),
            cancelled_at: Set(None),
            cancel_reason: Set(None),
            version: Set(1),
            custom_fields: Set(self.custom_fields.clone()),
            created_at: Set(now),
            updated_at: Set(now),
        })
    }

    /// Dòng đơn và thuế của dòng.
    fn lines(
        &self,
        tenant: &TenantContext,
    ) -> Result<Vec<(line::ActiveModel, Vec<line_tax::ActiveModel>)>, Error> {
        let mut rows = quoted_rows(tenant, self.order_id, self.quote)?;
        if let Some(planned) = &self.advance {
            for (index, (row, _)) in rows.iter_mut().enumerate() {
                if let Some(advance) =
                    planned.split.line_advance.get(index).filter(|a| a.minor() > 0)
                {
                    row.advance_minor = Set(Some(advance.minor()));
                    row.preorder_commit_at = Set(planned.commit_at.get(index).copied().flatten());
                }
            }
        }
        Ok(rows)
    }

    fn payment_row(&self, tenant: &TenantContext, order: &order::Model) -> payment::ActiveModel {
        let now = crate::service::now();
        // Trả trước một phần: khoản lúc đặt là `upfront` (hàng sẵn + phần trả trước).
        let (purpose, amount) = match &self.advance {
            Some(planned) => (crate::advance::PURPOSE_ADVANCE, planned.split.upfront.minor()),
            None => (payments::PURPOSE_ORDER, order.total_minor),
        };
        payment::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            order_id: Set(order.id),
            purpose: Set(purpose.to_owned()),
            method: Set(self.input.payment_method.clone()),
            gateway: Set(None),
            ledger_payment_id: Set(None),
            external_id: Set(None),
            reference: Set(payments::reference(order.id, purpose, 1)),
            amount_minor: Set(amount),
            currency: Set(order.currency.clone()),
            tendered_minor: Set(None),
            status: Set(payments::PENDING.to_owned()),
            action: Set(None),
            expires_at: Set(order.payment_due_at),
            succeeded_at: Set(None),
            installment: Set(self.input.installment.as_ref().map(|plan| {
                json!({"partner": plan.partner.trim(), "months": plan.months}).to_string()
            })),
            created_at: Set(now),
            updated_at: Set(now),
        }
    }
}

/// Dòng đơn và thuế của dòng từ báo giá `quote` (đặt hàng online và bán tại quầy dùng chung).
/// Số dòng mỗi câu `INSERT` nhiều dòng: dòng đơn ~40 cột × 200 = 8 000 tham số, dưới giới hạn của
/// SQLite (32 766) và Postgres (65 535).
const INSERT_BATCH: usize = 200;

/// Ghi dòng đơn rồi dòng thuế của chúng, mỗi bảng một câu `INSERT` cho mỗi lô
/// (D-20261120-step39-hot-paths; trước: một câu mỗi dòng và mỗi dòng thuế — đơn 6 dòng tốn thêm
/// 5–10 câu trong transaction ghi). Mọi trường của hai kiểu dòng đều `Set` nên gộp không đổi giá
/// trị nào.
pub(crate) async fn insert_lines(
    transaction: &sea_orm::DatabaseTransaction,
    rows: Vec<(line::ActiveModel, Vec<line_tax::ActiveModel>)>,
) -> Result<(), Error> {
    let (lines, taxes): (Vec<_>, Vec<_>) = rows.into_iter().unzip();
    let taxes: Vec<line_tax::ActiveModel> = taxes.into_iter().flatten().collect();
    for chunk in lines.chunks(INSERT_BATCH) {
        line::Entity::insert_many(chunk.to_vec()).exec_without_returning(transaction).await?;
    }
    for chunk in taxes.chunks(INSERT_BATCH) {
        line_tax::Entity::insert_many(chunk.to_vec()).exec_without_returning(transaction).await?;
    }
    Ok(())
}

pub(crate) fn quoted_rows(
    tenant: &TenantContext,
    order_id: Uuid,
    quote: &Quote,
) -> Result<Vec<(line::ActiveModel, Vec<line_tax::ActiveModel>)>, Error> {
    let evaluation = quote.evaluation.as_ref();
    quote
        .lines
        .iter()
        .map(|quoted| {
            let id = Uuid::now_v7();
            let discounts: Vec<_> = evaluation
                .map(|evaluation| {
                    evaluation.lines.iter().filter(|d| d.line_key == quoted.key).cloned().collect()
                })
                .unwrap_or_default();
            let row = line_row(tenant, order_id, id, quoted, &discounts)?;
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
        allocated_quantity: Set(0),
        cancelled_quantity: Set(0),
        preorder_delayed_to: Set(None),
        advance_minor: Set(None),
        preorder_commit_at: Set(None),
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
        created_at: Set(crate::service::now()),
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
