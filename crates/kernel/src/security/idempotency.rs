//! Idempotency key: resending a write command (because of a flaky network, or an AI agent retrying) does not
//! make it happen twice (D-058).
//!
//! A client sends the header `Idempotency-Key: <string chosen by the client>` with a `POST`, `PUT`, `PATCH`
//! or `DELETE` (per the IETF draft "The Idempotency-Key HTTP Header Field"). First time: the request runs
//! normally and the response is stored. Resending the same key (same caller, same request): receives exactly
//! the stored response, with `Idempotent-Replayed: true`, and the handler does not run again. The same key
//! with a different request: 422. The first run not finished yet: 409.
//!
//! Storage sits behind the [`IdempotencyStore`] trait, provided by a module (the kernel has no table); the HTTP
//! layer is in `http::idempotency`.

use crate::error::Error;
use crate::tenancy::TenantContext;
use crate::time::Timestamp;

/// Stored response of a write command.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StoredResponse {
    pub status: u16,
    /// Headers to send back exactly as they were (`content-type`, `location`, ...).
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl StoredResponse {
    /// A saved response: status code, headers to send back as they were, and body.
    pub const fn new(status: u16, headers: Vec<(String, String)>, body: Vec<u8>) -> Self {
        StoredResponse { status, headers, body }
    }
}

/// Result of starting a request that carries an idempotency key.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Begin {
    /// First time: run the request, then call [`IdempotencyStore::complete`] (or `abandon`)
    /// with this `id`.
    Started { id: uuid::Uuid },
    /// A result already exists: return it.
    Replay(StoredResponse),
    /// The first run is still in progress.
    InProgress,
    /// The same key, but the request differs from the first one.
    Mismatch,
}

/// Where idempotency keys are stored. `scope` is the caller (hashed), `fingerprint` is the request (hashed).
#[async_trait::async_trait]
pub trait IdempotencyStore: Send + Sync {
    /// Registers the request; holds the right to run until `locked_until`.
    async fn begin(
        &self,
        tenant: &TenantContext,
        scope: &str,
        key: &str,
        fingerprint: &str,
        locked_until: Timestamp,
    ) -> Result<Begin, Error>;

    /// Stores the response of run `id`.
    async fn complete(
        &self,
        tenant: &TenantContext,
        id: uuid::Uuid,
        response: StoredResponse,
    ) -> Result<(), Error>;

    /// Run `id` failed on the server side: removes the registration so the client can resend.
    async fn abandon(&self, tenant: &TenantContext, id: uuid::Uuid) -> Result<(), Error>;
}
