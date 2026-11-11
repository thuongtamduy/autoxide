//! `IdempotencyStore` trên bảng `idempotency_record`.
//!
//! Hai request cùng key đến cùng lúc: index unique `(scope, key)` để đúng một bên chèn
//! được dòng và chạy; bên kia đọc dòng đó và nhận `InProgress` (hoặc kết quả, nếu bên
//! đầu đã xong).

use std::time::Duration;

use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::idempotency::{Begin, IdempotencyStore, StoredResponse};
use autoxide_kernel::job::{JobContext, JobHandler, Schedule};
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, SqlErr};
use uuid::Uuid;

use crate::entity::{self, Column, Entity};

/// Giữ một key chừng này: gửi lại trong khoảng đó nhận lại kết quả cũ.
const KEEP: chrono::Duration = chrono::Duration::hours(24);
/// Loại job dọn key hết hạn (cũng là tên lịch).
pub(crate) const CLEANUP: &str = "idempotency.cleanup";

/// Dọn mỗi giờ.
pub(crate) fn cleanup_schedule() -> Schedule {
    Schedule::new(CLEANUP, CLEANUP, Duration::from_secs(3600))
}

pub(crate) struct Store {
    database: Database,
}

impl Store {
    pub(crate) fn new(database: Database) -> Self {
        Store { database }
    }

    /// Chèn dòng mới cho lần đầu. `None` nếu dòng của key đã có.
    async fn insert(
        &self,
        scope: &str,
        key: &str,
        fingerprint: &str,
        locked_until: DateTime<Utc>,
    ) -> Result<Option<Uuid>, Error> {
        let now = now();
        let id = Uuid::now_v7();
        let inserted = entity::ActiveModel {
            id: Set(id),
            scope: Set(scope.to_owned()),
            key: Set(key.to_owned()),
            fingerprint: Set(fingerprint.to_owned()),
            status: Set(None),
            headers: Set(None),
            body: Set(None),
            locked_until: Set(locked_until),
            expires_at: Set(now + KEEP),
            created_at: Set(now),
        }
        .insert(self.database.write())
        .await;
        match inserted {
            Ok(_) => Ok(Some(id)),
            Err(error) if matches!(error.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))) => {
                Ok(None)
            }
            Err(error) => Err(error.into()),
        }
    }
}

#[async_trait::async_trait]
impl IdempotencyStore for Store {
    async fn begin(
        &self,
        scope: &str,
        key: &str,
        fingerprint: &str,
        locked_until: DateTime<Utc>,
    ) -> Result<Begin, Error> {
        // Vòng lặp chạy lại nhiều nhất một lần: khi dòng cũ đã hết hạn và vừa bị xoá.
        for _ in 0..2 {
            if let Some(id) = self.insert(scope, key, fingerprint, locked_until).await? {
                return Ok(Begin::Started { id });
            }
            let Some(row) = Entity::find()
                .filter(Column::Scope.eq(scope))
                .filter(Column::Key.eq(key))
                .one(self.database.write())
                .await?
            else {
                continue; // vừa bị dọn giữa hai câu lệnh
            };
            let now = now();
            if row.expires_at <= now {
                Entity::delete_by_id(row.id).exec(self.database.write()).await?;
                continue;
            }
            if row.fingerprint != fingerprint {
                return Ok(Begin::Mismatch);
            }
            if let Some(status) = row.status {
                let headers = row
                    .headers
                    .as_deref()
                    .map(serde_json::from_str::<Vec<(String, String)>>)
                    .transpose()
                    .map_err(Error::internal)?
                    .unwrap_or_default();
                return Ok(Begin::Replay(StoredResponse {
                    status: u16::try_from(status).map_err(Error::internal)?,
                    headers,
                    body: row.body.unwrap_or_default(),
                }));
            }
            if row.locked_until > now {
                return Ok(Begin::InProgress);
            }
            // Lần đầu đã quá hạn mà chưa xong (tiến trình chết): nhận lại quyền chạy, có
            // điều kiện để chỉ một request nhận được.
            let taken = Entity::update_many()
                .col_expr(Column::LockedUntil, Expr::value(locked_until))
                .filter(Column::Id.eq(row.id))
                .filter(Column::Status.is_null())
                .filter(Column::LockedUntil.lte(now))
                .exec(self.database.write())
                .await?;
            return Ok(if taken.rows_affected == 1 {
                Begin::Started { id: row.id }
            } else {
                Begin::InProgress
            });
        }
        Ok(Begin::InProgress)
    }

