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
    pub(crate) fn wrap(module: Box<dyn Module>, registered: Registered) -> Box<dyn Module> {
        if module.manifest().declared {
            Box::new(Started { inner: module, registered })
        } else {
            module
        }
    }
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
