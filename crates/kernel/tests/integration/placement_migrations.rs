//! Migrate placement bằng API (D-334): chạy thử liệt kê migration còn chờ mà không đổi gì; chạy
//! thật bằng job, ghi phần đã áp dụng; một lượt một lúc; chỉ nhân sự nền tảng. Cả SQLite và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use sea_orm::{ConnectionTrait, Statement};
use serde_json::{Value, json};

async fn call(
    app: &TestApp,
    method: Method,
    path: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = app.request(method, &format!("/api/v1/kernel{path}")).token(who);
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

#[tokio::test]
async fn migrate_placement_bang_api() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .tenant_dedicated("solo")
            .user("ops", 1, &["kernel.tenants.operate"])
            .user("reader", 2, &["kernel.tenants.read"])
            .build()
            .await
            .unwrap();
        // Làm placement của `solo` chậm một migration: xoá bảng và dòng lịch sử của nó.
        let solo = app.tenant("solo").await.unwrap();
        let write = solo.database().write();
        let backend = write.get_database_backend();
        for sql in [
            "DROP TABLE kernel_sequences",
            "DELETE FROM kernel_migrations WHERE version = 'm0016_create_sequences'",
        ] {
            write.execute_raw(Statement::from_string(backend, sql)).await.unwrap();
        }

        // Không phải nhân sự vận hành: bị chặn.
        let body = json!({"dry_run": true});
        let (status, _) =
            call(&app, Method::POST, "/placements/migrations", "reader", Some(body.clone())).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // Chạy thử: thấy migration còn chờ, không đổi gì.
        let (status, dry) =
            call(&app, Method::POST, "/placements/migrations", "ops", Some(body.clone())).await;
        assert_eq!(status, StatusCode::OK, "{dry}");
        let pending = dry["data"]["pending"].as_array().unwrap();
        assert_eq!(pending.len(), 1, "{dry}");
        assert_eq!(pending[0]["migrations"], json!(["kernel/m0016_create_sequences"]));
        let (_, again) =
            call(&app, Method::POST, "/placements/migrations", "ops", Some(body)).await;
        assert_eq!(again["data"]["pending"], dry["data"]["pending"], "chạy thử không đổi gì");
        let placement = pending[0]["placement"].as_str().unwrap().to_owned();

        // Placement lạ: 404.
        let (status, missing) = call(
            &app,
            Method::POST,
            "/placements/migrations",
            "ops",
            Some(json!({"placement": "nowhere"})),
        )
        .await;
        assert_eq!(
            (status, missing["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("tenant_placement_not_found"))
        );

        // Chạy thật: một lượt một lúc.
        let (status, run) = call(
            &app,
            Method::POST,
            "/placements/migrations",
            "ops",
            Some(json!({"placement": placement})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{run}");
        assert_eq!(run["data"]["status"], "pending");
        let id = run["data"]["id"].as_str().unwrap().to_owned();
        let (status, busy) =
            call(&app, Method::POST, "/placements/migrations", "ops", Some(json!({}))).await;
        assert_eq!(
            (status, busy["code"].as_str()),
            (StatusCode::CONFLICT, Some("kernel.migration_run_busy"))
        );

        let worker = app.modules().worker(2).unwrap();
        for _ in 0..5 {
            if worker.run_once().await.unwrap() == 0 {
                break;
            }
        }
        let (status, done) =
            call(&app, Method::GET, &format!("/placements/migrations/{id}"), "ops", None).await;
        assert_eq!(status, StatusCode::OK, "{done}");
        assert_eq!(done["data"]["status"], "done", "{done}");
        assert_eq!(
            done["data"]["applied"][0]["migrations"],
            json!(["kernel/m0016_create_sequences"])
        );
        let (_, listed) = call(&app, Method::GET, "/placements/migrations", "ops", None).await;
        assert_eq!(listed["data"][0]["id"], id);
        // Bảng đã có lại; chạy thử không còn gì chờ.
        let (_, dry) = call(
            &app,
            Method::POST,
            "/placements/migrations",
            "ops",
            Some(json!({"dry_run": true})),
        )
        .await;
        assert_eq!(dry["data"]["pending"][0]["migrations"], json!([]));

        // Lượt trước đã xong (khoá đã nhả); nhiều yêu cầu cùng lúc: đúng một lượt được xếp. Trước
        // đây bước "chưa có lượt nào đang chạy" đọc không khoá: Postgres xếp nhiều lượt song song.
        let tries = (0..6)
            .map(|_| call(&app, Method::POST, "/placements/migrations", "ops", Some(json!({}))));
        let outcomes = futures_util::future::join_all(tries).await;
        let created = outcomes.iter().filter(|(status, _)| *status == StatusCode::CREATED).count();
        assert_eq!(created, 1, "{outcomes:?}");
        for (status, answer) in &outcomes {
            assert!(
                *status == StatusCode::CREATED || answer["code"] == "kernel.migration_run_busy",
                "{status} {answer}"
            );
        }
    })
    .await
    .unwrap();
}

/// Lượt migrate mọi placement không dừng ở placement hỏng (bước 71): placement sau vẫn được nâng,
/// lượt kết thúc `failed` và lỗi nêu placement hỏng.
#[tokio::test]
async fn luot_migrate_lam_tiep_qua_placement_hong() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .tenant_dedicated("alpha")
            .tenant_dedicated("beta")
            .user("ops", 1, &["kernel.tenants.operate"])
            .build()
            .await
            .unwrap();
        // alpha: chỉ xoá dòng lịch sử — chạy lại migration thì bảng đã có, hỏng. beta: thiếu thật.
        let alpha = app.tenant("alpha").await.unwrap();
        let beta = app.tenant("beta").await.unwrap();
        let forget = "DELETE FROM kernel_migrations WHERE version = 'm0016_create_sequences'";
        for (tenant, sqls) in
            [(&alpha, vec![forget]), (&beta, vec!["DROP TABLE kernel_sequences", forget])]
        {
            let write = tenant.database().write();
            let backend = write.get_database_backend();
            for sql in sqls {
                write.execute_raw(Statement::from_string(backend, sql)).await.unwrap();
            }
        }

        let (status, run) =
            call(&app, Method::POST, "/placements/migrations", "ops", Some(json!({}))).await;
        assert_eq!(status, StatusCode::CREATED, "{run}");
        let id = run["data"]["id"].as_str().unwrap().to_owned();
        let worker = app.modules().worker(2).unwrap();
        for _ in 0..5 {
            if worker.run_once().await.unwrap() == 0 {
                break;
            }
        }
        let (status, done) =
            call(&app, Method::GET, &format!("/placements/migrations/{id}"), "ops", None).await;
        assert_eq!(status, StatusCode::OK, "{done}");
        assert_eq!(done["data"]["status"], "failed", "{done}");
        let applied = done["data"]["applied"].as_array().unwrap();
        assert_eq!(applied.len(), 1, "{done}");
        assert_eq!(applied[0]["migrations"], json!(["kernel/m0016_create_sequences"]));
        let error = done["data"]["error"].as_str().unwrap();
        assert!(error.contains("placement `alpha"), "{error}");
        assert!(!error.contains(applied[0]["placement"].as_str().unwrap()), "{error}");
    })
    .await
    .unwrap();
}

