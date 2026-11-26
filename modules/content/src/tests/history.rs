use super::*;
use crate::publishing::redirects::Resolved;
use crate::publishing::schedules::Action;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;

fn title(resolved: Resolved) -> String {
    match resolved {
        Resolved::Entry(row) => row.title,
        other => panic!("{other:?}"),
    }
}

/// Revision: xem, khôi phục vào nháp (không tự đăng); dọn bớt giữ revision của bản công khai.
#[tokio::test]
async fn revision_xem_khoi_phuc_don_bot() {
    TestDatabase::run(async |test| {
        let (content, _, database) =
            service(test, config(&[("AUTOXIDE_CONTENT_MAX_REVISIONS", "10")])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let entry = content.create(&tenant, &ada, post("Version 0")).await.unwrap();
        content.publish(&tenant, &ada, entry.id, None, None, false).await.unwrap();
        for n in 1..=14 {
            let changes = EntryFields { title: Some(format!("Version {n}")), ..Default::default() };
            content.update(&tenant, &ada, entry.id, changes, true, None).await.unwrap();
        }
        // initial + published + 14 lần lưu.
        assert_eq!(content.revision_count(&tenant, entry.id).await, 16);
        let page = content
            .list_revisions(&tenant, &ada, entry.id, &PageRequest::first(50, Order::Asc))
            .await
            .unwrap();
        let published = page.data.iter().find(|row| row.reason == "published").unwrap().id;
        let first_save = page.data.iter().find(|row| row.reason == "explicit_save").unwrap().id;

        let restored = content.restore(&tenant, &ada, entry.id, first_save, None).await.unwrap();
        assert_eq!(
            (restored.title.as_str(), restored.has_unpublished_changes),
            ("Version 1", true)
        );
        assert_eq!(
            title(
                content
                    .resolve(
                        &tenant,
                        &autoxide_kernel::tenancy::Site::whole(&tenant),
                        "en",
                        "/blog/version-0"
                    )
                    .await
                    .unwrap()
            ),
            "Version 0"
        );
        assert!(events(&database).await.contains(&"content.entry_restored".to_owned()));

        let removed = content.prune_revisions(&database).await.unwrap();
        assert_eq!(
            content.revision_count(&tenant, entry.id).await,
            11,
            "10 newest + the published one"
        );
        assert_eq!(removed, 6);
        assert!(content.get_revision(&tenant, &ada, entry.id, published).await.is_ok());
        let wrong =
            content.get_revision(&tenant, &ada, Uuid::now_v7(), published).await.unwrap_err();
        assert_eq!(wrong.code(), "content.revision_not_found");
    })
    .await
    .unwrap();
}

/// Hẹn giờ đăng: đăng đúng bản nháp lúc đặt lịch; chạy lại không đăng hai lần; mỗi mục một
/// lịch đăng đang chờ; huỷ; lịch gỡ.
#[tokio::test]
async fn hen_gio_dang_va_go() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let entry = content.create(&tenant, &ada, post("Planned")).await.unwrap();
        let past = content
            .create_schedule(
                &tenant,
                &ada,
                entry.id,
                Action::Publish,
                Timestamp::now() - SignedDuration::from_mins(1),
                None,
            )
            .await;
        assert_eq!(past.unwrap_err().fields()[0].code, "schedule_in_past");
        let at = Timestamp::now() + SignedDuration::from_hours(1);
        let schedule = content
            .create_schedule(&tenant, &ada, entry.id, Action::Publish, at, None)
            .await
            .unwrap();
        let conflict = content
            .create_schedule(&tenant, &ada, entry.id, Action::Publish, at, None)
            .await
            .unwrap_err();
        assert_eq!(conflict.code(), "content.schedule_conflict");
        assert_eq!(
            count(
                &database,
                "SELECT COUNT(*) FROM content_jobs WHERE kind = 'content.run_schedule'"
            )
            .await,
            1
        );
        // Sửa nháp sau khi đặt lịch: lịch vẫn đăng bản lúc đặt.
        let later = EntryFields { title: Some("Edited later".into()), ..Default::default() };
        content.update(&tenant, &ada, entry.id, later, false, None).await.unwrap();

        content.run_schedule(&tenant, schedule.id, Timestamp::now()).await.unwrap();
        content.run_schedule(&tenant, schedule.id, Timestamp::now()).await.unwrap();
        assert_eq!(
            title(
                content
                    .resolve(
                        &tenant,
                        &autoxide_kernel::tenancy::Site::whole(&tenant),
                        "en",
                        "/blog/planned"
                    )
                    .await
                    .unwrap()
            ),
            "Planned"
        );
        let after = content.get(&tenant, &ada, entry.id).await.unwrap();
        assert_eq!((after.status.as_str(), after.has_unpublished_changes), ("published", true));
        let published = events(&database)
            .await
            .iter()
            .filter(|name| *name == "content.entry_published")
            .count();
        assert_eq!(published, 1, "running twice publishes once");
        let rows = content
            .list_schedules(&tenant, &ada, entry.id, &Default::default())
            .await
            .unwrap()
            .data;
        assert_eq!(rows[0].status, "succeeded");

        let take_down = content
            .create_schedule(&tenant, &ada, entry.id, Action::Unpublish, at, None)
            .await
            .unwrap();
        let canceled = content.cancel_schedule(&tenant, &ada, take_down.id).await.unwrap();
        assert_eq!(canceled.status, "canceled");
        assert_eq!(
            content.cancel_schedule(&tenant, &ada, take_down.id).await.unwrap_err().code(),
            "content.schedule_not_pending"
        );
        let take_down = content
            .create_schedule(&tenant, &ada, entry.id, Action::Unpublish, at, None)
            .await
            .unwrap();
        content.run_schedule(&tenant, take_down.id, Timestamp::now()).await.unwrap();
        assert_eq!(
            content
                .resolve(
                    &tenant,
                    &autoxide_kernel::tenancy::Site::whole(&tenant),
                    "en",
                    "/blog/planned"
                )
                .await
                .unwrap_err()
                .code(),
            "content.not_found"
        );
    })
    .await
    .unwrap();
}

