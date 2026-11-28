//! Tests of the module, on the database `TEST_DATABASE_URL` selects: unset is SQLite, a
//! `postgres://…` URL is PostgreSQL. Run them on both — `../check.sh` does.
//!
//! `TestDatabase::run` gives every test an empty database. `TestApp` builds the module the way
//! the server does (settings → migrations → `start` → routes and tools → the HTTP layers of the
//! core) and calls it over REST and with a real MCP client.

use std::sync::Arc;

use autoxide_kernel::event::{Delivered, EventHandler, Outbox};
use autoxide_kernel::job::{EnqueueOptions, Job, JobQueue};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::time::{SignedDuration, Timestamp};
use autoxide_testkit::{TestApp, TestDatabase, leak_check, tool};
use autoxide_testkit::http::StatusCode;
use sea_orm::{ActiveModelTrait, ConnectionTrait, Set, Statement};
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::Bookmarks;
use crate::config::BookmarksConfig;
use crate::service::BookmarkService;

const ALL: [&str; 2] = ["bookmarks.read", "bookmarks.write"];
const ADA: u128 = 0xada;
const BOB: u128 = 0xb0b;

/// Two users: token `ada` may read and write, token `bob` may only read.
async fn app(test: &TestDatabase) -> TestApp {
    TestApp::builder(test)
        .user("ada", ADA, &ALL)
        .user("bob", BOB, &["bookmarks.read"])
        .module(crate::module())
        .build()
        .await
        .unwrap()
}

async fn create(app: &TestApp, token: &str, url: &str, title: &str) -> (StatusCode, Value) {
    let answer = app
        .post("/api/v1/bookmarks")
        .token(token)
        .header("accept-language", "vi")
        .json(json!({"url": url, "title": title}))
        .send()
        .await
        .unwrap();
    (answer.status_code(), answer.json())
}

async fn events(app: &TestApp) -> Vec<String> {
    let read = app.database().read();
    let sql = "SELECT name FROM bookmarks_events ORDER BY id";
    let rows = read.query_all_raw(Statement::from_string(read.get_database_backend(), sql)).await;
    rows.unwrap().iter().map(|row| row.try_get("", "name").unwrap()).collect()
}

/// Puts a bookmark created `age_days` ago into the table of `tenant`.
async fn old_bookmark(tenant: &TenantContext, owner: u128, age_days: i64) {
    crate::entity::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant.id().as_uuid()),
        owner_id: Set(Uuid::from_u128(owner)),
        url: Set(format!("https://example.com/{age_days}")),
        title: Set("old".to_owned()),
        created_at: Set(Timestamp::now() - SignedDuration::from_hours(24 * age_days)),
    }
    .insert(tenant.database().write())
    .await
    .unwrap();
}

