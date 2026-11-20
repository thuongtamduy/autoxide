//! Bán tại quầy (POS) cho chuỗi (đặc tả 45 mục "POS", D-227, D-340):
//!
//! - **Ca** (`orders_pos_shifts`): mở ca với tiền đầu ca, một ca mở mỗi quầy (cột khoá `open_key`
//!   unique — NULL khi đóng, như `active_buyer_key`). Đóng ca: `chênh lệch = tiền đếm − tiền dự
//!   kiến` (tiền đầu ca + tiền mặt thực vào két của các lượt bán trong ca); khác 0 thì bắt buộc lý do
//!   thuộc danh sách `orders.cash_variance` và ghi chú; ghi sổ ngay vào tài khoản chờ xử lý của cửa
//!   hàng (thiếu: Nợ `orders.cash_shortage` / Có `cash.on_hand`; thừa: Nợ `cash.on_hand` / Có
//!   `orders.cash_overage`). Vượt ngưỡng `orders.cash_variance_approval_minor` thì ca chờ một người
//!   KHÁC duyệt rồi mới ghi sổ.
//! - **Bán** (`POST /pos/sales`): cùng hàm báo giá với giỏ online (giá của cửa hàng, khuyến mại; mã
//!   không áp được thì bỏ qua và trả lý do), giữ tồn CHỈ ở kho của cửa hàng (`fixed_org_id`), tổng
//!   các khoản thu phải bằng tổng đơn; đơn tạo ở `confirmed` rồi `delivered` trong cùng transaction
//!   (khách nhận hàng tại chỗ) — `inventory` trừ kho khi nghe `orders.order_confirmed`, doanh thu và
//!   hoá đơn từ máy tính tiền theo `orders.order_delivered`, hoàn tất bằng job như mọi đơn.
//! - **Chống trùng**: `reference` do máy quầy sinh (kể cả lúc ngoại tuyến) — unique theo ca; gửi lại
//!   cùng nội dung → 200 kết quả cũ, khác nội dung → 422.
//! - **Tiền**: core không giữ tiền (D-187) — tiền mặt ở két của cửa hàng, thẻ qua máy POS của ngân
//!   hàng, VietQR về tài khoản người bán; `orders` chỉ ghi sổ: Nợ `cash.on_hand` / `clearing.gateway`
//!   (thẻ) / `cash.bank` (VietQR), Có `receivable.customer`.

use autoxide_inventory::api::ReleaseReason;
use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::reference::{Submitted, fingerprint};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use autoxide_ledger::api::{
    AccountKey, Counterparty, LineAccount, NewEntry, NewLine, OpenAccount, Side,
};
use chrono::{Datelike, Duration};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QuerySelect, Set};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::OrderStatus;
use crate::checkout::{Holds, quoted_rows};
use crate::entity::{order, payment, pos_sale, pos_shift};
use crate::errors;
use crate::events::{Change, PosShiftClosed};
use crate::lifecycle::{self, Cause, Transition};
use crate::payments;
use crate::permissions::{POS_SELL, POS_SHIFT};
use crate::policy::CASH_VARIANCE_APPROVAL_MINOR;
use crate::pricing::{self, PriceInput, Quote};
use crate::reasons::CASH_VARIANCE;
use crate::service::{OrdersService, now};

pub(crate) const OPEN: &str = "open";
pub(crate) const PENDING_APPROVAL: &str = "pending_approval";
pub(crate) const CLOSED: &str = "closed";

/// Cách thu ở quầy.
pub(crate) const METHODS: [&str; 3] = ["cash", "card_terminal", "vietqr"];

/// Một dòng bán ở quầy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PosLine {
    pub variant_id: Uuid,
    pub quantity: i64,
}

/// Một khoản thu ở quầy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PosPayment {
    pub method: String,
    pub amount: i64,
    /// Tiền mặt khách đưa (≥ `amount`); tiền thối = `tendered − amount`.
    pub tendered: Option<i64>,
    /// Mã chuẩn chi của máy POS thẻ / mã giao dịch VietQR.
    pub approval_code: Option<String>,
}

