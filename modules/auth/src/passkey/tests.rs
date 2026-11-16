//! Test passkey trên database gate đang chọn (gate chạy cả SQLite và Postgres), với
//! authenticator phần mềm thay cho thiết bị thật.

use std::sync::Arc;

use autoxide_kernel::error::ErrorKind;
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Order, PageRequest};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_testkit::TestDatabase;
use serde_json::Value;
use uuid::Uuid;

use super::testing::SoftAuthenticator;
use super::{Passkey, PasskeyService};
use crate::testing::Services;
use crate::user::{NewUser, UserChanges};

const PASSWORD: &str = "correct horse battery staple";

async fn user(services: &Services, email: &str) -> Uuid {
    let tenant = services.tenant.clone();
    let input = NewUser {
        email: email.to_owned(),
        name: "Ada".to_owned(),
        password: Some(PASSWORD.to_owned()),
    };
    services.users.create(&tenant, Actor::system(), input).await.unwrap().id
}

/// Đăng ký passkey của `device` cho `user_id`.
async fn register(services: &Services, user_id: Uuid, device: &mut SoftAuthenticator) -> Passkey {
    let tenant = services.tenant.clone();
    let passkeys = &services.passkeys;
    let ceremony = passkeys.start_registration(&tenant, user_id, PASSWORD).await.unwrap();
    let credential = device.register(&ceremony.options);
    passkeys.finish_registration(&tenant, user_id, ceremony.id, "Phone", &credential).await.unwrap()
}

/// Một lượt đăng nhập: (ceremony id, câu trả lời của `device`).
async fn sign_in(
    passkeys: &PasskeyService,
    tenant: &TenantContext,
    device: &mut SoftAuthenticator,
) -> (Uuid, Value) {
    let ceremony = passkeys.start_login(tenant).await.unwrap();
    (ceremony.id, device.sign_in(&ceremony.options))
}

async fn list(services: &Services, user_id: Uuid) -> Vec<Passkey> {
    let tenant = services.tenant.clone();
    let page = PageRequest::first(50, Order::Asc);
    services.passkeys.list(&tenant, user_id, &page).await.unwrap().data
}

