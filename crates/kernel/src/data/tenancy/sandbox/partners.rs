//! Partners forced onto fake implementations in a sandbox (step 38, D-232, D-20261119-step38-sandbox-isolation).
//!
//! The kernel wraps EVERY implementation of a partner contract as soon as a module publishes it
//! (`StartContext::provide` / `contribute`): payments, payouts, balance movements, SMS, ZNS, push
//! notifications, e-invoices, eKYC, mail and voice providers (step 61, D-232). A call for a sandbox tenant
//! does not reach the partner: the wrapper returns a deterministic fake result (successful payment, invoice
//! number `SANDBOX-…`, eKYC approved) and records it in the fake mailbox ([`super::mailbox`]); a real tenant
//! goes straight to the original implementation. Business modules do not branch.
//!
//! Mail is a conditional exception: mail to the verified email of a staff member who may enter the sandbox
//! ([`SandboxRecipients`], published by the `auth` module) is still really sent; other recipients go to the
//! fake mailbox.
//!
//! Whether a tenant is a sandbox is looked up in the tenant registry by ID ([`DatabaseRegistry::tenant`],
//! with a TTL cache); single-tenant mode never has a sandbox so it does no lookup.
//!
//! Not wrapped: `TextModel` / `Embedder` (no legal or monetary consequence; the AI quota is counted separately
//! for sandboxes), `ExchangeRateSource` (reads public data only), `Storage`.

use crate::time::{Date, Timestamp};
use jiff::SignedDuration;
use std::any::Any;
use std::sync::{Arc, OnceLock};

use axum::http::HeaderMap;
use bytes::Bytes;
use serde_json::json;

use super::mailbox::{HeldChannel, HeldMessage, HeldReason, hold_back};
use crate::einvoice::{
    EInvoice, EInvoiceCancel, EInvoiceCapabilities, EInvoiceFormat, EInvoiceProvider,
    EInvoiceStatus, IssuedEInvoice,
};
use crate::error::Error;
use crate::identity::{
    IdentityVerifier, OutcomeStatus, VerificationMethod, VerificationOutcome, VerificationRequest,
    VerificationSession,
};
use crate::mail::{Mail, Mailer};
use crate::money::Money;
use crate::payment::{
    BankAccountRef, BankBatch, BankFeed, BankTransaction, GatewayCapabilities, GatewayNotification,
    GatewayPayment, GatewayPaymentStatus, GatewayPayout, GatewayRefund, PaymentGateway,
    PaymentIntent, PayoutGateway, PayoutIntent, PayoutStatus, RefundIntent, RefundStatus,
    StatementPage,
};
use crate::push::{
    DeviceToken, MobilePushSender, PushContent, PushDelivery, WebPushSender, WebPushSubscription,
};
use crate::sms::{SmsMessage, SmsReceipt, SmsSender};
use crate::tenancy::{DatabaseRegistry, TenancyMode, TenantContext, TenantId};
use crate::voice::{
    BrowserSession, BrowserSessionRequest, CallStatus, OutboundCall, ProviderCall,
    VoiceCapabilities, VoiceNotification, VoiceProvider,
};
use crate::zns::{ZnsMessage, ZnsSender};

/// Prefix of every code issued by the fake implementations: never a legal basis at a real tenant.
pub const SANDBOX_PREFIX: &str = "SANDBOX-";
/// Prefix of fake eKYC session codes (`fetch` carries no tenant, so a fake session is recognized by its code).
const IDENTITY_PREFIX: &str = "sandbox:";

/// Who receives real mail from a sandbox: the verified email of staff of the source tenant who may enter the
/// sandbox. The `auth` module publishes it with `context.provide::<dyn SandboxRecipients>(…)`; if nobody
/// publishes it, all sandbox mail goes to the fake mailbox.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait SandboxRecipients: Send + Sync {
    /// `email` (normalized to lowercase, no display name) receives mail from sandbox `sandbox`.
    async fn allowed(&self, sandbox: &TenantContext, email: &str) -> Result<bool, Error>;
}

