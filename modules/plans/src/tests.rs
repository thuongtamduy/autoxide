//! Test của module `plans` qua REST, MCP và tay cầm quyền dùng của kernel, trên database gate đang
//! chọn (cả SQLite và Postgres): gói là dữ liệu, gói mặc định duy nhất, gán gói, ghi đè, quyền dùng,
//! hạn mức giữ chỗ / trả lại đồng thời, lượng dùng theo kỳ, không rò rỉ giữa khách.

mod billing;
mod enforcement;
mod invoicing;
mod lifecycle;
mod provisioning;
mod signup;
mod store;

use std::sync::Arc;

use autoxide_kernel::entitlement::{EntitlementsHandle, Feature, Limit, Period};
use autoxide_kernel::module::Module;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, TenantId};
use autoxide_testkit::{TestApp, TestDatabase, tool};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

const ITEMS: Limit = Limit::count("demo.items", "Items kept.");
const CALLS: Limit = Limit::metered("demo.calls", "Calls each month.", Period::Month);
const EXPORT: Feature = Feature::new("demo.export", "Export items.");

/// Module thử: một tính năng, hai hạn mức; `demo.items` có bộ đếm thật (giá trị trong `real`).
#[derive(Default)]
struct Demo {
    real: Arc<std::sync::atomic::AtomicI64>,
    /// Có thì lần đếm đầu tiên dừng giữa chừng (test chen lệnh vào lúc đối soát đang đếm).
    gate: Option<Arc<CountGate>>,
}

/// Chặn một lần đếm: báo `entered`, chờ `release`.
#[derive(Default)]
struct CountGate {
    armed: std::sync::atomic::AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

struct RealCount(Arc<std::sync::atomic::AtomicI64>, Option<Arc<CountGate>>);

#[async_trait::async_trait]
impl autoxide_kernel::entitlement::UsageCounter for RealCount {
    async fn count(
        &self,
        _tenant: &autoxide_kernel::tenancy::TenantContext,
    ) -> Result<i64, autoxide_kernel::error::Error> {
        let value = self.0.load(std::sync::atomic::Ordering::SeqCst);
        if let Some(gate) = &self.1
            && gate.armed.swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            gate.entered.notify_one();
            gate.release.notified().await;
        }
        Ok(value)
    }
}

impl Module for Demo {
    fn name(&self) -> &'static str {
        "demo"
    }

    fn start(
        &mut self,
        context: &mut autoxide_kernel::module::StartContext<'_>,
    ) -> Result<(), autoxide_kernel::error::Error> {
        context.usage_counter(&ITEMS, Arc::new(RealCount(self.real.clone(), self.gate.clone())))
    }

    fn features(&self) -> Vec<Feature> {
        vec![EXPORT]
    }

    fn limits(&self) -> Vec<Limit> {
        vec![ITEMS, CALLS]
    }
}

const OPS: [&str; 4] = [
    "plans.plans.read",
    "plans.plans.manage",
    "plans.subscriptions.read",
    "plans.subscriptions.manage",
];

async fn app(test: &TestDatabase) -> TestApp {
    app_with(test, Demo::default()).await
}

async fn app_with(test: &TestDatabase, demo: Demo) -> TestApp {
    TestApp::builder(test)
        .setting("AUTOXIDE_PLANS_CACHE_SECONDS", "0")
        .module(demo)
        .module(crate::module())
        .tenant("acme")
        .tenant("beta")
        .user("ops", 1, &OPS)
        .user_in("acme", "owner", 2, &["plans.billing.read"])
        .user_in("beta", "other", 3, &["plans.billing.read"])
        // Có đủ permission vận hành nhưng ở khách thường: vẫn bị từ chối.
        .user_in("beta", "pretender", 4, &OPS)
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
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1/plans{uri}")).token(who);
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

async fn expect(
    app: &TestApp,
    method: &str,
    uri: &str,
    who: &str,
    body: Option<Value>,
    status: StatusCode,
) -> Value {
    let (got, body) = call(app, method, uri, who, body).await;
    assert_eq!(got, status, "{method} {uri}: {body}");
    body
}

async fn tenant(app: &TestApp, slug: &str) -> TenantId {
    app.tenant(slug).await.unwrap().id()
}

fn handle(app: &TestApp) -> EntitlementsHandle {
    app.modules().entitlements()
}

/// Tạo gói `code` (chính, đang bán) với hạn mức `items` cho `demo.items`.
async fn plan(app: &TestApp, code: &str, items: Option<i64>, extra: Value) -> String {
    let mut body = json!({
        "code": code,
        "name": code,
        "status": "active",
        "modules": ["demo"],
        "features": ["demo.export"],
        "limits": {"demo.items": items, "demo.calls": 100},
    });
    if let (Some(body), Some(extra)) = (body.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            body.insert(key.clone(), value.clone());
        }
    }
    let created = expect(app, "POST", "/plans", "ops", Some(body), StatusCode::CREATED).await;
    created["data"]["id"].as_str().unwrap().to_owned()
}

