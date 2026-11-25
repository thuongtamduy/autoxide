//! Hạn mức số nhân sự (`auth.members`) khi lần đồng bộ SCIM chỉ áp một phần, và kết quả đồng bộ khi
//! đặt luật ánh xạ (D-20261125-scim-sync-report). Khách riêng `alpha`: khách mặc định được miễn hạn
//! mức. Cả SQLite và Postgres.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use autoxide_kernel::entitlement::{Entitlements, Limit, TenantEntitlements, limit_exceeded};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::{TenantContext, TenantId};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use sea_orm::{ColumnTrait as _, EntityTrait as _, QueryFilter as _};
use serde_json::{Value, json};

use super::entity::mapping;
use super::tests::{scim, scim_token, user_body};
use crate::http::test_support::{PASSWORD, call_with};
use crate::testing::Services;
use crate::user::NewUser;

const GROUP_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:Group";
const TENANT: &str = "alpha";

/// Gói giả: có `auth.scim`, `auth.members` tối đa `max` người; giữ các suất đang dùng.
struct Members {
    max: Mutex<usize>,
    used: Mutex<BTreeSet<String>>,
}

impl Members {
    fn used(&self) -> BTreeSet<String> {
        self.used.lock().unwrap().clone()
    }

    fn set_max(&self, max: usize) {
        *self.max.lock().unwrap() = max;
    }
}

#[async_trait::async_trait]
impl Entitlements for Members {
    async fn snapshot(&self, _: TenantId) -> Result<Arc<TenantEntitlements>, Error> {
        Ok(Arc::new(TenantEntitlements {
            features: ["auth.scim".to_owned()].into(),
            limits: [("auth.members".to_owned(), None)].into(),
            ..TenantEntitlements::default()
        }))
    }

    async fn reserve(
        &self,
        _: TenantId,
        limit: &Limit,
        _: i64,
        reference: &str,
    ) -> Result<(), Error> {
        if limit.key() != "auth.members" {
            return Ok(());
        }
        let max = *self.max.lock().unwrap();
        let mut used = self.used.lock().unwrap();
        if used.contains(reference) {
            return Ok(());
        }
        if used.len() >= max {
            return Err(limit_exceeded("auth.members", used.len() as i64, max as i64));
        }
        used.insert(reference.to_owned());
        Ok(())
    }

    async fn confirm(&self, _: TenantId, _: &Limit, _: &str) -> Result<(), Error> {
        Ok(())
    }

    async fn release(&self, _: TenantId, _: &Limit, reference: &str) -> Result<(), Error> {
        self.used.lock().unwrap().remove(reference);
        Ok(())
    }

    async fn record_usage(&self, _: TenantId, _: &Limit, _: i64, _: &str) -> Result<(), Error> {
        Ok(())
    }
}

struct Setup {
    app: TestApp,
    services: Services,
    tenant: TenantContext,
    plan: Arc<Members>,
    /// Header `Authorization` của admin khách `alpha`.
    admin: String,
    admin_token: String,
    /// Token SCIM của người chỉ có `auth.scim.manage`.
    low_token: String,
}

async fn login(app: &TestApp, email: &str) -> String {
    let response = app
        .post("/api/v1/auth/login")
        .header("autoxide-tenant", TENANT)
        .json(json!({"email": email, "password": PASSWORD}))
        .send()
        .await
        .unwrap();
    format!("Bearer {}", response.json()["data"]["access_token"].as_str().unwrap())
}

