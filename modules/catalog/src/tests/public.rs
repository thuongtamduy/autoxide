//! Đọc công khai cho website: chỉ sản phẩm đang bán và có giá, lọc theo danh mục / thuộc tính /
//! khoảng giá, tìm không dấu, ETag, sitemap, cửa hàng ẩn sản phẩm của chuỗi, người bán.

use autoxide_kernel::tenancy::OrgId;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use super::{app, call, create_product};

/// Chạy hết job đang chờ (dựng lại storefront).
pub(super) async fn drain(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap();
    while worker.run_once().await.unwrap() > 0 {}
}

/// `GET /api/v1/catalog/public{uri}` không đăng nhập; trả trạng thái, thân, `ETag`.
pub(super) async fn public(
    app: &TestApp,
    uri: &str,
    etag: Option<&str>,
) -> (StatusCode, Value, String) {
    let mut request = app.request(Method::GET, &format!("/api/v1/catalog/public{uri}"));
    if let Some(etag) = etag {
        request = request.header("if-none-match", etag);
    }
    let answer = request.send().await.unwrap();
    let status = answer.status();
    let tag = answer.headers().get("etag").and_then(|v| v.to_str().ok()).unwrap_or("").to_owned();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (status, body, tag)
}

fn vnd(amount: i64) -> Value {
    json!({"amount": amount.to_string(), "currency": "VND"})
}

/// Bảng giá VND ở `org`; trả ID.
pub(super) async fn price_list(app: &TestApp, org: OrgId) -> String {
    let body = json!({"org_id": org.as_uuid(), "name": "Giá bán", "currency": "VND", "tax_inclusive": true});
    let (status, made) = call(app, "POST", "/price-lists", "boss", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    made["data"]["id"].as_str().unwrap().to_owned()
}

async fn set_price(app: &TestApp, list: &str, variant: &str, amount: i64, reference: &str) {
    let body =
        json!({"reference": reference, "prices": [{"variant_id": variant, "amount": vnd(amount)}]});
    let (status, added) =
        call(app, "POST", &format!("/price-lists/{list}/prices"), "boss", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{added}");
}

async fn activate(app: &TestApp, product: &str) {
    let body = json!({"status": "active"});
    let (status, sold) =
        call(app, "POST", &format!("/products/{product}/status"), "boss", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{sold}");
}

/// Sản phẩm đang bán ở `org`, giá `amount`; trả (ID sản phẩm, ID biến thể).
pub(super) async fn on_sale(
    app: &TestApp,
    list: &str,
    org: OrgId,
    sku: &str,
    name: &str,
    amount: i64,
) -> (String, String) {
    let (product, variant) = create_product(app, "boss", org, sku, name).await;
    set_price(app, list, &variant, amount, sku).await;
    activate(app, &product).await;
    (product, variant)
}

pub(super) fn names(listed: &Value) -> Vec<String> {
    listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap().to_owned())
        .collect()
}

/// Chỉ trả sản phẩm `active` có giá ở website; nháp, chưa có giá, đã lưu trữ thì không — kể cả
/// đọc thẳng theo slug.
#[tokio::test]
async fn chi_tra_san_pham_dang_ban() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let list = price_list(&app, tree.chain).await;
        let (sold, _) = on_sale(&app, &list, tree.chain, "S-1", "Đang bán", 100_000).await;
        let (_draft, variant) = create_product(&app, "boss", tree.chain, "S-2", "Bản nháp").await;
        set_price(&app, &list, &variant, 50_000, "S-2").await;
        let (unpriced, _) = create_product(&app, "boss", tree.chain, "S-3", "Chưa có giá").await;
        activate(&app, &unpriced).await;
        let (archived, _) = on_sale(&app, &list, tree.chain, "S-4", "Đã lưu trữ", 70_000).await;
        let body = json!({"status": "archived"});
        let (status, _) =
            call(&app, "POST", &format!("/products/{archived}/status"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        drain(&app).await;

        let site = tree.chain.as_uuid();
        let (status, listed, _) =
            public(&app, &format!("/products?locale=vi&site={site}"), None).await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        assert_eq!(names(&listed), vec!["Đang bán"]);
        let card = &listed["data"][0];
        assert_eq!(card["id"], sold.as_str());
        assert_eq!(card["price_from"], vnd(100_000));
        assert_eq!(card["path"], "/products/dang-ban");
        assert_eq!(card["seller"]["own"], true);

        let (status, page, _) =
            public(&app, &format!("/products/by-slug/dang-ban?locale=vi&site={site}"), None).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert_eq!(page["data"]["variants"][0]["price"], vnd(100_000));
        for slug in ["ban-nhap", "chua-co-gia", "da-luu-tru"] {
            let uri = format!("/products/by-slug/{slug}?locale=vi&site={site}");
            let (status, problem, _) = public(&app, &uri, None).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{slug}: {problem}");
            assert_eq!(problem["code"], "catalog.product_slug_not_found");
        }
        // Ngôn ngữ chưa có bản dịch: không có trên danh sách tiếng Anh.
        let (_, english, _) = public(&app, &format!("/products?locale=en&site={site}"), None).await;
        assert!(names(&english).is_empty());
        // Ngôn ngữ không hỗ trợ: 422.
        let (status, problem, _) = public(&app, "/products?locale=xx", None).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("catalog.unsupported_locale"))
        );
    })
    .await
    .unwrap();
}

