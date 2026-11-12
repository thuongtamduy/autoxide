//! Đường gửi mail: SMTP, hoặc ghi file `.eml` khi chưa cấu hình SMTP.

use std::path::PathBuf;

use autoxide_kernel::error::Error;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use uuid::Uuid;

pub(crate) enum Transport {
    Smtp(AsyncSmtpTransport<Tokio1Executor>),
    /// Thư mục chứa file `.eml`: mở bằng ứng dụng mail nào cũng đọc được.
    File(PathBuf),
}

/// Kết quả gửi một mail. Lỗi tạm thời (mạng, 4xx) là `Err`: job queue thử lại sau.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Delivery {
    Sent,
    /// Máy chủ nhận từ chối hẳn (SMTP 5xx): gửi lại cũng vô ích.
    Rejected {
        code: String,
    },
}

impl Transport {
    pub(crate) fn new(smtp_url: Option<&str>, file_dir: PathBuf) -> Result<Self, Error> {
        match smtp_url {
            Some(url) => Ok(Transport::Smtp(
                AsyncSmtpTransport::<Tokio1Executor>::from_url(url)
                    .map_err(|error| Error::internal(format!("smtp: {error}")))?
                    .build(),
            )),
            None => Ok(Transport::File(file_dir)),
        }
    }

    pub(crate) async fn deliver(&self, message: Message) -> Result<Delivery, Error> {
        match self {
            Transport::Smtp(smtp) => match smtp.send(message).await {
                Ok(_) => {}
                Err(error) if error.is_permanent() => {
                    let code =
                        error.status().map_or_else(|| "5xx".to_owned(), |code| code.to_string());
                    return Ok(Delivery::Rejected { code });
                }
                Err(error) => return Err(Error::internal(format!("smtp: {error}"))),
            },
            Transport::File(directory) => {
                let directory = directory.clone();
                let bytes = message.formatted();
                // Ghi file là việc chặn: đẩy sang thread dành cho việc chặn.
                tokio::task::spawn_blocking(move || {
                    std::fs::create_dir_all(&directory)?;
                    let name = format!("{}.eml", Uuid::now_v7());
                    std::fs::write(directory.join(name), bytes)
                })
                .await
                .map_err(Error::internal)?
                .map_err(|error| Error::internal(format!("cannot write mail file: {error}")))?;
            }
        }
        Ok(Delivery::Sent)
    }
}
