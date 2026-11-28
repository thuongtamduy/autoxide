//! Test của `store.rs`: lớp idempotency của kernel chạy trên bảng của module, qua REST thật.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use autoxide_kernel::database::Database;
use autoxide_kernel::http::{NoStore, State, ValidJson, Validate, endpoint};
use autoxide_kernel::module::{Manifest, Module, StartContext};
use autoxide_kernel::response::Created;
use autoxide_kernel::routes;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_testkit::http::StatusCode;
use autoxide_testkit::{TestApp, TestDatabase, all_tenants};
use serde::Deserialize;

/// Đặt cột thời gian `column` của mọi dòng về một giây trước (giả lập quá hạn giữ / hết hạn).
async fn set_all(database: &Database, column: autoxide_kernel::db::Col<Row, Timestamp>) {
    let transaction = all_tenants(database).begin_write().await.unwrap();
    let past = now() - SignedDuration::from_secs(1);
    transaction.update::<Row>().set(column, past).exec().await.unwrap();
    transaction.commit().await.unwrap();
}

/// Đếm số lần handler thật sự chạy.
#[derive(Default)]
struct Counter {
    runs: AtomicUsize,
}

/// What the fake handler is asked to do.
#[derive(Debug, Deserialize, Validate)]
struct Probe {
    text: String,
}

/// Create a note.
#[endpoint(post, path = "/", public, responses((status = 201, description = "Created.")))]
async fn create(
    State(counter): State<Arc<Counter>>,
    ValidJson(probe): ValidJson<Probe>,
) -> Result<Created<String>, Error> {
    let body = probe.text;
    let run = counter.runs.fetch_add(1, Ordering::SeqCst) + 1;
    // Đủ lâu để các request đồng thời chắc chắn chồng lên nhau.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    if body.contains("explode") {
        return Err(Error::internal("boom"));
    }
    if body.contains("throttled") {
        return Err(autoxide_kernel::entitlement::quota_exhausted("notes.notes", 60));
    }
    if body.contains("panic") {
        // Như handler đã commit rồi mới chết.
        panic!("after commit");
    }
    if body.contains("huge") {
        return Ok(Created::new("/notes/1", "x".repeat(300 * 1024)));
    }
    Ok(Created::new("/notes/1", format!("run {run}: {body}")))
}

/// Create a key shown once.
#[endpoint(post, path = "/secret", public, responses((status = 201, description = "Created.")))]
async fn create_secret(
    State(counter): State<Arc<Counter>>,
    ValidJson(probe): ValidJson<Probe>,
) -> Result<NoStore<Created<String>>, Error> {
    let run = counter.runs.fetch_add(1, Ordering::SeqCst) + 1;
    // Như response tạo API key: secret chỉ trả một lần (`Cache-Control: no-store`).
    Ok(NoStore(Created::new("/notes/1", format!("run {run}: {}", probe.text))))
}

/// Module `notes` giả: một route `POST /api/v1/notes` đếm số lần chạy.
struct Notes(Arc<Counter>);

impl Module for Notes {
    fn manifest(&self) -> Manifest {
        Manifest::new("notes", "Fake notes.")
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.routes(routes![create, create_secret].with_state(self.0.clone()))
    }
}

/// `Bearer ada` và `Bearer ada-renewed` là cùng một người (token vừa làm mới);
/// `Bearer bob` là người khác; không token thì không xác thực được.
async fn setup(test: &TestDatabase) -> (TestApp, Arc<Counter>, Database) {
    let counter = Arc::new(Counter::default());
    let app = TestApp::builder(test)
        .user("ada", 1, &[])
        .user("ada-renewed", 1, &[])
        .user("bob", 2, &[])
        .module(crate::module())
        .module(Notes(counter.clone()))
        .build()
        .await
        .unwrap();
    let database = app.database().clone();
    (app, counter, database)
}

async fn post(
    app: &TestApp,
    key: Option<&str>,
    body: &str,
) -> (StatusCode, Option<String>, String) {
    post_as(app, Some("Bearer ada"), "/api/v1/notes", key, body).await
}

