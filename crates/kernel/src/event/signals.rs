//! Ephemeral signals over WebSocket (step 69 P3, D-20261119-step69-ws): "typing", "online" within
//! a topic (e.g. a conversation). NOT stored: relayed only between connections open in the same
//! process - a lost signal only loses a "typing" indicator, not data (data goes through REST and
//! events).
//!
//! The kernel does not know which topics exist or who may join them: the module that owns a topic kind
//! contributes a [`SignalTopics`] (`context.contribute::<dyn SignalTopics>(...)`) answering "may this person
//! send and receive signals on topic `<kind>:<id>`". If no module contributes that kind, nobody can join.

use std::sync::{Arc, OnceLock};

use crate::access::Principal;
use crate::error::Error;
use crate::tenancy::TenantContext;

/// Contributed by a module to open its own signal topics (e.g. `conversations.thread`).
#[async_trait::async_trait]
pub trait SignalTopics: Send + Sync {
    /// Topic kind, `<module>.<kind>`; a topic is `<kind>:<id>`.
    fn kind(&self) -> &'static str;

    /// Whether `principal` may send and receive signals on topic `<kind>:<id>` of tenant `tenant` -
    /// usually: whether it can read record `id`.
    async fn allows(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: &str,
    ) -> Result<bool, Error>;
}

/// The [`SignalTopics`] contributed by modules; filled in after every module has started. Cheap to clone.
#[derive(Clone, Default)]
pub(crate) struct SignalTopicsSlot(Arc<OnceLock<Vec<Arc<dyn SignalTopics>>>>);

impl SignalTopicsSlot {
    pub(crate) fn fill(&self, topics: Vec<Arc<dyn SignalTopics>>) {
        let _ = self.0.set(topics);
    }

    /// The responder for kind `kind`.
    pub(crate) fn find(&self, kind: &str) -> Option<Arc<dyn SignalTopics>> {
        self.0.get()?.iter().find(|topics| topics.kind() == kind).cloned()
    }
}