#[tokio::test]
async fn dang_ky_roi_dang_nhap_bang_passkey() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let tenant = services.tenant.clone();
        let ada = user(&services, "ada@example.com").await;
        let mut phone = SoftAuthenticator::new(1);

        let added = register(&services, ada, &mut phone).await;
        assert_eq!((added.name.as_str(), added.last_used_at), ("Phone", None));
        assert_eq!(added.aaguid, None, "an authenticator that does not say has no kind");
        // Authenticator tự khai loại (D-272): giữ AAGUID, biết tên.
        let mut laptop = SoftAuthenticator::new(7);
        let hello = Uuid::parse_str("08987058-cadc-4b81-b6e1-30de50dcbe96").unwrap();
        laptop.aaguid = *hello.as_bytes();
        let kind = register(&services, ada, &mut laptop).await;
        assert_eq!(kind.aaguid, Some(hello));
        assert_eq!(kind.aaguid.and_then(super::authenticators::name), Some("Windows Hello"));

        let (ceremony, response) = sign_in(&services.passkeys, &tenant, &mut phone).await;
        let tokens =
            services.passkeys.finish_login(&tenant, ceremony, &response, None).await.unwrap();

        let bearer = format!("Bearer {}", tokens.access_token);
        assert_eq!(services.sessions.authenticate(&tenant, Some(&bearer)).await.unwrap().id, ada);
        let stored = list(&services, ada).await;
        assert!(stored[0].last_used_at.is_some());
        // Bộ đếm chữ ký đã lưu: lần sau vẫn đăng nhập được (bộ đếm tăng tiếp).
        let (ceremony, response) = sign_in(&services.passkeys, &tenant, &mut phone).await;
        services.passkeys.finish_login(&tenant, ceremony, &response, None).await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nghi_thuc_dung_mot_lan_va_khong_gui_lai_duoc() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let tenant = services.tenant.clone();
        let ada = user(&services, "ada@example.com").await;
        let mut phone = SoftAuthenticator::new(1);
        let passkeys = &services.passkeys;

        // Đăng ký: dùng lại cùng nghi thức thì bị từ chối.
        let ceremony = passkeys.start_registration(&tenant, ada, PASSWORD).await.unwrap();
        let credential = phone.register(&ceremony.options);
        passkeys
            .finish_registration(&tenant, ada, ceremony.id, "Phone", &credential)
            .await
            .unwrap();
        let again =
            passkeys.finish_registration(&tenant, ada, ceremony.id, "Phone", &credential).await;
        assert_eq!(again.unwrap_err().code(), "auth.invalid_passkey_ceremony");

        // Đăng nhập: gửi lại câu trả lời cũ cho nghi thức mới (challenge khác) thì hỏng.
        let (first, response) = sign_in(passkeys, &tenant, &mut phone).await;
        passkeys.finish_login(&tenant, first, &response, None).await.unwrap();
        let replayed = passkeys.finish_login(&tenant, first, &response, None).await.unwrap_err();
        assert_eq!(replayed.code(), "auth.invalid_passkey_ceremony");
        let fresh = passkeys.start_login(&tenant).await.unwrap();
        let stolen = passkeys.finish_login(&tenant, fresh.id, &response, None).await.unwrap_err();
        assert_eq!(
            (stolen.kind(), stolen.code()),
            (ErrorKind::Unauthenticated, "auth.passkey_sign_in_failed")
        );

        // Nghi thức đăng ký của người khác không dùng được.
        let bob = user(&services, "bob@example.com").await;
        let theirs = passkeys.start_registration(&tenant, ada, PASSWORD).await.unwrap();
        let wrong =
            passkeys.finish_registration(&tenant, bob, theirs.id, "Phone", &credential).await;
        assert_eq!(wrong.unwrap_err().code(), "auth.invalid_passkey_ceremony");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cau_tra_loi_sai_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let tenant = services.tenant.clone();
        let ada = user(&services, "ada@example.com").await;
        let passkeys = &services.passkeys;

        // Trang web lạ (origin không được phép) không đăng ký được.
        let mut phishing = SoftAuthenticator::new(2);
        phishing.origin = "https://evil.example.net".to_owned();
        let ceremony = passkeys.start_registration(&tenant, ada, PASSWORD).await.unwrap();
        let credential = phishing.register(&ceremony.options);
        let rejected =
            passkeys.finish_registration(&tenant, ada, ceremony.id, "x", &credential).await;
        let rejected = rejected.unwrap_err();
        assert_eq!(
            (rejected.kind(), rejected.code()),
            (ErrorKind::BadRequest, "auth.passkey_rejected")
        );

        // Passkey chưa đăng ký ở đây.
        let mut stranger = SoftAuthenticator::new(3);
        stranger.user_handle = vec![7; 64];
        let (ceremony, response) = sign_in(passkeys, &tenant, &mut stranger).await;
        let unknown = passkeys.finish_login(&tenant, ceremony, &response, None).await.unwrap_err();
        assert_eq!(unknown.code(), "auth.passkey_sign_in_failed");

        // Đúng credential nhưng tự nhận là người khác (user handle khác).
        let mut phone = SoftAuthenticator::new(4);
        register(&services, ada, &mut phone).await;
        phone.user_handle = vec![9; 64];
        let (ceremony, response) = sign_in(passkeys, &tenant, &mut phone).await;
        let impostor = passkeys.finish_login(&tenant, ceremony, &response, None).await.unwrap_err();
        assert_eq!(impostor.code(), "auth.passkey_sign_in_failed");

        // JSON không phải câu trả lời WebAuthn: lỗi đầu vào, nghi thức còn nguyên.
        let ceremony = passkeys.start_login(&tenant).await.unwrap();
        let invalid =
            passkeys.finish_login(&tenant, ceremony.id, &serde_json::json!({"x": 1}), None).await;
        assert_eq!(invalid.unwrap_err().code(), "validation_failed");
        let name =
            passkeys.finish_registration(&tenant, ada, Uuid::now_v7(), " ", &credential).await;
        assert_eq!(name.unwrap_err().code(), "validation_failed");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nhieu_passkey_xoa_va_tai_khoan_bi_khoa() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let tenant = services.tenant.clone();
        let ada = user(&services, "ada@example.com").await;
        let (mut phone, mut key) = (SoftAuthenticator::new(5), SoftAuthenticator::new(6));

        let first = register(&services, ada, &mut phone).await;
        let ceremony = services.passkeys.start_registration(&tenant, ada, PASSWORD).await.unwrap();
        // Passkey đã có được báo cho trình duyệt (không tạo trùng trên cùng thiết bị), và
        // mọi passkey của một người dùng chung một user handle.
        assert_eq!(ceremony.options["excludeCredentials"].as_array().unwrap().len(), 1);
        let credential = key.register(&ceremony.options);
        services
            .passkeys
            .finish_registration(&tenant, ada, ceremony.id, "Key", &credential)
            .await
            .unwrap();
        assert_eq!(phone.user_handle, key.user_handle);
        assert_eq!(list(&services, ada).await.len(), 2);

        services.passkeys.delete(&tenant, ada, first.id).await.unwrap();
        let gone = services.passkeys.delete(&tenant, ada, first.id).await.unwrap_err();
        assert_eq!(gone.code(), "auth.passkey_not_found");
        let (ceremony, response) = sign_in(&services.passkeys, &tenant, &mut phone).await;
        let removed =
            services.passkeys.finish_login(&tenant, ceremony, &response, None).await.unwrap_err();
        assert_eq!(removed.code(), "auth.passkey_sign_in_failed");

        // Tài khoản bị khoá: passkey còn đó nhưng không đăng nhập được.
        let disable = UserChanges { name: None, disabled: Some(true) };
        services.users.update(&tenant, Actor::system(), ada, disable, None).await.unwrap();
        let (ceremony, response) = sign_in(&services.passkeys, &tenant, &mut key).await;
        let disabled =
            services.passkeys.finish_login(&tenant, ceremony, &response, None).await.unwrap_err();
        assert_eq!(disabled.code(), "auth.user_disabled");
        // Người khác không xoá được passkey của Ada.
        let bob = user(&services, "bob@example.com").await;
        let keys = list(&services, ada).await;
        assert_eq!(
            services.passkeys.delete(&tenant, bob, keys[0].id).await.unwrap_err().code(),
            "auth.passkey_not_found"
        );
    })
    .await
    .unwrap();
}

