//! Tool MCP của module `inventory`. Mỗi tool: kiểu đầu vào kiểm như thân JSON của REST,
//! permission như endpoint REST tương ứng, thân là lời gọi service — không có logic riêng (luật
//! kiến trúc 8).

use autoxide_kernel::time::{Date, Timestamp};
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::mcp::{PageInput, Tool};
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::Page;
use autoxide_kernel::tenancy::TenantContext;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::http::documents::TransferResponse;
use crate::http::dto::{AdjustmentResponse, KindDto, LevelResponse, MovementResponse};
use crate::http::reservations::ReservationResponse;
use crate::levels::{AdjustInput, AdjustLine, LevelFilter, MovementFilter};
use crate::permissions::{ADJUST, READ, RECEIVE, TRANSFER};
use crate::service::InventoryService;
use crate::transfers::{NewTransfer, ReceiveLine, TransferLineInput};

/// Stock levels to list.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryGetLevelsInput)]
pub(crate) struct GetLevelsInput {
    /// Only this warehouse or store.
    pub org_id: Option<Uuid>,
    /// Only this variant.
    pub variant_id: Option<Uuid>,
    /// Maximum number of levels, 1 to 200. Defaults to 50.
    pub limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    pub cursor: Option<String>,
}

/// Ledger lines to list.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryListMovementsInput)]
pub(crate) struct ListMovementsInput {
    pub org_id: Option<Uuid>,
    pub variant_id: Option<Uuid>,
    #[schema(inline)]
    pub kind: Option<KindDto>,
    /// Lines written at or after this time.
    pub from: Option<Timestamp>,
    /// Lines written before this time.
    pub to: Option<Timestamp>,
    /// Maximum number of lines, 1 to 200. Defaults to 50.
    pub limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    pub cursor: Option<String>,
}

/// A stock adjustment in one warehouse.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryAdjustStockInput)]
pub(crate) struct AdjustStockInput {
    /// The warehouse or store.
    pub org_id: Uuid,
    /// Your id of this adjustment; the same reference and content again changes nothing.
    pub reference: String,
    /// 1 to 500 lines.
    #[schema(inline)]
    pub lines: Vec<AdjustStockLine>,
}

// Trường phẳng (client MCP không đọc `$ref`): giá vốn là số nguyên đơn vị nhỏ nhất dạng chuỗi.
/// One line of a stock adjustment.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryAdjustStockLine)]
pub(crate) struct AdjustStockLine {
    pub variant_id: Uuid,
    /// Units to add (positive) or remove (negative); not 0.
    pub quantity: i64,
    /// Code from the reason list `inventory.adjustment`, e.g. `damaged`.
    pub reason_code: String,
    pub note: Option<String>,
    /// Unit cost of added units, integer minor units as a string, e.g. `"150000"`.
    pub unit_cost_amount: Option<String>,
    /// Currency of `unit_cost_amount`, e.g. `VND`.
    pub unit_cost_currency: Option<String>,
    /// Lot of the units (required when adding units of a lot-tracked variant).
    pub lot_code: Option<String>,
    /// Expiry date of the lot, `YYYY-MM-DD`, when adding.
    pub expires_on: Option<Date>,
    /// Serial numbers when the variant tracks serials.
    #[serde(default)]
    pub serials: Vec<String>,
}

impl AdjustStockLine {
    fn into_line(self, index: usize) -> Result<AdjustLine, Error> {
        let unit_cost = match (self.unit_cost_amount, self.unit_cost_currency) {
            (None, None) => None,
            (Some(amount), Some(currency)) => Some(Money::parse(amount.trim(), currency.trim())?),
            _ => {
                return Err(Error::validation(vec![FieldError::new(
                    format!("lines[{index}].unit_cost_amount"),
                    "invalid",
                    "give both `unit_cost_amount` and `unit_cost_currency`, or neither",
                )]));
            }
        };
        Ok(AdjustLine {
            variant_id: self.variant_id,
            quantity: self.quantity,
            reason_code: Some(self.reason_code),
            note: self.note,
            unit_cost,
            lot_code: self.lot_code,
            expires_on: self.expires_on,
            serials: self.serials,
        })
    }
}

