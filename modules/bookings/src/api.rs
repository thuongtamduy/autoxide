//! Contract of the `bookings` module for other modules. No module needs to call into `bookings` yet; the
//! module's events (`bookings.booking_created`, `…_confirmed`, `…_cancelled`, `…_rescheduled`, `…_no_show`,
//! `…_completed`) are the way to listen for changes. Add traits here when a module needs them.

/// Names of the booking events, for modules to listen to through `Module::subscriptions`.
pub const BOOKING_EVENTS: [&str; 6] = [
    "bookings.booking_created",
    "bookings.booking_confirmed",
    "bookings.booking_cancelled",
    "bookings.booking_rescheduled",
    "bookings.booking_no_show",
    "bookings.booking_completed",
];
