//! Token đặt lại mật khẩu và xác minh email chuyển sang bảng chung của kernel (`auth_tokens`,
//! D-111). Token còn hạn, chưa dùng được chép sang (link trong mail đã gửi vẫn dùng được); hai
//! bảng cũ bị bỏ.

use autoxide_kernel::migration::prelude::*;
use autoxide_kernel::tokens::tokens_migration;
use chrono::{DateTime, Utc};
use sea_orm::ConnectionTrait;
use uuid::Uuid;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0019_move_tokens"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        tokens_migration("m0019_move_tokens", "auth").up(manager).await?;
        let connection = manager.get_connection();
        let now = Utc::now();
        for (table, purpose, email) in [
            ("auth_password_reset", "password_reset", false),
            ("auth_email_verification", "email_verification", true),
        ] {
            let mut columns = vec!["id", "user_id", "token_hash", "expires_at", "created_at"];
            if email {
                columns.push("email");
            }
            let select = Query::select()
                .columns(columns.iter().map(|column| Alias::new(*column)))
                .from(Alias::new(table))
                .and_where(Expr::col(Alias::new("used_at")).is_null())
                .and_where(Expr::col(Alias::new("expires_at")).gt(now))
                .to_owned();
            for row in connection.query_all(&select).await? {
                let data: Option<String> =
                    if email { Some(row.try_get("", "email")?) } else { None };
                let insert = Query::insert()
                    .into_table(Alias::new("auth_tokens"))
                    .columns(
                        [
                            "id",
                            "purpose",
                            "subject",
                            "data",
                            "token_hash",
                            "expires_at",
                            "created_at",
                        ]
                        .map(Alias::new),
                    )
                    .values_panic([
                        row.try_get::<Uuid>("", "id")?.into(),
                        purpose.into(),
                        row.try_get::<Uuid>("", "user_id")?.to_string().into(),
                        data.into(),
                        row.try_get::<String>("", "token_hash")?.into(),
                        row.try_get::<DateTime<Utc>>("", "expires_at")?.into(),
                        row.try_get::<DateTime<Utc>>("", "created_at")?.into(),
                    ])
                    .to_owned();
                connection.execute(&insert).await?;
            }
            manager.drop_table(Table::drop().table(Alias::new(table)).to_owned()).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Dựng lại hai bảng cũ đúng như m0011, m0013 (+ cột của m0012 không chạm hai bảng
        // này); token đang chờ không được chép ngược — người dùng xin mail mới.
        super::m0011_create_password_reset::Migration.up(manager).await?;
        super::m0013_create_email_verification::Migration.up(manager).await?;
        tokens_migration("m0019_move_tokens", "auth").down(manager).await
    }
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
    use autoxide_kernel::database::Database;
    use autoxide_kernel::migration::{Migrator, ModuleMigrations};
    use autoxide_kernel::secret::sha256;
    use autoxide_kernel::tokens::OneTimeTokens;
    use autoxide_testkit::TestDatabase;

    use super::*;

    /// Token còn hạn trong hai bảng cũ vẫn dùng được sau khi chuyển; token đã dùng không sang.
    #[tokio::test]
    async fn token_cu_con_dung_duoc_sau_khi_chuyen() {
        TestDatabase::run(async |test| {
            let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
            let mut reader = ConfigReader::new(&source);
            let config = Config::read(&mut reader);
            let database =
                Database::connect(&reader.finish(config).unwrap().0.database).await.unwrap();
            let mut before = crate::migration::all();
            before.truncate(18);
            Migrator::new(vec![ModuleMigrations::new("auth", before)])
                .unwrap()
                .up(&database)
                .await
                .unwrap();
            let user = Uuid::now_v7();
            let now = Utc::now();
            let insert_user = Query::insert()
                .into_table(Alias::new("auth_user"))
                .columns(
                    ["id", "email", "name", "created_at", "updated_at", "search_text"]
                        .map(Alias::new),
                )
                .values_panic([
                    user.into(),
                    "ada@example.com".into(),
                    "Ada".into(),
                    now.into(),
                    now.into(),
                    "ada".into(),
                ])
                .to_owned();
            database.write().execute(&insert_user).await.unwrap();
            let later = now + chrono::Duration::hours(1);
            let rows: [(&str, Vec<&str>, Vec<sea_orm::Value>); 3] = [
                (
                    "auth_password_reset",
                    vec!["id", "user_id", "token_hash", "expires_at", "used_at", "created_at"],
                    vec![
                        Uuid::now_v7().into(),
                        user.into(),
                        sha256("reset-live").into(),
                        later.into(),
                        Option::<DateTime<Utc>>::None.into(),
                        now.into(),
                    ],
                ),
                (
                    "auth_password_reset",
                    vec!["id", "user_id", "token_hash", "expires_at", "used_at", "created_at"],
                    vec![
                        Uuid::now_v7().into(),
                        user.into(),
                        sha256("reset-used").into(),
                        later.into(),
                        Some(now).into(),
                        now.into(),
                    ],
                ),
                (
                    "auth_email_verification",
                    vec![
                        "id",
                        "user_id",
                        "email",
                        "token_hash",
                        "expires_at",
                        "used_at",
                        "created_at",
                    ],
                    vec![
                        Uuid::now_v7().into(),
                        user.into(),
                        "ada@example.com".into(),
                        sha256("verify-live").into(),
                        later.into(),
                        Option::<DateTime<Utc>>::None.into(),
                        now.into(),
                    ],
                ),
            ];
            for (table, columns, values) in rows {
                let insert = Query::insert()
                    .into_table(Alias::new(table))
                    .columns(columns.into_iter().map(Alias::new))
                    .values_panic(values.into_iter().map(Into::into))
                    .to_owned();
                database.write().execute(&insert).await.unwrap();
            }

            Migrator::new(vec![ModuleMigrations::new("auth", crate::migration::all())])
                .unwrap()
                .up(&database)
                .await
                .unwrap();

            let tokens = OneTimeTokens::new("auth");
            let reset = tokens
                .consume(database.write(), "password_reset", "reset-live")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(reset.subject, user.to_string());
            assert!(
                tokens
                    .consume(database.write(), "password_reset", "reset-used")
                    .await
                    .unwrap()
                    .is_none()
            );
            let verify = tokens
                .consume(database.write(), "email_verification", "verify-live")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(verify.data.as_deref(), Some("ada@example.com"));
        })
        .await
        .unwrap();
    }
}
