//! Kiểu trả về của REST (`ToSchema`, chép vào OpenAPI). Doc comment `///` ở đây viết tiếng Anh cho
//! người gọi API.

use autoxide_kernel::money::Money;
use autoxide_kernel::time::{Date, Timestamp};
use autoxide_promotions::api::{AppliedPromotion, RejectedCode};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::carts::{CartView, ShipTo};
use crate::pricing::{LineProblem, QuotedLine};

/// Why a cart line cannot be ordered now.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersLineProblem)]
pub(crate) struct LineProblemResponse {
    /// `orders.variant_unavailable`, `orders.out_of_stock` or `orders.preorder_closed`.
    code: &'static str,
    /// Units left, for `orders.out_of_stock`.
    available: Option<i64>,
}

/// Pre-order information shown before ordering.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersPreorderInfo)]
pub(crate) struct PreorderResponse {
    /// Expected date the item arrives (shop time zone).
    available_on: Option<Date>,
    /// Pre-orders close at.
    ends_at: Option<Timestamp>,
}

/// One cart line, priced again by the server.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersCartLine)]
pub(crate) struct CartLineResponse {
    /// Line key: the variant id, or `gift:<promotion>:<variant>` for a free gift.
    key: String,
    variant_id: Uuid,
    product_id: Option<Uuid>,
    sku: Option<String>,
    name: Option<String>,
    /// Option labels, like `Red / M`.
    variant_label: Option<String>,
    image_media_id: Option<Uuid>,
    quantity: i64,
    /// Unit price before discounts.
    unit_price: Money,
    /// Struck-through price, for display only.
    compare_at: Option<Money>,
    /// Whether `unit_price` includes taxes.
    tax_inclusive: bool,
    /// `unit_price × quantity`.
    subtotal: Money,
    discount: Money,
    tax: Money,
    /// What the buyer pays for the line: after discounts, taxes included.
    total: Money,
    /// The promotion that added this free gift.
    gift_of: Option<Uuid>,
    /// Set when the line cannot be ordered now.
    problem: Option<LineProblemResponse>,
    /// Set for pre-order items.
    preorder: Option<PreorderResponse>,
}

impl CartLineResponse {
    pub(crate) fn new(line: &QuotedLine) -> Self {
        let variant = line.variant.as_ref();
        let problem = line.problem.as_ref().map(|problem| match problem {
            LineProblem::Unavailable => {
                LineProblemResponse { code: "orders.variant_unavailable", available: None }
            }
            LineProblem::OutOfStock { available } => {
                LineProblemResponse { code: "orders.out_of_stock", available: Some(*available) }
            }
            LineProblem::PreorderClosed => {
                LineProblemResponse { code: "orders.preorder_closed", available: None }
            }
        });
        let preorder = variant
            .filter(|variant| variant.sale_mode == autoxide_catalog::api::SaleMode::Preorder)
            .map(|variant| PreorderResponse {
                available_on: variant.preorder_available_on,
                ends_at: variant.preorder_ends_at,
            });
        CartLineResponse {
            key: line.key.clone(),
            variant_id: line.variant_id,
            product_id: variant.map(|variant| variant.product_id),
            sku: variant.map(|variant| variant.sku.clone()),
            name: variant.map(|variant| variant.name.clone()),
            variant_label: variant.map(|variant| variant.variant_label.clone()),
            image_media_id: variant.and_then(|variant| variant.image_media_id),
            quantity: line.quantity,
            unit_price: line.unit_price,
            compare_at: line.compare_at,
            tax_inclusive: line.tax_inclusive,
            subtotal: line.subtotal,
            discount: line.discount,
            tax: line.tax,
            total: line.gross,
            gift_of: line.gift_of,
            problem,
            preorder,
        }
    }
}

