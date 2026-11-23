//! The sandbox's fake mailbox (step 38, D-232): what the sandbox would have sent out but is held back: partner
//! calls (payments, payouts, e-invoices, eKYC), SMS, ZNS, push notifications, mail to people
//! other than staff, webhooks to URLs not allowed.
//!
//! - The `kernel_sandbox_messages` table, tenant-level, in the sandbox's database (the same placement as
//!   the original tenant); deleted together with the sandbox in the tenant deletion flow. Append-only.
//! - Written with [`hold_back`] outside any transaction of the caller (a partner call must not sit inside a
//!   write transaction anyway): a row is a log for the person testing to look at; losing a row when the
//!   database fails does not break the business operation.
//! - People in the sandbox read it with `GET /api/v1/kernel/sandbox/messages` (`kernel.sandboxes.read`).
use crate::time::Timestamp;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use super::SANDBOXES_READ;
use crate::access::Principal;
use crate::error::Error;
use crate::migration::prelude::*;
use crate::pagination::{Page, PageRequest, fetch_page};
use crate::tenancy::{ScopeExt, TenantContext, TenantScoped};

const TABLE: &str = "kernel_sandbox_messages";
/// Maximum length of text written to the mailbox (subject, recipient): truncated instead of erroring.
const SHORT_MAX: usize = 320;

pub(crate) mod entity {
    //! The `kernel_sandbox_messages` table.
    use crate::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "kernel_sandbox_messages")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub channel: String,
        pub provider: String,
        pub reason: String,
        pub recipient: Option<String>,
        pub subject: Option<String>,
        /// JSON: details of the call (amount, content...).
        pub body: String,
        pub created_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

impl TenantScoped for entity::Entity {
    fn tenant_column() -> Self::Column {
        entity::Column::TenantId
    }
}

/// Channel of a held-back item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeldChannel {
    /// Partner call: payment, payout, balance movement, e-invoice, eKYC.
    Partner,
    Sms,
    Zns,
    /// Push notification to a browser / phone.
    Push,
    Mail,
    Webhook,
}

impl HeldChannel {
    pub fn as_str(self) -> &'static str {
        match self {
            HeldChannel::Partner => "partner",
            HeldChannel::Sms => "sms",
            HeldChannel::Zns => "zns",
            HeldChannel::Push => "push",
            HeldChannel::Mail => "mail",
            HeldChannel::Webhook => "webhook",
        }
    }
}

/// Why it was held back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeldReason {
    /// Partners are always simulated in a sandbox.
    Simulated,
    /// Mail to a person who is not a staff member who can enter the sandbox.
    RecipientNotAllowed,
    /// Webhook to a URL not in `kernel.sandbox.webhook_urls`.
    UrlNotAllowed,
}

impl HeldReason {
    pub fn as_str(self) -> &'static str {
        match self {
            HeldReason::Simulated => "simulated",
            HeldReason::RecipientNotAllowed => "recipient_not_allowed",
            HeldReason::UrlNotAllowed => "url_not_allowed",
        }
    }
}

/// Something the sandbox would have sent out.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct HeldMessage {
    pub channel: HeldChannel,
    /// Name of the partner (`vnpay`, `smtp`...) or the operation (`payment.create`).
    pub provider: String,
    pub reason: HeldReason,
    pub recipient: Option<String>,
    pub subject: Option<String>,
    pub body: Value,
}

impl HeldMessage {
    pub fn new(channel: HeldChannel, provider: impl Into<String>, reason: HeldReason) -> Self {
        HeldMessage {
            channel,
            provider: provider.into(),
            reason,
            recipient: None,
            subject: None,
            body: Value::Null,
        }
    }

    #[must_use]
    pub fn recipient(mut self, recipient: impl Into<String>) -> Self {
        self.recipient = Some(recipient.into());
        self
    }

    #[must_use]
    pub fn subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }

    #[must_use]
    pub fn body(mut self, body: Value) -> Self {
        self.body = body;
        self
    }
}

fn short(text: Option<String>) -> Option<String> {
    text.map(|text| text.chars().take(SHORT_MAX).collect())
}

/// Writes a held-back item into the fake mailbox of sandbox `tenant`. Call it outside a transaction.
pub async fn hold_back(tenant: &TenantContext, message: HeldMessage) -> Result<(), Error> {
    let id = Uuid::now_v7();
    entity::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant.id().as_uuid()),
        channel: Set(message.channel.as_str().to_owned()),
        provider: Set(message.provider.chars().take(64).collect()),
        reason: Set(message.reason.as_str().to_owned()),
        recipient: Set(short(message.recipient)),
        subject: Set(short(message.subject)),
        body: Set(message.body.to_string()),
        created_at: Set(Timestamp::now()),
    }
    .insert(tenant.database().write())
    .await?;
    tracing::info!(
        message_id = %id,
        channel = message.channel.as_str(),
        reason = message.reason.as_str(),
        "sandbox held back an outgoing message"
    );
    metrics::counter!("kernel_sandbox_held_total", "channel" => message.channel.as_str())
        .increment(1);
    Ok(())
}