/// Access token bị lộ không đủ để gắn passkey; chủ thật đặt lại mật khẩu qua email thì
/// passkey và API key gắn trước đó bị thu hồi (D-079).
#[tokio::test]
async fn can_mat_khau_va_dat_lai_mat_khau_thu_hoi_passkey() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let tenant = services.tenant.clone();
        let ada = user(&services, "ada@example.com").await;
        let wrong =
            services.passkeys.start_registration(&tenant, ada, "guess").await.err().unwrap();
        assert_eq!(wrong.code(), "auth.wrong_password");

        let mut attacker = SoftAuthenticator::new(9);
        register(&services, ada, &mut attacker).await;
        let key = crate::api_key::NewApiKey {
            org_id: None,
            name: "agent".to_owned(),
            permissions: vec!["auth.users.read".to_owned()],
            expires_at: None,
            current_password: PASSWORD.to_owned(),
            second_factor: None,
        };
        // Ada chưa có role nào: thử với quyền admin để key tạo được.
        let admin = autoxide_kernel::access::Principal::with_all_permissions(ada);
        services.api_keys.create(&tenant, &admin, key).await.unwrap();

        crate::testing::request_reset(
            &services,
            "ada@example.com",
            autoxide_kernel::i18n::Language::En,
        )
        .await;
        let token = crate::testing::last_token(&services);
        services
            .resets
            .confirm(Some(&tenant), &token, "a brand new passphrase".to_owned())
            .await
            .unwrap();

        assert!(list(&services, ada).await.is_empty());
        let keys = services
            .api_keys
            .list(&tenant, ada, &PageRequest::first(10, Order::Asc))
            .await
            .unwrap();
        assert!(keys.data.is_empty());
        let (ceremony, response) = sign_in(&services.passkeys, &tenant, &mut attacker).await;
        let refused =
            services.passkeys.finish_login(&tenant, ceremony, &response, None).await.unwrap_err();
        assert_eq!(refused.code(), "auth.passkey_sign_in_failed");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chua_cau_hinh_thi_passkey_tat() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let tenant = services.tenant.clone();
        let off = PasskeyService::new(
            services.users.clone(),
            services.sessions.clone(),
            autoxide_kernel::event::Outbox::standalone("auth"),
            None,
        )
        .unwrap();

        let error = off.start_login(&tenant).await.err().unwrap();

        assert_eq!((error.kind(), error.code()), (ErrorKind::Forbidden, "auth.passkeys_disabled"));
        assert!(off.start_registration(&tenant, Uuid::now_v7(), PASSWORD).await.is_err());
    })
    .await
    .unwrap();
}

