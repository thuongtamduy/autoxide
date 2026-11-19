//! `POST /quote`: báo giá cho nhân viên và agent — cùng hàm `ShippingRates::quote` mà `orders` gọi.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::money::Money;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use axum::extract::State;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use super::addresses::PostalAddressRequest;
use crate::api::{Address, MethodKind, Quote, QuoteItem, QuoteRequest, ShippingOption};
use crate::errors;
use crate::permissions::READ;
use crate::service::ShippingService;

/// One line of the cart, with the figures of the variant (from the catalog).
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[schema(as = ShippingQuoteItem)]
#[serde(deny_unknown_fields)]
pub(crate) struct ItemBody {
    pub(crate) variant_id: Uuid,
    #[serde(default)]
    pub(crate) sku: String,
    pub(crate) quantity: i64,
    /// `false` for digital goods and services: not weighed, not shipped.
    #[serde(default = "yes")]
    pub(crate) requires_shipping: bool,
    /// Weight of one item, grams.
    pub(crate) weight_grams: Option<i64>,
    pub(crate) length_mm: Option<i64>,
    pub(crate) width_mm: Option<i64>,
    pub(crate) height_mm: Option<i64>,
}

fn yes() -> bool {
    true
}

/// A cart to quote.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = ShippingQuoteRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct QuoteBody {
    /// The selling store; methods of the units above it apply too.
    pub(crate) org_id: Uuid,
    /// The warehouse shipping, when known.
    pub(crate) origin_org_id: Option<Uuid>,
    /// Where to deliver; leave out for store pickup only.
    #[schema(inline)]
    pub(crate) destination: Option<PostalAddressRequest>,
    #[schema(inline)]
    pub(crate) items: Vec<ItemBody>,
    /// Order value after discounts, for free shipping.
    pub(crate) order_value: Money,
    /// Time of the order (default: now).
    pub(crate) at: Option<DateTime<Utc>>,
    /// Language of method names (default `en`).
    pub(crate) locale: Option<String>,
}

/// Expected delivery dates.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ShippingEta)]
pub(crate) struct EtaBody {
    earliest_date: NaiveDate,
    latest_date: NaiveDate,
    /// Order before this time to make today's dispatch.
    order_by: Option<DateTime<Utc>>,
    time_zone: String,
}

/// A way to ship the cart.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ShippingOption)]
pub(crate) struct OptionBody {
    method_id: Uuid,
    method_code: String,
    name: String,
    kind: MethodKind,
    /// Fee the buyer pays, VAT included; zero when free.
    amount: Money,
    free_shipping_applied: bool,
    /// Free from this order value.
    free_over: Option<Money>,
    /// Tax category of the fee (VAT line on the invoice).
    tax_category: String,
    carrier_code: Option<String>,
    service_code: Option<String>,
    /// Store pickup: the store.
    pickup_org_id: Option<Uuid>,
    chargeable_weight_grams: i64,
    /// Days, as declared by the method: `[min, max]`.
    estimated_days: Option<[u16; 2]>,
    eta: Option<EtaBody>,
    /// Store pickup: when the order is ready.
    ready_at: Option<DateTime<Utc>>,
    /// Store pickup: days the parcel waits.
    hold_days: Option<u16>,
    /// Send back when placing the order; a different fee or destination answers
    /// `shipping.option_changed`.
    option_key: String,
}

impl From<ShippingOption> for OptionBody {
    fn from(option: ShippingOption) -> Self {
        OptionBody {
            method_id: option.method_id,
            method_code: option.method_code,
            name: option.name,
            kind: option.kind,
            amount: option.amount,
            free_shipping_applied: option.free_shipping_applied,
            free_over: option.free_over,
            tax_category: option.tax_category,
            carrier_code: option.carrier_code,
            service_code: option.service_code,
            pickup_org_id: option.pickup_org_id.map(|org| org.as_uuid()),
            chargeable_weight_grams: option.chargeable_weight_grams,
            estimated_days: option.estimated_days.map(|(min, max)| [min, max]),
            eta: option.eta.map(|eta| EtaBody {
                earliest_date: eta.earliest_date,
                latest_date: eta.latest_date,
                order_by: eta.order_by,
                time_zone: eta.time_zone,
            }),
            ready_at: option.ready_at,
            hold_days: option.hold_days,
            option_key: option.option_key,
        }
    }
}

/// A method that cannot ship this cart, and why.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ShippingUnavailableMethod)]
pub(crate) struct UnavailableBody {
    method_code: String,
    /// `shipping.weight_missing`, `shipping.no_rate`, `shipping.country_not_served`,
    /// `shipping.no_pickup_point`, `shipping.carrier_unavailable`…
    reason_code: String,
}

/// Shipping options of a cart.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ShippingQuote)]
pub(crate) struct QuoteResponse {
    /// Cheapest first; empty when nothing in the cart needs shipping.
    options: Vec<OptionBody>,
    unavailable: Vec<UnavailableBody>,
}

impl From<Quote> for QuoteResponse {
    fn from(quote: Quote) -> Self {
        QuoteResponse {
            options: quote.options.into_iter().map(OptionBody::from).collect(),
            unavailable: quote
                .unavailable
                .into_iter()
                .map(|item| UnavailableBody {
                    method_code: item.method_code,
                    reason_code: item.reason_code,
                })
                .collect(),
        }
    }
}

/// Dựng `QuoteRequest` từ thân; kiểm địa chỉ đích.
pub(crate) async fn request_of(
    service: &ShippingService,
    body: QuoteBody,
) -> Result<QuoteRequest, Error> {
    let items = body
        .items
        .into_iter()
        .map(|item| {
            QuoteItem::new(item.variant_id, item.quantity)
                .sku(item.sku)
                .requires_shipping(item.requires_shipping)
                .weight(item.weight_grams)
                .dimensions(item.length_mm, item.width_mm, item.height_mm)
        })
        .collect();
    let mut request = QuoteRequest::new(
        OrgId::from_uuid(body.org_id),
        items,
        body.order_value,
        body.at.unwrap_or_else(Utc::now),
    )
    .locale(body.locale.unwrap_or_else(|| "en".to_owned()));
    if let Some(origin) = body.origin_org_id {
        request = request.origin(OrgId::from_uuid(origin));
    }
    if let Some(destination) = body.destination {
        let checked: Address = service
            .check_address(&destination.into(), "destination.")
            .await
            .map_err(errors::invalid_address)?
            .address;
        request = request.destination(checked);
    }
    Ok(request)
}

/// Quote shipping options for a cart.
///
/// Needs `shipping.read` at the store. The same computation orders use at checkout: inherited
/// methods, zones by province, weight tiers with volumetric weight, free shipping, store pickup.
/// Methods that cannot ship the cart are listed in `unavailable` with a reason.
#[utoipa::path(
    post,
    path = "/quote",
    extensions(("x-permissions" = json!(["shipping.read"]))),
    operation_id = "quote_shipping",
    security(("bearer" = [])),
    request_body = QuoteBody,
    responses(
        (status = 200, description = "The options.", body = Data<QuoteResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.read` at the store; codes `permission_denied`, `org_out_of_scope`.", body = Problem),
        (status = 422, description = "The destination is not valid (`shipping.invalid_address`) or another value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn quote(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(body): ValidJson<QuoteBody>,
) -> Result<Data<QuoteResponse>, Error> {
    principal.require(&READ)?;
    let request = request_of(&service, body).await?;
    let quote = service.quote_for(&tenant, &principal, &request).await?;
    Ok(Data { data: quote.into() })
}
