//! Hash và kiểm mật khẩu bằng Argon2id.
//!
//! Tham số theo khuyến nghị của OWASP cho Argon2id: 19 MiB bộ nhớ, 2 lượt, 1 luồng
//! song song — khoảng vài chục mili giây một lần hash trên máy thường. Hash lưu dạng
//! chuỗi PHC (`$argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>`), tự mang tham số
//! của nó: đổi tham số sau này thì hash cũ vẫn kiểm được, và được hash lại theo
//! tham số mới ở lần đăng nhập đúng kế tiếp.
//!
//! Hash cố tình chậm và tốn bộ nhớ, nên:
//!
//! - chạy trên thread riêng cho việc chặn (`spawn_blocking`), không chặn thread
//!   async đang phục vụ request khác;
//! - số lần hash chạy cùng lúc bị giới hạn bằng số nhân CPU, để một loạt request
//!   đăng nhập không ăn hết bộ nhớ của máy.

use crate::policy::TenantPolicy;
use std::num::NonZero;
use std::sync::Arc;

use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher as _, PasswordVerifier as _};
use argon2::{Algorithm, Argon2, Params, Version};
use autoxide_kernel::error::{Error, FieldError};
use tokio::sync::Semaphore;

/// Bộ nhớ dùng cho một lần hash, theo KiB.
const MEMORY_KIB: u32 = 19 * 1024;
/// Số lượt đi qua bộ nhớ.
const PASSES: u32 = 2;
/// Số luồng song song trong một lần hash.
const LANES: u32 = 1;

/// Độ dài tối thiểu của mật khẩu, tính theo ký tự (code point). 15: NIST SP 800-63B-4 §3.1.1.2 buộc mật khẩu dùng
/// làm yếu tố duy nhất dài ít nhất 15 ký tự, và ở đây mật khẩu một mình đăng nhập được (D-20261127-breached-passwords).
pub(crate) const MIN_LENGTH: usize = 15;
/// Độ dài tối đa: đủ cho cụm từ dài và trình quản lý mật khẩu, nhưng chặn việc gửi
/// mật khẩu khổng lồ để làm máy chủ hash mãi.
pub(crate) const MAX_LENGTH: usize = 128;

/// Hash của một mật khẩu ngẫu nhiên không ai biết, cùng tham số với hash thật. Khi
/// email không tồn tại, vẫn kiểm mật khẩu với hash này để thời gian trả lời giống hệt
/// trường hợp email có thật — kẻ tấn công không dò được email nào đã đăng ký.
const DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$KX6rM7yQDDCBNPvfwr74AQ$\
                          Wx0cTW03FPozUVKh0CcLHMLCwXPUDepEC5CCluyuRX4";

/// Kết quả kiểm một mật khẩu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verification {
    /// Đúng mật khẩu. `rehash`: hash đang lưu dùng tham số cũ, nên hash lại.
    Valid { rehash: bool },
    /// Sai mật khẩu (hoặc tài khoản không có mật khẩu).
    Invalid,
}

/// Bộ hash mật khẩu dùng chung của module. Clone rẻ: các bản clone dùng chung giới
/// hạn số lần hash chạy cùng lúc.
#[derive(Debug, Clone)]
pub(crate) struct Passwords {
    // `Semaphore`: một chồng vé. Mỗi lần hash lấy một vé, xong thì trả; hết vé thì
    // lần hash sau chờ tới khi có vé.
    permits: Arc<Semaphore>,
    /// Chính sách theo khách (độ dài tối thiểu).
    policy: TenantPolicy,
}

impl Passwords {
    pub(crate) fn new() -> Self {
        let cores = std::thread::available_parallelism().map_or(1, NonZero::get);
        Passwords { permits: Arc::new(Semaphore::new(cores)), policy: TenantPolicy::default() }
    }

