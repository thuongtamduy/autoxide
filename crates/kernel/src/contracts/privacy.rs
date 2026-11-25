//! Personal data: exporting and deleting one user's data across every module.
//!
//! - **Export:** a module that holds user data implements [`PersonalData`] and registers it with
//!   [`StartContext::personal_data`](crate::module::StartContext::personal_data).
//!   The account-management module gathers all parts with [`PersonalDataExport::export`].
//! - **Deletion:** the account-management module emits the [`USER_DELETED`] event when an account is
//!   deleted; other modules receive it through `Module::subscriptions` and delete that person's data
//!   (get the id with [`deleted_user`]). Events are redelivered until the receiver finishes, so the
//!   receiver must tolerate receiving it again.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use serde_json::Value;
use uuid::Uuid;

use crate::error::Error;
use crate::event::Delivered;
use crate::tenancy::TenantContext;

/// Name of the event emitted when a user account is deleted. `payload.before.id` is the id of the
/// user.
pub const USER_DELETED: &str = "auth.user_deleted";

/// User id of a [`USER_DELETED`] event; `None` if the event is different or has the wrong
/// shape.
pub fn deleted_user(event: &Delivered) -> Option<Uuid> {
    if event.name != USER_DELETED {
        return None;
    }
    event.payload.get("before")?.get("id")?.as_str()?.parse().ok()
}

