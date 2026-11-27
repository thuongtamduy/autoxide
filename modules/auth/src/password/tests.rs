//! Test của `password.rs`, ở file riêng (D-20261127-codeql-skip-tests: CodeQL bỏ qua `tests.rs`, nơi mật khẩu mẫu
//! của test không phải lỗ hổng).

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
