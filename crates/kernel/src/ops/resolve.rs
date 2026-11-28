//! Open a record directly by code (step 51 P9): a module registers a **code resolver** (order number,
//! tracking number, invoice number, article path, ...) with
//! [`StartContext::code_resolver`](crate::module::StartContext). `GET /api/v1/kernel/resolve?code=` asks, in
//! turn, the resolvers the caller has permission for and returns the first record found. No permission and no
//! record give the same 404, so existence is not revealed.

use std::sync::{Arc, PoisonError, RwLock};

use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::access::{Permission, Principal};
use crate::entitlement::EntitlementsHandle;
use crate::error::{Error, FieldError};
use crate::find::Found;
use crate::response::{Data, Problem};
use crate::tenancy::TenantContext;

/// A code resolver.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait CodeResolver: Send + Sync {
    /// Kind of record, `<module>.<kind>`, e.g. `content.entry`.
    fn kind(&self) -> &'static str;
    fn permission(&self) -> Permission;
    /// The record with code `code` that `principal` may see; `None` when there is none (or it may not be seen).
    async fn resolve(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        code: &str,
    ) -> Result<Option<Found>, Error>;
}

type Entry = (&'static str, Arc<dyn CodeResolver>);

/// Registry of every module's code resolvers. Cheap to clone.
#[derive(Clone, Default)]
pub struct CodeResolvers {
    entries: Arc<RwLock<Vec<Entry>>>,
}

impl CodeResolvers {
    pub(crate) fn register(
        &self,
        module: &'static str,
        resolver: Arc<dyn CodeResolver>,
    ) -> Result<(), Error> {
        let kind = resolver.kind();
        if !kind.starts_with(&format!("{module}.")) {
            return Err(Error::internal(format!(
                "code resolver `{kind}` of module `{module}` must be named `{module}.<type>`"
            )));
        }
        let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
        if entries.iter().any(|(_, known)| known.kind() == kind) {
            return Err(Error::internal(format!("code resolver `{kind}` is registered twice")));
        }
        entries.push((module, resolver));
        Ok(())
    }

    fn all(&self) -> Vec<Entry> {
        self.entries.read().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

/// The record a code opens.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct ResolvedView {
    #[schema(example = "content.entry")]
    pub kind: String,
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    /// Screen id of the navigation that opens the record.
    pub screen: String,
}

/// Which code.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ResolveQuery {
    /// An order number, tracking number, invoice number, page address…, as typed.
    code: Option<String>,
}

#[derive(Clone)]
pub(crate) struct ResolveState {
    pub resolvers: CodeResolvers,
    pub entitlements: EntitlementsHandle,
    /// Time limit of one source (`AUTOXIDE_SOURCE_TIMEOUT_MS`).
    pub timeout: std::time::Duration,
}

/// Open a record by its code.
///
/// Any signed-in user. Asks the modules' code readers (order numbers, tracking numbers, invoice
/// numbers, page addresses…), among those the caller has permission for, and answers the first
/// record found. No record and no permission answer the same 404, so a code never reveals that a
/// record exists.
#[utoipa::path(
    get,
    path = "/resolve",
    extensions(("x-permissions" = json!([]))),
    operation_id = "resolve_code",
    security(("bearer" = [])),
    params(ResolveQuery),
    responses(
        (status = 200, description = "The record.", body = Data<ResolvedView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "No record you may see has this code; code `kernel.code_not_found`.", body = Problem),
        (status = 422, description = "`code` is missing or longer than 200 characters; see `errors`.", body = Problem),
    )
)]
async fn resolve_code(
    State(state): State<Arc<ResolveState>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<ResolveQuery>,
) -> Result<Data<ResolvedView>, Error> {
    let code = query.code.as_deref().map(str::trim).unwrap_or_default();
    if code.is_empty() || code.chars().count() > 200 {
        return Err(Error::validation(vec![FieldError::new(
            "code",
            "invalid",
            "must be 1 to 200 characters long",
        )]));
    }
    for (module, resolver) in state.resolvers.all() {
        if !state.entitlements.module_enabled(tenant.id(), module).await?
            || principal.require(&resolver.permission()).is_err()
        {
            continue;
        }
        let answer =
            tokio::time::timeout(state.timeout, resolver.resolve(&tenant, &principal, code)).await;
        match answer {
            Ok(Ok(Some(found))) => {
                return Ok(Data {
                    data: ResolvedView {
                        kind: resolver.kind().to_owned(),
                        id: found.id,
                        title: found.title,
                        subtitle: found.subtitle,
                        screen: found.screen.to_owned(),
                    },
                });
            }
            Ok(Ok(None)) => {}
            Ok(Err(error)) => {
                tracing::warn!(resolver = resolver.kind(), error = %error, "code resolver failed");
            }
            Err(_) => tracing::warn!(resolver = resolver.kind(), "code resolver timed out"),
        }
    }
    Err(Error::not_found("kernel.code_not_found", "Nothing you may see has the code `{code}`.")
        .with_param("code", code.to_owned()))
}

pub(crate) fn routes(state: ResolveState) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(resolve_code)).with_state(Arc::new(state))
}
