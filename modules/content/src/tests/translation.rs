//! Dịch bằng AI (D-109): yêu cầu → job → nháp của bản dịch; không ghi đè bản người đã sửa; giữ
//! cấu trúc, link, ảnh; không bao giờ tự đăng.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::ai::{Part, Prompt};
use autoxide_kernel::error::Error;
use autoxide_testkit::ai::{FakeEmbedder, FakeTextModel};
use sea_orm::EntityTrait;

use super::*;
use crate::service::ContentService;
use crate::testing::service_with;
use autoxide_kernel::tenancy::TenantContext;

/// "Dịch" bằng cách thêm `[vi] ` trước mỗi mẩu (giữ nguyên thẻ).
fn translator() -> FakeTextModel {
    FakeTextModel::new(|prompt: &Prompt| {
        let Part::Text(input) = &prompt.input[0] else { panic!() };
        let items: Vec<Value> = serde_json::from_str(input).unwrap();
        let segments: Vec<Value> = items
            .iter()
            .map(|item| json!({"id": item["id"], "text": format!("[vi] {}", item["text"].as_str().unwrap())}))
            .collect();
        Ok(json!({"segments": segments}))
    })
}

fn rich_body(image: Uuid) -> Value {
    json!({"type": "doc", "content": [
        {"type": "paragraph", "content": [
            {"type": "text", "text": "See the "},
            {"type": "text", "text": "map", "marks": [{"type": "link", "attrs": {"href": "https://example.com/map"}}]}
        ]},
        {"type": "codeBlock", "content": [{"type": "text", "text": "let x = 1;"}]},
        {"type": "image", "attrs": {"mediaId": image.to_string(), "alt": "A lake", "caption": ""}}
    ]})
}

async fn setup(
    test: &TestDatabase,
    ai: FakeTextModel,
    pairs: &[(&str, &str)],
) -> (Arc<ContentService>, Arc<crate::testing::FakeMedia>, Database, Arc<FakeTextModel>) {
    let ai = Arc::new(ai);
    let (content, media, database) =
        service_with(test, config(pairs), ai.clone(), Arc::new(FakeEmbedder::disabled())).await;
    (content, media, database, ai)
}