async fn assign(app: &TestApp, tenant: TenantId, plan_id: &str) -> Value {
    let body = json!({"plan_id": plan_id, "trial": false});
    expect(
        app,
        "PUT",
        &format!("/tenants/{tenant}/subscription"),
        "ops",
        Some(body),
        StatusCode::OK,
    )
    .await
}

#[tokio::test]
async fn goi_la_du_lieu_kiem_khai_bao_va_chi_nhan_su_nen_tang() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let bad = json!({
            "code": "Bad Code",
            "name": "",
            "modules": ["nope"],
            "features": ["demo.nope"],
            "limits": {"demo.nope": 1, "demo.items": -1},
        });
        let body = expect(&app, "POST", "/plans", "ops", Some(bad), StatusCode::UNPROCESSABLE_ENTITY).await;
        let mut fields: Vec<&str> =
            body["errors"].as_array().unwrap().iter().map(|e| e["field"].as_str().unwrap()).collect();
        fields.sort_unstable();
        assert_eq!(
            fields,
            ["code", "features[0]", "limits.demo.items", "limits.demo.nope", "modules[0]", "name"]
        );

        let id = plan(&app, "pro", Some(10), json!({"description": "Pro."})).await;
        let body = json!({"code": "pro", "name": "Again"});
        let taken = expect(&app, "POST", "/plans", "ops", Some(body), StatusCode::CONFLICT).await;
        assert_eq!(taken["code"], "plans.plan_code_taken");
        let got = expect(&app, "GET", &format!("/plans/{id}"), "ops", None, StatusCode::OK).await;
        assert_eq!(got["data"]["limits"], json!({"demo.items": 10, "demo.calls": 100}));
        assert_eq!(got["data"]["grace_days"], 7);

        // Thêm giá; danh mục công khai chỉ có gói đang bán.
        let price = json!({"kind": "recurring", "interval": "month", "amount": {"amount": "199000", "currency": "VND"}});
        expect(&app, "POST", &format!("/plans/{id}/prices"), "ops", Some(price), StatusCode::CREATED).await;
        plan(&app, "draft", None, json!({"status": "draft"})).await;
        let catalog = app.get("/api/v1/plans/catalog").send().await.unwrap();
        let codes: Vec<String> = catalog.json()["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|plan| plan["code"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(codes, ["pro"]);
        assert_eq!(catalog.json()["data"][0]["prices"][0]["amount"]["amount"], "199000");

        // Người của khách thường: dù có permission cũng bị từ chối.
        let denied = expect(&app, "GET", "/plans", "pretender", None, StatusCode::FORBIDDEN).await;
        assert_eq!(denied["code"], "tenant_platform_only");

        let retired = expect(&app, "POST", &format!("/plans/{id}/retire"), "ops", None, StatusCode::OK).await;
        assert_eq!(retired["data"]["status"], "retired");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn goi_mac_dinh_chi_mot_du_dat_dong_thoi() {
    TestDatabase::run(async |test| {
        let app = Arc::new(app(test).await);
        let tasks: Vec<_> = (0..6)
            .map(|index| {
                let app = app.clone();
                tokio::spawn(async move {
                    plan(&app, &format!("p{index}"), Some(1), json!({"is_default": true})).await
                })
            })
            .collect();
        for task in tasks {
            task.await.unwrap();
        }
        let page = expect(&app, "GET", "/plans?limit=50", "ops", None, StatusCode::OK).await;
        let defaults =
            page["data"].as_array().unwrap().iter().filter(|p| p["is_default"] == true).count();
        assert_eq!(defaults, 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khong_thue_bao_khong_goi_mac_dinh_thi_khong_gioi_han_roi_goi_mac_dinh_ap_dung() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let acme = tenant(&app, "acme").await;
        let open = expect(&app, "GET", "/entitlements", "owner", None, StatusCode::OK).await;
        assert_eq!(open["data"]["unrestricted"], true);
        assert!(handle(&app).module_enabled(acme, "demo").await.unwrap());

        plan(&app, "free", Some(2), json!({"is_default": true, "modules": [], "features": []}))
            .await;
        let limited = expect(&app, "GET", "/entitlements", "owner", None, StatusCode::OK).await;
        assert_eq!(limited["data"]["unrestricted"], false);
        assert!(!handle(&app).module_enabled(acme, "demo").await.unwrap());
        // Module nền luôn bật dù gói không nêu.
        assert!(handle(&app).module_enabled(acme, "plans").await.unwrap());
        let modules = limited["data"]["modules"].as_array().unwrap();
        assert!(modules.contains(&json!("plans")) && !modules.contains(&json!("demo")));
        // Khách nền tảng không bao giờ bị giới hạn.
        assert!(handle(&app).module_enabled(DEFAULT_TENANT, "demo").await.unwrap());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gan_goi_ghi_de_va_quyen_dung_doi_ngay() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let acme = tenant(&app, "acme").await;
        let pro = plan(&app, "pro", Some(5), json!({"trial_days": 14})).await;
        let addon =
            plan(&app, "more", Some(3), json!({"kind": "addon", "modules": [], "features": []}))
                .await;

        let body = json!({"plan_id": pro, "addons": [{"plan_id": addon, "quantity": 2}]});
        let sub = expect(
            &app,
            "PUT",
            &format!("/tenants/{acme}/subscription"),
            "ops",
            Some(body),
            StatusCode::OK,
        )
        .await;
        assert_eq!(sub["data"]["status"], "trialing", "{sub}");
        assert_eq!(sub["data"]["trial_used"], true);
        let usage = expect(&app, "GET", "/usage", "owner", None, StatusCode::OK).await;
        let items = usage["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["key"] == "demo.items")
            .unwrap()
            .clone();
        assert_eq!(items["max"], 11, "5 + 3 × 2");
        assert!(handle(&app).has_feature(acme, &EXPORT).await.unwrap());

        // Ghi đè: tắt tính năng, đổi hạn mức — có hiệu lực ngay (bộ nhớ đệm 0 giây).
        let off =
            json!({"kind": "feature", "key": "demo.export", "enabled": false, "reason": "Abuse"});
        expect(
            &app,
            "POST",
            &format!("/tenants/{acme}/overrides"),
            "ops",
            Some(off),
            StatusCode::CREATED,
        )
        .await;
        let cap =
            json!({"kind": "limit", "key": "demo.items", "max_value": 1, "reason": "Trial cap"});
        let cap = expect(
            &app,
            "POST",
            &format!("/tenants/{acme}/overrides"),
            "ops",
            Some(cap),
            StatusCode::CREATED,
        )
        .await;
        let error = handle(&app).require_feature(acme, &EXPORT).await.unwrap_err();
        assert_eq!((error.code(), error.kind().status()), ("feature_not_enabled", 403));
        let now = expect(&app, "GET", "/entitlements", "owner", None, StatusCode::OK).await;
        let items = now["data"]["limits"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["key"] == "demo.items")
            .unwrap()
            .clone();
        assert_eq!(items["max"], 1);
        let cap_id = cap["data"]["id"].as_str().unwrap();
        expect(
            &app,
            "DELETE",
            &format!("/tenants/{acme}/overrides/{cap_id}"),
            "ops",
            None,
            StatusCode::NO_CONTENT,
        )
        .await;
        let list =
            expect(&app, "GET", &format!("/tenants/{acme}/overrides"), "ops", None, StatusCode::OK)
                .await;
        assert_eq!(list["data"].as_array().unwrap().len(), 1);

        // Khách nền tảng không có thuê bao.
        let body = json!({"plan_id": pro});
        let exempt = expect(
            &app,
            "PUT",
            &format!("/tenants/{DEFAULT_TENANT}/subscription"),
            "ops",
            Some(body),
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
        assert_eq!(exempt["code"], "plans.platform_exempt");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khong_ro_ri_giua_khach() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let acme = tenant(&app, "acme").await;
        let pro = plan(&app, "pro", Some(5), json!({})).await;
        assign(&app, acme, &pro).await;
        // Khách beta chỉ thấy thuê bao của mình (chưa có).
        let none = expect(&app, "GET", "/subscription", "other", None, StatusCode::NOT_FOUND).await;
        assert_eq!(none["code"], "plans.subscription_not_found");
        let mine = expect(&app, "GET", "/subscription", "owner", None, StatusCode::OK).await;
        assert_eq!(mine["data"]["tenant_id"], acme.to_string());
        for uri in [
            format!("/tenants/{acme}/subscription"),
            format!("/tenants/{acme}/usage"),
            format!("/tenants/{acme}/overrides"),
            "/subscriptions".to_owned(),
        ] {
            let denied = expect(&app, "GET", &uri, "pretender", None, StatusCode::FORBIDDEN).await;
            assert_eq!(denied["code"], "tenant_platform_only", "{uri}");
        }
        // Tool MCP đi cùng đường.
        let client = app.mcp("pretender").await.unwrap();
        let answer = client
            .call_tool(tool("plans.get_subscription", json!({"tenant_id": acme})))
            .await
            .unwrap();
        assert_eq!(answer.is_error, Some(true));
    })
    .await
    .unwrap();
}

/// Một khách có gói giới hạn `demo.items` ở `max`.
async fn limited(app: &TestApp, max: i64) -> TenantId {
    let acme = tenant(app, "acme").await;
    let pro = plan(app, "pro", Some(max), json!({})).await;
    assign(app, acme, &pro).await;
    acme
}

#[tokio::test]
async fn giu_cho_dong_thoi_khong_vuot_han_muc() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let acme = limited(&app, 33).await;
        let handle = handle(&app);
        let tasks: Vec<_> = (0..40)
            .map(|index| {
                let handle = handle.clone();
                tokio::spawn(async move {
                    handle.reserve(acme, &ITEMS, 1, &format!("demo.item:{index}")).await
                })
            })
            .collect();
        let mut ok = 0;
        for task in tasks {
            match task.await.unwrap() {
                Ok(()) => ok += 1,
                Err(error) => assert_eq!(error.code(), "limit_exceeded"),
            }
        }
        assert_eq!(ok, 33);
        let usage = expect(&app, "GET", "/usage", "owner", None, StatusCode::OK).await;
        let items = usage["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["key"] == "demo.items")
            .unwrap()
            .clone();
        assert_eq!(items["used"], 33);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn giu_cho_cung_reference_dong_thoi_chi_mot_lan() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let acme = limited(&app, 100).await;
        let handle = handle(&app);
        let tasks: Vec<_> = (0..16)
            .map(|_| {
                let handle = handle.clone();
                tokio::spawn(async move { handle.reserve(acme, &ITEMS, 5, "demo.item:same").await })
            })
            .collect();
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        let usage = expect(&app, "GET", "/usage", "owner", None, StatusCode::OK).await;
        let items = usage["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["key"] == "demo.items")
            .unwrap()
            .clone();
        assert_eq!(items["used"], 5);
        let reused = handle.reserve(acme, &ITEMS, 6, "demo.item:same").await.unwrap_err();
        assert_eq!(reused.code(), "reference_reused");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tra_lai_dong_thoi_chi_tru_mot_lan() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let acme = limited(&app, 100).await;
        let handle = handle(&app);
        for index in 0..3 {
            handle.reserve(acme, &ITEMS, 2, &format!("demo.item:{index}")).await.unwrap();
            handle.confirm(acme, &ITEMS, &format!("demo.item:{index}")).await.unwrap();
        }
        let tasks: Vec<_> = (0..12)
            .map(|_| {
                let handle = handle.clone();
                tokio::spawn(async move { handle.release(acme, &ITEMS, "demo.item:0").await })
            })
            .collect();
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        handle.release(acme, &ITEMS, "demo.item:unknown").await.unwrap();
        let usage = expect(&app, "GET", "/usage", "owner", None, StatusCode::OK).await;
        let items = usage["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["key"] == "demo.items")
            .unwrap()
            .clone();
        assert_eq!(items["used"], 4);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn giu_va_tra_xen_ke_tong_khong_lech() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let acme = limited(&app, 1_000).await;
        let handle = handle(&app);
        // 200 thao tác, giả ngẫu nhiên nhưng lặp lại được: 8 luồng, mỗi luồng giữ / trả trên 10 mã.
        let tasks: Vec<_> = (0..8u64)
            .map(|worker| {
                let handle = handle.clone();
                tokio::spawn(async move {
                    let mut seed = worker * 7919 + 13;
                    for _ in 0..25 {
                        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                        let reference = format!("demo.item:{}", (seed >> 33) % 10);
                        let amount = i64::try_from((seed >> 40) % 3 + 1).unwrap();
                        if (seed >> 20) % 2 == 0 {
                            // Cùng mã mà khác lượng thì bị từ chối: hợp lệ.
                            let _ = handle.reserve(acme, &ITEMS, amount, &reference).await;
                        } else {
                            handle.release(acme, &ITEMS, &reference).await.unwrap();
                        }
                    }
                })
            })
            .collect();
        for task in tasks {
            task.await.unwrap();
        }
        let held: i64 = {
            use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
            crate::entity::reservation::Entity::find()
                .filter(crate::entity::reservation::Column::TenantId.eq(acme.as_uuid()))
                .filter(crate::entity::reservation::Column::Status.ne("released"))
                .all(app.registry().control().read())
                .await
                .unwrap()
                .iter()
                .map(|row| row.amount)
                .sum()
        };
        let usage = expect(&app, "GET", "/usage", "owner", None, StatusCode::OK).await;
        let items = usage["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["key"] == "demo.items")
            .unwrap()
            .clone();
        assert_eq!(items["used"], held);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ghi_luong_dung_trung_reference_dong_thoi_va_vuot_quota_tra_429() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let acme = limited(&app, 10).await;
        let handle = handle(&app);
        let tasks: Vec<_> = (0..16)
            .map(|_| {
                let handle = handle.clone();
                tokio::spawn(async move {
                    handle.record_usage(acme, &CALLS, 30, "node-1:minute-1").await
                })
            })
            .collect();
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        handle.record_usage(acme, &CALLS, 60, "node-1:minute-2").await.unwrap();
        // 90 / 100: lượt 20 nữa vượt quota của tháng.
        let error = handle.record_usage(acme, &CALLS, 20, "node-1:minute-3").await.unwrap_err();
        assert_eq!((error.code(), error.kind().status()), ("quota_exhausted", 429));
        let usage = expect(&app, "GET", "/usage", "owner", None, StatusCode::OK).await;
        let calls = usage["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["key"] == "demo.calls")
            .unwrap()
            .clone();
        assert_eq!(calls["used"], 90);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ha_goi_qua_han_muc_chi_chan_moi() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let acme = limited(&app, 10).await;
        let handle = handle(&app);
        for index in 0..5 {
            handle.reserve(acme, &ITEMS, 1, &format!("demo.item:{index}")).await.unwrap();
        }
        let small = plan(&app, "small", Some(3), json!({})).await;
        assign(&app, acme, &small).await;
        let now = expect(&app, "GET", "/entitlements", "owner", None, StatusCode::OK).await;
        let items = now["data"]["limits"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["key"] == "demo.items")
            .unwrap()
            .clone();
        assert_eq!(
            (items["used"].clone(), items["max"].clone(), items["over_limit"].clone()),
            (json!(5), json!(3), json!(true))
        );
        let error = handle.reserve(acme, &ITEMS, 1, "demo.item:new").await.unwrap_err();
        assert_eq!(error.code(), "limit_exceeded");
        // Không xoá gì; trả lại vẫn được.
        handle.release(acme, &ITEMS, "demo.item:0").await.unwrap();
        let used = expect(&app, "GET", "/usage", "owner", None, StatusCode::OK).await;
        let items = used["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["key"] == "demo.items")
            .unwrap()
            .clone();
        assert_eq!(items["used"], 4);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tool_quyen_dung_cho_moi_nguoi_da_dang_nhap() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let client = app.mcp("owner").await.unwrap();
        let answer = client.call_tool(tool("plans.get_entitlements", json!({}))).await.unwrap();
        assert_ne!(answer.is_error, Some(true), "{answer:?}");
        let text = serde_json::to_string(&answer.structured_content).unwrap();
        assert!(text.contains("\"unrestricted\":true"), "{text}");
    })
    .await
    .unwrap();
}

/// Sửa thẳng database điều khiển (giả lập thời gian trôi qua / máy chủ khác).
async fn sql(app: &TestApp, text: &str) {
    use sea_orm::{ConnectionTrait, Statement};
    let control = app.registry().control().write();
    control
        .execute_raw(Statement::from_string(control.get_database_backend(), text))
        .await
        .unwrap();
}

async fn used(app: &TestApp, key: &str) -> Value {
    let usage = expect(app, "GET", "/usage", "owner", None, StatusCode::OK).await;
    usage["data"].as_array().unwrap().iter().find(|l| l["key"] == key).unwrap()["used"].clone()
}

#[tokio::test]
async fn giu_cho_het_han_duoc_tra() {
    TestDatabase::run(async |test| {
        // Bản ghi đã xác nhận có thật (bộ đếm nói 2); job đối soát chạy cùng lượt cũng không lệch.
        let demo = Demo::default();
        demo.real.store(2, std::sync::atomic::Ordering::SeqCst);
        let app = app_with(test, demo).await;
        let acme = limited(&app, 10).await;
        let handle = handle(&app);
        handle.reserve(acme, &ITEMS, 3, "demo.item:lost").await.unwrap();
        handle.reserve(acme, &ITEMS, 2, "demo.item:kept").await.unwrap();
        handle.confirm(acme, &ITEMS, "demo.item:kept").await.unwrap();
        assert_eq!(used(&app, "demo.items").await, 5);
        // Mọi chỗ giữ coi như quá hạn; chỗ đã xác nhận không bị trả.
        sql(&app, "UPDATE plans_reservation SET expires_at = '2000-01-01T00:00:00Z'").await;
        // Chạy tới khi hết job: đối soát tính cả chỗ giữ quá hạn (job trả chỗ trừ lại sau).
        let worker = app.modules().worker(4).unwrap();
        for _ in 0..3 {
            if worker.run_once().await.unwrap() == 0 {
                break;
            }
        }
        assert_eq!(used(&app, "demo.items").await, 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doi_soat_sua_so_lech() {
    TestDatabase::run(async |test| {
        let demo = Demo::default();
        let real = demo.real.clone();
        let app = app_with(test, demo).await;
        let acme = limited(&app, 100).await;
        handle(&app).reserve(acme, &ITEMS, 4, "demo.item:pending").await.unwrap();
        // Bộ đếm thật nói đang có 7 bản ghi (vd khách có dữ liệu từ trước khi vào gói).
        real.store(7, std::sync::atomic::Ordering::SeqCst);
        app.modules().worker(4).unwrap().run_once().await.unwrap();
        assert_eq!(used(&app, "demo.items").await, 11, "7 thật + 4 đang giữ");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doi_soat_khong_ghi_de_lan_giu_cho_chen_giua() {
    TestDatabase::run(async |test| {
        let gate = Arc::new(CountGate::default());
        gate.armed.store(true, std::sync::atomic::Ordering::SeqCst);
        let demo = Demo { gate: Some(gate.clone()), ..Demo::default() };
        demo.real.store(5, std::sync::atomic::Ordering::SeqCst);
        let app = app_with(test, demo).await;
        let acme = limited(&app, 100).await;
        let handle = handle(&app);
        handle.reserve(acme, &ITEMS, 2, "demo.item:a").await.unwrap();
        let worker = app.modules().worker(4).unwrap();
        // Đối soát đang đếm số thật thì có thêm một lần giữ chỗ. Trước đây đối soát ghi tuyệt đối
        // `used = thật + chỗ giữ đã đọc trước khi đếm`: lần giữ chen giữa bị xoá khỏi sổ (cả hai DB).
        let run = async {
            worker.run_once().await.unwrap();
        };
        let poke = async {
            gate.entered.notified().await;
            handle.reserve(acme, &ITEMS, 3, "demo.item:b").await.unwrap();
            gate.release.notify_one();
        };
        tokio::join!(run, poke);
        assert_eq!(used(&app, "demo.items").await, 10, "5 thật + 2 + 3 đang giữ");
    })
    .await
    .unwrap();
}
