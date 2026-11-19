//! Bộ sưu tập sản phẩm (bước 55): chọn tay có thứ tự và phân trang, theo quy tắc cho cùng kết quả
//! trên hai database, quy tắc sai bị từ chối, bộ sưu tập nháp không công khai, sổ đối tượng, quyền
//! theo nút.

use autoxide_kernel::resource::ResourceDirectory;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use super::public::{drain, names, on_sale, price_list, public};
use super::{app, call};

async fn create(app: &TestApp, who: &str, body: Value) -> (StatusCode, Value) {
    call(app, "POST", "/collections", who, Some(body)).await
}

async fn send_with(
    app: &TestApp,
    method: &str,
    uri: &str,
    version: &str,
    body: Value,
) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let answer = app
        .request(method, &format!("/api/v1/catalog{uri}"))
        .token("boss")
        .header("if-match", version)
        .json(body)
        .send()
        .await
        .unwrap();
    (answer.status(), answer.json())
}

#[tokio::test]
async fn bo_suu_tap_chon_tay_theo_thu_tu() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let list = price_list(&app, tree.chain).await;
        let (p1, _) = on_sale(&app, &list, tree.chain, "C-1", "Một", 100_000).await;
        let (p2, _) = on_sale(&app, &list, tree.chain, "C-2", "Hai", 200_000).await;
        let (p3, _) = on_sale(&app, &list, tree.chain, "C-3", "Ba", 300_000).await;
        drain(&app).await;
        let body =
            json!({"kind": "manual", "status": "active", "texts": {"vi": {"title": "Tuyển chọn"}}});
        let (status, made) = create(&app, "boss", body).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        let id = made["data"]["id"].as_str().unwrap().to_owned();
        assert_eq!(made["data"]["sort"], "manual");
        assert_eq!(made["data"]["texts"]["vi"]["slug"], "tuyen-chon");
        let items = json!({"product_ids": [p3, p1, p2]});
        let (status, saved) =
            send_with(&app, "PUT", &format!("/collections/{id}/items"), "*", items).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert_eq!(saved["data"]["items"], 3);

        let site = tree.chain.as_uuid();
        let (status, page, _) =
            public(&app, &format!("/collections/tuyen-chon?locale=vi&site={site}&limit=2"), None)
                .await;
        assert_eq!(status, StatusCode::OK, "{page}");
        let shown = |page: &Value| -> Vec<String> {
            page["data"]["products"]
                .as_array()
                .unwrap()
                .iter()
                .map(|card| card["name"].as_str().unwrap().to_owned())
                .collect()
        };
        assert_eq!(shown(&page), ["Ba", "Một"]);
        let cursor = page["data"]["pagination"]["next_cursor"].as_str().unwrap().to_owned();
        let (_, next, _) = public(
            &app,
            &format!("/collections/tuyen-chon?locale=vi&site={site}&limit=2&cursor={cursor}"),
            None,
        )
        .await;
        assert_eq!(shown(&next), ["Hai"]);
        assert_eq!(next["data"]["pagination"]["next_cursor"], Value::Null);

        // Đổi sang sắp theo giá giảm dần.
        let (status, changed) = send_with(
            &app,
            "PATCH",
            &format!("/collections/{id}"),
            "*",
            json!({"sort": "price_desc"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{changed}");
        let (_, page, _) =
            public(&app, &format!("/collections/tuyen-chon?locale=vi&site={site}"), None).await;
        assert_eq!(shown(&page), ["Ba", "Hai", "Một"]);

        // Nháp thì không công khai; sổ đối tượng cũng không đưa tiêu đề ra.
        let (status, _) = send_with(
            &app,
            "PATCH",
            &format!("/collections/{id}"),
            "*",
            json!({"status": "draft"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, missing, _) =
            public(&app, &format!("/collections/tuyen-chon?locale=vi&site={site}"), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");
        assert_eq!(missing["code"], "catalog.collection_not_found");
        let directory: ResourceDirectory = app.modules().resources();
        let tenant = app.default_tenant();
        let found = directory.resolve(&tenant, "catalog.collection", &id).await.unwrap().unwrap();
        assert!(!found.public && found.title.is_empty());
        send_with(&app, "PATCH", &format!("/collections/{id}"), "*", json!({"status": "active"}))
            .await;
        let found = directory.resolve(&tenant, "catalog.collection", &id).await.unwrap().unwrap();
        assert!(found.public);
        assert_eq!(found.url.as_deref(), Some("/collections/tuyen-chon"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bo_suu_tap_theo_quy_tac_dung_ca_hai_database() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let list = price_list(&app, tree.chain).await;
        for (sku, name, price) in [
            ("R-1", "Rẻ", 90_000),
            ("R-2", "Vừa A", 150_000),
            ("R-3", "Vừa B", 150_000),
            ("R-4", "Đắt", 900_000),
        ] {
            on_sale(&app, &list, tree.chain, sku, name, price).await;
        }
        drain(&app).await;
        let body = json!({
            "kind": "rule",
            "rules": {"price_min": 100_000, "price_max": 500_000},
            "sort": "price_asc",
            "status": "active",
            "texts": {"vi": {"title": "Tầm trung", "slug": "tam-trung"}},
        });
        let (status, made) = create(&app, "boss", body).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        assert_eq!(made["data"]["rules"]["match"], "all");
        let site = tree.chain.as_uuid();
        let (status, page, _) =
            public(&app, &format!("/collections/tam-trung?locale=vi&site={site}"), None).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        let shown: Vec<&str> = page["data"]["products"]
            .as_array()
            .unwrap()
            .iter()
            .map(|card| card["name"].as_str().unwrap())
            .collect();
        // Cùng giá: thứ tự ổn định theo ID (tạo trước đứng trước khi sắp tăng dần).
        assert_eq!(shown, ["Vừa A", "Vừa B"]);
        let _ = names;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn quy_tac_sai_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let (app, _tree) = app(test).await;
        let texts = json!({"vi": {"title": "X"}});
        let unknown = json!({"kind": "rule", "rules": {"category_ids": [uuid::Uuid::now_v7()]}, "texts": texts});
        let (status, body) = create(&app, "boss", unknown).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["code"], "catalog.rule_invalid");
        let empty = json!({"kind": "rule", "rules": {}, "texts": texts});
        assert_eq!(create(&app, "boss", empty).await.1["code"], "catalog.rule_invalid");
        let any = json!({"kind": "rule", "rules": {"price_max": 1, "match": "any"}, "texts": texts});
        assert_eq!(create(&app, "boss", any).await.1["code"], "catalog.rule_invalid");
        let promo = json!({"kind": "rule", "rules": {"on_promotion": true}, "texts": texts});
        assert_eq!(create(&app, "boss", promo).await.1["code"], "catalog.rule_needs_module");
        let manual_rules = json!({"kind": "manual", "rules": {"price_max": 1}, "texts": texts});
        assert_eq!(create(&app, "boss", manual_rules).await.0, StatusCode::UNPROCESSABLE_ENTITY);
        let manual_sort = json!({"kind": "rule", "rules": {"price_max": 1}, "sort": "manual", "texts": texts});
        assert_eq!(create(&app, "boss", manual_sort).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cua_hang_chi_quan_bo_suu_tap_cua_minh() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let texts = json!({"vi": {"title": "Của B"}});
        let body = json!({"kind": "manual", "org_id": tree.b.as_uuid(), "texts": texts});
        let (status, _) = create(&app, "store_a", body.clone()).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, made) = create(&app, "store_b", body).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        let id = made["data"]["id"].as_str().unwrap();
        let (status, _) = call(&app, "GET", &format!("/collections/{id}"), "store_a", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (_, listed) = call(&app, "GET", "/collections", "store_a", None).await;
        assert_eq!(listed["data"].as_array().unwrap().len(), 0);
        let (status, again) = create(
            &app,
            "store_b",
            json!({"kind": "manual", "org_id": tree.b.as_uuid(), "texts": texts}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{again}");
        assert_eq!(again["code"], "catalog.slug_taken");
    })
    .await
    .unwrap();
}