/// Giỏ ở quầy.
#[derive(Debug, Clone)]
pub(crate) struct PosCart {
    pub lines: Vec<PosLine>,
    pub codes: Vec<String>,
    pub customer_id: Option<Uuid>,
}

/// Lượt bán đã ghi.
#[derive(Debug, Clone)]
pub(crate) struct PosSold {
    pub order_id: Uuid,
    pub change: Money,
    pub quote: Option<Quote>,
}

impl OrdersService {
    async fn shift(&self, tenant: &TenantContext, id: Uuid) -> Result<pos_shift::Model, Error> {
        pos_shift::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(errors::shift_not_found)
    }

    /// `POST /pos/shifts`: mở ca cho quầy `register_code` của cửa hàng `org` (`orders.pos.shift`).
    pub(crate) async fn open_shift(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: OrgId,
        register_code: &str,
        opening_cash: Money,
    ) -> Result<pos_shift::Model, Error> {
        principal.require(&POS_SHIFT)?;
        let read = tenant.database().read();
        principal.require_at(read, &POS_SHIFT, OrgTarget::new(org, None)).await?;
        let register = register_code.trim();
        let mut problems = Vec::new();
        if !(1..=40).contains(&register.chars().count()) {
            problems.push(FieldError::new("register_code", "length", "must be 1 to 40 characters"));
        }
        if opening_cash.is_negative() {
            problems.push(FieldError::new("opening_cash", "negative", "must not be negative"));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let at = now();
        let row = pos_shift::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(org.as_uuid()),
            register_code: Set(register.to_owned()),
            open_key: Set(Some(format!("{org}:{register}"))),
            opened_by: Set(principal.user_id()),
            opened_at: Set(at),
            currency: Set(opening_cash.currency().code().to_owned()),
            opening_cash_minor: Set(opening_cash.minor()),
            closed_by: Set(None),
            closed_at: Set(None),
            expected_cash_minor: Set(None),
            counted_cash_minor: Set(None),
            variance_minor: Set(None),
            variance_reason: Set(None),
            variance_note: Set(None),
            approved_by: Set(None),
            approved_at: Set(None),
            status: Set(OPEN.to_owned()),
            updated_at: Set(at),
        };
        match row.insert(tenant.database().write()).await {
            Ok(saved) => Ok(saved),
            // Quầy đang có ca mở (unique trên `open_key`).
            Err(error) => {
                let open = pos_shift::Entity::find()
                    .filter(pos_shift::Column::OpenKey.eq(format!("{org}:{register}")))
                    .scoped(tenant)
                    .one(tenant.database().write())
                    .await?;
                match open {
                    Some(_) => Err(errors::shift_already_open(register)),
                    None => Err(error.into()),
                }
            }
        }
    }

    /// Tiền mặt dự kiến trong két của ca: tiền đầu ca + tiền mặt thực vào của các lượt bán.
    async fn expected_cash(
        &self,
        tenant: &TenantContext,
        shift: &pos_shift::Model,
    ) -> Result<i64, Error> {
        let sales: Vec<i64> = pos_sale::Entity::find()
            .select_only()
            .column(pos_sale::Column::CashMinor)
            .filter(pos_sale::Column::ShiftId.eq(shift.id))
            .scoped(tenant)
            .into_tuple()
            .all(tenant.database().write())
            .await?;
        Ok(shift.opening_cash_minor + sales.iter().sum::<i64>())
    }

