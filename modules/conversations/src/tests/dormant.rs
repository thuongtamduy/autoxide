//! Chế độ nghỉ (bước 70 P9): `conversations` có trong chương trình nhưng đang tắt vẫn góp phần vào bản xuất dữ
//! liệu cá nhân và vẫn ẩn danh hoá phía người mua khi tài khoản bị xoá — bảng của nó còn từ hồi module chạy.

use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::module::Modules;
use autoxide_kernel::privacy::USER_DELETED;
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

        // Xoá tài khoản: cuộc trò chuyện bỏ liên kết với người mua và đóng lại; nhận lại event cũng vậy.
        let deleted = user_deleted(&tenant, user);
        crate::privacy::DormantPrivacy.handle(&deleted).await.unwrap();
        crate::privacy::DormantPrivacy.handle(&deleted).await.unwrap();
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