async fn post_as(
    app: &TestApp,
    authorization: Option<&str>,
    uri: &str,
    key: Option<&str>,
    body: &str,
) -> (StatusCode, Option<String>, String) {
    let mut request = app.post(uri);
    if let Some(authorization) = authorization {
        request = request.header("authorization", authorization);
    }
    if let Some(key) = key {
        request = request.header("idempotency-key", key);
    }
    // Handler giả cố ý trả câu trả lời ngoài hợp đồng (500 / 429, panic) để thử lớp idempotency, nên
    // không kiểm theo `docs/openapi.json`.
    let text = serde_json::json!({ "text": body });
    let answer = request.json(text).unchecked().send().await.unwrap();
    let replayed =
        Some(answer.header("idempotent-replayed").to_owned()).filter(|value| !value.is_empty());
    let raw = String::from_utf8(answer.body().to_vec()).unwrap();
    // 201 trả `{"data": "<chữ>"}`: lấy phần chữ; lỗi (`Problem`) giữ nguyên để tìm `code` trong đó.
    let data = serde_json::from_str::<serde_json::Value>(&raw)
        .ok()
        .and_then(|value| value.get("data").and_then(|data| data.as_str()).map(str::to_owned));
    (answer.status_code(), replayed, data.unwrap_or(raw))
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
    TestDatabase::run(async |test| {
        let (app, counter, database) = setup(test).await;
        let notes = "/api/v1/notes";

        post_as(&app, Some("Bearer ada"), notes, Some("k"), "hello").await;
        // Client vừa làm mới token rồi gửi lại: vẫn là gửi lại.
        let renewed = post_as(&app, Some("Bearer ada-renewed"), notes, Some("k"), "hello").await;
        assert_eq!(renewed.1.as_deref(), Some("true"));
        // Người khác dùng trùng key: là việc riêng của họ.
        let bob = post_as(&app, Some("Bearer bob"), notes, Some("k"), "hello").await;
        assert_eq!(bob.1, None);
        // Cùng key, khác query: request khác, bị từ chối.
        let other =
            post_as(&app, Some("Bearer ada"), "/api/v1/notes?draft=1", Some("k"), "hello").await;
        assert_eq!(other.0, StatusCode::UNPROCESSABLE_ENTITY, "{}", other.2);
        assert_eq!(counter.runs.load(Ordering::SeqCst), 2);

        // Không xác thực được: handler chạy như thường, không ghi dòng nào.
        let count = || async { all_tenants(&database).read().find::<Row>().count().await.unwrap() };
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
        // `Arc`: mỗi task giữ một tham chiếu tới cùng app, sống tới khi task xong.
        let app = Arc::new(app);

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
/// giải thích. Response quá lớn không được lưu (D-157).
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

        let secret = |app| {
            post_as(app, Some("Bearer ada"), "/api/v1/notes/secret", Some("key-once"), "axk_123")
        };
        let first = secret(&app).await;
        let second = secret(&app).await;

        assert_eq!(first.0, StatusCode::CREATED);
        assert!(first.2.contains("axk_123"));
        assert_eq!(second.0, StatusCode::CONFLICT);
        assert_eq!(second.1.as_deref(), Some("true"));
        assert!(second.2.contains("idempotency_result_not_kept"), "{}", second.2);
        assert!(!second.2.contains("axk_123"));
        assert_eq!(counter.runs.load(Ordering::SeqCst), 1);
        let rows = all_tenants(&database).read().find::<Row>().all().await.unwrap();
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
        let (app, _, database) = setup(test).await;
        let store = Store;
        let tenant = app.default_tenant();
        let soon = now() + SignedDuration::from_secs(60);

        let Begin::Started { id } = store.begin(&tenant, "s", "k", "f", soon).await.unwrap() else {
            panic!("first request must start");
        };
        assert_eq!(store.begin(&tenant, "s", "k", "f", soon).await.unwrap(), Begin::InProgress);
        // Lần đầu chết giữa chừng: quá hạn giữ thì request sau nhận quyền chạy.
        set_all(&database, Row::LOCKED_UNTIL).await;
        assert_eq!(store.begin(&tenant, "s", "k", "f", soon).await.unwrap(), Begin::Started { id });
        let stored = StoredResponse::new(201, vec![], b"ok".to_vec());
        store.complete(&tenant, id, stored.clone()).await.unwrap();
        assert_eq!(store.begin(&tenant, "s", "k", "f", soon).await.unwrap(), Begin::Replay(stored));
        // Người gọi khác dùng cùng key: không đụng nhau.
        assert!(matches!(
            store.begin(&tenant, "other", "k", "f", soon).await.unwrap(),
            Begin::Started { .. }
        ));

        // Hết hạn giữ key: dọn đi, key dùng lại được như mới.
        set_all(&database, Row::EXPIRES_AT).await;
        let context =
            JobContext::for_database(database.clone(), CLEANUP, serde_json::Value::Null, 1);
        Cleanup.run(&context).await.unwrap();
        assert!(matches!(
            store.begin(&tenant, "s", "k", "different", soon).await.unwrap(),
            Begin::Started { .. }
        ));
    })
    .await
    .unwrap();
}

