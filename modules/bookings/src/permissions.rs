//! Permission của module `bookings`: `<module>.<việc>`, đều theo nút (bước 34). Handler REST và
//! tool MCP kiểm qua service.

use autoxide_kernel::access::Permission;

/// Quản lý lịch đặt của nút: xem mọi lịch, đặt hộ khách, xác nhận, check-in, vắng mặt, huỷ, đổi giờ.
pub(crate) const BOOKINGS_MANAGE: Permission = Permission::new(
    "bookings.manage",
    "See and manage every booking of the unit: book for customers, confirm, check in, mark no-shows, \
     cancel and reschedule.",
)
.scoped();

/// Chỉ xem mọi lịch đặt của nút (lễ tân, báo cáo).
pub(crate) const BOOKINGS_READ: Permission =
    Permission::new("bookings.read", "See every booking of the unit, without changing them.")
        .scoped();

/// Nhân viên: xem lịch của chính mình (tài nguyên gắn với tài khoản), check-in, xong, vắng mặt.
pub(crate) const BOOKINGS_STAFF: Permission = Permission::new(
    "bookings.staff",
    "See your own appointments (resources linked to your account), check customers in, mark them \
     done or no-show.",
)
.scoped();

/// Dịch vụ, nhân viên / phòng / thiết bị, lịch làm việc và ngày nghỉ.
pub(crate) const SERVICES_MANAGE: Permission = Permission::new(
    "bookings.services.manage",
    "Manage bookable services, staff, rooms and equipment, their weekly schedules and exceptions.",
)
.scoped();

pub(crate) fn all() -> Vec<Permission> {
    vec![BOOKINGS_MANAGE, BOOKINGS_READ, BOOKINGS_STAFF, SERVICES_MANAGE]
}