/// Decides "is this tenant a sandbox" for the wrappers. Cheap to clone.
#[derive(Clone)]
pub(crate) struct PartnerGuard(Arc<GuardInner>);

struct GuardInner {
    registry: DatabaseRegistry,
    recipients: OnceLock<Option<Arc<dyn SandboxRecipients>>>,
}

impl PartnerGuard {
    pub(crate) fn new(registry: DatabaseRegistry) -> Self {
        PartnerGuard(Arc::new(GuardInner { registry, recipients: OnceLock::new() }))
    }

    /// Fills in the publisher of [`SandboxRecipients`] after every module has started.
    pub(crate) fn fill(&self, recipients: Option<Arc<dyn SandboxRecipients>>) {
        let _ = self.0.recipients.set(recipients);
    }

    /// The context of tenant `tenant` if it is a sandbox.
    async fn sandbox(&self, tenant: TenantId) -> Result<Option<TenantContext>, Error> {
        if self.0.registry.mode() == TenancyMode::Single {
            return Ok(None);
        }
        match self.0.registry.tenant(tenant).await {
            Ok(context) => Ok(context.is_sandbox().then_some(context)),
            // Khách không có trong sổ (test dựng ngữ cảnh tay…): không phải sandbox.
            Err(error) if error.code() == "tenant_not_found" => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn mail_allowed(&self, sandbox: &TenantContext, to: &str) -> Result<bool, Error> {
        let Some(Some(recipients)) = self.0.recipients.get() else { return Ok(false) };
        let Some(email) = address_of(to) else { return Ok(false) };
        recipients.allowed(sandbox, &email).await
    }

    /// Wraps `api` if `T` is a partner contract; returns `api` unchanged for every other trait.
    //
    // `T` chỉ biết lúc biên dịch là "một kiểu nào đó": so `TypeId` (qua `dyn Any`) với từng hợp
    // đồng để lấy ra `Arc<dyn Hợp đồng>`, bọc, rồi ép ngược về `Arc<T>` — cả hai lần ép đều có
    // kiểm tra, sai kiểu thì trả `None` chứ không hỏng bộ nhớ.
    pub(crate) fn wrap<T>(&self, api: Arc<T>) -> Arc<T>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        macro_rules! guard {
            ($contract:path, $wrapper:ident) => {
                if let Some(inner) = (&api as &dyn Any).downcast_ref::<Arc<dyn $contract>>() {
                    let wrapped: Arc<dyn $contract> =
                        Arc::new($wrapper { inner: inner.clone(), guard: self.clone() });
                    let boxed: Box<dyn Any> = Box::new(wrapped);
                    if let Ok(back) = boxed.downcast::<Arc<T>>() {
                        return *back;
                    }
                }
            };
        }
        guard!(PaymentGateway, GuardedPayments);
        guard!(PayoutGateway, GuardedPayouts);
        guard!(BankFeed, GuardedBankFeed);
        guard!(SmsSender, GuardedSms);
        guard!(ZnsSender, GuardedZns);
        guard!(WebPushSender, GuardedWebPush);
        guard!(MobilePushSender, GuardedMobilePush);
        guard!(EInvoiceProvider, GuardedEInvoices);
        guard!(IdentityVerifier, GuardedIdentity);
        guard!(Mailer, GuardedMailer);
        guard!(VoiceProvider, GuardedVoice);
        api
    }
}

/// The email in `Ada <ada@example.com>` or `ada@example.com`, lowercase.
fn address_of(to: &str) -> Option<String> {
    let raw = match (to.rfind('<'), to.rfind('>')) {
        (Some(start), Some(end)) if start < end => &to[start + 1..end],
        _ => to,
    };
    let email = raw.trim().to_lowercase();
    email.contains('@').then_some(email)
}

/// Records a simulated partner call in the fake mailbox.
async fn simulated(
    sandbox: &TenantContext,
    channel: HeldChannel,
    provider: &str,
    recipient: Option<&str>,
    subject: &str,
    body: serde_json::Value,
) -> Result<(), Error> {
    let mut message =
        HeldMessage::new(channel, provider, HeldReason::Simulated).subject(subject).body(body);
    if let Some(recipient) = recipient {
        message = message.recipient(recipient);
    }
    hold_back(sandbox, message).await
}

fn money(amount: Money) -> serde_json::Value {
    json!({"minor": amount.minor(), "currency": amount.currency().code()})
}

// ---- Thanh toán ------------------------------------------------------------------------------

struct GuardedPayments {
    inner: Arc<dyn PaymentGateway>,
    guard: PartnerGuard,
}

/// Fake transaction code carrying the amount: `payment_status` returns exactly the collected amount without storing anything.
fn payment_ref(intent: &PaymentIntent) -> String {
    format!(
        "{SANDBOX_PREFIX}{}-{}-{}",
        intent.payment_id,
        intent.amount.minor(),
        intent.amount.currency().code()
    )
}

fn amount_of_ref(reference: &str) -> Option<Money> {
    let rest = reference.strip_prefix(SANDBOX_PREFIX)?;
    let mut parts = rest.rsplitn(3, '-');
    let currency = parts.next()?;
    let minor = parts.next()?.parse().ok()?;
    Money::from_columns(minor, currency).ok()
}

#[async_trait::async_trait]
impl PaymentGateway for GuardedPayments {
    fn provider(&self) -> &'static str {
        self.inner.provider()
    }

