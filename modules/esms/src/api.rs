//! Contract of the `esms` module for other modules: only the provider name. Sending goes through the trait
//! `autoxide_kernel::sms::SmsSender` that this module publishes.

/// Provider name, as `SmsSender::provider` returns it.
pub const PROVIDER: &str = "esms";
