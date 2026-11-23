//! Internal module `antispam` (always registered, right after `kernel`): reads the anti-spam configuration,
//! publishes [`SpamGuard`] for other modules, and serves the public form token endpoint
//! `GET /api/v1/antispam/token`.

use crate::time::Timestamp;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::{AntispamConfig, SpamGuard};
use crate::access::Principal;
use crate::config::{ConfigReader, Invalid};
use crate::error::Error;
use crate::module::{Module, StartContext};
use crate::response::{Data, Problem};
use crate::secret::SecretKey;

/// Name of the internal module; other modules must not use this name.
pub(crate) const NAME: &str = "antispam";

#[derive(Default)]
pub(crate) struct AntispamModule {
    config: Option<AntispamConfig>,
    guard: Option<Arc<SpamGuard>>,
}

impl Module for AntispamModule {
    fn name(&self) -> &'static str {
        NAME
    }

    fn description(&self) -> &'static str {
        "Spam protection shared by public forms: form tokens, proof-of-work challenges and \
         rate limits."
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(AntispamConfig::read(reader)?);
        Ok(())
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let secret = context.require::<SecretKey>()?;
        let config = self.config.clone().unwrap_or_default();
        let guard = Arc::new(SpamGuard::new((*secret).clone(), config));
        self.guard = Some(guard.clone());
        context.provide::<SpamGuard>(guard)
    }

    fn routes(&self) -> OpenApiRouter {
        match &self.guard {
            Some(guard) => OpenApiRouter::new().routes(routes!(token)).with_state(guard.clone()),
            None => OpenApiRouter::new(),
        }
    }
}

/// Which form the token is for.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct TokenQuery {
    /// The form, as the module that receives it names it, e.g. `forms:contact`: 1 to 64
    /// lower-case letters, digits, `_`, `-`, `.` or `:`.
    #[param(example = "forms:contact")]
    scope: String,
}

/// A form token.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct FormTokenResponse {
    /// Send it with the form (the receiving endpoint says in which field). Valid once.
    token: String,
    expires_at: Timestamp,
    /// Submissions sent before this moment are held for review: people take longer than bots.
    submit_after: Timestamp,
    /// Parameters of the challenge the browser must solve, when the server uses one; `null`
    /// otherwise.
    #[schema(value_type = Option<Object>)]
    challenge: Option<serde_json::Value>,
}

/// Get a form token (public).
///
/// No sign-in needed. Fetch one each time a public form is shown (never put it in cached HTML)
/// and send it with the form: the server refuses public submissions without a valid token.
/// Answers with `Cache-Control: no-store`.
#[utoipa::path(
    get,
    path = "/token",
    extensions(("x-public" = json!(true))),
    operation_id = "get_antispam_token",
    security((), ("bearer" = [])),
    params(TokenQuery),
    responses(
        (status = 200, description = "A fresh token.", body = Data<FormTokenResponse>),
        (status = 422, description = "`scope` is malformed; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn token(
    State(guard): State<Arc<SpamGuard>>,
    tenant: crate::tenancy::TenantContext,
    _principal: Option<Principal>,
    Query(query): Query<TokenQuery>,
) -> Result<Response, Error> {
    let issued = guard.issue(&tenant, &query.scope).await?;
    let data = FormTokenResponse {
        token: issued.token,
        expires_at: issued.expires_at,
        submit_after: issued.submit_after,
        challenge: issued.challenge,
    };
    let mut response = Data { data }.into_response();
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}
