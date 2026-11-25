//! Chế độ nghỉ (bước 70 P9): `support` có trong chương trình nhưng đang tắt vẫn góp phần vào bản xuất dữ
//! liệu cá nhân và vẫn ẩn danh hoá phiếu khi tài khoản bị xoá — bảng của nó còn từ hồi module chạy.

use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::module::Modules;
use autoxide_kernel::privacy::USER_DELETED;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::{TestApp, TestDatabase};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel};
use serde_json::json;
use uuid::Uuid;

/// App có `support` nhưng tắt, cùng bảng của `support` ở database chính như hồi nó còn chạy.
async fn dormant_app(test: &TestDatabase) -> (TestApp, TenantContext) {
    let app = TestApp::builder(test).module_turned_off(crate::module()).build().await.unwrap();
    assert!(app.modules().inactive().iter().any(|module| module.name == "support"));
    assert!(
        app.modules().user_deleted_receivers().contains(&"support"),
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
        crate::entity::ticket::Model {
            id,
            tenant_id: tenant.id().as_uuid(),
            org_id: tenant.id().as_uuid(),
            number: "T-1".into(),
            requester_user_id: Some(user),
            requester_email: Some("ada@example.com".into()),
            requester_phone: None,
            requester_name: "Ada".into(),
            locale: None,
            verified_at: None,
            token_hash: None,
            otp_hash: None,
            otp_expires_at: None,
            otp_attempts: 0,
            category: "question".into(),
            priority: "normal".into(),
            status: "open".into(),
            resolution: None,
            subject: "Where is my parcel?".into(),
            channel: "web".into(),
            assignee_user_id: None,
            team_id: None,
            first_response_due_at: None,
            resolve_due_at: None,
            first_responded_at: None,
            sla_paused_at: None,
            sla_paused_minutes: 0,
            solved_at: None,
            closed_at: None,
            breached: false,
            case_id: None,
            dispute_id: None,
            follow_up_of: None,
            escalated_to_platform: false,
            satisfaction: None,
            satisfaction_note: None,
            auto_reply_paused: false,
            auto_replies_in_row: 0,
            last_message_at: now,
            reference: None,
            search_key: String::new(),
            created_at: now,
            updated_at: now,
        }
        .into_active_model()
        .insert(app.database().write())
        .await
        .unwrap();

        let exported = app.modules().personal_data().export(&tenant, user).await.unwrap();
        assert_eq!(exported["support"][0]["subject"], "Where is my parcel?", "{exported:?}");

        // Xoá tài khoản: phiếu giữ lại nhưng ẩn danh hoá; nhận lại event cũng vậy.
        let deleted = user_deleted(&tenant, user);
        crate::privacy::DormantPrivacy.handle(&deleted).await.unwrap();
        crate::privacy::DormantPrivacy.handle(&deleted).await.unwrap();
        let ticket = crate::entity::ticket::Entity::find_by_id(id)
            .scoped(&tenant)
            .one(app.database().read())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ticket.requester_user_id, None);
        assert_eq!(ticket.requester_email, None);
        assert_eq!(ticket.requester_name, "");
        let exported = app.modules().personal_data().export(&tenant, user).await.unwrap();
        assert_eq!(exported["support"], json!([]));
    })
    .await
    .unwrap();
}