async fn setup(test: &TestDatabase) -> Setup {
    let services = crate::testing::services(test).await;
    let mailer: Arc<dyn autoxide_kernel::mail::Mailer> = services.mails.clone();
    let plan = Arc::new(Members { max: Mutex::new(100), used: Mutex::new(BTreeSet::new()) });
    let provider: Arc<dyn Entitlements> = plan.clone();
    let app = TestApp::builder(test)
        .tenant(TENANT)
        .provide::<dyn autoxide_kernel::mail::Mailer>(mailer)
        .provide::<dyn Entitlements>(provider)
        .module(crate::module())
        .build()
        .await
        .unwrap();
    let tenant = app.tenant(TENANT).await.unwrap();
    let boss = NewUser {
        email: "boss@alpha.example".to_owned(),
        name: "Boss".to_owned(),
        password: Some(PASSWORD.to_owned()),
    };
    services.users.create_with_roles(&tenant, Actor::system(), boss, true).await.unwrap();
    let admin = login(&app, "boss@alpha.example").await;
    let admin_token = scim_token(&app, &admin, Some(TENANT)).await;
    let input = crate::role::RoleInput {
        denies: Vec::new(),
        requires_mfa: false,
        name: "sync".to_owned(),
        description: String::new(),
        permissions: vec!["auth.scim.manage".to_owned()],
        grants: Vec::new(),
    };
    let role = services.roles.create(&tenant, Actor::system(), input).await.unwrap();
    let it = NewUser {
        email: "it@alpha.example".to_owned(),
        name: "IT".to_owned(),
        password: Some(PASSWORD.to_owned()),
    };
    let it = services.users.create(&tenant, Actor::system(), it).await.unwrap();
    services.roles.set_user_roles(&tenant, Actor::system(), it.id, vec![role.id]).await.unwrap();
    let low = login(&app, "it@alpha.example").await;
    let low_token = scim_token(&app, &low, Some(TENANT)).await;
    Setup { app, services, tenant, plan, admin, admin_token, low_token }
}

/// Gọi REST quản trị của `auth` ở khách `alpha` bằng admin.
async fn admin_call(
    setup: &Setup,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let headers = [("authorization", setup.admin.as_str()), ("autoxide-tenant", TENANT)];
    let (status, _, body) = call_with(&setup.app, method, path, &headers, body).await;
    (status, body)
}

async fn role_id(setup: &Setup, name: &str) -> String {
    let (_, roles) = admin_call(setup, "GET", "/roles", None).await;
    roles["data"].as_array().unwrap().iter().find(|role| role["name"] == name).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn group(setup: &Setup, name: &str, members: &[&str]) -> String {
    let members: Vec<Value> = members.iter().map(|id| json!({"value": id})).collect();
    let body = json!({"schemas": [GROUP_SCHEMA], "displayName": name, "members": members});
    let (status, group) = scim(&setup.app, "POST", "/Groups", &setup.admin_token, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{group}");
    group["id"].as_str().unwrap().to_owned()
}

async fn user(setup: &Setup, token: &str, email: &str) -> String {
    let (status, user) =
        scim(&setup.app, "POST", "/Users", token, Some(user_body(email, email))).await;
    assert_eq!(status, StatusCode::CREATED, "{user}");
    user["id"].as_str().unwrap().to_owned()
}

async fn map(setup: &Setup, group_id: &str, role: &str) -> (StatusCode, Value) {
    let root = setup.tenant.id().as_uuid();
    let body = json!({"group_id": group_id, "org_id": root, "role_id": role_id(setup, role).await});
    admin_call(setup, "POST", "/scim/group-mappings", Some(body)).await
}

fn add(user_id: &str) -> Value {
    json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
        "Operations": [{"op": "add", "path": "members", "value": [{"value": user_id}]}]})
}

fn reference(user_id: &str) -> String {
    crate::role::member_reference(uuid::Uuid::parse_str(user_id).unwrap())
}

async fn is_admin(setup: &Setup, user_id: &str) -> bool {
    let id = uuid::Uuid::parse_str(user_id).unwrap();
    setup.services.roles.principal(&setup.tenant, id).await.unwrap().has_all_permissions()
}