    fn capabilities(&self) -> GatewayCapabilities {
        self.inner.capabilities()
    }

    async fn create_payment(
        &self,
        tenant: TenantId,
        intent: &PaymentIntent,
    ) -> Result<GatewayPayment, Error> {
        let Some(sandbox) = self.guard.sandbox(tenant).await? else {
            return self.inner.create_payment(tenant, intent).await;
        };
        let body = json!({
            "payment_id": intent.payment_id,
            "amount": money(intent.amount),
            "method": intent.method,
            "description": intent.description,
        });
        simulated(
            &sandbox,
            HeldChannel::Partner,
            self.inner.provider(),
            None,
            "payment.create",
            body,
        )
        .await?;
        Ok(GatewayPayment::new(payment_ref(intent)))
    }

    async fn refund(
        &self,
        tenant: TenantId,
        intent: &RefundIntent,
    ) -> Result<GatewayRefund, Error> {
        let Some(sandbox) = self.guard.sandbox(tenant).await? else {
            return self.inner.refund(tenant, intent).await;
        };
        let body = json!({
            "refund_id": intent.refund_id,
            "payment_ref": intent.payment_provider_ref,
            "amount": money(intent.amount),
        });
        simulated(
            &sandbox,
            HeldChannel::Partner,
            self.inner.provider(),
            None,
            "payment.refund",
            body,
        )
        .await?;
        let reference = format!("{SANDBOX_PREFIX}REFUND-{}", intent.refund_id);
        Ok(GatewayRefund::new(reference, RefundStatus::Succeeded))
    }

    async fn payment_status(
        &self,
        tenant: TenantId,
        provider_ref: &str,
    ) -> Result<GatewayPaymentStatus, Error> {
        if self.guard.sandbox(tenant).await?.is_none() {
            return self.inner.payment_status(tenant, provider_ref).await;
        }
        Ok(match amount_of_ref(provider_ref) {
            Some(amount) => GatewayPaymentStatus::Succeeded { amount },
            None => GatewayPaymentStatus::Pending,
        })
    }

    async fn verify_webhook(
        &self,
        tenant: TenantId,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<GatewayNotification, Error> {
        // Đối tác thật không biết giao dịch giả nên không gửi webhook cho sandbox; kiểm chữ ký thật.
        self.inner.verify_webhook(tenant, headers, body).await
    }

    fn webhook_ack(&self) -> Option<crate::provider::WebhookAck> {
        self.inner.webhook_ack()
    }

    async fn statement(
        &self,
        tenant: TenantId,
        from: Date,
        to: Date,
        cursor: Option<String>,
    ) -> Result<StatementPage, Error> {
        if self.guard.sandbox(tenant).await?.is_none() {
            return self.inner.statement(tenant, from, to, cursor).await;
        }
        Ok(StatementPage::new(Vec::new(), None))
    }
}

struct GuardedPayouts {
    inner: Arc<dyn PayoutGateway>,
    guard: PartnerGuard,
}

#[async_trait::async_trait]
impl PayoutGateway for GuardedPayouts {
    fn provider(&self) -> &'static str {
        self.inner.provider()
    }