/// Something the sandbox would have sent out and kept instead: a partner call (payment, payout,
/// e-invoice, identity check), an SMS, a Zalo message, a push notification, a mail to someone who
/// is not staff of the tenant, or a webhook to a URL not allowed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SandboxMessage {
    pub id: Uuid,
    /// `partner`, `sms`, `zns`, `push`, `mail` or `webhook`.
    #[schema(example = "mail")]
    pub channel: String,
    /// The partner or operation, e.g. `smtp`, `payment.create`, `einvoice.issue`.
    pub provider: String,
    /// `simulated` (partners are always simulated in a sandbox), `recipient_not_allowed` (mail
    /// to someone who cannot enter the sandbox) or `url_not_allowed` (webhook URL not listed in
    /// the tenant setting `kernel.sandbox.webhook_urls`).
    #[schema(example = "recipient_not_allowed")]
    pub reason: String,
    /// Address, phone number or URL it was meant for.
    pub recipient: Option<String>,
    /// Subject or short title.
    pub subject: Option<String>,
    /// Details of the call (amounts, text, payload).
    pub body: Value,
    pub created_at: Timestamp,
}

fn describe(row: entity::Model) -> SandboxMessage {
    SandboxMessage {
        id: row.id,
        channel: row.channel,
        provider: row.provider,
        reason: row.reason,
        recipient: row.recipient,
        subject: row.subject,
        body: serde_json::from_str(&row.body).unwrap_or(Value::Null),
        created_at: row.created_at,
    }
}

/// `tenancy.sandbox_only` (409): an action that only makes sense inside a sandbox.
pub fn sandbox_only() -> Error {
    Error::conflict(
        "tenancy.sandbox_only",
        "This only works inside a sandbox; send `Autoxide-Tenant: <sandbox id>`.",
    )
}

/// One page of the current sandbox's fake mailbox; `channel` filters by channel.
pub(crate) async fn list(
    tenant: &TenantContext,
    principal: &Principal,
    channel: Option<&str>,
    page: &PageRequest,
) -> Result<Page<SandboxMessage>, Error> {
    principal.require(&SANDBOXES_READ)?;
    if !tenant.is_sandbox() {
        return Err(sandbox_only());
    }
    let mut select = entity::Entity::find().scoped(tenant);
    if let Some(channel) = channel {
        select = select.filter(entity::Column::Channel.eq(channel));
    }
    let page = fetch_page(select, entity::Column::Id, |row| row.id, page, tenant.database().read())
        .await?;
    Ok(page.map(describe))
}

/// The table linking a sandbox record to its original record in the real tenant (configuration copy, push to
/// the real tenant).
const LINKS: &str = "kernel_sandbox_links";

/// Migration `m0038_create_sandbox_tables`, in every data database: the fake mailbox and the table linking
/// sandbox records ↔ original tenant (`kind`, record in the sandbox, original record, record pushed to the
/// real tenant).
pub(crate) struct CreateSandboxMessages;

impl MigrationName for CreateSandboxMessages {
    fn name(&self) -> &str {
        "m0038_create_sandbox_tables"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateSandboxMessages {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(TABLE))
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("channel", 16))
                    .col(schema::string_len("provider", 64))
                    .col(schema::string_len("reason", 32))
                    .col(schema::string_len_null("recipient", 320))
                    .col(schema::string_len_null("subject", 320))
                    .col(schema::text("body"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("kernel_sandbox_messages_tenant")
                    .table(Alias::new(TABLE))
                    .col(Alias::new("tenant_id"))
                    .col(Alias::new("id"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(LINKS))
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("kind", 64))
                    .col(schema::uuid("record_id"))
                    .col(schema::uuid_null("source_id"))
                    .col(schema::uuid_null("promoted_id"))
                    .col(schema::string_len_null("promoted_fingerprint", 64))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("kernel_sandbox_links_record")
                    .table(Alias::new(LINKS))
                    .col(Alias::new("tenant_id"))
                    .col(Alias::new("kind"))
                    .col(Alias::new("record_id"))
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("kernel_sandbox_links_source")
                    .table(Alias::new(LINKS))
                    .col(Alias::new("tenant_id"))
                    .col(Alias::new("kind"))
                    .col(Alias::new("source_id"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Alias::new(LINKS)).to_owned()).await?;
        manager.drop_table(Table::drop().table(Alias::new(TABLE)).to_owned()).await
    }
}
