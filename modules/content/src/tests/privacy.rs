use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::privacy::PersonalData;
use autoxide_kernel::tenancy::TenantContext;
use chrono::{Duration, Utc};

use super::*;
use crate::publishing::schedules::Action;
use crate::taxonomy::authors::AuthorInput;

/// Xoá tài khoản: hồ sơ tác giả gắn với người đó bị xoá (cùng liên kết với bài), lịch họ
/// đặt bị huỷ, `created_by` bỏ trống; bài vẫn giữ. Nhận lại event không đổi kết quả.
/// Xuất dữ liệu: hồ sơ tác giả và các mục họ tạo.
#[tokio::test]
async fn xoa_tai_khoan_va_xuat_du_lieu() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let ada = editor();
        let other = editor();
        let mut profile = AuthorInput { display_name: Some("Ada".into()), ..Default::default() };
        profile.user_id = Some(Some(ada.user_id()));
        let author = content.create_author(&ada, profile).await.unwrap();
        let kept = content
            .create_author(
                &other,
                AuthorInput { display_name: Some("Bob".into()), ..Default::default() },
            )
            .await
            .unwrap();
        let entry = content.create(&ada, post("Ada's post")).await.unwrap();
        content.set_entry_authors(&ada, entry.id, vec![author.id, kept.id]).await.unwrap();
        content
            .create_schedule(&ada, entry.id, Action::Publish, Utc::now() + Duration::days(1), None)
            .await
            .unwrap();

        let exported = content
            .export(&TenantContext::legacy_default(&content.database), ada.user_id())
            .await
            .unwrap();
        assert_eq!(exported["authors"][0]["display_name"], "Ada");
        assert_eq!(exported["entries"][0]["title"], "Ada's post");
        assert_eq!(
            content
                .export(&TenantContext::legacy_default(&content.database), Uuid::now_v7())
                .await
                .unwrap(),
            json!({"authors": [], "entries": []})
        );

        let deleted = Delivered::new(
            autoxide_kernel::tenancy::TenantContext::legacy_default(&database),
            "auth",
            autoxide_kernel::privacy::USER_DELETED.to_owned(),
            json!({"before": {"id": ada.user_id()}}),
        );
        content.handle(&deleted).await.unwrap();
        content.handle(&deleted).await.unwrap();

        let left = crate::taxonomy::authors::author_ids_of(content.database.read(), &[entry.id])
            .await
            .unwrap();
        assert_eq!(left[&entry.id], [kept.id], "profile and its link removed, others kept");
        assert_eq!(
            content.get(&other, entry.id).await.unwrap().created_by,
            None,
            "entry kept, unlinked"
        );
        assert_eq!(
            count(&database, "SELECT COUNT(*) FROM content_schedules WHERE status = 'scheduled'")
                .await,
            0
        );
        assert_eq!(
            count(&database, "SELECT COUNT(*) FROM content_revisions WHERE created_by IS NOT NULL")
                .await,
            0
        );
        assert_eq!(
            content
                .export(&TenantContext::legacy_default(&content.database), ada.user_id())
                .await
                .unwrap(),
            json!({"authors": [], "entries": []})
        );
    })
    .await
    .unwrap();
}
