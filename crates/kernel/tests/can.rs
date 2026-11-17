//! Giải thích quyền (bước 51 P11): `POST /api/v1/me/can` trả lý do khớp đúng lỗi mà endpoint thật
//! trả cho cùng người gọi — module ngoài gói (`module_not_enabled`), thiếu permission
//! (`missing_permission`) — và `unknown_permission` cho tên lạ.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::sync::Arc;

use autoxide_kernel::access::{Permission, Principal};
use autoxide_kernel::entitlement::{Entitlements, Limit, TenantEntitlements};
use autoxide_kernel::error::Error;
use autoxide_kernel::module::Module;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantId;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::json;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

const SHOP_READ: Permission = Permission::new("shop.read", "See the shop.");
const OFF_READ: Permission = Permission::new("off.read", "See the module that is off.");

/// Read the shop.
#[utoipa::path(get, path = "/", extensions(("x-permissions" = json!(["shop.read"]))), responses(
    (status = 200, description = "Read.", body = Data<bool>),
    (status = 403, description = "Missing permission; code `permission_denied`.", body = Problem),
))]
async fn read_shop(principal: Principal) -> Result<Data<bool>, Error> {
    principal.require(&SHOP_READ)?;
    Ok(Data { data: true })
}

/// Read the module that is off.
#[utoipa::path(get, path = "/", extensions(("x-permissions" = json!(["off.read"]))), responses(
    (status = 200, description = "Read.", body = Data<bool>),
    (status = 403, description = "Missing permission; code `permission_denied`.", body = Problem),
))]
async fn read_off(principal: Principal) -> Result<Data<bool>, Error> {
    principal.require(&OFF_READ)?;
    Ok(Data { data: true })
}

struct Shop;

impl Module for Shop {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![SHOP_READ]
    }

    fn routes(&self) -> OpenApiRouter {
        OpenApiRouter::new().routes(routes!(read_shop))
    }
}

struct Off;

impl Module for Off {
    fn name(&self) -> &'static str {
        "off"
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![OFF_READ]
    }

    fn routes(&self) -> OpenApiRouter {
        OpenApiRouter::new().routes(routes!(read_off))
    }
}

/// Gói của mọi khách: chỉ bật `shop`.
struct Plan;

#[async_trait::async_trait]
impl Entitlements for Plan {
    async fn snapshot(&self, _tenant: TenantId) -> Result<Arc<TenantEntitlements>, Error> {
        Ok(Arc::new(TenantEntitlements {
            modules: BTreeSet::from(["shop".to_owned()]),
            ..TenantEntitlements::default()
        }))
    }

    async fn reserve(&self, _: TenantId, _: &Limit, _: i64, _: &str) -> Result<(), Error> {
        Ok(())
    }

    async fn confirm(&self, _: TenantId, _: &Limit, _: &str) -> Result<(), Error> {
        Ok(())
    }

    async fn release(&self, _: TenantId, _: &Limit, _: &str) -> Result<(), Error> {
        Ok(())
    }

    async fn record_usage(&self, _: TenantId, _: &Limit, _: i64, _: &str) -> Result<(), Error> {
        Ok(())
    }
}

#[tokio::test]
async fn ly_do_khop_loi_that_cua_endpoint() {
    TestDatabase::run(async |test| {
        let plan: Arc<dyn Entitlements> = Arc::new(Plan);
        let app = TestApp::builder(test)
            .tenant("acme")
            .user_in("acme", "clerk", 2, &["shop.read", "off.read"])
            .user_in("acme", "nobody", 3, &[])
            .provide::<dyn Entitlements>(plan)
            .module(Shop)
            .module(Off)
            .build()
            .await
            .unwrap();
        let ask = async |who: &str| {
            app.post("/api/v1/me/can")
                .token(who)
                .header("autoxide-tenant", "acme")
                .json(json!({"checks": [
                    {"permission": "shop.read"},
                    {"permission": "off.read"},
                    {"permission": "nope.read"},
                ]}))
                .send()
                .await
                .unwrap()
        };
        let call = async |who: &str, path: &str| {
            app.get(path).token(who).header("autoxide-tenant", "acme").send().await.unwrap()
        };

        let clerk = ask("clerk").await;
        assert_eq!(clerk.status(), StatusCode::OK, "{}", clerk.json());
        let answers = clerk.json()["data"].clone();
        assert_eq!(answers[0]["allowed"], true);
        assert_eq!(call("clerk", "/api/v1/shop").await.status(), StatusCode::OK);
        assert_eq!(
            (answers[1]["reason"].as_str(), answers[1]["required_module"].as_str()),
            (Some("module_not_enabled"), Some("off"))
        );
        let off = call("clerk", "/api/v1/off").await;
        assert_eq!(off.json()["code"], "module_not_enabled", "{}", off.json());
        assert_eq!(answers[2]["reason"], "unknown_permission");

        let nobody = ask("nobody").await.json()["data"].clone();
        assert_eq!(nobody[0]["reason"], "missing_permission");
        let denied = call("nobody", "/api/v1/shop").await;
        assert_eq!(denied.json()["code"], "permission_denied", "{}", denied.json());

        let empty = app
            .post("/api/v1/me/can")
            .token("clerk")
            .header("autoxide-tenant", "acme")
            .json(json!({"checks": []}))
            .send()
            .await
            .unwrap();
        assert_eq!(empty.status(), StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}
