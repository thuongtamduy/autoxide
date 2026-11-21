use super::*;
use autoxide_kernel::error::ErrorKind;
use autoxide_kernel::pagination::Order;
use autoxide_testkit::TestDatabase;

const PASSWORD: &str = "correct horse battery staple";

async fn service(test: &TestDatabase) -> (UserService, TenantContext) {
    let database = crate::testing::database(test).await;
    let users = UserService::new(Passwords::new(), Outbox::standalone("auth"));
    (users, TenantContext::default_on(database))
}

fn new_user(email: &str) -> NewUser {
    NewUser {
        email: email.to_owned(),
        name: "Ada Lovelace".to_owned(),
        password: Some(PASSWORD.to_owned()),
    }
}

#[tokio::test]
async fn tao_roi_doc_lai_nguoi_dung() {
    TestDatabase::run(async |test| {
        let (users, tenant) = service(test).await;

        let created =
            users.create(&tenant, Actor::system(), new_user("  Ada@Example.COM ")).await.unwrap();

        assert_eq!(created.email, "ada@example.com");
        assert_eq!(created.name, "Ada Lovelace");
        assert!(created.is_active());
        assert_eq!(created.id.get_version_num(), 7);
        assert_eq!(users.get(&tenant, created.id).await.unwrap(), Some(created.clone()));
        assert_eq!(users.find_by_email(&tenant, "ADA@example.com").await.unwrap(), Some(created));
        assert_eq!(users.get(&tenant, Uuid::now_v7()).await.unwrap(), None);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn mat_khau_luu_duoi_dang_hash_argon2id() {
    TestDatabase::run(async |test| {
        let (users, tenant) = service(test).await;
        let created =
            users.create(&tenant, Actor::system(), new_user("ada@example.com")).await.unwrap();

        let row = Entity::find_by_id(created.id).one(tenant.database().read()).await.unwrap();

        let hash = row.unwrap().password_hash.unwrap();
        assert!(hash.starts_with("$argon2id$"), "{hash}");
        assert!(!hash.contains(PASSWORD));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn email_trung_khong_phan_biet_hoa_thuong_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let (users, tenant) = service(test).await;
        users.create(&tenant, Actor::system(), new_user("ada@example.com")).await.unwrap();

        let error =
            users.create(&tenant, Actor::system(), new_user("ADA@example.com")).await.unwrap_err();

        assert_eq!(error.kind(), ErrorKind::Conflict);
        assert_eq!(error.code(), "auth.email_taken");
        assert_eq!(
            error.message(),
            "A user with the email `ada@example.com` already exists. Use another email, or \
                 sign in with this one."
        );
    })
    .await
    .unwrap();
}

/// Nhiều request tạo cùng một email cùng lúc: đúng một cái thành công, còn lại
/// nhận `auth.email_taken` — trên cả hai database.
#[tokio::test]
async fn tao_dong_thoi_cung_email_chi_mot_thanh_cong() {
    TestDatabase::run(async |test| {
        let (users, tenant) = service(test).await;
        let users = std::sync::Arc::new(users);

        let attempts: Vec<_> = (0..6)
            .map(|_| {
                let users = users.clone();
                let tenant = tenant.clone();
                tokio::spawn(async move {
                    users.create(&tenant, Actor::system(), new_user("ada@example.com")).await
                })
            })
            .collect();
        let mut created = 0;
        for attempt in attempts {
            match attempt.await.unwrap() {
                Ok(_) => created += 1,
                Err(error) => assert_eq!(error.code(), "auth.email_taken", "{error:?}"),
            }
        }

        assert_eq!(created, 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn du_lieu_sai_bao_du_moi_truong() {
    TestDatabase::run(async |test| {
        let (users, tenant) = service(test).await;
        let input = NewUser {
            email: "not an email".to_owned(),
            name: "   ".to_owned(),
            password: Some("short".to_owned()),
        };

        let error = users.create(&tenant, Actor::system(), input).await.unwrap_err();

        assert_eq!(error.code(), "validation_failed");
        let fields: Vec<(&str, &str)> = error
            .fields()
            .iter()
            .map(|field| (field.field.as_str(), field.code.as_str()))
            .collect();
        assert_eq!(fields, [("email", "email"), ("name", "length"), ("password", "length")]);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn kiem_thong_tin_dang_nhap() {
    TestDatabase::run(async |test| {
        let (users, tenant) = service(test).await;
        let created =
            users.create(&tenant, Actor::system(), new_user("ada@example.com")).await.unwrap();

        let signed_in =
            users.check_credentials(&tenant, " ADA@example.com", PASSWORD).await.unwrap();
        let wrong = users.check_credentials(&tenant, "ada@example.com", "wrong password").await;
        let unknown = users.check_credentials(&tenant, "bob@example.com", PASSWORD).await;
        let huge = users.check_credentials(&tenant, "ada@example.com", &"a".repeat(10_000)).await;

        assert_eq!(signed_in, created);
        // Email lạ và mật khẩu sai: cùng một lỗi, không lộ email nào có thật.
        for error in [wrong, unknown, huge] {
            let error = error.unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Unauthenticated);
            assert_eq!(error.code(), "auth.invalid_credentials");
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tai_khoan_chua_co_mat_khau_khong_dang_nhap_duoc() {
    TestDatabase::run(async |test| {
        let (users, tenant) = service(test).await;
        let input = NewUser { password: None, ..new_user("ada@example.com") };
        users.create(&tenant, Actor::system(), input).await.unwrap();

        let error = users.check_credentials(&tenant, "ada@example.com", "").await.unwrap_err();

        assert_eq!(error.code(), "auth.invalid_credentials");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tai_khoan_bi_khoa_khong_dang_nhap_duoc() {
    TestDatabase::run(async |test| {
        let (users, tenant) = service(test).await;
        let created =
            users.create(&tenant, Actor::system(), new_user("ada@example.com")).await.unwrap();
        Entity::update_many()
            .col_expr(Column::DisabledAt, Expr::value(Some(now())))
            .filter(Column::Id.eq(created.id))
            .exec(tenant.database().write())
            .await
            .unwrap();

        let error =
            users.check_credentials(&tenant, "ada@example.com", PASSWORD).await.unwrap_err();

        assert_eq!(error.kind(), ErrorKind::Forbidden);
        assert_eq!(error.code(), "auth.user_disabled");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doi_mat_khau() {
    TestDatabase::run(async |test| {
        let (users, tenant) = service(test).await;
        let created =
            users.create(&tenant, Actor::system(), new_user("ada@example.com")).await.unwrap();
        let new_password = "a brand new passphrase";

        users
            .set_password(&tenant, Actor::system(), created.id, new_password.to_owned())
            .await
            .unwrap();

        assert!(users.check_credentials(&tenant, "ada@example.com", PASSWORD).await.is_err());
        let user = users.check_credentials(&tenant, "ada@example.com", new_password).await.unwrap();
        assert!(user.updated_at >= created.updated_at);

        let missing = users
            .set_password(&tenant, Actor::system(), Uuid::now_v7(), new_password.to_owned())
            .await;
        assert_eq!(missing.unwrap_err().code(), "auth.user_not_found");
        let short =
            users.set_password(&tenant, Actor::system(), created.id, "short".to_owned()).await;
        assert_eq!(short.unwrap_err().code(), "validation_failed");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hash_cu_duoc_hash_lai_khi_dang_nhap_dung() {
    use argon2::password_hash::PasswordHasher as _;
    use argon2::{Algorithm, Argon2, Params, Version};

    TestDatabase::run(async |test| {
        let (users, tenant) = service(test).await;
        let created =
            users.create(&tenant, Actor::system(), new_user("ada@example.com")).await.unwrap();
        let weak = Argon2::new(
            Algorithm::Argon2id,
            Version::V0x13,
            Params::new(8 * 1024, 1, 1, None).unwrap(),
        );
        let old = weak.hash_password(PASSWORD.as_bytes()).unwrap().to_string();
        Entity::update_many()
            .col_expr(Column::PasswordHash, Expr::value(Some(old.clone())))
            .filter(Column::Id.eq(created.id))
            .exec(tenant.database().write())
            .await
            .unwrap();

        users.check_credentials(&tenant, "ada@example.com", PASSWORD).await.unwrap();

        let row = Entity::find_by_id(created.id).one(tenant.database().read()).await.unwrap();
        let upgraded = row.unwrap().password_hash.unwrap();
        assert_ne!(upgraded, old);
        assert!(upgraded.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{upgraded}");
        users.check_credentials(&tenant, "ada@example.com", PASSWORD).await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn danh_sach_theo_trang() {
    TestDatabase::run(async |test| {
        let (users, tenant) = service(test).await;
        let mut created = Vec::new();
        for index in 0..3 {
            let input = NewUser { password: None, ..new_user(&format!("u{index}@example.com")) };
            created.push(users.create(&tenant, Actor::system(), input).await.unwrap().id);
        }

        let first =
            users.list(&tenant, None, None, &PageRequest::first(2, Order::Asc)).await.unwrap();
        let cursor: Uuid = first.pagination.next_cursor.as_deref().unwrap().parse().unwrap();
        let second = users
            .list(&tenant, None, None, &PageRequest::first(2, Order::Asc).after(cursor))
            .await
            .unwrap();

        let ids: Vec<Uuid> = first.data.iter().chain(&second.data).map(|user| user.id).collect();
        assert_eq!(ids, created);
        assert_eq!(second.pagination.next_cursor, None);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn trait_users_cho_module_khac() {
    TestDatabase::run(async |test| {
        let (service, tenant) = service(test).await;
        let service = std::sync::Arc::new(service);
        let created =
            service.create(&tenant, Actor::system(), new_user("ada@example.com")).await.unwrap();
        let users: std::sync::Arc<dyn Users> = service;

        assert_eq!(users.find(&tenant, created.id).await.unwrap(), Some(created));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn quan_tri_vien_chi_dat_duoc_mat_khau_dau_tien() {
    TestDatabase::run(async |test| {
        let (users, tenant) = service(test).await;
        let bob = NewUser { password: None, ..new_user("bob@example.com") };
        let bob = users.create(&tenant, Actor::system(), bob).await.unwrap();
        let ada =
            users.create(&tenant, Actor::system(), new_user("ada@example.com")).await.unwrap();

        let short =
            users.set_initial_password(&tenant, Actor::system(), bob.id, "short".to_owned());
        assert_eq!(short.await.unwrap_err().code(), "validation_failed");
        users
            .set_initial_password(&tenant, Actor::system(), bob.id, PASSWORD.to_owned())
            .await
            .unwrap();
        users.check_credentials(&tenant, "bob@example.com", PASSWORD).await.unwrap();

        for id in [bob.id, ada.id] {
            let taken =
                users.set_initial_password(&tenant, Actor::system(), id, PASSWORD.to_owned());
            let taken = taken.await.unwrap_err();
            assert_eq!(taken.kind(), ErrorKind::Conflict);
            assert_eq!(taken.code(), "auth.password_already_set");
        }
        let ghost =
            users.set_initial_password(&tenant, Actor::system(), Uuid::now_v7(), PASSWORD.into());
        assert_eq!(ghost.await.unwrap_err().code(), "auth.user_not_found");
    })
    .await
    .unwrap();
}

/// Hai quản trị viên cùng đặt mật khẩu đầu tiên: đúng một người được — trên cả hai
/// database.
#[tokio::test]
async fn dat_mat_khau_dau_tien_dong_thoi_chi_mot_ben_duoc() {
    TestDatabase::run(async |test| {
        let (users, tenant) = service(test).await;
        let users = std::sync::Arc::new(users);
        let bob = NewUser { password: None, ..new_user("bob@example.com") };
        let bob = users.create(&tenant, Actor::system(), bob).await.unwrap();

        let attempts: Vec<_> = (0..4)
            .map(|index| {
                let users = users.clone();
                let tenant = tenant.clone();
                tokio::spawn(async move {
                    let password = format!("{PASSWORD} number {index}");
                    users.set_initial_password(&tenant, Actor::system(), bob.id, password).await
                })
            })
            .collect();
        let mut succeeded = 0;
        for attempt in attempts {
            if attempt.await.unwrap().is_ok() {
                succeeded += 1;
            }
        }

        assert_eq!(succeeded, 1);
    })
    .await
    .unwrap();
}

/// Lấy lại tài khoản bằng mail đặt lại mật khẩu: liên kết OIDC, link đăng nhập và token xác nhận
/// lại phát trước đó không còn dùng được.
#[tokio::test]
async fn lay_lai_tai_khoan_go_lien_ket_va_token_cho() {
    TestDatabase::run(async |test| {
        let (users, tenant) = service(test).await;
        let user =
            users.create(&tenant, Actor::system(), new_user("ada@example.com")).await.unwrap();
        let external = crate::oidc::identity::External {
            subject: "sub-1".to_owned(),
            email: Some("ada@example.com".to_owned()),
            email_verified: true,
            name: None,
        };
        let write = tenant.database().write();
        crate::oidc::identity::insert(write, &tenant, user.id, Uuid::now_v7(), &external)
            .await
            .unwrap();
        let tokens = OneTimeTokens::new("auth");
        let subject = user.id.to_string();
        let ttl = jiff::SignedDuration::from_mins(10);
        let mut issued = Vec::new();
        for purpose in ["login_link", "reauth"] {
            let token = tokens.issue(write, &tenant, purpose, &subject, None, ttl).await.unwrap();
            issued.push((purpose, token.token));
        }
        users
            .recover_password(&tenant, Actor::system(), user.id, "another good password".to_owned())
            .await
            .unwrap();
        let linked = crate::oidc::identity::for_user(&tenant, user.id).await.unwrap();
        assert!(linked.is_empty(), "liên kết OIDC bị gỡ");
        for (purpose, token) in issued {
            let used = tokens.consume(write, &tenant, purpose, &token).await.unwrap();
            assert!(used.is_none(), "{purpose} bị thu hồi");
        }
    })
    .await
    .unwrap();
}