#[tokio::test]
async fn dat_luat_dong_bo_moi_thanh_vien_va_bao_ai_loi() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let first = user(&setup, &setup.admin_token, "a@alpha.example").await;
        let second = user(&setup, &setup.admin_token, "b@alpha.example").await;
        let group_id = group(&setup, "Ops", &[&first, &second]).await;
        // Gói còn đúng một chỗ: người đầu nhận role, người sau lỗi — luật vẫn được đặt, câu trả
        // lời nói rõ ai đã nhận, ai lỗi.
        setup.plan.set_max(setup.plan.used().len() + 1);
        let (status, mapped) = map(&setup, &group_id, "admin").await;
        assert_eq!(status, StatusCode::CREATED, "{mapped}");
        assert_eq!(mapped["data"]["granted_user_ids"], json!([first]), "{mapped}");
        assert_eq!(mapped["data"]["failed"][0]["user_id"], second, "{mapped}");
        assert_eq!(mapped["data"]["failed"][0]["code"], "limit_exceeded", "{mapped}");
        assert!(is_admin(&setup, &first).await);
        assert!(!is_admin(&setup, &second).await);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bo_qua_phan_them_thi_tra_cho() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;

        // (a) Người nằm trong nhóm có luật admin mà chưa có role (lần đồng bộ trước lỗi vì hết chỗ).
        // Token quyền thấp đưa họ vào nhóm khác: đồng bộ bỏ qua role admin, người đó không giữ role
        // nào — không được giữ chỗ trong hạn mức.
        let waiting = user(&setup, &setup.low_token, "waiting@alpha.example").await;
        let admins = group(&setup, "Admins", &[&waiting]).await;
        setup.plan.set_max(setup.plan.used().len());
        let (status, mapped) = map(&setup, &admins, "admin").await;
        assert_eq!(status, StatusCode::CREATED, "{mapped}");
        assert_eq!(mapped["data"]["failed"][0]["user_id"], waiting, "{mapped}");
        setup.plan.set_max(100);
        let other = group(&setup, "Other", &[]).await;
        let path = format!("/Groups/{other}");
        let (status, body) =
            scim(&setup.app, "PATCH", &path, &setup.low_token, Some(add(&waiting))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(!is_admin(&setup, &waiting).await);
        assert!(
            !setup.plan.used().contains(&reference(&waiting)),
            "chỗ bị giữ cho người không có role"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bo_qua_phan_go_thi_giu_cho() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let other = group(&setup, "Other", &[]).await;
        // (b) Người giữ admin do SCIM cấp; luật bị gỡ khỏi bảng mà chưa đồng bộ. Token quyền thấp
        // đồng bộ họ: không gỡ được admin (bỏ qua) — người đó vẫn giữ role, chỗ không được trả.
        let chief = user(&setup, &setup.admin_token, "chief@alpha.example").await;
        let bosses = group(&setup, "Bosses", &[]).await;
        let (status, _) = map(&setup, &bosses, "admin").await;
        assert_eq!(status, StatusCode::CREATED);
        let path = format!("/Groups/{bosses}");
        let (status, body) =
            scim(&setup.app, "PATCH", &path, &setup.admin_token, Some(add(&chief))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(is_admin(&setup, &chief).await);
        assert!(setup.plan.used().contains(&reference(&chief)));
        mapping::Entity::delete_many()
            .filter(mapping::Column::GroupId.eq(uuid::Uuid::parse_str(&bosses).unwrap()))
            .exec(setup.tenant.database().write())
            .await
            .unwrap();
        let path = format!("/Groups/{other}");
        let (status, body) =
            scim(&setup.app, "PATCH", &path, &setup.low_token, Some(add(&chief))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(is_admin(&setup, &chief).await);
        assert!(setup.plan.used().contains(&reference(&chief)), "chỗ bị trả dù người đó còn role");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gui_lai_cung_request_thi_dong_bo_lai_nguoi_con_cho() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let admins = group(&setup, "Admins", &[]).await;
        let (status, _) = map(&setup, &admins, "admin").await;
        assert_eq!(status, StatusCode::CREATED);
        let newcomer = user(&setup, &setup.admin_token, "new@alpha.example").await;
        // Hết chỗ: thành viên được ghi, đồng bộ lỗi, hệ thống danh tính nhận lỗi.
        setup.plan.set_max(setup.plan.used().len());
        let path = format!("/Groups/{admins}");
        let (status, body) =
            scim(&setup.app, "PATCH", &path, &setup.admin_token, Some(add(&newcomer))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(!is_admin(&setup, &newcomer).await);
        // Gửi lại đúng request đó khi đã có chỗ: thành viên không đổi thêm, nhưng người chờ được
        // đồng bộ lại.
        setup.plan.set_max(100);
        let (status, body) =
            scim(&setup.app, "PATCH", &path, &setup.admin_token, Some(add(&newcomer))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(is_admin(&setup, &newcomer).await);
        assert!(setup.plan.used().contains(&reference(&newcomer)));
    })
    .await
    .unwrap();
}
