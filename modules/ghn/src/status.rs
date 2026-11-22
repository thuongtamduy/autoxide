//! Trạng thái đơn của GHN (trang "Order Status Codes") → trạng thái chung của `shipping`.

use autoxide_shipping::api::ShipmentStatus::{self, *};

/// Trạng thái chung của một trạng thái GHN. `None`: GHN có trạng thái mà `shipping` không có
/// trạng thái tương ứng (`exception` — cần người xử lý), hoặc trạng thái mới chưa có trong tài liệu;
/// cập nhật đó bị bỏ qua thay vì đoán.
pub(crate) fn map(ghn: &str) -> Option<ShipmentStatus> {
    Some(match ghn {
        "ready_to_pick" | "picking" | "money_collect_picking" => ReadyToShip,
        "picked" => PickedUp,
        "storing" | "sorting" | "transporting" => InTransit,
        "delivering" | "money_collect_delivering" => OutForDelivery,
        "delivered" => Delivered,
        "delivery_fail" => DeliveryFailed,
        "waiting_to_return"
        | "return"
        | "return_transporting"
        | "return_sorting"
        | "returning"
        | "return_fail" => Returning,
        "returned" => Returned,
        "cancel" => Cancelled,
        // `damage` (hư hỏng) và `scrap` (tiêu huỷ): hàng không tới người nhận và không về người
        // gửi — với `shipping` là mất hàng. Nguyên văn của GHN vẫn nằm ở `carrier_status`.
        "lost" | "damage" | "scrap" => Lost,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moi_trang_thai_trong_tai_lieu() {
        let documented = [
            ("ready_to_pick", Some(ReadyToShip)),
            ("picking", Some(ReadyToShip)),
            ("money_collect_picking", Some(ReadyToShip)),
            ("picked", Some(PickedUp)),
            ("storing", Some(InTransit)),
            ("sorting", Some(InTransit)),
            ("transporting", Some(InTransit)),
            ("delivering", Some(OutForDelivery)),
            ("money_collect_delivering", Some(OutForDelivery)),
            ("delivered", Some(Delivered)),
            ("delivery_fail", Some(DeliveryFailed)),
            ("waiting_to_return", Some(Returning)),
            ("return", Some(Returning)),
            ("return_transporting", Some(Returning)),
            ("return_sorting", Some(Returning)),
            ("returning", Some(Returning)),
            ("return_fail", Some(Returning)),
            ("returned", Some(Returned)),
            ("cancel", Some(Cancelled)),
            ("exception", None),
            ("lost", Some(Lost)),
            ("damage", Some(Lost)),
            ("scrap", Some(Lost)),
        ];
        for (ghn, expected) in documented {
            assert_eq!(map(ghn), expected, "{ghn}");
        }
        assert_eq!(map("something_new"), None);
    }
}
