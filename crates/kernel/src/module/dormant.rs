//! Dormant mode (step 70 P9, D-20261125-dormant-modules): a module that is in the program but turned off
//! (`AUTOXIDE_MODULES`) still takes part in personal data and the tenant lifecycle, so its data is never
//! left out when a person asks for their data or for its deletion, or when a tenant is moved or deleted
//! (Vietnam, Personal Data Protection Law 91/2025/QH15 and Decree 13/2023/ND-CP; check the original texts).
//!
//! - A turned-off module never starts: no routes, no MCP tools, no jobs, no event other than
//!   [`USER_DELETED`](crate::privacy::USER_DELETED), no migrations on a database that never had its tables.
//! - Instead the kernel calls [`Module::dormant`](super::Module::dormant) after every enabled module has
//!   started. The module registers there its part of a person's data export
//!   ([`DormantContext::personal_data`]) and what it deletes when an account is deleted
//!   ([`DormantContext::on_user_deleted`]) — usually the same objects it registers in `start`.
//! - The kernel calls those parts only for a tenant whose database has the module's tables (its migration
//!   history): a database that never ran the module holds none of its data.
//! - Tenant lifecycle needs nothing from the module: deleting, exporting and moving a tenant cover the tables
//!   of turned-off modules too, by their declarations (`Module::retained_tables`,
//!   `Module::control_plane_tables`, `Module::relocation`, `Module::migrations`).

use std::sync::Arc;

use super::{Apis, Module, ModuleError};
use crate::error::Error;
use crate::event::{Delivered, EventHandler, Subscription};
use crate::i18n::Translations;
use crate::privacy::{PersonalData, PersonalDataExport, USER_DELETED};
use crate::tenancy::{DatabaseRegistry, TenantContext};

/// What a turned-off module can use and register in [`Module::dormant`](super::Module::dormant).
///
/// Boundary: a turned-off module reads its configuration and may open its own resources (e.g. its file store)
/// only to export, delete and move personal data. Nothing registered here serves requests: no routes, MCP
/// tools, jobs, schedules or events other than [`USER_DELETED`](crate::privacy::USER_DELETED), and no trait for
/// other modules. [`tenancy`](Self::tenancy) hands out the whole database registry because a dormant part reads
/// a tenant's database at call time; a dormant part uses it only to reach tenant databases and the shared file
/// store, never to change the registry.
pub struct DormantContext<'a> {
    pub(super) module: &'static str,
    pub(super) apis: &'a Apis,
    pub(super) tenancy: DatabaseRegistry,
    pub(super) translations: &'a Translations,
    pub(super) personal_data: &'a PersonalDataExport,
    pub(super) subscriptions: &'a mut Vec<(&'static str, Subscription)>,
    pub(super) sync_keep: jiff::SignedDuration,
    pub(super) storage: &'a mut Option<SharedStorage>,
}

/// What setting up the turned-off modules gives back: their account-deletion subscriptions and the file store
/// one of them opened, if any.
pub(super) type Started = (Vec<(&'static str, Subscription)>, Option<SharedStorage>);

/// The file store a turned-off module opened itself, with its opener of placement stores.
pub(super) type SharedStorage =
    (Arc<dyn crate::storage::Storage>, Option<Arc<dyn crate::storage::StorageOpener>>);

impl DormantContext<'_> {
    /// The name of the module being set up.
    pub fn module(&self) -> &'static str {
        self.module
    }

    /// The database registry: a tenant's database through `tenancy().tenant(id)`.
    pub fn tenancy(&self) -> DatabaseRegistry {
        self.tenancy.clone()
    }

    /// Translations of every module.
    pub fn translations(&self) -> &Translations {
        self.translations
    }

    /// The trait `T` that the kernel or an ENABLED module published (e.g. `dyn Storage` of `files`,
    /// `SecretKey`); `None` when no enabled module publishes it. A turned-off module never publishes
    /// anything, so a dormant part must work without other turned-off modules.
    pub fn api<T>(&self) -> Option<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.apis.get::<T>()
    }

    /// Where the module writes the change stream for offline apps (`<module>_changes`), as in
    /// [`StartContext::sync_log`](super::StartContext::sync_log): deleting a person's data records the
    /// deletions there too.
    pub fn sync_log(&self) -> crate::sync::SyncLog {
        crate::sync::SyncLog::new(self.module, self.sync_keep)
    }

    /// Traits that the kernel or ENABLED modules contribute to (`StartContext::contribute`), e.g. provider
    /// connectors a dormant part must call to delete a person's data held at the provider.
    pub fn contributions<T>(&self) -> Vec<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.apis.contributions::<T>()
    }

    /// Offers the file store the turned-off module opened from its own configuration (the `files` module
    /// does this): when no enabled module publishes `dyn Storage`, the kernel uses it for the tenant
    /// lifecycle and other dormant parts reach it through `tenancy().shared_storage()`, so stored content is
    /// deleted too. Only the first offer is kept.
    // Lối riêng cho `files` (module duy nhất mở kho): ẩn khỏi tài liệu công khai, không thuộc hợp đồng ổn định
    // (D-20261123-compatibility-policy không cam kết phần `#[doc(hidden)]`).
    #[doc(hidden)]
    pub fn share_storage(
        &mut self,
        storage: Arc<dyn crate::storage::Storage>,
        opener: Option<Arc<dyn crate::storage::StorageOpener>>,
    ) {
        if self.storage.is_none() {
            *self.storage = Some((storage, opener));
        }
    }

    /// Registers the module's part of a person's data export (the same [`PersonalData`] it registers in
    /// `start`). Called only for tenants whose database has the module's tables.
    pub fn personal_data(&mut self, exporter: Arc<dyn PersonalData>) -> Result<(), Error> {
        self.personal_data.register_dormant(self.module, exporter)
    }

    /// Registers what the module deletes or anonymizes when an account is deleted
    /// ([`USER_DELETED`]); the same handler it subscribes when enabled. Called only for events of tenants
    /// whose database has the module's tables. Must tolerate the same event twice, like every receiver.
    pub fn on_user_deleted(&mut self, handler: Arc<dyn EventHandler>) -> Result<(), Error> {
        if self.subscriptions.iter().any(|(module, _)| *module == self.module) {
            return Err(Error::internal(format!(
                "module `{}` registers its dormant `{USER_DELETED}` handler twice; register it once",
                self.module
            )));
        }
        let handler = Arc::new(WhenInstalled { module: self.module, inner: handler });
        self.subscriptions.push((self.module, Subscription::new(USER_DELETED, handler)));
        Ok(())
    }
}

