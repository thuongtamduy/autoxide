//! Kiểu dữ liệu của REST (và tool MCP dùng lại). Doc comment `///` thành mô tả OpenAPI: viết cho
//! người gọi API, bằng tiếng Anh.

use autoxide_kernel::money::Money;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::entity::{level, movement};
use crate::levels::{AdjustLine, Adjustment};
use crate::stock::MovementKind;

/// The stock of one variant in one warehouse or store.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = InventoryLevel)]
pub(crate) struct LevelResponse {
    pub id: Uuid,
    /// The warehouse or store (organization unit).
    pub org_id: Uuid,
    pub variant_id: Uuid,
    /// Units physically in the warehouse: the sum of the movement ledger.
    pub on_hand: i64,
    /// Units that can still be sold: `on_hand` minus `reserved`. Negative when a product that
    /// may be sold out of stock is oversold.
    pub available: i64,
    /// Units held for orders not shipped yet.
    pub reserved: i64,
    /// Units on the way (transfers in transit, purchase orders).
    pub incoming: i64,
    /// `inventory.low_stock` fires when `available` falls to this or below; `null`: never.
    pub reorder_point: Option<i64>,
    /// Weighted average unit cost of the stock; `null` until goods with a cost come in.
    pub average_cost: Option<Money>,
    pub updated_at: DateTime<Utc>,
}

impl From<level::Model> for LevelResponse {
    fn from(row: level::Model) -> Self {
        let average_cost = row
            .average_cost_currency
            .as_deref()
            .and_then(|currency| Money::from_columns(row.average_cost_minor, currency).ok());
        LevelResponse {
            id: row.id,
            org_id: row.org_id,
            variant_id: row.variant_id,
            on_hand: row.on_hand,
            available: row.available,
            reserved: row.on_hand - row.available,
            incoming: row.incoming,
            reorder_point: row.reorder_point,
            average_cost,
            updated_at: row.updated_at,
        }
    }
}

/// Kind of a movement ledger line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = InventoryMovementKind)]
pub(crate) enum KindDto {
    Receipt,
    Sale,
    Return,
    Adjustment,
    Count,
    TransferOut,
    TransferIn,
    Expiry,
    SupplierReturn,
}

impl From<KindDto> for MovementKind {
    fn from(kind: KindDto) -> Self {
        match kind {
            KindDto::Receipt => MovementKind::Receipt,
            KindDto::Sale => MovementKind::Sale,
            KindDto::Return => MovementKind::Return,
            KindDto::Adjustment => MovementKind::Adjustment,
            KindDto::Count => MovementKind::Count,
            KindDto::TransferOut => MovementKind::TransferOut,
            KindDto::TransferIn => MovementKind::TransferIn,
            KindDto::Expiry => MovementKind::Expiry,
            KindDto::SupplierReturn => MovementKind::SupplierReturn,
        }
    }
}

/// One line of the stock movement ledger. Lines are only ever added.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = InventoryMovement)]
pub(crate) struct MovementResponse {
    pub id: Uuid,
    pub level_id: Uuid,
    pub org_id: Uuid,
    pub variant_id: Uuid,
    /// `receipt`, `sale`, `return`, `adjustment`, `count`, `transfer_out`, `transfer_in`,
    /// `expiry` or `supplier_return`.
    pub kind: String,
    /// Signed: positive came in, negative went out.
    pub quantity: i64,
    pub unit_cost: Option<Money>,
    pub lot_id: Option<Uuid>,
    /// Code from the reason list `inventory.adjustment`, for adjustments and counts.
    pub reason_code: Option<String>,
    pub note: String,
    /// What caused the line: `adjustment`, `reservation`, `transfer`, `receipt`…
    pub source_type: String,
    pub source_id: Uuid,
    pub actor_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

impl From<movement::Model> for MovementResponse {
    fn from(row: movement::Model) -> Self {
        let unit_cost = match (row.unit_cost_minor, row.unit_cost_currency.as_deref()) {
            (Some(minor), Some(currency)) => Money::from_columns(minor, currency).ok(),
            _ => None,
        };
        MovementResponse {
            id: row.id,
            level_id: row.level_id,
            org_id: row.org_id,
            variant_id: row.variant_id,
            kind: row.kind,
            quantity: row.quantity,
            unit_cost,
            lot_id: row.lot_id,
            reason_code: row.reason_code,
            note: row.note,
            source_type: row.source_type,
            source_id: row.source_id,
            actor_id: row.actor_id,
            created_at: row.created_at,
        }
    }
}

/// A stock adjustment to make in one warehouse.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryAdjustmentRequest)]
pub(crate) struct AdjustmentRequest {
    /// The warehouse or store.
    pub org_id: Uuid,
    /// Your id of this adjustment (1 to 100 visible ASCII characters). Sending the same
    /// reference and content again returns the adjustment already made.
    #[schema(example = "ADJ-2026-0042")]
    pub reference: String,
    /// 1 to 500 lines.
    pub lines: Vec<AdjustmentLineRequest>,
}

/// One line of a stock adjustment.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryAdjustmentLine)]
pub(crate) struct AdjustmentLineRequest {
    pub variant_id: Uuid,
    /// Units to add (positive) or remove (negative); not 0.
    pub quantity: i64,
    /// Code from the reason list `inventory.adjustment` (see
    /// `GET /api/v1/kernel/reasons/inventory.adjustment`), e.g. `damaged`.
    pub reason_code: Option<String>,
    /// Up to 500 characters.
    pub note: Option<String>,
    /// Unit cost of the units added (e.g. opening stock); updates the average cost. Only with a
    /// positive quantity.
    pub unit_cost: Option<Money>,
    /// Lot of the units: required when adding units of a variant that tracks lots; when removing,
    /// omit it to take the lots that expire first.
    pub lot_code: Option<String>,
    /// Expiry date of the lot, when adding.
    pub expires_on: Option<chrono::NaiveDate>,
    /// Exactly `|quantity|` serial numbers when the variant tracks serials.
    #[serde(default)]
    pub serials: Vec<String>,
}

impl From<AdjustmentLineRequest> for AdjustLine {
    fn from(line: AdjustmentLineRequest) -> Self {
        AdjustLine {
            variant_id: line.variant_id,
            quantity: line.quantity,
            reason_code: line.reason_code,
            note: line.note,
            unit_cost: line.unit_cost,
            lot_code: line.lot_code,
            expires_on: line.expires_on,
            serials: line.serials,
        }
    }
}

/// A stock adjustment with the ledger lines it wrote.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = InventoryAdjustment)]
pub(crate) struct AdjustmentResponse {
    pub id: Uuid,
    pub org_id: Uuid,
    pub reference: String,
    pub actor_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub movements: Vec<MovementResponse>,
}

impl From<Adjustment> for AdjustmentResponse {
    fn from(adjustment: Adjustment) -> Self {
        AdjustmentResponse {
            id: adjustment.row.id,
            org_id: adjustment.row.org_id,
            reference: adjustment.row.reference,
            actor_id: adjustment.row.actor_id,
            created_at: adjustment.row.created_at,
            movements: adjustment.movements.into_iter().map(MovementResponse::from).collect(),
        }
    }
}