    /// `POST /pos/shifts/{id}/close`: đóng ca với tiền đếm được. Chênh lệch khác 0 cần lý do và ghi
    /// chú; vượt ngưỡng thì chờ người khác duyệt.
    pub(crate) async fn close_shift(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        counted: i64,
        reason: Option<String>,
        note: Option<String>,
    ) -> Result<pos_shift::Model, Error> {
        principal.require(&POS_SHIFT)?;
        let shift = self.shift(tenant, id).await?;
        let org = OrgId::from_uuid(shift.org_id);
        principal
            .require_at(tenant.database().read(), &POS_SHIFT, OrgTarget::new(org, None))
            .await?;
        if shift.status != OPEN {
            return Err(errors::shift_not_open());
        }
        let expected = self.expected_cash(tenant, &shift).await?;
        let variance = counted - expected;
        let mut problems = Vec::new();
        if counted < 0 {
            problems.push(FieldError::new("counted_cash", "negative", "must not be negative"));
        }
        let reason = reason.map(|reason| reason.trim().to_owned()).filter(|r| !r.is_empty());
        let note = note.map(|note| note.trim().to_owned()).filter(|n| !n.is_empty());
        if variance != 0 {
            match &reason {
                Some(code) => {
                    if let Err(error) = self.reasons.ensure(tenant, CASH_VARIANCE, code).await {
                        problems.push(FieldError::new(
                            "variance_reason",
                            "invalid",
                            "is not a reason of the list `orders.cash_variance`",
                        ));
                        tracing::debug!(code = error.code(), "cash variance reason refused");
                    }
                }
                None => problems.push(FieldError::new(
                    "variance_reason",
                    "required",
                    "is required when the counted cash differs from the expected cash",
                )),
            }
            if note.as_ref().is_none_or(|note| note.chars().count() > 1000) {
                problems.push(FieldError::new(
                    "variance_note",
                    "required",
                    "is required (up to 1000 characters) when the cash differs",
                ));
            }
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let threshold = self
            .setting(tenant, org, &CASH_VARIANCE_APPROVAL_MINOR)
            .await?
            .as_ref()
            .and_then(Value::as_i64)
            .unwrap_or(200_000);
        let status = if variance.abs() > threshold { PENDING_APPROVAL } else { CLOSED };
        let at = now();
        let closed = pos_shift::Entity::update_many()
            .col_expr(pos_shift::Column::Status, Expr::value(status))
            .col_expr(pos_shift::Column::OpenKey, Expr::value(Option::<String>::None))
            .col_expr(pos_shift::Column::ClosedBy, Expr::value(principal.user_id()))
            .col_expr(pos_shift::Column::ClosedAt, Expr::value(at))
            .col_expr(pos_shift::Column::ExpectedCashMinor, Expr::value(expected))
            .col_expr(pos_shift::Column::CountedCashMinor, Expr::value(counted))
            .col_expr(pos_shift::Column::VarianceMinor, Expr::value(variance))
            .col_expr(pos_shift::Column::VarianceReason, Expr::value(reason))
            .col_expr(pos_shift::Column::VarianceNote, Expr::value(note))
            .col_expr(pos_shift::Column::UpdatedAt, Expr::value(at))
            .filter(pos_shift::Column::Id.eq(id))
            .filter(pos_shift::Column::Status.eq(OPEN))
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        if closed.rows_affected == 0 {
            return Err(errors::shift_not_open());
        }
        if status == CLOSED {
            self.finish_shift(tenant, Actor::user(principal.user_id()), id).await?;
        }
        self.shift(tenant, id).await
    }

    /// `POST /pos/shifts/{id}/approve`: người KHÁC người đóng ca duyệt chênh lệch vượt ngưỡng.
    pub(crate) async fn approve_shift(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<pos_shift::Model, Error> {
        principal.require(&POS_SHIFT)?;
        let shift = self.shift(tenant, id).await?;
        let org = OrgId::from_uuid(shift.org_id);
        principal
            .require_at(tenant.database().read(), &POS_SHIFT, OrgTarget::new(org, None))
            .await?;
        if shift.status != PENDING_APPROVAL {
            return Err(errors::shift_not_pending());
        }
        if shift.closed_by == Some(principal.user_id()) {
            return Err(errors::shift_approval_self());
        }
        let at = now();
        let approved = pos_shift::Entity::update_many()
            .col_expr(pos_shift::Column::Status, Expr::value(CLOSED))
            .col_expr(pos_shift::Column::ApprovedBy, Expr::value(principal.user_id()))
            .col_expr(pos_shift::Column::ApprovedAt, Expr::value(at))
            .col_expr(pos_shift::Column::UpdatedAt, Expr::value(at))
            .filter(pos_shift::Column::Id.eq(id))
            .filter(pos_shift::Column::Status.eq(PENDING_APPROVAL))
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        if approved.rows_affected == 0 {
            return Err(errors::shift_not_pending());
        }
        self.finish_shift(tenant, Actor::user(principal.user_id()), id).await?;
        self.shift(tenant, id).await
    }

    /// Ca đã đóng hẳn: event `orders.pos_shift_closed` rồi bút toán chênh lệch (gọi lại không ghi
    /// thêm nhờ mã tham chiếu).
    async fn finish_shift(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
    ) -> Result<(), Error> {
        let shift = self.shift(tenant, id).await?;
        let transaction = tenant.database().begin_write().await?;
        let event = PosShiftClosed(Change::after(json!({
            "shift_id": shift.id, "tenant_id": shift.tenant_id, "org_id": shift.org_id,
            "register_code": shift.register_code, "currency": shift.currency,
            "expected_cash_minor": shift.expected_cash_minor,
            "counted_cash_minor": shift.counted_cash_minor, "variance_minor": shift.variance_minor,
            "variance_reason": shift.variance_reason, "approved_by": shift.approved_by,
        })));
        self.outbox
            .record(&transaction, tenant, actor, Some(format!("orders.pos_shift:{id}")), &event)
            .await?;
        transaction.commit().await?;
        let variance = shift.variance_minor.unwrap_or(0);
        let Some(ledger) = &self.ledger else { return Ok(()) };
        if variance == 0 {
            return Ok(());
        }
        let currency = Currency::parse(&shift.currency)?;
        let book = OrgId::from_uuid(shift.org_id);
        let amount = Money::new(variance.abs(), currency);
        let cash = AccountKey::new(book, "cash.on_hand", currency);
        let (purpose, side) = if variance < 0 {
            ("orders.cash_shortage", Side::Debit)
        } else {
            ("orders.cash_overage", Side::Credit)
        };
        let suspense = AccountKey::new(book, purpose, currency);
        ledger.open_account(tenant, OpenAccount::new(suspense.clone()).normal_side(side)).await?;
        let lines = if variance < 0 {
            vec![
                NewLine::debit(LineAccount::Key(suspense), amount),
                NewLine::credit(LineAccount::Key(cash), amount),
            ]
        } else {
            vec![
                NewLine::debit(LineAccount::Key(cash), amount),
                NewLine::credit(LineAccount::Key(suspense), amount),
            ]
        };
        let entry = NewEntry::new(
            book,
            "orders",
            "pos_shift",
            format!("orders:pos_shift:{id}:variance"),
            format!("Cash difference of register {}", shift.register_code),
            lines,
        )
        .source_id(id.to_string());
        ledger.post(tenant, Actor::system(), entry).await?;
        Ok(())
    }

    /// Đầu vào báo giá của giỏ ở quầy.
    fn pos_price_input(
        &self,
        shift: &pos_shift::Model,
        cart: &PosCart,
    ) -> Result<PriceInput, Error> {
        Ok(PriceInput {
            org_id: OrgId::from_uuid(shift.org_id),
            currency: Currency::parse(&shift.currency)?,
            locale: "vi".to_owned(),
            lines: cart.lines.iter().map(|line| (line.variant_id, line.quantity)).collect(),
            codes: cart.codes.clone(),
            voucher_ids: Vec::new(),
            gift_selections: Vec::new(),
            buyer_id: cart.customer_id,
            buyer_keys: cart.customer_id.map(|id| vec![format!("user:{id}")]).unwrap_or_default(),
            profile: crate::api::BuyerProfile::default(),
            destination: None,
            shipping_option: None,
            payment_method: None,
            client: Some("pos".to_owned()),
            first_order: false,
            at: now(),
        })
    }

    fn check_cart(&self, cart: &PosCart, problems: &mut Vec<FieldError>) {
        if cart.lines.is_empty() || cart.lines.len() > self.config.cart_max_lines {
            problems.push(
                FieldError::new("lines", "length", "must have 1 to {max} lines")
                    .with_param("max", self.config.cart_max_lines),
            );
        }
        for (index, line) in cart.lines.iter().enumerate() {
            if !(1..=self.config.line_max_quantity).contains(&line.quantity) {
                problems.push(
                    FieldError::new(
                        format!("lines[{index}].quantity"),
                        "range",
                        "must be 1 to {max}",
                    )
                    .with_param("max", self.config.line_max_quantity),
                );
            }
        }
    }

    /// Ca mở của cửa hàng mà người bán có `orders.pos.sell`.
    async fn sell_shift(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        shift_id: Uuid,
    ) -> Result<pos_shift::Model, Error> {
        principal.require(&POS_SELL)?;
        let shift = self.shift(tenant, shift_id).await?;
        let target = OrgTarget::new(OrgId::from_uuid(shift.org_id), None);
        principal.require_at(tenant.database().read(), &POS_SELL, target).await?;
        if shift.status != OPEN {
            return Err(errors::shift_not_open());
        }
        Ok(shift)
    }

    /// `POST /pos/quote`: tạm tính giỏ ở quầy, không ghi gì.
    pub(crate) async fn pos_quote(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        shift_id: Uuid,
        cart: &PosCart,
    ) -> Result<Quote, Error> {
        let shift = self.sell_shift(tenant, principal, shift_id).await?;
        let mut problems = Vec::new();
        self.check_cart(cart, &mut problems);
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let input = self.pos_price_input(&shift, cart)?;
        pricing::quote(&self.sources, tenant, &input, self.config.quote_ttl_minutes).await
    }

    /// `POST /pos/sales`: một lượt bán ở quầy (xem đầu file).
    pub(crate) async fn pos_sell(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        shift_id: Uuid,
        reference: &str,
        cart: PosCart,
        paid: Vec<PosPayment>,
    ) -> Result<Submitted<PosSold>, Error> {
        let shift = self.sell_shift(tenant, principal, shift_id).await?;
        let currency = Currency::parse(&shift.currency)?;
        let reference = reference.trim();
        let mut problems = Vec::new();
        if !(1..=200).contains(&reference.chars().count()) {
            problems.push(FieldError::new("reference", "length", "must be 1 to 200 characters"));
        }
        self.check_cart(&cart, &mut problems);
        if paid.is_empty() {
            problems.push(FieldError::new("payments", "required", "needs at least one payment"));
        }
        for (index, payment) in paid.iter().enumerate() {
            if !METHODS.contains(&payment.method.as_str()) {
                problems.push(
                    FieldError::new(
                        format!("payments[{index}].method"),
                        "invalid",
                        "must be one of {values}",
                    )
                    .with_param("values", METHODS.join(", ")),
                );
            }
            if payment.amount <= 0 {
                problems.push(FieldError::new(
                    format!("payments[{index}].amount"),
                    "range",
                    "must be more than 0",
                ));
            }
            match (payment.method.as_str(), payment.tendered) {
                ("cash", Some(tendered)) if tendered < payment.amount => {
                    problems.push(FieldError::new(
                        format!("payments[{index}].tendered"),
                        "range",
                        "must be at least the amount",
                    ))
                }
                ("cash", _) => {}
                (_, Some(_)) => problems.push(FieldError::new(
                    format!("payments[{index}].tendered"),
                    "not_allowed",
                    "is only for cash",
                )),
                _ => {}
            }
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let print = fingerprint(&json!({"lines": cart.lines, "codes": cart.codes,
            "customer_id": cart.customer_id, "payments": paid}))?;
        if let Some(sold) = self.pos_replay(tenant, shift_id, reference, &print).await? {
            return Ok(Submitted::Replayed(sold));
        }
        let input = self.pos_price_input(&shift, &cart)?;
        let quote =
            pricing::quote(&self.sources, tenant, &input, self.config.quote_ttl_minutes).await?;
        if let Some(error) = quote.lines.iter().find_map(pricing::line_error) {
            return Err(error);
        }
        let total: i64 = paid.iter().map(|payment| payment.amount).sum();
        if total != quote.total.minor() {
            return Err(errors::payments_mismatch(Money::new(total, currency), quote.total));
        }
        let change: i64 = paid
            .iter()
            .filter_map(|payment| payment.tendered.map(|tendered| tendered - payment.amount))
            .sum();
        let cash: i64 = paid
            .iter()
            .filter(|payment| payment.method == "cash")
            .map(|payment| payment.amount)
            .sum();

        // Giữ tồn ở kho của cửa hàng và lượt khuyến mại, ngoài transaction (như đặt hàng online).
        let order_id = Uuid::now_v7();
        let org = OrgId::from_uuid(shift.org_id);
        let mut holds = Holds::default();
        let until = now() + Duration::minutes(self.config.hold_minutes);
        if let Err(error) =
            self.hold_all(tenant, order_id, org, Some(org), &quote, &input, until, &mut holds).await
        {
            self.release_holds(tenant, order_id, &holds, ReleaseReason::CheckoutAborted).await;
            return Err(error);
        }
        let created = self
            .create_pos_order(
                tenant, principal, &shift, order_id, reference, &print, &quote, &paid, cash,
            )
            .await;
        match created {
            Ok(true) => {}
            Ok(false) => {
                // Một lượt gửi song song cùng `reference` đã ghi trước: trả chỗ của lượt này.
                self.release_holds(tenant, order_id, &holds, ReleaseReason::CheckoutAborted).await;
                return match self.pos_replay(tenant, shift_id, reference, &print).await? {
                    Some(sold) => Ok(Submitted::Replayed(sold)),
                    None => Err(errors::shift_not_open()),
                };
            }
            Err(error) => {
                self.release_holds(tenant, order_id, &holds, ReleaseReason::CheckoutAborted).await;
                return Err(error);
            }
        }
        self.post_pos_payments(tenant, order_id).await?;
        Ok(Submitted::Created(PosSold {
            order_id,
            change: Money::new(change, currency),
            quote: Some(quote),
        }))
    }

    /// Lượt bán đã ghi với cùng `reference` trong ca: cùng nội dung → kết quả cũ, khác → 422.
    async fn pos_replay(
        &self,
        tenant: &TenantContext,
        shift_id: Uuid,
        reference: &str,
        print: &str,
    ) -> Result<Option<PosSold>, Error> {
        let found = pos_sale::Entity::find()
            .filter(pos_sale::Column::ShiftId.eq(shift_id))
            .filter(pos_sale::Column::Reference.eq(reference))
            .scoped(tenant)
            .one(tenant.database().write())
            .await?;
        let Some(found) = found else { return Ok(None) };
        if found.fingerprint != print {
            return Err(errors::checkout_key_reused(reference));
        }
        let paid = payment::Entity::find()
            .filter(payment::Column::OrderId.eq(found.order_id))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        let order = self.order(tenant, found.order_id).await?;
        let change: i64 = paid
            .iter()
            .filter_map(|payment| {
                payment.tendered_minor.map(|tendered| tendered - payment.amount_minor)
            })
            .sum();
        Ok(Some(PosSold {
            order_id: found.order_id,
            change: Money::from_columns(change, &order.currency)?,
            quote: None,
        }))
    }

    /// Một transaction: số đơn, đơn `confirmed` → `delivered`, dòng, thuế, khoản thu, lượt bán.
    /// `false`: một lượt song song cùng `reference` đã ghi trước (không tạo gì).
    #[expect(clippy::too_many_arguments, reason = "một lượt bán cần chừng đó dữ liệu")]
    async fn create_pos_order(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        shift: &pos_shift::Model,
        order_id: Uuid,
        reference: &str,
        print: &str,
        quote: &Quote,
        paid: &[PosPayment],
        cash: i64,
    ) -> Result<bool, Error> {
        let database = tenant.database();
        let outbox = &self.outbox;
        let actor = Actor::user(principal.user_id());
        let funds_flow = match &self.ledger {
            Some(ledger) => payments::funds_flow(
                ledger.flow_settings(tenant, OrgId::from_uuid(shift.org_id)).await?.mode,
            ),
            None => crate::api::FundsFlow::SelfCollect,
        };
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let at = now();
                let inserted = pos_sale::Entity::insert(pos_sale::ActiveModel {
                    tenant_id: Set(tenant.id().as_uuid()),
                    shift_id: Set(shift.id),
                    reference: Set(reference.to_owned()),
                    fingerprint: Set(print.to_owned()),
                    order_id: Set(order_id),
                    cash_minor: Set(cash),
                    created_at: Set(at),
                })
                .on_conflict_do_nothing()
                .exec_without_returning(&transaction)
                .await?;
                if !matches!(inserted, sea_orm::TryInsertResult::Inserted(rows) if rows > 0) {
                    return Ok(false);
                }
                let year = at.year();
                let sequence = autoxide_kernel::numbering::next(
                    &transaction,
                    tenant.id(),
                    &format!("orders.order:{year}"),
                )
                .await?;
                for (row, taxes) in quoted_rows(tenant, order_id, quote)? {
                    row.insert(&transaction).await?;
                    for tax in taxes {
                        tax.insert(&transaction).await?;
                    }
                }
                let method =
                    if paid.len() == 1 { paid[0].method.clone() } else { "split".to_owned() };
                let row = order::ActiveModel {
                    id: Set(order_id),
                    tenant_id: Set(tenant.id().as_uuid()),
                    org_id: Set(shift.org_id),
                    checkout_id: Set(None),
                    number: Set(format!("{year}-{sequence:06}")),
                    channel: Set("pos".to_owned()),
                    external_ref: Set(None),
                    buyer_user_id: Set(None),
                    buyer_key: Set(None),
                    contact: Set(json!({"name": "", "phone": ""}).to_string()),
                    ship_to: Set(None),
                    shipping_option: Set(None),
                    pickup_org_id: Set(None),
                    status: sea_orm::ActiveValue::NotSet,
                    payment_status: Set(payments::PAID.to_owned()),
                    fulfillment_status: Set("fulfilled".to_owned()),
                    payment_method: Set(method),
                    funds_flow: Set(funds_flow.as_str().to_owned()),
                    currency: Set(shift.currency.clone()),
                    subtotal_minor: Set(quote.subtotal.minor()),
                    discount_minor: Set(quote.discount.minor()),
                    shipping_fee_minor: Set(0),
                    shipping_discount_minor: Set(0),
                    tax_minor: Set(quote.tax.minor()),
                    total_minor: Set(quote.total.minor()),
                    paid_minor: Set(quote.total.minor()),
                    refunded_minor: Set(0),
                    payment_due_at: Set(None),
                    attribution: Set(None),
                    promotions: Set(quote
                        .evaluation
                        .as_ref()
                        .map(|evaluation| {
                            serde_json::to_string(evaluation).map_err(Error::internal)
                        })
                        .transpose()?),
                    stock_shortage: Set(false),
                    invoice_request: Set(None),
                    terms_version: Set(None),
                    note: Set(Some(format!("POS {} / {}", shift.register_code, reference))),
                    locale: Set("vi".to_owned()),
                    placed_at: Set(at),
                    confirmed_at: Set(None),
                    delivered_at: Set(None),
                    completed_at: Set(None),
                    cancelled_at: Set(None),
                    cancel_reason: Set(None),
                    preorder_accepted_at: Set(None),
                    version: Set(1),
                    created_at: Set(at),
                    updated_at: Set(at),
                };
                let confirm = Transition::new(OrderStatus::Confirmed, Cause::Staff, actor);
                lifecycle::create(&transaction, tenant, outbox, row, &confirm).await?;
                for (index, item) in paid.iter().enumerate() {
                    payment::ActiveModel {
                        id: Set(Uuid::now_v7()),
                        tenant_id: Set(tenant.id().as_uuid()),
                        order_id: Set(order_id),
                        purpose: Set(payments::PURPOSE_ORDER.to_owned()),
                        method: Set(item.method.clone()),
                        gateway: Set(None),
                        ledger_payment_id: Set(None),
                        external_id: Set(item.approval_code.clone()),
                        reference: Set(format!("orders:{order_id}:pos:{}", index + 1)),
                        amount_minor: Set(item.amount),
                        currency: Set(shift.currency.clone()),
                        tendered_minor: Set(item.tendered),
                        status: Set(payments::SUCCEEDED.to_owned()),
                        action: Set(None),
                        expires_at: Set(None),
                        succeeded_at: Set(Some(at)),
                        created_at: Set(at),
                        updated_at: Set(at),
                    }
                    .insert(&transaction)
                    .await?;
                }
                // Khách nhận hàng tại chỗ: giao xong ngay.
                let current = lifecycle::lock(&transaction, tenant, order_id).await?;
                let deliver = Transition::new(OrderStatus::Delivered, Cause::Staff, actor);
                lifecycle::apply(&transaction, tenant, outbox, current, &deliver, |_| {}).await?;
                transaction.commit().await?;
                Ok(true)
            })
            .await
    }

    /// Bút toán khoản thu ở quầy (gọi lại không ghi thêm nhờ mã tham chiếu theo khoản).
    async fn post_pos_payments(&self, tenant: &TenantContext, order_id: Uuid) -> Result<(), Error> {
        let Some(ledger) = &self.ledger else { return Ok(()) };
        let row = self.order(tenant, order_id).await?;
        let book = OrgId::from_uuid(row.org_id);
        let paid = payment::Entity::find()
            .filter(payment::Column::OrderId.eq(order_id))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        for item in paid {
            let amount = Money::from_columns(item.amount_minor, &item.currency)?;
            let currency = amount.currency();
            let debit = match item.method.as_str() {
                "card_terminal" => AccountKey::new(book, "clearing.gateway", currency)
                    .with_counterparty(Counterparty::new("partner", "card_terminal")),
                "vietqr" => AccountKey::new(book, "cash.bank", currency),
                _ => AccountKey::new(book, "cash.on_hand", currency),
            };
            let entry = NewEntry::new(
                book,
                "orders",
                "pos_payment",
                item.reference.clone(),
                format!("Payment at the counter for order {}", row.number),
                vec![
                    NewLine::debit(LineAccount::Key(debit), amount),
                    NewLine::credit(
                        LineAccount::Key(AccountKey::new(book, "receivable.customer", currency)),
                        amount,
                    ),
                ],
            )
            .source_id(order_id.to_string());
            ledger.post(tenant, Actor::system(), entry).await?;
        }
        Ok(())
    }

    /// `GET /pos/catalog?barcode=`: một biến thể theo mã vạch / SKU với giá của cửa hàng.
    pub(crate) async fn pos_lookup(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        shift_id: Uuid,
        code: &str,
    ) -> Result<Quote, Error> {
        let shift = self.sell_shift(tenant, principal, shift_id).await?;
        let variant = self
            .sources
            .catalog
            .variant_by_code(tenant, code.trim(), Some("vi"))
            .await?
            .ok_or_else(|| errors::variant_unavailable(code))?;
        let cart = PosCart {
            lines: vec![PosLine { variant_id: variant.variant_id, quantity: 1 }],
            codes: Vec::new(),
            customer_id: None,
        };
        let input = self.pos_price_input(&shift, &cart)?;
        pricing::quote(&self.sources, tenant, &input, self.config.quote_ttl_minutes).await
    }
}
