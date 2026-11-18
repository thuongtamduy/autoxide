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
