//! Quyền cho AI agent (D-109): `write_own` / `publish_own`, quyền xoá riêng, xin duyệt,
//! `ai_generated` / `ai_reviewed`.

use autoxide_kernel::access::Principal;
use sea_orm::EntityTrait;

use super::*;
use crate::entries::revisions::Snapshot;

/// Agent dùng API key `key` của người dùng `user_id`.
fn agent(user_id: Uuid, key: Uuid) -> Principal {
    Principal::new(
        user_id,
        ["read", "write_own", "publish_own"].iter().map(|p| format!("content.{p}")),
    )
    .via_api_key(key)
}

/// Chỉ sửa, đăng, gỡ mục do chính mình (cùng người, cùng key) tạo; không lưu trữ, không
/// xoá; xoá cần `content.delete` (kể cả người có `content.publish`).
#[tokio::test]
async fn chi_lam_tren_muc_cua_minh() {
    TestDatabase::run(async |test| {
        let (content, _, _) = service(test, config(&[])).await;
        let owner = Uuid::now_v7();
        let bot = agent(owner, Uuid::now_v7());
        let other_key = agent(owner, Uuid::now_v7());
        let ada = editor();

        let mine = content.create(&bot, post("Mine")).await.unwrap();
        assert_eq!(mine.created_by_api_key_id, bot.api_key_id());
        let theirs = content.create(&ada, post("Theirs")).await.unwrap();

        let mine = content
            .update(&bot, mine.id, fields("Mine, edited"), false, Some(mine.updated_at))
            .await
            .unwrap();
        for (who, id) in [(&other_key, mine.id), (&bot, theirs.id)] {
            let error = content.update(who, id, fields("x"), false, None).await.unwrap_err();
            assert_eq!((error.kind(), error.code()), (ErrorKind::Forbidden, "content.not_owner"));
            assert_eq!(
                content.publish(who, id, None, None, false).await.unwrap_err().code(),
                "content.not_owner"
            );
        }
        // Bản dịch: chỉ từ bản gốc của mình; bản dịch cũng là của mình.
        let vi = content.create_translation(&bot, mine.id, "vi", fields("Của tôi")).await;
        let vi = vi.unwrap();
        assert_eq!(vi.created_by_api_key_id, bot.api_key_id());
        assert_eq!(
            content
                .create_translation(&bot, theirs.id, "vi", fields("x"))
                .await
                .unwrap_err()
                .code(),
            "content.not_owner"
        );

        let mine = content.publish(&bot, mine.id, None, None, false).await.unwrap();
        assert_eq!(mine.status, "published");
        let mine = content.unpublish(&bot, mine.id, None).await.unwrap();
        for error in [
            content.archive(&bot, mine.id, None).await.unwrap_err(),
            content.delete(&bot, mine.id, None).await.unwrap_err(),
        ] {
            assert_eq!(error.code(), "permission_denied");
        }
        // Người có `content.publish` mà không có `content.delete` không xoá được.
        let publisher = user(&["read", "write", "publish"]);
        assert_eq!(
            content.delete(&publisher, theirs.id, None).await.unwrap_err().code(),
            "permission_denied"
        );
        content.delete(&ada, theirs.id, None).await.unwrap();
        // Không có quyền viết nào: 403 trước khi biết mục có tồn tại không.
        let reader = user(&["read"]);
        assert_eq!(
            content
                .update(&reader, Uuid::now_v7(), fields("x"), false, None)
                .await
                .unwrap_err()
                .code(),
            "permission_denied"
        );
    })
    .await
    .unwrap();
}

/// Xin duyệt rồi rút lại; người đăng xác nhận đã duyệt nội dung do AI tạo; API key không
/// xác nhận được; `ai_generated` đi qua revision và khôi phục.
#[tokio::test]
async fn xin_duyet_va_xac_nhan_da_duyet() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let bot = agent(Uuid::now_v7(), Uuid::now_v7());
        let ada = editor();
        let mut input = post("Written by a bot");
        input.fields.ai_generated = Some(true);
        let entry = content.create(&bot, input).await.unwrap();
        assert!(entry.ai_generated);

        let entry = content.set_review_request(&bot, entry.id, Some(entry.updated_at), false).await;
        let entry = entry.unwrap();
        assert!(entry.review_requested_at.is_some());
        assert_eq!(entry.review_requested_by, Some(bot.user_id()));
        let entry = content.set_review_request(&bot, entry.id, None, true).await.unwrap();
        assert_eq!(entry.review_requested_at, None);
        let entry = content.set_review_request(&bot, entry.id, None, false).await.unwrap();
        let names = events(&database).await;
        assert!(names.contains(&"content.review_requested".to_owned()));
        assert!(names.contains(&"content.review_withdrawn".to_owned()));

        assert_eq!(
            content.publish(&bot, entry.id, None, None, true).await.unwrap_err().code(),
            "content.ai_review_requires_human"
        );
        let entry = content.publish(&ada, entry.id, None, None, true).await.unwrap();
        assert_eq!(entry.review_requested_at, None);
        let published = crate::entity::published::Entity::find_by_id(entry.id)
            .one(database.read())
            .await
            .unwrap()
            .unwrap();
        assert!(published.ai_generated);
        assert_eq!(published.ai_reviewed_by, Some(ada.user_id()));
        assert!(published.ai_reviewed_at.is_some());
        // Đăng lại mà không xác nhận: không còn ghi là đã duyệt.
        content.publish(&ada, entry.id, None, None, false).await.unwrap();
        let published = crate::entity::published::Entity::find_by_id(entry.id)
            .one(database.read())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(published.ai_reviewed_by, None);

        // Revision giữ cờ; khôi phục đưa cờ về nháp.
        let revisions = content
            .list_revisions(&ada, entry.id, &PageRequest::first(50, Order::Asc))
            .await
            .unwrap();
        let first = &revisions.data[0];
        let snapshot: Snapshot = serde_json::from_str(&first.snapshot).unwrap();
        assert!(snapshot.ai_generated);
        let entry = content
            .update(
                &ada,
                entry.id,
                EntryFields { ai_generated: Some(false), ..Default::default() },
                false,
                None,
            )
            .await
            .unwrap();
        assert!(!entry.ai_generated);
        let entry = content.restore(&ada, entry.id, first.id, None).await.unwrap();
        assert!(entry.ai_generated);
        // Revision cũ (trước D-109) không có trường: đọc được, coi là `false`.
        let old: Snapshot = serde_json::from_value(json!({
            "title": "t", "slug": "t", "excerpt": "", "body": paragraph("x"),
            "featured_media_id": null, "featured_media_alt": "", "meta_title": "",
            "meta_description": "", "og_media_id": null, "canonical_url": "", "noindex": false
        }))
        .unwrap();
        assert!(!old.ai_generated);
    })
    .await
    .unwrap();
}
