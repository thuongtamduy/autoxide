//! Chế độ nghỉ (bước 70 P9): `conversations` có trong chương trình nhưng đang tắt vẫn góp phần vào bản xuất dữ
//! liệu cá nhân và vẫn ẩn danh hoá phía người mua khi tài khoản bị xoá — bảng của nó còn từ hồi module chạy.

use std::sync::Arc;

use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::module::Modules;
use autoxide_kernel::privacy::USER_DELETED;
use autoxide_kernel::storage::{Storage, TenantStorage};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::{TestApp, TestDatabase};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel};
use serde_json::json;
use uuid::Uuid;

/// App có `conversations` nhưng tắt, cùng bảng của `conversations` ở database chính như hồi nó còn chạy.
async fn dormant_app(test: &TestDatabase) -> (TestApp, TenantContext) {
    let app = TestApp::builder(test).module_turned_off(crate::module()).build().await.unwrap();
    assert!(app.modules().inactive().iter().any(|module| module.name == "conversations"));
    assert!(
        app.modules().user_deleted_receivers().contains(&"conversations"),
        "{:?}",
        app.modules().user_deleted_receivers()
    );
    let migrator = Modules::new(vec![Box::new(crate::module())]).unwrap().migrator().unwrap();
    migrator.up(app.database()).await.unwrap();
    let tenant = app.default_tenant();
    (app, tenant)
}

fn user_deleted(tenant: &TenantContext, user: Uuid) -> Delivered {
    Delivered::new(tenant.clone(), "auth", USER_DELETED.to_owned(), json!({"before": {"id": user}}))
}

#[tokio::test]
async fn module_tat_van_xuat_va_xoa_du_lieu_ca_nhan() {
    TestDatabase::run(async |test| {
        let (app, tenant) = dormant_app(test).await;
        let user = Uuid::now_v7();
        let now = Timestamp::now();
        let id = Uuid::now_v7();
        crate::entity::thread::Model {
            id,
            tenant_id: tenant.id().as_uuid(),
            org_id: tenant.id().as_uuid(),
            customer_user_id: Some(user),
            subject: Some("Size question".into()),
            status: "open".into(),
            assignee_user_id: None,
            open_key: Some(format!("{user}")),
            unread_customer: 0,
            unread_staff: 0,
            auto_reply_paused: false,
            auto_replies_in_row: 0,
            needs_agent: false,
            ticket_id: None,
            draft_reply: None,
            draft_for: None,
            locale: None,
            last_message_at: now,
            closed_at: None,
            notify_customer_from: None,
            notify_staff_from: None,
            created_at: now,
            updated_at: now,
        }
        .into_active_model()
        .insert(app.database().write())
        .await
        .unwrap();

        let exported = app.modules().personal_data().export(&tenant, user).await.unwrap();
        assert_eq!(exported["conversations"][0]["subject"], "Size question", "{exported:?}");

        // Một cuộc gọi có bản ghi âm trong kho.
        let call_id = Uuid::now_v7();
        let key = format!("conversations/recordings/{call_id}");
        crate::entity::call::Model {
            id: call_id,
            tenant_id: tenant.id().as_uuid(),
            org_id: tenant.id().as_uuid(),
            thread_id: id,
            callback_id: None,
            direction: "inbound".into(),
            status: "completed".into(),
            provider: "fake".into(),
            provider_call_id: None,
            agent_user_id: None,
            caller_user_id: Some(user),
            started_at: Some(now),
            answered_at: Some(now),
            ended_at: Some(now),
            duration_seconds: Some(30),
            recording_enabled: true,
            recording_ack_at: Some(now),
            // Còn bản ghi ở nhà cung cấp mà không bộ nối thoại nào bật (D-20261127-voice-recording-provider-off).
            provider_recording_id: Some("rec-dormant".into()),
            recording_ref: Some(key.clone()),
            recording_expires_at: None,
            created_at: now,
            updated_at: now,
        }
        .into_active_model()
        .insert(app.database().write())
        .await
        .unwrap();
        let storage: Arc<dyn Storage> = Arc::new(autoxide_testkit::MemoryStorage::default());
        let store = TenantStorage::new(storage.clone(), &tenant);
        let body = futures_util::stream::iter([Ok(bytes::Bytes::from_static(b"audio"))]);
        store.put(&key, Box::pin(body)).await.unwrap();

        // Không có kho: không báo xong — lỗi để event được thử lại, bản ghi còn nguyên.
        let deleted = user_deleted(&tenant, user);
        let blind =
            crate::privacy::DormantPrivacy { tenancy: app.registry().clone(), voice: Vec::new() };
        assert!(app.registry().shared_storage().is_none());
        assert!(blind.handle(&deleted).await.is_err());
        assert!(store.get(&key).await.unwrap().is_some());

        // Có kho (của `files` đang bật hay `files` "nghỉ"): xoá cả bản ghi; cuộc trò chuyện bỏ liên kết với
        // người mua và đóng lại; nhận lại event cũng vậy.
        let eraser = crate::voice::recordings::RecordingEraser::Dormant {
            storage: Some(storage.clone()),
            voice: &[],
        };
        crate::privacy::forget_for_test(&eraser, &tenant, user).await.unwrap();
        crate::privacy::forget_for_test(&eraser, &tenant, user).await.unwrap();
        assert!(store.get(&key).await.unwrap().is_none(), "the recording is deleted");
        let call = crate::entity::call::Entity::find_by_id(call_id)
            .scoped(&tenant)
            .one(app.database().read())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(call.recording_ref, None);
        // Không bộ nối nào bật: event không lỗi, dấu vết ở nhà cung cấp còn, hạn lưu về bây giờ cho job dọn theo hạn
        // (chạy khi module bật lại).
        assert_eq!(call.provider_recording_id.as_deref(), Some("rec-dormant"));
        assert!(call.recording_expires_at.is_some_and(|at| at <= Timestamp::now()));
        assert_eq!(call.caller_user_id, None);
        let thread = crate::entity::thread::Entity::find_by_id(id)
            .scoped(&tenant)
            .one(app.database().read())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(thread.customer_user_id, None);
        assert_eq!(thread.status, crate::service::CLOSED);
        let exported = app.modules().personal_data().export(&tenant, user).await.unwrap();
        assert_eq!(exported["conversations"], json!([]));
    })
    .await
    .unwrap();
}
