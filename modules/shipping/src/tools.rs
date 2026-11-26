//! Tool MCP của module `shipping`. Mỗi tool: đầu vào như thân JSON của REST, permission như
//! endpoint REST tương ứng, thân là lời gọi service — không có logic riêng (luật kiến trúc 8).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::mcp::{PageInput, Tool};
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::Page;
use autoxide_kernel::tenancy::TenantContext;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::api::ShipmentStatus;
use crate::http::addresses::PostalAddressRequest;
use crate::http::cod::{DiscrepancyBody, discrepancy};
use crate::http::quote::{ItemBody, QuoteBody, QuoteResponse, request_of};
use crate::http::shipments::{SummaryBody, detail, summary};
use crate::permissions::{
    COD_MANAGE, COD_READ, READ, SHIPMENTS_MANAGE, SHIPMENTS_READ, require_cod_read,
};
use crate::service::ShippingService;
use crate::shipments::ShipmentFilter;

/// A cart to quote (amounts in minor units of `currency`).
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ShippingQuoteInput)]
pub(crate) struct QuoteInput {
    /// The selling store; methods of the units above it apply too.
    pub org_id: Uuid,
    pub origin_org_id: Option<Uuid>,
    /// Where to deliver; leave out for store pickup only.
    #[schema(inline)]
    pub destination: Option<PostalAddressRequest>,
    #[schema(inline)]
    pub items: Vec<ItemBody>,
    /// Order value after discounts, in minor units (e.g. `"250000"` dong), for free shipping.
    pub order_value: String,
    /// ISO 4217 code, default `VND`.
    pub currency: Option<String>,
    /// Language of method names (default `en`).
    pub locale: Option<String>,
}

/// Shipments to list.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ShippingListShipmentsInput)]
pub(crate) struct ListShipmentsInput {
    pub order_id: Option<Uuid>,
    #[schema(inline)]
    pub status: Option<ShipmentStatus>,
    pub org_id: Option<Uuid>,
    /// Tracking number of the carrier.
    pub tracking: Option<String>,
    /// Maximum number of shipments, 1 to 200. Defaults to 50.
    pub limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    pub cursor: Option<String>,
    /// `true` to also get `pagination.total` (the number of matching items) on the first page.
    count: Option<bool>,
}

/// A shipment to act on.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ShippingShipmentId)]
pub(crate) struct ShipmentId {
    /// Id of the shipment.
    pub id: Uuid,
}

/// A status set by hand.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ShippingUpdateStatusInput)]
pub(crate) struct UpdateStatusInput {
    pub id: Uuid,
    #[schema(inline)]
    pub status: ShipmentStatus,
    #[serde(default)]
    pub note: String,
}

/// Discrepancies to list.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ShippingListDiscrepanciesInput)]
pub(crate) struct ListDiscrepanciesInput {
    /// `open` or `resolved`.
    pub status: Option<String>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
    /// `true` to also get `pagination.total` (the number of matching items) on the first page.
    count: Option<bool>,
}

pub(crate) fn all(service: &Arc<ShippingService>) -> Vec<Tool> {
    // Mỗi tool giữ một bản `Arc` của service (clone `Arc` chỉ tăng bộ đếm tham chiếu).
    let s = || service.clone();
    let (quote, list, get, book, cancel, status, cod) = (s(), s(), s(), s(), s(), s(), s());
    vec![
        Tool::new(
            "quote",
            "Quote the shipping options of a cart from a store to a destination, cheapest first; \
             methods that cannot ship it are listed in `unavailable` with a reason.",
            move |tenant: TenantContext, principal: Principal, input: QuoteInput| {
                let service = quote.clone();
                async move {
                    let currency = input.currency.as_deref().unwrap_or("VND");
                    let body = QuoteBody {
                        org_id: input.org_id,
                        origin_org_id: input.origin_org_id,
                        destination: input.destination,
                        items: input.items,
                        order_value: Money::parse(&input.order_value, currency)?,
                        at: None,
                        locale: input.locale,
                    };
                    let request = request_of(&service, body).await?;
                    let quote = service.quote_for(&tenant, &principal, &request).await?;
                    Ok::<_, Error>(QuoteResponse::from(quote))
                }
            },
        )
        .requires(READ)
        .read_only(),
        Tool::new(
            "list_shipments",
            "List the shipments in your scope, newest first; filter by order, status, unit or \
             tracking number; follow `pagination.next_cursor` for more.",
            move |tenant: TenantContext, principal: Principal, input: ListShipmentsInput| {
                let service = list.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None)
                        .with_count(input.count.unwrap_or(false))
                        .page()?;
                    let filter = ShipmentFilter {
                        order_id: input.order_id,
                        status: input.status,
                        org_id: input.org_id,
                        tracking: input.tracking,
                    };
                    let found = service.list_shipments(&tenant, &principal, filter, &page).await?;
                    let mut data = Vec::with_capacity(found.data.len());
                    for row in found.data {
                        data.push(summary(row)?);
                    }
                    Ok::<Page<SummaryBody>, Error>(Page { data, pagination: found.pagination })
                }
            },
        )
        .requires(SHIPMENTS_READ)
        .read_only(),
        Tool::new(
            "get_shipment",
            "Get a shipment with its parcels, items and tracking history.",
            move |tenant: TenantContext, principal: Principal, input: ShipmentId| {
                let service = get.clone();
                async move {
                    let view = service.get_shipment(&tenant, &principal, input.id).await?;
                    detail(view)
                }
            },
        )
        .requires(SHIPMENTS_READ)
        .read_only(),
        Tool::new(
            "book_shipment",
            "Book a pending shipment with the carrier of its method; it gets a tracking number.",
            move |tenant: TenantContext, principal: Principal, input: ShipmentId| {
                let service = book.clone();
                async move {
                    let view = service.book_by_staff(&tenant, &principal, input.id).await?;
                    detail(view)
                }
            },
        )
        .requires(SHIPMENTS_MANAGE),
        Tool::new(
            "cancel_shipment",
            "Cancel a shipment (also at the carrier); refused once the carrier picked it up.",
            move |tenant: TenantContext, principal: Principal, input: ShipmentId| {
                let service = cancel.clone();
                async move {
                    let view = service.cancel_by_staff(&tenant, &principal, input.id).await?;
                    detail(view)
                }
            },
        )
        .requires(SHIPMENTS_MANAGE)
        .destructive(),
        Tool::new(
            "update_status",
            "Set the status of a shipment by hand (own delivery staff, store pickup); only moves \
             allowed by the status table.",
            move |tenant: TenantContext, principal: Principal, input: UpdateStatusInput| {
                let service = status.clone();
                async move {
                    let note: String = input.note.chars().take(1000).collect();
                    let view = service
                        .staff_status(&tenant, &principal, input.id, input.status, note)
                        .await?;
                    detail(view)
                }
            },
        )
        .requires(SHIPMENTS_MANAGE),
        Tool::new(
            "list_cod_discrepancies",
            "List the differences between carrier cash-on-delivery statements and shipments.",
            move |tenant: TenantContext, principal: Principal, input: ListDiscrepanciesInput| {
                let service = cod.clone();
                async move {
                    require_cod_read(&principal)?;
                    let page = PageInput::new(input.limit, input.cursor, None)
                        .with_count(input.count.unwrap_or(false))
                        .page()?;
                    let found = service.list_discrepancies(&tenant, input.status, &page).await?;
                    Ok::<Page<DiscrepancyBody>, Error>(found.map(discrepancy))
                }
            },
        )
        .requires(COD_READ)
        .or_requires(COD_MANAGE)
        .read_only(),
    ]
}
