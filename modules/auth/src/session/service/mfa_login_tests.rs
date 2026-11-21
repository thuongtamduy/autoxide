use super::*;
use autoxide_kernel::error::ErrorKind;
use autoxide_testkit::TestDatabase;

use crate::mfa::service::tests::code_at;
use crate::user::NewUser;

const PASSWORD: &str = "correct horse battery staple";

fn unix_now() -> u64 {
    u64::try_from(Timestamp::now().as_unix_seconds()).unwrap()
}

/// Ada đã bật MFA. Trả về service, secret TOTP và recovery code.
async fn setup(test: &TestDatabase) -> (Arc<SessionService>, String, Vec<String>, TenantContext) {
    let services = crate::testing::services(test).await;
    let tenant = services.tenant.clone();
    let ada = services
        .users
        .create(
            &tenant,
            Actor::system(),
            NewUser {
                email: "ada@example.com".to_owned(),
                name: "Ada".to_owned(),
                password: Some(PASSWORD.to_owned()),
            },
        )
        .await
        .unwrap();
    let setup = services.mfa.start_totp(&tenant, ada.id, &ada.email).await.unwrap();
    let codes = services
        .mfa
        .confirm_totp(&tenant, ada.id, &code_at(&setup.secret, unix_now()))
        .await
        .unwrap();
    (services.sessions, setup.secret, codes, tenant)
}

async fn challenge(sessions: &SessionService, tenant: &TenantContext) -> String {
    match sessions.login(tenant, "ada@example.com", PASSWORD, None).await.unwrap() {
        LoginOutcome::MfaRequired { mfa_token, expires_in } => {
            assert_eq!(expires_in, 300);
            mfa_token
        }
        LoginOutcome::Tokens(_) => panic!("expected an MFA challenge"),
    }
}

#[tokio::test]
async fn dung_mat_khau_thi_con_phai_nhap_ma() {
    TestDatabase::run(async |test| {
        let (sessions, secret, _, tenant) = setup(test).await;

        let mfa_token = challenge(&sessions, &tenant).await;
        let code = code_at(&secret, unix_now() + 30);
        let (_, tokens) =
            sessions.complete_mfa(Some(&tenant), &mfa_token, &code, None).await.unwrap();

        assert!(
            sessions
                .authenticate(&tenant, Some(&format!("Bearer {}", tokens.access_token)))
                .await
                .is_ok()
        );
        // Mỗi lượt chỉ cấp token một lần.
        let again =
            sessions.complete_mfa(Some(&tenant), &mfa_token, &code, None).await.unwrap_err();
        assert_eq!(again.code(), "auth.invalid_mfa_token");
        let unknown = sessions.complete_mfa(Some(&tenant), "nope", &code, None).await.unwrap_err();
        assert_eq!(unknown.code(), "auth.invalid_mfa_token");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dang_nhap_bang_recovery_code() {
    TestDatabase::run(async |test| {
        let (sessions, _, codes, tenant) = setup(test).await;

        let mfa_token = challenge(&sessions, &tenant).await;
        sessions.complete_mfa(Some(&tenant), &mfa_token, &codes[0], None).await.unwrap();

        let mfa_token = challenge(&sessions, &tenant).await;
        let reused =
            sessions.complete_mfa(Some(&tenant), &mfa_token, &codes[0], None).await.unwrap_err();
        assert_eq!(reused.code(), "auth.invalid_mfa_code");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doan_ma_sai_bi_gioi_han() {
    TestDatabase::run(async |test| {
        let (sessions, _, _, tenant) = setup(test).await;
        let mfa_token = challenge(&sessions, &tenant).await;

        for _ in 0..5 {
            let error =
                sessions.complete_mfa(Some(&tenant), &mfa_token, "000000", None).await.unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Unauthenticated);
            assert_eq!(error.code(), "auth.invalid_mfa_code");
        }

        // Hết lượt nhập sai của token này…
        let used_up =
            sessions.complete_mfa(Some(&tenant), &mfa_token, "000000", None).await.unwrap_err();
        assert_eq!(used_up.code(), "auth.invalid_mfa_token");
        // …và các lần sai được đếm theo tài khoản: lượt mới (đúng mật khẩu) cũng bị chặn nhập mã.
        let fresh = challenge(&sessions, &tenant).await;
        let locked =
            sessions.complete_mfa(Some(&tenant), &fresh, "000000", None).await.unwrap_err();
        assert_eq!(locked.code(), "auth.login_throttled");
    })
    .await
    .unwrap();
}

/// Nhiều request hoàn tất CÙNG một lượt với cùng một mã đúng: đúng một bên nhận
/// token — trên cả hai database.
#[tokio::test]
async fn hoan_tat_dong_thoi_chi_mot_ben_nhan_token() {
    TestDatabase::run(async |test| {
        let (sessions, secret, _, tenant) = setup(test).await;
        let mfa_token = challenge(&sessions, &tenant).await;
        let code = code_at(&secret, unix_now() + 30);

        let attempts: Vec<_> = (0..6)
            .map(|_| {
                let sessions = sessions.clone();
                let (mfa_token, code) = (mfa_token.clone(), code.clone());
                let tenant = tenant.clone();
                tokio::spawn(async move {
                    sessions.complete_mfa(Some(&tenant), &mfa_token, &code, None).await
                })
            })
            .collect();
        let mut signed_in = 0;
        for attempt in attempts {
            if attempt.await.unwrap().is_ok() {
                signed_in += 1;
            }
        }

        assert_eq!(signed_in, 1);
    })
    .await
    .unwrap();
}
