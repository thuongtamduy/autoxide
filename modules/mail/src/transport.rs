//! Đường gửi mail: SMTP, hoặc ghi file `.eml` khi chưa cấu hình SMTP.

use std::path::PathBuf;

use autoxide_kernel::error::Error;
use autoxide_kernel::net;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::transport::smtp::extension::ClientId;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use uuid::Uuid;

/// Scheme nhận cho máy chủ SMTP.
pub(crate) const SCHEMES: [&str; 2] = ["smtp", "smtps"];

pub(crate) enum Transport {
    Smtp(AsyncSmtpTransport<Tokio1Executor>),
    /// Thư mục chứa file `.eml`: mở bằng ứng dụng mail nào cũng đọc được.
    File(PathBuf),
}

/// Gửi không được: `error` đầy đủ (log, job queue thử lại), `public` là câu chung cho người dùng.
#[derive(Debug)]
pub(crate) struct Failed {
    pub(crate) error: Box<Error>,
    pub(crate) public: String,
}

/// Kết quả gửi một mail. Lỗi tạm thời (mạng, 4xx) là `Err`: job queue thử lại sau.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Delivery {
    Sent,
    /// Máy chủ nhận từ chối hẳn CHÍNH ĐỊA CHỈ NGƯỜI NHẬN (SMTP 5xx với mã mở rộng 5.1.x, 5.2.x
    /// — không có hộp thư, hộp thư bị khoá…): gửi lại cũng vô ích.
    Rejected {
        code: String,
        /// Mã trạng thái mở rộng (RFC 3463), vd `5.1.1`.
        enhanced: Option<String>,
    },
}

impl Transport {
    pub(crate) fn new(smtp_url: Option<&str>, file_dir: PathBuf) -> Result<Self, Error> {
        match smtp_url {
            Some(url) => Self::smtp(url),
            None => Ok(Transport::File(file_dir)),
        }
    }

    /// Đường gửi qua máy chủ SMTP ở `url`. Lỗi không nhắc lại URL (có mật khẩu).
    pub(crate) fn smtp(url: &str) -> Result<Self, Error> {
        Ok(Transport::Smtp(
            AsyncSmtpTransport::<Tokio1Executor>::from_url(url)
                .map_err(|error| Error::internal(format!("smtp: {error}")))?
                .build(),
        ))
    }

    /// Đường gửi qua máy chủ SMTP RIÊNG CỦA KHÁCH ở `raw` (D-20261121-ssrf-targets): scheme
    /// `smtp`/`smtps`, host phân giải NGAY BÂY GIỜ và chỉ giữ IP public (trừ khi `allow_private`),
    /// rồi kết nối vào đúng IP đó — lettre không tự phân giải lại, nên tên miền không đổi sang IP
    /// nội bộ được giữa lúc kiểm và lúc gửi. TLS vẫn kiểm chứng chỉ theo TÊN MIỀN trong URL. Lỗi trả
    /// câu chung (không nhắc URL, mật khẩu hay lỗi mạng chi tiết).
    pub(crate) async fn tenant_smtp(raw: &str, allow_private: bool) -> Result<Self, String> {
        let url = net::check_target_url(raw, &SCHEMES, allow_private)
            .map_err(|_| NOT_ALLOWED.to_owned())?;
        let host = url.host_str().unwrap_or_default().to_owned();
        let tls =
            url.query_pairs().find(|(key, _)| key == "tls").map(|(_, value)| value.into_owned());
        let name = host.trim_start_matches('[').trim_end_matches(']').to_owned();
        let parameters = || TlsParameters::new(name.clone()).map_err(|_| INVALID.to_owned());
        // Cổng mặc định như lettre: 25 (smtp), 587 (STARTTLS), 465 (TLS ngay từ đầu).
        let (port, tls) = match (url.scheme(), tls.as_deref()) {
            ("smtp", None) => (25, Tls::None),
            ("smtp", Some("required")) => (587, Tls::Required(parameters()?)),
            ("smtp", Some("opportunistic")) => (587, Tls::Opportunistic(parameters()?)),
            ("smtps", _) => (465, Tls::Wrapper(parameters()?)),
            _ => return Err(INVALID.to_owned()),
        };
        let port = url.port().unwrap_or(port);
        let addresses = net::resolve_target(&host, port, allow_private).await.map_err(|error| {
            match error.get_ref() {
                Some(inner) if inner.is::<net::PrivateAddress>() => NOT_ALLOWED.to_owned(),
                _ => UNREACHABLE.to_owned(),
            }
        })?;
        let address = addresses.first().ok_or_else(|| UNREACHABLE.to_owned())?;
        let mut builder =
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(address.ip().to_string())
                .port(address.port())
                .tls(tls);
        if let Some(password) = url.password() {
            let user = percent_decode(url.username()).ok_or_else(|| INVALID.to_owned())?;
            let password = percent_decode(password).ok_or_else(|| INVALID.to_owned())?;
            builder = builder.credentials(Credentials::new(user, password));
        }
        let hello = url.path().trim_matches('/');
        if !hello.is_empty() {
            builder = builder.hello_name(ClientId::Domain(hello.to_owned()));
        }
        Ok(Transport::Smtp(builder.build()))
    }

