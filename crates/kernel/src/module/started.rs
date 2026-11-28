//! The start-time half of the module contract (D-20261126-module-manifest): what a module built with a
//! [`Manifest`](super::Manifest) registers on [`StartContext`] in `start` (routes, MCP tools, subscriptions, jobs…),
//! and [`StartContext::kernel`], the one way to get a handle of the kernel.

// Cài đặt: module kiểu mới đăng ký trong `start` vào `Registered` của `StartContext`; sau `start`, registry bọc
// module trong [`Started`], kiểu trả các thứ đã đăng ký qua đúng các hàm cũ của `Module` mà kernel đang đọc
// (`tools()`, `subscriptions()`…). Nhờ vậy kernel không phải sửa hàng trăm chỗ đọc trong lúc chuyển; khi module
// cuối đã chuyển, các hàm cũ bị gỡ và kernel đọc thẳng (D-20261127-transition-paths).

use std::sync::Arc;

use crate::error::Error;
use crate::event::Subscription;
use crate::job::JobRegistration;

use super::{ClientSection, Manifest, Module, StartContext};

/// What a module built with a manifest registered in `start`.
#[derive(Default)]
pub(crate) struct Registered {
    subscriptions: Vec<Subscription>,
    routes: Option<crate::http::Routes>,
    tools: Vec<crate::mcp::Tool>,
    jobs: Vec<JobRegistration>,
    tasks: Vec<crate::task::Task>,
    checks: Vec<crate::doctor::Check>,
    prompts: Vec<crate::mcp::Prompt>,
    mcp_resources: Vec<crate::mcp::McpResource>,
    client_config: Option<ClientSection>,
    seed_tenant: Option<Arc<dyn crate::tenancy::TenantSeed>>,
    sample_data: Option<Arc<dyn crate::tenancy::TenantSeed>>,
    sandbox_copy: Option<Arc<dyn crate::tenancy::sandbox::SandboxCopy>>,
}

