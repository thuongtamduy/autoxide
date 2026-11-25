//! `Mailer` của module: đưa mail vào job queue; job gửi qua [`Transport`].
//!
//! Theo khách (bước 33): mail xếp hàng trên database của khách gửi nó; job gửi qua máy chủ SMTP và
//! địa chỉ người gửi của khách đó khi khách đã đặt (`mail.connection.smtp.url`,
//! `mail.connection.smtp.from` — D-274; tên theo quy ước kết nối đối tác, tên cũ `mail.smtp_url` /
//! `mail.from` vẫn được nhận), không thì qua cấu hình chung của máy chủ.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Event, Outbox};
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler, JobQueue};
use autoxide_kernel::mail::{DELIVERY_FAILED, Mail, Mailer};
use autoxide_kernel::tenancy::{CONNECTION_GROUP, TenantContext, TenantSetting, TenantSettings};
use lettre::Message;
use lettre::message::header::{HeaderName, HeaderValue};
use lettre::message::{Mailbox, MultiPart, SinglePart};
use serde::{Deserialize, Serialize};

use crate::bounces::Bounces;
use crate::transport::{Delivery, Transport};

/// Loại job gửi một mail.
pub(crate) const SEND: &str = "mail.send";

/// Payload của job: chính nội dung mail. Nằm trong `mail_jobs` tới khi job xong và
/// được dọn (7 ngày) — đừng gửi mật khẩu qua mail.
#[derive(Debug, Serialize, Deserialize)]
struct SendMail {
    to: String,
    subject: String,
    text: String,
    html: Option<String>,
    // `serde(default)`: job xếp hàng trước khi có các trường này vẫn đọc được.
    #[serde(default)]
    reply_to: Option<String>,
    #[serde(default)]
    headers: Vec<(String, String)>,
    #[serde(default)]
    dedupe_key: Option<String>,
}

impl Job for SendMail {
    const KIND: &'static str = SEND;
}

/// Event [`DELIVERY_FAILED`]: máy chủ nhận từ chối hẳn một mail.
#[derive(Debug, Serialize)]
pub(crate) struct DeliveryFailed {
    to: String,
    dedupe_key: Option<String>,
    code: String,
}

impl Event for DeliveryFailed {
    const NAME: &'static str = DELIVERY_FAILED;
}

/// Máy chủ SMTP riêng của khách (URL có mật khẩu: lưu mã hoá, không bao giờ trả ra).
pub(crate) const TENANT_SMTP_URL: TenantSetting = TenantSetting::new(
    "mail.connection.smtp.url",
    "SMTP server this tenant's mail goes through, like `smtps://user:password@smtp.example.com`; \
     unset: the server's own (`AUTOXIDE_MAIL_SMTP_URL`).",
)
.secret()
.group(CONNECTION_GROUP)
.formerly("mail.smtp_url")
.checked(check_smtp_url);

/// `AUTOXIDE_MAIL_ALLOW_PRIVATE_NETWORKS` cho bộ kiểm LÚC LƯU: bộ kiểm cấu hình của khách là con trỏ
/// hàm (không mang trạng thái) nên đọc cờ ở đây, đặt lúc module khởi động. Chỉ bật, không tắt lại
/// (một process chạy một module `mail`; test chạy song song không giẫm nhau). Lúc KẾT NỐI dùng cờ
/// của chính [`Sender`].
static ALLOW_PRIVATE_NETWORKS: AtomicBool = AtomicBool::new(false);

pub(crate) fn allow_private_networks(allow: bool) {
    ALLOW_PRIVATE_NETWORKS.fetch_or(allow, Ordering::Relaxed);
}

fn private_allowed() -> bool {
    ALLOW_PRIVATE_NETWORKS.load(Ordering::Relaxed)
}

/// Kiểm lúc khách lưu (D-20261121-ssrf-targets): `smtp://` / `smtps://`, host không bị biết trước là
/// nội bộ. Tên miền được kiểm lại lúc kết nối ([`Transport::tenant_smtp`]).
fn check_smtp_url(raw: &str) -> Result<(), FieldError> {
    autoxide_kernel::net::check_target_url(raw, &crate::transport::SCHEMES, private_allowed())
        .map(|_| ())
}

/// Kết nối SMTP riêng của khách được dùng lại chừng này rồi phân giải lại (IP của máy chủ có thể đổi).
const TENANT_TRANSPORT_TTL: Duration = Duration::from_secs(600);

/// Người gửi riêng của khách. Cùng nhóm kết nối: địa chỉ gửi đi kèm tài khoản SMTP (tên miền của
/// nó phải được máy chủ SMTP cho gửi).
pub(crate) const TENANT_FROM: TenantSetting = TenantSetting::new(
    "mail.connection.smtp.from",
    "Sender of this tenant's mail, like `Shop <no-reply@shop.example>`; unset: the server's own \
     (`AUTOXIDE_MAIL_FROM`). Its domain must be allowed to send through the SMTP server.",
)
.group(CONNECTION_GROUP)
.formerly("mail.from");