/// Nhiều request cùng hoàn tất MỘT nghi thức đăng nhập: đúng một bên được — trên cả hai
/// database.
#[tokio::test]
async fn dung_dong_thoi_mot_nghi_thuc_chi_mot_ben_duoc() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let tenant = services.tenant.clone();
        let ada = user(&services, "ada@example.com").await;
        let mut phone = SoftAuthenticator::new(8);
        register(&services, ada, &mut phone).await;
        let passkeys: Arc<PasskeyService> = services.passkeys.clone();
        let (ceremony, response) = sign_in(&passkeys, &tenant, &mut phone).await;

        let attempts: Vec<_> = (0..5)
            .map(|_| {
                let (passkeys, response) = (passkeys.clone(), response.clone());
                let tenant = tenant.clone();
                tokio::spawn(async move {
                    passkeys.finish_login(&tenant, ceremony, &response, None).await
                })
            })
            .collect();
        let mut succeeded = 0;
        for attempt in attempts {
            match attempt.await.unwrap() {
                Ok(_) => succeeded += 1,
                Err(error) => assert_eq!(error.code(), "auth.invalid_passkey_ceremony"),
            }
        }

        assert_eq!(succeeded, 1);
    })
    .await
    .unwrap();
}

/// Kẻ gian giữ mật khẩu cũ mở sẵn ba thứ rồi chờ: nghi thức thêm passkey, mail đổi email về địa
/// chỉ của nó, lượt chờ mã MFA. Chủ tài khoản đặt lại mật khẩu: cả ba hết dùng được (D-156). Đổi
/// email thì mail đặt lại gửi tới địa chỉ cũ hết dùng được.
#[tokio::test]
async fn dat_lai_mat_khau_dong_moi_cua_mo_bang_mat_khau_cu() {
    TestDatabase::run(async |test| {
        use autoxide_kernel::i18n::Language;
        let services = crate::testing::services(test).await;
        let tenant = services.tenant.clone();
        let ada = user(&services, "ada@example.com").await;
        let mut attacker = SoftAuthenticator::new(7);

        // 1. Nghi thức thêm passkey mở bằng mật khẩu cũ.
        let ceremony = services.passkeys.start_registration(&tenant, ada, PASSWORD).await.unwrap();
        let credential = attacker.register(&ceremony.options);
        // 2. Mail đổi email về địa chỉ của kẻ gian.
        services
            .verifications
            .change_email(&tenant, ada, PASSWORD, "evil@example.com", Language::En)
            .await
            .unwrap();
        let change_token = crate::testing::last_token(&services);
        // 3. Lượt chờ mã MFA (TOTP đã bật, kẻ gian có một recovery code).
        let setup = services.mfa.start_totp(&tenant, ada, "ada@example.com").await.unwrap();
        let now = u64::try_from(chrono::Utc::now().timestamp()).unwrap();
        let code = crate::mfa::service::tests::code_at(&setup.secret, now);
        let recovery = services.mfa.confirm_totp(&tenant, ada, &code).await.unwrap();
        let crate::session::LoginOutcome::MfaRequired { mfa_token, .. } =
            services.sessions.login(&tenant, "ada@example.com", PASSWORD, None).await.unwrap()
        else {
            panic!("MFA is on");
        };

        // Chủ tài khoản lấy lại tài khoản.
        crate::testing::request_reset(&services, "ada@example.com", Language::En).await;
        let reset = crate::testing::last_token(&services);
        services
            .resets
            .confirm(Some(&tenant), &reset, "a brand new passphrase".to_owned())
            .await
            .unwrap();

        let added =
            services.passkeys.finish_registration(&tenant, ada, ceremony.id, "Evil", &credential);
        assert_eq!(added.await.unwrap_err().code(), "auth.invalid_passkey_ceremony");
        assert!(list(&services, ada).await.is_empty());
        let changed = services
            .verifications
            .confirm(Some(&tenant), &change_token, Language::En)
            .await
            .unwrap_err();
        assert_eq!(changed.code(), "auth.invalid_verification_token");
        let finished =
            services.sessions.complete_mfa(Some(&tenant), &mfa_token, &recovery[0], None).await;
        assert_eq!(finished.unwrap_err().code(), "auth.invalid_mfa_token");
        let still = services.users.get(&tenant, ada).await.unwrap().unwrap();
        assert_eq!(still.email, "ada@example.com");

        // Đổi email (đúng chủ): mail đặt lại đã gửi tới địa chỉ cũ hết dùng được. (Bỏ lượt chặn
        // gửi dồn 60 giây của mail đặt lại vừa rồi.)
        use sea_orm::ConnectionTrait as _;
        let older = sea_orm::sea_query::Query::update()
            .table(sea_orm::sea_query::Alias::new("auth_tokens"))
            .value(
                sea_orm::sea_query::Alias::new("created_at"),
                chrono::Utc::now() - chrono::Duration::minutes(5),
            )
            .to_owned();
        services.database.write().execute(&older).await.unwrap();
        crate::testing::request_reset(&services, "ada@example.com", Language::En).await;
        let old_reset = crate::testing::last_token(&services);
        services
            .verifications
            .change_email(
                &tenant,
                ada,
                "a brand new passphrase",
                "ada@new.example.com",
                Language::En,
            )
            .await
            .unwrap();
        let confirm = crate::testing::last_token(&services);
        services.verifications.confirm(Some(&tenant), &confirm, Language::En).await.unwrap();
        let stale = services
            .resets
            .confirm(Some(&tenant), &old_reset, "another new passphrase".to_owned())
            .await;
        assert_eq!(stale.unwrap_err().code(), "auth.invalid_reset_token");
    })
    .await
    .unwrap();
}

