//! Mail báo quản trị viên khi có bài gửi: nghe event `forms.submission_received` của chính
//! module (event ghi cùng transaction với bài gửi, nên không bài nào bị sót), gửi qua `dyn
//! Mailer` — mail vào job queue của module `mail`, gửi nền, lỗi tạm thời thì thử lại.
//!
//! - Người nhận là `notify_emails` do quản trị viên đặt, KHÔNG BAO GIỜ lấy từ dữ liệu gửi lên
//!   (form không thành máy chuyển tiếp thư). `Reply-To` là email người gửi (nếu có).
//! - Event được giao lại (D-054): mỗi mail mang `dedupe_key` = bài gửi + người nhận, nên giao
//!   lại không sinh mail thứ hai.
//! - Mail này gửi cho quản trị viên nên được chứa câu trả lời (hợp đồng chung §5).

use std::sync::Arc;

use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::event::{Delivered, Event, EventHandler};
use autoxide_kernel::i18n::Language;
use autoxide_kernel::mail::MailTemplate;
use uuid::Uuid;

use crate::events::SubmissionReceived;
use crate::forms::fields::{self, Locale};
use crate::service::FormsService;
use crate::submissions::inbox::SubmissionStatus;

pub(crate) const SUBMISSION_MAIL: MailTemplate = MailTemplate::new(
    "forms.mail.submission",
    "New submission: {form}",
    "The form \"{form}\" received a submission.\n\n{answers}\n\nSubmission: {id}\nSent at: {sent_at} (UTC)\n\nReply to this mail to answer the sender, when the form asked for an email address.\n",
);

/// Bên nhận event `forms.submission_received`.
pub(crate) struct Notifier(pub Arc<FormsService>);

#[async_trait::async_trait]
impl EventHandler for Notifier {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        if event.name != SubmissionReceived::NAME {
            return Ok(());
        }
        let id =
            event.payload.get("after").and_then(|after| after.get("id")).and_then(|id| id.as_str());
        match id.and_then(|id| id.parse::<Uuid>().ok()) {
            Some(id) => self.0.notify(id).await.map(|_| ()),
            None => Ok(()),
        }
    }
}

impl FormsService {
    /// Gửi mail báo bài gửi `id` tới mọi `notify_emails` của form. Trả số mail đã xếp hàng.
    /// Bài đã bị xoá hay đã về `spam` thì thôi.
    pub(crate) async fn notify(&self, id: Uuid) -> Result<usize, Error> {
        let submission = match self.find_submission(id).await {
            Ok(found) => found,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error),
        };
        if submission.status() == SubmissionStatus::Spam {
            return Ok(0);
        }
        let form = match self.find_form(submission.row.form_id).await {
            Ok(form) => form,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error),
        };
        if form.notify_emails.is_empty() {
            return Ok(0);
        }
        let language = self.config.notify_language;
        let locale = if language == Language::Vi { Locale::Vi } else { Locale::En };
        // Nhãn theo ĐÚNG phiên bản lúc gửi.
        let answers: Vec<String> = submission
            .fields
            .iter()
            .filter_map(|field| {
                let value = submission.data.get(&field.key)?;
                let text = fields::display(value).replace('\n', "\n  ");
                Some(format!("{}: {text}", field.label.get(locale)))
            })
            .collect();
        let answers = answers.join("\n");
        let id_text = id.to_string();
        let sent_at = submission.row.created_at.format("%Y-%m-%d %H:%M").to_string();
        let params = [
            ("form", form.row.name.as_str()),
            ("answers", answers.as_str()),
            ("id", id_text.as_str()),
            ("sent_at", sent_at.as_str()),
        ];
        for to in &form.notify_emails {
            let mut mail = SUBMISSION_MAIL.mail(to, &self.translations, language, &params);
            mail.reply_to = submission.row.email.clone();
            // Khoá ngắn (cột `unique_key` của job giới hạn độ dài): địa chỉ băm.
            mail.dedupe_key =
                Some(format!("forms.notify:{id}:{}", autoxide_kernel::secret::sha256(to)));
            match self.mailer.send(mail.clone()).await {
                Ok(()) => {}
                // Bên gửi mail không nhận địa chỉ người gửi làm `Reply-To` (địa chỉ cũ lưu từ
                // trước khi kiểm chặt hơn…): vẫn báo quản trị viên, chỉ bỏ `Reply-To` — không để
                // event bị thử lại tới khi hỏng hẳn. Không ghi địa chỉ vào log.
                Err(error) if refuses_reply_to(&error) => {
                    tracing::warn!(
                        submission_id = %id,
                        "the mailer refused the sender's address as Reply-To; notifying without it"
                    );
                    mail.reply_to = None;
                    self.mailer.send(mail).await?;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(form.notify_emails.len())
    }
}

/// Lỗi của bên gửi mail chỉ vì `reply_to` sai dạng.
fn refuses_reply_to(error: &Error) -> bool {
    error.kind() == ErrorKind::Validation
        && !error.fields().is_empty()
        && error.fields().iter().all(|field| field.field == "reply_to")
}
