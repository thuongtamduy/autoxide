//! Bước kiểm của `mail` cho `autoxide doctor` (D-166): máy chủ SMTP có nhận kết nối (và đăng nhập)
//! không; chưa cấu hình SMTP thì cảnh báo rằng mail chỉ được ghi ra file, không gửi đi.

use std::sync::Arc;

use autoxide_kernel::doctor::{Finding, Probe};

use crate::transport::Transport;

pub(crate) struct SmtpProbe {
    pub transport: Arc<Transport>,
}

#[async_trait::async_trait]
impl Probe for SmtpProbe {
    async fn probe(&self) -> Finding {
        match self.transport.as_ref() {
            Transport::File(dir) => Finding::warning(format!(
                "AUTOXIDE_MAIL_SMTP_URL is not set: mail is written to files in {} instead of \
                 being sent.",
                dir.display()
            ))
            .with_hint("Set AUTOXIDE_MAIL_SMTP_URL to your mail provider's SMTP address before going live."),
            Transport::Smtp(smtp) => match smtp.test_connection().await {
                Ok(true) => Finding::ok("The SMTP server accepted a connection."),
                Ok(false) => Finding::failed("the SMTP server did not answer a NOOP command")
                    .with_hint("Check the host and port in AUTOXIDE_MAIL_SMTP_URL."),
                Err(error) => Finding::failed(format!("cannot reach the SMTP server: {error}"))
                    .with_hint(
                        "Check the host, port, user and password in AUTOXIDE_MAIL_SMTP_URL, and \
                         that this server may connect to it.",
                    ),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::doctor::Status;

    use super::*;

    #[tokio::test]
    async fn chua_co_smtp_thi_canh_bao() {
        let transport = Arc::new(Transport::new(None, "mail-out".into()).unwrap());
        let finding = SmtpProbe { transport }.probe().await;
        assert_eq!(finding.status, Status::Warning);
        assert!(finding.message.contains("mail-out"), "{}", finding.message);
    }

    /// Cổng không ai nghe: báo hỏng kèm gợi ý, không treo.
    #[tokio::test]
    async fn smtp_khong_tra_loi_thi_hong() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let url = format!("smtp://127.0.0.1:{port}");
        let transport = Arc::new(Transport::new(Some(&url), "unused".into()).unwrap());
        let finding = SmtpProbe { transport }.probe().await;
        assert_eq!(finding.status, Status::Failed, "{finding:?}");
        assert!(finding.hint.unwrap().contains("AUTOXIDE_MAIL_SMTP_URL"));
    }
}