    async fn payout(
        &self,
        tenant: TenantId,
        intent: &PayoutIntent,
    ) -> Result<GatewayPayout, Error> {
        let Some(sandbox) = self.guard.sandbox(tenant).await? else {
            return self.inner.payout(tenant, intent).await;
        };
        let body = json!({
            "payout_id": intent.payout_id,
            "payee": intent.payee,
            "amount": money(intent.amount),
        });
        simulated(
            &sandbox,
            HeldChannel::Partner,
            self.inner.provider(),
            None,
            "payout.create",
            body,
        )
        .await?;
        let reference = format!("{SANDBOX_PREFIX}PAYOUT-{}", intent.payout_id);
        Ok(GatewayPayout::new(reference, PayoutStatus::Succeeded))
    }

    async fn payout_status(
        &self,
        tenant: TenantId,
        provider_ref: &str,
    ) -> Result<PayoutStatus, Error> {
        if self.guard.sandbox(tenant).await?.is_none() {
            return self.inner.payout_status(tenant, provider_ref).await;
        }
        Ok(PayoutStatus::Succeeded)
    }
}

struct GuardedBankFeed {
    inner: Arc<dyn BankFeed>,
    guard: PartnerGuard,
}

#[async_trait::async_trait]
impl BankFeed for GuardedBankFeed {
    fn provider(&self) -> &'static str {
        self.inner.provider()
    }

    async fn verify_webhook(
        &self,
        tenant: TenantId,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<Vec<BankTransaction>, Error> {
        self.inner.verify_webhook(tenant, headers, body).await
    }

    fn webhook_ack(&self) -> Option<crate::provider::WebhookAck> {
        self.inner.webhook_ack()
    }

    async fn fetch(
        &self,
        tenant: TenantId,
        account: &BankAccountRef,
        cursor: Option<String>,
    ) -> Result<BankBatch, Error> {
        if self.guard.sandbox(tenant).await?.is_none() {
            return self.inner.fetch(tenant, account, cursor).await;
        }
        // Sandbox không đọc tài khoản ngân hàng thật: không có biến động.
        Ok(BankBatch::new(Vec::new(), cursor))
    }
}

// ---- Tin nhắn --------------------------------------------------------------------------------

struct GuardedSms {
    inner: Arc<dyn SmsSender>,
    guard: PartnerGuard,
}

#[async_trait::async_trait]
impl SmsSender for GuardedSms {
    fn provider(&self) -> &'static str {
        self.inner.provider()
    }

    async fn send(&self, message: SmsMessage) -> Result<SmsReceipt, Error> {
        let Some(sandbox) = self.guard.sandbox(message.tenant).await? else {
            return self.inner.send(message).await;
        };
        let body = json!({"text": message.text, "reference": message.reference});
        let to = message.to.as_str().to_owned();
        simulated(&sandbox, HeldChannel::Sms, self.inner.provider(), Some(&to), "sms", body)
            .await?;
        Ok(SmsReceipt::new(format!("{SANDBOX_PREFIX}SMS-{}", message.reference)))
    }
}

struct GuardedZns {
    inner: Arc<dyn ZnsSender>,
    guard: PartnerGuard,
}

#[async_trait::async_trait]
impl ZnsSender for GuardedZns {
    fn provider(&self) -> &'static str {
        self.inner.provider()
    }

    async fn send(&self, message: ZnsMessage) -> Result<String, Error> {
        let Some(sandbox) = self.guard.sandbox(message.tenant).await? else {
            return self.inner.send(message).await;
        };
        let body = json!({
            "template_id": message.template_id,
            "params": message.params,
            "reference": message.reference,
        });
        let to = message.to.as_str().to_owned();
        simulated(&sandbox, HeldChannel::Zns, self.inner.provider(), Some(&to), "zns", body)
            .await?;
        Ok(format!("{SANDBOX_PREFIX}ZNS-{}", message.reference))
    }
}

