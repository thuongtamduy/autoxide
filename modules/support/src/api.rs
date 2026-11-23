//! Contract of the `support` module for other modules (`autoxide_support::api::…`). A module using it must
//! register AFTER `support` in `crates/app/src/modules.rs` and get the trait at startup:
//!
//! ```text
//! let intake = context.require::<dyn autoxide_support::api::SupportIntake>()?;
//! ```

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use uuid::Uuid;

/// The asker of a ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Requester {
    /// An account (a signed-in buyer).
    User(Uuid),
    /// The asker's phone number, proven (e.g. OTP on the order-view page, step 45), in E.164 form.
    VerifiedPhone(String),
    /// The asker's email, proven.
    VerifiedEmail(String),
}

/// A ticket another module opens on behalf of a buyer (e.g. a "open support" button on the order page).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTicket {
    /// The node that receives the ticket (store, stall); the root node = the tenant ID.
    pub org_id: Uuid,
    pub requester: Requester,
    /// Display name of the asker (empty: "Customer").
    pub requester_name: String,
    /// Kind code (`order`, `product`, `payment`, `account`, `other`, or a kind the tenant declared itself).
    pub category: String,
    pub subject: String,
    pub body: String,
    /// Related record, in the form `<kind>:<id>` (e.g. `orders.order:<id>`).
    pub links: Vec<String>,
    /// The caller's deduplication key: calling again with the same `reference` returns the existing ticket, creating no new one.
    pub reference: String,
}

/// The ticket just opened (or already existing with the same `reference`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketRef {
    pub id: Uuid,
    /// The ticket number shown to the buyer.
    pub number: String,
    /// The view token of a guest buyer's ticket — present only on the first creation (the database keeps only
    /// the hash); `None` for an asker with an account or when called again.
    pub access_token: Option<String>,
}

/// Opens a support ticket on behalf of a buyer.
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`; macro đổi hàm `async`
// thành hàm trả `Box<dyn Future>`.
#[async_trait::async_trait]
pub trait SupportIntake: Send + Sync {
    /// Creates a ticket; the same `reference` → the existing ticket.
    async fn open(&self, tenant: &TenantContext, ticket: NewTicket) -> Result<TicketRef, Error>;
}

/// A question intent that a [`FactSource`] can answer with exact data (D-230).
#[derive(Debug, Clone, PartialEq)]
pub struct FactIntent {
    /// `<module>.<intent>`, e.g. `shipping.tracking`; unique across the whole program.
    pub intent: &'static str,
    /// An English sentence describing the question for the classifier (translated through the declaring module's `text:`).
    pub description: &'static str,
    /// JSON Schema of the parameters the classifier extracts from the question (e.g. `{"order_number": …}`).
    pub params: serde_json::Value,
}

/// A source of answers built from exact data (order status, shipment journey…). The answer
/// is a sentence template i18n of the source module filled with real data — never text written by a model.
#[async_trait::async_trait]
pub trait FactSource: Send + Sync {
    /// The intents this source answers.
    fn intents(&self) -> Vec<FactIntent>;

    /// The answer for `intent` with parameters `params`, in language `locale`. `None` when not
    /// certain: not found, several results, or `requester` cannot prove they own the data
    /// (e.g. the order does not belong to the asker's account or verified phone number).
    async fn answer(
        &self,
        tenant: &TenantContext,
        requester: &Requester,
        intent: &str,
        params: &serde_json::Value,
        locale: &str,
    ) -> Result<Option<String>, Error>;
}

/// Plug point for modules registered AFTER `support` (e.g. `shipping`, `orders`): attach a [`FactSource`] at
/// `start` with `context.require::<dyn SupportFacts>()?.attach(Arc::new(…))`. If no module
/// attaches one, automatic replies use only the approved canned answers.
pub trait SupportFacts: Send + Sync {
    /// Attaches a source; an intent that clashes with an attached source, or lacks the module prefix → error.
    fn attach(&self, source: std::sync::Arc<dyn FactSource>) -> Result<(), Error>;
}

/// A reply email received (the real mail receiver — an email provider's webhook — is a separate module).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundEmail {
    /// Receiving address, in the form `support+<reply key>@…`.
    pub recipient: String,
    /// Sender address.
    pub sender: String,
    /// Plain-text body of the email (quoted older mail is cut off).
    pub text: String,
}

/// Where the email went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundOutcome {
    /// Becomes a message of the ticket.
    Added { ticket_id: Uuid },
    /// Dropped: matches no ticket, unknown sender, ticket closed, or the node has not enabled inbound email.
    Dropped,
}

/// The point where reply emails come back into a ticket (off by default: the node's `inbound_email` policy).
#[async_trait::async_trait]
pub trait InboundMail: Send + Sync {
    /// Reply key of the ticket: the `Reply-To` address of the notification email is `support+<key>@<domain>`.
    async fn reply_address_key(
        &self,
        tenant: &TenantContext,
        ticket_id: Uuid,
    ) -> Result<String, Error>;

    /// Receives an email; never creates a new ticket.
    async fn receive(
        &self,
        tenant: &TenantContext,
        mail: InboundEmail,
    ) -> Result<InboundOutcome, Error>;
}

/// A buyer's question that must be decided for automatic reply or not (a ticket or a step 61 chat).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundQuestion {
    /// The node receiving the question: its automatic-reply policy and canned answers apply.
    pub org_id: Uuid,
    /// The Q&A thread: `support.ticket:<id>` or `conversations.thread:<id>`.
    pub thread: String,
    pub requester: Requester,
    pub text: String,
    /// The buyer's language (`vi`, `en`).
    pub locale: String,
    /// Number of consecutive automatic replies already sent in the thread.
    pub auto_replies_in_row: u32,
}

/// Source of an automatic reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoSource {
    /// An approved canned answer (verbatim, variables filled in).
    Macro(Uuid),
    /// A sentence template of a [`FactSource`].
    Fact { intent: String },
}

/// The decision for a question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyDecision {
    /// Send `text` verbatim (labeled "Automatic reply" and with a "Talk to an agent" button).
    Send { text: String, source: AutoSource },
    /// Not eligible: leave it to staff to answer (the caller may draft an AI suggestion).
    Draft { text: String },
    /// Hand over to a person immediately (risk, blocking keyword, too many replies, turned off).
    Handoff,
}

/// The automatic responder (D-230) — `conversations` (step 61) calls it for chat messages.
#[async_trait::async_trait]
pub trait AutoResponder: Send + Sync {
    async fn decide(
        &self,
        tenant: &TenantContext,
        question: InboundQuestion,
    ) -> Result<ReplyDecision, Error>;
}
