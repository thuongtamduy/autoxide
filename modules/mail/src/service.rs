//! `Mailer` của module: đưa mail vào job queue; job gửi qua [`Transport`].

use std::sync::Arc;

use autoxide_kernel::database::Database;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler, JobQueue};
use autoxide_kernel::mail::{Mail, Mailer};
use lettre::Message;
use lettre::message::{Mailbox, MultiPart, SinglePart};
use serde::{Deserialize, Serialize};

use crate::transport::Transport;

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
}

impl Job for SendMail {
    const KIND: &'static str = SEND;
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
        if mail.to.parse::<Mailbox>().is_err() {
            return Err(Error::validation(vec![FieldError::new(
                "to",
                "email",
                "must be a mail address, like `ada@example.com`",
            )]));
        }
        let job = SendMail { to: mail.to, subject: mail.subject, text: mail.text, html: mail.html };
        let id = self.queue.enqueue(self.database.write(), &job, EnqueueOptions::default()).await?;
        // Không ghi địa chỉ hay nội dung vào log: chỉ ID của job.
        tracing::info!(job_id = ?id, "mail queued");
        Ok(())
    }
}

/// Bộ chạy job gửi mail.
pub(crate) struct Sender {
    transport: Arc<Transport>,
    from: Mailbox,
}

impl Sender {
    pub(crate) fn new(transport: Arc<Transport>, from: Mailbox) -> Self {
        Sender { transport, from }
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
        let builder = Message::builder().from(self.from.clone()).to(to).subject(mail.subject);
        let message = match mail.html {
            Some(html) => builder.multipart(MultiPart::alternative_plain_html(mail.text, html)),
            None => builder.singlepart(SinglePart::plain(mail.text)),
        }
        .map_err(|error| Error::internal(format!("cannot build the mail: {error}")))?;
        self.transport.deliver(message).await?;
        tracing::info!(job_id = %job.id, "mail sent");
        Ok(())
    }
}
