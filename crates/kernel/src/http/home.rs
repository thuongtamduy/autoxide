//! `GET /`: a JSON home page - a person or agent opening the server address for the first time sees at once
//! what this is and where to go next (the OpenAPI document, the API viewer if enabled, MCP, health checks).
//! The core has only an API (architecture rule 9), so this is not an HTML page.

use std::sync::Arc;

use axum::extract::State;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::{DOCS_PATH, LLMS_PATH, MCP_PATH, OPENAPI_PATH};
use crate::response::Data;

/// What this server is and where to go next.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct Home {
    /// Always `Autoxide`.
    #[schema(example = "Autoxide")]
    pub name: String,
    /// Version of the server.
    #[schema(example = "0.16.0")]
    pub version: String,
    /// Path of the OpenAPI document describing every endpoint.
    #[schema(example = "/api/v1/openapi.json")]
    pub openapi: String,
    /// Path of the interactive API reference; `null` when it is turned off
    /// (`AUTOXIDE_HTTP_API_DOCS`).
    #[schema(example = "/api/docs")]
    pub docs: Option<String>,
    /// Path of the MCP endpoint for AI agents; `null` when no module authenticates callers.
    #[schema(example = "/mcp")]
    pub mcp: Option<String>,
    /// Path of the plain-text overview for AI agents (llmstxt.org).
    #[schema(example = "/llms.txt")]
    pub llms: String,
    /// Path of the readiness probe.
    #[schema(example = "/health/ready")]
    pub health: String,
}

/// The `GET /` route. `docs`: whether the API viewer is enabled; `mcp`: whether the MCP endpoint is mounted.
pub(super) fn routes(docs: bool, mcp: bool) -> OpenApiRouter {
    let info = Home {
        name: "Autoxide".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        openapi: OPENAPI_PATH.to_owned(),
        docs: docs.then(|| DOCS_PATH.to_owned()),
        mcp: mcp.then(|| MCP_PATH.to_owned()),
        llms: LLMS_PATH.to_owned(),
        health: "/health/ready".to_owned(),
    };
    // `Arc`: mỗi request chỉ tăng bộ đếm tham chiếu, không chép cả struct.
    OpenApiRouter::new().routes(routes!(home)).with_state(Arc::new(info))
}

/// Where to start: name, version and links to the API description.
///
/// Open the server's address to see what it is: the OpenAPI document, the interactive API
/// reference (when turned on), the MCP endpoint for AI agents, the overview for AI agents and the
/// readiness probe. Needs no sign-in.
#[utoipa::path(
    get,
    path = "/",
    extensions(("x-public" = json!(true))),
    tag = "health",
    operation_id = "home",
    responses((status = 200, description = "Name, version and links of this server.", body = Data<Home>))
)]
async fn home(State(home): State<Arc<Home>>) -> Data<Home> {
    Data { data: Home::clone(&home) }
}
