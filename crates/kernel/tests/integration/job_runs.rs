//! Job theo dõi được (bước 51 P15): job hỏng hẳn hiện `failed` kèm lỗi; chạy lại cần
//! `kernel.jobs.manage`, ghi event, đưa job về hàng đợi.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use autoxide_kernel::error::Error;
use autoxide_kernel::job::{
    EnqueueOptions, Job, JobContext, JobHandler, JobQueue, JobRegistration, jobs_migration,
    tenant_table,
};
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::tenancy::migration::adopt;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct Boom;

impl Job for Boom {
    const KIND: &'static str = "demo.boom";
}

struct Explodes;

#[async_trait::async_trait]
impl JobHandler for Explodes {
    async fn run(&self, _: &JobContext) -> Result<(), Error> {
        Err(Error::internal("the partner did not answer"))
    }
}

struct Demo(Arc<Mutex<Option<JobQueue>>>);

impl Module for Demo {
    fn name(&self) -> &'static str {
        "demo"
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(context.jobs());
        Ok(())
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            jobs_migration("m0001_create_jobs", "demo"),
            adopt("m0002_tenancy", vec![tenant_table("demo")]),
        ]
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        vec![JobRegistration::new(Boom::KIND, Arc::new(Explodes))]
    }
}

#[tokio::test]
async fn job_hong_hien_failed_va_chay_lai_duoc() {
    TestDatabase::run(async |test| {
        let queue = Arc::new(Mutex::new(None));
        let app = TestApp::builder(test)
            .user("ops", 1, &["kernel.jobs.read", "kernel.jobs.manage"])
            .user("clerk", 2, &[])
            .module(Demo(queue.clone()))
            .build()
            .await
            .unwrap();
        let jobs = queue.lock().unwrap().clone().unwrap();
        let tenant = autoxide_kernel::tenancy::DatabaseRegistry::single(app.database().clone())
            .default_tenant();
        let transaction = tenant.database().begin_write().await.unwrap();
        let id = jobs
            .enqueue(&transaction, &tenant, &Boom, EnqueueOptions::default().max_attempts(1))
            .await
            .unwrap()
            .unwrap();
        autoxide_kernel::job::runs::track(
            &transaction,
            &tenant,
            "demo",
            id,
            Boom::KIND,
            "Demo job",
            Some(uuid::Uuid::from_u128(2)),
            None,
        )
        .await
        .unwrap();
        transaction.commit().await.unwrap();

        let worker = app.modules().worker(1).unwrap();
        while worker.run_once().await.unwrap() > 0 {}
        let failed =
            app.get(&format!("/api/v1/kernel/jobs/{id}")).token("clerk").send().await.unwrap();
        assert_eq!(failed.status(), StatusCode::OK, "người yêu cầu xem được: {}", failed.json());
        assert_eq!(failed.json()["data"]["status"], "failed");
        assert!(failed.json()["data"]["error"].as_str().unwrap().contains("partner"));

        let denied = app
            .post(&format!("/api/v1/kernel/jobs/{id}/retry"))
            .token("clerk")
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        let retried =
            app.post(&format!("/api/v1/kernel/jobs/{id}/retry")).token("ops").send().await.unwrap();
        assert_eq!(retried.status(), StatusCode::OK, "{}", retried.json());
        assert_eq!(retried.json()["data"]["status"], "queued");
        let again =
            app.post(&format!("/api/v1/kernel/jobs/{id}/retry")).token("ops").send().await.unwrap();
        assert_eq!(
            (again.status(), again.json()["code"].as_str()),
            (StatusCode::CONFLICT, Some("kernel.job_not_failed"))
        );
        let all =
            app.get("/api/v1/kernel/jobs?status=queued").token("ops").send().await.unwrap().json();
        assert_eq!(
            all["data"].as_array().unwrap().len(),
            1,
            "`kernel.jobs.read` thấy job của người khác"
        );
    })
    .await
    .unwrap();
}

// Danh sách job phân trang keyset trong database: nhiều dòng hơn `limit`, theo `next_cursor` tới hết không
// trùng / sót, cả hai chiều; người không có `kernel.jobs.read` chỉ thấy job của mình; lọc sai và tham số trang
// sai báo chung một lỗi 422.
#[tokio::test]
async fn danh_sach_job_phan_trang_theo_con_tro() {
    TestDatabase::run(async |test| {
        let queue = Arc::new(Mutex::new(None));
        let app = TestApp::builder(test)
            .user("ops", 1, &["kernel.jobs.read"])
            .user("clerk", 2, &[])
            .module(Demo(queue.clone()))
            .build()
            .await
            .unwrap();
        let jobs = queue.lock().unwrap().clone().unwrap();
        let tenant = autoxide_kernel::tenancy::DatabaseRegistry::single(app.database().clone())
            .default_tenant();
        let mut ids = Vec::new();
        for index in 0..5u128 {
            let transaction = tenant.database().begin_write().await.unwrap();
            let id = jobs
                .enqueue(&transaction, &tenant, &Boom, EnqueueOptions::default())
                .await
                .unwrap()
                .unwrap();
            // Hai job đầu do `clerk` (user 2) yêu cầu, còn lại của người khác.
            let by = if index < 2 { 2 } else { 3 };
            autoxide_kernel::job::runs::track(
                &transaction,
                &tenant,
                "demo",
                id,
                Boom::KIND,
                "Demo job",
                Some(uuid::Uuid::from_u128(by)),
                None,
            )
            .await
            .unwrap();
            transaction.commit().await.unwrap();
            ids.push(id.to_string());
        }
        let walk = async |order: &str| -> Vec<String> {
            let mut seen = Vec::new();
            let mut cursor: Option<String> = None;
            loop {
                let mut path = format!("/api/v1/kernel/jobs?limit=2&order={order}");
                if let Some(cursor) = &cursor {
                    path.push_str(&format!("&cursor={cursor}"));
                }
                let page = app.get(&path).token("ops").send().await.unwrap().json();
                assert_eq!(page["pagination"]["limit"], 2, "{page}");
                seen.extend(
                    page["data"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|job| job["id"].as_str().unwrap().to_owned()),
                );
                match page["pagination"]["next_cursor"].as_str() {
                    Some(next) => cursor = Some(next.to_owned()),
                    None => return seen,
                }
            }
        };
        assert_eq!(walk("asc").await, ids);
        let mut newest_first = ids.clone();
        newest_first.reverse();
        assert_eq!(walk("desc").await, newest_first);

        let counted = app
            .get("/api/v1/kernel/jobs?limit=1&count=true")
            .token("ops")
            .send()
            .await
            .unwrap()
            .json();
        assert_eq!(counted["pagination"]["total"], 5);
        let mine =
            app.get("/api/v1/kernel/jobs?count=true").token("clerk").send().await.unwrap().json();
        assert_eq!(mine["pagination"]["total"], 2, "không có `kernel.jobs.read`: chỉ job của mình");
        assert_eq!(mine["data"].as_array().unwrap().len(), 2);

        let wrong =
            app.get("/api/v1/kernel/jobs?status=done&limit=0").token("ops").send().await.unwrap();
        assert_eq!(wrong.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let fields: Vec<String> = wrong.json()["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|error| error["field"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(fields, ["status", "limit"]);
    })
    .await
    .unwrap();
}