/// A module exports the data it holds about a user.
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`.
#[async_trait::async_trait]
pub trait PersonalData: Send + Sync {
    /// All data of user `user_id` of tenant `tenant` that the module holds, as human-readable JSON
    /// (no secrets, hashes or tokens).
    async fn export(&self, tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error>;
}

/// Part of a module: (module name, exporter, the module is turned off — dormant mode, step 70 P9).
type Exporter = (&'static str, Arc<dyn PersonalData>, bool);

/// Registry of the registered [`PersonalData`] exporters, shared by all modules. Cloning is cheap: every
/// clone sees the same registry, including registrations made after the clone.
#[derive(Clone, Default)]
pub struct PersonalDataExport {
    // Ghi chỉ lúc khởi động; đọc mỗi lần xuất. `RwLock` của thư viện chuẩn: không bao
    // giờ giữ khoá qua một `await` (chép danh sách ra rồi mới gọi).
    exporters: Arc<RwLock<Vec<Exporter>>>,
}

impl PersonalDataExport {
    /// Registers the part for module `module`. Modules use
    /// [`StartContext::personal_data`](crate::module::StartContext::personal_data); calling this function
    /// directly is for tests only.
    pub fn register(
        &self,
        module: &'static str,
        exporter: Arc<dyn PersonalData>,
    ) -> Result<(), Error> {
        self.insert(module, exporter, false)
    }

    /// Registers the part of module `module`, which is turned off (dormant mode,
    /// D-20261125-dormant-modules): it is asked only for tenants whose database has the module's tables.
    pub(crate) fn register_dormant(
        &self,
        module: &'static str,
        exporter: Arc<dyn PersonalData>,
    ) -> Result<(), Error> {
        self.insert(module, exporter, true)
    }

    fn insert(
        &self,
        module: &'static str,
        exporter: Arc<dyn PersonalData>,
        dormant: bool,
    ) -> Result<(), Error> {
        let mut exporters = self.exporters.write().map_err(|_| poisoned())?;
        if exporters.iter().any(|(name, _, _)| *name == module) {
            return Err(Error::internal(format!(
                "module `{module}` registers personal data twice; register it once"
            )));
        }
        exporters.push((module, exporter, dormant));
        Ok(())
    }

    /// Names of the modules that registered a part (turned-off modules included), in registration order.
    pub fn modules(&self) -> Vec<&'static str> {
        self.exporters
            .read()
            .map(|exporters| exporters.iter().map(|(name, _, _)| *name).collect())
            .unwrap_or_default()
    }

    /// Data of user `user_id` of tenant `tenant` from every registered module, keyed by module
    /// name.
    pub async fn export(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<BTreeMap<&'static str, Value>, Error> {
        let exporters = self.exporters.read().map_err(|_| poisoned())?.clone();
        let mut data = BTreeMap::new();
        for (module, exporter, dormant) in exporters {
            // Module đang tắt mà database của khách chưa từng có bảng của nó: không có dữ liệu.
            if dormant && !crate::module::installed(module, tenant).await? {
                continue;
            }
            data.insert(module, exporter.export(tenant, user_id).await?);
        }
        Ok(data)
    }
}

fn poisoned() -> Error {
    Error::internal("the personal data registry is unusable after a panic")
}

// --- Thời hạn lưu theo loại dữ liệu (bước 37, D-360) -------------------------------------------

// Loại dữ liệu cá nhân theo Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15 (cần đối chiếu văn bản gốc).
/// Kind of personal data (Vietnam, Personal Data Protection Law 91/2025/QH15).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum DataCategory {
    /// Basic personal data (name, email, phone number, address…).
    Basic,
    /// Sensitive personal data (identity documents, biometrics, location…).
    Sensitive,
    /// Security traces (sessions, tokens, sign-in logs, hashed IP addresses).
    Security,
    /// Accounting records and legal proof the law requires to keep.
    Accounting,
}

impl DataCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            DataCategory::Basic => "basic",
            DataCategory::Sensitive => "sensitive",
            DataCategory::Security => "security",
            DataCategory::Accounting => "accounting",
        }
    }
}

/// How long it is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keep {
    /// Deleted after this many days.
    Days(u32),
    /// Kept until the user (or tenant) deletes it.
    UntilDeleted,
    /// Kept forever (only suitable for documents / evidence the law requires keeping).
    Forever,
}

/// A kind of data a module holds and its retention period (`Module::retention`). `keep` is read from the
/// module's current configuration (called after `configure`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// `<module>.<kind>`, e.g. `auth.sessions`.
    pub name: &'static str,
    /// English description for API readers (`GET /api/v1/compliance/retention`).
    pub description: &'static str,
    pub category: DataCategory,
    pub keep: Keep,
    /// Configuration variable that sets this period, if any.
    pub setting: Option<&'static str>,
}

impl RetentionPolicy {
    pub fn new(
        name: &'static str,
        description: &'static str,
        category: DataCategory,
        keep: Keep,
    ) -> Self {
        RetentionPolicy { name, description, category, keep, setting: None }
    }

    #[must_use]
    pub fn setting(mut self, name: &'static str) -> Self {
        self.setting = Some(name);
        self
    }
}

/// Registry of every module's retention periods (gathered once at startup, after `configure`). Cheap to clone.
#[derive(Clone, Debug, Default)]
pub struct RetentionCatalog(Arc<Vec<(&'static str, RetentionPolicy)>>);

impl RetentionCatalog {
    /// Registry from (module name, period); built by the kernel, built by hand in tests.
    pub fn new(policies: Vec<(&'static str, RetentionPolicy)>) -> Self {
        RetentionCatalog(Arc::new(policies))
    }

    /// Every entry, in module registration order.
    pub fn all(&self) -> &[(&'static str, RetentionPolicy)] {
        &self.0
    }
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

    #[test]
    fn catalog_thoi_han_luu_gom_moi_module() {
        let catalog = RetentionCatalog::new(vec![
            (
                "auth",
                RetentionPolicy::new(
                    "auth.sessions",
                    "Sessions.",
                    DataCategory::Security,
                    Keep::Days(30),
                )
                .setting("AUTOXIDE_AUTH_SESSION_DAYS"),
            ),
            (
                "notes",
                RetentionPolicy::new(
                    "notes.notes",
                    "Notes.",
                    DataCategory::Basic,
                    Keep::UntilDeleted,
                ),
            ),
        ]);
        let clone = catalog.clone();
        assert_eq!(clone.all().len(), 2);
        assert_eq!(clone.all()[0].1.setting, Some("AUTOXIDE_AUTH_SESSION_DAYS"));
        assert_eq!(clone.all()[1].0, "notes");
        assert_eq!(DataCategory::Sensitive.as_str(), "sensitive");
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
                occurred_at: crate::time::Timestamp::now(),
            };
            assert_eq!(deleted_user(&event), Some(id));
            event.name = "auth.user_updated".to_owned();
            assert_eq!(deleted_user(&event), None);
        })
        .await
        .unwrap();
    }
}
