//! Job định kỳ dọn dữ liệu đã hết hạn của module `auth`: số đếm đăng nhập sai cũ,
//! refresh token hết hạn hay đã thu hồi, lượt MFA hết hạn. Không có job này các bảng
//! đó lớn mãi (D-047).

use std::time::Duration;

use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::job::{JobContext, JobHandler, Schedule};
use chrono::Utc;
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Condition, Expr, ExprTrait as _, Query};

/// Loại job (cũng là tên lịch).
pub(crate) const KIND: &str = "auth.cleanup";

/// Chạy mỗi giờ.
pub(crate) fn schedule() -> Schedule {
    Schedule::new(KIND, KIND, Duration::from_secs(3600))
}

pub(crate) struct Cleanup {
    database: Database,
}

impl Cleanup {
    pub(crate) fn new(database: Database) -> Self {
        Cleanup { database }
    }
}

#[async_trait::async_trait]
impl JobHandler for Cleanup {
    async fn run(&self, _job: &JobContext) -> Result<(), Error> {
        let now = Utc::now();
        let day_ago = now - chrono::Duration::days(1);
        let connection = self.database.write();
        let mut removed = 0;
        // Số đếm của cửa sổ đã qua lâu và không còn khoá (bảng `auth_throttle` của kernel).
        let throttle = autoxide_kernel::throttle::Throttle::new(
            self.database.clone(),
            "auth",
            1,
            std::time::Duration::from_secs(60),
        )?;
        removed += throttle.purge(day_ago).await?;
        // Refresh token đã hết hạn, hay đã thu hồi hơn một ngày (gửi lại token đã xoá
        // vẫn bị từ chối như token lạ).
        removed += delete(
            connection,
            "auth_refresh_token",
            Condition::any()
                .add(Expr::col(Alias::new("expires_at")).lt(now))
                .add(Expr::col(Alias::new("revoked_at")).lt(day_ago)),
        )
        .await?;
        removed += delete(
            connection,
            "auth_mfa_challenge",
            Condition::all().add(Expr::col(Alias::new("expires_at")).lt(now)),
        )
        .await?;
        // Token đặt lại mật khẩu và xác minh email đã hết hạn hay đã dùng (bảng `auth_tokens`
        // của kernel).
        removed +=
            autoxide_kernel::tokens::OneTimeTokens::new("auth").purge(connection, now).await?;
        // Nghi thức passkey bỏ dở.
        let expired = Condition::all().add(Expr::col(Alias::new("expires_at")).lt(now));
        removed += delete(connection, "auth_passkey_ceremony", expired).await?;
        tracing::info!(removed, "expired sign-in data removed");
        Ok(())
    }
}

async fn delete<C: ConnectionTrait>(
    connection: &C,
    table: &str,
    condition: Condition,
) -> Result<u64, Error> {
    let mut statement = Query::delete();
    statement.from_table(Alias::new(table)).cond_where(condition);
    Ok(connection.execute(&statement).await?.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::event::Actor;
    use autoxide_testkit::TestDatabase;
    use sea_orm::Statement;
    use uuid::Uuid;

    use crate::session::LoginOutcome;
    use crate::user::NewUser;

    async fn count(database: &Database, table: &str) -> i64 {
        let backend = database.write().get_database_backend();
        let sql = format!("SELECT COUNT(*) AS n FROM {table}");
        let row = database
            .write()
            .query_one_raw(Statement::from_string(backend, sql))
            .await
            .unwrap()
            .unwrap();
        row.try_get("", "n").unwrap()
    }

    async fn age(database: &Database, sql: &str) {
        let backend = database.write().get_database_backend();
        database.write().execute_raw(Statement::from_string(backend, sql)).await.unwrap();
    }

    #[tokio::test]
    async fn don_du_lieu_het_han_giu_du_lieu_con_dung() {
        TestDatabase::run(async |test| {
            let services = crate::testing::services(test).await;
            let database = services.database.clone();
            let input = NewUser {
                email: "ada@example.com".to_owned(),
                name: "Ada".to_owned(),
                password: Some("correct horse battery staple".to_owned()),
            };
            services.users.create(Actor::system(), input).await.unwrap();
            let old = match services
                .sessions
                .login("ada@example.com", "correct horse battery staple", None)
                .await
                .unwrap()
            {
                LoginOutcome::Tokens(tokens) => tokens,
                LoginOutcome::MfaRequired { .. } => panic!("no MFA here"),
            };
            services
                .sessions
                .login("ada@example.com", "correct horse battery staple", None)
                .await
                .unwrap();
            services.sessions.login("bob@example.com", "wrong", None).await.unwrap_err();
            services.sessions.login("eve@example.com", "wrong", None).await.unwrap_err();
            // Một token và một số đếm đã cũ.
            age(&database, "UPDATE auth_refresh_token SET expires_at = created_at").await;
            services
                .sessions
                .login("ada@example.com", "correct horse battery staple", None)
                .await
                .unwrap();
            age(
                &database,
                "UPDATE auth_throttle SET window_started_at = '2000-01-01 00:00:00+00:00'",
            )
            .await;
            services.sessions.login("bob@example.com", "wrong", None).await.unwrap_err();
            assert_eq!(count(&database, "auth_refresh_token").await, 3);

            let context = JobContext {
                id: Uuid::now_v7(),
                kind: KIND.to_owned(),
                payload: serde_json::Value::Null,
                attempt: 1,
            };
            Cleanup::new(database.clone()).run(&context).await.unwrap();

            // Còn token mới nhất (chưa hết hạn) và số đếm vừa ghi của bob.
            assert_eq!(count(&database, "auth_refresh_token").await, 1);
            assert_eq!(count(&database, "auth_throttle").await, 1);
            assert!(services.sessions.refresh(&old.refresh_token).await.is_err());
        })
        .await
        .unwrap();
    }
}