fn push_body(content: &PushContent) -> serde_json::Value {
    json!({"body": content.body, "link": content.link, "reference": content.reference})
}

struct GuardedWebPush {
    inner: Arc<dyn WebPushSender>,
    guard: PartnerGuard,
}

#[async_trait::async_trait]
impl WebPushSender for GuardedWebPush {
    fn provider(&self) -> &'static str {
        self.inner.provider()
    }

    fn public_key(&self) -> String {
        self.inner.public_key()
    }

    async fn send(
        &self,
        tenant: TenantId,
        subscription: &WebPushSubscription,
        content: &PushContent,
    ) -> Result<PushDelivery, Error> {
        let Some(sandbox) = self.guard.sandbox(tenant).await? else {
            return self.inner.send(tenant, subscription, content).await;
        };
        // Endpoint của trình duyệt là định danh thiết bị: không ghi.
        simulated(
            &sandbox,
            HeldChannel::Push,
            self.inner.provider(),
            Some("web"),
            &content.title,
            push_body(content),
        )
        .await?;
        let provider_reference = format!("{SANDBOX_PREFIX}PUSH-{}", content.reference);
        Ok(PushDelivery::Sent { provider_reference })
    }
}

struct GuardedMobilePush {
    inner: Arc<dyn MobilePushSender>,
    guard: PartnerGuard,
}

#[async_trait::async_trait]
impl MobilePushSender for GuardedMobilePush {
    fn provider(&self) -> &'static str {
        self.inner.provider()
    }

    async fn send(
        &self,
        tenant: TenantId,
        device: &DeviceToken,
        content: &PushContent,
    ) -> Result<PushDelivery, Error> {
        let Some(sandbox) = self.guard.sandbox(tenant).await? else {
            return self.inner.send(tenant, device, content).await;
        };
        simulated(
            &sandbox,
            HeldChannel::Push,
            self.inner.provider(),
            Some(&device.platform),
            &content.title,
            push_body(content),
        )
        .await?;
        let provider_reference = format!("{SANDBOX_PREFIX}PUSH-{}", content.reference);
        Ok(PushDelivery::Sent { provider_reference })
    }
}

// ---- Hoá đơn điện tử ----------------------------------------------------------------------------

struct GuardedEInvoices {
    inner: Arc<dyn EInvoiceProvider>,
    guard: PartnerGuard,
}

#[async_trait::async_trait]
impl EInvoiceProvider for GuardedEInvoices {
    fn provider(&self) -> &'static str {
        self.inner.provider()
    }

    fn capabilities(&self) -> EInvoiceCapabilities {
        self.inner.capabilities()
    }

    async fn issue(&self, tenant: TenantId, invoice: &EInvoice) -> Result<IssuedEInvoice, Error> {
        let Some(sandbox) = self.guard.sandbox(tenant).await? else {
            return self.inner.issue(tenant, invoice).await;
        };
        let body = json!({
            "document_id": invoice.document_id,
            "series": invoice.series,
            "number": invoice.number,
            "lines": invoice.lines.len(),
        });
        simulated(
            &sandbox,
            HeldChannel::Partner,
            self.inner.provider(),
            None,
            "einvoice.issue",
            body,
        )
        .await?;
        let reference = format!("{SANDBOX_PREFIX}{}", invoice.document_id);
        // Số giả: số hệ thống đã cấp, hay 1 khi nhà cung cấp lẽ ra cấp số.
        let number = invoice.number.or(Some(1));
        Ok(IssuedEInvoice::new(reference.clone()).number(number).lookup_code(reference))
    }

    async fn status(&self, tenant: TenantId, provider_ref: &str) -> Result<EInvoiceStatus, Error> {
        if self.guard.sandbox(tenant).await?.is_none() {
            return self.inner.status(tenant, provider_ref).await;
        }
        let authority_code = Some(format!("{SANDBOX_PREFIX}AUTHORITY"));
        Ok(EInvoiceStatus::Accepted { authority_code, number: None })
    }

    async fn cancel(&self, tenant: TenantId, request: &EInvoiceCancel) -> Result<(), Error> {
        let Some(sandbox) = self.guard.sandbox(tenant).await? else {
            return self.inner.cancel(tenant, request).await;
        };
        let body = json!({"provider_ref": request.provider_ref, "reason": request.reason});
        simulated(
            &sandbox,
            HeldChannel::Partner,
            self.inner.provider(),
            None,
            "einvoice.cancel",
            body,
        )
        .await
    }

    async fn download(
        &self,
        tenant: TenantId,
        provider_ref: &str,
        format: EInvoiceFormat,
    ) -> Result<Bytes, Error> {
        if self.guard.sandbox(tenant).await?.is_none() {
            return self.inner.download(tenant, provider_ref, format).await;
        }
        let text = match format {
            EInvoiceFormat::Xml => {
                format!(
                    "<SandboxInvoice><Ref>{provider_ref}</Ref><Legal>false</Legal></SandboxInvoice>"
                )
            }
            EInvoiceFormat::Pdf => format!("SANDBOX INVOICE {provider_ref} - NOT A LEGAL DOCUMENT"),
        };
        Ok(Bytes::from(text))
    }
}