/// Job của API và lệnh `migrate up --all` cùng lúc: cùng khoá, cùng chỗ `active` — không bên nào hỏng, lượt đã
/// xếp được một bên chạy rồi khép lại. Lượt bỏ dở của tiến trình đã chết không chặn lượt mới (không 409 mãi).
#[tokio::test]
async fn job_api_va_lenh_cung_luc_va_luot_bo_do() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .tenant_dedicated("alpha")
            .tenant_dedicated("beta")
            .user("ops", 1, &["kernel.tenants.operate"])
            .build()
            .await
            .unwrap();
        let forget = [
            "DROP TABLE kernel_sequences",
            "DELETE FROM kernel_migrations WHERE version = 'm0016_create_sequences'",
        ];
        for slug in ["alpha", "beta"] {
            let tenant = app.tenant(slug).await.unwrap();
            let write = tenant.database().write();
            let backend = write.get_database_backend();
            for sql in forget {
                write.execute_raw(Statement::from_string(backend, sql)).await.unwrap();
            }
        }
        let placements = app.modules().placements(app.registry()).unwrap();
        placements.check().await.unwrap();

        let (status, run) =
            call(&app, Method::POST, "/placements/migrations", "ops", Some(json!({}))).await;
        assert_eq!(status, StatusCode::CREATED, "{run}");
        let id = run["data"]["id"].as_str().unwrap().to_owned();
        let worker = app.modules().worker(2).unwrap();
        let (direct, job) = tokio::join!(placements.up_all(), worker.run_once());
        direct.unwrap();
        job.unwrap();
        for _ in 0..5 {
            if worker.run_once().await.unwrap() == 0 {
                break;
            }
        }
        let (_, done) =
            call(&app, Method::GET, &format!("/placements/migrations/{id}"), "ops", None).await;
        assert_eq!(done["data"]["status"], "done", "{done}");
        for check in placements.check().await.unwrap() {
            assert_eq!(check.state, autoxide_kernel::tenancy::SchemaState::Ready, "{check:?}");
        }

        // Lượt `running` của tiến trình đã chết: lượt mới vẫn được xếp, lượt cũ thành `failed`.
        let write = app.registry().control().write();
        let backend = write.get_database_backend();
        let run_id: uuid::Uuid = id.parse().unwrap();
        let sql = "UPDATE kernel_migration_runs SET status = 'running', active_key = 'active' WHERE id = $1";
        let update = autoxide_kernel::database::statement(backend, sql, [run_id.into()]);
        write.execute_raw(update).await.unwrap();
        let (status, again) =
            call(&app, Method::POST, "/placements/migrations", "ops", Some(json!({}))).await;
        assert_eq!(status, StatusCode::CREATED, "{again}");
        let (_, old) =
            call(&app, Method::GET, &format!("/placements/migrations/{id}"), "ops", None).await;
        assert_eq!(old["data"]["status"], "failed", "{old}");
        assert!(old["data"]["error"].as_str().unwrap().starts_with("interrupted"), "{old}");
    })
    .await
    .unwrap();
}