/// A way to deliver the cart.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersShippingOption)]
pub(crate) struct ShippingOptionResponse {
    /// Send it back in `PUT /carts/{id}/delivery` to choose this option.
    option_key: String,
    method_code: String,
    name: String,
    /// `table_rate`, `carrier` or `pickup`.
    kind: String,
    amount: Money,
    /// Spending over this ships free.
    free_over: Option<Money>,
    carrier_code: Option<String>,
    /// Store of a pickup option.
    pickup_org_id: Option<Uuid>,
    /// Earliest and latest expected delivery dates.
    eta_earliest: Option<Date>,
    eta_latest: Option<Date>,
}

/// Delivery part of a cart.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersCartShipping)]
pub(crate) struct ShippingResponse {
    /// Some item must be delivered.
    required: bool,
    options: Vec<ShippingOptionResponse>,
    /// The chosen option, when still available.
    selected: Option<String>,
    /// The chosen option is no longer available: choose again.
    selected_unavailable: bool,
}

/// Where the cart goes.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersCartShipTo)]
pub(crate) struct ShipToResponse {
    /// An address of your address book.
    address_id: Option<Uuid>,
    recipient_name: Option<String>,
    phone: Option<String>,
    address: Option<autoxide_shipping::api::Address>,
}

impl From<ShipTo> for ShipToResponse {
    fn from(ship_to: ShipTo) -> Self {
        ShipToResponse {
            address_id: ship_to.address_id,
            recipient_name: ship_to.recipient_name,
            phone: ship_to.phone,
            address: ship_to.address,
        }
    }
}

/// Money totals of a cart.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersCartTotals)]
pub(crate) struct TotalsResponse {
    subtotal: Money,
    discount: Money,
    shipping_fee: Money,
    shipping_discount: Money,
    /// Taxes (already in `total` when prices include taxes).
    tax: Money,
    /// What the buyer pays.
    total: Money,
}

/// Referral of the cart.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersAttribution)]
pub(crate) struct AttributionResponse {
    click_id: Option<Uuid>,
    code: Option<String>,
    live_session_id: Option<Uuid>,
}

/// A cart with its quote, computed again by the server on every read.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersCart)]
pub(crate) struct CartResponse {
    id: Uuid,
    /// The shop (or marketplace store) selling.
    org_id: Uuid,
    /// `active`, `abandoned`, `merged` or `converted`.
    status: String,
    currency: String,
    buyer_user_id: Option<Uuid>,
    lines: Vec<CartLineResponse>,
    /// Coupon codes entered.
    codes: Vec<String>,
    /// Vouchers of your wallet used for this cart.
    voucher_ids: Vec<Uuid>,
    /// Promotions applied, with their terms.
    #[schema(value_type = Vec<Object>)]
    promotions: Vec<AppliedPromotion>,
    /// Codes and vouchers that do not apply, with the reason.
    #[schema(value_type = Vec<Object>)]
    rejected_codes: Vec<RejectedCode>,
    shipping: ShippingResponse,
    ship_to: Option<ShipToResponse>,
    payment_method: Option<String>,
    /// Ways to pay this cart: `cod` (cash on delivery, when the shop allows it and the cart is
    /// delivered; a buyer with too many refused deliveries may still be asked to pay first),
    /// `bank_transfer`, `gateway` (card, wallet… of the payment partner) and `installment`
    /// (through a partner).
    payment_methods: Vec<String>,
    /// Pre-ordered items paid in part (when paying online; not with cash on delivery): what is
    /// paid now and what is asked once the items are ready. Show it, then send
    /// `accept_preorder_terms: true` at checkout. This is an advance payment, not a deposit.
    advance: Option<CartAdvanceResponse>,
    /// The most each other way to pay can cover for this cart, before checkout (e.g. the most loyalty
    /// points usable). Only ways the shop offers and that can tell without more input appear (a gift
    /// card needs its code). Pay with them through `tenders` at checkout.
    tender_limits: Vec<CartTenderLimitResponse>,
    attribution: Option<AttributionResponse>,
    totals: TotalsResponse,
    /// How much more to spend for free shipping (`0`: already free), when the option has a
    /// threshold.
    free_shipping_remaining: Option<Money>,
    /// Send it to `POST /checkout`: checkout refuses with `orders.quote_changed` when prices,
    /// discounts or fees changed since.
    quote_token: String,
    quote_expires_at: Timestamp,
    /// Guests only, once, when the cart is created: send it back in the `Cart-Token` header.
    #[serde(skip_serializing_if = "Option::is_none")]
    cart_token: Option<String>,
    updated_at: Timestamp,
}