pub(crate) fn all(service: &Arc<InventoryService>) -> Vec<Tool> {
    // Mỗi tool giữ một bản `Arc` của service (clone `Arc` chỉ tăng bộ đếm tham chiếu).
    let s = || service.clone();
    let (levels, movements, adjust, low, release) = (s(), s(), s(), s(), s());
    let (create_transfer, receive_transfer, create_receipt) = (s(), s(), s());
    vec![
        Tool::new(
            "get_levels",
            "List stock levels (variant × warehouse) in your scope with on hand, available, \
             reserved and average cost; follow `pagination.next_cursor` for more.",
            move |tenant: TenantContext, principal: Principal, input: GetLevelsInput| {
                let service = levels.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None).page()?;
                    let filter = LevelFilter {
                        org_id: input.org_id,
                        variant_id: input.variant_id,
                        below_reorder: false,
                    };
                    let found = service.list_levels(&tenant, &principal, &filter, &page).await?;
                    Ok::<Page<LevelResponse>, Error>(found.map(LevelResponse::from))
                }
            },
        )
        .requires(READ)
        .read_only(),
        Tool::new(
            "list_low_stock",
            "List stock levels whose available stock is at or below their reorder point.",
            move |tenant: TenantContext, principal: Principal, input: GetLevelsInput| {
                let service = low.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None).page()?;
                    let filter = LevelFilter {
                        org_id: input.org_id,
                        variant_id: input.variant_id,
                        below_reorder: true,
                    };
                    let found = service.list_levels(&tenant, &principal, &filter, &page).await?;
                    Ok::<Page<LevelResponse>, Error>(found.map(LevelResponse::from))
                }
            },
        )
        .requires(READ)
        .read_only(),
        Tool::new(
            "list_movements",
            "List the append-only stock movement ledger in your scope, newest first.",
            move |tenant: TenantContext, principal: Principal, input: ListMovementsInput| {
                let service = movements.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None).page()?;
                    let filter = MovementFilter {
                        org_id: input.org_id,
                        variant_id: input.variant_id,
                        kind: input.kind.map(Into::into),
                        from: input.from,
                        to: input.to,
                    };
                    let found = service.list_movements(&tenant, &principal, &filter, &page).await?;
                    Ok::<Page<MovementResponse>, Error>(found.map(MovementResponse::from))
                }
            },
        )
        .requires(READ)
        .read_only(),
        Tool::new(
            "adjust_stock",
            "Adjust stock in one warehouse; every line needs a reason code from the list \
             `inventory.adjustment`. Sending the same `reference` and content again changes \
             nothing.",
            move |tenant: TenantContext, principal: Principal, input: AdjustStockInput| {
                let service = adjust.clone();
                async move {
                    let mut lines = Vec::with_capacity(input.lines.len());
                    for (index, line) in input.lines.into_iter().enumerate() {
                        lines.push(line.into_line(index)?);
                    }
                    let input =
                        AdjustInput { org_id: input.org_id, reference: input.reference, lines };
                    let done = service.adjust(&tenant, &principal, input).await?;
                    Ok::<_, Error>(AdjustmentResponse::from(done.into_inner()))
                }
            },
        )
        .requires(ADJUST),
        Tool::new(
            "release_reservation",
            "Release a stuck stock reservation by its id: units not shipped yet go back to the \
             available stock. Releasing again changes nothing.",
            move |tenant: TenantContext, principal: Principal, input: ReservationId| {
                let service = release.clone();
                async move {
                    let done = service.release_by_hand(&tenant, &principal, input.id).await?;
                    Ok::<_, Error>(ReservationResponse::from(done))
                }
            },
        )
        .requires(ADJUST)
        .destructive(),
        Tool::new(
            "create_transfer",
            "Create a stock transfer between two warehouses as a draft (one line per variant); \
             ship it with the REST endpoint `POST /api/v1/inventory/transfers/{id}/ship`.",
            move |tenant: TenantContext, principal: Principal, input: CreateTransferInput| {
                let service = create_transfer.clone();
                async move {
                    let input = NewTransfer {
                        from_org_id: input.from_org_id,
                        to_org_id: input.to_org_id,
                        reference: input.reference,
                        note: input.note,
                        lines: input
                            .lines
                            .into_iter()
                            .map(|line| TransferLineInput {
                                variant_id: line.variant_id,
                                quantity: line.quantity,
                                serials: line.serials,
                            })
                            .collect(),
                    };
                    let done = service.create_transfer(&tenant, &principal, input).await?;
                    Ok::<_, Error>(TransferResponse::from(done))
                }
            },
        )
        .requires(TRANSFER),
        Tool::new(
            "receive_transfer",
            "Receive goods of a stock transfer in transit, all (no lines) or part.",
            move |tenant: TenantContext, principal: Principal, input: ReceiveTransferInput| {
                let service = receive_transfer.clone();
                async move {
                    let lines = input
                        .lines
                        .into_iter()
                        .map(|line| ReceiveLine {
                            variant_id: line.variant_id,
                            quantity: line.quantity,
                        })
                        .collect();
                    let done =
                        service.receive_transfer(&tenant, &principal, input.id, lines).await?;
                    Ok::<_, Error>(TransferResponse::from(done))
                }
            },
        )
        .requires(TRANSFER),
        Tool::new(
            "create_receipt",
            "Receive goods into a warehouse, against a purchase order or directly from a \
             supplier; each line updates the average cost. The same `reference` and content \
             again changes nothing.",
            move |tenant: TenantContext, principal: Principal, input: CreateReceiptInput| {
                let service = create_receipt.clone();
                async move {
                    let mut lines = Vec::with_capacity(input.lines.len());
                    for line in input.lines {
                        lines.push(crate::receipts::ReceiptLineInput {
                            variant_id: line.variant_id,
                            quantity: line.quantity,
                            unit_cost: Money::parse(
                                line.unit_cost_amount.trim(),
                                line.unit_cost_currency.trim(),
                            )?,
                            lot_code: line.lot_code,
                            expires_on: line.expires_on,
                            serials: line.serials,
                        });
                    }
                    let input = crate::receipts::ReceiptInput {
                        org_id: input.org_id,
                        supplier_id: input.supplier_id,
                        purchase_order_id: input.purchase_order_id,
                        reference: input.reference,
                        lines,
                    };
                    let done = service.create_receipt(&tenant, &principal, input).await?;
                    crate::http::purchasing::ReceiptResponse::try_from(done.into_inner())
                }
            },
        )
        .requires(RECEIVE),
    ]
}