// ---- eKYC ------------------------------------------------------------------------------------

struct GuardedIdentity {
    inner: Arc<dyn IdentityVerifier>,
    guard: PartnerGuard,
}

#[async_trait::async_trait]
impl IdentityVerifier for GuardedIdentity {
    fn provider(&self) -> &'static str {
        self.inner.provider()
    }

    fn methods(&self) -> Vec<VerificationMethod> {
        self.inner.methods()
    }

    async fn start(&self, request: VerificationRequest) -> Result<VerificationSession, Error> {
        let Some(sandbox) = self.guard.sandbox(request.tenant).await? else {
            return self.inner.start(request).await;
        };
        let body = json!({"attempt_id": request.attempt_id});
        simulated(
            &sandbox,
            HeldChannel::Partner,
            self.inner.provider(),
            None,
            "identity.start",
            body,
        )
        .await?;
        Ok(VerificationSession::Pending {
            provider_reference: format!("{IDENTITY_PREFIX}{}", request.attempt_id),
        })
    }

    async fn fetch(&self, provider_reference: &str) -> Result<VerificationOutcome, Error> {
        if !provider_reference.starts_with(IDENTITY_PREFIX) {
            return self.inner.fetch(provider_reference).await;
        }
        // Đủ mục người bán của NĐ 248/2026 Điều 18 (quy tắc `kyc.required_activities` từ 01/01/2027): luồng thử
        // ở sandbox không hỏng từ ngày đó.
        let mut outcome = VerificationOutcome::new(provider_reference, OutcomeStatus::Verified)
            .verified_name("Sandbox")
            .assurance("sandbox")
            .address("Sandbox address")
            .representative("Sandbox");
        if let (Some(birth), Some(until)) = (Date::new(1990, 1, 1), Date::new(2099, 12, 31)) {
            outcome = outcome.birth_date(birth).document_validity("VN", until);
        }
        outcome.expires_at = Some(Timestamp::now() + SignedDuration::from_hours(24 * 30));
        Ok(outcome)
    }

    async fn verify_callback(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<VerificationOutcome, Error> {
        self.inner.verify_callback(headers, body).await
    }
}

// ---- Mail ------------------------------------------------------------------------------------

struct GuardedMailer {
    inner: Arc<dyn Mailer>,
    guard: PartnerGuard,
}

#[async_trait::async_trait]
impl Mailer for GuardedMailer {
    async fn send(&self, tenant: &TenantContext, mail: Mail) -> Result<(), Error> {
        if !tenant.is_sandbox() || self.guard.mail_allowed(tenant, &mail.to).await? {
            return self.inner.send(tenant, mail).await;
        }
        let message = HeldMessage::new(HeldChannel::Mail, "mail", HeldReason::RecipientNotAllowed)
            .recipient(mail.to.clone())
            .subject(mail.subject.clone())
            .body(json!({"text": mail.text, "html": mail.html.is_some()}));
        hold_back(tenant, message).await
    }
}

