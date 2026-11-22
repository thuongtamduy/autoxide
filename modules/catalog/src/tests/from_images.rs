//! Tạo sản phẩm nháp từ ảnh bằng AI (D-229): kết quả chỉ là sản phẩm nháp `ai_assisted`; danh mục
//! ngoài cây bị bỏ; thuộc tính chỉ giữ giá trị đúng kiểu của danh mục; ảnh không có trong `media` bị
//! từ chối ngay; mô hình không đọc ảnh → 503.

use autoxide_kernel::time::Timestamp;
use std::sync::{Arc, Mutex};

use autoxide_kernel::ai::{Part, TextModel};
use autoxide_media::api::testing::{FakeMedia, image};
use autoxide_media::api::{AiImage, Media};
use autoxide_testkit::ai::FakeTextModel;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use super::public::drain;
use super::{Tree, app_with, call};

const PHOTO: Uuid = Uuid::from_u128(0x0193_0000_0000_7000_8000_0000_0000_0101);

/// Câu trả lời của mô hình giả; `category` thay được giữa các bài.
fn model(category: Arc<Mutex<String>>) -> Arc<FakeTextModel> {
    Arc::new(FakeTextModel::new(move |_| {
        Ok(json!({
            "name": "Giày chạy bộ màu xanh",
            "short_description": "Giày chạy bộ nhẹ, đế êm.",
            "description": "Giày chạy bộ màu xanh.",
            "category_id": category.lock().unwrap().clone(),
            "attributes": [
                {"code": "color", "value": "blue"},
                {"code": "color", "value": "red"},
                {"code": "pairs", "value": "nhiều"},
                {"code": "unknown", "value": "x"}
            ],
            "meta_title": "Giày chạy bộ xanh",
            "meta_description": "Giày chạy bộ màu xanh, nhẹ và êm cho người mới tập chạy, đi hằng ngày cũng hợp."
        }))
    }))
}

/// Thư viện ảnh giả: chỉ có ảnh công khai `PHOTO`, gửi được cho AI.
fn photo_library() -> Arc<FakeMedia> {
    let media = FakeMedia::new();
    media.insert(image(PHOTO, true));
    media.set_ai_image(PHOTO, AiImage::new("image/jpeg", vec![0xff, 0xd8, 0xff]));
    Arc::new(media)
}

async fn shop(test: &TestDatabase, ai: Arc<FakeTextModel>) -> (TestApp, Tree) {
    app_with(test, move |builder| {
        builder.provide::<dyn TextModel>(ai).provide::<dyn Media>(photo_library())
    })
    .await
}