/// Bản công khai được đăng sau khi đặt lịch: lịch thất bại `content.schedule_outdated`, không
/// ghi đè. Job chạy cùng lúc với đăng tay: không hỏng dữ liệu — trên cả hai database.
#[tokio::test]
async fn lich_loi_thoi_va_chay_dong_thoi() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let entry = content.create(&tenant, &ada, post("Original")).await.unwrap();
        let at = Timestamp::now() + SignedDuration::from_hours(1);
        let schedule = content
            .create_schedule(&tenant, &ada, entry.id, Action::Publish, at, None)
            .await
            .unwrap();
        let manual = EntryFields { title: Some("Manual".into()), ..Default::default() };
        content.update(&tenant, &ada, entry.id, manual, false, None).await.unwrap();
        content.publish(&tenant, &ada, entry.id, None, None, false).await.unwrap();
        content.run_schedule(&tenant, schedule.id, Timestamp::now()).await.unwrap();
        let rows = content
            .list_schedules(&tenant, &ada, entry.id, &Default::default())
            .await
            .unwrap()
            .data;
        assert_eq!(
            (rows[0].status.as_str(), rows[0].error_code.as_deref()),
            ("failed", Some("content.schedule_outdated"))
        );
        assert_eq!(
            title(
                content
                    .resolve(
                        &tenant,
                        &autoxide_kernel::tenancy::Site::whole(&tenant),
                        "en",
                        "/blog/original"
                    )
                    .await
                    .unwrap()
            ),
            "Manual"
        );

        for _ in 0..3 {
            let entry = content
                .create(&tenant, &ada, post(&format!("Race {}", Uuid::now_v7())))
                .await
                .unwrap();
            let schedule = content
                .create_schedule(&tenant, &ada, entry.id, Action::Publish, at, None)
                .await
                .unwrap();
            let entry = content.get(&tenant, &ada, entry.id).await.unwrap();
            let (job, manual) = {
                let (a, b) = (content.clone(), content.clone());
                let (ta, tb) = (tenant.clone(), tenant.clone());
                (
                    tokio::spawn(async move {
                        a.run_schedule(&ta, schedule.id, Timestamp::now()).await
                    }),
                    tokio::spawn(async move {
                        b.publish(&tb, &editor(), entry.id, Some(entry.updated_at), None, false)
                            .await
                    }),
                )
            };
            job.await.unwrap().unwrap();
            let manual = manual.await.unwrap();
            let rows = content
                .list_schedules(&tenant, &ada, entry.id, &Default::default())
                .await
                .unwrap()
                .data;
            let status = rows[0].status.clone();
            match manual {
                // Đăng tay trước: lịch lỗi thời (hoặc đã chạy trước nhưng lệnh tay thấy 412).
                Ok(_) => assert_eq!(status, "failed"),
                Err(error) => {
                    assert_eq!(error.kind(), ErrorKind::PreconditionFailed);
                    assert_eq!(status, "succeeded");
                }
            }
            assert_eq!(content.get(&tenant, &ada, entry.id).await.unwrap().status, "published");
        }
    })
    .await
    .unwrap();
}
