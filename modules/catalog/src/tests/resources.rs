//! `catalog.product` và `catalog.category` trong sổ đối tượng (D-202, D-238): công khai chỉ khi
//! đang bán / đang hiện, theo ngôn ngữ; danh mục con theo thứ tự hiển thị; nhãn `seller` chỉ cho
//! người quản lý sản phẩm ở đúng nhánh; lưu trữ / xoá là event "đã gỡ" cho hỏi đáp.

use autoxide_kernel::access::{Grant, Principal, Scope};
use autoxide_kernel::module::Module;
use autoxide_kernel::resource::ResourceChange;
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use serde_json::json;
use uuid::Uuid;

use super::public::{on_sale, price_list};
use super::{app, call, create_product};

fn manager(org: autoxide_kernel::tenancy::OrgId, scope: Scope) -> Principal {
    Principal::with_grants(
        Uuid::from_u128(77),
        [("catalog.products.write".to_owned(), Grant::new(org, scope))],
    )
}

#[tokio::test]
async fn san_pham_trong_so_doi_tuong_va_nhan_seller() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let tenant = app.default_tenant();
        let resources = app.modules().resources();
        let (draft, _) = create_product(&app, "boss", tree.a, "R-1", "Nháp").await;
        let list = price_list(&app, tree.chain).await;
        let (sold, _) = on_sale(&app, &list, tree.a, "R-2", "Đèn bàn", 1).await;
        let ids = vec![draft.clone(), sold.clone(), Uuid::now_v7().to_string()];
        let found = resources.resolve_many(&tenant, "catalog.product", &ids, Some("vi")).await.unwrap();
        assert_eq!(found.len(), 2, "unknown id is absent");
        assert!(!found[&draft].public);
        assert_eq!(found[&draft].title, "", "a draft does not leak its name");
        assert!(found[&sold].public);
        assert_eq!(found[&sold].title, "Đèn bàn");
        assert_eq!(found[&sold].url.as_deref(), Some("/products/den-ban"));

        // Nhãn `seller`: quản lý cửa hàng A (chủ sản phẩm) hay cả khách có; cửa hàng B không.
        let badges = |principal: Principal| {
            let (resources, tenant, sold) = (resources.clone(), tenant.clone(), sold.clone());
            async move { resources.badges(&tenant, "catalog.product", &sold, &principal).await.unwrap() }
        };
        assert_eq!(badges(manager(tree.a, Scope::Unit)).await, vec!["seller"]);
        assert_eq!(badges(manager(tree.chain, Scope::Subtree)).await, vec!["seller"]);
        assert!(badges(manager(tree.b, Scope::Unit)).await.is_empty());
        assert!(badges(Principal::customer(Uuid::from_u128(5))).await.is_empty());

        // Lưu trữ, xoá: event "đã gỡ" để `comments` đóng và ẩn hỏi đáp.
        let events: Vec<(&str, ResourceChange)> = Module::resource_events(&crate::module())
            .iter()
            .filter(|event| event.kind == "catalog.product")
            .map(|event| (event.event, event.change))
            .collect();
        assert_eq!(
            events,
            vec![
                ("catalog.product_archived", ResourceChange::Removed),
                ("catalog.product_deleted", ResourceChange::Removed),
            ]
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn danh_muc_trong_so_doi_tuong() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let tenant = app.default_tenant();
        let resources = app.modules().resources();
        let make = |name: &'static str, en: Option<&'static str>, parent: Option<String>| {
            let app = &app;
            async move {
                let mut texts = json!({"vi": {"name": name}});
                if let Some(en) = en {
                    texts["en"] = json!({"name": en});
                }
                let body = json!({"parent_id": parent, "texts": texts});
                let (status, made) = call(app, "POST", "/categories", "boss", Some(body)).await;
                assert_eq!(status, StatusCode::CREATED, "{made}");
                made["data"]["id"].as_str().unwrap().to_owned()
            }
        };
        let home = make("Nhà cửa", Some("Home"), None).await;
        let kitchen = make("Bếp", Some("Kitchen"), Some(home.clone())).await;
        let bath = make("Phòng tắm", None, Some(home.clone())).await;
        let hidden = make("Ẩn", None, Some(home.clone())).await;
        let body = json!({"visible": false});
        let (status, _) =
            call(&app, "PATCH", &format!("/categories/{hidden}"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK);

        let ids = vec![home.clone(), kitchen.clone(), bath.clone(), hidden.clone()];
        let vi =
            resources.resolve_many(&tenant, "catalog.category", &ids, Some("vi")).await.unwrap();
        assert_eq!(vi[&home].title, "Nhà cửa");
        assert_eq!(vi[&kitchen].url.as_deref(), Some("/categories/bep"));
        assert!(!vi[&hidden].public);
        // Tiếng Anh: danh mục chưa có tên tiếng Anh không công khai ở ngôn ngữ đó.
        let en =
            resources.resolve_many(&tenant, "catalog.category", &ids, Some("en")).await.unwrap();
        assert_eq!(en[&kitchen].title, "Kitchen");
        assert!(!en[&bath].public);

        assert!(resources.has_children("catalog.category"));
        let children =
            resources.children(&tenant, "catalog.category", &home, Some("vi"), 10).await.unwrap();
        let ids: Vec<String> = children.into_iter().map(|child| child.id).collect();
        assert_eq!(
            ids,
            vec![kitchen.clone(), bath.clone()],
            "hidden child left out, display order"
        );
        let first =
            resources.children(&tenant, "catalog.category", &home, Some("vi"), 1).await.unwrap();
        assert_eq!(first.len(), 1);
    })
    .await
    .unwrap();
}
