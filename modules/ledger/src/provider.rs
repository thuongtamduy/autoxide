//! `dyn Ledger` module `ledger` công bố.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::Money;
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use chrono::NaiveDate;
use sea_orm::EntityTrait;
use uuid::Uuid;

use crate::api::{
    AccountKey, Entry, FlowSettings, Hold, Ledger, NewEntry, NewPayment, NewPayout, NewRefund,
    NewReversal, NewTransferPayment, OpenAccount, Payment, Payout, Refund, Settled, Settlement,
};
use crate::entity::entry;
use crate::service::{LedgerService, describe_entry};

pub(crate) struct Provider(pub Arc<LedgerService>);

#[async_trait::async_trait]
impl Ledger for Provider {
    async fn post(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        entry: NewEntry,
    ) -> Result<Submitted<Entry>, Error> {
        self.0.post(tenant, actor, entry).await
    }

    async fn reverse(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        entry: Uuid,
        reversal: NewReversal,
    ) -> Result<Submitted<Entry>, Error> {
        self.0.reverse(tenant, actor, entry, reversal).await
    }

    async fn ensure_account(&self, tenant: &TenantContext, key: AccountKey) -> Result<Uuid, Error> {
        Ok(self.0.open_account(tenant, &key, None, None, None, None).await?.0.id)
    }

    async fn open_account(
        &self,
        tenant: &TenantContext,
        account: OpenAccount,
    ) -> Result<Uuid, Error> {
        let found = self
            .0
            .open_account(
                tenant,
                &account.key,
                account.normal_side,
                account.floor_minor,
                account.name,
                None,
            )
            .await?;
        Ok(found.0.id)
    }

    async fn balance(
        &self,
        tenant: &TenantContext,
        key: AccountKey,
        as_of: Option<NaiveDate>,
    ) -> Result<Money, Error> {
        self.0.balance_of_key(tenant, &key, as_of).await
    }

    async fn entry(&self, tenant: &TenantContext, id: Uuid) -> Result<Option<Entry>, Error> {
        let read = tenant.database().read();
        match entry::Entity::find_by_id(id).scoped(tenant).one(read).await? {
            Some(row) => Ok(Some(describe_entry(read, tenant, row).await?)),
            None => Ok(None),
        }
    }

    async fn create_payment(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        payment: NewPayment,
    ) -> Result<Submitted<Payment>, Error> {
        self.0.create_payment(tenant, actor, payment).await
    }

    async fn refund(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        refund: NewRefund,
    ) -> Result<Submitted<Refund>, Error> {
        self.0.refund(tenant, actor, refund).await
    }

    async fn payment(&self, tenant: &TenantContext, id: Uuid) -> Result<Option<Payment>, Error> {
        match LedgerService::find_payment(tenant.database().read(), tenant, id).await? {
            Some(row) => Ok(Some(crate::payments::describe_payment(&row)?)),
            None => Ok(None),
        }
    }

    async fn balances(
        &self,
        tenant: &TenantContext,
        keys: &[AccountKey],
        as_of: Option<NaiveDate>,
    ) -> Result<Vec<Money>, Error> {
        self.0.balances_of_keys(tenant, keys, as_of).await
    }

    async fn flow_settings(
        &self,
        tenant: &TenantContext,
        org: OrgId,
    ) -> Result<FlowSettings, Error> {
        self.0.flow_settings(tenant, org).await
    }

    async fn settle(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        settlement: Settlement,
    ) -> Result<Submitted<Settled>, Error> {
        self.0.settle(tenant, actor, settlement).await
    }

    async fn hold_status(
        &self,
        tenant: &TenantContext,
        source_module: &str,
        source_id: &str,
    ) -> Result<Option<Hold>, Error> {
        self.0.hold_status(tenant, source_module, source_id).await
    }

    async fn create_payout(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        payout: NewPayout,
    ) -> Result<Submitted<Payout>, Error> {
        self.0.create_payout(tenant, actor, payout).await
    }

    async fn create_transfer_payment(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        payment: NewTransferPayment,
    ) -> Result<Submitted<Payment>, Error> {
        self.0.create_transfer_payment(tenant, actor, payment).await
    }
}