    /// Độ dài tối thiểu theo khách (`auth.password_min_length`).
    pub(crate) fn with_policy(mut self, policy: TenantPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Lỗi của trường `field` nếu `password` không đạt luật của khách `tenant`: độ dài (xem [`check_length`])
    /// rồi danh sách mật khẩu phổ biến / đã lộ (xem [`check_common`]).
    pub(crate) async fn problem(
        &self,
        tenant: &autoxide_kernel::tenancy::TenantContext,
        field: &str,
        password: &str,
    ) -> Result<Option<FieldError>, Error> {
        let min = self.policy.password_min_length(tenant).await?;
        Ok(check_length(field, password, min).or_else(|| check_common(field, password)))
    }

    /// Hash `password` với salt ngẫu nhiên; trả chuỗi PHC để lưu.
    pub(crate) async fn hash(&self, password: String) -> Result<String, Error> {
        self.run(move || {
            let hash = argon2()?
                .hash_password(password.as_bytes())
                .map_err(|error| Error::internal(format!("argon2: cannot hash: {error}")))?;
            Ok(hash.to_string())
        })
        .await
    }

    /// Kiểm `password` với `stored` (chuỗi PHC đã lưu). `stored` là `None` (không có
    /// tài khoản, hoặc tài khoản chưa đặt mật khẩu) thì vẫn tốn đúng một lần kiểm
    /// rồi trả [`Verification::Invalid`].
    pub(crate) async fn verify(
        &self,
        password: String,
        stored: Option<String>,
    ) -> Result<Verification, Error> {
        self.run(move || {
            let known = stored.is_some();
            let stored = stored.as_deref().unwrap_or(DUMMY_HASH);
            let parsed = PasswordHash::new(stored).map_err(|error| {
                Error::internal(format!("argon2: stored password hash is malformed: {error}"))
            })?;
            let matches = argon2()?.verify_password(password.as_bytes(), &parsed).is_ok();
            Ok(if known && matches {
                Verification::Valid { rehash: is_outdated(&parsed) }
            } else {
                Verification::Invalid
            })
        })
        .await
    }

    /// Chạy `work` trên thread dành cho việc chặn, sau khi lấy được một vé.
    async fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> Result<T, Error> + Send + 'static,
    ) -> Result<T, Error> {
        // `_permit` giữ vé tới hết hàm (khi biến bị huỷ thì vé tự trả lại).
        let _permit = self.permits.acquire().await.map_err(Error::internal)?;
        tokio::task::spawn_blocking(work).await.map_err(Error::internal)?
    }
}

/// Bộ Argon2id với tham số hiện hành.
fn argon2() -> Result<Argon2<'static>, Error> {
    let params = Params::new(MEMORY_KIB, PASSES, LANES, None)
        .map_err(|error| Error::internal(format!("argon2: invalid parameters: {error}")))?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

/// Hash `parsed` không dùng đúng thuật toán và tham số hiện hành.
fn is_outdated(parsed: &PasswordHash) -> bool {
    if parsed.algorithm.as_str() != Algorithm::Argon2id.as_str() {
        return true;
    }
    match Params::try_from(parsed) {
        Ok(params) => {
            (params.m_cost(), params.t_cost(), params.p_cost()) != (MEMORY_KIB, PASSES, LANES)
        }
        Err(_) => true,
    }
}

/// Lỗi của trường `field` nếu `password` không đạt luật của server: độ dài, rồi danh sách mật khẩu phổ biến /
/// đã lộ.
///
/// Không bắt buộc chữ hoa, số hay ký tự đặc biệt: theo NIST SP 800-63B-4 §3.1.1.2, luật ghép ký tự làm
/// mật khẩu dễ đoán hơn chứ không khó hơn; thay vào đó chặn mật khẩu nằm trong danh sách đã lộ.
pub(crate) fn check_rules(field: &str, password: &str) -> Option<FieldError> {
    check_length(field, password, MIN_LENGTH).or_else(|| check_common(field, password))
}

/// Mã lỗi của TRƯỜNG (như `length`, không phải `code` của lỗi — lỗi là `validation_failed`) khi mật khẩu nằm
/// trong danh sách mật khẩu phổ biến / đã lộ; hợp đồng với client.
pub(crate) const COMMON_PASSWORD: &str = "common_password";

/// Lỗi của trường `field` nếu `password` là mật khẩu phổ biến hay đã lộ (D-20261127-breached-passwords).
/// Lỗi không nhắc lại mật khẩu, và không mật khẩu nào được ghi vào log.
pub(crate) fn check_common(field: &str, password: &str) -> Option<FieldError> {
    crate::breached::is_common(password).then(|| {
        FieldError::new(
            field,
            COMMON_PASSWORD,
            "is a commonly used or breached password; choose one that is harder to guess",
        )
    })
}