impl StartContext<'_> {
    // Một khái niệm một đường: module còn khai bằng hàm cũ của `Module` thì không được đăng ký kiểu mới (kernel
    // sẽ không đọc chỗ đó), báo lỗi rõ thay vì lặng lẽ bỏ qua.
    fn registered(&mut self, what: &str) -> Result<&mut Registered, Error> {
        if !self.declared {
            return Err(Error::internal(format!(
                "module `{}` registers {what} in `start` but does not implement `Module::manifest`: a module \
                 declares itself either with a manifest (and registers in `start`) or with the old `Module` \
                 functions, not both",
                self.module
            )));
        }
        Ok(&mut self.registered)
    }

    /// Subscribes the module to an event (of any module, including itself); the handler must accept the same
    /// event twice (D-054).
    pub fn subscribe(&mut self, subscription: Subscription) -> Result<(), Error> {
        self.registered("a subscription (`StartContext::subscribe`)")?
            .subscriptions
            .push(subscription);
        Ok(())
    }

    /// The module's REST endpoints (functions with `#[endpoint]`, listed in [`routes!`](crate::routes)), mounted
    /// under `/api/v1/<module name>`: `/` and `/{id}` of module `notes` become `/api/v1/notes` and
    /// `/api/v1/notes/{id}`. Called once; a second call adds to the first.
    pub fn routes(&mut self, routes: crate::http::Routes) -> Result<(), Error> {
        let registered = self.registered("routes (`StartContext::routes`)")?;
        registered.routes = Some(match registered.routes.take() {
            Some(earlier) => earlier.merge(routes),
            None => routes,
        });
        Ok(())
    }

    /// The module's MCP tools (D-068). Names are declared short (`create_note`); the kernel adds `<module>.`.
    pub fn tools(
        &mut self,
        tools: impl IntoIterator<Item = crate::mcp::Tool>,
    ) -> Result<(), Error> {
        self.registered("MCP tools (`StartContext::tools`)")?.tools.extend(tools);
        Ok(())
    }

    /// A runner for one of the module's job kinds (`<module>.<action>`). With jobs the module adds
    /// `job::jobs_migration` to its migrations.
    pub fn job(&mut self, job: JobRegistration) -> Result<(), Error> {
        self.registered("a job (`StartContext::job`)")?.jobs.push(job);
        Ok(())
    }

    /// An admin command for `autoxide task` (D-169), declared short; the kernel adds `<module>.`.
    pub fn task(&mut self, task: crate::task::Task) -> Result<(), Error> {
        self.registered("a task (`StartContext::task`)")?.tasks.push(task);
        Ok(())
    }

    /// An installation check for `autoxide doctor` (D-166), declared short; the kernel adds `<module>.`. A check
    /// is read-only, or cleans up whatever it wrote as a probe.
    pub fn check(&mut self, check: crate::doctor::Check) -> Result<(), Error> {
        self.registered("a doctor check (`StartContext::check`)")?.checks.push(check);
        Ok(())
    }

    /// An MCP prompt template (step 40 P2), declared short; the kernel adds `<module>.`.
    pub fn prompt(&mut self, prompt: crate::mcp::Prompt) -> Result<(), Error> {
        self.registered("an MCP prompt (`StartContext::prompt`)")?.prompts.push(prompt);
        Ok(())
    }

    /// A read-only MCP resource (step 40 P2); the kernel serves it at `autoxide://<module>/<path>`.
    pub fn mcp_resource(&mut self, resource: crate::mcp::McpResource) -> Result<(), Error> {
        self.registered("an MCP resource (`StartContext::mcp_resource`)")?
            .mcp_resources
            .push(resource);
        Ok(())
    }

    /// Non-secret configuration the UI needs, served without sign-in at `GET /api/v1/kernel/client-config` under
    /// the module name (D-271): only values anyone may see.
    pub fn client_config(&mut self, section: ClientSection) -> Result<(), Error> {
        let registered = self.registered("client configuration (`StartContext::client_config`)")?;
        if registered.client_config.replace(section).is_some() {
            return Err(Error::internal("client configuration registered twice".to_owned()));
        }
        Ok(())
    }

    /// Seeds the module's initial data for a new tenant (the `admin` role, lock rows...); must be re-runnable.
    pub fn seed_tenant(&mut self, seed: Arc<dyn crate::tenancy::TenantSeed>) -> Result<(), Error> {
        let registered = self.registered("a tenant seed (`StartContext::seed_tenant`)")?;
        if registered.seed_tenant.replace(seed).is_some() {
            return Err(Error::internal("tenant seed registered twice".to_owned()));
        }
        Ok(())
    }

    /// Sample data for tenants provisioned under a plan with `sample_data` (step 35); must be re-runnable and
    /// must not create duplicates.
    pub fn sample_data(&mut self, seed: Arc<dyn crate::tenancy::TenantSeed>) -> Result<(), Error> {
        let registered = self.registered("sample data (`StartContext::sample_data`)")?;
        if registered.sample_data.replace(seed).is_some() {
            return Err(Error::internal("sample data registered twice".to_owned()));
        }
        Ok(())
    }

    /// Copies the module's configuration into a new sandbox (step 38, D-232): configuration only, never personal
    /// data, transactions or secrets.
    pub fn sandbox_copy(
        &mut self,
        copy: Arc<dyn crate::tenancy::sandbox::SandboxCopy>,
    ) -> Result<(), Error> {
        let registered = self.registered("a sandbox copy (`StartContext::sandbox_copy`)")?;
        if registered.sandbox_copy.replace(copy).is_some() {
            return Err(Error::internal("sandbox copy registered twice".to_owned()));
        }
        Ok(())
    }

    /// A handle of the kernel for this module: `context.kernel::<Outbox>()?`, `context.kernel::<SyncLog>()?`,
    /// `context.kernel::<EntitlementsHandle>()?`… (the types that implement [`KernelHandle`]).
    pub fn kernel<H: KernelHandle>(&self) -> Result<H, Error> {
        H::from_context(self)
    }
}

mod sealed {
    pub trait Sealed {}
}

