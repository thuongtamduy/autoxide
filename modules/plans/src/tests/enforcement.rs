//! Kernel cưỡng chế theo gói (bước 35 phần C, D-325): route, tool MCP, job, event của module mà gói
//! của khách không có bị chặn; khách chỉ đọc vì thuê bao: ghi bị chặn, đọc vẫn qua; quyền dùng đổi
//! có hiệu lực sau thời gian của bộ nhớ đệm.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, Event, EventHandler, EventInfo, Outbox, Subscription};
use autoxide_kernel::job::{
    EnqueueOptions, Job, JobContext, JobHandler, JobQueue, JobRegistration,
};
use autoxide_kernel::mcp::{NoInput, Tool};
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::response::Data;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_testkit::{TestApp, TestDatabase, tool};
use axum::extract::State;
use axum::http::StatusCode;
use serde::Serialize;
use serde_json::{Value, json};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

#[derive(Debug, Serialize)]
struct ItemAdded {}

impl Event for ItemAdded {
    const NAME: &'static str = "gated.item_added";
}

#[derive(Debug, Serialize)]
struct Work {}

impl Job for Work {
    const KIND: &'static str = "gated.work";
}

#[derive(Default)]
struct Counters {
    jobs: AtomicUsize,
    events: AtomicUsize,
}

#[derive(Clone)]
struct GatedState {
    outbox: Outbox,
    jobs: JobQueue,
}

/// Count the items.
#[utoipa::path(get, path = "/items", extensions(("x-permissions" = json!([]))), responses((status = 200, description = "The count.", body = Data<u32>)))]
async fn count_items(_tenant: TenantContext) -> Data<u32> {
    Data { data: 0 }
}

/// Add an item.
#[utoipa::path(post, path = "/items", extensions(("x-permissions" = json!([]))), responses((status = 200, description = "Added.", body = Data<u32>)))]
async fn add_item(
    State(state): State<GatedState>,
    tenant: TenantContext,
) -> Result<Data<u32>, Error> {
    let transaction = tenant.database().begin_write().await?;
    let actor = autoxide_kernel::event::Actor::system();
    state.outbox.record(&transaction, &tenant, actor, None, &ItemAdded {}).await?;
    state.jobs.enqueue(&transaction, &tenant, &Work {}, EnqueueOptions::default()).await?;
    transaction.commit().await?;
    Ok(Data { data: 1 })
}

struct CountJob(Arc<Counters>);

