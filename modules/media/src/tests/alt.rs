//! Gợi ý alt text (D-109): gửi ảnh thu nhỏ (không bao giờ ảnh gốc lớn), ảnh riêng tư chỉ khi
//! được phép, ảnh trang trí thì alt rỗng; không ghi gì.

use std::sync::Arc;

use autoxide_kernel::ai::Part;
use autoxide_testkit::ai::FakeTextModel;
use image::ImageFormat;

use super::*;
use crate::images::variants::samples::encoded;
use crate::testing::service_with_ai;

fn image_of(ai: &FakeTextModel, call: usize) -> (String, image::DynamicImage) {
    let prompt = ai.prompts.lock().unwrap()[call].clone();
    let Part::Image { media_type, bytes } = &prompt.input[0] else { panic!("no image") };
    (media_type.clone(), image::load_from_memory(bytes).unwrap())
}

#[tokio::test]
async fn goi_y_alt_tu_anh_thu_nho() {
    TestDatabase::run(async |test| {
        let ai = Arc::new(FakeTextModel::new(|prompt| {
            let Part::Text(context) = &prompt.input[1] else { panic!() };
            let decorative = context.contains("divider");
            Ok(json!({"alt": format!("  {} ", "x".repeat(300)), "decorative": decorative}))
        }));
        let (media, _, database) = service_with_ai(test, limits(), ai.clone(), false).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let ada = editor();
        // Ảnh lớn chưa có ảnh thu nhỏ: thu nhỏ tạm còn 1280 px.
        let big = encoded(2000, 1000, ImageFormat::Png, false);
        let item = media
            .upload(&tenant, &ada, upload("big.png", "image/png"), body(&big))
            .await
            .unwrap()
            .item;
        let changes = ItemChanges { visibility: Some(Visibility::Public), ..Default::default() };
        let item = media.update(&tenant, &ada, item.id, changes, None).await.unwrap();
        let suggestion =
            media.suggest_alt(&tenant, &ada, item.id, "vi", Some("Bài về Đà Lạt")).await.unwrap();
        assert_eq!(suggestion.alt.chars().count(), 250);
        assert!(!suggestion.decorative);
        let (media_type, sent) = image_of(&ai, 0);
        assert_eq!((media_type.as_str(), sent.width(), sent.height()), ("image/jpeg", 1280, 640));
        let prompt = ai.prompts.lock().unwrap()[0].clone();
        assert_eq!(prompt.purpose, "media.suggest_alt");
        assert!(prompt.instructions.contains("`vi`"));
        // Đã có ảnh thu nhỏ (cấu hình test: 100 và 200): gửi bản lớn nhất.
        media.make_variants(&tenant, item.id, item.version_id).await.unwrap();
        let decorative =
            media.suggest_alt(&tenant, &ada, item.id, "en", Some("a divider")).await.unwrap();
        assert_eq!((decorative.alt.as_str(), decorative.decorative), ("", true));
        assert_eq!(image_of(&ai, 1).1.width(), 200);
        // Không ghi gì.
        assert_eq!(media.get(&tenant, &ada, item.id).await.unwrap().alt, "");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn anh_rieng_tu_loai_khong_ho_tro_va_quyen() {
    TestDatabase::run(async |test| {
        let ai = Arc::new(FakeTextModel::new(|_| Ok(json!({"alt": "A cat", "decorative": false}))));
        let (media, _, database) = service_with_ai(test, limits(), ai.clone(), false).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let ada = editor();
        let small = encoded(40, 30, ImageFormat::Png, false);
        let private = media
            .upload(&tenant, &ada, upload("cat.png", "image/png"), body(&small))
            .await
            .unwrap()
            .item;
        let error = media.suggest_alt(&tenant, &ada, private.id, "en", None).await.unwrap_err();
        assert_eq!(
            (error.kind(), error.code()),
            (ErrorKind::Forbidden, "media.ai_private_not_allowed")
        );
        let pdf = media
            .upload(&tenant, &ada, upload("a.pdf", "application/pdf"), body(samples::PDF))
            .await
            .unwrap()
            .item;
        assert_eq!(
            media.suggest_alt(&tenant, &ada, pdf.id, "en", None).await.unwrap_err().code(),
            "media.ai_unsupported_image"
        );
        let bad = media
            .suggest_alt(&tenant, &ada, private.id, "not a code", Some(&"x".repeat(501)))
            .await
            .unwrap_err();
        assert_eq!(
            bad.fields().iter().map(|f| f.field.as_str()).collect::<Vec<_>>(),
            ["locale", "context"]
        );
        assert_eq!(
            media
                .suggest_alt(&tenant, &user(&["read"]), private.id, "en", None)
                .await
                .unwrap_err()
                .code(),
            "permission_denied"
        );
        assert_eq!(ai.calls(), 0);

        // Cho gửi ảnh riêng tư. Ảnh gốc đã nhỏ vẫn được làm lại (bỏ metadata), không
        // gửi chính file gốc (D-149).
        let (allowed, _, database) = service_with_ai(test, limits(), ai.clone(), true).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let item = allowed
            .upload(&tenant, &ada, upload("cat.png", "image/png"), body(&small))
            .await
            .unwrap()
            .item;
        assert_eq!(
            allowed.suggest_alt(&tenant, &ada, item.id, "en", None).await.unwrap().alt,
            "A cat"
        );
        let (media_type, sent) = image_of(&ai, 0);
        assert_eq!((media_type.as_str(), sent.width()), ("image/jpeg", 40));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ai_tat_hay_khong_doc_anh() {
    TestDatabase::run(async |test| {
        let (media, _, database) = service(test, limits()).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let ada = editor();
        let small = encoded(40, 30, ImageFormat::Png, false);
        let item = media
            .upload(&tenant, &ada, upload("cat.png", "image/png"), body(&small))
            .await
            .unwrap()
            .item;
        let changes = ItemChanges { visibility: Some(Visibility::Public), ..Default::default() };
        media.update(&tenant, &ada, item.id, changes, None).await.unwrap();
        assert_eq!(
            media.suggest_alt(&tenant, &ada, item.id, "en", None).await.unwrap_err().code(),
            "ai.disabled"
        );
    })
    .await
    .unwrap();
}
