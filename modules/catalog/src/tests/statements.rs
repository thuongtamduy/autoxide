//! Số câu SQL của đường đọc công khai (D-20261120-step39-hot-paths): không tăng theo số sản phẩm
//! trên trang (không N+1).

use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;

use super::app_with;
use super::public::{drain, on_sale, price_list};

/// Số câu SQL của `GET /api/v1/catalog{uri}` (không đăng nhập); kiểm trả 200.
async fn count(app: &TestApp, uri: &str) -> usize {
    let (answer, statements) =
        app.statements(app.get(&format!("/api/v1/catalog{uri}")).send()).await;
    let answer = answer.unwrap();
    assert_eq!(answer.status(), StatusCode::OK, "{uri}: {}", answer.text());
    // Bộ đếm chỉ thấy câu của chính request (testkit): 0 câu nghĩa là việc chạy ở task khác, phép đo mù.
    assert!(!statements.is_empty(), "{uri}: no statement counted");
    statements.len()
}

/// Đếm mọi đường công khai với `products` sản phẩm đang bán.
async fn measure(test: &TestDatabase, products: usize) -> Vec<(String, usize)> {
    // Cache theo khách (cài đặt khách, khách theo tên miền) giữ suốt bài đo: hết hạn giữa chừng (mặc định 10 giây,
    // bài chạy lâu hơn trên Postgres) thì một lượt đo có thêm câu nạp lại cache — đo sai, không phải N+1.
    let (app, tree) =
        app_with(test, |builder| builder.setting("AUTOXIDE_TENANCY_CACHE_SECONDS", "3600")).await;
    let list = price_list(&app, tree.chain).await;
    let mut first = String::new();
    for index in 0..products {
        let name = format!("Áo thun {index}");
        let (product, _) =
            on_sale(&app, &list, tree.chain, &format!("S-{index}"), &name, 100_000).await;
        if index == 0 {
            first = product;
        }
    }
    drain(&app).await;
    let site = tree.chain.as_uuid();
    let uris = [
        format!("/public/products?locale=vi&site={site}"),
        format!("/public/products?locale=vi&site={site}&q=ao"),
        format!("/public/products/by-slug/ao-thun-0?locale=vi&site={site}"),
        format!("/public/products/facets?locale=vi&site={site}"),
        format!("/public/search/suggest?locale=vi&site={site}&q=ao"),
        format!("/public/products/{first}/similar?locale=vi&site={site}"),
        format!("/public/categories?locale=vi&site={site}"),
        format!("/public/sitemap?site={site}"),
    ];
    // Gọi mỗi đường một lần trước khi đếm: lượt đầu nạp cache (cài đặt khách…), không phải việc của số sản phẩm.
    for uri in &uris {
        count(&app, uri).await;
    }
    let mut counted = Vec::new();
    for uri in uris {
        let name = uri.split('?').next().unwrap_or_default().replace(&first, "{id}");
        counted.push((name, count(&app, &uri).await));
    }
    counted
}

/// Một sản phẩm hay mười hai sản phẩm trên trang: cùng số câu SQL.
#[tokio::test]
async fn khong_tang_theo_so_san_pham() {
    let one = TestDatabase::run(async |test| measure(test, 1).await).await.unwrap();
    let many = TestDatabase::run(async |test| measure(test, 12).await).await.unwrap();
    eprintln!("catalog SQL: 1 = {one:?}\n12 = {many:?}");
    assert_eq!(one, many);
}

/// `OrgDirectory` giả: đếm lượt tra từng nút và lượt tra theo lô.
#[derive(Default)]
struct CountingOrgs {
    one: std::sync::atomic::AtomicUsize,
    many: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl autoxide_organizations::api::OrgDirectory for CountingOrgs {
    async fn unit(
        &self,
        _: &autoxide_kernel::tenancy::TenantContext,
        id: autoxide_kernel::tenancy::OrgId,
    ) -> Result<Option<autoxide_organizations::api::Unit>, autoxide_kernel::error::Error> {
        self.one.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(Some(autoxide_organizations::api::Unit::new(id, None, "store", "Cửa hàng")))
    }
    async fn units(
        &self,
        _: &autoxide_kernel::tenancy::TenantContext,
        ids: &[autoxide_kernel::tenancy::OrgId],
    ) -> Result<Vec<autoxide_organizations::api::Unit>, autoxide_kernel::error::Error> {
        self.many.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(ids
            .iter()
            .map(|id| autoxide_organizations::api::Unit::new(*id, None, "store", "Cửa hàng"))
            .collect())
    }
    async fn root(
        &self,
        tenant: &autoxide_kernel::tenancy::TenantContext,
    ) -> Result<autoxide_organizations::api::Unit, autoxide_kernel::error::Error> {
        let root = autoxide_kernel::tenancy::root_of(tenant.id());
        Ok(autoxide_organizations::api::Unit::new(root, None, "root", "Root"))
    }
    async fn ancestors(
        &self,
        _: &autoxide_kernel::tenancy::TenantContext,
        _: autoxide_kernel::tenancy::OrgId,
    ) -> Result<Vec<autoxide_organizations::api::Unit>, autoxide_kernel::error::Error> {
        Ok(Vec::new())
    }
    async fn is_within(
        &self,
        _: &autoxide_kernel::tenancy::TenantContext,
        _: autoxide_kernel::tenancy::OrgId,
        _: autoxide_kernel::tenancy::OrgId,
    ) -> Result<bool, autoxide_kernel::error::Error> {
        Ok(true)
    }
    async fn setting(
        &self,
        _: &autoxide_kernel::tenancy::TenantContext,
        _: autoxide_kernel::tenancy::OrgId,
        _: &str,
    ) -> Result<Option<autoxide_organizations::api::EffectiveSetting>, autoxide_kernel::error::Error>
    {
        Ok(None)
    }
}

/// Danh sách có sản phẩm của ba người bán: tên người bán tra một lần cho cả trang, không lần nào
/// mỗi người bán.
#[tokio::test]
async fn ten_nguoi_ban_tra_mot_lan() {
    TestDatabase::run(async |test| {
        let orgs = std::sync::Arc::new(CountingOrgs::default());
        let provided = orgs.clone();
        let (app, tree) = super::app_with(test, move |builder| {
            builder.provide::<dyn autoxide_organizations::api::OrgDirectory>(provided)
        })
        .await;
        let list = price_list(&app, tree.chain).await;
        for (index, org) in [tree.chain, tree.a, tree.b, tree.a].into_iter().enumerate() {
            on_sale(&app, &list, org, &format!("S-{index}"), &format!("Áo {index}"), 100_000).await;
        }
        drain(&app).await;
        orgs.one.store(0, std::sync::atomic::Ordering::Relaxed);
        orgs.many.store(0, std::sync::atomic::Ordering::Relaxed);
        let site = tree.chain.as_uuid();
        let answer = app
            .get(&format!("/api/v1/catalog/public/products?locale=vi&site={site}"))
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), StatusCode::OK, "{}", answer.text());
        assert_eq!(answer.json()["data"].as_array().unwrap().len(), 4);
        assert_eq!(orgs.one.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert_eq!(orgs.many.load(std::sync::atomic::Ordering::Relaxed), 1);
    })
    .await
    .unwrap();
}
