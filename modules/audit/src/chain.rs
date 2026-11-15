//! Chuỗi băm của audit log (D-183): sửa, xoá hay chèn một dòng cũ đều bị phát hiện.
//!
//! Mỗi dòng mang:
//!
//! - `sequence`: số thứ tự trong chuỗi, 1, 2, 3… không hở;
//! - `payload_hash`: mã băm của phần dữ liệu (`before`, `after`, `details`) — phần có thể chứa
//!   dữ liệu cá nhân;
//! - `chain_hash`: mã băm của `chain_hash` dòng trước, các cột còn lại của dòng (ai, làm gì, với
//!   bản ghi nào, lúc nào) và `payload_hash`.
//!
//! Bảng `audit_chain` giữ đầu chuỗi (số thứ tự và mã băm của dòng cuối): xoá dòng ở cuối cũng bị
//! phát hiện. **Mỗi khách một chuỗi** (bước 33, D-276): số thứ tự, đầu chuỗi và việc kiểm đều theo
//! khách; dữ liệu của khách này không chen vào chuỗi của khách khác. Ghi một dòng mới khoá đầu chuỗi tới hết transaction, nên các dòng nối đuôi nhau
//! trên cả SQLite và Postgres.
//!
//! **Xoá dữ liệu cá nhân vẫn giữ được chuỗi:** khi tài khoản bị xoá, audit bỏ `before` / `after` /
//! `details` của các dòng về tài khoản đó nhưng giữ `payload_hash` (chuỗi chỉ băm `payload_hash`,
//! không băm dữ liệu). Lúc kiểm, dòng đã bỏ dữ liệu chỉ được chấp nhận khi chuỗi có event xoá tài
//! khoản cùng `subject` — bỏ dữ liệu của dòng khác vẫn bị báo.
//!
//! Cách băm dưới đây là định dạng lưu trữ: đổi nó làm mọi chuỗi đã ghi thành "bị sửa". Cần đổi
//! thì thêm phiên bản mới (tiền tố `audit-v2`), không sửa bản `v1`.

use autoxide_kernel::database::Database;
use autoxide_kernel::doctor::{Finding, Probe};
use autoxide_kernel::error::Error;
use autoxide_kernel::privacy::USER_DELETED;
use autoxide_kernel::rows;
use chrono::{DateTime, SecondsFormat, Utc};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use std::collections::HashSet;
use uuid::Uuid;

use crate::entity::{self, head};

/// Khoá chính của dòng duy nhất trong `audit_chain`.
pub(crate) const HEAD: &str = "entries";

/// Các cột của một dòng mà chuỗi băm bảo vệ.
pub(crate) struct Link<'a> {
    pub sequence: i64,
    pub id: Uuid,
    pub module: &'a str,
    pub action: &'a str,
    pub actor_user_id: Option<Uuid>,
    pub actor_api_key_id: Option<Uuid>,
    pub actor_kind: Option<&'a str>,
    pub subject: Option<&'a str>,
    pub occurred_at: DateTime<Utc>,
    pub recorded_at: DateTime<Utc>,
    pub payload_hash: &'a str,
}

impl<'a> Link<'a> {
    pub(crate) fn of(row: &'a entity::Model, sequence: i64, payload_hash: &'a str) -> Self {
        Link {
            sequence,
            id: row.id,
            module: &row.module,
            action: &row.action,
            actor_user_id: row.actor_user_id,
            actor_api_key_id: row.actor_api_key_id,
            actor_kind: row.actor_kind.as_deref(),
            subject: row.subject.as_deref(),
            occurred_at: row.occurred_at,
            recorded_at: row.recorded_at,
            payload_hash,
        }
    }
}

/// Mã băm của phần dữ liệu một dòng.
pub(crate) fn payload_hash(
    before: Option<&str>,
    after: Option<&str>,
    details: Option<&str>,
) -> String {
    let mut text = String::from("audit-payload-v1");
    for part in [before, after, details] {
        push(&mut text, part);
    }
    autoxide_kernel::secret::sha256(&text)
}

