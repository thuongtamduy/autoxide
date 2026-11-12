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
    async fn create(State(counter): State<Arc<Counter>>, body: String) -> axum::response::Response {
        use axum::response::IntoResponse as _;
        let run = counter.runs.fetch_add(1, Ordering::SeqCst) + 1;
        // Đủ lâu để các request đồng thời chắc chắn chồng lên nhau.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        if body.contains("explode") {
            return (StatusCode::INTERNAL_SERVER_ERROR, "boom".to_owned()).into_response();
        }
        if body.contains("throttled") {
            return (StatusCode::TOO_MANY_REQUESTS, "slow down".to_owned()).into_response();
        }
        if body.contains("panic") {
            // Như handler đã commit rồi mới chết.
            panic!("after commit");
        }
        if body.contains("huge") {
            return (StatusCode::CREATED, "x".repeat(300 * 1024)).into_response();
        }
        let text = format!("run {run}: {body}");
        if body.contains("secret") {
            // Như response tạo API key: secret chỉ trả một lần.
            let no_store = [(axum::http::header::CACHE_CONTROL, "no-store")];
            return (StatusCode::CREATED, no_store, text).into_response();
        }
        (StatusCode::CREATED, text).into_response()
    }

    /// `Bearer ada` và `Bearer ada-renewed` là cùng một người (token vừa làm mới);
    /// `Bearer bob` là người khác; không token thì không xác thực được.
    struct FakeAuthenticator;

    #[async_trait::async_trait]
    impl autoxide_kernel::access::Authenticator for FakeAuthenticator {
        async fn authenticate(
            &self,
            authorization: Option<&str>,
        ) -> Result<autoxide_kernel::access::Principal, autoxide_kernel::error::Error> {
            let user = match authorization {
                Some("Bearer ada" | "Bearer ada-renewed") => 1,
                Some("Bearer bob") => 2,
                _ => return Err(autoxide_kernel::error::Error::internal("unknown test token")),
            };
            Ok(autoxide_kernel::access::Principal::new(uuid::Uuid::from_u128(user), []))
        }
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
        let extras = autoxide_kernel::http::Extras {
            idempotency: Some(store),
            authenticator: Some(Arc::new(FakeAuthenticator)),
            ..Default::default()
        };
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
        post_as(app, Some("Bearer ada"), "/api/v1/notes", key, body).await
    }

    async fn post_as(
        app: &Router,
        authorization: Option<&str>,
        uri: &str,
        key: Option<&str>,
        body: &str,
    ) -> (StatusCode, Option<String>, String) {
        let mut request = Request::builder().method("POST").uri(uri);
        if let Some(authorization) = authorization {
            request = request.header("authorization", authorization);
        }
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

    /// Theo danh tính, không theo chuỗi token; người lạ không được lưu gì (D-079).
    #[tokio::test]
    async fn pham_vi_theo_nguoi_goi_va_nguoi_la_khong_duoc_luu() {
        use sea_orm::ConnectionTrait as _;
        TestDatabase::run(async |test| {
            let (app, counter, database) = setup(test).await;
            let notes = "/api/v1/notes";

            post_as(&app, Some("Bearer ada"), notes, Some("k"), "hello").await;
            // Client vừa làm mới token rồi gửi lại: vẫn là gửi lại.
            let renewed =
                post_as(&app, Some("Bearer ada-renewed"), notes, Some("k"), "hello").await;
            assert_eq!(renewed.1.as_deref(), Some("true"));
            // Người khác dùng trùng key: là việc riêng của họ.
            let bob = post_as(&app, Some("Bearer bob"), notes, Some("k"), "hello").await;
            assert_eq!(bob.1, None);
            // Cùng key, khác query: request khác, bị từ chối.
            let other =
                post_as(&app, Some("Bearer ada"), "/api/v1/notes?draft=1", Some("k"), "hello")
                    .await;
            assert_eq!(other.0, StatusCode::UNPROCESSABLE_ENTITY, "{}", other.2);
            assert_eq!(counter.runs.load(Ordering::SeqCst), 2);

            // Không xác thực được: handler chạy như thường, không ghi dòng nào.
            let backend = database.read().get_database_backend();
            let count = || async {
                let sql = "SELECT COUNT(*) AS n FROM idempotency_record";
                let row = database
                    .read()
                    .query_one_raw(sea_orm::Statement::from_string(backend, sql))
                    .await
                    .unwrap()
                    .unwrap();
                row.try_get::<i64>("", "n").unwrap()
            };
            let before = count().await;
            post_as(&app, None, notes, Some("anon"), "hello").await;
            post_as(&app, None, notes, Some("anon"), "hello").await;
            assert_eq!(count().await, before);
            assert_eq!(counter.runs.load(Ordering::SeqCst), 4);
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

    /// Lỗi tạm (429) không được lưu: gửi lại cùng key khi hết giới hạn thì việc chạy thật.
    /// Handler chết giữa chừng (có thể đã commit): gửi lại không chạy lại việc mà nhận 409
    /// giải thích. Response quá lớn không được lưu (D-156).
    #[tokio::test]
    async fn loi_tam_khong_luu_ket_qua_khong_ro_khong_chay_lai() {
        TestDatabase::run(async |test| {
            let (app, counter, _) = setup(test).await;

            let first = post(&app, Some("busy"), "throttled").await;
            let second = post(&app, Some("busy"), "throttled").await;
            assert_eq!(
                (first.0, second.0),
                (StatusCode::TOO_MANY_REQUESTS, StatusCode::TOO_MANY_REQUESTS)
            );
            assert_eq!(second.1, None, "not a replay");
            assert_eq!(counter.runs.load(Ordering::SeqCst), 2);

            let died = post(&app, Some("died"), "panic").await;
            assert_eq!(died.0, StatusCode::INTERNAL_SERVER_ERROR);
            let again = post(&app, Some("died"), "panic").await;
            assert_eq!(again.0, StatusCode::CONFLICT);
            assert!(again.2.contains("idempotency_result_unknown"), "{}", again.2);
            assert_eq!(counter.runs.load(Ordering::SeqCst), 3);

            let big = post(&app, Some("big"), "huge").await;
            assert_eq!((big.0, big.2.len()), (StatusCode::CREATED, 300 * 1024));
            let big_again = post(&app, Some("big"), "huge").await;
            assert_eq!(big_again.0, StatusCode::CONFLICT);
            assert!(big_again.2.contains("idempotency_result_too_large"), "{}", big_again.2);
            assert_eq!(counter.runs.load(Ordering::SeqCst), 4);
        })
        .await
        .unwrap();
    }

    /// Response có `Cache-Control: no-store` (chứa secret) không vào database; gửi lại nhận
    /// 409 giải thích, không chạy lại việc (D-149).
    #[tokio::test]
    async fn response_chua_secret_khong_duoc_luu() {
        TestDatabase::run(async |test| {
            let (app, counter, database) = setup(test).await;

            let first = post(&app, Some("key-once"), "secret axk_123").await;
            let second = post(&app, Some("key-once"), "secret axk_123").await;

            assert_eq!(first.0, StatusCode::CREATED);
            assert!(first.2.contains("axk_123"));
            assert_eq!(second.0, StatusCode::CONFLICT);
            assert_eq!(second.1.as_deref(), Some("true"));
            assert!(second.2.contains("idempotency_result_not_kept"), "{}", second.2);
            assert!(!second.2.contains("axk_123"));
            assert_eq!(counter.runs.load(Ordering::SeqCst), 1);
            let rows = entity::Entity::find().all(database.read()).await.unwrap();
            assert_eq!(rows.len(), 1);
            assert!(rows.iter().all(|row| {
                row.body
                    .as_deref()
                    .is_none_or(|body| !String::from_utf8_lossy(body).contains("axk_123"))
            }));
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