pub(crate) struct MailService {
    queue: JobQueue,
    /// Sổ dội: thư tới địa chỉ đang chặn không xếp hàng (D-20261125-mail-bounces).
    bounces: Arc<Bounces>,
}

impl MailService {
    pub(crate) fn new(queue: JobQueue, bounces: Arc<Bounces>) -> Self {
        MailService { queue, bounces }
    }
}

#[async_trait::async_trait]
impl Mailer for MailService {
    async fn send(&self, tenant: &TenantContext, mail: Mail) -> Result<(), Error> {
        let mut problems = Vec::new();
        if mail.to.parse::<Mailbox>().is_err() {
            problems.push(FieldError::new(
                "to",
                "email",
                "must be a mail address, like `ada@example.com`",
            ));
        }
        if mail.reply_to.as_deref().is_some_and(|reply_to| reply_to.parse::<Mailbox>().is_err()) {
            problems.push(FieldError::new(
                "reply_to",
                "email",
                "must be a mail address, like `ada@example.com`",
            ));
        }
        problems.extend(mail.header_problems());
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        // Địa chỉ đang chặn ở khách này: không xếp hàng, để dấu vết trên sổ dội; trả `Ok` như thường để API
        // (quên mật khẩu…) không để lộ địa chỉ nào bị chặn.
        if let Some(address) = crate::bounces::address_of(&mail.to)
            && self.bounces.skip_if_suppressed(tenant, &address, mail.dedupe_key.as_deref()).await?
        {
            return Ok(());
        }
        // Khoá chống trùng của bên gửi thành `unique_key` của job: chạy lại không gửi hai lần.
        let mut options = EnqueueOptions::default();
        if let Some(key) = &mail.dedupe_key {
            options = options.unique_key(format!("mail:{key}"));
        }
        let job = SendMail {
            to: mail.to,
            subject: mail.subject,
            text: mail.text,
            html: mail.html,
            reply_to: mail.reply_to,
            headers: mail.headers,
            dedupe_key: mail.dedupe_key,
        };
        let id = self.queue.enqueue(tenant.database().write(), tenant, &job, options).await?;
        // Không ghi địa chỉ hay nội dung vào log: chỉ ID của job.
        tracing::info!(job_id = ?id, "mail queued");
        Ok(())
    }
}

/// Kết nối SMTP (của khách, hay của máy chủ) trong sổ kết nối ra ngoài của kernel.
pub(crate) const CONNECTION: &str = "mail.connection.smtp";

/// Bộ chạy job gửi mail.
pub(crate) struct Sender {
    transport: Arc<Transport>,
    from: Mailbox,
    outbox: Outbox,
    settings: Arc<dyn TenantSettings>,
    bounces: Arc<Bounces>,
    /// `AUTOXIDE_MAIL_ALLOW_PRIVATE_NETWORKS`.
    allow_private_networks: bool,
    /// Kết nối SMTP riêng của khách, theo URL: dựng một lần (đã phân giải IP), dùng lại (pool của
    /// lettre) tới hết [`TENANT_TRANSPORT_TTL`].
    tenant_transports: Mutex<HashMap<String, (Instant, Arc<Transport>)>>,
}

impl Sender {
    pub(crate) fn new(
        transport: Arc<Transport>,
        from: Mailbox,
        outbox: Outbox,
        settings: Arc<dyn TenantSettings>,
        bounces: Arc<Bounces>,
        allow_private_networks: bool,
    ) -> Self {
        Sender {
            transport,
            from,
            outbox,
            settings,
            bounces,
            allow_private_networks,
            tenant_transports: Mutex::new(HashMap::new()),
        }
    }

    /// Đường gửi và người gửi của `tenant`: của riêng khách khi đã đặt, không thì của máy chủ.
    async fn delivery_for(
        &self,
        tenant: &TenantContext,
    ) -> Result<(Arc<Transport>, Mailbox), Error> {
        let from = match self.settings.get(tenant, &TENANT_FROM).await? {
            Some(value) => value.as_str().parse::<Mailbox>().map_err(|error| {
                Error::internal(format!(
                    "tenant setting `{}` is not a mailbox: {error}",
                    TENANT_FROM.name()
                ))
            })?,
            None => self.from.clone(),
        };
        let Some(url) = self.settings.get(tenant, &TENANT_SMTP_URL).await? else {
            return Ok((self.transport.clone(), from));
        };
        // Khoá chỉ giữ trong lúc tra / thêm vào bảng (không qua `await`); lỗi khoá (một thread
        // khác đã panic) thì dựng kết nối mới, không dùng bảng.
        if let Ok(cache) = self.tenant_transports.lock()
            && let Some((built, found)) = cache.get(url.as_str())
            && built.elapsed() < TENANT_TRANSPORT_TTL
        {
            return Ok((found.clone(), from));
        }
        let transport = Transport::tenant_smtp(url.as_str(), self.allow_private_networks)
            .await
            .map_err(smtp_unreachable)?;
        let transport = Arc::new(transport);
        if let Ok(mut cache) = self.tenant_transports.lock() {
            cache.insert(url.as_str().to_owned(), (Instant::now(), transport.clone()));
        }
        Ok((transport, from))
    }
}