/// Mã băm của `link` nối sau dòng có mã băm `previous` (`""` cho dòng đầu tiên).
pub(crate) fn chain_hash(previous: &str, link: &Link<'_>) -> String {
    let mut text = String::from("audit-v1");
    let sequence = link.sequence.to_string();
    let (id, user, key) = (
        link.id.to_string(),
        link.actor_user_id.map(|id| id.to_string()),
        link.actor_api_key_id.map(|id| id.to_string()),
    );
    // Thời gian băm ở độ chính xác micro giây: đúng độ chính xác cả hai database lưu, nên đọc
    // lại từ database nào cũng cho cùng một chuỗi.
    let occurred = link.occurred_at.to_rfc3339_opts(SecondsFormat::Micros, true);
    let recorded = link.recorded_at.to_rfc3339_opts(SecondsFormat::Micros, true);
    for part in [
        Some(previous),
        Some(sequence.as_str()),
        Some(id.as_str()),
        Some(link.module),
        Some(link.action),
        user.as_deref(),
        key.as_deref(),
        link.actor_kind,
        link.subject,
        Some(occurred.as_str()),
        Some(recorded.as_str()),
        Some(link.payload_hash),
    ] {
        push(&mut text, part);
    }
    autoxide_kernel::secret::sha256(&text)
}

/// Nối một phần vào chuỗi đem băm: `|<độ dài>:<giá trị>` hoặc `|-` khi không có. Ghi độ dài nên
/// không thể dời ranh giới giữa hai phần để ra cùng một chuỗi.
fn push(text: &mut String, part: Option<&str>) {
    match part {
        Some(value) => {
            text.push('|');
            text.push_str(&value.len().to_string());
            text.push(':');
            text.push_str(value);
        }
        None => text.push_str("|-"),
    }
}

/// Khoá đầu chuỗi của khách `tenant` tới hết `transaction` và trả (số thứ tự, mã băm) của dòng
/// cuối. Đầu chuỗi gieo khi tạo khách ([`seed_head`]); khách mặc định có từ migration `m0004`.
pub(crate) async fn lock_head(
    transaction: &DatabaseTransaction,
    tenant: Uuid,
) -> Result<head::Model, Error> {
    rows::lock_one(transaction, head::Entity::find_by_id((tenant, HEAD.to_owned())))
        .await?
        .ok_or_else(|| Error::internal("audit_chain has no head row; run `autoxide migrate up`"))
}