async fn request(database: &Database, id: Uuid) -> crate::ai::translate::TranslationRequest {
    crate::entity::translation_request::Entity::find_by_id(id)
        .one(database.read())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn dich_moi_giu_cau_truc_va_khong_tu_dang() {
    TestDatabase::run(async |test| {
        let (content, media, database, ai) = setup(test, translator(), &[]).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let image = media.add(true);
        let mut input = post("Two days in Da Lat");
        input.fields.excerpt = Some("A short trip.".into());
        input.fields.body = Some(rich_body(image));
        let source = content.create(&tenant, &ada, input).await.unwrap();
        content.publish(&tenant, &ada, source.id, None, None, false).await.unwrap();

        let queued = content.request_translation(&tenant, &ada, source.id, "vi").await.unwrap();
        assert_eq!((queued.status.as_str(), queued.target_entry_id), ("queued", None));
        content.run_translation(&tenant, queued.id, 1).await.unwrap();
        let done = request(&database, queued.id).await;
        assert_eq!(done.status, "succeeded");
        assert_eq!(done.active_key, None);
        let vi = content.get(&tenant, &ada, done.result_entry_id.unwrap()).await.unwrap();
        assert_eq!((vi.locale.as_str(), vi.status.as_str()), ("vi", "draft"));
        assert_eq!(
            (vi.title.as_str(), vi.excerpt.as_str()),
            ("[vi] Two days in Da Lat", "[vi] A short trip.")
        );
        assert_eq!(vi.slug, "vi-two-days-in-da-lat");
        assert!(vi.ai_generated);
        assert_eq!(vi.translated_from_revision_id, Some(done.source_revision_id));
        assert_eq!(vi.translation_group_id, source.translation_group_id);
        let body: Value = serde_json::from_str(&vi.body).unwrap();
        assert_eq!(body["content"][0]["content"][0]["text"], "[vi] See the ");
        assert_eq!(
            body["content"][0]["content"][1]["marks"][0]["attrs"]["href"],
            "https://example.com/map"
        );
        assert_eq!(body["content"][1]["content"][0]["text"], "let x = 1;");
        assert_eq!(body["content"][2]["attrs"]["alt"], "[vi] A lake");
        assert_eq!(body["content"][2]["attrs"]["mediaId"], image.to_string());
        let prompt = ai.prompts.lock().unwrap()[0].clone();
        assert!(prompt.instructions.contains("`en`") && prompt.instructions.contains("`vi`"));
        let Part::Text(sent) = &prompt.input[0] else { panic!() };
        assert!(!sent.contains("example.com"), "the model never sees links");
        let names = events(&database).await;
        for name in ["content.translation_requested", "content.translation_succeeded"] {
            assert!(names.contains(&name.to_owned()), "{name}");
        }
        // Chạy lại job (thử lại): không làm gì thêm.
        content.run_translation(&tenant, queued.id, 2).await.unwrap();
        assert_eq!(ai.calls(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cap_nhat_ban_dich_co_san_va_bao_da_cu() {
    TestDatabase::run(async |test| {
        let (content, _, database, _) = setup(test, translator(), &[]).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let source = content.create(&tenant, &ada, post("Hello")).await.unwrap();
        let vi = content
            .create_translation(&tenant, &ada, source.id, "vi", fields("Xin chào"))
            .await
            .unwrap();
        let queued = content.request_translation(&tenant, &ada, source.id, "vi").await.unwrap();
        assert_eq!(queued.target_entry_id, Some(vi.id));
        content.run_translation(&tenant, queued.id, 1).await.unwrap();
        let updated = content.get(&tenant, &ada, vi.id).await.unwrap();
        assert_eq!(
            (updated.title.as_str(), updated.slug.as_str()),
            ("[vi] Hello", vi.slug.as_str())
        );
        let view = |entry| {
            let content = content.clone();
            let tenant = tenant.clone();
            async move {
                let described =
                    crate::http::describe(&content, &tenant, vec![entry]).await.unwrap();
                serde_json::to_value(&described[0]).unwrap()
            }
        };
        assert_eq!(view(updated.clone()).await["translation_outdated"], false);
        // Đăng bản gốc đổi `updated_at` nhưng không đổi chữ: chưa cũ.
        content.publish(&tenant, &ada, source.id, None, None, false).await.unwrap();
        assert_eq!(view(updated.clone()).await["translation_outdated"], false);
        content.update(&tenant, &ada, source.id, fields("Hello again"), false, None).await.unwrap();
        assert_eq!(view(updated).await["translation_outdated"], true);

        // Có người sửa bản dịch trong lúc chờ: không ghi đè.
        let queued = content.request_translation(&tenant, &ada, source.id, "vi").await.unwrap();
        content.update(&tenant, &ada, vi.id, fields("Sửa tay"), false, None).await.unwrap();
        content.run_translation(&tenant, queued.id, 1).await.unwrap();
        let failed = request(&database, queued.id).await;
        assert_eq!(
            (failed.status.as_str(), failed.error_code.as_deref()),
            ("failed", Some("content.translation_conflict"))
        );
        assert_eq!(content.get(&tenant, &ada, vi.id).await.unwrap().title, "Sửa tay");
        assert!(events(&database).await.contains(&"content.translation_failed".to_owned()));

        // Bản gốc không đổi (cùng revision với lần dịch trước), người sửa bản dịch sau khi
        // yêu cầu: vẫn là xung đột, không báo nhầm "thành công".
        let queued = content.request_translation(&tenant, &ada, source.id, "vi").await.unwrap();
        content.run_translation(&tenant, queued.id, 1).await.unwrap();
        assert_eq!(request(&database, queued.id).await.status, "succeeded");
        let again = content.request_translation(&tenant, &ada, source.id, "vi").await.unwrap();
        assert_eq!(again.source_revision_id, queued.source_revision_id);
        content.update(&tenant, &ada, vi.id, fields("Sửa tay lần hai"), false, None).await.unwrap();
        content.run_translation(&tenant, again.id, 1).await.unwrap();
        assert_eq!(
            request(&database, again.id).await.error_code.as_deref(),
            Some("content.translation_conflict")
        );
        assert_eq!(content.get(&tenant, &ada, vi.id).await.unwrap().title, "Sửa tay lần hai");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn sai_cau_truc_loi_tam_thoi_va_chia_lo() {
    TestDatabase::run(async |test| {
        // Bỏ mất thẻ: bị từ chối hai lần → thất bại, không ghi gì.
        let broken = FakeTextModel::new(|prompt: &Prompt| {
            let Part::Text(input) = &prompt.input[0] else { panic!() };
            let items: Vec<Value> = serde_json::from_str(input).unwrap();
            let segments: Vec<Value> =
                items.iter().map(|item| json!({"id": item["id"], "text": "no tags"})).collect();
            Ok(json!({"segments": segments}))
        });
        let (content, media, database, ai) = setup(test, broken, &[]).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let mut input = post("Hello");
        input.fields.body = Some(rich_body(media.add(true)));
        let source = content.create(&tenant, &ada, input).await.unwrap();
        let queued = content.request_translation(&tenant, &ada, source.id, "vi").await.unwrap();
        content.run_translation(&tenant, queued.id, 1).await.unwrap();
        let failed = request(&database, queued.id).await;
        assert_eq!(failed.error_code.as_deref(), Some("content.translation_invalid"));
        assert_eq!(ai.calls(), 2);

        // Nhà cung cấp bận: thử lại (lỗi trả về cho job queue), lần cuối thì thất bại.
        // Mã của module `ai` (không phải của `content`): để trong biến cho `cargo xtask check`.
        let code = "ai.provider_unavailable";
        let busy = FakeTextModel::new(move |_| Err(Error::unavailable(code, "busy")));
        let (content, _, database, _) = setup(test, busy, &[]).await;
        let tenant = TenantContext::default_on(database.clone());
        let source = content.create(&tenant, &ada, post("Hello")).await.unwrap();
        let queued = content.request_translation(&tenant, &ada, source.id, "vi").await.unwrap();
        assert_eq!(content.run_translation(&tenant, queued.id, 1).await.unwrap_err().code(), "ai.provider_unavailable");
        assert_eq!(request(&database, queued.id).await.status, "running");
        content.run_translation(&tenant, queued.id, 8).await.unwrap();
        let failed = request(&database, queued.id).await;
        assert_eq!((failed.status.as_str(), failed.error_code.as_deref()), ("failed", Some("ai.provider_unavailable")));

        // Bài dài: chia nhiều lô.
        let (content, _, _, ai) =
            setup(test, translator(), &[("AUTOXIDE_CONTENT_TRANSLATE_BATCH_TOKENS", "500")]).await;
        let paragraphs: Vec<Value> = (0..6)
            .map(|i| json!({"type": "paragraph", "content": [{"type": "text", "text": format!("{i} {}", "word ".repeat(80))}]}))
            .collect();
        let mut input = post("Long");
        input.fields.body = Some(json!({"type": "doc", "content": paragraphs}));
        let source = content.create(&tenant, &ada, input).await.unwrap();
        let queued = content.request_translation(&tenant, &ada, source.id, "vi").await.unwrap();
        content.run_translation(&tenant, queued.id, 1).await.unwrap();
        assert!(ai.calls() > 1, "{} calls", ai.calls());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dong_thoi_quyen_va_ai_tat() {
    TestDatabase::run(async |test| {
        let (content, _, database, _) = setup(test, translator(), &[]).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let source = content.create(&tenant, &ada, post("Hello")).await.unwrap();
        // Hai yêu cầu cùng lúc cho cùng một ngôn ngữ: đúng một được nhận.
        let attempts: Vec<_> = (0..4)
            .map(|_| {
                let content = content.clone();
                let tenant = tenant.clone();
                tokio::spawn(async move {
                    content.request_translation(&tenant, &editor(), source.id, "vi").await
                })
            })
            .collect();
        let mut accepted = Vec::new();
        for attempt in attempts {
            match attempt.await.unwrap() {
                Ok(request) => accepted.push(request),
                Err(error) => assert_eq!(error.code(), "content.translation_in_progress"),
            }
        }
        assert_eq!(accepted.len(), 1);
        content.run_translation(&tenant, accepted[0].id, 1).await.unwrap();
        // Xong rồi thì xin dịch lại được.
        content.request_translation(&tenant, &ada, source.id, "vi").await.unwrap();
        let bad = content.request_translation(&tenant, &ada, source.id, "en").await.unwrap_err();
        assert_eq!(bad.fields()[0].field, "locale");

        let stranger = Principal::new(Uuid::now_v7(), ["content.write_own".to_owned()])
            .via_api_key(Uuid::now_v7());
        assert_eq!(
            content
                .request_translation(&tenant, &stranger, source.id, "vi")
                .await
                .unwrap_err()
                .code(),
            "content.not_owner"
        );

        let (off, _, database) = service(test, config(&[])).await;

        let tenant = TenantContext::default_on(database.clone());
        let source = off.create(&tenant, &ada, post("Hello")).await.unwrap();
        assert_eq!(
            off.request_translation(&tenant, &ada, source.id, "vi").await.unwrap_err().code(),
            "ai.disabled"
        );
    })
    .await
    .unwrap();
}
