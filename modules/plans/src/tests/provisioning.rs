//! Cấp phát khách mới theo gói (bước 35 mục 5, D-338): tạo khách → gán gói → dữ liệu mẫu của module
//! gói bật → mời quản trị viên; bước hỏng thì `failed`, chạy lại tiếp từ bước đã ghi; hai lượt cùng
//! slug không cùng nhận một khách. Cả SQLite và Postgres.

use autoxide_kernel::time::Timestamp;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use autoxide_auth::api::{InvitationSummary, Invitations, NewInvitation};
use autoxide_kernel::error::Error;
use autoxide_kernel::module::Module;
use autoxide_kernel::tenancy::{TenantContext, TenantId, TenantSeed};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use super::lifecycle::{call, ok};

/// Lời mời giả: ghi lại (khách, email, role); `fail` bật thì từ chối một lần.
#[derive(Default)]
pub(super) struct FakeInvitations {
    pub(super) sent: Mutex<Vec<(TenantId, String, Vec<String>)>>,
    fail: AtomicBool,
}

#[async_trait::async_trait]
impl Invitations for FakeInvitations {
    async fn invite(
        &self,
        tenant: &TenantContext,
        invitation: NewInvitation,
    ) -> Result<InvitationSummary, Error> {
        if self.fail.swap(false, Ordering::SeqCst) {
            return Err(Error::unavailable(
                "plans.invitations_unavailable",
                "The mail server is down.",
            ));
        }
        self.sent.lock().unwrap().push((
            tenant.id(),
            invitation.email.clone(),
            invitation.roles.clone(),
        ));
        Ok(InvitationSummary::new(uuid::Uuid::now_v7(), invitation.email, Timestamp::now()))
    }
}

/// Module thử có dữ liệu mẫu: ghi lại khách được gieo.
struct Sample(Arc<Mutex<Vec<TenantId>>>);

struct SampleSeed(Arc<Mutex<Vec<TenantId>>>);

#[async_trait::async_trait]
impl TenantSeed for SampleSeed {
    async fn seed(&self, tenant: &TenantContext) -> Result<(), Error> {
        let mut seeded = self.0.lock().unwrap();
        if !seeded.contains(&tenant.id()) {
            seeded.push(tenant.id());
        }
        Ok(())
    }
}

impl Module for Sample {
    fn name(&self) -> &'static str {
        "sample"
    }

    fn sample_data(&self) -> Option<Arc<dyn TenantSeed>> {
        Some(Arc::new(SampleSeed(self.0.clone())))
    }
}

const OPS: [&str; 4] =
    ["plans.plans.manage", "plans.subscriptions.read", "plans.provision", "kernel.tenants.read"];

struct Setup {
    app: TestApp,
    invitations: Arc<FakeInvitations>,
    seeded: Arc<Mutex<Vec<TenantId>>>,
}

async fn setup(test: &TestDatabase) -> Setup {
    let invitations = Arc::new(FakeInvitations::default());
    let seeded = Arc::new(Mutex::new(Vec::new()));
    let dyn_invitations: Arc<dyn Invitations> = invitations.clone();
    let app = TestApp::builder(test)
        .setting("AUTOXIDE_PLANS_CACHE_SECONDS", "0")
        .provide::<dyn Invitations>(dyn_invitations)
        .module(Sample(seeded.clone()))
        .module(crate::module())
        .tenant("beta")
        .user("ops", 1, &OPS)
        .user_in("beta", "pretender", 4, &OPS)
        .build()
        .await
        .unwrap();
    Setup { app, invitations, seeded }
}

/// Gói đang bán `code`, bật module `sample`.
async fn plan(app: &TestApp, code: &str, extra: Value) -> String {
    let mut body = json!({"code": code, "name": code, "status": "active", "modules": ["sample"], "sample_data": true});
    if let (Some(body), Some(extra)) = (body.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            body.insert(key.clone(), value.clone());
        }
    }
    let created = ok(app, Method::POST, "/plans/plans", "ops", Some(body)).await;
    created["data"]["id"].as_str().unwrap().to_owned()
}

/// Chạy worker tới khi hết job.
async fn drain(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap();
    for _ in 0..10 {
        if worker.run_once().await.unwrap() == 0 {
            break;
        }
    }
}

fn request(slug: &str, plan: &str) -> Value {
    json!({"slug": slug, "name": "Acme Ltd", "plan_id": plan, "admin_email": "owner@acme.example", "admin_name": "Owner"})
}

