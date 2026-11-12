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
    /// Máy chủ nhận từ chối hẳn CHÍNH ĐỊA CHỈ NGƯỜI NHẬN (SMTP 5xx với mã mở rộng 5.1.x, 5.2.x
    /// — không có hộp thư, hộp thư bị khoá…): gửi lại cũng vô ích.
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
                Err(error) if error.is_permanent() && blames_recipient(&error.to_string()) => {
                    let code =
                        error.status().map_or_else(|| "5xx".to_owned(), |code| code.to_string());
                    return Ok(Delivery::Rejected { code });
                }
                // Lỗi 5xx khác (sai mật khẩu SMTP 535, địa chỉ gửi không được phép, hết hạn mức
                // 5.4.5, nội dung bị chặn…) là lỗi của BÊN GỬI: thử lại theo job queue rồi vào
                // dead-letter queue — không báo người nhận hỏng, không để bản tin chặn cả danh
                // sách khi SMTP cấu hình sai (D-157).
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

/// Lỗi SMTP vĩnh viễn (`text` là câu trả lời của máy chủ) có phải do địa chỉ người nhận không:
/// mã trạng thái mở rộng (RFC 3463) đầu tiên là `5.1.x` (địa chỉ) hoặc `5.2.x` (hộp thư). Không
/// có mã mở rộng thì không đoán: coi là lỗi bên gửi.
pub(crate) fn blames_recipient(text: &str) -> bool {
    text.split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | ':' | ',' | ';' | '[' | ']'))
        .find(|word| is_enhanced_code(word))
        .is_some_and(|code| code.starts_with("5.1.") || code.starts_with("5.2."))
}

/// `class.subject.detail`: `2|4|5`, rồi hai số 1–3 chữ số.
fn is_enhanced_code(word: &str) -> bool {
    let parts: Vec<&str> = word.split('.').collect();
    parts.len() == 3
        && matches!(parts[0], "2" | "4" | "5")
        && parts[1..]
            .iter()
            .all(|part| (1..=3).contains(&part.len()) && part.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::blames_recipient;

    #[test]
    fn chi_loi_ve_dia_chi_nguoi_nhan_moi_do_cho_nguoi_nhan() {
        for text in [
            "permanent error (550): 5.1.1 The email account that you tried to reach does not exist",
            "permanent error (552): 5.2.2 mailbox full",
            "permanent error (550): Requested action not taken: 5.1.10 recipient rejected",
        ] {
            assert!(blames_recipient(text), "{text}");
        }
        for text in [
            "permanent error (535): 5.7.8 Username and Password not accepted",
            "permanent error (550): 5.4.5 Daily user sending limit exceeded",
            "permanent error (554): 5.7.1 Message rejected: sending paused",
            "permanent error (550): mailbox unavailable",
            "permanent error (553): 5.7.1 sender address not allowed",
        ] {
            assert!(!blames_recipient(text), "{text}");
        }
    }
}
