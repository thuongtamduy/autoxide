//! `dyn Invoices` module `invoices` công bố.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::EntityTrait;
use uuid::Uuid;

use crate::api::{Document, Invoices, NewDocument};
use crate::entity::document;
use crate::service::{InvoicesService, describe};

pub(crate) struct Provider(pub Arc<InvoicesService>);

#[async_trait::async_trait]
impl Invoices for Provider {
    async fn create_document(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        draft: NewDocument,
    ) -> Result<Submitted<Document>, Error> {
        self.0.create_document(tenant, actor, draft).await
    }

    async fn issue(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        document: Uuid,
    ) -> Result<Document, Error> {
        self.0.issue(tenant, actor, document).await
    }

    async fn document(&self, tenant: &TenantContext, id: Uuid) -> Result<Option<Document>, Error> {
        let read = tenant.database().read();
        match document::Entity::find_by_id(id).scoped(tenant).one(read).await? {
            Some(row) => Ok(Some(describe(read, tenant, row).await?)),
            None => Ok(None),
        }
    }

    async fn cancel(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        document: Uuid,
        reason: String,
    ) -> Result<Document, Error> {
        self.0.cancel(tenant, actor, document, reason).await
    }
}