/// The most a way to pay can cover for a cart.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersCartTenderLimit)]
pub(crate) struct CartTenderLimitResponse {
    /// The tender, as sent in `tenders[].kind` at checkout: `points`, `store_credit`…
    #[schema(example = "points")]
    kind: String,
    /// The most it can take off what is left to pay.
    max_amount: Money,
    /// The same limit in the tender's own units, when it has them: for `points`, the most points
    /// usable (send that number or less as `{"points": n}`).
    max_units: Option<i64>,
}

/// Advance payment terms of a cart with pre-ordered items.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersCartAdvance)]
pub(crate) struct CartAdvanceResponse {
    /// Paid when ordering: items in stock and the advance.
    upfront: Money,
    /// Paid when the pre-ordered items are ready (`total − upfront`).
    balance: Money,
    /// Hours the buyer has to pay the balance once asked.
    balance_due_hours: i64,
    /// How the balance is paid: `payment_link`.
    balance_methods: Vec<String>,
    /// Kept from the advance of a line when the buyer cancels after its cut-off (basis points;
    /// `0` = always refunded in full).
    retention_bp: i64,
    retention_max: Option<Money>,
    lines: Vec<CartAdvanceLineResponse>,
}

/// The advance of one pre-ordered line.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersCartAdvanceLine)]
pub(crate) struct CartAdvanceLineResponse {
    line_key: String,
    amount: Money,
    /// Cancelling after this may keep part of the advance; `null`: always refunded in full.
    commit_at: Option<Timestamp>,
}

impl CartAdvanceResponse {
    fn new(planned: &crate::advance_flow::Planned, quote: &crate::pricing::Quote) -> Self {
        let lines = quote
            .lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| {
                let amount = *planned.split.line_advance.get(index)?;
                (amount.minor() > 0).then(|| CartAdvanceLineResponse {
                    line_key: line.key.clone(),
                    amount,
                    commit_at: planned.commit_at.get(index).copied().flatten(),
                })
            })
            .collect();
        CartAdvanceResponse {
            upfront: planned.split.upfront,
            balance: planned.split.balance,
            balance_due_hours: planned.terms.balance_due_hours,
            balance_methods: vec![crate::advance::PAYMENT_LINK.to_owned()],
            retention_bp: planned.terms.retention_bp,
            retention_max: planned.terms.retention_max,
            lines,
        }
    }
}

