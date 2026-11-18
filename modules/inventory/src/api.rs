//! Hợp đồng của module `inventory` cho module khác (`orders`, `catalog`, `channels`, báo
//! cáo…): chỉ những gì ở đây được dùng từ bên ngoài (`autoxide_inventory::api::…`). Module khác
//! lấy trait lúc khởi động bằng `context.require::<dyn …>()` và phải đăng ký SAU `inventory`
//! trong `crates/app/src/modules.rs`. Không module nào đọc bảng `inventory_…` (luật kiến trúc 1).
//! File này không tham chiếu phần bên trong module.

use autoxide_kernel::error::Error;
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Số ID tối đa mỗi lần gọi một hàm nhận danh sách ID.
pub const MAX_IDS: usize = 500;

/// Tên event `inventory` phát mà module khác hay nghe (payload `{"before": …, "after": …}`).
pub mod events {
    /// Tồn của một dòng (biến thể × kho) đổi. `after`: `level_id`, `org_id`, `variant_id`,
    /// `on_hand`, `available`, `reserved`; `before`: `on_hand`, `available`. Nhiều thay đổi của
    /// một dòng trong một transaction gộp thành một event.
    pub const LEVEL_CHANGED: &str = "inventory.level_changed";
    /// Tồn bán được đi từ trên xuống ≤ điểm đặt hàng lại (`reorder_point`).
    pub const LOW_STOCK: &str = "inventory.low_stock";
    /// Tồn bán được đi từ trên 0 xuống ≤ 0.
    pub const OUT_OF_STOCK: &str = "inventory.out_of_stock";
    /// Một dòng của sổ biến động được ghi.
    pub const MOVEMENT_RECORDED: &str = "inventory.movement_recorded";
    /// Giá vốn bình quân của biến thể ở một kho đổi. `after`: `org_id`, `variant_id`,
    /// `unit_cost` (dạng JSON của `Money`) — `catalog` nghe để có giá vốn cho sàn giá (D-222).
    pub const COST_CHANGED: &str = "inventory.cost_changed";
    /// Hàng được giữ cho một đơn (`after`: chỗ giữ).
    pub const RESERVATION_HELD: &str = "inventory.reservation_held";
    /// Chỗ giữ được xác nhận (đơn đã đặt).
    pub const RESERVATION_CONFIRMED: &str = "inventory.reservation_confirmed";
    /// Chỗ giữ được trả (`after.reason`: `cancelled`, `payment_failed`, `checkout_aborted`, `manual`).
    pub const RESERVATION_RELEASED: &str = "inventory.reservation_released";
    /// Chỗ giữ hết hạn khi chưa xác nhận.
    pub const RESERVATION_EXPIRED: &str = "inventory.reservation_expired";
}

/// Một dòng của đơn cần giữ hàng.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ReserveLine {
    /// Mã dòng của đơn (duy nhất trong đơn, 1–100 ký tự): xuất kho và trả lời lỗi theo mã này.
    pub line_key: String,
    pub variant_id: Uuid,
    /// 1…1 000 000 đơn vị (combo: số combo).
    pub quantity: i64,
}

impl ReserveLine {
    pub fn new(line_key: impl Into<String>, variant_id: Uuid, quantity: i64) -> Self {
        ReserveLine { line_key: line_key.into(), variant_id, quantity }
    }
}

/// Lệnh giữ hàng cho cả một đơn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ReserveRequest {
    /// Mã tham chiếu: một đơn giữ một lần.
    pub order_id: Uuid,
    /// Cửa hàng / kênh bán: kho phục vụ nó là ứng viên.
    pub selling_org_id: OrgId,
    /// Nhận tại cửa hàng, bán tại quầy: chỉ lấy từ kho này.
    pub fixed_org_id: Option<OrgId>,
    /// Mã tỉnh (module `address`) của nơi giao, để ưu tiên kho cùng tỉnh.
    pub ship_to_province: Option<String>,
    /// Hạn thanh toán do `orders` đặt (≤ `AUTOXIDE_INVENTORY_MAX_HOLD_HOURS` từ bây giờ).
    pub expires_at: DateTime<Utc>,
    pub lines: Vec<ReserveLine>,
}

impl ReserveRequest {
    pub fn new(
        order_id: Uuid,
        selling_org_id: OrgId,
        expires_at: DateTime<Utc>,
        lines: Vec<ReserveLine>,
    ) -> Self {
        ReserveRequest {
            order_id,
            selling_org_id,
            fixed_org_id: None,
            ship_to_province: None,
            expires_at,
            lines,
        }
    }

    /// Chỉ lấy hàng từ kho `org` (nhận tại cửa hàng, bán tại quầy).
    pub fn from_warehouse(mut self, org: OrgId) -> Self {
        self.fixed_org_id = Some(org);
        self
    }

    /// Giao tới tỉnh `province` (mã của `address`): kho cùng tỉnh trước khi cửa hàng bật ưu tiên.
    pub fn ship_to(mut self, province: impl Into<String>) -> Self {
        self.ship_to_province = Some(province.into());
        self
    }
}

