//! `dyn TaxEngine` module `tax` công bố.

use autoxide_kernel::time::{Date, Timestamp};
use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{OrgId, TenantContext};

use crate::api::{
    InvoiceRequirements, TaxEngine, TaxProfile, TaxQuote, TaxQuoteRequest, WithholdingQuote,
    WithholdingRequest,
};
use crate::service::{TaxService, describe_profile};

pub(crate) struct Provider(pub Arc<TaxService>);

#[async_trait::async_trait]
impl TaxEngine for Provider {
    async fn quote(
        &self,
        tenant: &TenantContext,
        request: TaxQuoteRequest,
    ) -> Result<TaxQuote, Error> {
        self.0.quote(tenant, request).await
    }

    async fn withholding(
        &self,
        _tenant: &TenantContext,
        request: WithholdingRequest,
    ) -> Result<WithholdingQuote, Error> {
        self.0.withholding(request).await
    }

    async fn profile(
        &self,
        tenant: &TenantContext,
        org: OrgId,
    ) -> Result<Option<TaxProfile>, Error> {
        Ok(self.0.nearest_profile(tenant, org).await?.as_ref().map(describe_profile))
    }

    async fn invoice_requirements(
        &self,
        tenant: &TenantContext,
        org: OrgId,
        on: Date,
    ) -> Result<InvoiceRequirements, Error> {
        self.0.invoice_requirements(tenant, org, on).await
    }

    async fn tax_date(
        &self,
        tenant: &TenantContext,
        org: OrgId,
        at: Timestamp,
    ) -> Result<Date, Error> {
        self.0.tax_date(tenant, org, at).await
    }

    async fn einvoice_cancel_forbidden(
        &self,
        tenant: &TenantContext,
        org: OrgId,
        on: Date,
    ) -> Result<Option<String>, Error> {
        self.0.einvoice_cancel_forbidden(tenant, org, on).await
    }
}
