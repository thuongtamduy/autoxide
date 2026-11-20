//! `dyn Invoices` module `invoices` công bố.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::EntityTrait;
use uuid::Uuid;

use crate::api::{Document, Invoices, NewDocument, NewDocumentLine, RenderFormat, Rendered};
use crate::entity::document;
use crate::service::{InvoicesService, describe, document_not_found};

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

    async fn adjust(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        original: Uuid,
        direction: &str,
        lines: Vec<NewDocumentLine>,
        reference: String,
    ) -> Result<Submitted<Document>, Error> {
        let found = document::Entity::find_by_id(original)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(document_not_found)?;
        let correction = crate::issue::Correction {
            relation: "adjustment",
            direction: Some(direction.to_owned()),
            buyer: None,
            lines,
            reference,
        };
        self.0.correct_document(tenant, actor, found, correction).await
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

    async fn render(
        &self,
        tenant: &TenantContext,
        document: Uuid,
        format: RenderFormat,
        locale: &str,
    ) -> Result<Rendered, Error> {
        let found = self.document(tenant, document).await?.ok_or_else(document_not_found)?;
        self.0.render_document(tenant, &found, format, locale).await
    }

    async fn archive(&self, tenant: &TenantContext, actor: Actor) -> Result<Uuid, Error> {
        Ok(self.0.archive_tenant(tenant, actor, "manual").await?.id)
    }
}
