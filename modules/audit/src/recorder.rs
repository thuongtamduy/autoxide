//! Ghi audit log: nhận MỌI event của mọi module và thêm một dòng cho mỗi event.
//!
//! Event mang `{"before": …, "after": …}` thì hai phần đó vào cột `before` / `after`;
//! phần còn lại (nếu có) vào `details`. Event bị giao lại (giao ít nhất một lần) thì
//! dòng đã có — bỏ qua, không lỗi.

use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::privacy::deleted_user;
use chrono::{SubsecRound, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, SqlErr};
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
        let (before, after, details) = split(&redacted(event));
        let inserted = entity::ActiveModel {
            id: Set(event.id),
            module: Set(event.module.to_owned()),
            action: Set(event.name.clone()),
            actor_user_id: Set(event.actor.user_id),
            actor_api_key_id: Set(event.actor.api_key_id),
            actor_kind: Set(Some(event.actor.kind().to_owned())),
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
            Ok(_) => {}
            // Đã ghi ở lần giao trước: đúng như mong muốn.
            Err(error) if matches!(error.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))) => {
            }
            Err(error) => return Err(error.into()),
        }
        if deleted_user(event).is_some() {
            self.forget(event).await?;
        }
        Ok(())
    }
}

impl Recorder {
    /// Tài khoản bị xoá: bỏ phần dữ liệu (`before`, `after`, `details` — có email, tên) của
    /// mọi dòng cùng `subject` với event xoá (các event về chính tài khoản đó), chỉ giữ ai
    /// làm gì lúc nào (D-080, D-149). Chạy lại vô hại.
    async fn forget(&self, event: &Delivered) -> Result<(), Error> {
        let Some(subject) = &event.subject else {
            return Ok(());
        };
        let cleared = entity::Entity::update_many()
            .col_expr(entity::Column::Before, Expr::value(None::<String>))
            .col_expr(entity::Column::After, Expr::value(None::<String>))
            .col_expr(entity::Column::Details, Expr::value(None::<String>))
            .filter(entity::Column::Subject.eq(subject.as_str()))
            .filter(entity::Column::Id.ne(event.id))
            .exec(self.database.write())
            .await?;
        tracing::info!(rows = cleared.rows_affected, "audit entries of a deleted user cleared");
        Ok(())
    }
}

/// Event mang địa chỉ email của người nhận mà không gắn với tài khoản nào (nên không xoá được
/// theo `subject` khi tài khoản bị xoá): audit chỉ giữ bản băm của địa chỉ (D-150). Bên nhận
/// khác (bản tin, webhook) vẫn nhận địa chỉ đầy đủ.
const EMAIL_FIELDS: [(&str, &str); 1] = [("mail.delivery_failed", "to")];

/// Payload sẽ lưu: các trường email ở [`EMAIL_FIELDS`] thay bằng `sha256:<hex>` của địa chỉ đã
/// chuẩn hoá (chữ thường, bỏ khoảng trắng hai đầu).
fn redacted(event: &Delivered) -> Value {
    let mut payload = event.payload.clone();
    for (name, field) in EMAIL_FIELDS {
        if event.name == name
            && let Some(value) = payload.get_mut(field)
            && let Some(email) = value.as_str()
        {
            let hash = autoxide_kernel::secret::sha256(&email.trim().to_lowercase());
            *value = Value::String(format!("sha256:{hash}"));
        }
    }
    payload
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
    fn audit_khong_giu_email_cua_mail_bi_tu_choi() {
        let event = Delivered::new(
            "mail",
            "mail.delivery_failed".to_owned(),
            json!({"to": " Ada@Example.com", "dedupe_key": null, "code": "550"}),
        );
        let stored = redacted(&event);
        let expected = autoxide_kernel::secret::sha256("ada@example.com");
        assert_eq!(stored["to"], format!("sha256:{expected}"));
        assert_eq!(stored["code"], "550");
        let mut other = event.clone();
        other.name = "mail.something_else".to_owned();
        assert_eq!(redacted(&other)["to"], " Ada@Example.com");
    }

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
            let event = Delivered::new(
                "notes",
                "notes.note_renamed".to_owned(),
                json!({"before": {"title": "a"}, "after": {"title": "b"}}),
            )
            .actor(Actor::user(user))
            .subject(Some("notes.note:1".to_owned()))
            .occurred_at(Utc::now().trunc_subsecs(6));

            recorder.handle(&event).await.unwrap();
            recorder.handle(&event).await.unwrap();

            let rows = entity::Entity::find().all(database.read()).await.unwrap();
            assert_eq!(rows.len(), 1);
            let row = &rows[0];
            assert_eq!(row.id, event.id);
            assert_eq!((row.module.as_str(), row.action.as_str()), ("notes", "notes.note_renamed"));
            assert_eq!(row.actor_user_id, Some(user));
            assert_eq!(row.actor_kind.as_deref(), Some("user"));
            assert_eq!(row.subject.as_deref(), Some("notes.note:1"));
            assert_eq!(row.before.as_deref(), Some(r#"{"title":"a"}"#));
            assert_eq!(row.after.as_deref(), Some(r#"{"title":"b"}"#));
            assert_eq!(row.occurred_at, event.occurred_at);
        })
        .await
        .unwrap();
    }

    /// Tài khoản bị xoá: các dòng về tài khoản đó mất email, tên; dòng khác giữ nguyên.
    #[tokio::test]
    async fn xoa_tai_khoan_thi_bo_du_lieu_cua_cac_dong_ve_no() {
        TestDatabase::run(async |test| {
            let database = database(test).await;
            let recorder = Recorder::new(database.clone());
            let user = Uuid::now_v7();
            let subject = Some(format!("auth.user:{user}"));
            let happened = |name: &str, subject: Option<String>, payload| {
                Delivered::new("auth", name.to_owned(), payload)
                    .actor(Actor::user(user))
                    .subject(subject)
                    .occurred_at(Utc::now().trunc_subsecs(6))
            };
            let created = happened(
                "auth.user_created",
                subject.clone(),
                json!({"after": {"email": "ada@example.com", "name": "Ada"}}),
            );
            let other = happened(
                "auth.user_updated",
                Some("auth.user:other".to_owned()),
                json!({"after": {"name": "Bob"}}),
            );
            let deleted = happened(
                autoxide_kernel::privacy::USER_DELETED,
                subject.clone(),
                json!({"before": {"id": user}}),
            );
            for event in [&created, &other, &deleted, &deleted] {
                recorder.handle(event).await.unwrap();
            }

            let row = |id| entity::Entity::find_by_id(id).one(database.read());
            let kept = row(created.id).await.unwrap().unwrap();
            assert_eq!((kept.before, kept.after, kept.details), (None, None, None));
            assert_eq!(kept.action, "auth.user_created");
            assert_eq!(kept.actor_user_id, Some(user));
            assert!(row(other.id).await.unwrap().unwrap().after.unwrap().contains("Bob"));
            assert!(row(deleted.id).await.unwrap().unwrap().before.is_some());
        })
        .await
        .unwrap();
    }
}
