//! In-app purchases through the App Store and Google Play (step 69 P4, D-20261119-step69-store): neutral
//! traits to verify transactions with the app store's server and to receive its notifications (App Store
//! Server Notifications v2, Google Real-time Developer Notifications) with signature verification.
//!
//! The real implementations (App Store Server API, Google Play Developer API) are a separate module that
//! needs a developer account and keys supplied by the deployer, contributed with
//! `context.contribute::<dyn StorePurchases>(…)` and registered BEFORE the `plans` module. Tests use
//! `FakeStorePurchases` from the testkit.
//!
//! Money is collected and held by Apple / Google (licensed partners) — in line with D-187: the core holds no money.

use crate::error::Error;
use crate::time::Timestamp;

/// An app store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AppStore {
    /// Apple App Store (`app_store`).
    Apple,
    /// Google Play (`google_play`).
    Google,
}

impl AppStore {
    pub const ALL: [AppStore; 2] = [AppStore::Apple, AppStore::Google];

    /// Stable name in the API and database.
    pub fn as_str(self) -> &'static str {
        match self {
            AppStore::Apple => "app_store",
            AppStore::Google => "google_play",
        }
    }

    pub fn parse(text: &str) -> Option<AppStore> {
        AppStore::ALL.into_iter().find(|store| store.as_str() == text)
    }
}

/// State of a transaction (subscription) according to the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurchaseState {
    /// Active until `expires_at`.
    Active,
    /// Expired, no longer renewed.
    Expired,
    /// The store refunded: access is revoked immediately.
    Refunded,
    /// The store revoked it (e.g. family sharing removed): access is revoked immediately.
    Revoked,
}

/// A transaction confirmed by the store (signature verified / its server called).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StorePurchase {
    pub store: AppStore,
    /// Product id declared in the store.
    pub product_id: String,
    /// Original id of the subscription chain (the first `originalTransactionId` / `purchaseToken`): unchanged
    /// across renewals — the key that keeps a repeated call from granting twice.
    pub original_transaction_id: String,
    /// Id of this purchase / renewal.
    pub transaction_id: String,
    pub state: PurchaseState,
    /// Expires at this time (subscription); `None` for a one-time purchase.
    pub expires_at: Option<Timestamp>,
    /// `true`: the store's test environment (sandbox / license testing).
    pub sandbox: bool,
}

impl StorePurchase {
    pub fn new(
        store: AppStore,
        product_id: impl Into<String>,
        original_transaction_id: impl Into<String>,
        transaction_id: impl Into<String>,
        state: PurchaseState,
        expires_at: Option<Timestamp>,
    ) -> Self {
        StorePurchase {
            store,
            product_id: product_id.into(),
            original_transaction_id: original_transaction_id.into(),
            transaction_id: transaction_id.into(),
            state,
            expires_at,
            sandbox: false,
        }
    }

    #[must_use]
    pub fn sandbox(mut self, sandbox: bool) -> Self {
        self.sandbox = sandbox;
        self
    }
}

/// A server notification from the store, signature verified.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StoreNotification {
    /// The store's notification id (`notificationUUID` / `messageId`): a repeated delivery is not processed twice.
    pub id: String,
    /// New state of the transaction.
    pub purchase: StorePurchase,
}

impl StoreNotification {
    pub fn new(id: impl Into<String>, purchase: StorePurchase) -> Self {
        StoreNotification { id: id.into(), purchase }
    }
}

/// Error 401 when the store's notification has no valid signature — the connector returns it when signature verification fails.
pub fn signature_invalid() -> Error {
    Error::unauthenticated(
        "store_signature_invalid",
        "The store notification is not signed by the store.",
    )
}

/// Connection to one app store.
#[async_trait::async_trait]
pub trait StorePurchases: Send + Sync {
    fn store(&self) -> AppStore;

    /// Verifies with the store the purchase proof the app sent (StoreKit 2 signed transaction / Google
    /// Play `purchaseToken`). Wrong, forged or another app's proof → error `validation`.
    async fn verify(&self, proof: &str) -> Result<StorePurchase, Error>;

    /// Reads and verifies the signature of a store server notification (`headers`: headers of the webhook
    /// request, `body`: the untouched body). Wrong signature → error `unauthenticated`.
    async fn notification(
        &self,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> Result<StoreNotification, Error>;
}