/// Lọc theo danh mục (gồm danh mục con), thuộc tính lọc được, khoảng giá; sắp theo giá.
#[tokio::test]
async fn loc_theo_thuoc_tinh_va_khoang_gia() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let list = price_list(&app, tree.chain).await;
        let body = json!({"code": "color", "data_type": "option", "names": {"vi": "Màu"}, "filterable": true});
        let (status, made) = call(&app, "POST", "/attributes", "boss", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        let color = made["data"]["id"].as_str().unwrap().to_owned();
        let options = json!({"options": [
            {"code": "red", "labels": {"vi": "Đỏ"}},
            {"code": "blue", "labels": {"vi": "Xanh"}},
        ]});
        let (status, _) =
            call(&app, "PUT", &format!("/attributes/{color}/options"), "boss", Some(options)).await;
        assert_eq!(status, StatusCode::OK);
        let category = |name: &str, parent: Option<String>| {
            json!({"parent_id": parent, "texts": {"vi": {"name": name}}})
        };
        let (_, made) = call(&app, "POST", "/categories", "boss", Some(category("Thời trang", None))).await;
        let fashion = made["data"]["id"].as_str().unwrap().to_owned();
        let (_, made) =
            call(&app, "POST", "/categories", "boss", Some(category("Áo", Some(fashion.clone())))).await;
        let shirts = made["data"]["id"].as_str().unwrap().to_owned();

        let goods = [("A-1", "Áo đỏ", 300_000, "red"), ("A-2", "Áo xanh", 100_000, "blue"), ("A-3", "Áo đỏ rẻ", 50_000, "red")];
        for (sku, name, amount, colour) in goods {
            let (product, variant) = create_product(&app, "boss", tree.chain, sku, name).await;
            set_price(&app, &list, &variant, amount, sku).await;
            let body = json!({"attributes": [{"attribute_id": color, "value": colour}]});
            let (status, _) =
                call(&app, "PUT", &format!("/products/{product}/attributes"), "boss", Some(body)).await;
            assert_eq!(status, StatusCode::OK);
            if sku != "A-3" {
                let body = json!({"category_ids": [shirts]});
                let (status, _) =
                    call(&app, "PUT", &format!("/products/{product}/categories"), "boss", Some(body)).await;
                assert_eq!(status, StatusCode::OK);
            }
            activate(&app, &product).await;
        }
        drain(&app).await;

        let site = tree.chain.as_uuid();
        let get = |query: &str| format!("/products?locale=vi&site={site}&{query}");
        let (_, cheap_first, _) = public(&app, &get("sort=price_asc"), None).await;
        assert_eq!(names(&cheap_first), vec!["Áo đỏ rẻ", "Áo xanh", "Áo đỏ"]);
        let (_, dear_first, _) = public(&app, &get("sort=price_desc&limit=1"), None).await;
        assert_eq!(names(&dear_first), vec!["Áo đỏ"]);
        let cursor = dear_first["pagination"]["next_cursor"].as_str().unwrap().to_owned();
        let (_, next, _) = public(&app, &get(&format!("sort=price_desc&limit=1&cursor={cursor}")), None).await;
        assert_eq!(names(&next), vec!["Áo xanh"]);
        let (_, red, _) = public(&app, &get("attr.color=red&sort=price_asc"), None).await;
        assert_eq!(names(&red), vec!["Áo đỏ rẻ", "Áo đỏ"]);
        let (_, either, _) = public(&app, &get("attr.color=red,blue&price_max=150000&sort=price_asc"), None).await;
        assert_eq!(names(&either), vec!["Áo đỏ rẻ", "Áo xanh"]);
        let (_, ranged, _) = public(&app, &get("price_min=60000&price_max=200000"), None).await;
        assert_eq!(names(&ranged), vec!["Áo xanh"]);
        // Danh mục cha gồm danh mục con.
        let (_, in_fashion, _) = public(&app, &get("category=thoi-trang&sort=price_asc"), None).await;
        assert_eq!(names(&in_fashion), vec!["Áo xanh", "Áo đỏ"]);
        // Thuộc tính không lọc được, giá trị sai, danh mục không có: 422, gom một lần.
        let (status, problem, _) =
            public(&app, &get("attr.color=green&attr.size=m&category=khong-co"), None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["errors"].as_array().unwrap().len(), 3, "{problem}");

        // Cây danh mục công khai, và một danh mục theo slug.
        let (status, tree_listed, _) = public(&app, "/categories?locale=vi", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(tree_listed["data"].as_array().unwrap().len(), 2);
        let (status, found, _) = public(&app, "/categories/ao?locale=vi", None).await;
        assert_eq!((status, &found["data"]["parent_id"]), (StatusCode::OK, &json!(fashion)));
        // Ẩn danh mục cha: cả nhánh biến mất.
        let body = json!({"visible": false});
        let (status, _) = call(&app, "PATCH", &format!("/categories/{fashion}"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _, _) = public(&app, "/categories/ao?locale=vi", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (_, tree_listed, _) = public(&app, "/categories?locale=vi", None).await;
        assert!(tree_listed["data"].as_array().unwrap().is_empty());
    })
    .await
    .unwrap();
}

/// Tìm có dấu hay không dấu đều ra; câu tìm quá dài bị từ chối.
#[tokio::test]
async fn tim_khong_dau() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let list = price_list(&app, tree.chain).await;
        on_sale(&app, &list, tree.chain, "T-1", "Áo thun Đà Lạt", 120_000).await;
        on_sale(&app, &list, tree.chain, "T-2", "Quần jean", 220_000).await;
        drain(&app).await;
        let site = tree.chain.as_uuid();
        for q in ["ao%20thun", "%C3%A1o%20thun", "da%20lat", "DA%20LAT"] {
            let (status, found, _) =
                public(&app, &format!("/products?locale=vi&site={site}&q={q}"), None).await;
            assert_eq!(status, StatusCode::OK, "{found}");
            assert_eq!(names(&found), vec!["Áo thun Đà Lạt"], "q={q}");
        }
        let long = "a".repeat(201);
        let (status, _, _) = public(&app, &format!("/products?locale=vi&q={long}"), None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

/// `ETag` giữ nguyên khi không đổi gì (→ 304), đổi khi giá hay nội dung đổi.
#[tokio::test]
async fn etag_doi_khi_gia_hay_noi_dung_doi() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let list = price_list(&app, tree.chain).await;
        let (product, variant) = on_sale(&app, &list, tree.chain, "E-1", "Ấm trà", 150_000).await;
        drain(&app).await;
        let uri = format!("/products/by-slug/am-tra?locale=vi&site={}", tree.chain.as_uuid());
        let (status, _, first) = public(&app, &uri, None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!first.is_empty());
        let (status, _, _) = public(&app, &uri, Some(&first)).await;
        assert_eq!(status, StatusCode::NOT_MODIFIED);

        set_price(&app, &list, &variant, 140_000, "E-1b").await;
        drain(&app).await;
        let (status, page, priced) = public(&app, &uri, Some(&first)).await;
        assert_eq!(status, StatusCode::OK, "price changed: not 304");
        assert_eq!(page["data"]["price_from"], vnd(140_000));
        assert_ne!(priced, first);

        let body =
            json!({"name": "Ấm trà", "slug": "am-tra", "short_description": "Gốm Bát Tràng"});
        let (status, _) =
            call(&app, "PUT", &format!("/products/{product}/translations/vi"), "boss", Some(body))
                .await;
        assert_eq!(status, StatusCode::OK);
        let (status, page, described) = public(&app, &uri, Some(&priced)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(page["data"]["short_description"], "Gốm Bát Tràng");
        assert_ne!(described, priced);
        let (_, _, listed) = public(&app, "/categories?locale=vi", None).await;
        assert!(!listed.is_empty(), "every public answer has an ETag");
    })
    .await
    .unwrap();
}