#[async_trait::async_trait]
impl JobHandler for Sender {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let mail: SendMail =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        let to: Mailbox = mail.to.parse().map_err(|error| {
            Error::internal(format!("the recipient address is invalid: {error}"))
        })?;
        let tenant = job
            .tenant
            .as_ref()
            .ok_or_else(|| Error::internal("a mail job must belong to a tenant"))?;
        let (transport, from) = self.delivery_for(tenant).await?;
        let mut builder = Message::builder().from(from).to(to).subject(mail.subject);
        if let Some(reply_to) = &mail.reply_to {
            let reply_to = reply_to.parse().map_err(|error| {
                Error::internal(format!("the reply-to address is invalid: {error}"))
            })?;
            builder = builder.reply_to(reply_to);
        }
        for (name, value) in &mail.headers {
            let name = HeaderName::new_from_ascii(name.clone())
                .map_err(|error| Error::internal(format!("invalid header name: {error}")))?;
            builder = builder.raw_header(HeaderValue::new(name, value.clone()));
        }
        let message = match mail.html {
            Some(html) => builder.multipart(MultiPart::alternative_plain_html(mail.text, html)),
            None => builder.singlepart(SinglePart::plain(mail.text)),
        }
        .map_err(|error| Error::internal(format!("cannot build the mail: {error}")))?;
        // Ghi lần dùng của kết nối SMTP (bước 51): người quản trị thấy lỗi cuối mà không đọc log.
        let delivered = transport.deliver(message).await;
        let error = delivered.as_ref().err().map(|failed| failed.public.clone());
        autoxide_kernel::connections::ConnectionStatus
            .record(tenant, CONNECTION, error.as_deref())
            .await;
        match delivered.map_err(|failed| *failed.error)? {
            Delivery::Sent => {
                tracing::info!(job_id = %job.id, "mail sent");
            }
            // Lỗi vĩnh viễn: thử lại vô ích. Báo cho module khác qua event, rồi xong job.
            Delivery::Rejected { code, enhanced } => {
                tracing::warn!(job_id = %job.id, code = %code, "mail rejected for good by the receiving server");
                let address = crate::bounces::address_of(&mail.to);
                let transaction = tenant.database().begin_write().await?;
                // Sổ dội cùng transaction với event: không bao giờ có cái này mà thiếu cái kia.
                // Chỉ mã chắc chắn về NGƯỜI NHẬN mới vào sổ dội (lỗi của bên gửi không làm chặn oan người nhận).
                if let Some(address) = &address
                    && crate::bounces::counts_as_hard(enhanced.as_deref())
                {
                    let label = enhanced.as_deref().map_or(code.clone(), |e| format!("{code} {e}"));
                    self.bounces.record_hard(&transaction, tenant, address, &label).await?;
                }
                let event = DeliveryFailed { to: mail.to, dedupe_key: mail.dedupe_key, code };
                self.outbox.record(&transaction, tenant, Actor::system(), None, &event).await?;
                transaction.commit().await?;
                metrics::counter!("mail_rejected_total").increment(1);
            }
        }
        Ok(())
    }
}

fn smtp_unreachable(reason: String) -> Error {
    Error::unavailable("mail.smtp_unreachable", "Cannot use the SMTP server: {reason}.")
        .with_param("reason", reason)
}

/// Thử kết nối SMTP của khách (bước 51): mở kết nối và gửi NOOP, không gửi thư nào.
#[async_trait::async_trait]
impl autoxide_kernel::connections::ConnectionProbe for Sender {
    fn connection(&self) -> &'static str {
        CONNECTION
    }

    async fn test(&self, tenant: &TenantContext) -> Result<(), Error> {
        let (transport, _) = self.delivery_for(tenant).await?;
        match transport.as_ref() {
            crate::transport::Transport::File(_) => {
                Err(smtp_unreachable("no SMTP server is set; mail is written to files".to_owned()))
            }
            crate::transport::Transport::Smtp(smtp) => match smtp.test_connection().await {
                Ok(true) => Ok(()),
                Ok(false) => Err(smtp_unreachable("it did not answer a NOOP command".to_owned())),
                // Câu chung: không trả lỗi mạng chi tiết (D-20261121-ssrf-targets).
                Err(error) => Err(smtp_unreachable(crate::transport::describe(&error))),
            },
        }
    }
}