/// Bước 33: cùng key ở hai khách là hai lần chạy khác nhau (trên database chung và database
/// riêng); job dọn chạy trên từng database.
#[tokio::test]
async fn cung_key_o_hai_khach_deu_chay() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .tenant("a")
            .tenant_dedicated("b")
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let soon = now() + SignedDuration::from_secs(60);
        let (a, b) = (app.tenant("a").await.unwrap(), app.tenant("b").await.unwrap());
        for tenant in [&a, &b, &app.default_tenant()] {
            let started = Store.begin(tenant, "s", "k", "f", soon).await.unwrap();
            assert!(matches!(started, Begin::Started { .. }), "{started:?}");
        }
        assert_eq!(Store.begin(&a, "s", "k", "f", soon).await.unwrap(), Begin::InProgress);
        let rows = b.db().read().find::<Row>().all().await.unwrap();
        assert_eq!(rows.len(), 1);
    })
    .await
    .unwrap();
}

/// Create a memo in the tenant of the caller.
#[endpoint(post, path = "/", public, responses((status = 201, description = "Created.")))]
async fn create_memo(
    _tenant: TenantContext,
    State(counter): State<Arc<Counter>>,
) -> Created<String> {
    counter.runs.fetch_add(1, Ordering::SeqCst);
    Created::new("/memos/1", String::new())
}

/// Module giả `memos`: lệnh ghi cần khách của request (bị chặn khi khách chỉ đọc).
struct Memos(Arc<Counter>);

impl Module for Memos {
    fn manifest(&self) -> Manifest {
        Manifest::new("memos", "Fake memos.")
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.routes(routes![create_memo].with_state(self.0.clone()))
    }
}

/// Khách chỉ đọc: lệnh ghi bị từ chối và câu từ chối không được lưu theo key — gửi lại cùng key
/// sau khi khách ghi được lại thì lệnh chạy thật (đối chiếu thiết kế 2, A-3.11).
#[tokio::test]
async fn khach_chi_doc_khong_luu_cau_tu_choi() {
    use autoxide_kernel::tenancy::{TenantAdmin, TenantChanges};
    TestDatabase::run(async |test| {
        let counter = Arc::new(Counter::default());
        let app = TestApp::builder(test)
            .tenant("acme")
            .user_in("acme", "cara", 3, &[])
            .module(crate::module())
            .module(Memos(counter.clone()))
            .build()
            .await
            .unwrap();
        let admin = app.api::<TenantAdmin>().unwrap();
        let id = app.tenant("acme").await.unwrap().id().as_uuid();
        let set = |read_only: bool| {
            let changes = TenantChanges { read_only: Some(read_only), ..Default::default() };
            admin.update(autoxide_kernel::event::Actor::system(), id, changes, None)
        };
        set(true).await.unwrap();
        let refused = post_as(&app, Some("Bearer cara"), "/api/v1/memos", Some("k"), "").await;
        assert_eq!(refused.0, StatusCode::FORBIDDEN, "{}", refused.2);
        assert!(refused.2.contains("tenant_read_only"), "{}", refused.2);
        set(false).await.unwrap();
        let done = post_as(&app, Some("Bearer cara"), "/api/v1/memos", Some("k"), "").await;
        assert_eq!((done.0, done.1), (StatusCode::CREATED, None));
        assert_eq!(counter.runs.load(Ordering::SeqCst), 1);
    })
    .await
    .unwrap();
}
