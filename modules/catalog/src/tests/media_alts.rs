//! `alt` của ảnh sản phẩm theo ngôn ngữ (D-202): bản ghi đè → `alt` của sản phẩm ở ngôn ngữ mặc
//! định → `alt` của media theo ngôn ngữ; bỏ ảnh khỏi sản phẩm thì bỏ bản ghi đè.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_media::api::{Media, MediaRef, MediaVisibility};
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use super::public::{drain, on_sale, price_list, public};
use super::{app_with, call};

/// Thư viện ảnh giả: mọi ID là ảnh công khai; `alt` tiếng Anh "Red shirt", mặc định "ảnh".
struct FakeMedia;

fn item(id: Uuid, alt: &str) -> MediaRef {
    MediaRef {
        id,
        visibility: MediaVisibility::Public,
        content_type: "image/jpeg".to_owned(),
        width: Some(800),
        height: Some(800),
        alt: alt.to_owned(),
        alt_ai_assisted: false,
        url: format!("/api/v1/media/{id}/content?v=1"),
        variants: Vec::new(),
    }
}

#[async_trait::async_trait]
impl Media for FakeMedia {
    async fn find_many(&self, _: &TenantContext, ids: &[Uuid]) -> Result<Vec<MediaRef>, Error> {
        Ok(ids.iter().map(|id| item(*id, "ảnh")).collect())
    }

    async fn find_many_in(
        &self,
        _: &TenantContext,
        ids: &[Uuid],
        locale: &str,
    ) -> Result<Vec<MediaRef>, Error> {
        let alt = if locale == "en" { "Red shirt" } else { "ảnh" };
        Ok(ids.iter().map(|id| item(*id, alt)).collect())
    }
}

async fn page_alt(app: &autoxide_testkit::TestApp, slug: &str, locale: &str, site: Uuid) -> Value {
    let uri = format!("/products/by-slug/{slug}?locale={locale}&site={site}");
    let (status, page, _) = public(app, &uri, None).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    page["data"]["images"][0]["alt"].clone()
}

#[tokio::test]
async fn alt_san_pham_theo_ngon_ngu_lui_ve_media() {
    TestDatabase::run(async |test| {
        let (app, tree) =
            app_with(test, |builder| builder.provide::<dyn Media>(Arc::new(FakeMedia))).await;
        let list = price_list(&app, tree.chain).await;
        let (product, _) = on_sale(&app, &list, tree.chain, "M-1", "Áo đỏ", 1).await;
        let (image, other) = (Uuid::now_v7(), Uuid::now_v7());
        let body = json!({"media": [{"media_id": image, "alt": "Áo thun màu đỏ"}]});
        let (status, put) =
            call(&app, "PUT", &format!("/products/{product}/media"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{put}");
        let body = json!({"name": "Red T-shirt"});
        let uri = format!("/products/{product}/translations/en");
        let (status, _) = call(&app, "PUT", &uri, "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        drain(&app).await;
        let site = tree.chain.as_uuid();
        // Ngôn ngữ mặc định: `alt` của sản phẩm; tiếng Anh chưa ghi đè: `alt` tiếng Anh của media.
        assert_eq!(page_alt(&app, "ao-do", "vi", site).await, "Áo thun màu đỏ");
        assert_eq!(page_alt(&app, "red-t-shirt", "en", site).await, "Red shirt");

        let alt_uri = format!("/products/{product}/media/{image}/alts/en");
        let (status, saved) =
            call(&app, "PUT", &alt_uri, "boss", Some(json!({"alt": "A red T-shirt"}))).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert_eq!(
            saved["data"]["media_alts"],
            json!([{"media_id": image, "locale": "en", "alt": "A red T-shirt"}])
        );
        assert_eq!(page_alt(&app, "red-t-shirt", "en", site).await, "A red T-shirt");
        // Ngôn ngữ chưa cấu hình; ảnh không thuộc sản phẩm.
        let (status, problem) = call(
            &app,
            "PUT",
            &format!("/products/{product}/media/{image}/alts/fr"),
            "boss",
            Some(json!({"alt": "x"})),
        )
        .await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("catalog.unsupported_locale"))
        );
        let (status, problem) = call(
            &app,
            "PUT",
            &format!("/products/{product}/media/{other}/alts/en"),
            "boss",
            Some(json!({"alt": "x"})),
        )
        .await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("catalog.product_media_not_found"))
        );
        // Cửa hàng B không sửa được sản phẩm của chuỗi.
        let (status, _) = call(&app, "PUT", &alt_uri, "store_b", Some(json!({"alt": "x"}))).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let (status, removed) = call(&app, "DELETE", &alt_uri, "boss", None).await;
        assert_eq!(status, StatusCode::OK, "{removed}");
        assert_eq!(page_alt(&app, "red-t-shirt", "en", site).await, "Red shirt");

        // Đổi ảnh: bản ghi đè của ảnh bị bỏ cũng mất.
        let (status, _) =
            call(&app, "PUT", &alt_uri, "boss", Some(json!({"alt": "A red T-shirt"}))).await;
        assert_eq!(status, StatusCode::OK);
        let body = json!({"media": [{"media_id": other}]});
        let (_, put) =
            call(&app, "PUT", &format!("/products/{product}/media"), "boss", Some(body)).await;
        assert_eq!(put["data"]["media_alts"], json!([]));
    })
    .await
    .unwrap();
}