/// Trạng thái của chỗ giữ hàng.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReservationStatus {
    /// Đang giữ, có hạn (`expires_at`).
    Held,
    /// Đơn đã đặt: giữ tới khi xuất hay huỷ, không hạn.
    Confirmed,
    /// Mọi dòng đã xuất (hay phần còn lại đã trả sau khi xuất một phần).
    Fulfilled,
    /// Đã trả lại (huỷ, thanh toán lỗi…) trước khi xuất gì.
    Released,
    /// Hết hạn khi còn `Held`: hàng đã về tồn bán được.
    Expired,
}

impl ReservationStatus {
    /// Tên ổn định: `held`, `confirmed`, `fulfilled`, `released`, `expired`.
    pub const fn as_str(self) -> &'static str {
        match self {
            ReservationStatus::Held => "held",
            ReservationStatus::Confirmed => "confirmed",
            ReservationStatus::Fulfilled => "fulfilled",
            ReservationStatus::Released => "released",
            ReservationStatus::Expired => "expired",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [
            ReservationStatus::Held,
            ReservationStatus::Confirmed,
            ReservationStatus::Fulfilled,
            ReservationStatus::Released,
            ReservationStatus::Expired,
        ]
        .into_iter()
        .find(|status| status.as_str() == text)
    }
}

/// Một phần của dòng đơn đã giữ ở một kho. Một dòng đơn có nhiều phần khi là combo (mỗi thành
/// phần một phần) hay khi cửa hàng cho chia kho.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ReservedLine {
    pub line_key: String,
    /// Biến thể thật bị trừ (thành phần, khi dòng là combo).
    pub variant_id: Uuid,
    /// Biến thể combo của dòng đơn, khi dòng là combo.
    pub bundle_variant_id: Option<Uuid>,
    pub warehouse_org_id: OrgId,
    /// Đơn vị của biến thể này đã giữ.
    pub quantity: i64,
    pub fulfilled_quantity: i64,
    pub released_quantity: i64,
    /// Giữ khi tồn bán được không đủ (sản phẩm cho bán khi hết hàng).
    pub backordered: bool,
    /// Dòng đặt trước (D-204).
    pub preordered: bool,
}

/// Chỗ giữ hàng của một đơn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Reservation {
    pub id: Uuid,
    pub order_id: Uuid,
    pub selling_org_id: OrgId,
    pub status: ReservationStatus,
    /// `None` khi đã xác nhận (hay đã kết thúc).
    pub expires_at: Option<DateTime<Utc>>,
    /// Dòng không theo dõi tồn (hàng số, dịch vụ) không có phần nào ở đây.
    pub lines: Vec<ReservedLine>,
}

/// Một dòng của lần xuất kho.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FulfilLine {
    pub line_key: String,
    /// Số đơn vị của dòng đơn (combo: số combo) xuất lần này.
    pub quantity: i64,
    /// Lô xuất; trống khi biến thể theo dõi lô thì tự chọn lô hết hạn trước.
    pub lot_code: Option<String>,
    /// Bắt buộc, đủ `quantity` số, khi biến thể theo dõi serial.
    pub serials: Vec<String>,
}

impl FulfilLine {
    pub fn new(line_key: impl Into<String>, quantity: i64) -> Self {
        FulfilLine { line_key: line_key.into(), quantity, lot_code: None, serials: Vec::new() }
    }

    pub fn lot(mut self, lot_code: impl Into<String>) -> Self {
        self.lot_code = Some(lot_code.into());
        self
    }

    pub fn serials(mut self, serials: Vec<String>) -> Self {
        self.serials = serials;
        self
    }
}

/// Lệnh xuất kho theo chỗ đã giữ của một đơn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FulfilRequest {
    pub order_id: Uuid,
    /// Mã của lần xuất (vd mã vận đơn): gửi lại cùng mã không xuất lần hai.
    pub fulfilment_key: String,
    /// Trống: xuất hết phần còn lại.
    pub lines: Vec<FulfilLine>,
}

impl FulfilRequest {
    pub fn new(order_id: Uuid, fulfilment_key: impl Into<String>, lines: Vec<FulfilLine>) -> Self {
        FulfilRequest { order_id, fulfilment_key: fulfilment_key.into(), lines }
    }
}

/// Một dòng hàng trả về nhập lại kho.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RestockLine {
    pub variant_id: Uuid,
    pub quantity: i64,
    /// `false`: chỉ ghi nhận (hàng hỏng), không cộng vào tồn.
    pub sellable: bool,
    pub lot_code: Option<String>,
    pub serials: Vec<String>,
}

impl RestockLine {
    pub fn new(variant_id: Uuid, quantity: i64, sellable: bool) -> Self {
        RestockLine { variant_id, quantity, sellable, lot_code: None, serials: Vec::new() }
    }
}