#[async_trait::async_trait]
impl JobHandler for CountJob {
    async fn run(&self, _job: &JobContext) -> Result<(), Error> {
        self.0.jobs.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct CountEvent(Arc<Counters>);

#[async_trait::async_trait]
impl EventHandler for CountEvent {
    async fn handle(&self, _event: &Delivered) -> Result<(), Error> {
        self.0.events.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// Module bị gói chi phối: route, tool đọc / ghi, job, event.
struct Gated {
    counters: Arc<Counters>,
    state: Option<GatedState>,
}

impl Module for Gated {
    fn name(&self) -> &'static str {
        "gated"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            autoxide_kernel::event::outbox_migration("m0001_create_events", "gated"),
            autoxide_kernel::event::outbox_actor_migration("m0002_add_event_actor", "gated"),
            autoxide_kernel::job::jobs_migration("m0003_create_jobs", "gated"),
            autoxide_kernel::tenancy::migration::adopt(
                "m0004_tenancy",
                vec![
                    autoxide_kernel::event::tenant_table("gated"),
                    autoxide_kernel::job::tenant_table("gated"),
                ],
            ),
            autoxide_kernel::event::outbox_access_migration("k0001_event_access", "gated"),
        ]
    }

    fn events(&self) -> Vec<EventInfo> {
        vec![EventInfo::new("gated.item_added", "An item was added.")]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        self.state = Some(GatedState { outbox: context.outbox()?, jobs: context.jobs() });
        Ok(())
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        vec![JobRegistration::new("gated.work", Arc::new(CountJob(self.counters.clone())))]
    }

    fn tools(&self) -> Vec<Tool> {
        vec![
            Tool::new("count", "Count the items.", |_: TenantContext, _, _: NoInput| async {
                Ok::<_, Error>(json!(0))
            })
            .for_any_signed_in()
            .read_only(),
            Tool::new("add", "Add an item.", |_: TenantContext, _, _: NoInput| async {
                Ok::<_, Error>(json!(1))
            })
            .for_any_signed_in(),
        ]
    }

    fn routes(&self) -> OpenApiRouter {
        self.state.clone().map_or_else(OpenApiRouter::new, |state| {
            OpenApiRouter::new().routes(routes!(count_items, add_item)).with_state(state)
        })
    }
}

/// Module chỉ nhận event của `gated`.
struct Watcher(Arc<Counters>);

impl Module for Watcher {
    fn name(&self) -> &'static str {
        "watcher"
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        vec![Subscription::new("gated.item_added", Arc::new(CountEvent(self.0.clone())))]
    }
}

const OPS: [&str; 2] = ["plans.plans.manage", "plans.subscriptions.manage"];

async fn app(test: &TestDatabase, cache: &str, counters: &Arc<Counters>) -> TestApp {
    TestApp::builder(test)
        .setting("AUTOXIDE_PLANS_CACHE_SECONDS", cache)
        .module(Gated { counters: counters.clone(), state: None })
        .module(Watcher(counters.clone()))
        .module(crate::module())
        .tenant("acme")
        .user("ops", 1, &OPS)
        .user_in("acme", "owner", 2, &[])
        .build()
        .await
        .unwrap()
}

async fn call(
    app: &TestApp,
    method: &str,
    uri: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = axum::http::Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1{uri}")).token(who);
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

/// Đặt khách `acme` vào gói bật đúng các module `modules`.
async fn put_on(app: &TestApp, code: &str, modules: &[&str]) {
    let body = json!({"code": code, "name": code, "status": "active", "modules": modules});
    let (status, plan) = call(app, "POST", "/plans/plans", "ops", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{plan}");
    let acme = app.tenant("acme").await.unwrap().id();
    let body = json!({"plan_id": plan["data"]["id"], "trial": false});
    let (status, sub) =
        call(app, "PUT", &format!("/plans/tenants/{acme}/subscription"), "ops", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{sub}");
}

#[tokio::test]
async fn route_module_chua_bat_tra_403_module_not_enabled() {
    TestDatabase::run(async |test| {
        let counters = Arc::new(Counters::default());
        let app = app(test, "0", &counters).await;
        // Chưa có gói nào: không chặn.
        assert_eq!(call(&app, "GET", "/gated/items", "owner", None).await.0, StatusCode::OK);
        put_on(&app, "basic", &[]).await;
        let (status, body) = call(&app, "GET", "/gated/items", "owner", None).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("module_not_enabled")),
            "{body}"
        );
        // Module nền và `plans` vẫn chạy.
        assert_eq!(call(&app, "GET", "/plans/entitlements", "owner", None).await.0, StatusCode::OK);
        put_on(&app, "full", &["gated", "watcher"]).await;
        assert_eq!(call(&app, "GET", "/gated/items", "owner", None).await.0, StatusCode::OK);
        // Tài liệu OpenAPI nói rõ 403 vì gói.
        let doc = serde_json::to_string(app.openapi()).unwrap();
        assert!(doc.contains("module_not_enabled"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tool_mcp_module_chua_bat_bi_an_va_bi_chan() {
    TestDatabase::run(async |test| {
        let counters = Arc::new(Counters::default());
        let app = app(test, "0", &counters).await;
        put_on(&app, "basic", &[]).await;
        let client = app.mcp("owner").await.unwrap();
        let listed = client.list_all_tools().await.unwrap();
        let names: Vec<&str> = listed.iter().map(|tool| tool.name.as_ref()).collect();
        assert!(
            !names.contains(&"gated.count") && names.contains(&"plans.get_entitlements"),
            "{names:?}"
        );
        let answer = client.call_tool(tool("gated.count", json!({}))).await.unwrap();
        assert_eq!(answer.is_error, Some(true));
        let text = serde_json::to_string(&answer.structured_content).unwrap();
        assert!(text.contains("module_not_enabled"), "{text}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn job_module_chua_bat_bi_hoan_khong_tinh_luot_thu() {
    TestDatabase::run(async |test| {
        let counters = Arc::new(Counters::default());
        let app = app(test, "0", &counters).await;
        put_on(&app, "full", &["gated"]).await;
        assert_eq!(
            call(&app, "POST", "/gated/items", "owner", Some(json!({}))).await.0,
            StatusCode::OK
        );
        put_on(&app, "basic", &[]).await;
        let worker = app.modules().worker(4).unwrap().blocked_delay(Duration::ZERO);
        worker.run_once().await.unwrap();
        assert_eq!(counters.jobs.load(Ordering::SeqCst), 0, "bị hoãn, không chạy");
        let failed =
            worker.list("gated", autoxide_kernel::job::JobStatus::Failed, 10).await.unwrap();
        assert!(failed.is_empty());
        // Bật lại: job chạy, không có lượt thử nào bị tính.
        put_on(&app, "again", &["gated"]).await;
        worker.run_once().await.unwrap();
        assert_eq!(counters.jobs.load(Ordering::SeqCst), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn event_cho_module_chua_bat_bi_bo_qua() {
    TestDatabase::run(async |test| {
        let counters = Arc::new(Counters::default());
        let app = app(test, "0", &counters).await;
        // `gated` bật, `watcher` không: event của `gated` không tới `watcher`.
        put_on(&app, "no_watcher", &["gated"]).await;
        assert_eq!(
            call(&app, "POST", "/gated/items", "owner", Some(json!({}))).await.0,
            StatusCode::OK
        );
        let dispatcher = app.modules().dispatcher().unwrap();
        while dispatcher.run_once().await.unwrap() > 0 {}
        assert_eq!(counters.events.load(Ordering::SeqCst), 0);
        // Bật `watcher`: event mới tới.
        put_on(&app, "with_watcher", &["gated", "watcher"]).await;
        assert_eq!(
            call(&app, "POST", "/gated/items", "owner", Some(json!({}))).await.0,
            StatusCode::OK
        );
        while dispatcher.run_once().await.unwrap() > 0 {}
        assert_eq!(counters.events.load(Ordering::SeqCst), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chi_doc_chan_ghi_cho_doc_va_tool_read_only_van_chay() {
    TestDatabase::run(async |test| {
        let counters = Arc::new(Counters::default());
        let app = app(test, "0", &counters).await;
        put_on(&app, "full", &["gated"]).await;
        // Thuê bao tạm khoá (vòng đời ở phần E): sửa thẳng trạng thái.
        {
            use sea_orm::{ConnectionTrait, Statement};
            let control = app.registry().control().write();
            let sql = "UPDATE plans_subscription SET status = 'suspended'";
            control
                .execute_raw(Statement::from_string(control.get_database_backend(), sql))
                .await
                .unwrap();
        }
        assert_eq!(call(&app, "GET", "/gated/items", "owner", None).await.0, StatusCode::OK);
        let (status, body) = call(&app, "POST", "/gated/items", "owner", Some(json!({}))).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("tenant_read_only")),
            "{body}"
        );
        let client = app.mcp("owner").await.unwrap();
        let read = client.call_tool(tool("gated.count", json!({}))).await.unwrap();
        assert_ne!(read.is_error, Some(true), "{read:?}");
        let write = client.call_tool(tool("gated.add", json!({}))).await.unwrap();
        assert_eq!(write.is_error, Some(true));
        // Đọc quyền dùng vẫn được (module `plans` luôn bật).
        let (status, now) = call(&app, "GET", "/plans/entitlements", "owner", None).await;
        assert_eq!((status, now["data"]["access"].as_str()), (StatusCode::OK, Some("read_only")));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doi_quyen_dung_co_hieu_luc_sau_cache() {
    TestDatabase::run(async |test| {
        let counters = Arc::new(Counters::default());
        let app = app(test, "1", &counters).await;
        put_on(&app, "full", &["gated"]).await;
        assert_eq!(call(&app, "GET", "/gated/items", "owner", None).await.0, StatusCode::OK);
        // Máy chủ khác tắt module (sửa thẳng database, máy chủ này không biết).
        {
            use sea_orm::{ConnectionTrait, Statement};
            let control = app.registry().control().write();
            let sql = "DELETE FROM plans_plan_module";
            control
                .execute_raw(Statement::from_string(control.get_database_backend(), sql))
                .await
                .unwrap();
        }
        assert_eq!(
            call(&app, "GET", "/gated/items", "owner", None).await.0,
            StatusCode::OK,
            "còn trong bộ nhớ đệm"
        );
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert_eq!(call(&app, "GET", "/gated/items", "owner", None).await.0, StatusCode::FORBIDDEN);
    })
    .await
    .unwrap();
}
