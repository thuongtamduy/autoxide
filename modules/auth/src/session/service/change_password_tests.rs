use super::*;
use crate::user::NewUser;
use autoxide_testkit::TestDatabase;

const OLD: &str = "correct horse battery staple";
const NEW: &str = "a brand new passphrase";

async fn setup(test: &TestDatabase) -> (Arc<SessionService>, User, TenantContext) {
    let (sessions, users, tenant) = crate::testing::sessions(test).await;
    let ada = NewUser {
        email: "ada@example.com".to_owned(),
        name: "Ada".to_owned(),
        password: Some(OLD.to_owned()),
    };
    let user = users.create(&tenant, Actor::system(), ada).await.unwrap();
    (sessions, user, tenant)
}

#[tokio::test]
async fn doi_mat_khau_can_mat_khau_cu_va_cap_phien_moi() {
    TestDatabase::run(async |test| {
        let (sessions, user, tenant) = setup(test).await;
        let old_session =
            sessions.login(&tenant, "ada@example.com", OLD, None).await.unwrap().tokens();

        let wrong =
            sessions.change_password(&tenant, user.id, "not my password", NEW.to_owned(), None);
        assert_eq!(wrong.await.unwrap_err().code(), "auth.wrong_password");
        let short = sessions.change_password(&tenant, user.id, OLD, "short".to_owned(), None);
        assert_eq!(short.await.unwrap_err().code(), "validation_failed");

        let tokens =
            sessions.change_password(&tenant, user.id, OLD, NEW.to_owned(), None).await.unwrap();

        // Phiên cũ kết thúc; phiên mới dùng tiếp được.
        assert!(sessions.refresh(Some(&tenant), &old_session.refresh_token).await.is_err());
        sessions.refresh(Some(&tenant), &tokens.refresh_token).await.unwrap();
        assert!(sessions.login(&tenant, "ada@example.com", OLD, None).await.is_err());
        sessions.login(&tenant, "ada@example.com", NEW, None).await.unwrap().tokens();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doan_mat_khau_cu_nhieu_lan_bi_khoa() {
    TestDatabase::run(async |test| {
        let (sessions, user, tenant) = setup(test).await;
        for _ in 0..5 {
            let wrong =
                sessions.change_password(&tenant, user.id, "guess", NEW.to_owned(), None).await;
            assert_eq!(wrong.unwrap_err().code(), "auth.wrong_password");
        }

        // Đã khoá: kể cả mật khẩu đúng cũng phải chờ.
        let locked = sessions.change_password(&tenant, user.id, OLD, NEW.to_owned(), None).await;
        assert_eq!(locked.unwrap_err().code(), "auth.login_throttled");
    })
    .await
    .unwrap();
}