/// Lệnh nhập lại hàng trả về.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RestockRequest {
    /// Vd `return:<return_id>`; chống trùng vĩnh viễn.
    pub reference: String,
    pub order_id: Option<Uuid>,
    pub warehouse_org_id: OrgId,
    pub lines: Vec<RestockLine>,
}

impl RestockRequest {
    pub fn new(
        reference: impl Into<String>,
        order_id: Option<Uuid>,
        warehouse_org_id: OrgId,
        lines: Vec<RestockLine>,
    ) -> Self {
        RestockRequest { reference: reference.into(), order_id, warehouse_org_id, lines }
    }
}

/// Tồn của một biến thể ở một kho phục vụ cửa hàng.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Availability {
    pub variant_id: Uuid,
    pub warehouse_org_id: OrgId,
    pub on_hand: i64,
    pub available: i64,
    pub incoming: i64,
}

/// Vì sao trả chỗ giữ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseReason {
    Cancelled,
    PaymentFailed,
    Expired,
    CheckoutAborted,
    /// Nhân viên gỡ chỗ giữ bị kẹt bằng tay.
    Manual,
}

impl ReleaseReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            ReleaseReason::Cancelled => "cancelled",
            ReleaseReason::PaymentFailed => "payment_failed",
            ReleaseReason::Expired => "expired",
            ReleaseReason::CheckoutAborted => "checkout_aborted",
            ReleaseReason::Manual => "manual",
        }
    }
}

/// Tồn kho cho module khác: giữ chỗ → xác nhận → xuất / trả lại theo `order_id` (`orders`, bước
/// 45), nhập lại hàng trả về, đọc tồn bán được.
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`; macro này đổi hàm
// `async` thành hàm trả về `Box<dyn Future>` để gọi được.
#[async_trait::async_trait]
pub trait Inventory: Send + Sync {
    /// Tồn của các biến thể ở từng kho phục vụ `selling_org_id` (theo thứ tự chọn kho), tối đa
    /// [`MAX_IDS`] biến thể. Biến thể chưa có tồn ở kho nào thì vắng.
    async fn availability(
        &self,
        tenant: &TenantContext,
        selling_org_id: OrgId,
        variant_ids: &[Uuid],
    ) -> Result<Vec<Availability>, Error>;

    /// Giữ hàng cho cả đơn: tất cả hoặc không gì. Gọi lại cùng `order_id` + cùng nội dung →
    /// `Submitted::Replayed`; khác nội dung → `reference_reused`. Combo được tách thành thành phần;
    /// biến thể không theo dõi tồn được bỏ qua. Lỗi: `inventory.insufficient_stock`,
    /// `inventory.no_source`, `inventory.hold_too_long`, `inventory.preorder_sold_out`.
    async fn reserve(
        &self,
        tenant: &TenantContext,
        request: &ReserveRequest,
    ) -> Result<Submitted<Reservation>, Error>;

    /// Giữ → xác nhận (bỏ hạn). Đã xác nhận / đã xuất: trả như cũ. Đã hết hạn / đã trả:
    /// `inventory.reservation_expired`; không có: `inventory.reservation_not_found`.
    async fn confirm(&self, tenant: &TenantContext, order_id: Uuid) -> Result<Reservation, Error>;

    /// Trả phần chưa xuất về tồn bán được. Không có chỗ giữ: `Ok(None)`; đã trả / đã kết thúc:
    /// trả như cũ (gọi lại an toàn).
    async fn release(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
        reason: ReleaseReason,
    ) -> Result<Option<Reservation>, Error>;

    /// Xuất kho theo chỗ đã giữ (một phần được; chỗ còn giữ thì được xác nhận luôn). Gọi lại cùng
    /// `fulfilment_key` + cùng nội dung: trả như cũ, không xuất lần hai.
    async fn fulfil(
        &self,
        tenant: &TenantContext,
        request: &FulfilRequest,
    ) -> Result<Reservation, Error>;

    /// Nhập lại hàng trả về. Gọi lại cùng `reference` + cùng nội dung: không cộng lần hai.
    async fn restock(&self, tenant: &TenantContext, request: &RestockRequest) -> Result<(), Error>;

    /// Các biến thể (trong `variant_ids`, tối đa [`MAX_IDS`]) còn hàng bán được ngay ở cửa hàng
    /// `store_org_id` — bộ lọc "chỉ hàng có sẵn ở cửa hàng X" của `catalog` (D-200).
    async fn in_stock_at(
        &self,
        tenant: &TenantContext,
        store_org_id: OrgId,
        variant_ids: &[Uuid],
    ) -> Result<Vec<Uuid>, Error>;

    /// Biến thể (trong `variant_ids`) mà tài khoản `user_id` đang có đăng ký báo có hàng lại đã
    /// xác nhận — để `customers` (bước 47) không gửi trùng tin cho cùng một lần có hàng (D-237).
    async fn restock_alerts_of(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        variant_ids: &[Uuid],
    ) -> Result<Vec<Uuid>, Error>;
}