/// Khoá mới cho test xác nhận lại (D-272).
fn agent_key(
    password: &str,
    second_factor: Option<crate::api_key::Reauth>,
) -> crate::api_key::NewApiKey {
    crate::api_key::NewApiKey {
        org_id: None,
        name: "agent".to_owned(),
        permissions: vec!["auth.users.read".to_owned()],
        expires_at: None,
        current_password: password.to_owned(),
        second_factor,
    }
}

/// Đề bài giao diện D: tạo API key xác nhận bằng passkey hay mã TOTP thay cho mật khẩu; passkey
/// của người khác, mã sai, gửi cả mật khẩu lẫn yếu tố khác đều bị từ chối. Quản trị viên có
/// `auth.api_keys.manage` xem và thu hồi key của mọi người.
#[tokio::test]
async fn tao_api_key_xac_nhan_bang_passkey_hay_ma_va_quan_tri_thu_hoi() {
    TestDatabase::run(async |test| {
        use crate::api_key::Reauth;
        let services = crate::testing::services(test).await;
        let tenant = services.tenant.clone();
        let ada = user(&services, "ada@example.com").await;
        let bob = user(&services, "bob@example.com").await;
        let as_ada = autoxide_kernel::access::Principal::with_all_permissions(ada);
        let mut ada_phone = SoftAuthenticator::new(3);
        register(&services, ada, &mut ada_phone).await;
        let mut bob_phone = SoftAuthenticator::new(4);
        register(&services, bob, &mut bob_phone).await;

        let (ceremony_id, credential) = sign_in(&services.passkeys, &tenant, &mut ada_phone).await;
        let proof = Reauth::Passkey { ceremony_id, credential };
        let made =
            services.api_keys.create(&tenant, &as_ada, agent_key("", Some(proof))).await.unwrap();
        assert_eq!(made.key.user_id, ada);

        let (ceremony_id, credential) = sign_in(&services.passkeys, &tenant, &mut bob_phone).await;
        let proof = Reauth::Passkey { ceremony_id, credential };
        let refused = services.api_keys.create(&tenant, &as_ada, agent_key("", Some(proof))).await;
        assert_eq!(refused.err().unwrap().code(), "auth.passkey_sign_in_failed");

        let both = agent_key(PASSWORD, Some(Reauth::Totp("123456".to_owned())));
        let error = services.api_keys.create(&tenant, &as_ada, both).await.err().unwrap();
        assert_eq!(error.kind(), ErrorKind::Validation);
        let no_totp = agent_key("", Some(Reauth::Totp("123456".to_owned())));
        let error = services.api_keys.create(&tenant, &as_ada, no_totp).await.err().unwrap();
        assert_eq!(error.code(), "auth.mfa_not_enabled");

        let setup = services.mfa.start_totp(&tenant, ada, "ada@example.com").await.unwrap();
        let now = u64::try_from(chrono::Utc::now().timestamp()).unwrap();
        let code = crate::mfa::service::tests::code_at(&setup.secret, now);
        services.mfa.confirm_totp(&tenant, ada, &code).await.unwrap();
        let wrong = agent_key("", Some(Reauth::Totp("000000".to_owned())));
        let error = services.api_keys.create(&tenant, &as_ada, wrong).await.err().unwrap();
        assert_eq!(error.fields()[0].field, "totp_code", "{error:?}");
        let next = crate::mfa::service::tests::code_at(&setup.secret, now + 30);
        let by_code = agent_key("", Some(Reauth::Totp(next)));
        services.api_keys.create(&tenant, &as_ada, by_code).await.unwrap();

        // Quản trị viên: xem key của mọi người, thu hồi một key của Ada.
        let first = PageRequest::first(50, Order::Asc);
        let nobody = autoxide_kernel::access::Principal::new(bob, Vec::<String>::new());
        let denied =
            services.api_keys.list_all(&tenant, &nobody, None, &first).await.err().unwrap();
        assert_eq!(denied.code(), "permission_denied");
        let admin =
            autoxide_kernel::access::Principal::new(bob, ["auth.api_keys.manage".to_owned()]);
        let all = services.api_keys.list_all(&tenant, &admin, Some(ada), &first).await.unwrap();
        assert_eq!(all.data.len(), 2);
        services.api_keys.revoke_any(&tenant, &admin, made.key.id).await.unwrap();
        let left = services.api_keys.list(&tenant, ada, &first).await.unwrap();
        assert_eq!(left.data.len(), 1);
        let again = services.api_keys.revoke_any(&tenant, &admin, made.key.id).await.err().unwrap();
        assert_eq!(again.code(), "auth.api_key_not_found");
    })
    .await
    .unwrap();
}