#[tokio::test]
async fn cap_phat_tao_khach_gan_goi_du_lieu_mau_va_moi_quan_tri() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let app = &setup.app;
        let pro = plan(app, "pro", json!({})).await;
        let (status, created) =
            call(app, Method::POST, "/plans/provisionings", "ops", Some(request("acme", &pro)))
                .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        assert_eq!(created["data"]["status"], "pending");
        let id = created["data"]["id"].as_str().unwrap().to_owned();
        drain(app).await;

        let got = ok(app, Method::GET, &format!("/plans/provisionings/{id}"), "ops", None).await;
        assert_eq!(
            (got["data"]["status"].as_str(), got["data"]["step"].as_str()),
            (Some("done"), Some("done")),
            "{got}"
        );
        let steps: Vec<&str> = got["data"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|step| step["step"].as_str().unwrap())
            .collect();
        assert_eq!(steps, ["created", "tenant_created", "subscribed", "seeded", "invited", "done"]);
        let tenant = got["data"]["tenant_id"].as_str().unwrap().to_owned();
        let row = ok(app, Method::GET, &format!("/kernel/tenants/{tenant}"), "ops", None).await;
        assert_eq!(
            (row["data"]["slug"].as_str(), row["data"]["status"].as_str()),
            (Some("acme"), Some("active"))
        );
        let sub =
            ok(app, Method::GET, &format!("/plans/tenants/{tenant}/subscription"), "ops", None)
                .await;
        assert_eq!(sub["data"]["plan_id"], pro.as_str());
        let tenant = TenantId::from_uuid(tenant.parse().unwrap());
        assert_eq!(*setup.seeded.lock().unwrap(), [tenant]);
        let sent = setup.invitations.sent.lock().unwrap().clone();
        assert_eq!(sent, [(tenant, "owner@acme.example".to_owned(), vec!["admin".to_owned()])]);

        // Danh sách theo trạng thái; slug đã dùng thì không cấp phát lần nữa.
        let done = ok(app, Method::GET, "/plans/provisionings?status=done", "ops", None).await;
        assert_eq!(done["data"].as_array().unwrap().len(), 1);
        let (status, taken) =
            call(app, Method::POST, "/plans/provisionings", "ops", Some(request("acme", &pro)))
                .await;
        assert_eq!(
            (status, taken["code"].as_str()),
            (StatusCode::CONFLICT, Some("tenant_slug_taken"))
        );
        let (status, again) =
            call(app, Method::POST, &format!("/plans/provisionings/{id}/retry"), "ops", None).await;
        assert_eq!(
            (status, again["code"].as_str()),
            (StatusCode::CONFLICT, Some("plans.provisioning_not_retryable"))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cap_phat_kiem_dau_vao_va_chi_cho_nhan_su_nen_tang() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let app = &setup.app;
        let pro = plan(app, "pro", json!({})).await;
        let bad = json!({"slug": "Bad Slug", "name": "", "plan_id": pro, "admin_email": "nope"});
        let (status, body) =
            call(app, Method::POST, "/plans/provisionings", "ops", Some(bad)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        let mut fields: Vec<&str> = body["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["field"].as_str().unwrap())
            .collect();
        fields.sort_unstable();
        assert_eq!(fields, ["admin_email", "name", "slug"]);
        // Gói `dedicated` phải nêu placement; gói cần KYC chờ bước 37.
        let dedicated = plan(app, "big", json!({"placement": "dedicated"})).await;
        let (status, body) = call(
            app,
            Method::POST,
            "/plans/provisionings",
            "ops",
            Some(request("big", &dedicated)),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["errors"][0]["field"], "placement");
        let kyc = plan(app, "kyc", json!({"requires_kyc": true})).await;
        let (status, body) =
            call(app, Method::POST, "/plans/provisionings", "ops", Some(request("kyc", &kyc)))
                .await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::CONFLICT, Some("plans.kyc_required"))
        );
        let (status, body) =
            call(app, Method::POST, "/plans/provisionings", "pretender", Some(request("x", &pro)))
                .await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("tenant_platform_only"))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cap_phat_dung_giua_chung_chay_lai_tiep_buoc() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let app = &setup.app;
        let pro = plan(app, "pro", json!({})).await;
        setup.invitations.fail.store(true, Ordering::SeqCst);
        let created =
            ok(app, Method::POST, "/plans/provisionings", "ops", Some(request("acme", &pro))).await;
        let id = created["data"]["id"].as_str().unwrap().to_owned();
        drain(app).await;
        let failed = ok(app, Method::GET, &format!("/plans/provisionings/{id}"), "ops", None).await;
        assert_eq!(
            (
                failed["data"]["status"].as_str(),
                failed["data"]["step"].as_str(),
                failed["data"]["error_code"].as_str()
            ),
            (Some("failed"), Some("seeded"), Some("plans.invitations_unavailable")),
            "{failed}"
        );
        let tenant = failed["data"]["tenant_id"].as_str().unwrap().to_owned();
        ok(app, Method::POST, &format!("/plans/provisionings/{id}/retry"), "ops", None).await;
        drain(app).await;
        let done = ok(app, Method::GET, &format!("/plans/provisionings/{id}"), "ops", None).await;
        assert_eq!(done["data"]["status"], "done", "{done}");
        // Chạy tiếp, không làm lại: cùng khách, một lời mời.
        assert_eq!(done["data"]["tenant_id"].as_str(), Some(tenant.as_str()));
        assert_eq!(setup.invitations.sent.lock().unwrap().len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cap_phat_chay_hai_lan_cung_luc_mot_khach() {
    TestDatabase::run(async |test| {
        let setup = Arc::new(setup(test).await);
        let pro = plan(&setup.app, "pro", json!({})).await;
        let tasks: Vec<_> = (0..4)
            .map(|_| {
                let setup = setup.clone();
                let body = request("acme", &pro);
                tokio::spawn(async move {
                    call(&setup.app, Method::POST, "/plans/provisionings", "ops", Some(body))
                        .await
                        .0
                })
            })
            .collect();
        let mut statuses = Vec::new();
        for task in tasks {
            statuses.push(task.await.unwrap());
        }
        assert_eq!(
            statuses.iter().filter(|status| **status == StatusCode::CREATED).count(),
            1,
            "{statuses:?}"
        );
        assert!(
            statuses
                .iter()
                .all(|status| [StatusCode::CREATED, StatusCode::CONFLICT].contains(status))
        );
        drain(&setup.app).await;
        assert_eq!(setup.invitations.sent.lock().unwrap().len(), 1);
    })
    .await
    .unwrap();
}