/// Sitemap có mọi sản phẩm đang bán, mỗi ngôn ngữ một đường dẫn, đi hết bằng con trỏ.
#[tokio::test]
async fn sitemap_du_moi_duong_dan() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let list = price_list(&app, tree.chain).await;
        let (teapot, _) = on_sale(&app, &list, tree.chain, "M-1", "Ấm trà", 150_000).await;
        on_sale(&app, &list, tree.chain, "M-2", "Chén trà", 50_000).await;
        create_product(&app, "boss", tree.chain, "M-3", "Nháp").await;
        let body = json!({"name": "Teapot"});
        let (status, _) =
            call(&app, "PUT", &format!("/products/{teapot}/translations/en"), "boss", Some(body))
                .await;
        assert_eq!(status, StatusCode::OK);
        drain(&app).await;

        let site = tree.chain.as_uuid();
        let mut paths = Vec::new();
        let mut cursor = String::new();
        loop {
            let uri = format!("/sitemap?site={site}&limit=1{cursor}");
            let (status, page, _) = public(&app, &uri, None).await;
            assert_eq!(status, StatusCode::OK, "{page}");
            for item in page["data"].as_array().unwrap() {
                for text in item["translations"].as_array().unwrap() {
                    paths.push(format!(
                        "{}:{}",
                        text["locale"].as_str().unwrap(),
                        text["path"].as_str().unwrap()
                    ));
                }
                assert!(item["modified_at"].is_string());
            }
            match page["pagination"]["next_cursor"].as_str() {
                Some(next) => cursor = format!("&cursor={next}"),
                None => break,
            }
        }
        paths.sort();
        assert_eq!(
            paths,
            vec!["en:/products/teapot", "vi:/products/am-tra", "vi:/products/chen-tra"]
        );
        let (_, english, _) = public(&app, &format!("/sitemap?site={site}&locale=en"), None).await;
        assert_eq!(english["data"].as_array().unwrap().len(), 1);
    })
    .await
    .unwrap();
}