#[tokio::test]
async fn rest_and_mcp_share_one_service() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let ada = app.mcp("ada").await.unwrap();

        // Written over REST, read over MCP: the same shape, `data` + `pagination`.
        let (status, created) = create(&app, "ada", "https://www.rust-lang.org/", "Rust").await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let listed = ada.call_tool(tool("bookmarks.list_bookmarks", json!({}))).await.unwrap();
        let listed = listed.structured_content.unwrap();
        assert_eq!(listed["data"][0]["id"], created["data"]["id"]);
        assert!(listed["pagination"].is_object(), "{listed}");

        // The same error with the same code both ways; REST answers in the language asked for.
        let (status, problem) = create(&app, "ada", "https://www.rust-lang.org/", "Again").await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(problem["code"], "bookmarks.url_taken");
        assert_eq!(problem["detail"], "Bạn đã lưu liên kết `https://www.rust-lang.org/` rồi.");
        let again = json!({"url": "https://www.rust-lang.org/", "title": "Again"});
        let taken = ada.call_tool(tool("bookmarks.create_bookmark", again)).await.unwrap();
        assert_eq!(taken.is_error, Some(true));
        assert_eq!(taken.structured_content.unwrap()["code"], "bookmarks.url_taken");

        // Every problem of the input is reported at once.
        let (status, problem) = create(&app, "ada", "ftp://nowhere", "   ").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let fields: Vec<&str> = problem["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|error| error["field"].as_str().unwrap())
            .collect();
        assert_eq!(fields, ["url", "title"], "{problem}");

        let id = created["data"]["id"].as_str().unwrap();
        let deleted = ada.call_tool(tool("bookmarks.delete_bookmark", json!({"id": id}))).await;
        assert_eq!(deleted.unwrap().is_error, Some(false));
        let gone = app.get(&format!("/api/v1/bookmarks/{id}")).token("ada").send().await.unwrap();
        assert_eq!(gone.status_code(), StatusCode::NOT_FOUND);
        assert_eq!(gone.json()["code"], "bookmarks.bookmark_not_found");

        // One event per successful write, in the outbox table of the module.
        assert_eq!(
            events(&app).await,
            ["bookmarks.bookmark_created", "bookmarks.bookmark_deleted"]
        );
        ada.cancel().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn permissions_are_checked_and_users_see_only_their_own() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (status, created) = create(&app, "ada", "https://example.com/a", "A").await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let id = created["data"]["id"].as_str().unwrap();

        // `bob` has no `bookmarks.write`.
        let (status, problem) = create(&app, "bob", "https://example.com/b", "B").await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(problem["code"], "permission_denied");
        // No token at all.
        let anonymous = app.get("/api/v1/bookmarks").send().await.unwrap();
        assert_eq!(anonymous.status_code(), StatusCode::UNAUTHORIZED);

        // `bob` may read, but not the bookmarks of `ada`.
        let list = app.get("/api/v1/bookmarks").token("bob").send().await.unwrap();
        assert_eq!(list.json()["data"], json!([]));
        let one = app.get(&format!("/api/v1/bookmarks/{id}")).token("bob").send().await.unwrap();
        assert_eq!(one.status_code(), StatusCode::NOT_FOUND);

        // The trait other modules use sees the same data.
        let bookmarks = app.api::<dyn Bookmarks>().unwrap();
        let tenant = app.default_tenant();
        assert_eq!(bookmarks.count(&tenant, Uuid::from_u128(ADA)).await.unwrap(), 1);
        assert_eq!(bookmarks.count(&tenant, Uuid::from_u128(BOB)).await.unwrap(), 0);
    })
    .await
    .unwrap();
}

/// Anything written concurrently needs a concurrency test: ten requests keep the same link at
/// once, the unique index lets exactly one through.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_same_link_at_the_same_time_is_kept_once() {
    TestDatabase::run(async |test| {
        let app = Arc::new(app(test).await);
        let attempts: Vec<_> = (0..10)
            .map(|number| {
                let app = app.clone();
                tokio::spawn(async move {
                    create(&app, "ada", "https://example.com/race", &format!("try {number}"))
                        .await
                        .0
                })
            })
            .collect();
        let mut created = 0;
        for attempt in attempts {
            match attempt.await.unwrap() {
                StatusCode::CREATED => created += 1,
                status => assert_eq!(status, StatusCode::CONFLICT),
            }
        }
        assert_eq!(created, 1);
        assert_eq!(events(&app).await, ["bookmarks.bookmark_created"]);
    })
    .await
    .unwrap();
}

/// Two customers on one server, one in the shared database and one in a database of its own, with
/// the same user id in both: nothing of one is visible to the other, by any read route or tool.
#[tokio::test]
async fn tenants_do_not_see_each_other() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .tenant("shared")
            .tenant_dedicated("own")
            .user_in("shared", "token-shared", ADA, &ALL)
            .user_in("own", "token-own", ADA, &ALL)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        for (owner, other) in [("token-shared", "token-own"), ("token-own", "token-shared")] {
            let marker = format!("LEAK-{owner}");
            let url = format!("https://example.com/{marker}");
            let (status, created) = create(&app, owner, &url, &marker).await;
            assert_eq!(status, StatusCode::CREATED, "{created}");
            let id = created["data"]["id"].as_str().unwrap().to_owned();

            let leaks = leak_check(&app, other, &marker, &[("id", id.clone())]).await.unwrap();
            assert_eq!(leaks, Vec::<String>::new());
            let uri = format!("/api/v1/bookmarks/{id}");
            let removed = app.delete(&uri).token(other).send().await.unwrap();
            assert_eq!(removed.status_code(), StatusCode::NOT_FOUND);
            let mine = app.get(&uri).token(owner).send().await.unwrap();
            assert_eq!(mine.json()["data"]["title"], marker);
        }
    })
    .await
    .unwrap();
}