// ---- Thoại ----------------------------------------------------------------------------------

/// A sandbox makes no real calls: WebRTC sessions and outgoing calls are fake (code `SANDBOX-…`), recorded in
/// the fake mailbox with the number masked; there is no recording.
struct GuardedVoice {
    inner: Arc<dyn VoiceProvider>,
    guard: PartnerGuard,
}

#[async_trait::async_trait]
impl VoiceProvider for GuardedVoice {
    fn provider(&self) -> &'static str {
        self.inner.provider()
    }

    fn capabilities(&self) -> VoiceCapabilities {
        self.inner.capabilities()
    }

    async fn browser_session(
        &self,
        tenant: TenantId,
        request: &BrowserSessionRequest,
    ) -> Result<BrowserSession, Error> {
        let Some(sandbox) = self.guard.sandbox(tenant).await? else {
            return self.inner.browser_session(tenant, request).await;
        };
        let body = json!({"identity": request.identity, "call_id": request.call_id});
        let hotline = request.hotline.masked();
        simulated(
            &sandbox,
            HeldChannel::Partner,
            self.provider(),
            Some(&hotline),
            "voice.session",
            body,
        )
        .await?;
        Ok(BrowserSession {
            token: format!("{SANDBOX_PREFIX}VOICE-{}", uuid::Uuid::now_v7()),
            expires_at: Timestamp::now() + SignedDuration::from_mins(10),
        })
    }

    async fn place_call(
        &self,
        tenant: TenantId,
        call: &OutboundCall,
    ) -> Result<ProviderCall, Error> {
        let Some(sandbox) = self.guard.sandbox(tenant).await? else {
            return self.inner.place_call(tenant, call).await;
        };
        let body = json!({"call_id": call.call_id, "record": call.record});
        let to = call.to.masked();
        simulated(&sandbox, HeldChannel::Partner, self.provider(), Some(&to), "voice.call", body)
            .await?;
        Ok(ProviderCall {
            provider_call_id: format!("{SANDBOX_PREFIX}CALL-{}", call.call_id),
            status: CallStatus::Queued,
        })
    }

    fn verify_webhook(
        &self,
        tenant: TenantId,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<Vec<VoiceNotification>, Error> {
        // Nhà cung cấp thật không biết cuộc gọi giả nên không gửi webhook cho sandbox; kiểm chữ ký thật.
        self.inner.verify_webhook(tenant, headers, body)
    }

    async fn fetch_recording(
        &self,
        tenant: TenantId,
        provider_recording_id: &str,
    ) -> Result<crate::storage::ByteStream, Error> {
        if self.guard.sandbox(tenant).await?.is_none() {
            return self.inner.fetch_recording(tenant, provider_recording_id).await;
        }
        // Sandbox không có cuộc gọi thật nên không có bản ghi.
        Err(Error::not_found(
            "voice.recording_not_found",
            "The voice provider has no recording `{id}`.",
        )
        .with_param("id", provider_recording_id))
    }

    async fn delete_recording(
        &self,
        tenant: TenantId,
        provider_recording_id: &str,
    ) -> Result<(), Error> {
        if self.guard.sandbox(tenant).await?.is_none() {
            return self.inner.delete_recording(tenant, provider_recording_id).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lay_dia_chi_mail_tu_ten_hien_thi() {
        assert_eq!(address_of("Ada <Ada@Example.com>").as_deref(), Some("ada@example.com"));
        assert_eq!(address_of(" bob@example.com ").as_deref(), Some("bob@example.com"));
        assert_eq!(address_of("not an address"), None);
    }

    #[test]
    fn ma_thanh_toan_gia_mang_so_tien() {
        let amount = Money::from_columns(125_000, "VND").unwrap();
        let intent =
            PaymentIntent::new(uuid::Uuid::now_v7(), amount, "card", "Order", Timestamp::now());
        let reference = payment_ref(&intent);
        assert!(reference.starts_with(SANDBOX_PREFIX));
        assert_eq!(amount_of_ref(&reference), Some(amount));
        assert_eq!(amount_of_ref("vnpay-123"), None);
    }
}
