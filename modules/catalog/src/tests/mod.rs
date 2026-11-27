//! Test của `catalog`, trên database gate đang chọn (gate chạy cả SQLite và Postgres). Dựng app
//! như binary bằng `TestApp` của testkit; gọi qua REST, MCP và `dyn Catalog`.

mod blocks;
mod card;
mod categories;
mod changes;
mod collections;
mod digital;
mod display_price;
mod extensions;
mod from_images;
mod media_alts;
mod pages;
mod pricing;
mod products;
mod public;
mod readiness;
mod recommend;
mod resources;
mod search;
mod seller_kyc;
mod statements;
mod suggestions;
mod transfers;
mod variant_attributes;

use autoxide_kernel::access::Scope;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, root_of};
use autoxide_testkit::{TestApp, TestAppBuilder, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use uuid::Uuid;

pub(crate) const ALL: [&str; 12] = [
    "catalog.products.read",
    "catalog.products.write",
    "catalog.products.publish",
    "catalog.prices.read",
    "catalog.prices.write",
    "catalog.prices.approve",
    "catalog.categories.manage",
    "catalog.documents.manage",
    "catalog.collections.manage",
    "catalog.import",
    "catalog.export",
    "catalog.downloads.manage",
];

/// Cây gốc → chuỗi; chuỗi → cửa hàng A, B. ID biết trước để khai người dùng, dựng cây sau.
pub(crate) struct Tree {
    pub chain: OrgId,
    pub a: OrgId,
    pub b: OrgId,
}

/// App với: `boss` (mọi quyền cả khách), `store_a` / `store_b` (mọi quyền ở cửa hàng của mình,
/// độ sâu `unit`).
pub(crate) async fn app(test: &TestDatabase) -> (TestApp, Tree) {
    app_with(test, |builder| builder).await
}

/// Như [`app`], thêm module / trait giả bằng `extra` (gọi sau khi đã đăng ký `catalog`).
pub(crate) async fn app_with<'a>(
    test: &'a TestDatabase,
    extra: impl FnOnce(TestAppBuilder<'a>) -> TestAppBuilder<'a>,
) -> (TestApp, Tree) {
    let org = || OrgId::from_uuid(Uuid::now_v7());
    let tree = Tree { chain: org(), a: org(), b: org() };
    let scoped = |scope: Scope| ALL.map(|name| (name, scope));
    let builder = TestApp::builder(test)
        .member("boss", 1, root_of(DEFAULT_TENANT), &scoped(Scope::Tenant))
        .member("store_a", 2, tree.a, &scoped(Scope::Unit))
        .member("store_b", 3, tree.b, &scoped(Scope::Unit))
        .module(crate::module());
    let app = extra(builder).build().await.unwrap();
    let write = app.database().begin_write().await.unwrap();
    let root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, tree.chain, root, 8).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, tree.a, tree.chain, 8).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, tree.b, tree.chain, 8).await.unwrap();
    write.commit().await.unwrap();
    (app, tree)
}

/// Gọi `/api/v1/catalog{uri}`; trả trạng thái và thân JSON (`null` khi rỗng).
pub(crate) async fn call(
    app: &TestApp,
    method: &str,
    uri: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1/catalog{uri}")).token(who);
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let status = answer.status();
    (status, if answer.body().is_empty() { Value::Null } else { answer.json() })
}

/// Thân tạo sản phẩm vật lý ở nút `org` với SKU `sku`.
pub(crate) fn product_body(org: OrgId, sku: &str, name: &str) -> Value {
    json!({
        "org_id": org.as_uuid(),
        "kind": "physical",
        "tax_category": "vat.standard",
        "locale": "vi",
        "text": {"name": name},
        "variant": {"sku": sku, "weight_grams": 200},
    })
}

/// Tạo sản phẩm bằng `who`; trả (ID sản phẩm, ID biến thể đầu).
pub(crate) async fn create_product(
    app: &TestApp,
    who: &str,
    org: OrgId,
    sku: &str,
    name: &str,
) -> (String, String) {
    let (status, created) =
        call(app, "POST", "/products", who, Some(product_body(org, sku, name))).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let data = &created["data"];
    (
        data["id"].as_str().unwrap().to_owned(),
        data["variants"][0]["id"].as_str().unwrap().to_owned(),
    )
}
