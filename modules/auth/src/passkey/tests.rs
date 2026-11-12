//! Test passkey trên database gate đang chọn (gate chạy cả SQLite và Postgres), với
//! authenticator phần mềm thay cho thiết bị thật.

use std::sync::Arc;

use autoxide_kernel::error::ErrorKind;
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Order, PageRequest};
use autoxide_testkit::TestDatabase;
use serde_json::Value;
use uuid::Uuid;

use super::testing::SoftAuthenticator;
use super::{Passkey, PasskeyService};
use crate::testing::Services;
use crate::user::{NewUser, UserChanges};

const PASSWORD: &str = "correct horse battery staple";

async fn user(services: &Services, email: &str) -> Uuid {
    let input = NewUser {
        email: email.to_owned(),
        name: "Ada".to_owned(),
        password: Some(PASSWORD.to_owned()),
    };
    services.users.create(Actor::system(), input).await.unwrap().id
}

/// Đăng ký passkey của `device` cho `user_id`.
async fn register(services: &Services, user_id: Uuid, device: &mut SoftAuthenticator) -> Passkey {
    let passkeys = &services.passkeys;
    let ceremony = passkeys.start_registration(user_id, PASSWORD).await.unwrap();
    let credential = device.register(&ceremony.options);
    passkeys.finish_registration(user_id, ceremony.id, "Phone", &credential).await.unwrap()
}

/// Một lượt đăng nhập: (ceremony id, câu trả lời của `device`).
async fn sign_in(passkeys: &PasskeyService, device: &mut SoftAuthenticator) -> (Uuid, Value) {
    let ceremony = passkeys.start_login().await.unwrap();
    (ceremony.id, device.sign_in(&ceremony.options))
}

async fn list(services: &Services, user_id: Uuid) -> Vec<Passkey> {
    let page = PageRequest::first(50, Order::Asc);
    services.passkeys.list(user_id, &page).await.unwrap().data
}

