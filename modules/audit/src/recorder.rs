//! Ghi audit log: nhận MỌI event của mọi module và thêm một dòng cho mỗi event.
//!
//! Event mang `{"before": …, "after": …}` thì hai phần đó vào cột `before` / `after`;
//! phần còn lại (nếu có) vào `details`. Event bị giao lại (giao ít nhất một lần) thì
//! dòng đã có — bỏ qua, không lỗi. Mỗi dòng nối vào chuỗi băm (`crate::chain`, D-183).

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::privacy::deleted_user;
use chrono::{SubsecRound, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, Set};
use serde_json::Value;

use crate::chain::{self, Link};
use crate::entity;

/// Ghi audit log. Không giữ database: dòng ghi vào database của khách mang event (bước 33).
pub(crate) struct Recorder;

#[async_trait::async_trait]
impl EventHandler for Recorder {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let (before, after, details) = split(&redacted(event));
        let payload_hash =
            chain::payload_hash(before.as_deref(), after.as_deref(), details.as_deref());
        // Lưu và băm ở độ chính xác micro giây — độ chính xác của cả hai database.
        let tenant = event.tenant.id().as_uuid();
        let row = entity::Model {
            id: event.id,
            tenant_id: tenant,
            module: event.module.to_owned(),
            action: event.name.clone(),
            actor_user_id: event.actor.user_id,
            actor_api_key_id: event.actor.api_key_id,
            actor_kind: Some(event.actor.kind().to_owned()),
            subject: event.subject.clone(),
            before,
            after,
            details,
            occurred_at: event.occurred_at.trunc_subsecs(6),
            recorded_at: Utc::now().trunc_subsecs(6),
            sequence: None,
            payload_hash: Some(payload_hash),
            chain_hash: None,
        };
        // Nối vào chuỗi: khoá đầu chuỗi, thêm dòng, dời đầu chuỗi — một transaction. Các dòng
        // nối đuôi nhau; deadlock / bận thì chạy lại cả transaction.
        let (row, database) = (&row, event.tenant.database());
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let head = chain::lock_head(&transaction, tenant).await?;
                // Kiểm SAU khi khoá: hai lần giao cùng một event không cùng qua được.
                if entity::Entity::find_by_id(row.id)
                    .filter(entity::Column::TenantId.eq(tenant))
                    .one(&transaction)
                    .await?
                    .is_some()
                {
                    // Đã ghi ở lần giao trước: đúng như mong muốn.
                    return Ok(());
                }
                let sequence = head.sequence + 1;
                let payload_hash = row.payload_hash.as_deref().unwrap_or_default();
                let hash = chain::chain_hash(&head.hash, &Link::of(row, sequence, payload_hash));
                let mut active = row.clone().into_active_model();
                active.sequence = Set(Some(sequence));
                active.chain_hash = Set(Some(hash.clone()));
                active.insert(&transaction).await?;
                chain::advance(&transaction, tenant, sequence, &hash).await?;
                transaction.commit().await?;
                Ok(())
            })
            .await?;
        if deleted_user(event).is_some() {
            self.forget(event).await?;
        }
        Ok(())
    }
}

impl Recorder {
    /// Tài khoản bị xoá: bỏ phần dữ liệu (`before`, `after`, `details` — có email, tên) của
    /// mọi dòng cùng `subject` với event xoá (các event về chính tài khoản đó), chỉ giữ ai
    /// làm gì lúc nào (D-080, D-149). Chạy lại vô hại. `payload_hash` giữ nguyên nên chuỗi băm
    /// vẫn kiểm được (D-183).
    async fn forget(&self, event: &Delivered) -> Result<(), Error> {
        let Some(subject) = &event.subject else {
            return Ok(());
        };
        let cleared = entity::Entity::update_many()
            .col_expr(entity::Column::Before, Expr::value(None::<String>))
            .col_expr(entity::Column::After, Expr::value(None::<String>))
            .col_expr(entity::Column::Details, Expr::value(None::<String>))
            .filter(entity::Column::TenantId.eq(event.tenant.id().as_uuid()))
            .filter(entity::Column::Subject.eq(subject.as_str()))
            .filter(entity::Column::Id.ne(event.id))
            .exec(event.tenant.database().write())
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
    use autoxide_kernel::database::Database;
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
    async fn audit_khong_giu_email_cua_mail_bi_tu_choi() {
        TestDatabase::run(async |test| {
            let database = database(test).await;
            let event = Delivered::new(
                autoxide_kernel::tenancy::DatabaseRegistry::single(database.clone())
                    .default_tenant(),
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
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn moi_event_mot_dong_ke_ca_khi_giao_lai() {
        TestDatabase::run(async |test| {
            let database = database(test).await;
            let recorder = Recorder;
            let user = Uuid::now_v7();
            let event = Delivered::new(
                autoxide_kernel::tenancy::DatabaseRegistry::single(database.clone())
                    .default_tenant(),
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
            let recorder = Recorder;
            let user = Uuid::now_v7();
            let subject = Some(format!("auth.user:{user}"));
            let happened = |name: &str, subject: Option<String>, payload| {
                Delivered::new(
                    autoxide_kernel::tenancy::DatabaseRegistry::single(database.clone())
                        .default_tenant(),
                    "auth",
                    name.to_owned(),
                    payload,
                )
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
