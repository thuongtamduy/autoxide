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

    pub(crate) async fn deliver(&self, message: Message) -> Result<(), Error> {
        match self {
            Transport::Smtp(smtp) => {
                smtp.send(message)
                    .await
                    .map_err(|error| Error::internal(format!("smtp: {error}")))?;
            }
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
        Ok(())
    }
}
