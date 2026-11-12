//! `Mailer` của module: đưa mail vào job queue; job gửi qua [`Transport`].

use std::sync::Arc;

use autoxide_kernel::database::Database;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Event, Outbox};
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler, JobQueue};
use autoxide_kernel::mail::{DELIVERY_FAILED, Mail, Mailer};
use lettre::Message;
use lettre::message::header::{HeaderName, HeaderValue};
use lettre::message::{Mailbox, MultiPart, SinglePart};
use serde::{Deserialize, Serialize};

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

pub(crate) struct MailService {
    database: Database,
    queue: JobQueue,
}

impl MailService {
    pub(crate) fn new(database: Database, queue: JobQueue) -> Self {
        MailService { database, queue }
    }
}

#[async_trait::async_trait]
impl Mailer for MailService {
    async fn send(&self, mail: Mail) -> Result<(), Error> {
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
        // Khoá chống trùng của bên gửi thành `unique_key` của job: chạy lại không gửi hai lần.
        let options = EnqueueOptions {
            unique_key: mail.dedupe_key.as_ref().map(|key| format!("mail:{key}")),
            ..EnqueueOptions::default()
        };
        let job = SendMail {
            to: mail.to,
            subject: mail.subject,
            text: mail.text,
            html: mail.html,
            reply_to: mail.reply_to,
            headers: mail.headers,
            dedupe_key: mail.dedupe_key,
        };
        let id = self.queue.enqueue(self.database.write(), &job, options).await?;
        // Không ghi địa chỉ hay nội dung vào log: chỉ ID của job.
        tracing::info!(job_id = ?id, "mail queued");
        Ok(())
    }
}

/// Bộ chạy job gửi mail.
pub(crate) struct Sender {
    transport: Arc<Transport>,
    from: Mailbox,
    database: Database,
    outbox: Outbox,
}

impl Sender {
    pub(crate) fn new(
        transport: Arc<Transport>,
        from: Mailbox,
        database: Database,
        outbox: Outbox,
    ) -> Self {
        Sender { transport, from, database, outbox }
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
        let mut builder = Message::builder().from(self.from.clone()).to(to).subject(mail.subject);
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
        match self.transport.deliver(message).await? {
            Delivery::Sent => {
                tracing::info!(job_id = %job.id, "mail sent");
            }
            // Lỗi vĩnh viễn: thử lại vô ích. Báo cho module khác qua event, rồi xong job.
            Delivery::Rejected { code } => {
                tracing::warn!(job_id = %job.id, code = %code, "mail rejected for good by the receiving server");
                let event = DeliveryFailed { to: mail.to, dedupe_key: mail.dedupe_key, code };
                let transaction = self.database.begin_write().await?;
                self.outbox.record(&transaction, Actor::system(), None, &event).await?;
                transaction.commit().await?;
                metrics::counter!("mail_rejected_total").increment(1);
            }
        }
        Ok(())
    }
}
