//! Event `catalog.display_price_changed` (D-233): phát khi giá công khai đổi thật, không phát khi
//! dựng lại mà giá như cũ.

use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::{Value, json};

use super::app;
use super::call;
use super::public::{drain, on_sale, price_list};

/// `after.price.amount` của mọi event giá hiển thị, theo thứ tự ghi.
async fn display_prices(app: &TestApp) -> Vec<Value> {
    use sea_orm::{ConnectionTrait as _, Statement};
    let read = app.database().read();
    let sql = "SELECT payload FROM catalog_events WHERE name = 'catalog.display_price_changed' \
               ORDER BY id";
    read.query_all_raw(Statement::from_string(read.get_database_backend(), sql))
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            let payload: Value =
                serde_json::from_str(&row.try_get_by_index::<String>(0).unwrap()).unwrap();
            json!([payload["before"]["price"]["amount"], payload["after"]["price"]["amount"]])
        })
        .collect()
}

#[tokio::test]
async fn gia_hien_thi_doi_thi_phat_event() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let list = price_list(&app, tree.chain).await;
        let (_, variant) = on_sale(&app, &list, tree.chain, "D-1", "Đèn", 100_000).await;
        drain(&app).await;
        assert_eq!(display_prices(&app).await, vec![json!([null, "100000"])]);
        let body = json!({"reference": "cheaper", "prices": [{"variant_id": variant,
            "amount": {"amount": "90000", "currency": "VND"}}]});
        let (status, added) =
            call(&app, "POST", &format!("/price-lists/{list}/prices"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{added}");
        drain(&app).await;
        // Dựng lại cả khách mà giá như cũ: không thêm event.
        let tasks = app.modules().tasks().unwrap();
        let (_, task) =
            tasks.iter().find(|(name, _)| name == "catalog.rebuild_storefront").unwrap();
        task.run(&task.parse(&[]).unwrap()).await.unwrap();
        drain(&app).await;
        assert_eq!(
            display_prices(&app).await,
            vec![json!([null, "100000"]), json!(["100000", "90000"])]
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hai_lan_dung_lai_dong_thoi_chi_phat_mot_event() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let list = price_list(&app, tree.chain).await;
        let (_, variant) = on_sale(&app, &list, tree.chain, "D-2", "Quạt", 200_000).await;
        drain(&app).await;
        let body = json!({"reference": "cheaper", "prices": [{"variant_id": variant,
            "amount": {"amount": "150000", "currency": "VND"}}]});
        let (status, added) =
            call(&app, "POST", &format!("/price-lists/{list}/prices"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{added}");
        // Hai lần dựng lại cả khách cùng lúc, trước khi worker kịp chạy job của lần đổi giá:
        // Postgres từng cho cả hai cùng thấy giá cũ và cùng ghi event.
        let tasks = app.modules().tasks().unwrap();
        let (_, task) =
            tasks.iter().find(|(name, _)| name == "catalog.rebuild_storefront").unwrap();
        let args = task.parse(&[]).unwrap();
        let (first, second) = tokio::join!(task.run(&args), task.run(&args));
        first.unwrap();
        second.unwrap();
        drain(&app).await;
        assert_eq!(
            display_prices(&app).await,
            vec![json!([null, "200000"]), json!(["200000", "150000"])]
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doc_gia_cong_khai_theo_san_pham_va_bien_the() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let list = price_list(&app, tree.chain).await;
        let (product, variant) = on_sale(&app, &list, tree.chain, "D-3", "Bàn", 300_000).await;
        let (draft, _) = super::create_product(&app, "boss", tree.chain, "D-4", "Ghế").await;
        let product: uuid::Uuid = product.parse().unwrap();
        let variant: uuid::Uuid = variant.parse().unwrap();
        let draft: uuid::Uuid = draft.parse().unwrap();
        let catalog = app.api::<dyn crate::api::Catalog>().unwrap();
        let tenant = app.default_tenant();
        let prices = catalog.display_prices(&tenant, &[product, draft], &[variant]).await.unwrap();
        assert_eq!(prices.products.get(&product).map(|money| money.minor()), Some(300_000));
        assert_eq!(prices.variants.get(&variant).map(|money| money.minor()), Some(300_000));
        // Sản phẩm chưa bán (nháp): vắng.
        assert!(!prices.products.contains_key(&draft));
        let body = json!({"reference": "cheaper", "prices": [{"variant_id": variant,
            "amount": {"amount": "250000", "currency": "VND"}}]});
        let (status, added) =
            call(&app, "POST", &format!("/price-lists/{list}/prices"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{added}");
        let prices = catalog.display_prices(&tenant, &[product], &[]).await.unwrap();
        assert_eq!(prices.products.get(&product).map(|money| money.minor()), Some(250_000));
        assert!(prices.variants.is_empty());
    })
    .await
    .unwrap();
}
