//! Chế độ nghỉ (bước 70 P9): `analytics` có trong chương trình nhưng đang tắt vẫn góp phần vào bản xuất dữ
//! liệu cá nhân và vẫn gỡ `user_id` khỏi sự kiện khi tài khoản bị xoá — bảng của nó còn từ hồi module chạy.

use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::module::Modules;
use autoxide_kernel::privacy::USER_DELETED;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::{TestApp, TestDatabase};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel};
use serde_json::json;
use uuid::Uuid;

/// App có `analytics` nhưng tắt, cùng bảng của `analytics` ở database chính như hồi nó còn chạy.
async fn dormant_app(test: &TestDatabase) -> (TestApp, TenantContext) {
    let app = TestApp::builder(test).module_turned_off(crate::module()).build().await.unwrap();
    assert!(app.modules().inactive().iter().any(|module| module.name == "analytics"));
    assert!(
        app.modules().user_deleted_receivers().contains(&"analytics"),
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
        crate::entity::event::Model {
            id,
            tenant_id: tenant.id().as_uuid(),
            site_org_id: tenant.id().as_uuid(),
            occurred_at: now,
            day: "2026-11-25".into(),
            kind: "page_view".into(),
            session_key: "s".into(),
            user_id: Some(user),
            path: "/pricing".into(),
            referrer_host: None,
            source: "direct".into(),
            landing: true,
            utm_source: None,
            utm_medium: None,
            utm_campaign: None,
            utm_term: None,
            utm_content: None,
            object: None,
            props: None,
        }
        .into_active_model()
        .insert(app.database().write())
        .await
        .unwrap();

        let exported = app.modules().personal_data().export(&tenant, user).await.unwrap();
        assert_eq!(exported["analytics"]["events"][0]["path"], "/pricing", "{exported:?}");

        // Xoá tài khoản: sự kiện giữ lại nhưng không còn gắn với người đó.
        let deleted = user_deleted(&tenant, user);
        crate::privacy::DormantPrivacy.handle(&deleted).await.unwrap();
        crate::privacy::DormantPrivacy.handle(&deleted).await.unwrap();
        let event = crate::entity::event::Entity::find_by_id(id)
            .scoped(&tenant)
            .one(app.database().read())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.user_id, None);
        let exported = app.modules().personal_data().export(&tenant, user).await.unwrap();
        assert_eq!(exported["analytics"], json!({"events": []}));
    })
    .await
    .unwrap();
}