impl CartResponse {
    pub(crate) fn new(view: CartView, cart_token: Option<String>) -> Self {
        let quote = &view.quote;
        let evaluation = quote.evaluation.as_ref();
        let options = quote
            .shipping_options
            .iter()
            .map(|option| ShippingOptionResponse {
                option_key: option.option_key.clone(),
                method_code: option.method_code.clone(),
                name: option.name.clone(),
                kind: option.kind.as_str().to_owned(),
                amount: option.amount,
                free_over: option.free_over,
                carrier_code: option.carrier_code.clone(),
                pickup_org_id: option.pickup_org_id.map(|org| org.as_uuid()),
                eta_earliest: option.eta.as_ref().map(|eta| eta.earliest_date),
                eta_latest: option.eta.as_ref().map(|eta| eta.latest_date),
            })
            .collect();
        CartResponse {
            id: view.cart.id,
            org_id: view.cart.org_id,
            status: view.cart.status.clone(),
            currency: view.cart.currency.clone(),
            buyer_user_id: view.cart.buyer_user_id,
            lines: quote.lines.iter().map(CartLineResponse::new).collect(),
            codes: view.codes,
            voucher_ids: view.voucher_ids,
            promotions: evaluation.map(|evaluation| evaluation.applied.clone()).unwrap_or_default(),
            rejected_codes: evaluation
                .map(|evaluation| evaluation.rejected_codes.clone())
                .unwrap_or_default(),
            shipping: ShippingResponse {
                required: quote.requires_shipping,
                options,
                selected: quote.shipping.as_ref().map(|option| option.option_key.clone()),
                selected_unavailable: quote.shipping_option_unavailable,
            },
            ship_to: view.ship_to.map(ShipToResponse::from),
            payment_method: view.cart.payment_method.clone(),
            payment_methods: view.payment_methods,
            advance: view.advance.as_ref().map(|planned| CartAdvanceResponse::new(planned, quote)),
            tender_limits: view
                .tender_limits
                .iter()
                .map(|(kind, limit)| CartTenderLimitResponse {
                    kind: (*kind).to_owned(),
                    max_amount: limit.amount,
                    max_units: limit.units,
                })
                .collect(),
            attribution: view.attribution.map(|attribution| AttributionResponse {
                click_id: attribution.click_id,
                code: attribution.code,
                live_session_id: attribution.live_session_id,
            }),
            totals: TotalsResponse {
                subtotal: quote.subtotal,
                discount: quote.discount,
                shipping_fee: quote.shipping_fee,
                shipping_discount: quote.shipping_discount,
                tax: quote.tax,
                total: quote.total,
            },
            free_shipping_remaining: quote.free_shipping_remaining,
            quote_token: quote.token.clone(),
            quote_expires_at: quote.expires_at,
            cart_token,
            updated_at: view.cart.updated_at,
        }
    }
}

