//! A fake app store for tests (step 69 P4): signs purchase proofs and server notifications with one key
//! of the test, and verifies them just as the real implementation verifies Apple / Google signatures.

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::secret::sha256;
use autoxide_kernel::store::{
    AppStore, PurchaseState, StoreNotification, StorePurchase, StorePurchases,
};
use autoxide_kernel::time::Timestamp;
use serde_json::{Value, json};

/// A fake store: `proof` / `notification_body` produce signed data; `verify` / `notification` check it.
#[derive(Debug, Clone)]
pub struct FakeStorePurchases {
    store: AppStore,
    key: String,
}

impl FakeStorePurchases {
    pub fn new(store: AppStore, key: impl Into<String>) -> Self {
        FakeStorePurchases { store, key: key.into() }
    }

    fn sign(&self, payload: &str) -> String {
        sha256(&format!("{}:{payload}", self.key))
    }

    /// Purchase proof (like the signed transaction the app sends up) for `purchase`.
    pub fn proof(&self, purchase: &StorePurchase) -> String {
        let payload = payload(purchase).to_string();
        format!("{payload}.{}", self.sign(&payload))
    }

    /// Webhook body (like the store's server notification) carrying code `id` and the state of `purchase`.
    pub fn notification_body(&self, id: &str, purchase: &StorePurchase) -> Vec<u8> {
        let payload = payload(purchase).to_string();
        let signature = self.sign(&payload);
        json!({"id": id, "payload": payload, "signature": signature}).to_string().into_bytes()
    }

    fn open(&self, payload: &str, signature: &str) -> Option<StorePurchase> {
        if self.sign(payload) != signature {
            return None;
        }
        let value: Value = serde_json::from_str(payload).ok()?;
        let state = match value["state"].as_str()? {
            "active" => PurchaseState::Active,
            "expired" => PurchaseState::Expired,
            "refunded" => PurchaseState::Refunded,
            "revoked" => PurchaseState::Revoked,
            _ => return None,
        };
        let expires_at = match value["expires_at"].as_str() {
            Some(text) => Some(Timestamp::parse_rfc3339(text).ok()?),
            None => None,
        };
        Some(StorePurchase::new(
            self.store,
            value["product_id"].as_str()?,
            value["original_transaction_id"].as_str()?,
            value["transaction_id"].as_str()?,
            state,
            expires_at,
        ))
    }
}

fn payload(purchase: &StorePurchase) -> Value {
    let state = match purchase.state {
        PurchaseState::Active => "active",
        PurchaseState::Expired => "expired",
        PurchaseState::Refunded => "refunded",
        PurchaseState::Revoked => "revoked",
    };
    json!({
        "product_id": purchase.product_id,
        "original_transaction_id": purchase.original_transaction_id,
        "transaction_id": purchase.transaction_id,
        "state": state,
        "expires_at": purchase.expires_at.map(|at| at.to_rfc3339()),
    })
}

#[async_trait::async_trait]
impl StorePurchases for FakeStorePurchases {
    fn store(&self) -> AppStore {
        self.store
    }

    async fn verify(&self, proof: &str) -> Result<StorePurchase, Error> {
        proof
            .rsplit_once('.')
            .and_then(|(payload, signature)| self.open(payload, signature))
            .ok_or_else(|| {
                Error::validation(vec![FieldError::new(
                    "proof",
                    "invalid",
                    "is not a purchase the store confirms",
                )])
            })
    }

    async fn notification(
        &self,
        _headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> Result<StoreNotification, Error> {
        let refused = autoxide_kernel::store::signature_invalid;
        let value: Value = serde_json::from_slice(body).map_err(|_| refused())?;
        let (Some(id), Some(payload), Some(signature)) =
            (value["id"].as_str(), value["payload"].as_str(), value["signature"].as_str())
        else {
            return Err(refused());
        };
        let purchase = self.open(payload, signature).ok_or_else(refused)?;
        Ok(StoreNotification::new(id, purchase))
    }
}