/// A goods receipt.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryCreateReceiptInput)]
pub(crate) struct CreateReceiptInput {
    /// The receiving warehouse.
    pub org_id: Uuid,
    pub supplier_id: Option<Uuid>,
    pub purchase_order_id: Option<Uuid>,
    /// Your id of this receipt, e.g. the delivery note number.
    pub reference: String,
    #[schema(inline)]
    pub lines: Vec<ToolReceiptLine>,
}

// Trường phẳng (client MCP không đọc `$ref`): giá là số nguyên đơn vị nhỏ nhất dạng chuỗi.
/// One line of a goods receipt.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryToolReceiptLine)]
pub(crate) struct ToolReceiptLine {
    pub variant_id: Uuid,
    pub quantity: i64,
    /// Unit cost in minor units as a string, e.g. `"150000"`.
    pub unit_cost_amount: String,
    /// e.g. `VND`.
    pub unit_cost_currency: String,
    pub lot_code: Option<String>,
    /// `YYYY-MM-DD`.
    pub expires_on: Option<Date>,
    #[serde(default)]
    pub serials: Vec<String>,
}

/// A variant and a quantity.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryToolQuantityLine)]
pub(crate) struct ToolQuantityLine {
    pub variant_id: Uuid,
    /// 1 to 1 000 000.
    pub quantity: i64,
}

/// A stock transfer to create.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryCreateTransferInput)]
pub(crate) struct CreateTransferInput {
    pub from_org_id: Uuid,
    pub to_org_id: Uuid,
    pub reference: Option<String>,
    pub note: Option<String>,
    #[schema(inline)]
    pub lines: Vec<ToolTransferLine>,
}

/// One line of a transfer to create.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryToolTransferLine)]
pub(crate) struct ToolTransferLine {
    pub variant_id: Uuid,
    /// 1 to 1 000 000.
    pub quantity: i64,
    /// Exactly `quantity` serial numbers when the variant tracks serials.
    #[serde(default)]
    pub serials: Vec<String>,
}

/// Goods of a transfer to receive.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryReceiveTransferInput)]
pub(crate) struct ReceiveTransferInput {
    /// Id of the transfer.
    pub id: Uuid,
    /// Units received now; empty or omitted: everything still in transit.
    #[serde(default)]
    #[schema(inline)]
    pub lines: Vec<ToolQuantityLine>,
}

/// A reservation to act on.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryReservationId)]
pub(crate) struct ReservationId {
    /// Id of the reservation.
    pub id: Uuid,
}