/// A cart in a staff list (no quote).
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersCartSummary)]
pub(crate) struct CartSummaryResponse {
    id: Uuid,
    org_id: Uuid,
    status: String,
    currency: String,
    buyer_user_id: Option<Uuid>,
    /// When `orders.cart_abandoned` was sent.
    abandoned_notified_at: Option<Timestamp>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl From<crate::entity::cart::Model> for CartSummaryResponse {
    fn from(row: crate::entity::cart::Model) -> Self {
        CartSummaryResponse {
            id: row.id,
            org_id: row.org_id,
            status: row.status,
            currency: row.currency,
            buyer_user_id: row.buyer_user_id,
            abandoned_notified_at: row.abandoned_notified_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// A line of an order (as ordered).
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersOrderLine)]
pub(crate) struct OrderLineResponse {
    id: Uuid,
    variant_id: Uuid,
    product_id: Uuid,
    sku: String,
    name: String,
    quantity: i64,
    unit_price: Money,
    discount: Money,
    tax: Money,
    /// What the buyer pays for the line.
    total: Money,
    /// The promotion that added this free gift.
    gift_of: Option<Uuid>,
    returned_quantity: i64,
}

/// Money totals of an order.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersOrderTotals)]
pub(crate) struct OrderTotalsResponse {
    subtotal: Money,
    discount: Money,
    shipping_fee: Money,
    shipping_discount: Money,
    tax: Money,
    total: Money,
    paid: Money,
    refunded: Money,
}

/// An order.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersOrder)]
pub(crate) struct OrderResponse {
    id: Uuid,
    number: String,
    /// The shop (or seller) of the order.
    org_id: Uuid,
    channel: String,
    buyer_user_id: Option<Uuid>,
    /// `pending_payment`, `confirmed`, `preparing`, `shipping`, `delivered`, `delivery_failed`,
    /// `completed` or `cancelled`.
    status: String,
    /// `unpaid`, `cod_pending`, `paid`, `partially_refunded`, `refunded`…
    payment_status: String,
    /// `unfulfilled`, `partial`, `fulfilled`, `returned_partial`, `returned`.
    fulfillment_status: String,
    payment_method: String,
    /// `self_collect`, `partner_collect`, `split_at_gateway`, `channel_collect`.
    funds_flow: String,
    currency: String,
    totals: OrderTotalsResponse,
    /// Lines; empty in lists.
    lines: Vec<OrderLineResponse>,
    /// Pay before this, or the order is cancelled.
    payment_due_at: Option<Timestamp>,
    placed_at: Timestamp,
    confirmed_at: Option<Timestamp>,
    cancelled_at: Option<Timestamp>,
    cancel_reason: Option<String>,
    /// Changes on every update (for `If-Match`).
    version: i32,
    /// Collecting in store: the shop, and the pickup once the order is ready.
    pickup_org_id: Option<Uuid>,
    pickup: Option<PickupResponse>,
    /// Values of the shop's own fields (`custom_fields` given at checkout and staff-only fields).
    custom_fields: Option<serde_json::Map<String, serde_json::Value>>,
    /// Last status in which the buyer may still cancel (`pending_payment`, `confirmed` or
    /// `preparing`); omitted in lists.
    #[serde(skip_serializing_if = "Option::is_none")]
    buyer_cancel_until: Option<String>,
    /// Whether the buyer may cancel now; omitted in lists.
    #[serde(skip_serializing_if = "Option::is_none")]
    buyer_can_cancel: Option<bool>,
    /// Until when a return can be requested (delivered orders); omitted in lists.
    #[serde(skip_serializing_if = "Option::is_none")]
    return_until: Option<Timestamp>,
    /// Pre-ordered items paid in part; omitted in lists and for orders paid in full.
    #[serde(skip_serializing_if = "Option::is_none")]
    advance: Option<OrderAdvanceResponse>,
}

/// The advance payment plan of an order.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersOrderAdvance)]
pub(crate) struct OrderAdvanceResponse {
    advance: Money,
    upfront: Money,
    balance: Money,
    /// `awaiting_advance`, `advance_paid`, `balance_requested`, `balance_paid`, `balance_overdue`
    /// or `cancelled`.
    status: String,
    /// Pay the balance before this (once asked).
    balance_due_at: Option<Timestamp>,
}

/// An order waiting in store.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersPickup)]
pub(crate) struct PickupResponse {
    /// `ready`, `picked_up` or `expired`.
    status: String,
    /// The 6-digit code to show at the counter; only in your own order.
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,
    ready_at: Timestamp,
    /// Held until then.
    hold_until: Timestamp,
    handed_over_at: Option<Timestamp>,
}

impl OrderResponse {
    /// Đơn không kèm dòng (danh sách).
    pub(crate) fn summary(
        row: crate::entity::order::Model,
    ) -> Result<Self, autoxide_kernel::error::Error> {
        let money = |minor: i64| Money::from_columns(minor, &row.currency);
        Ok(OrderResponse {
            totals: OrderTotalsResponse {
                subtotal: money(row.subtotal_minor)?,
                discount: money(row.discount_minor)?,
                shipping_fee: money(row.shipping_fee_minor)?,
                shipping_discount: money(row.shipping_discount_minor)?,
                tax: money(row.tax_minor)?,
                total: money(row.total_minor)?,
                paid: money(row.paid_minor)?,
                refunded: money(row.refunded_minor)?,
            },
            id: row.id,
            number: row.number,
            org_id: row.org_id,
            channel: row.channel,
            buyer_user_id: row.buyer_user_id,
            status: row.status,
            payment_status: row.payment_status,
            fulfillment_status: row.fulfillment_status,
            payment_method: row.payment_method,
            funds_flow: row.funds_flow,
            currency: row.currency,
            lines: Vec::new(),
            payment_due_at: row.payment_due_at,
            placed_at: row.placed_at,
            confirmed_at: row.confirmed_at,
            cancelled_at: row.cancelled_at,
            cancel_reason: row.cancel_reason,
            version: row.version,
            pickup_org_id: row.pickup_org_id,
            pickup: None,
            custom_fields: row
                .custom_fields
                .as_deref()
                .and_then(|text| serde_json::from_str(text).ok()),
            buyer_cancel_until: None,
            buyer_can_cancel: None,
            return_until: None,
            advance: None,
        })
    }

    /// Đơn kèm dòng.
    pub(crate) fn new(
        view: crate::orders_read::OrderView,
    ) -> Result<Self, autoxide_kernel::error::Error> {
        let currency = view.order.currency.clone();
        let money = |minor: i64| Money::from_columns(minor, &currency);
        let lines = view
            .lines
            .iter()
            .map(|line| {
                Ok(OrderLineResponse {
                    id: line.id,
                    variant_id: line.variant_id,
                    product_id: line.product_id,
                    sku: line.sku.clone(),
                    name: line.name.clone(),
                    quantity: line.quantity,
                    unit_price: money(line.unit_price_minor)?,
                    discount: money(line.discount_minor)?,
                    tax: money(line.tax_minor)?,
                    total: money(line.total_minor)?,
                    gift_of: line.gift_of,
                    returned_quantity: line.returned_quantity,
                })
            })
            .collect::<Result<Vec<_>, autoxide_kernel::error::Error>>()?;
        let pickup = view.pickup.map(|pickup| PickupResponse {
            status: pickup.status,
            code: None,
            ready_at: pickup.ready_at,
            hold_until: pickup.hold_until,
            handed_over_at: pickup.handed_over_at,
        });
        let mut response = OrderResponse::summary(view.order)?;
        response.lines = lines;
        response.pickup = pickup;
        response.buyer_cancel_until = Some(view.deadlines.buyer_cancel_until.as_str().to_owned());
        response.buyer_can_cancel = Some(view.deadlines.buyer_can_cancel);
        response.return_until = view.deadlines.return_until;
        response.advance = match view.advance {
            Some(plan) => {
                let money = |minor: i64| Money::from_columns(minor, &plan.currency);
                Some(OrderAdvanceResponse {
                    advance: money(plan.advance_minor)?,
                    upfront: money(plan.upfront_minor)?,
                    balance: money(plan.balance_minor)?,
                    status: plan.status,
                    balance_due_at: plan.balance_due_at,
                })
            }
            None => None,
        };
        Ok(response)
    }

    /// Đơn của chính người mua: kèm mã nhận tại cửa hàng.
    pub(crate) fn for_buyer(
        view: crate::orders_read::OrderView,
    ) -> Result<Self, autoxide_kernel::error::Error> {
        let code = view.pickup.as_ref().map(|pickup| pickup.code.clone());
        let mut response = OrderResponse::new(view)?;
        if let Some(pickup) = response.pickup.as_mut() {
            pickup.code = code;
        }
        Ok(response)
    }
}

/// What the buyer does next to pay one amount.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersPaymentAction)]
pub(crate) struct PaymentActionResponse {
    order_id: Uuid,
    payment_id: Uuid,
    /// `redirect` (go to `redirect_url`), `qr` (show `qr_payload`), `cod` (pay on delivery) or
    /// `none` (the payment could not be started yet; retry from the order).
    kind: String,
    amount: Money,
    /// `pending`, `succeeded`, `failed`, `expired`…
    status: String,
    redirect_url: Option<String>,
    /// VietQR payload; the money goes to the seller's or the licensed partner's account.
    qr_payload: Option<String>,
    /// Transfer content the bank transfer must carry.
    transfer_code: Option<String>,
    expires_at: Option<Timestamp>,
    /// For `GET /public/payments/{token}`.
    status_token: String,
    /// Only with `kind = none`, when starting the payment failed: why (e.g. the payment partner
    /// refused). Fix the cause, then start the payment again from the order.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<PaymentStartError>,
}

/// Why a payment could not be started.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersPaymentStartError)]
pub(crate) struct PaymentStartError {
    /// Stable error code, as in a `Problem` (e.g. `momo.return_url_required`).
    code: String,
    /// What to fix, in the language of `Accept-Language`.
    message: String,
}

impl PaymentActionResponse {
    /// Việc của khoản `row`; lý do khởi tạo hỏng (nếu có) bằng `language` (bản dịch theo `code`,
    /// như `detail` của `Problem`).
    pub(crate) fn new(
        row: &crate::entity::payment::Model,
        status_token: String,
        translations: &autoxide_kernel::i18n::Translations,
        language: autoxide_kernel::i18n::Language,
    ) -> Result<Self, autoxide_kernel::error::Error> {
        let action = crate::payments::action_of(row);
        let text = |key: &str| action[key].as_str().map(str::to_owned);
        let kind = text("kind").unwrap_or_else(|| "none".to_owned());
        let error = action
            .get("error")
            .filter(|_| kind == "none")
            .and_then(|raw| {
                serde_json::from_value::<crate::payments::StartFailure>(raw.clone()).ok()
            })
            .map(|failure| {
                let error = failure.to_error();
                PaymentStartError {
                    code: failure.code,
                    message: translations.message(&error, language),
                }
            });
        Ok(PaymentActionResponse {
            order_id: row.order_id,
            payment_id: row.id,
            kind,
            amount: Money::from_columns(row.amount_minor, &row.currency)?,
            status: row.status.clone(),
            redirect_url: text("redirect_url"),
            qr_payload: text("qr_payload"),
            transfer_code: text("transfer_code"),
            expires_at: row.expires_at,
            status_token,
            error,
        })
    }
}

/// Result of a checkout.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersCheckoutResult)]
pub(crate) struct CheckoutResponse {
    pub orders: Vec<OrderResponse>,
    /// One per amount to pay (D-216).
    pub payment_actions: Vec<PaymentActionResponse>,
}

/// Public status of a payment.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersPublicPayment)]
pub(crate) struct PublicPaymentResponse {
    /// `pending`, `succeeded`, `failed`, `expired`.
    status: String,
    amount: Money,
    expires_at: Option<Timestamp>,
    order_number: String,
    order_status: String,
}

impl PublicPaymentResponse {
    pub(crate) fn new(
        payment: &crate::entity::payment::Model,
        order: &crate::entity::order::Model,
    ) -> Result<Self, autoxide_kernel::error::Error> {
        Ok(PublicPaymentResponse {
            status: payment.status.clone(),
            amount: Money::from_columns(payment.amount_minor, &payment.currency)?,
            expires_at: payment.expires_at,
            order_number: order.number.clone(),
            order_status: order.status.clone(),
        })
    }
}

/// A cart at the counter, priced (nothing is saved).
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersPosQuote)]
pub(crate) struct PosQuoteResponse {
    lines: Vec<CartLineResponse>,
    /// Promotions applied, with their terms.
    #[schema(value_type = Vec<Object>)]
    promotions: Vec<AppliedPromotion>,
    /// Codes that do not apply, with the reason; the sale goes on without them.
    #[schema(value_type = Vec<Object>)]
    rejected_codes: Vec<RejectedCode>,
    totals: TotalsResponse,
}

impl PosQuoteResponse {
    pub(crate) fn new(quote: &crate::pricing::Quote) -> Self {
        let evaluation = quote.evaluation.as_ref();
        PosQuoteResponse {
            lines: quote.lines.iter().map(CartLineResponse::new).collect(),
            promotions: evaluation.map(|evaluation| evaluation.applied.clone()).unwrap_or_default(),
            rejected_codes: evaluation
                .map(|evaluation| evaluation.rejected_codes.clone())
                .unwrap_or_default(),
            totals: TotalsResponse {
                subtotal: quote.subtotal,
                discount: quote.discount,
                shipping_fee: quote.shipping_fee,
                shipping_discount: quote.shipping_discount,
                tax: quote.tax,
                total: quote.total,
            },
        }
    }
}
