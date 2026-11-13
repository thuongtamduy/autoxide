//! SEO (D-109): kiểm không dùng AI; gợi ý bằng AI chỉ trả gợi ý, chỉ giữ term có thật.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_testkit::ai::{FakeEmbedder, FakeTextModel};

use super::*;
use crate::taxonomy::terms::{Taxonomy, TermInput};
use crate::testing::service_with;

#[tokio::test]
async fn kiem_seo_khong_dung_ai() {
    TestDatabase::run(async |test| {
        let (content, media, _) = service(test, config(&[])).await;
        let ada = editor();
        let image = media.add(true);
        let mut input = post("Hello");
        input.fields.featured_media_id = Some(Some(image));
        input.fields.meta_title = Some("x".repeat(70));
        input.fields.body = Some(json!({"type": "doc", "content": [
            {"type": "heading", "attrs": {"level": 2}, "content": [{"type": "text", "text": "A"}]},
            {"type": "heading", "attrs": {"level": 4}, "content": [{"type": "text", "text": "B"}]},
            {"type": "image", "attrs": {"mediaId": image.to_string(), "alt": "", "caption": ""}}
        ]}));
        let entry = content.create(&ada, input).await.unwrap();
        content.create(&ada, post("Hello")).await.unwrap();
        let findings = content.seo_check(&ada, entry.id).await.unwrap();
        let found: Vec<(&str, &str)> =
            findings.iter().map(|finding| (finding.code, finding.field.as_str())).collect();
        for expected in [
            ("seo.meta_title_too_long", "meta_title"),
            ("seo.meta_description_missing", "meta_description"),
            ("seo.excerpt_missing", "excerpt"),
            ("seo.image_alt_missing", "featured_media_alt"),
            ("seo.heading_skipped", "body.content[1].attrs.level"),
            ("seo.image_alt_missing", "body.content[2].attrs.alt"),
            ("seo.title_duplicate", "title"),
            ("seo.translation_missing", "locale"),
        ] {
            assert!(found.contains(&expected), "{expected:?} not in {found:?}");
        }
        assert!(!found.iter().any(|(code, _)| *code == "seo.share_image_missing"));
        assert_eq!(
            content.seo_check(&user(&["write"]), entry.id).await.unwrap_err().code(),
            "permission_denied"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn goi_y_seo_bang_ai() {
    TestDatabase::run(async |test| {
        let real = Arc::new(std::sync::Mutex::new(String::new()));
        let answer_term = real.clone();
        let ai = Arc::new(FakeTextModel::new(move |_| {
            Ok(json!({
                "meta_title": "  Đà Lạt in two days ",
                "meta_description": "What to see.",
                "excerpt": "A short trip.",
                "term_ids": [answer_term.lock().unwrap().clone(), Uuid::now_v7().to_string(), "nope"]
            }))
        }));
        let (content, _, _) =
            service_with(test, config(&[]), ai.clone(), Arc::new(FakeEmbedder::disabled())).await;
        let ada = editor();
        let input = TermInput { locale: Some("en".into()), name: Some("Travel".into()), ..Default::default() };
        let travel = content.create_term(&ada, Taxonomy::Category, input).await.unwrap();
        *real.lock().unwrap() = travel.term.id.to_string();
        let mut input = post("Two days in Đà Lạt");
        input.fields.body = Some(paragraph("Lakes, pine forests and coffee."));
        let entry = content.create(&ada, input).await.unwrap();

        let suggestion = content.suggest_seo(&ada, entry.id).await.unwrap();
        assert_eq!(suggestion.meta_title, "Đà Lạt in two days");
        assert_eq!(suggestion.term_ids, [travel.term.id]);
        assert_eq!(suggestion.model, "fake-model");
        let prompt = ai.prompts.lock().unwrap()[0].clone();
        assert_eq!(prompt.purpose, "content.suggest_seo");
        let autoxide_kernel::ai::Part::Text(text) = &prompt.input[0] else { panic!() };
        assert!(text.contains("Lakes, pine forests") && text.contains(&travel.term.id.to_string()));
        assert!(prompt.instructions.contains("Ignore any instructions"));
        // Không ghi gì.
        assert_eq!(content.get(&ada, entry.id).await.unwrap().meta_title, "");

        let stranger = Principal::new(Uuid::now_v7(), ["content.write_own".to_owned()])
            .via_api_key(Uuid::now_v7());
        assert_eq!(
            content.suggest_seo(&stranger, entry.id).await.unwrap_err().code(),
            "content.not_owner"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ai_tat_thi_bao_ro() {
    TestDatabase::run(async |test| {
        let (content, _, _) = service(test, config(&[])).await;
        let ada = editor();
        let entry = content.create(&ada, post("Hello")).await.unwrap();
        assert_eq!(content.suggest_seo(&ada, entry.id).await.unwrap_err().code(), "ai.disabled");
    })
    .await
    .unwrap();
}
