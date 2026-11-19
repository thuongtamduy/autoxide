//! Kiểu trả về của REST (`ToSchema`, chép vào OpenAPI). Doc comment `///` ở đây viết tiếng Anh cho
//! người gọi API.

use autoxide_kernel::money::Money;
use autoxide_promotions::api::{AppliedPromotion, RejectedCode};
use chrono::{DateTime, NaiveDate, Utc};
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
    available_on: Option<NaiveDate>,
    /// Pre-orders close at.
    ends_at: Option<DateTime<Utc>>,
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
    fn new(line: &QuotedLine) -> Self {
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
    eta_earliest: Option<NaiveDate>,
    eta_latest: Option<NaiveDate>,
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
    attribution: Option<AttributionResponse>,
    totals: TotalsResponse,
    /// How much more to spend for free shipping (`0`: already free), when the option has a
    /// threshold.
    free_shipping_remaining: Option<Money>,
    /// Send it to `POST /checkout`: checkout refuses with `orders.quote_changed` when prices,
    /// discounts or fees changed since.
    quote_token: String,
    quote_expires_at: DateTime<Utc>,
    /// Guests only, once, when the cart is created: send it back in the `Cart-Token` header.
    #[serde(skip_serializing_if = "Option::is_none")]
    cart_token: Option<String>,
    updated_at: DateTime<Utc>,
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
    abandoned_notified_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
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