#[tokio::test]
async fn dang_ky_roi_dang_nhap_bang_passkey() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let ada = user(&services, "ada@example.com").await;
        let mut phone = SoftAuthenticator::new(1);

        let added = register(&services, ada, &mut phone).await;
        assert_eq!((added.name.as_str(), added.last_used_at), ("Phone", None));

        let (ceremony, response) = sign_in(&services.passkeys, &mut phone).await;
        let tokens = services.passkeys.finish_login(ceremony, &response, None).await.unwrap();

        let bearer = format!("Bearer {}", tokens.access_token);
        assert_eq!(services.sessions.authenticate(Some(&bearer)).await.unwrap().id, ada);
        let stored = list(&services, ada).await;
        assert!(stored[0].last_used_at.is_some());
        // Bộ đếm chữ ký đã lưu: lần sau vẫn đăng nhập được (bộ đếm tăng tiếp).
        let (ceremony, response) = sign_in(&services.passkeys, &mut phone).await;
        services.passkeys.finish_login(ceremony, &response, None).await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nghi_thuc_dung_mot_lan_va_khong_gui_lai_duoc() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let ada = user(&services, "ada@example.com").await;
        let mut phone = SoftAuthenticator::new(1);
        let passkeys = &services.passkeys;

        // Đăng ký: dùng lại cùng nghi thức thì bị từ chối.
        let ceremony = passkeys.start_registration(ada, PASSWORD).await.unwrap();
        let credential = phone.register(&ceremony.options);
        passkeys.finish_registration(ada, ceremony.id, "Phone", &credential).await.unwrap();
        let again = passkeys.finish_registration(ada, ceremony.id, "Phone", &credential).await;
        assert_eq!(again.unwrap_err().code(), "auth.invalid_passkey_ceremony");

        // Đăng nhập: gửi lại câu trả lời cũ cho nghi thức mới (challenge khác) thì hỏng.
        let (first, response) = sign_in(passkeys, &mut phone).await;
        passkeys.finish_login(first, &response, None).await.unwrap();
        let replayed = passkeys.finish_login(first, &response, None).await.unwrap_err();
        assert_eq!(replayed.code(), "auth.invalid_passkey_ceremony");
        let fresh = passkeys.start_login().await.unwrap();
        let stolen = passkeys.finish_login(fresh.id, &response, None).await.unwrap_err();
        assert_eq!(
            (stolen.kind(), stolen.code()),
            (ErrorKind::Unauthenticated, "auth.passkey_sign_in_failed")
        );

        // Nghi thức đăng ký của người khác không dùng được.
        let bob = user(&services, "bob@example.com").await;
        let theirs = passkeys.start_registration(ada, PASSWORD).await.unwrap();
        let wrong = passkeys.finish_registration(bob, theirs.id, "Phone", &credential).await;
        assert_eq!(wrong.unwrap_err().code(), "auth.invalid_passkey_ceremony");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cau_tra_loi_sai_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let ada = user(&services, "ada@example.com").await;
        let passkeys = &services.passkeys;

        // Trang web lạ (origin không được phép) không đăng ký được.
        let mut phishing = SoftAuthenticator::new(2);
        phishing.origin = "https://evil.example.net".to_owned();
        let ceremony = passkeys.start_registration(ada, PASSWORD).await.unwrap();
        let credential = phishing.register(&ceremony.options);
        let rejected = passkeys.finish_registration(ada, ceremony.id, "x", &credential).await;
        let rejected = rejected.unwrap_err();
        assert_eq!(
            (rejected.kind(), rejected.code()),
            (ErrorKind::BadRequest, "auth.passkey_rejected")
        );

        // Passkey chưa đăng ký ở đây.
        let mut stranger = SoftAuthenticator::new(3);
        stranger.user_handle = vec![7; 64];
        let (ceremony, response) = sign_in(passkeys, &mut stranger).await;
        let unknown = passkeys.finish_login(ceremony, &response, None).await.unwrap_err();
        assert_eq!(unknown.code(), "auth.passkey_sign_in_failed");

        // Đúng credential nhưng tự nhận là người khác (user handle khác).
        let mut phone = SoftAuthenticator::new(4);
        register(&services, ada, &mut phone).await;
        phone.user_handle = vec![9; 64];
        let (ceremony, response) = sign_in(passkeys, &mut phone).await;
        let impostor = passkeys.finish_login(ceremony, &response, None).await.unwrap_err();
        assert_eq!(impostor.code(), "auth.passkey_sign_in_failed");

        // JSON không phải câu trả lời WebAuthn: lỗi đầu vào, nghi thức còn nguyên.
        let ceremony = passkeys.start_login().await.unwrap();
        let invalid = passkeys.finish_login(ceremony.id, &serde_json::json!({"x": 1}), None).await;
        assert_eq!(invalid.unwrap_err().code(), "validation_failed");
        let name = passkeys.finish_registration(ada, Uuid::now_v7(), " ", &credential).await;
        assert_eq!(name.unwrap_err().code(), "validation_failed");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nhieu_passkey_xoa_va_tai_khoan_bi_khoa() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let ada = user(&services, "ada@example.com").await;
        let (mut phone, mut key) = (SoftAuthenticator::new(5), SoftAuthenticator::new(6));

        let first = register(&services, ada, &mut phone).await;
        let ceremony = services.passkeys.start_registration(ada, PASSWORD).await.unwrap();
        // Passkey đã có được báo cho trình duyệt (không tạo trùng trên cùng thiết bị), và
        // mọi passkey của một người dùng chung một user handle.
        assert_eq!(ceremony.options["excludeCredentials"].as_array().unwrap().len(), 1);
        let credential = key.register(&ceremony.options);
        services.passkeys.finish_registration(ada, ceremony.id, "Key", &credential).await.unwrap();
        assert_eq!(phone.user_handle, key.user_handle);
        assert_eq!(list(&services, ada).await.len(), 2);

        services.passkeys.delete(ada, first.id).await.unwrap();
        let gone = services.passkeys.delete(ada, first.id).await.unwrap_err();
        assert_eq!(gone.code(), "auth.passkey_not_found");
        let (ceremony, response) = sign_in(&services.passkeys, &mut phone).await;
        let removed = services.passkeys.finish_login(ceremony, &response, None).await.unwrap_err();
        assert_eq!(removed.code(), "auth.passkey_sign_in_failed");

        // Tài khoản bị khoá: passkey còn đó nhưng không đăng nhập được.
        let disable = UserChanges { name: None, disabled: Some(true) };
        services.users.update(Actor::system(), ada, disable, None).await.unwrap();
        let (ceremony, response) = sign_in(&services.passkeys, &mut key).await;
        let disabled = services.passkeys.finish_login(ceremony, &response, None).await.unwrap_err();
        assert_eq!(disabled.code(), "auth.user_disabled");
        // Người khác không xoá được passkey của Ada.
        let bob = user(&services, "bob@example.com").await;
        let keys = list(&services, ada).await;
        assert_eq!(
            services.passkeys.delete(bob, keys[0].id).await.unwrap_err().code(),
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
        let ada = user(&services, "ada@example.com").await;
        let wrong = services.passkeys.start_registration(ada, "guess").await.err().unwrap();
        assert_eq!(wrong.code(), "auth.wrong_password");

        let mut attacker = SoftAuthenticator::new(9);
        register(&services, ada, &mut attacker).await;
        let key = crate::api_key::NewApiKey {
            name: "agent".to_owned(),
            permissions: vec!["auth.users.read".to_owned()],
            expires_at: None,
            current_password: PASSWORD.to_owned(),
        };
        // Ada chưa có role nào: thử với quyền admin để key tạo được.
        let admin = autoxide_kernel::access::Principal::with_all_permissions(ada);
        services.api_keys.create(&admin, key).await.unwrap();

        services
            .resets
            .request("ada@example.com", autoxide_kernel::i18n::Language::En)
            .await
            .unwrap();
        let token = crate::testing::last_token(&services);
        services.resets.confirm(&token, "a brand new passphrase".to_owned()).await.unwrap();

        assert!(list(&services, ada).await.is_empty());
        let keys = services.api_keys.list(ada, &PageRequest::first(10, Order::Asc)).await.unwrap();
        assert!(keys.data.is_empty());
        let (ceremony, response) = sign_in(&services.passkeys, &mut attacker).await;
        let refused = services.passkeys.finish_login(ceremony, &response, None).await.unwrap_err();
        assert_eq!(refused.code(), "auth.passkey_sign_in_failed");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chua_cau_hinh_thi_passkey_tat() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let off = PasskeyService::new(
            services.database.clone(),
            services.users.clone(),
            services.sessions.clone(),
            autoxide_kernel::event::Outbox::standalone("auth"),
            None,
        )
        .unwrap();

        let error = off.start_login().await.err().unwrap();

        assert_eq!((error.kind(), error.code()), (ErrorKind::Forbidden, "auth.passkeys_disabled"));
        assert!(off.start_registration(Uuid::now_v7(), PASSWORD).await.is_err());
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
        let ada = user(&services, "ada@example.com").await;
        let mut phone = SoftAuthenticator::new(8);
        register(&services, ada, &mut phone).await;
        let passkeys: Arc<PasskeyService> = services.passkeys.clone();
        let (ceremony, response) = sign_in(&passkeys, &mut phone).await;

        let attempts: Vec<_> = (0..5)
            .map(|_| {
                let (passkeys, response) = (passkeys.clone(), response.clone());
                tokio::spawn(async move { passkeys.finish_login(ceremony, &response, None).await })
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
