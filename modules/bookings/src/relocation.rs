//! Chính sách tách / gộp khách của các bảng của `bookings` (bước 38, D-20261118-step38-relocation):
//! dịch vụ, tài nguyên và lượt đặt theo nút; bảng con đi theo dòng cha; mã xác minh của khách vãng
//! lai là dữ liệu tạm, không đi đâu cả.

use autoxide_kernel::tenancy::Relocation;

pub(crate) fn relocation() -> Relocation {
    Relocation::new()
        .by_org(&["bookings_services", "bookings_resources", "bookings_bookings"])
        .follow("bookings_service_resources", "service_id", "bookings_services")
        .follow("bookings_schedules", "resource_id", "bookings_resources")
        .follow("bookings_exceptions", "resource_id", "bookings_resources")
        .follow("bookings_sessions", "service_id", "bookings_services")
        .follow("bookings_slots", "resource_id", "bookings_resources")
        .ephemeral(&["bookings_guest_challenges", "bookings_throttle"])
}
