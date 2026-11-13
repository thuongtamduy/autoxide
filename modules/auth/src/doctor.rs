//! Bước kiểm của `auth` cho `autoxide doctor` (D-166): đã có quản trị viên đang hoạt động chưa.
//! Bản cài mới chưa có ai thì không ai đăng nhập quản lý được — lỗi hay gặp nhất của người tự cài.

use autoxide_kernel::database::Database;
use autoxide_kernel::doctor::{Finding, Probe};

use crate::role::count_active_admins_unlocked;

pub(crate) struct Administrator {
    pub database: Database,
}

#[async_trait::async_trait]
impl Probe for Administrator {
    async fn probe(&self) -> Finding {
        match count_active_admins_unlocked(self.database.read()).await {
            Ok(0) => Finding::warning("No active account has the `admin` role, so nobody can sign in to manage this server.")
                .with_hint("Run `autoxide user create --email <email> --name <name> --admin`."),
            Ok(count) => Finding::ok(format!("{count} active administrator(s).")),
            Err(error) => Finding::failed(format!("cannot count administrators: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::doctor::Status;
    use autoxide_kernel::event::Actor;
    use autoxide_testkit::TestDatabase;

    use super::*;
    use crate::user::NewUser;

    /// Chưa có admin thì cảnh báo kèm lệnh tạo; có rồi thì đạt — trên cả hai database.
    #[tokio::test]
    async fn bao_khi_chua_co_quan_tri_vien() {
        TestDatabase::run(async |test| {
            let services = crate::testing::services(test).await;
            let probe = Administrator { database: services.database.clone() };

            let finding = probe.probe().await;
            assert_eq!(finding.status, Status::Warning, "{finding:?}");
            assert!(finding.hint.unwrap().contains("--admin"));

            let user = |email: &str| NewUser {
                email: email.to_owned(),
                name: "Ada".to_owned(),
                password: None,
            };
            services.users.create(Actor::system(), user("member@example.com")).await.unwrap();
            assert_eq!(probe.probe().await.status, Status::Warning);
            let admin = user("admin@example.com");
            services.users.create_with_roles(Actor::system(), admin, true).await.unwrap();
            let finding = probe.probe().await;
            assert_eq!(
                (finding.status, finding.message.as_str()),
                (Status::Ok, "1 active administrator(s).")
            );
        })
        .await
        .unwrap();
    }
}
