//! Hợp đồng của module `bookings` cho module khác. Chưa module nào cần gọi vào `bookings`; event của
//! module (`bookings.booking_created`, `…_confirmed`, `…_cancelled`, `…_rescheduled`, `…_no_show`,
//! `…_completed`) là cách nghe thay đổi. Thêm trait vào đây khi có module cần.

/// Tên các event lịch đặt, cho module nghe qua `Module::subscriptions`.
pub const BOOKING_EVENTS: [&str; 6] = [
    "bookings.booking_created",
    "bookings.booking_confirmed",
    "bookings.booking_cancelled",
    "bookings.booking_rescheduled",
    "bookings.booking_no_show",
    "bookings.booking_completed",
];
