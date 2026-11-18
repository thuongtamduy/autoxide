//! Test đầu-cuối của module `catalog` trên chương trình thật (bước 41): ảnh sản phẩm là media
//! thật của module `media` (phải công khai), sản phẩm mở bán, giá đọc qua `dyn Catalog` như module
//! thương mại sau sẽ đọc. Chạy trên cả SQLite và Postgres.

use autoxide_catalog::api::{Catalog, PriceQuery};
use autoxide_kernel::money::Currency;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, TenantContext, root_of};
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use crate::e2e::media::{admin_token, call, png, start};

#[tokio::test]
async fn san_pham_co_anh_media_va_gia_doc_qua_api() {
    TestDatabase::run(async |test| {
        let dir = std::env::temp_dir().join(format!("autoxide-catalog-{}", Uuid::now_v7()));
        let storage = [("AUTOXIDE_FILES_DIR", dir.display().to_string())];
        let runtime = start(test.url(), &storage).await;
        let token = admin_token(&runtime, "catalog@example.com").await;
        let auth = format!("Bearer {token}");
        let json_headers = [("authorization", auth.as_str()), ("content-type", "application/json")];

        // Ảnh tải lên mặc định riêng tư: chưa gắn được vào sản phẩm.
        let image_headers = [("authorization", auth.as_str()), ("content-type", "image/png")];
        let (status, _, media) =
            call(&runtime, "POST", "/api/v1/media?name=ao.png", &image_headers, png(40, 40)).await;
        assert_eq!(status, StatusCode::CREATED, "{media}");
        let media_id = media["data"]["id"].as_str().unwrap().to_owned();

        let product = json!({
            "kind": "physical", "tax_category": "vat.standard", "locale": "vi",
            "text": {"name": "Áo thun cotton"}, "variant": {"sku": "TEE-1", "weight_grams": 180},
        });
        let (status, _, created) = call(
            &runtime,
            "POST",
            "/api/v1/catalog/products",
            &json_headers,
            product.to_string().into_bytes(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let id = created["data"]["id"].as_str().unwrap().to_owned();
        let variant: Uuid = created["data"]["variants"][0]["id"].as_str().unwrap().parse().unwrap();

        let images = json!({"media": [{"media_id": media_id}]}).to_string();
        let uri = format!("/api/v1/catalog/products/{id}/media");
        let (status, _, problem) =
            call(&runtime, "PUT", &uri, &json_headers, images.clone().into_bytes()).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("catalog.media_not_public"))
        );
        let public = [
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
            ("if-match", "*"),
        ];
        let (status, _, _) = call(
            &runtime,
            "PATCH",
            &format!("/api/v1/media/{media_id}"),
            &public,
            json!({"visibility": "public"}).to_string().into_bytes(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _, with_image) =
            call(&runtime, "PUT", &uri, &json_headers, images.into_bytes()).await;
        assert_eq!(status, StatusCode::OK, "{with_image}");
        assert_eq!(with_image["data"]["media"][0]["media_id"], media_id.as_str());

        // Bảng giá ở gốc, rồi đọc giá như `orders` sẽ đọc.
        let list = json!({"name": "Bán lẻ", "currency": "VND", "tax_inclusive": true}).to_string();
        let (status, _, list) =
            call(&runtime, "POST", "/api/v1/catalog/price-lists", &json_headers, list.into_bytes())
                .await;
        assert_eq!(status, StatusCode::CREATED, "{list}");
        let prices = json!({
            "reference": "opening",
            "prices": [{"variant_id": variant, "amount": {"amount": "199000", "currency": "VND"}}],
        });
        let uri =
            format!("/api/v1/catalog/price-lists/{}/prices", list["data"]["id"].as_str().unwrap());
        let (status, _, added) =
            call(&runtime, "POST", &uri, &json_headers, prices.to_string().into_bytes()).await;
        assert_eq!(status, StatusCode::OK, "{added}");

        let catalog = runtime.modules.api::<dyn Catalog>().unwrap();
        let tenant = TenantContext::default_on(runtime.database.clone());
        let at = Utc::now() + chrono::Duration::seconds(1);
        let query = PriceQuery::new(root_of(DEFAULT_TENANT), Currency::VND, at, vec![variant]);
        let found = catalog.prices(&tenant, &query).await.unwrap();
        assert_eq!(found[0].amount.minor(), 199_000);
        let snapshot = catalog.variants(&tenant, &[variant], None).await.unwrap();
        assert_eq!((snapshot[0].sku.as_str(), snapshot[0].weight_grams), ("TEE-1", Some(180)));
    })
    .await
    .unwrap();
}

/// Đổi giá hàng loạt (D-222) khi cài đặt kế thừa `catalog.price_change_requires_second_person`
/// (của `organizations`) bật ở gốc: người tạo không tự duyệt được, người khác duyệt được.
#[tokio::test]
async fn nguoi_tao_khong_tu_duyet() {
    TestDatabase::run(async |test| {
        let dir = std::env::temp_dir().join(format!("autoxide-catalog-{}", Uuid::now_v7()));
        let storage = [("AUTOXIDE_FILES_DIR", dir.display().to_string())];
        let runtime = start(test.url(), &storage).await;
        let maker = format!("Bearer {}", admin_token(&runtime, "maker@example.com").await);
        let checker = format!("Bearer {}", admin_token(&runtime, "checker@example.com").await);
        let headers = |auth: &str| -> Vec<(&'static str, String)> {
            vec![("authorization", auth.to_owned()), ("content-type", "application/json".to_owned())]
        };
        let send = |auth: String, method: &'static str, uri: String, body: serde_json::Value| {
            let runtime = &runtime;
            async move {
                let owned = headers(&auth);
                let pairs: Vec<(&str, &str)> =
                    owned.iter().map(|(name, value)| (*name, value.as_str())).collect();
                let (status, _, body) =
                    call(runtime, method, &uri, &pairs, body.to_string().into_bytes()).await;
                (status, body)
            }
        };
        let root = root_of(DEFAULT_TENANT).as_uuid();
        let (status, set) = send(
            maker.clone(),
            "PUT",
            format!("/api/v1/organizations/units/{root}/settings/catalog.price_change_requires_second_person"),
            json!({"value": true}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{set}");

        let product = json!({
            "kind": "physical", "tax_category": "vat.standard", "locale": "vi",
            "text": {"name": "Hàng"}, "variant": {"sku": "S-1"},
        });
        let (status, made) =
            send(maker.clone(), "POST", "/api/v1/catalog/products".to_owned(), product).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        let variant = made["data"]["variants"][0]["id"].clone();
        let list = json!({"name": "Bán lẻ", "currency": "VND", "tax_inclusive": true});
        let (_, list) = send(maker.clone(), "POST", "/api/v1/catalog/price-lists".to_owned(), list).await;
        let list = list["data"]["id"].as_str().unwrap().to_owned();
        let prices = json!({"reference": "open", "prices": [{"variant_id": variant, "amount": {"amount": "10000", "currency": "VND"}}]});
        let (status, _) =
            send(maker.clone(), "POST", format!("/api/v1/catalog/price-lists/{list}/prices"), prices).await;
        assert_eq!(status, StatusCode::OK);
        let (_, change) = send(
            maker.clone(),
            "POST",
            format!("/api/v1/catalog/price-lists/{list}/changes"),
            json!({"operation": "amount", "value": 1000}),
        )
        .await;
        let change = format!(
            "/api/v1/catalog/price-lists/{list}/changes/{}",
            change["data"]["id"].as_str().unwrap()
        );
        let (status, _) = send(maker.clone(), "POST", format!("{change}/preview"), json!({})).await;
        assert_eq!(status, StatusCode::OK);
        let (status, problem) = send(maker.clone(), "POST", format!("{change}/approve"), json!({})).await;
        assert_eq!((status, problem["code"].as_str()), (StatusCode::FORBIDDEN, Some("catalog.self_approval")));
        let (status, approved) = send(checker.clone(), "POST", format!("{change}/approve"), json!({})).await;
        assert_eq!(status, StatusCode::OK, "{approved}");
    })
    .await
    .unwrap();
}