/// Danh mục "Giày" có thuộc tính `color` (chọn) và `pairs` (số nguyên).
async fn shoes(app: &TestApp) -> String {
    let body = json!({"texts": {"vi": {"name": "Giày"}}});
    let (_, made) = call(app, "POST", "/categories", "boss", Some(body)).await;
    let category = made["data"]["id"].as_str().unwrap().to_owned();
    let body = json!({"code": "color", "data_type": "option", "names": {"vi": "Màu"}});
    let (_, color) = call(app, "POST", "/attributes", "boss", Some(body)).await;
    let color = color["data"]["id"].as_str().unwrap().to_owned();
    let options = json!({"options": [{"code": "blue", "labels": {"vi": "Xanh"}}, {"code": "red", "labels": {"vi": "Đỏ"}}]});
    let (status, saved) =
        call(app, "PUT", &format!("/attributes/{color}/options"), "boss", Some(options)).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let body = json!({"code": "pairs", "data_type": "integer", "names": {"vi": "Số đôi"}});
    let (_, pairs) = call(app, "POST", "/attributes", "boss", Some(body)).await;
    let today = Timestamp::now().date_utc();
    let body = json!({"attributes": [
        {"attribute_id": color, "required": false, "effective_from": today},
        {"attribute_id": pairs["data"]["id"], "required": true, "effective_from": today}
    ]});
    let (status, saved) =
        call(app, "PUT", &format!("/categories/{category}/attributes"), "boss", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    category
}

/// Gửi yêu cầu, chạy job; trả lần tạo nháp đã xong.
async fn draft(app: &TestApp, body: Value) -> Value {
    let (status, made) = call(app, "POST", "/products/from-images", "boss", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    let id = made["data"]["id"].as_str().unwrap().to_owned();
    drain(app).await;
    let (status, found) =
        call(app, "GET", &format!("/products/from-images/{id}"), "boss", None).await;
    assert_eq!(status, StatusCode::OK, "{found}");
    found["data"].clone()
}

#[tokio::test]
async fn tao_tu_anh_chi_la_nhap() {
    TestDatabase::run(async |test| {
        let category = Arc::new(Mutex::new(String::new()));
        let ai = model(category.clone());
        let (app, tree) = shop(test, ai.clone()).await;
        let shoes = shoes(&app).await;
        *category.lock().unwrap() = shoes.clone();
        let body = json!({"media_ids": [PHOTO], "locale": "vi", "tax_category": "vat.standard",
            "org_id": tree.a.as_uuid()});
        let done = draft(&app, body).await;
        assert_eq!(done["status"], "succeeded", "{done}");
        let product = done["product_id"].as_str().unwrap();
        let (_, detail) = call(&app, "GET", &format!("/products/{product}"), "boss", None).await;
        let data = &detail["data"];
        assert_eq!(data["status"], "draft");
        assert_eq!(data["ai_assisted"], true);
        assert_eq!(data["kind"], "physical");
        assert_eq!(data["category_ids"], json!([shoes]));
        assert!(data["variants"][0]["sku"].as_str().unwrap().starts_with("AI-"));
        assert_eq!(data["media"][0]["media_id"], PHOTO.to_string(), "{data}");
        // Màu đầu tiên (đúng kiểu) được giữ; số đôi không phải số thì để trống → còn thiếu.
        let (_, values) =
            call(&app, "GET", &format!("/products/{product}/attributes"), "boss", None).await;
        assert_eq!(values["data"].as_array().unwrap().len(), 1, "{values}");
        assert_eq!(data["readiness"], "missing_attributes");
        // Gửi ảnh cho mô hình, kèm cây danh mục.
        let prompt = ai.prompts.lock().unwrap()[0].clone();
        assert_eq!(prompt.purpose, "catalog.draft_from_images");
        assert!(matches!(&prompt.input[1], Part::Image { .. }));
        let Part::Text(catalog) = &prompt.input[0] else { panic!("no catalog") };
        assert!(catalog.contains("Giày") && catalog.contains("pairs"), "{catalog}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn danh_muc_ngoai_cay_bi_bo() {
    TestDatabase::run(async |test| {
        let category = Arc::new(Mutex::new(Uuid::now_v7().to_string()));
        let (app, tree) = shop(test, model(category)).await;
        let body = json!({"media_ids": [PHOTO], "locale": "vi", "tax_category": "vat.standard",
            "org_id": tree.a.as_uuid()});
        let done = draft(&app, body).await;
        assert_eq!(done["status"], "succeeded", "{done}");
        let product = done["product_id"].as_str().unwrap();
        let (_, detail) = call(&app, "GET", &format!("/products/{product}"), "boss", None).await;
        assert_eq!(detail["data"]["category_ids"], json!([]));
        let (_, values) =
            call(&app, "GET", &format!("/products/{product}/attributes"), "boss", None).await;
        assert_eq!(values["data"], json!([]));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khong_co_anh_trong_media_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let ai = model(Arc::new(Mutex::new(String::new())));
        let (app, _) = shop(test, ai.clone()).await;
        let body =
            json!({"media_ids": [PHOTO, Uuid::now_v7()], "locale": "xx", "tax_category": "VAT"});
        let (status, problem) =
            call(&app, "POST", "/products/from-images", "boss", Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        let fields: Vec<&str> = problem["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["field"].as_str().unwrap())
            .collect();
        assert_eq!(fields, ["locale", "media_ids[1]", "tax_category"], "{problem}");
        assert_eq!(ai.calls(), 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn mo_hinh_khong_doc_anh_tra_503() {
    TestDatabase::run(async |test| {
        let blind = Arc::new(FakeTextModel::new(|_| Ok(json!({}))).without_images());
        let (app, _) = app_with(test, move |builder| {
            builder.provide::<dyn TextModel>(blind).provide::<dyn Media>(photo_library())
        })
        .await;
        let body = json!({"media_ids": [PHOTO], "locale": "vi", "tax_category": "vat.standard"});
        let (status, problem) =
            call(&app, "POST", "/products/from-images", "boss", Some(body)).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::SERVICE_UNAVAILABLE, Some("catalog.ai_images_unavailable"))
        );
    })
    .await
    .unwrap();
}
