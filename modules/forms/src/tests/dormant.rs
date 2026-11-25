//! Chế độ nghỉ (bước 70 P9): `forms` có trong chương trình nhưng đang tắt vẫn góp phần vào bản xuất dữ
//! liệu cá nhân và vẫn bỏ người tạo form khi tài khoản bị xoá — bảng của nó còn từ hồi module chạy.

use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::module::Modules;
use autoxide_kernel::privacy::USER_DELETED;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::{TestApp, TestDatabase};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel};
use serde_json::json;
use uuid::Uuid;

/// Module đứng tên cho module mà `forms` phụ thuộc (`mail`, `files`): bộ chọn module chỉ cần tên
/// có mặt; module tắt không dùng gì của chúng.
struct Stand(&'static str);

impl autoxide_kernel::module::Module for Stand {
    fn name(&self) -> &'static str {
        self.0
    }
}

/// App có `forms` nhưng tắt, cùng bảng của `forms` ở database chính như hồi nó còn chạy.
async fn dormant_app(test: &TestDatabase) -> (TestApp, TenantContext) {
    let app = TestApp::builder(test)
        .module(Stand("mail"))
        .module(Stand("files"))
        .module_turned_off(crate::module())
        .build()
        .await
        .unwrap();
    assert!(app.modules().inactive().iter().any(|module| module.name == "forms"));
    assert!(
        app.modules().user_deleted_receivers().contains(&"forms"),
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
        crate::entity::form::Model {
            id,
            tenant_id: tenant.id().as_uuid(),
            org_id: None,
            key: "contact".into(),
            name: "Contact".into(),
            status: "open".into(),
            fields: "[]".into(),
            version: 1,
            allowed_origins: "[]".into(),
            success_message: "{}".into(),
            notify_emails: "[]".into(),
            reply_to_field: None,
            ip_limit_per_hour: None,
            email_limit_per_hour: None,
            retention_days: None,
            created_by: Some(user),
            created_at: now,
            updated_at: now,
        }
        .into_active_model()
        .insert(app.database().write())
        .await
        .unwrap();

        let exported = app.modules().personal_data().export(&tenant, user).await.unwrap();
        assert_eq!(exported["forms"]["forms"][0]["key"], "contact", "{exported:?}");

        // Xoá tài khoản: form vẫn giữ, chỉ bỏ người tạo; nhận lại event cũng vậy.
        let deleted = user_deleted(&tenant, user);
        crate::privacy::DormantPrivacy.handle(&deleted).await.unwrap();
        crate::privacy::DormantPrivacy.handle(&deleted).await.unwrap();
        let form = crate::entity::form::Entity::find_by_id(id)
            .scoped(&tenant)
            .one(app.database().read())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(form.created_by, None);
        let exported = app.modules().personal_data().export(&tenant, user).await.unwrap();
        assert_eq!(exported["forms"], json!({"forms": []}));
    })
    .await
    .unwrap();
}
