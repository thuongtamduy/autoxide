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
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let other = editor();
        let mut profile = AuthorInput { display_name: Some("Ada".into()), ..Default::default() };
        profile.user_id = Some(Some(ada.user_id()));
        let author = content.create_author(&tenant, &ada, profile).await.unwrap();
        let kept = content
            .create_author(
                &tenant,
                &other,
                AuthorInput { display_name: Some("Bob".into()), ..Default::default() },
            )
            .await
            .unwrap();
        let entry = content.create(&tenant, &ada, post("Ada's post")).await.unwrap();
        content.set_entry_authors(&tenant, &ada, entry.id, vec![author.id, kept.id]).await.unwrap();
        content
            .create_schedule(
                &tenant,
                &ada,
                entry.id,
                Action::Publish,
                Utc::now() + Duration::days(1),
                None,
            )
            .await
            .unwrap();

        let exported = content.export(&tenant, ada.user_id()).await.unwrap();
        assert_eq!(exported["authors"][0]["display_name"], "Ada");
        assert_eq!(exported["entries"][0]["title"], "Ada's post");
        assert_eq!(
            content.export(&tenant, Uuid::now_v7()).await.unwrap(),
            json!({"authors": [], "entries": []})
        );

        let deleted = Delivered::new(
            tenant.clone(),
            "auth",
            autoxide_kernel::privacy::USER_DELETED.to_owned(),
            json!({"before": {"id": ada.user_id()}}),
        );
        content.handle(&deleted).await.unwrap();
        content.handle(&deleted).await.unwrap();

        let left =
            crate::taxonomy::authors::author_ids_of(tenant.database().read(), &tenant, &[entry.id])
                .await
                .unwrap();
        assert_eq!(left[&entry.id], [kept.id], "profile and its link removed, others kept");
        assert_eq!(
            content.get(&tenant, &other, entry.id).await.unwrap().created_by,
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
            content.export(&tenant, ada.user_id()).await.unwrap(),
            json!({"authors": [], "entries": []})
        );
    })
    .await
    .unwrap();
}
