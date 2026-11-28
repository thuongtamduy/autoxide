//! Chế độ nghỉ (bước 70 P9): `comments` có trong chương trình nhưng đang tắt vẫn góp phần vào bản xuất dữ
//! liệu cá nhân và vẫn xoá dữ liệu của người đó khi tài khoản bị xoá — bảng của nó còn từ hồi module chạy.

use autoxide_kernel::db::Queries as _;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::module::Modules;
use autoxide_kernel::privacy::USER_DELETED;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::{TestApp, TestDatabase};
use serde_json::json;
use uuid::Uuid;

/// Module đứng tên cho module mà `comments` phụ thuộc (`mail`): bộ chọn module chỉ cần tên
/// có mặt; module tắt không dùng gì của chúng.
struct Stand(&'static str);

impl autoxide_kernel::module::Module for Stand {
    fn name(&self) -> &'static str {
        self.0
    }
}

/// App có `comments` nhưng tắt, cùng bảng của `comments` ở database chính như hồi nó còn chạy.
async fn dormant_app(test: &TestDatabase) -> (TestApp, TenantContext) {
    let app = TestApp::builder(test)
        .module(Stand("mail"))
        .module_turned_off(crate::module())
        .build()
        .await
        .unwrap();
    assert!(app.modules().inactive().iter().any(|module| module.name == "comments"));
    assert!(
        app.modules().user_deleted_receivers().contains(&"comments"),
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
        let transaction = tenant.db().begin_write().await.unwrap();
        transaction
            .insert(crate::entity::profile::Model {
                tenant_id: autoxide_kernel::db::TenantKey::new(tenant.id()),
                user_id: user,
                display_name: "Ada".into(),
                name_key: "ada".into(),
                created_at: now,
                updated_at: now,
            })
            .exec()
            .await
            .unwrap();
        transaction.commit().await.unwrap();

        let exported = app.modules().personal_data().export(&tenant, user).await.unwrap();
        assert_eq!(exported["comments"]["profile"]["display_name"], "Ada", "{exported:?}");

        // Xoá tài khoản: hồ sơ bị xoá; nhận lại event cũng không lỗi.
        let deleted = user_deleted(&tenant, user);
        let privacy = crate::privacy::DormantPrivacy::new();
        privacy.handle(&deleted).await.unwrap();
        privacy.handle(&deleted).await.unwrap();
        let left = tenant
            .db()
            .read()
            .find::<crate::entity::profile::Model>()
            .filter(crate::entity::profile::Model::USER_ID.eq(user))
            .one()
            .await
            .unwrap();
        assert!(left.is_none());
        let exported = app.modules().personal_data().export(&tenant, user).await.unwrap();
        assert_eq!(exported["comments"]["profile"], json!(null));
    })
    .await
    .unwrap();
}
