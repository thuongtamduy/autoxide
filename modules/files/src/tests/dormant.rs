//! Chế độ nghỉ (bước 70 P9): `files` có trong chương trình nhưng đang tắt vẫn góp phần vào bản xuất dữ
//! liệu cá nhân và vẫn xoá file của người đó khi tài khoản bị xoá — bảng của nó còn từ hồi module chạy.

use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::module::Modules;
use autoxide_kernel::privacy::USER_DELETED;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::{TestApp, TestDatabase};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel};
use serde_json::json;
use uuid::Uuid;

/// App có `files` nhưng tắt, cùng bảng của `files` ở database chính như hồi nó còn chạy.
async fn dormant_app(test: &TestDatabase) -> (TestApp, TenantContext) {
    let app = TestApp::builder(test).module_turned_off(crate::module()).build().await.unwrap();
    assert!(app.modules().inactive().iter().any(|module| module.name == "files"));
    assert!(
        app.modules().user_deleted_receivers().contains(&"files"),
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
        crate::entity::Model {
            id,
            tenant_id: tenant.id().as_uuid(),
            owner_id: user,
            name: "notes.txt".into(),
            content_type: "text/plain".into(),
            size: 3,
            sha256: String::new(),
            created_at: now,
        }
        .into_active_model()
        .insert(app.database().write())
        .await
        .unwrap();

        let exported = app.modules().personal_data().export(&tenant, user).await.unwrap();
        assert_eq!(exported["files"][0]["name"], "notes.txt", "{exported:?}");

        // Xoá tài khoản khi `files` tắt: không có nơi lưu (`dyn Storage` do chính `files` công bố),
        // nên chỉ xoá dòng; nhận lại event cũng không lỗi.
        assert!(app.api::<dyn autoxide_kernel::storage::Storage>().is_none());
        let deleted = user_deleted(&tenant, user);
        let privacy = crate::service::DormantPrivacy { storage: None };
        privacy.handle(&deleted).await.unwrap();
        privacy.handle(&deleted).await.unwrap();
        let left = crate::entity::Entity::find_by_id(id)
            .scoped(&tenant)
            .one(app.database().read())
            .await
            .unwrap();
        assert!(left.is_none());
        let exported = app.modules().personal_data().export(&tenant, user).await.unwrap();
        assert_eq!(exported["files"], json!([]));
    })
    .await
    .unwrap();
}
