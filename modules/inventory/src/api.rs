//! Hợp đồng của module `inventory` cho module khác (`orders`, `catalog`, `channels`, báo
//! cáo…): chỉ những gì ở đây được dùng từ bên ngoài (`autoxide_inventory::api::…`). Module khác
//! lấy trait lúc khởi động bằng `context.require::<dyn …>()` và phải đăng ký SAU `inventory`
//! trong `crates/app/src/modules.rs`. Không module nào đọc bảng `inventory_…` (luật kiến trúc 1).
//! File này không tham chiếu phần bên trong module.

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
}
