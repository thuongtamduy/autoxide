//! Bảng chuyển trạng thái của vận đơn (D-20261118-step44-shipping-carriers). Trạng thái chỉ đi tới theo
//! bảng; cập nhật của hãng đến sai thứ tự (vd `in_transit` sau `delivered`) được ghi vào hành trình
//! nhưng không đổi trạng thái. Nhân viên đổi trái bảng → `shipping.invalid_transition`.

use crate::api::ShipmentStatus::{self, *};

/// `from → to` có hợp lệ không.
pub(crate) fn allowed(from: ShipmentStatus, to: ShipmentStatus) -> bool {
    if from == to {
        return false;
    }
    match from {
        Pending => matches!(
            to,
            Booking
                | ReadyToShip
                | ReadyForPickup
                | PickedUp
                | InTransit
                | OutForDelivery
                | Delivered
                | DeliveryFailed
                | Cancelled
        ),
        Booking => matches!(to, ReadyToShip | Pending | Cancelled),
        ReadyToShip => matches!(
            to,
            PickedUp | InTransit | OutForDelivery | Delivered | DeliveryFailed | Cancelled | Lost
        ),
        PickedUp => {
            matches!(to, InTransit | OutForDelivery | Delivered | DeliveryFailed | Returning | Lost)
        }
        InTransit => matches!(to, OutForDelivery | Delivered | DeliveryFailed | Returning | Lost),
        OutForDelivery => matches!(to, Delivered | DeliveryFailed | Returning | Lost),
        // Giao lại sau lần thất bại.
        DeliveryFailed => {
            matches!(to, OutForDelivery | InTransit | Delivered | Returning | Returned | Lost)
        }
        Returning => matches!(to, Returned | Lost),
        ReadyForPickup => matches!(to, PickedUpByBuyer | Returning | Cancelled),
        Delivered | Returned | PickedUpByBuyer | Cancelled | Lost => false,
    }
}

/// Hãng đã cầm hàng: không huỷ được nữa.
pub(crate) fn picked_up(status: ShipmentStatus) -> bool {
    !matches!(status, Pending | Booking | ReadyToShip | ReadyForPickup | Cancelled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bang_chuyen_trang_thai() {
        assert!(allowed(Pending, Booking));
        assert!(allowed(InTransit, Delivered));
        assert!(allowed(DeliveryFailed, OutForDelivery), "giao lại");
        assert!(!allowed(Delivered, InTransit), "đã giao không lùi");
        assert!(!allowed(Cancelled, Pending));
        assert!(!allowed(Returned, Returning));
        assert!(picked_up(InTransit));
        assert!(!picked_up(ReadyToShip));
        for status in ShipmentStatus::ALL {
            if status.is_final() {
                assert!(ShipmentStatus::ALL.iter().all(|to| !allowed(status, *to)), "{status}");
            }
        }
    }
}