/// A handle the kernel hands to a module at start through [`StartContext::kernel`]. Sealed: only the kernel's
/// own handles implement it.
//
// `Sealed`: trait riêng tư làm "khoá" — crate ngoài không cài được `KernelHandle`, nên thêm hàm vào nó sau này
// không phá vỡ ai.
pub trait KernelHandle: Sized + sealed::Sealed {
    #[doc(hidden)]
    fn from_context(context: &StartContext<'_>) -> Result<Self, Error>;
}

// Mỗi handle một dòng: kiểu, và cách lấy từ `StartContext` (đúng getter cũ, để hai đường trả cùng một thứ).
macro_rules! kernel_handles {
    ($($handle:ty => |$context:ident| $get:expr;)+) => {$(
        impl sealed::Sealed for $handle {}
        impl KernelHandle for $handle {
            fn from_context($context: &StartContext<'_>) -> Result<Self, Error> {
                $get
            }
        }
    )+};
}

kernel_handles! {
    crate::event::Outbox => |context| context.outbox();
    crate::job::JobQueue => |context| Ok(context.jobs());
    crate::sync::SyncLog => |context| Ok(context.sync_log());
    crate::db::ControlPlane => |context| Ok(context.control_plane());
    crate::tenancy::DatabaseRegistry => |context| Ok(context.tenancy());
    crate::tenancy::OrgSettingCatalog => |context| Ok(context.org_settings());
    crate::entitlement::EntitlementsHandle => |context| Ok(context.entitlements());
    crate::privacy::PersonalDataExport => |context| Ok(context.personal_data_export());
    crate::privacy::RetentionCatalog => |context| Ok(context.retention_catalog());
    crate::resource::ResourceDirectory => |context| Ok(context.resources());
    crate::access::PostingVerification => |context| Ok(context.posting_verification());
    crate::ops::suggest::SuggestSources => |context| Ok(context.suggest_sources());
    crate::ops::summary::SummaryMetrics => |context| Ok(context.summary_metrics());
    crate::ops::reasons::ReasonCodes => |context| Ok(context.reason_codes());
    crate::ops::custom_fields::CustomFields => |context| Ok(context.custom_fields());
    crate::ops::texts::TextOverrides => |context| Ok(context.text_overrides());
    crate::ops::connections::ConnectionStatus => |context| Ok(context.connection_status());
    crate::blocks::Blocks => |context| Ok(context.blocks());
    crate::analytics::AnalyticsCatalog => |context| Ok(context.analytics_catalog());
    crate::automation::AutomationDirectory => |context| Ok(context.automation_directory());
    crate::tenancy::distribution::Distributor => |context| Ok(context.distributor());
}

/// A module built with a manifest, after `start`: answers the kernel's reads of the old start-time functions
/// with what the module registered on [`StartContext`]. Every other read goes to the module's manifest.
pub(crate) struct Started {
    inner: Box<dyn Module>,
    registered: Registered,
}

impl Started {
    /// Wraps `module` after its `start` when it is built with a manifest; a module still on the old functions
    /// is returned as it is (it could not register anything, see `StartContext::registered`).
    pub(crate) fn wrap(
        module: Box<dyn Module>,
        registered: Registered,
    ) -> Result<Box<dyn Module>, String> {
        if !module.manifest().declared {
            return Ok(module);
        }
        // Vỏ trả thứ đăng ký trong `start` thay cho hàm cũ: module còn cài hàm cũ "gọi sau start" thì phần đó sẽ mất
        // im lặng — báo lỗi khởi động thay vì bỏ qua.
        let old = runtime_overrides(module.as_ref());
        if !old.is_empty() {
            return Err(mixed_message(module.name(), &old));
        }
        Ok(Box::new(Started { inner: module, registered }))
    }
}

/// The message for a module that implements `Module::manifest` and old declaration functions too.
pub(crate) fn mixed_message(module: &str, old: &[&str]) -> String {
    let list = old.iter().map(|name| format!("`Module::{name}`")).collect::<Vec<_>>().join(", ");
    format!(
        "module `{module}` implements `Module::manifest` and also {list}: the core reads only one of them; declare \
         it in the manifest (or register it on `StartContext` in `start`) and remove the old function"
    )
}

/// Old declaration functions that a module with a manifest still implements, found by comparing each one with its
/// manifest (the default of an old function reads the manifest, so the two differ only when the function is
/// overridden). Lists are compared by length, which every element type supports.
pub(crate) fn static_overrides(module: &dyn Module) -> Vec<&'static str> {
    let manifest = module.manifest();
    if !manifest.declared {
        return Vec::new();
    }
    let unstable = &manifest.unstable;
    let mut old = Vec::new();
    let mut differs = |name: &'static str, different: bool| {
        if different {
            old.push(name);
        }
    };
    differs("name", module.name() != manifest.name);
    differs("description", module.description() != manifest.description);
    differs("stability", module.stability() != manifest.stability);
    differs("essential", module.essential() != manifest.essential);
    differs("always_enabled", module.always_enabled() != manifest.always_enabled);
    differs("depends_on", module.depends_on() != manifest.depends_on);
    differs(
        "migrations",
        module.migrations().len() != manifest.migrations.map_or(0, |build| build().len()),
    );
    differs(
        "upgrades",
        module.upgrades().len() != manifest.upgrades.map_or(0, |build| build().len()),
    );
    differs("records", module.records() != manifest.records);
    differs("tenant_settings", module.tenant_settings().len() != manifest.tenant_settings.len());
    differs("org_settings", module.org_settings().len() != manifest.org_settings.len());
    differs("retained_tables", module.retained_tables() != manifest.retained_tables);
    differs("retention", module.retention() != manifest.retention);
    differs("control_plane_tables", module.control_plane_tables() != manifest.control_plane_tables);
    differs("features", module.features() != manifest.features);
    differs("limits", module.limits() != manifest.limits);
    differs("permissions", module.permissions() != manifest.permissions);
    differs("events", module.events() != manifest.events);
    differs("resource_events", module.resource_events() != manifest.resource_events);
    differs("schedules", module.schedules() != manifest.schedules);
    differs("deprecations", module.deprecations() != manifest.deprecations);
    differs("body_limits", module.body_limits() != manifest.body_limits);
    differs("translations", module.translations() != manifest.translations);
    differs("navigation", module.navigation() != manifest.navigation);
    differs("reason_lists", module.reason_lists() != manifest.reason_lists);
    differs("overridable_texts", module.overridable_texts() != manifest.overridable_texts);
    differs("custom_field_targets", module.custom_field_targets() != unstable.custom_field_targets);
    differs("facts", module.facts().len() != unstable.facts.len());
    differs("analytics_events", module.analytics_events().len() != unstable.analytics_events.len());
    differs("funnels", module.funnels() != unstable.funnels);
    differs("automation_templates", module.automation_templates() != unstable.automation_templates);
    differs("promotable", module.promotable().len() != unstable.promotable.len());
    differs("sync_resources", module.sync_resources() != unstable.sync_resources);
    differs("blocks", module.blocks().len() != unstable.blocks.len());
    old
}