/// Như [`check_rules`] với độ dài tối thiểu `min` (của khách, không dưới [`MIN_LENGTH`]).
pub(crate) fn check_length(field: &str, password: &str, min: usize) -> Option<FieldError> {
    let length = password.chars().count();
    (!(min..=MAX_LENGTH).contains(&length)).then(|| {
        FieldError::new(
            field,
            "length",
            "must be between {min_length} and {max_length} characters long",
        )
        .with_param("min_length", min)
        .with_param("max_length", MAX_LENGTH)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWORD: &str = "correct horse battery staple";

    #[tokio::test]
    async fn hash_dung_argon2id_voi_tham_so_hien_hanh() {
        let hash = Passwords::new().hash(PASSWORD.to_owned()).await.unwrap();

        assert!(hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{hash}");
        assert!(!hash.contains(PASSWORD));
    }

    #[tokio::test]
    async fn cung_mat_khau_hai_lan_hash_khac_nhau() {
        let passwords = Passwords::new();

        let first = passwords.hash(PASSWORD.to_owned()).await.unwrap();
        let second = passwords.hash(PASSWORD.to_owned()).await.unwrap();

        // Salt ngẫu nhiên: hai người cùng mật khẩu không có cùng hash.
        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn kiem_dung_va_sai_mat_khau() {
        let passwords = Passwords::new();
        let hash = passwords.hash(PASSWORD.to_owned()).await.unwrap();

        let right = passwords.verify(PASSWORD.to_owned(), Some(hash.clone())).await.unwrap();
        let wrong = passwords.verify("wrong password!".to_owned(), Some(hash)).await.unwrap();

        assert_eq!(right, Verification::Valid { rehash: false });
        assert_eq!(wrong, Verification::Invalid);
    }

    #[tokio::test]
    async fn khong_co_hash_thi_luon_sai() {
        let verification = Passwords::new().verify(PASSWORD.to_owned(), None).await.unwrap();

        assert_eq!(verification, Verification::Invalid);
    }

    #[test]
    fn hash_gia_dung_tham_so_hien_hanh() {
        // Hash giả phải tốn đúng bằng hash thật, nếu không thời gian trả lời lộ
        // email nào có thật.
        let parsed = PasswordHash::new(DUMMY_HASH).unwrap();
        assert!(!is_outdated(&parsed));
    }

    #[tokio::test]
    async fn hash_theo_tham_so_cu_van_kiem_duoc_va_can_hash_lai() {
        let weak = Argon2::new(
            Algorithm::Argon2id,
            Version::V0x13,
            Params::new(8 * 1024, 1, 1, None).unwrap(),
        );
        let old = weak.hash_password(PASSWORD.as_bytes()).unwrap().to_string();

        let verification = Passwords::new().verify(PASSWORD.to_owned(), Some(old)).await.unwrap();

        assert_eq!(verification, Verification::Valid { rehash: true });
    }

    #[tokio::test]
    async fn hash_hong_la_loi_noi_bo() {
        let error = Passwords::new()
            .verify(PASSWORD.to_owned(), Some("not a hash".to_owned()))
            .await
            .unwrap_err();

        assert_eq!(error.code(), "internal_error");
    }

    #[test]
    fn luat_do_dai_tinh_theo_ky_tu() {
        assert!(check_rules("password", &"a".repeat(MIN_LENGTH - 1)).is_some());
        assert_eq!(check_rules("password", &"ab".repeat(MIN_LENGTH / 2 + 1)), None);
        assert_eq!(check_rules("password", &"ab".repeat(MAX_LENGTH / 2)), None);
        assert!(check_rules("password", &"ab".repeat(MAX_LENGTH / 2 + 1)).is_some());
        // 15 ký tự tiếng Việt có dấu (nhiều byte mỗi ký tự) vẫn là 15 ký tự.
        assert_eq!(check_rules("password", "mậtkhẩuđủdàirồi"), None);
        assert!(check_rules("password", "mậtkhẩuđủdài").is_some());

        let error = check_rules("password", "short").unwrap();
        assert_eq!(error.field, "password");
        assert_eq!(error.code, "length");
        assert_eq!(error.message, "must be between 15 and 128 characters long");
    }

    #[test]
    fn chan_mat_khau_da_lo_sau_luat_do_dai() {
        let error = check_rules("password", "Password123456789").unwrap();
        assert_eq!(error.code, COMMON_PASSWORD);
        assert!(!error.message.contains("Password123456789"));
        // Quá ngắn thì báo độ dài trước: người dùng sửa từng lỗi một.
        assert_eq!(check_rules("password", "123456").unwrap().code, "length");
        assert_eq!(check_length("password", "Password123456789", MIN_LENGTH), None);
        assert_eq!(check_common("password", PASSWORD), None);
    }
}
