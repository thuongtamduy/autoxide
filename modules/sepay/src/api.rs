//! Public contract of the `sepay` connector: only the connector name. All work goes through the kernel's
//! `BankFeed` trait; other modules do not call into here directly.

/// Stable name of the connector: the `feed_provider` value of a bank account in `ledger` and the
/// `{provider}` segment in the webhook path `/api/v1/ledger/bank-feeds/{provider}/{hook}`.
pub const PROVIDER: &str = "sepay";