/// Calls [`Module::dormant`] of every turned-off module, after every enabled module has started; returns the
/// account-deletion subscriptions they registered.
pub(super) fn start(
    dormant: &[Box<dyn Module>],
    apis: &Apis,
    tenancy: &DatabaseRegistry,
    translations: &Translations,
    personal_data: &PersonalDataExport,
    sync_keep: jiff::SignedDuration,
) -> Result<Started, ModuleError> {
    let mut subscriptions = Vec::new();
    let mut storage = None;
    for module in dormant {
        let name = module.name();
        let mut context = DormantContext {
            module: name,
            apis,
            tenancy: tenancy.clone(),
            translations,
            personal_data,
            subscriptions: &mut subscriptions,
            sync_keep,
            storage: &mut storage,
        };
        module.dormant(&mut context).map_err(|error| {
            ModuleError::new(format!(
                "turned-off module `{name}` failed to set up its dormant part: {error}"
            ))
        })?;
        // Kho module tắt vừa mở thì module tắt đăng ký sau thấy được ngay (vd `conversations` sau `files`).
        if let Some((shared, opener)) = &storage
            && apis.get::<dyn crate::storage::Storage>().is_none()
            && tenancy.shared_storage().is_none()
        {
            tenancy.use_storage(Some(shared.clone()), opener.clone());
        }
    }
    Ok((subscriptions, storage))
}

/// One migrator per turned-off module (its own migrations only), keyed on the control database like the
/// placements' migrator: a move applies them only where the module already has tables.
pub(super) fn migrators(
    dormant: &[Box<dyn Module>],
    keys: &crate::secret::KeyConfig,
    tenancy: &DatabaseRegistry,
) -> Result<Vec<(&'static str, crate::migration::Migrator)>, ModuleError> {
    dormant
        .iter()
        .map(|module| {
            let migrations =
                crate::migration::ModuleMigrations::new(module.name(), module.migrations())
                    .with_upgrades(module.upgrades());
            let migrator = crate::migration::Migrator::new(vec![migrations])
                .map_err(|error| ModuleError::new(error.to_string()))?
                .with_keys(keys.clone())
                .keyed_on(tenancy.control());
            Ok((module.name(), migrator))
        })
        .collect()
}

/// The module's tables exist in the database of `tenant` (it has the module's migration history).
pub(crate) async fn installed(module: &str, tenant: &TenantContext) -> Result<bool, Error> {
    let history = format!("{module}_migrations");
    Ok(crate::migration::module_installed(tenant.database().read(), &history).await?)
}

/// Calls the dormant handler only when the tenant's database has the module's tables; otherwise there is
/// nothing to delete and the event counts as delivered.
struct WhenInstalled {
    module: &'static str,
    inner: Arc<dyn EventHandler>,
}

#[async_trait::async_trait]
impl EventHandler for WhenInstalled {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        if installed(self.module, &event.tenant).await? {
            self.inner.handle(event).await
        } else {
            Ok(())
        }
    }
}
