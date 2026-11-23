//! Configuration for the UI (UI brief C, D-271): `GET /api/v1/kernel/client-config` gathers the
//! section each module declares through [`Module::client_config`](crate::module::Module::client_config) — only
//! non-secret values the UI needs so it does not have to guess the core's defaults (language, upload
//! limits, public address…).
//!
//! Values are read once at startup (configuration does not change while the program runs). A module with
//! per-tenant settings (e.g. the language of `content`, cross-checked with design 2 A-b7) attaches an extra
//! [`TenantClientSection`]: when the request identifies the tenant, the fields the tenant has set replace the
//! shared values. The OpenAPI schema of the response is assembled from each section's schema (the module's
//! `T: ToSchema`), so the documentation describes every field fully without the kernel knowing which modules
//! exist.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use serde::Serialize;
use serde_json::Value;
use utoipa::ToSchema;
use utoipa::openapi::RefOr;
use utoipa::openapi::schema::{ObjectBuilder, Schema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::Error;
use crate::module::Module;
use crate::response::{Data, Problem};

/// Per-tenant values of a UI configuration section.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait TenantClientSection: Send + Sync {
    /// The fields (top level of the section) that tenant `tenant` sets differently from the shared
    /// configuration, with the same names and types as in the section; absent fields keep the shared value.
    async fn overrides(
        &self,
        tenant: &crate::tenancy::TenantContext,
    ) -> Result<serde_json::Map<String, Value>, Error>;
}

/// A module's UI configuration section: its value and OpenAPI schema.
#[derive(Clone)]
pub struct ClientSection {
    value: Value,
    schema: RefOr<Schema>,
    tenant: Option<Arc<dyn TenantClientSection>>,
}

// `RefOr<Schema>` của utoipa không cài `Debug`: chỉ in giá trị.
impl std::fmt::Debug for ClientSection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("ClientSection").field("value", &self.value).finish_non_exhaustive()
    }
}

impl ClientSection {
    /// A section from `value`. `T` should only have simple fields (strings, numbers, flags, lists of them):
    /// its schema is copied verbatim into the OpenAPI documentation. Each field carries a `///` description (in
    /// English) and holds only values anyone may see — the endpoint needs no login.
    pub fn new<T: Serialize + ToSchema>(value: &T) -> Result<Self, Error> {
        let value = serde_json::to_value(value).map_err(|error| {
            Error::internal(format!("client config does not serialize: {error}"))
        })?;
        Ok(ClientSection { value, schema: T::schema(), tenant: None })
    }

    /// The same section, with the fields the tenant sets itself taken from `tenant` (the request identifies
    /// the tenant).
    #[must_use]
    pub fn per_tenant(mut self, tenant: Arc<dyn TenantClientSection>) -> Self {
        self.tenant = Some(tenant);
        self
    }

    /// The section's value for tenant `tenant` (`None`: the shared value).
    async fn value_for(
        &self,
        tenant: Option<&crate::tenancy::TenantContext>,
    ) -> Result<Value, Error> {
        let mut value = self.value.clone();
        if let (Some(overlay), Some(tenant), Some(object)) =
            (&self.tenant, tenant, value.as_object_mut())
        {
            object.extend(overlay.overrides(tenant).await?);
        }
        Ok(value)
    }
}

/// The configuration sections of every module, filled once after every module has started.
#[derive(Clone, Default)]
pub(crate) struct ClientConfigs(Arc<OnceLock<Vec<(&'static str, ClientSection)>>>);

impl ClientConfigs {
    /// Filled from the modules that have started. A module that returns an error has no section (the error is
    /// logged).
    pub(crate) fn fill(&self, modules: &[Box<dyn Module>]) {
        let sections = modules
            .iter()
            .filter_map(|module| match module.client_config() {
                Ok(section) => section.map(|section| (module.name(), section)),
                Err(error) => {
                    tracing::error!(module = module.name(), %error, "client config left out");
                    None
                }
            })
            .collect();
        let _ = self.0.set(sections);
    }

    fn sections(&self) -> &[(&'static str, ClientSection)] {
        self.0.get().map_or(&[], Vec::as_slice)
    }
}

/// Settings of the server an interface needs, by module (`kernel`, `content`, `media`…): only
/// values safe to show anyone. A module without such settings is absent.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(transparent)]
pub(crate) struct ClientConfigResponse(BTreeMap<&'static str, Value>);

pub(crate) fn routes(configs: ClientConfigs) -> OpenApiRouter {
    let mut router = OpenApiRouter::new()
        .routes(routes!(get_client_config))
        .with_state(Arc::new(configs.clone()));
    // Thay lược đồ chung (object bất kỳ) bằng lược đồ ghép từ phần của từng module.
    let mut object = ObjectBuilder::new().description(Some(
        "Settings of the server an interface needs, by module: only values safe to show anyone. \
         A module without such settings is absent.",
    ));
    for (name, section) in configs.sections() {
        object = object.property(*name, section.schema.clone()).required(*name);
    }
    if let Some(components) = router.get_openapi_mut().components.as_mut() {
        components.schemas.insert("ClientConfigResponse".to_owned(), object.build().into());
    }
    router
}

/// Get the settings an interface needs.
///
/// No sign-in needed. Values the server's operator set (or the defaults), by module: languages,
/// path prefixes, upload limits, the public address… — so an interface does not guess the
/// server's defaults. When the request names a tenant (its domain, the `Autoxide-Tenant` header
/// or a credential), values that tenant set for itself replace the server's (e.g. the languages
/// of `content` and of `media` alt texts, see `GET /api/v1/kernel/settings`).
#[utoipa::path(
    get,
    path = "/client-config",
    extensions(("x-public" = json!(true))),
    operation_id = "get_client_config",
    security((), ("bearer" = [])),
    responses(
        (status = 200, description = "The settings, by module.", body = Data<ClientConfigResponse>),
        (status = 403, description = "The tenant of the request is suspended; code `tenant_suspended`.", body = Problem),
        (status = 500, description = "Unexpected server error.", body = Problem),
        (status = 503, description = "The tenant of the request is under maintenance; code `tenant_maintenance`.", body = Problem),
    )
)]
async fn get_client_config(
    axum::extract::State(configs): axum::extract::State<Arc<ClientConfigs>>,
    crate::tenancy::OptionalTenant(tenant): crate::tenancy::OptionalTenant,
) -> Result<Data<ClientConfigResponse>, Error> {
    let mut sections = BTreeMap::new();
    for (name, section) in configs.sections() {
        sections.insert(*name, section.value_for(tenant.as_ref()).await?);
    }
    Ok(Data { data: ClientConfigResponse(sections) })
}

/// Kernel's own section.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct KernelClientConfig {
    /// The public address of the API (`AUTOXIDE_HTTP_PUBLIC_URL`), used in links of mails and by
    /// MCP clients (`<public_url>/mcp`); `null` when the operator did not set it.
    #[schema(example = "https://api.example.com")]
    pub(crate) public_url: Option<String>,
    /// Version of the server.
    #[schema(example = "0.15.0")]
    pub(crate) version: String,
}
