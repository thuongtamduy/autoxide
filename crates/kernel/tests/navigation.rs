//! Sổ điều hướng (bước 51 P1): lọc theo module đã bật, permission ở nút, tính năng của gói
//! (thiếu thì khoá, không ẩn); nhóm nền tảng chỉ cho nhân sự nền tảng; nhãn theo ngôn ngữ; ETag.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::entitlement::{Entitlements, Limit, TenantEntitlements};
use autoxide_kernel::error::Error;
use autoxide_kernel::module::Module;
use autoxide_kernel::module::navigation::{NavGroup, NavItem, Navigation};
use autoxide_kernel::tenancy::TenantId;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::Value;

const SHOP_READ: Permission = Permission::new("shop.read", "See the shop.");
const OFF_READ: Permission = Permission::new("off.read", "See the module that is off.");

struct Shop;

impl Module for Shop {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![SHOP_READ]
    }

    fn navigation(&self) -> Navigation {
        Navigation::default()
            .group(NavGroup::new("sales", "Sales", "cart", 20))
            .item(NavItem::new("sales.orders", "sales", "Orders", 10).permissions(&[SHOP_READ]))
            .item(
                NavItem::new("sales.pos", "sales", "Point of sale", 20)
                    .permissions(&[SHOP_READ])
                    .feature("shop.pos"),
            )
    }
}

/// Module chưa được bật cho khách: mục của nó không hiện.
struct Off;

impl Module for Off {
    fn name(&self) -> &'static str {
        "off"
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![OFF_READ]
    }

    fn navigation(&self) -> Navigation {
        Navigation::default()
            .group(NavGroup::new("sales", "Sales", "cart", 20))
            .item(NavItem::new("sales.off", "sales", "Off", 30).permissions(&[OFF_READ]))
    }
}

/// Gói của mọi khách: bật `shop`, không bật `off`, không có tính năng `shop.pos`.
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

fn ids(view: &Value) -> Vec<String> {
    view["data"]["groups"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|group| group["items"].as_array().unwrap().iter())
        .map(|item| item["id"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn so_dieu_huong_loc_theo_goi_quyen_va_khong_gian() {
    TestDatabase::run(async |test| {
        let plan: Arc<dyn Entitlements> = Arc::new(Plan);
        let app = TestApp::builder(test)
            .tenant("acme")
            .user("platform", 1, &["shop.read", "off.read", "kernel.tenants.read"])
            .user_in("acme", "clerk", 2, &["shop.read", "off.read", "kernel.tenants.read"])
            .user_in("acme", "nobody", 3, &[])
            .provide::<dyn Entitlements>(plan)
            .module(Shop)
            .module(Off)
            .build()
            .await
            .unwrap();
        let get = async |who: &str, tenant: &str, extra: &[(&str, &str)]| {
            let mut request = app.get("/api/v1/kernel/navigation").token(who);
            if tenant != "default" {
                request = request.header("autoxide-tenant", tenant);
            }
            for (name, value) in extra {
                request = request.header(name, value);
            }
            request.send().await.unwrap()
        };

        let clerk = get("clerk", "acme", &[]).await;
        assert_eq!(clerk.status(), StatusCode::OK, "{}", clerk.json());
        let view = clerk.json();
        let items = ids(&view);
        assert!(items.contains(&"sales.orders".to_owned()), "{items:?}");
        assert!(!items.contains(&"sales.off".to_owned()), "module chưa bật thì ẩn: {items:?}");
        assert!(
            !items.iter().any(|id| id.starts_with("platform.")),
            "khách thường không thấy nền tảng"
        );
        let sales = view["data"]["groups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|group| group["id"] == "sales")
            .unwrap()
            .clone();
        assert_eq!(sales["items"][0]["state"], "available");
        assert_eq!(
            (sales["items"][1]["state"].as_str(), sales["items"][1]["required_feature"].as_str()),
            (Some("locked"), Some("shop.pos")),
            "thiếu tính năng thì khoá, không ẩn"
        );

        // Không có permission nào của mục: mục ẩn, nhóm không còn mục thì bỏ.
        let nobody = get("nobody", "acme", &[]).await.json();
        assert!(!ids(&nobody).contains(&"sales.orders".to_owned()));
        assert!(
            nobody["data"]["groups"].as_array().unwrap().iter().all(|group| group["id"] != "sales")
        );

        // Nhân sự nền tảng thấy nhóm nền tảng.
        let platform = get("platform", "default", &[]).await.json();
        assert!(ids(&platform).contains(&"platform.tenants".to_owned()), "{platform}");

        // Nhãn theo ngôn ngữ; ETag và 304.
        let vi = get("clerk", "acme", &[("accept-language", "vi")]).await;
        let home = vi.json()["data"]["groups"][0].clone();
        assert_eq!(
            (home["id"].as_str(), home["label"].as_str()),
            (Some("home"), Some("Trang chủ"))
        );
        let etag = clerk.header("etag").to_owned();
        let again = get("clerk", "acme", &[("if-none-match", etag.as_str())]).await;
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED);
    })
    .await
    .unwrap();
}