/// Cửa hàng A ẩn sản phẩm của chuỗi trên website của mình; cửa hàng B vẫn thấy. Sản phẩm của
/// cửa hàng A hiện trên website của chuỗi như hàng của người bán, không hiện ở cửa hàng B.
#[tokio::test]
async fn cua_hang_an_san_pham_cua_chuoi() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let list = price_list(&app, tree.chain).await;
        let (shared, _) = on_sale(&app, &list, tree.chain, "C-1", "Hàng chuỗi", 100_000).await;
        on_sale(&app, &list, tree.a, "C-2", "Hàng cửa hàng A", 80_000).await;
        drain(&app).await;
        let listed = async |site: OrgId, extra: &str| {
            let uri = format!("/products?locale=vi&site={}{extra}", site.as_uuid());
            let (status, found, _) = public(&app, &uri, None).await;
            assert_eq!(status, StatusCode::OK, "{found}");
            let mut found = names(&found);
            found.sort();
            found
        };
        assert_eq!(listed(tree.chain, "").await, vec!["Hàng chuỗi", "Hàng cửa hàng A"]);
        assert_eq!(listed(tree.chain, "&sold_by=own").await, vec!["Hàng chuỗi"]);
        assert_eq!(listed(tree.chain, "&sold_by=marketplace").await, vec!["Hàng cửa hàng A"]);
        let seller = format!("&seller={}", tree.a.as_uuid());
        assert_eq!(listed(tree.chain, &seller).await, vec!["Hàng cửa hàng A"]);
        assert_eq!(listed(tree.a, "").await, vec!["Hàng chuỗi", "Hàng cửa hàng A"]);
        assert_eq!(listed(tree.b, "").await, vec!["Hàng chuỗi"]);

        // Cửa hàng B không tuỳ biến được cửa hàng A.
        let uri = format!("/stores/{}/overrides/{shared}", tree.a.as_uuid());
        let (status, _) = call(&app, "PUT", &uri, "store_b", Some(json!({"hidden": true}))).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, saved) =
            call(&app, "PUT", &uri, "store_a", Some(json!({"hidden": true}))).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        drain(&app).await;
        assert_eq!(listed(tree.a, "").await, vec!["Hàng cửa hàng A"]);
        assert_eq!(listed(tree.b, "").await, vec!["Hàng chuỗi"]);
        assert_eq!(listed(tree.chain, "").await, vec!["Hàng chuỗi", "Hàng cửa hàng A"]);
        let (status, _, _) = public(
            &app,
            &format!("/products/by-slug/hang-chuoi?locale=vi&site={}", tree.a.as_uuid()),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // Hiện lại.
        let (status, _) = call(&app, "PUT", &uri, "store_a", Some(json!({"hidden": false}))).await;
        assert_eq!(status, StatusCode::OK);
        drain(&app).await;
        assert_eq!(listed(tree.a, "").await, vec!["Hàng chuỗi", "Hàng cửa hàng A"]);
    })
    .await
    .unwrap();
}