    /// Gửi `message`. Lỗi mang hai mặt: lỗi đầy đủ cho log / job queue, và câu chung cho quản trị
    /// viên của khách (`last_error` của kết nối).
    pub(crate) async fn deliver(&self, message: Message) -> Result<Delivery, Failed> {
        match self {
            Transport::Smtp(smtp) => match smtp.send(message).await {
                Ok(_) => {}
                Err(error) if error.is_permanent() && blames_recipient(&error.to_string()) => {
                    let code =
                        error.status().map_or_else(|| "5xx".to_owned(), |code| code.to_string());
                    let enhanced = enhanced_code(&error.to_string());
                    return Ok(Delivery::Rejected { code, enhanced });
                }
                // Lỗi 5xx khác (sai mật khẩu SMTP 535, địa chỉ gửi không được phép, hết hạn mức
                // 5.4.5, nội dung bị chặn…) là lỗi của BÊN GỬI: thử lại theo job queue rồi vào
                // dead-letter queue — không báo người nhận hỏng, không để bản tin chặn cả danh
                // sách khi SMTP cấu hình sai (D-158).
                Err(error) => {
                    return Err(Failed {
                        public: describe(&error),
                        error: Box::new(Error::internal(format!("smtp: {error}"))),
                    });
                }
            },
            Transport::File(directory) => {
                // Lần đầu một thư bị ghi ra file thay vì gửi: nói một lần, lúc nó thật sự xảy ra.
                static WARNED: std::sync::Once = std::sync::Once::new();
                WARNED.call_once(|| {
                    tracing::warn!(
                        directory = %directory.display(),
                        "AUTOXIDE_MAIL_SMTP_URL is not set: this mail is written to a file, not sent; set it \
                         for production"
                    );
                });
                let directory = directory.clone();
                let bytes = message.formatted();
                // Ghi file là việc chặn: đẩy sang thread dành cho việc chặn.
                tokio::task::spawn_blocking(move || {
                    std::fs::create_dir_all(&directory)?;
                    let name = format!("{}.eml", Uuid::now_v7());
                    std::fs::write(directory.join(name), bytes)
                })
                .await
                .map_err(Error::internal)
                .and_then(|written| {
                    written.map_err(|error| {
                        Error::internal(format!("cannot write mail file: {error}"))
                    })
                })
                .map_err(|error| Failed {
                    public: "cannot write the mail file".to_owned(),
                    error: Box::new(error),
                })?;
            }
        }
        Ok(Delivery::Sent)
    }
}

/// Câu chung khi địa chỉ SMTP của khách không được phép (mạng riêng / nội bộ).
pub(crate) const NOT_ALLOWED: &str =
    "the SMTP server must be on the public Internet, not in a private or local network";
/// Câu chung khi URL SMTP của khách đọc không được.
pub(crate) const INVALID: &str = "the SMTP address is not a valid `smtp://` or `smtps://` URL";
/// Câu chung khi không kết nối được (không nói lỗi mạng cụ thể: không biến thành công cụ dò cổng).
pub(crate) const UNREACHABLE: &str = "cannot connect to the SMTP server";

/// Mô tả lỗi SMTP cho quản trị viên của khách (thử kết nối, `last_error`): chỉ mã trạng thái máy
/// chủ trả, không kèm câu trả lời hay lỗi mạng chi tiết.
pub(crate) fn describe(error: &lettre::transport::smtp::Error) -> String {
    match error.status() {
        Some(code) => format!("the SMTP server answered with status {code}"),
        None => UNREACHABLE.to_owned(),
    }
}

/// Giải `%xx` trong user / mật khẩu của URL (như lettre làm với URL của máy chủ).
fn percent_decode(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = raw.get(index + 1..index + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Lỗi SMTP vĩnh viễn (`text` là câu trả lời của máy chủ) có phải do địa chỉ người nhận không:
/// mã trạng thái mở rộng (RFC 3463) đầu tiên là `5.1.x` (địa chỉ) hoặc `5.2.x` (hộp thư). Không
/// có mã mở rộng thì không đoán: coi là lỗi bên gửi.
pub(crate) fn blames_recipient(text: &str) -> bool {
    enhanced_code(text).is_some_and(|code| code.starts_with("5.1.") || code.starts_with("5.2."))
}

/// Mã trạng thái mở rộng (RFC 3463) đầu tiên trong câu trả lời `text` của máy chủ, nếu có.
pub(crate) fn enhanced_code(text: &str) -> Option<String> {
    text.split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | ':' | ',' | ';' | '[' | ']'))
        .find(|word| is_enhanced_code(word))
        .map(str::to_owned)
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
    use super::{Transport, blames_recipient, percent_decode};

    #[test]
    fn giai_ma_phan_tram_trong_user_va_mat_khau() {
        assert_eq!(percent_decode("a%40b%2Fc").as_deref(), Some("a@b/c"));
        assert_eq!(percent_decode("plain").as_deref(), Some("plain"));
        assert_eq!(percent_decode("bad%4"), None);
        assert_eq!(percent_decode("bad%zz"), None);
    }

    #[tokio::test]
    async fn smtp_cua_khach_khong_toi_mang_noi_bo() {
        for raw in [
            "smtp://127.0.0.1:6379",
            "smtp://localhost:25",
            "smtps://[::1]:465",
            "smtp://169.254.169.254:80",
            "http://smtp.example.com",
        ] {
            let error = Transport::tenant_smtp(raw, false).await.err().unwrap();
            assert!(!error.contains("6379"), "{raw}: {error}");
            assert!(error == super::NOT_ALLOWED || error == super::INVALID, "{raw}: {error}");
        }
        // Người vận hành cho phép mạng riêng: dựng được (chưa kết nối).
        assert!(Transport::tenant_smtp("smtp://u:p%40ss@127.0.0.1:2525", true).await.is_ok());
    }

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