/// Gieo đầu chuỗi trống (số thứ tự 0) cho khách `tenant`. Chạy lại vô hại: đã có thì giữ nguyên.
pub(crate) async fn seed_head<C: ConnectionTrait>(
    connection: &C,
    tenant: Uuid,
) -> Result<(), Error> {
    let row = head::ActiveModel {
        tenant_id: Set(tenant),
        name: Set(HEAD.to_owned()),
        sequence: Set(0),
        hash: Set(String::new()),
    };
    head::Entity::insert(row)
        .on_conflict(
            sea_orm::sea_query::OnConflict::columns([head::Column::TenantId, head::Column::Name])
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(connection)
        .await?;
    Ok(())
}

/// Dời đầu chuỗi của khách `tenant` tới dòng vừa thêm.
pub(crate) async fn advance<C: ConnectionTrait>(
    connection: &C,
    tenant: Uuid,
    sequence: i64,
    hash: &str,
) -> Result<(), Error> {
    head::Entity::update(head::ActiveModel {
        tenant_id: Set(tenant),
        name: Set(HEAD.to_owned()),
        sequence: Set(sequence),
        hash: Set(hash.to_owned()),
    })
    .exec(connection)
    .await?;
    Ok(())
}

/// Thời gian tối đa cho bước kiểm chuỗi của `autoxide doctor`: đọc lại cả audit log.
pub(crate) const VERIFY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// Số dòng đọc mỗi lần khi kiểm chuỗi.
const BATCH: u64 = 1000;

/// Kết quả kiểm chuỗi.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Chuỗi còn nguyên; số dòng đã kiểm.
    Intact(u64),
    /// Dòng đầu tiên không khớp (số thứ tự mong đợi, ID nếu có dòng, lý do).
    Broken { sequence: i64, id: Option<Uuid>, reason: &'static str },
}

/// Kiểm cả chuỗi của khách `tenant` từ dòng đầu tới đầu chuỗi, dừng ở dòng hỏng đầu tiên.
pub(crate) async fn verify<C: ConnectionTrait>(
    connection: &C,
    tenant: Uuid,
) -> Result<Verdict, Error> {
    // Subject đã có event xoá tài khoản: dòng về chúng được phép đã bỏ dữ liệu.
    let forgotten: HashSet<String> = entity::Entity::find()
        .select_only()
        .column(entity::Column::Subject)
        .filter(entity::Column::TenantId.eq(tenant))
        .filter(entity::Column::Action.eq(USER_DELETED))
        .filter(entity::Column::Subject.is_not_null())
        .into_tuple::<Option<String>>()
        .all(connection)
        .await?
        .into_iter()
        .flatten()
        .collect();
    let empty = payload_hash(None, None, None);
    let mut previous = String::new();
    let mut expected: i64 = 1;
    loop {
        let rows = entity::Entity::find()
            .filter(entity::Column::TenantId.eq(tenant))
            .filter(entity::Column::Sequence.gte(expected))
            .order_by_asc(entity::Column::Sequence)
            .limit(BATCH)
            .all(connection)
            .await?;
        let count = rows.len();
        for row in &rows {
            let broken = |reason| Verdict::Broken { sequence: expected, id: Some(row.id), reason };
            let (Some(sequence), Some(stored_payload), Some(stored_chain)) =
                (row.sequence, row.payload_hash.as_deref(), row.chain_hash.as_deref())
            else {
                return Ok(broken("the entry has no place in the chain"));
            };
            if sequence != expected {
                return Ok(Verdict::Broken {
                    sequence: expected,
                    id: None,
                    reason: "the entry is missing (deleted)",
                });
            }
            let data = (row.before.as_deref(), row.after.as_deref(), row.details.as_deref());
            let cleared = data == (None, None, None);
            if cleared && stored_payload != empty {
                let allowed =
                    row.subject.as_ref().is_some_and(|subject| forgotten.contains(subject));
                if !allowed {
                    return Ok(broken("its data was removed without an account deletion"));
                }
            } else if payload_hash(data.0, data.1, data.2) != stored_payload {
                return Ok(broken("its data was changed"));
            }
            let hash = chain_hash(&previous, &Link::of(row, sequence, stored_payload));
            if hash != stored_chain {
                return Ok(broken("the entry or the one before it was changed"));
            }
            previous = hash;
            expected += 1;
        }
        if (count as u64) < BATCH {
            break;
        }
    }
    let unchained = entity::Entity::find()
        .filter(entity::Column::TenantId.eq(tenant))
        .filter(entity::Column::Sequence.is_null())
        .one(connection)
        .await?;
    if let Some(row) = unchained {
        return Ok(Verdict::Broken {
            sequence: expected,
            id: Some(row.id),
            reason: "the entry was inserted outside the chain",
        });
    }
    let head = head::Entity::find_by_id((tenant, HEAD.to_owned()))
        .one(connection)
        .await?
        .ok_or_else(|| Error::internal("audit_chain has no head row"))?;
    if head.sequence != expected - 1 || head.hash != previous {
        return Ok(Verdict::Broken {
            sequence: expected,
            id: None,
            reason: "entries at the end of the log are missing (deleted)",
        });
    }
    Ok(Verdict::Intact(u64::try_from(expected - 1).unwrap_or_default()))
}

/// Bước kiểm `audit.chain` của `autoxide doctor`: kiểm chuỗi của từng khách trên từng database.
pub(crate) struct ChainProbe {
    pub databases: autoxide_kernel::tenancy::DatabaseRegistry,
}

impl ChainProbe {
    /// Kiểm mọi chuỗi trên `database`: (số dòng đã kiểm, chỗ hỏng đầu tiên nếu có).
    async fn check(&self, database: &Database) -> Result<(u64, Option<String>), Error> {
        // Một bức ảnh nhất quán: dòng mới ghi giữa chừng không làm lệch đầu chuỗi với các dòng.
        let snapshot = database.begin_snapshot().await?;
        let tenants: Vec<Uuid> = head::Entity::find()
            .select_only()
            .column(head::Column::TenantId)
            .into_tuple()
            .all(&snapshot)
            .await?;
        let mut total = 0;
        for tenant in tenants {
            match verify(&snapshot, tenant).await? {
                Verdict::Intact(count) => total += count,
                Verdict::Broken { sequence, id, reason } => {
                    let at = id.map_or_else(String::new, |id| format!(" (id {id})"));
                    return Ok((
                        total,
                        Some(format!("tenant {tenant}: entry #{sequence}{at} — {reason}")),
                    ));
                }
            }
        }
        Ok((total, None))
    }
}

#[async_trait::async_trait]
impl Probe for ChainProbe {
    async fn probe(&self) -> Finding {
        let databases = match self.databases.databases().await {
            Ok(databases) => databases,
            Err(error) => return Finding::failed(format!("cannot list the databases: {error}")),
        };
        let mut total = 0;
        for (_, name, database) in databases {
            match self.check(&database).await {
                Ok((count, None)) => total += count,
                Ok((_, Some(broken))) => {
                    return Finding::failed(format!(
                        "The audit log was tampered with on placement `{name}`: {broken} does \
                         not match the hash chain."
                    ))
                    .with_hint(
                        "Restore the audit_entry table from a backup taken before that entry, and \
                         find out who had write access to the database.",
                    );
                }
                Err(error) => {
                    return Finding::failed(format!(
                        "cannot verify the audit log on placement `{name}`: {error}"
                    ));
                }
            }
        }
        Finding::ok(format!("The audit log hash chains are intact ({total} entries)."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recorder::Recorder;
    use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
    use autoxide_kernel::doctor::Status;
    use autoxide_kernel::event::{Actor, Delivered, EventHandler};
    use autoxide_kernel::migration::{Migrator, ModuleMigrations};
    use autoxide_testkit::TestDatabase;
    use chrono::SubsecRound;
    use sea_orm::sea_query::{Expr, ExprTrait as _, Query};
    use serde_json::json;
    use std::sync::Arc;

    async fn database(test: &TestDatabase) -> Database {
        let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
        let mut reader = ConfigReader::new(&source);
        let config = Config::read(&mut reader);
        let (config, _) = reader.finish(config).unwrap();
        let database = Database::connect(&config.database).await.unwrap();
        migrate(&database, crate::migration::all()).await;
        database
    }

    async fn migrate(
        database: &Database,
        migrations: Vec<Box<dyn autoxide_kernel::migration::prelude::MigrationTrait>>,
    ) {
        Migrator::new(vec![ModuleMigrations::new("audit", migrations)])
            .unwrap()
            .up(database)
            .await
            .unwrap();
    }

    fn event(
        database: &Database,
        name: &str,
        subject: &str,
        payload: serde_json::Value,
    ) -> Delivered {
        Delivered::new(
            autoxide_kernel::tenancy::DatabaseRegistry::single(database.clone()).default_tenant(),
            "auth",
            name.to_owned(),
            payload,
        )
        .actor(Actor::user(Uuid::now_v7()))
        .subject(Some(subject.to_owned()))
        .occurred_at(Utc::now().trunc_subsecs(6))
    }

    /// Ghi `count` event; trả ID theo thứ tự ghi.
    async fn record(database: &Database, count: usize) -> Vec<Uuid> {
        let recorder = Recorder;
        let mut ids = Vec::new();
        for index in 0..count {
            let event = event(
                database,
                "auth.user_updated",
                &format!("auth.user:{index}"),
                json!({"before": {"name": "a"}, "after": {"name": format!("b{index}")}}),
            );
            recorder.handle(&event).await.unwrap();
            ids.push(event.id);
        }
        ids
    }

    async fn set(database: &Database, id: Uuid, column: &'static str, value: Option<&str>) {
        let update = Query::update()
            .table("audit_entry")
            .value(column, value.map(str::to_owned))
            .and_where(Expr::col("id").eq(id))
            .to_owned();
        database.write().execute(&update).await.unwrap();
    }

    async fn verdict(database: &Database) -> Verdict {
        verify(database.read(), autoxide_kernel::tenancy::DEFAULT_TENANT.as_uuid()).await.unwrap()
    }

    #[test]
    fn bam_doi_khi_doi_bat_ky_phan_nao() {
        let at = Utc::now();
        let link = |module| Link {
            sequence: 1,
            id: Uuid::nil(),
            module,
            action: "a",
            actor_user_id: None,
            actor_api_key_id: None,
            actor_kind: None,
            subject: None,
            occurred_at: at,
            recorded_at: at,
            payload_hash: "p",
        };
        assert_ne!(chain_hash("", &link("ab")), chain_hash("", &link("a")));
        assert_ne!(chain_hash("x", &link("a")), chain_hash("", &link("a")));
        // Ranh giới giữa các phần không dời được: ("ab", None) khác ("a", "b").
        assert_ne!(payload_hash(Some("ab"), None, None), payload_hash(Some("a"), Some("b"), None));
        assert_ne!(payload_hash(None, None, None), payload_hash(Some(""), None, None));
    }

    #[tokio::test]
    async fn chuoi_nguyen_ven_ke_ca_khi_giao_lai() {
        TestDatabase::run(async |test| {
            let database = database(test).await;
            assert_eq!(verdict(&database).await, Verdict::Intact(0));
            record(&database, 3).await;
            let recorder = Recorder;
            let again =
                event(&database, "notes.note_created", "notes.note:1", json!({"after": {}}));
            recorder.handle(&again).await.unwrap();
            recorder.handle(&again).await.unwrap();
            assert_eq!(verdict(&database).await, Verdict::Intact(4));
            let finding = ChainProbe {
                databases: autoxide_kernel::tenancy::DatabaseRegistry::single(database.clone()),
            }
            .probe()
            .await;
            assert_eq!(finding.status, Status::Ok, "{finding:?}");
        })
        .await
        .unwrap();
    }

    /// Sửa dữ liệu, sửa cột khác, xoá dòng giữa, xoá dòng cuối, chèn dòng ngoài chuỗi: đều bị báo
    /// ở đúng dòng đầu tiên hỏng.
    #[tokio::test]
    async fn bao_dong_dau_tien_bi_sua() {
        TestDatabase::run(async |test| {
            let database = database(test).await;
            let ids = record(&database, 5).await;

            set(&database, ids[2], "after", Some(r#"{"name":"forged"}"#)).await;
            let broken = verdict(&database).await;
            assert_eq!(
                broken,
                Verdict::Broken { sequence: 3, id: Some(ids[2]), reason: "its data was changed" }
            );
            let finding = ChainProbe {
                databases: autoxide_kernel::tenancy::DatabaseRegistry::single(database.clone()),
            }
            .probe()
            .await;
            assert_eq!(finding.status, Status::Failed);
            assert!(finding.message.contains("entry #3"), "{}", finding.message);
            set(&database, ids[2], "after", Some(r#"{"name":"b2"}"#)).await;
            assert_eq!(verdict(&database).await, Verdict::Intact(5));

            set(&database, ids[1], "action", Some("auth.user_created")).await;
            assert!(matches!(verdict(&database).await, Verdict::Broken { sequence: 2, .. }));
            set(&database, ids[1], "action", Some("auth.user_updated")).await;

            // Bỏ dữ liệu mà không có event xoá tài khoản.
            for column in ["before", "after"] {
                set(&database, ids[0], column, None).await;
            }
            assert!(matches!(
                verdict(&database).await,
                Verdict::Broken {
                    sequence: 1,
                    reason: "its data was removed without an account deletion",
                    ..
                }
            ));
            set(&database, ids[0], "before", Some(r#"{"name":"a"}"#)).await;
            set(&database, ids[0], "after", Some(r#"{"name":"b0"}"#)).await;

            let delete = |id: Uuid| {
                Query::delete()
                    .from_table("audit_entry")
                    .and_where(Expr::col("id").eq(id))
                    .to_owned()
            };
            database.write().execute(&delete(ids[4])).await.unwrap();
            assert!(matches!(
                verdict(&database).await,
                Verdict::Broken {
                    sequence: 5,
                    reason: "entries at the end of the log are missing (deleted)",
                    ..
                }
            ));
            database.write().execute(&delete(ids[1])).await.unwrap();
            assert!(matches!(
                verdict(&database).await,
                Verdict::Broken { sequence: 2, id: None, .. }
            ));
        })
        .await
        .unwrap();
    }

    /// Xoá tài khoản bỏ dữ liệu cá nhân của các dòng về nó mà chuỗi vẫn nguyên.
    #[tokio::test]
    async fn xoa_du_lieu_ca_nhan_van_giu_chuoi() {
        TestDatabase::run(async |test| {
            let database = database(test).await;
            let recorder = Recorder;
            let user = Uuid::now_v7();
            let subject = format!("auth.user:{user}");
            let created = event(
                &database,
                "auth.user_created",
                &subject,
                json!({"after": {"email": "ada@example.com"}}),
            );
            let deleted = event(&database, USER_DELETED, &subject, json!({"before": {"id": user}}));
            recorder.handle(&created).await.unwrap();
            record(&database, 1).await;
            recorder.handle(&deleted).await.unwrap();

            let row = entity::Entity::find_by_id(created.id).one(database.read()).await.unwrap();
            assert_eq!(row.unwrap().after, None);
            assert_eq!(verdict(&database).await, Verdict::Intact(3));
        })
        .await
        .unwrap();
    }

    /// Nhiều event ghi cùng lúc: các dòng nối đuôi nhau, chuỗi không hở, không trùng vị trí.
    #[tokio::test(flavor = "multi_thread")]
    async fn ghi_dong_thoi_van_mot_chuoi() {
        TestDatabase::run(async |test| {
            let database = database(test).await;
            let recorder = Arc::new(Recorder);
            let mut tasks = tokio::task::JoinSet::new();
            for index in 0..24 {
                let (recorder, database) = (recorder.clone(), database.clone());
                tasks.spawn(async move {
                    let event = event(
                        &database,
                        "auth.user_updated",
                        &format!("auth.user:{index}"),
                        json!({}),
                    );
                    recorder.handle(&event).await.unwrap();
                    // Giao lại ngay: không thêm dòng.
                    recorder.handle(&event).await.unwrap();
                });
            }
            tasks.join_all().await;
            assert_eq!(verdict(&database).await, Verdict::Intact(24));
        })
        .await
        .unwrap();
    }

    /// Dòng ghi trước khi có chuỗi được migration điền chuỗi, theo thứ tự xảy ra.
    #[tokio::test]
    async fn migration_dien_chuoi_cho_du_lieu_cu() {
        TestDatabase::run(async |test| {
            let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
            let mut reader = ConfigReader::new(&source);
            let config = Config::read(&mut reader);
            let (config, _) = reader.finish(config).unwrap();
            let database = Database::connect(&config.database).await.unwrap();
            let mut migrations = crate::migration::all();
            migrations.truncate(3);
            migrate(&database, migrations).await;
            for index in 0..7 {
                let insert = Query::insert()
                    .into_table("audit_entry")
                    .columns([
                        "id",
                        "module",
                        "action",
                        "subject",
                        "after",
                        "occurred_at",
                        "recorded_at",
                    ])
                    .values([
                        Uuid::now_v7().into(),
                        "notes".into(),
                        "notes.note_created".into(),
                        format!("notes.note:{index}").into(),
                        (index != 3).then(|| r#"{"title":"x"}"#.to_owned()).into(),
                        Utc::now().into(),
                        Utc::now().into(),
                    ])
                    .unwrap()
                    .to_owned();
                database.write().execute(&insert).await.unwrap();
            }
            migrate(&database, crate::migration::all()).await;
            assert_eq!(verdict(&database).await, Verdict::Intact(7));
            record(&database, 2).await;
            assert_eq!(verdict(&database).await, Verdict::Intact(9));
        })
        .await
        .unwrap();
    }
}