    async fn complete(&self, id: Uuid, response: StoredResponse) -> Result<(), Error> {
        let headers = serde_json::to_string(&response.headers).map_err(Error::internal)?;
        Entity::update_many()
            .col_expr(Column::Status, Expr::value(Some(i32::from(response.status))))
            .col_expr(Column::Headers, Expr::value(Some(headers)))
            .col_expr(Column::Body, Expr::value(Some(response.body)))
            .filter(Column::Id.eq(id))
            .exec(self.database.write())
            .await?;
        Ok(())
    }

    async fn abandon(&self, id: Uuid) -> Result<(), Error> {
        Entity::delete_by_id(id).exec(self.database.write()).await?;
        Ok(())
    }
}

/// Job dọn key đã hết hạn.
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
        let removed = Entity::delete_many()
            .filter(Column::ExpiresAt.lt(now()))
            .exec(self.database.write())
            .await?;
        tracing::info!(removed = removed.rows_affected, "expired idempotency keys removed");
        Ok(())
    }
}

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
    use autoxide_kernel::migration::{Migrator, ModuleMigrations};
    use autoxide_testkit::TestDatabase;
    use axum::Router;
    use axum::body::Body;
    use axum::extract::State;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt as _;
    use utoipa_axum::router::OpenApiRouter;
    use utoipa_axum::routes;

    /// Đếm số lần handler thật sự chạy; `fail` thì trả 500.
    #[derive(Default)]
    struct Counter {
        runs: AtomicUsize,
    }

    /// Create a note.
    #[utoipa::path(post, path = "/", responses((status = 201, description = "Created.")))]
    async fn create(State(counter): State<Arc<Counter>>, body: String) -> (StatusCode, String) {
        let run = counter.runs.fetch_add(1, Ordering::SeqCst) + 1;
        // Đủ lâu để các request đồng thời chắc chắn chồng lên nhau.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        if body.contains("explode") {
            return (StatusCode::INTERNAL_SERVER_ERROR, "boom".to_owned());
        }
        (StatusCode::CREATED, format!("run {run}: {body}"))
    }

    async fn setup(test: &TestDatabase) -> (Router, Arc<Counter>, Database) {
        let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
        let mut reader = ConfigReader::new(&source);
        let config = Config::read(&mut reader);
        let (config, _) = reader.finish(config).unwrap();
        let database = Database::connect(&config.database).await.unwrap();
        Migrator::new(vec![ModuleMigrations::new("idempotency", crate::migration::all())])
            .unwrap()
            .up(&database)
            .await
            .unwrap();
        let counter = Arc::new(Counter::default());
        let routes = OpenApiRouter::new().routes(routes!(create)).with_state(counter.clone());
        let store: Arc<dyn IdempotencyStore> = Arc::new(Store::new(database.clone()));
        let extras =
            autoxide_kernel::http::Extras { idempotency: Some(store), ..Default::default() };
        let router = autoxide_kernel::http::build(
            &config.http,
            OpenApiRouter::new(),
            vec![("notes", routes)],
            extras,
        )
        .router;
        (router, counter, database)
    }

    async fn post(
        app: &Router,
        key: Option<&str>,
        body: &str,
    ) -> (StatusCode, Option<String>, String) {
        let mut request = Request::builder().method("POST").uri("/api/v1/notes");
        if let Some(key) = key {
            request = request.header("idempotency-key", key);
        }
        let request = request.body(Body::from(body.to_owned())).unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let replayed = response
            .headers()
            .get("idempotent-replayed")
            .map(|value| value.to_str().unwrap().to_owned());
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, replayed, String::from_utf8(bytes.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn gui_lai_cung_key_nhan_lai_ket_qua_dau() {
        TestDatabase::run(async |test| {
            let (app, counter, _) = setup(test).await;

            let first = post(&app, Some("key-1"), "hello").await;
            let again = post(&app, Some("key-1"), "hello").await;

            assert_eq!(first, (StatusCode::CREATED, None, "run 1: hello".to_owned()));
            assert_eq!(
                again,
                (StatusCode::CREATED, Some("true".to_owned()), "run 1: hello".to_owned())
            );
            assert_eq!(counter.runs.load(Ordering::SeqCst), 1);

            // Cùng key, request khác: từ chối, không chạy.
            let (status, _, body) = post(&app, Some("key-1"), "other").await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
            assert!(body.contains("idempotency_key_reused"), "{body}");
            // Không key: mỗi lần mỗi chạy.
            post(&app, None, "hello").await;
            post(&app, None, "hello").await;
            assert_eq!(counter.runs.load(Ordering::SeqCst), 3);
            // Key sai dạng.
            let long = "k".repeat(300);
            let (status, _, body) = post(&app, Some(&long), "hello").await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert!(body.contains("idempotency_key_invalid"), "{body}");
        })
        .await
        .unwrap();
    }

    /// Sáu request cùng key gửi cùng lúc: handler chạy đúng một lần; mọi bên khác nhận
    /// 409 (lần đầu chưa xong) — trên cả hai database.
    #[tokio::test]
    async fn gui_trung_dong_thoi_chi_chay_mot_lan() {
        TestDatabase::run(async |test| {
            let (app, counter, _) = setup(test).await;

            let attempts: Vec<_> = (0..6)
                .map(|_| {
                    let app = app.clone();
                    tokio::spawn(async move { post(&app, Some("same"), "hello").await })
                })
                .collect();
            let mut results = Vec::new();
            for attempt in attempts {
                let (status, replayed, _) = attempt.await.unwrap();
                results.push((status, replayed));
            }

            assert_eq!(counter.runs.load(Ordering::SeqCst), 1);
            // Đúng một response "thật"; mọi response khác là 409 (lần đầu chưa xong) hoặc
            // bản lưu lại của lần đầu (tuỳ chúng đến sớm hay muộn).
            let fresh = results
                .iter()
                .filter(|(status, replayed)| *status == StatusCode::CREATED && replayed.is_none());
            assert_eq!(fresh.count(), 1, "{results:?}");
            assert!(results.iter().all(|(status, replayed)| {
                *status == StatusCode::CONFLICT
                    || (*status == StatusCode::CREATED
                        && (replayed.is_none() || replayed.as_deref() == Some("true")))
            }));
            // Lần đầu xong rồi thì gửi lại nhận kết quả.
            let replay = post(&app, Some("same"), "hello").await;
            assert_eq!(replay.1.as_deref(), Some("true"));
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn loi_may_chu_thi_khong_luu_de_gui_lai_duoc() {
        TestDatabase::run(async |test| {
            let (app, counter, _) = setup(test).await;

            let first = post(&app, Some("boom"), "explode").await;
            let second = post(&app, Some("boom"), "explode").await;

            assert_eq!(first.0, StatusCode::INTERNAL_SERVER_ERROR);
            assert_eq!(second.0, StatusCode::INTERNAL_SERVER_ERROR);
            assert_eq!(second.1, None);
            assert_eq!(counter.runs.load(Ordering::SeqCst), 2);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn key_het_han_va_lan_dau_chet_giua_chung() {
        TestDatabase::run(async |test| {
            let (_, _, database) = setup(test).await;
            let store = Store::new(database.clone());
            let soon = now() + chrono::Duration::seconds(60);

            let Begin::Started { id } = store.begin("s", "k", "f", soon).await.unwrap() else {
                panic!("first request must start");
            };
            assert_eq!(store.begin("s", "k", "f", soon).await.unwrap(), Begin::InProgress);
            // Lần đầu chết giữa chừng: quá hạn giữ thì request sau nhận quyền chạy.
            Entity::update_many()
                .col_expr(Column::LockedUntil, Expr::value(now() - chrono::Duration::seconds(1)))
                .exec(database.write())
                .await
                .unwrap();
            assert_eq!(store.begin("s", "k", "f", soon).await.unwrap(), Begin::Started { id });
            let stored = StoredResponse { status: 201, headers: vec![], body: b"ok".to_vec() };
            store.complete(id, stored.clone()).await.unwrap();
            assert_eq!(store.begin("s", "k", "f", soon).await.unwrap(), Begin::Replay(stored));
            // Người gọi khác dùng cùng key: không đụng nhau.
            assert!(matches!(
                store.begin("other", "k", "f", soon).await.unwrap(),
                Begin::Started { .. }
            ));

            // Hết hạn giữ key: dọn đi, key dùng lại được như mới.
            Entity::update_many()
                .col_expr(Column::ExpiresAt, Expr::value(now() - chrono::Duration::seconds(1)))
                .exec(database.write())
                .await
                .unwrap();
            let context = JobContext {
                id: Uuid::now_v7(),
                kind: CLEANUP.to_owned(),
                payload: serde_json::Value::Null,
                attempt: 1,
            };
            Cleanup::new(database.clone()).run(&context).await.unwrap();
            assert!(matches!(
                store.begin("s", "k", "different", soon).await.unwrap(),
                Begin::Started { .. }
            ));
        })
        .await
        .unwrap();
    }
}
