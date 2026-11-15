//! Dữ liệu cá nhân: xuất và xoá dữ liệu của một người dùng xuyên mọi module.
//!
//! - **Xuất:** module nào giữ dữ liệu của người dùng thì cài [`PersonalData`] và đăng ký
//!   bằng [`StartContext::personal_data`](crate::module::StartContext::personal_data).
//!   Module quản lý tài khoản gom mọi phần lại bằng [`PersonalDataExport::export`].
//! - **Xoá:** module quản lý tài khoản phát event [`USER_DELETED`] khi một tài khoản bị
//!   xoá; module khác nhận nó qua `Module::subscriptions` rồi xoá dữ liệu của người đó
//!   (lấy id bằng [`deleted_user`]). Event được giao lại tới khi bên nhận xong việc, nên
//!   bên nhận phải chịu được việc nhận lại.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use serde_json::Value;
use uuid::Uuid;

use crate::error::Error;
use crate::event::Delivered;
use crate::tenancy::TenantContext;

/// Tên event phát khi một tài khoản người dùng bị xoá. `payload.before.id` là id của
/// người dùng.
pub const USER_DELETED: &str = "auth.user_deleted";

/// Id người dùng của một event [`USER_DELETED`]; `None` nếu event khác hoặc sai hình
/// dạng.
pub fn deleted_user(event: &Delivered) -> Option<Uuid> {
    if event.name != USER_DELETED {
        return None;
    }
    event.payload.get("before")?.get("id")?.as_str()?.parse().ok()
}

/// Một module xuất dữ liệu nó giữ về một người dùng.
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`.
#[async_trait::async_trait]
pub trait PersonalData: Send + Sync {
    /// Mọi dữ liệu của người dùng `user_id` của khách `tenant` mà module giữ, dạng JSON đọc
    /// được (không có secret, hash hay token).
    async fn export(&self, tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error>;
}

/// Phần của một module: (tên module, bộ xuất).
type Exporter = (&'static str, Arc<dyn PersonalData>);

/// Sổ các [`PersonalData`] đã đăng ký, dùng chung cho mọi module. Clone rẻ: mọi bản
/// clone thấy cùng một sổ, kể cả phần đăng ký sau khi clone.
#[derive(Clone, Default)]
pub struct PersonalDataExport {
    // Ghi chỉ lúc khởi động; đọc mỗi lần xuất. `RwLock` của thư viện chuẩn: không bao
    // giờ giữ khoá qua một `await` (chép danh sách ra rồi mới gọi).
    exporters: Arc<RwLock<Vec<Exporter>>>,
}

impl PersonalDataExport {
    /// Đăng ký phần của module `module`. Module dùng
    /// [`StartContext::personal_data`](crate::module::StartContext::personal_data); gọi
    /// thẳng hàm này chỉ dành cho test.
    pub fn register(
        &self,
        module: &'static str,
        exporter: Arc<dyn PersonalData>,
    ) -> Result<(), Error> {
        let mut exporters = self.exporters.write().map_err(|_| poisoned())?;
        if exporters.iter().any(|(name, _)| *name == module) {
            return Err(Error::internal(format!(
                "module `{module}` registers personal data twice; register it once in `start`"
            )));
        }
        exporters.push((module, exporter));
        Ok(())
    }

    /// Dữ liệu của người dùng `user_id` của khách `tenant` từ mọi module đã đăng ký, theo tên
    /// module.
    pub async fn export(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<BTreeMap<&'static str, Value>, Error> {
        let exporters = self.exporters.read().map_err(|_| poisoned())?.clone();
        let mut data = BTreeMap::new();
        for (module, exporter) in exporters {
            data.insert(module, exporter.export(tenant, user_id).await?);
        }
        Ok(data)
    }
}

fn poisoned() -> Error {
    Error::internal("the personal data registry is unusable after a panic")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Actor;
    use serde_json::json;

    struct Fixed(Value);

    #[async_trait::async_trait]
    impl PersonalData for Fixed {
        async fn export(
            &self,
            _tenant: &crate::tenancy::TenantContext,
            _: Uuid,
        ) -> Result<Value, Error> {
            Ok(self.0.clone())
        }
    }

    #[tokio::test]
    async fn gom_du_lieu_cua_moi_module_ke_ca_dang_ky_sau() {
        let registry = PersonalDataExport::default();
        let early = registry.clone();
        registry.register("notes", Arc::new(Fixed(json!([{"title": "a"}])))).unwrap();
        registry.register("auth", Arc::new(Fixed(json!({"email": "ada@example.com"})))).unwrap();
        assert!(registry.register("notes", Arc::new(Fixed(Value::Null))).is_err());

        let data = early.export(&crate::tenancy::test_tenant(), Uuid::now_v7()).await.unwrap();
        assert_eq!(data.keys().copied().collect::<Vec<_>>(), vec!["auth", "notes"]);
        assert_eq!(data["notes"][0]["title"], "a");
    }

    #[tokio::test]
    async fn doc_id_nguoi_dung_tu_event_xoa() {
        autoxide_testkit::TestDatabase::run(async |test| {
            let id = Uuid::now_v7();
            let source = crate::config::ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
            let mut reader = crate::config::ConfigReader::new(&source);
            let config = crate::config::DatabaseConfig::read(&mut reader);
            let database = crate::database::Database::connect(&reader.finish(config).unwrap().0)
                .await
                .unwrap();
            let tenant = crate::tenancy::TenantContext::default_on(database);
            let mut event = Delivered {
                id: Uuid::now_v7(),
                tenant,
                module: "auth",
                name: USER_DELETED.to_owned(),
                actor: Actor::system(),
                subject: None,
                payload: json!({"before": {"id": id}}),
                occurred_at: chrono::Utc::now(),
            };
            assert_eq!(deleted_user(&event), Some(id));
            event.name = "auth.user_updated".to_owned();
            assert_eq!(deleted_user(&event), None);
        })
        .await
        .unwrap();
    }
}