/// Old "after start" functions that a module with a manifest still implements: their defaults are empty, so
/// anything there comes from an override. Called after the module's `start`, when such functions have data.
fn runtime_overrides(module: &dyn Module) -> Vec<&'static str> {
    let has_paths =
        |router: utoipa_axum::router::OpenApiRouter| !router.get_openapi().paths.paths.is_empty();
    let mut old = Vec::new();
    let mut found = |name: &'static str, present: bool| {
        if present {
            old.push(name);
        }
    };
    found("subscriptions", !module.subscriptions().is_empty());
    found("routes", has_paths(module.routes()));
    found("endpoints", has_paths(module.endpoints().into_router()));
    found("tools", !module.tools().is_empty());
    found("jobs", !module.jobs().is_empty());
    found("tasks", !module.tasks().is_empty());
    found("checks", !module.checks().is_empty());
    found("prompts", !module.prompts().is_empty());
    found("mcp_resources", !module.mcp_resources().is_empty());
    found("client_config", !matches!(module.client_config(), Ok(None)));
    found("seed_tenant", module.seed_tenant().is_some());
    found("sample_data", module.sample_data().is_some());
    found("sandbox_copy", module.sandbox_copy().is_some());
    old
}

// Chỉ cài `manifest`, các hàm vòng đời và các hàm "gọi sau start". Hàm khai báo tĩnh (`permissions`, `events`…) để
// mặc định: mặc định của trait đọc `self.manifest()`, tức manifest của module bên trong.
impl Module for Started {
    fn manifest(&self) -> Manifest {
        self.inner.manifest()
    }

    fn configure(
        &mut self,
        reader: &mut crate::config::ConfigReader<'_>,
    ) -> Result<(), crate::config::Invalid> {
        self.inner.configure(reader)
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        self.inner.start(context)
    }

    fn dormant(&self, context: &mut super::DormantContext<'_>) -> Result<(), Error> {
        self.inner.dormant(context)
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        self.registered.subscriptions.clone()
    }

    fn endpoints(&self) -> crate::http::Routes {
        self.registered.routes.clone().unwrap_or_default()
    }

    fn tools(&self) -> Vec<crate::mcp::Tool> {
        self.registered.tools.clone()
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        self.registered.jobs.clone()
    }

    fn tasks(&self) -> Vec<crate::task::Task> {
        self.registered.tasks.clone()
    }

    fn checks(&self) -> Vec<crate::doctor::Check> {
        self.registered.checks.clone()
    }

    fn prompts(&self) -> Vec<crate::mcp::Prompt> {
        self.registered.prompts.clone()
    }

    fn mcp_resources(&self) -> Vec<crate::mcp::McpResource> {
        self.registered.mcp_resources.clone()
    }

    fn client_config(&self) -> Result<Option<ClientSection>, Error> {
        Ok(self.registered.client_config.clone())
    }

    fn seed_tenant(&self) -> Option<Arc<dyn crate::tenancy::TenantSeed>> {
        self.registered.seed_tenant.clone()
    }

    fn sample_data(&self) -> Option<Arc<dyn crate::tenancy::TenantSeed>> {
        self.registered.sample_data.clone()
    }

    fn sandbox_copy(&self) -> Option<Arc<dyn crate::tenancy::sandbox::SandboxCopy>> {
        self.registered.sandbox_copy.clone()
    }
}

/// Holds a module's place in the registry for the instant its wrapper is swapped in.
pub(crate) struct Placeholder;

impl Module for Placeholder {
    fn name(&self) -> &'static str {
        "placeholder"
    }
}
