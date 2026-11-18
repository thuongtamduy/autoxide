//! Sổ điều hướng (bước 51 P1): lọc theo module đã bật, permission ở nút, tính năng của gói
//! (thiếu thì khoá, không ẩn); nhóm nền tảng chỉ cho nhân sự nền tảng; nhãn theo ngôn ngữ; ETag.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::entitlement::{Entitlements, Limit, TenantEntitlements};
use autoxide_kernel::error::Error;
use autoxide_kernel::module::Module;
use autoxide_kernel::module::navigation::{MobilePlacement, NavGroup, NavItem, Navigation};
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

const SECRET_READ: Permission = Permission::new("shop.secret", "See the secret screen.");

/// Module khai nhiều mục cho thanh đáy hơn mức cho phép: một mục khoá, một mục ẩn trên điện
/// thoại, một mục người dùng không có quyền.
struct Phone;

impl Module for Phone {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![SHOP_READ, SECRET_READ]
    }

    fn navigation(&self) -> Navigation {
        let mut navigation = Navigation::default().group(NavGroup::new("ops", "Ops", "cart", 20));
        for (order, id) in
            [(1, "ops.a"), (2, "ops.b"), (3, "ops.c"), (4, "ops.d"), (5, "ops.e"), (6, "ops.f")]
        {
            navigation = navigation.item(
                NavItem::new(id, "ops", "Screen", 100 - order)
                    .permissions(&[SHOP_READ])
                    .mobile(MobilePlacement::Primary, order),
            );
        }
        navigation
            .item(
                NavItem::new("ops.locked", "ops", "Locked", 1)
                    .permissions(&[SHOP_READ])
                    .feature("shop.pos")
                    .mobile(MobilePlacement::Primary, 0),
            )
            .item(
                NavItem::new("ops.secret", "ops", "Secret", 2)
                    .permissions(&[SECRET_READ])
                    .mobile(MobilePlacement::Primary, 0),
            )
            .item(
                NavItem::new("ops.desk", "ops", "Desk only", 3)
                    .permissions(&[SHOP_READ])
                    .mobile(MobilePlacement::Hidden, 0),
            )
            .item(NavItem::new("ops.more", "ops", "More", 4).permissions(&[SHOP_READ]))
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

fn items_of(view: &Value) -> Vec<Value> {
    view["data"]["groups"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|group| group["items"].as_array().unwrap().iter().cloned())
        .collect()
}

#[tokio::test]
async fn dieu_huong_mobile_toi_da_5_muc_primary() {
    TestDatabase::run(async |test| {
        let plan: Arc<dyn Entitlements> = Arc::new(Plan);
        let app = TestApp::builder(test)
            .tenant("acme")
            .user_in("acme", "clerk", 1, &["shop.read"])
            .provide::<dyn Entitlements>(plan)
            .module(Phone)
            .build()
            .await
            .unwrap();
        let get = async |device: &str| {
            let path = if device.is_empty() {
                "/api/v1/kernel/navigation".to_owned()
            } else {
                format!("/api/v1/kernel/navigation?device={device}")
            };
            app.get(&path).token("clerk").header("autoxide-tenant", "acme").send().await.unwrap()
        };
        let mobile = get("mobile").await;
        assert_eq!(mobile.status(), StatusCode::OK, "{}", mobile.json());
        let items = items_of(&mobile.json());
        let primary: Vec<&str> = items
            .iter()
            .filter(|item| item["mobile"]["placement"] == "primary")
            .map(|item| item["id"].as_str().unwrap())
            .collect();
        // Năm mục `primary` có thứ tự nhỏ nhất trong số mục hiện và `available`.
        assert_eq!(primary, ["ops.a", "ops.b", "ops.c", "ops.d", "ops.e"]);
        let placement = |id: &str| {
            items
                .iter()
                .find(|item| item["id"] == id)
                .map(|item| item["mobile"]["placement"].clone())
        };
        assert_eq!(placement("ops.f").unwrap(), "secondary", "dư thì thành secondary");
        assert_eq!(placement("ops.locked").unwrap(), "secondary", "mục khoá không lên thanh đáy");
        assert_eq!(placement("ops.more").unwrap(), "secondary", "không khai thì secondary");
        assert_eq!(placement("ops.secret"), None, "vẫn lọc permission");
        assert_eq!(placement("ops.desk"), None, "mục hidden bị bỏ");
        // Trong nhóm: primary trước, rồi theo thứ tự.
        let ops: Vec<String> =
            ids(&mobile.json()).into_iter().filter(|id| id.starts_with("ops.")).collect();
        assert_eq!(ops[..5], ["ops.a", "ops.b", "ops.c", "ops.d", "ops.e"]);

        // Desktop trả như trước: không có `mobile`, vẫn có mục hidden trên điện thoại.
        let desktop = get("").await;
        let desktop_items = items_of(&desktop.json());
        assert!(desktop_items.iter().all(|item| item.get("mobile").is_none()));
        assert!(desktop_items.iter().any(|item| item["id"] == "ops.desk"));
        assert_eq!(get("desktop").await.json(), desktop.json());
        assert_ne!(desktop.header("etag"), mobile.header("etag"), "ETag gồm cả device");

        let wrong = get("watch").await;
        assert_eq!(
            (wrong.status(), wrong.json()["errors"][0]["field"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("device"))
        );
    })
    .await
    .unwrap();
}