/// Giá tạm thời hết hạn: lịch `catalog.apply_scheduled_prices` dựng lại storefront, website thấy
/// giá cũ mà không cần ai sửa gì.
#[tokio::test]
async fn gia_tam_thoi_het_han_storefront_ve_gia_cu() {
    #[derive(serde::Serialize)]
    struct Tick;
    impl autoxide_kernel::job::Job for Tick {
        const KIND: &'static str = crate::storefront::SCHEDULED_PRICES;
    }
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let list = price_list(&app, tree.chain).await;
        let (_, variant) = on_sale(&app, &list, tree.chain, "H-1", "Bình hoa", 200_000).await;
        // Đủ rộng để lượt đọc đầu (sau khi dựng lại storefront) chắc chắn còn trong hạn kể cả khi máy CI
        // chậm — 2 giây từng hết hạn trước lượt đọc đầu trên Postgres 18.
        let ends = chrono::Utc::now() + chrono::Duration::seconds(8);
        let body = json!({"reference": "sale", "prices": [{"variant_id": variant, "amount": vnd(150_000), "valid_to": ends}]});
        let (status, added) =
            call(&app, "POST", &format!("/price-lists/{list}/prices"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{added}");
        drain(&app).await;
        let uri = format!("/products?locale=vi&site={}", tree.chain.as_uuid());
        let (_, listed, _) = public(&app, &uri, None).await;
        assert_eq!(listed["data"][0]["price_from"], vnd(150_000));

        // Ngủ tới khi giá tạm thật sự hết hạn (cộng một chút), không ngủ một khoảng cố định.
        let left = (ends - chrono::Utc::now()).to_std().unwrap_or_default();
        tokio::time::sleep(left + std::time::Duration::from_millis(500)).await;
        let queue = autoxide_kernel::job::JobQueue::new("catalog");
        queue
            .enqueue_database(app.database().write(), &Tick, Default::default())
            .await
            .unwrap();
        drain(&app).await;
        let (_, listed, _) = public(&app, &uri, None).await;
        assert_eq!(listed["data"][0]["price_from"], vnd(200_000), "{listed}");
    })
    .await
    .unwrap();
}

