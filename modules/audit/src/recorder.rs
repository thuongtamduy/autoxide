//! Ghi audit log: nhận MỌI event của mọi module và thêm một dòng cho mỗi event.
//!
//! Event mang `{"before": …, "after": …}` thì hai phần đó vào cột `before` / `after`;
//! phần còn lại (nếu có) vào `details`. Event bị giao lại (giao ít nhất một lần) thì
//! dòng đã có — bỏ qua, không lỗi.

use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use chrono::{SubsecRound, Utc};
use sea_orm::{ActiveModelTrait, Set, SqlErr};
use serde_json::Value;

use crate::entity;

pub(crate) struct Recorder {
    database: Database,
}

impl Recorder {
    pub(crate) fn new(database: Database) -> Self {
        Recorder { database }
    }
}

#[async_trait::async_trait]
impl EventHandler for Recorder {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let (before, after, details) = split(&event.payload);
        let inserted = entity::ActiveModel {
            id: Set(event.id),
            module: Set(event.module.to_owned()),
            action: Set(event.name.clone()),
            actor_user_id: Set(event.actor.user_id),
            actor_api_key_id: Set(event.actor.api_key_id),
            subject: Set(event.subject.clone()),
            before: Set(before),
            after: Set(after),
            details: Set(details),
            occurred_at: Set(event.occurred_at),
            recorded_at: Set(Utc::now().trunc_subsecs(6)),
        }
        .insert(self.database.write())
        .await;
        match inserted {
            Ok(_) => Ok(()),
            // Đã ghi ở lần giao trước: đúng như mong muốn.
            Err(error) if matches!(error.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))) => {
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }
}

/// Tách payload thành (trước, sau, phần còn lại), mỗi phần dạng JSON chữ.
fn split(payload: &Value) -> (Option<String>, Option<String>, Option<String>) {
    let Value::Object(fields) = payload else {
        return (None, None, Some(payload.to_string()));
    };
    let mut rest = fields.clone();
    let before = rest.remove("before").map(|value| value.to_string());
    let after = rest.remove("after").map(|value| value.to_string());
    let details = (!rest.is_empty()).then(|| Value::Object(rest).to_string());
    (before, after, details)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tach_truoc_sau_va_phan_con_lai() {
        let (before, after, details) =
            split(&json!({"before": {"name": "a"}, "after": {"name": "b"}, "reason": "typo"}));

        assert_eq!(before.as_deref(), Some(r#"{"name":"a"}"#));
        assert_eq!(after.as_deref(), Some(r#"{"name":"b"}"#));
        assert_eq!(details.as_deref(), Some(r#"{"reason":"typo"}"#));
        assert_eq!(split(&json!({})), (None, None, None));
        assert_eq!(split(&json!([1, 2])), (None, None, Some("[1,2]".to_owned())));
    }
}

#[cfg(test)]
mod database_tests {
    use super::*;
    use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
    use autoxide_kernel::event::Actor;
    use autoxide_kernel::migration::{Migrator, ModuleMigrations};
    use autoxide_testkit::TestDatabase;
    use sea_orm::EntityTrait;
    use serde_json::json;
    use uuid::Uuid;

    async fn database(test: &TestDatabase) -> Database {
        let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
        let mut reader = ConfigReader::new(&source);
        let config = Config::read(&mut reader);
        let (config, _) = reader.finish(config).unwrap();
        let database = Database::connect(&config.database).await.unwrap();
        Migrator::new(vec![ModuleMigrations::new("audit", crate::migration::all())])
            .unwrap()
            .up(&database)
            .await
            .unwrap();
        database
    }

    #[tokio::test]
    async fn moi_event_mot_dong_ke_ca_khi_giao_lai() {
        TestDatabase::run(async |test| {
            let database = database(test).await;
            let recorder = Recorder::new(database.clone());
            let user = Uuid::now_v7();
            let event = Delivered {
                id: Uuid::now_v7(),
                module: "notes",
                name: "notes.note_renamed".to_owned(),
                actor: Actor::user(user),
                subject: Some("notes.note:1".to_owned()),
                payload: json!({"before": {"title": "a"}, "after": {"title": "b"}}),
                occurred_at: Utc::now().trunc_subsecs(6),
            };

            recorder.handle(&event).await.unwrap();
            recorder.handle(&event).await.unwrap();

            let rows = entity::Entity::find().all(database.read()).await.unwrap();
            assert_eq!(rows.len(), 1);
            let row = &rows[0];
            assert_eq!(row.id, event.id);
            assert_eq!((row.module.as_str(), row.action.as_str()), ("notes", "notes.note_renamed"));
            assert_eq!(row.actor_user_id, Some(user));
            assert_eq!(row.subject.as_deref(), Some("notes.note:1"));
            assert_eq!(row.before.as_deref(), Some(r#"{"title":"a"}"#));
            assert_eq!(row.after.as_deref(), Some(r#"{"title":"b"}"#));
            assert_eq!(row.occurred_at, event.occurred_at);
        })
        .await
        .unwrap();
    }
}
