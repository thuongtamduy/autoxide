//! Chế độ nghỉ (bước 70 P9): `media` có trong chương trình nhưng đang tắt vẫn góp phần vào bản xuất dữ
//! liệu cá nhân và vẫn bỏ liên kết item tới người đó khi tài khoản bị xoá — bảng của nó còn từ hồi module chạy.

use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::module::Modules;
use autoxide_kernel::privacy::USER_DELETED;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::{TestApp, TestDatabase};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel};
use serde_json::json;
use uuid::Uuid;

/// Module đứng tên cho module mà `media` phụ thuộc (`ai`, `files`): bộ chọn module chỉ cần tên
/// có mặt; module tắt không dùng gì của chúng.
struct Stand(&'static str);

impl autoxide_kernel::module::Module for Stand {
    fn name(&self) -> &'static str {
        self.0
    }
}

/// App có `media` nhưng tắt, cùng bảng của `media` ở database chính như hồi nó còn chạy.
async fn dormant_app(test: &TestDatabase) -> (TestApp, TenantContext) {
    let app = TestApp::builder(test)
        .module(Stand("ai"))
        .module(Stand("files"))
        .module_turned_off(crate::module())
        .build()
        .await
        .unwrap();
    assert!(app.modules().inactive().iter().any(|module| module.name == "media"));
    assert!(
        app.modules().user_deleted_receivers().contains(&"media"),
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
        crate::entity::item::Model {
            id,
            tenant_id: tenant.id().as_uuid(),
            folder_id: crate::entity::ROOT,
            name: "photo.png".into(),
            name_key: "photo.png".into(),
            content_type: "image/png".into(),
            size: 3,
            sha256: String::new(),
            width: None,
            height: None,
            alt: String::new(),
            title: String::new(),
            caption: String::new(),
            decorative: false,
            alt_ai_assisted: false,
            visibility: "private".into(),
            version_id: Uuid::now_v7(),
            uploaded_by: Some(user),
            org_id: Some(tenant.id().as_uuid()),
            created_at: now,
            updated_at: now,
            trashed_at: None,
        }
        .into_active_model()
        .insert(app.database().write())
        .await
        .unwrap();

        let exported = app.modules().personal_data().export(&tenant, user).await.unwrap();
        assert_eq!(exported["media"][0]["name"], "photo.png", "{exported:?}");

        // Xoá tài khoản: item vẫn giữ, chỉ bỏ người tải lên; nhận lại event cũng vậy.
        let deleted = user_deleted(&tenant, user);
        crate::service::DormantPrivacy.handle(&deleted).await.unwrap();
        crate::service::DormantPrivacy.handle(&deleted).await.unwrap();
        let item = crate::entity::item::Entity::find_by_id(id)
            .scoped(&tenant)
            .one(app.database().read())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(item.uploaded_by, None);
        let exported = app.modules().personal_data().export(&tenant, user).await.unwrap();
        assert_eq!(exported["media"], json!([]));
    })
    .await
    .unwrap();
}