/// Nguồn giá giảm giả: giảm 20% biến thể có giá niêm yết từ 150 000 trở lên, hết sau một ngày.
/// Chỉ giảm theo giá (không theo thứ tự cắm nguồn): job dựng lại storefront cả khách xếp từ trước
/// có thể chạy sau khi cắm nguồn, và khi đó mọi sản phẩm đều được hỏi lại giá giảm.
struct TwentyOff;

#[async_trait::async_trait]
impl crate::api::SalePriceSource for TwentyOff {
    async fn sale_prices(
        &self,
        _tenant: &autoxide_kernel::tenancy::TenantContext,
        _org_id: OrgId,
        at: chrono::DateTime<chrono::Utc>,
        prices: &[(uuid::Uuid, autoxide_kernel::money::Money)],
    ) -> Result<Vec<crate::api::SalePrice>, autoxide_kernel::error::Error> {
        Ok(prices
            .iter()
            .filter(|(_, price)| price.minor() >= 150_000)
            .map(|(id, price)| {
                let sale =
                    autoxide_kernel::money::Money::new(price.minor() * 8 / 10, price.currency());
                crate::api::SalePrice::new(*id, sale, at + chrono::Duration::days(1))
            })
            .collect())
    }
}

/// Giá giảm tự động (nguồn do `promotions` cắm qua `CatalogExtensions`) vào storefront: thẻ có
/// `sale_price` và `sale_ends_at`, `on_sale=true` chỉ trả hàng đang giảm.
#[tokio::test]
async fn gia_giam_tu_dong_va_loc_on_sale() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let list = price_list(&app, tree.chain).await;
        on_sale(&app, &list, tree.chain, "G-1", "Không giảm", 100_000).await;
        drain(&app).await;
        let extensions = app.api::<dyn crate::api::CatalogExtensions>().unwrap();
        assert!(extensions.set_sale_price_source(std::sync::Arc::new(TwentyOff)));
        assert!(
            !extensions.set_sale_price_source(std::sync::Arc::new(TwentyOff)),
            "one source only"
        );
        on_sale(&app, &list, tree.chain, "G-2", "Đang giảm", 200_000).await;
        drain(&app).await;
        let site = tree.chain.as_uuid();
        let (_, sale, _) =
            public(&app, &format!("/products?locale=vi&site={site}&on_sale=true"), None).await;
        assert_eq!(names(&sale), vec!["Đang giảm"]);
        assert_eq!(sale["data"][0]["sale_price"], vnd(160_000));
        assert!(sale["data"][0]["sale_ends_at"].is_string());
        let (_, all, _) = public(&app, &format!("/products?locale=vi&site={site}"), None).await;
        assert_eq!(all["data"].as_array().unwrap().len(), 2);
    })
    .await
    .unwrap();
}
