//! Contract of the `momo` module: no trait of its own. The module contributes a `dyn PaymentGateway`
//! (`autoxide_kernel::payment`) to `ledger`; other modules only need the partner name to choose a gateway.

/// Partner name: the `provider` value of a payment and the segment in the webhook path
/// (`/api/v1/ledger/webhooks/momo/{hook}`).
pub const PROVIDER: &str = "momo";