/// What `cargo xtask check` cannot see from the source, asked of the database itself: every table
/// belongs to the module (`bookmarks_…`), no foreign key leaves it, and tenant-level tables carry
/// `tenant_id` first in every key and index.
#[tokio::test]
async fn tables_follow_the_rules() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let connection = test.connect().await.unwrap();
        let ownership =
            autoxide_testkit::schema_ownership_violations(&connection, &app.modules().names())
                .await
                .unwrap();
        assert_eq!(ownership, Vec::<String>::new());

        let snapshot = autoxide_testkit::schema_snapshot(app.database().write()).await.unwrap();
        let control_plane = app.modules().control_plane_tables();
        let tenancy = autoxide_testkit::schema_tenancy_violations(&snapshot, &control_plane);
        assert_eq!(tenancy, Vec::<String>::new());
    })
    .await
    .unwrap();
}

/// The scheduled job, run now: put it into the queue of the database, then let a worker run it.
#[derive(Debug, Serialize)]
struct CleanupNow {}

impl Job for CleanupNow {
    const KIND: &'static str = crate::retention::CLEANUP;
}

#[tokio::test]
async fn the_retention_job_deletes_old_bookmarks() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            // Like an environment variable of the server.
            .setting("ACME_BOOKMARKS_RETENTION_DAYS", "30")
            .user("ada", ADA, &ALL)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let tenant = app.default_tenant();
        old_bookmark(&tenant, ADA, 31).await;
        old_bookmark(&tenant, ADA, 29).await;

        JobQueue::new("bookmarks")
            .enqueue_database(app.database().write(), &CleanupNow {}, EnqueueOptions::default())
            .await
            .unwrap();
        let worker = app.modules().worker(1).unwrap();
        assert_eq!(worker.run_once().await.unwrap(), 1);

        let list = app.get("/api/v1/bookmarks").token("ada").send().await.unwrap();
        let urls: Vec<Value> =
            list.json()["data"].as_array().unwrap().iter().map(|b| b["url"].clone()).collect();
        assert_eq!(urls, [json!("https://example.com/29")]);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn a_wrong_setting_stops_the_app_with_a_clear_message() {
    TestDatabase::run(async |test| {
        let built = TestApp::builder(test)
            .setting("ACME_BOOKMARKS_RETENTION_DAYS", "forever")
            .module(crate::module())
            .build()
            .await;
        let message = built.err().unwrap().to_string();
        assert!(message.contains("ACME_BOOKMARKS_RETENTION_DAYS"), "{message}");
    })
    .await
    .unwrap();
}

/// The account of `ada` is deleted (an event of the `auth` module): her bookmarks go, those of
/// `bob` stay. Receiving the event a second time changes nothing.
#[tokio::test]
async fn bookmarks_of_a_deleted_user_are_removed() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let tenant = app.default_tenant();
        old_bookmark(&tenant, ADA, 1).await;
        old_bookmark(&tenant, BOB, 1).await;

        let service =
            BookmarkService::new(Outbox::standalone("bookmarks"), BookmarksConfig::default());
        let deleted = Delivered::new(
            tenant.clone(),
            "auth",
            autoxide_kernel::privacy::USER_DELETED.to_owned(),
            json!({"before": {"id": Uuid::from_u128(ADA)}}),
        );
        service.handle(&deleted).await.unwrap();
        service.handle(&deleted).await.unwrap();

        assert_eq!(service.count(&tenant, Uuid::from_u128(ADA)).await.unwrap(), 0);
        assert_eq!(service.count(&tenant, Uuid::from_u128(BOB)).await.unwrap(), 1);
    })
    .await
    .unwrap();
}
