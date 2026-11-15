//! Endpoint REST và tool MCP của module `ai`, dưới `/api/v1/ai`: chỉ trạng thái — giao diện dùng
//! để ẩn hay hiện nút AI. Không bao giờ trả key hay URL có thể chứa key.

use std::sync::Arc;

use autoxide_kernel::access::{Permission, Principal, Risk};
use autoxide_kernel::ai::{Embedder, TextModel};
use autoxide_kernel::error::Error;
use autoxide_kernel::mcp::{NoInput, Tool};
use autoxide_kernel::response::{Data, Problem};
use axum::extract::State;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::service::{Embeddings, Text};
use crate::usage::UsageBook;

pub(crate) const AI_VIEW: Permission = Permission::new(
    "ai.view",
    "See whether AI features are on, which provider and model they use, and today's token use.",
)
.with_risk(Risk::Read);

/// Mọi thứ trạng thái cần.
pub(crate) struct Status {
    pub text: Arc<Text>,
    pub embeddings: Arc<Embeddings>,
    pub book: Arc<UsageBook>,
}

/// What the AI features of this server can do now.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct AiStatus {
    /// Text generation is on (suggestions, translation).
    enabled: bool,
    /// `anthropic` or `openai` (the Chat Completions format); empty when off.
    #[schema(example = "anthropic")]
    provider: String,
    /// Model used for text; empty when off.
    #[schema(example = "claude-opus-5-5")]
    model: String,
    /// The model reads images (alt text suggestions work).
    images: bool,
    /// Embeddings are on (search by meaning).
    embeddings_enabled: bool,
    /// Embedding model; empty when off.
    embedding_model: String,
    /// Length of each embedding vector; 0 when off.
    embedding_dimensions: usize,
    /// Tokens used today (UTC), text and embeddings together.
    tokens_today: u64,
    /// Daily token limit; 0 means no limit.
    daily_token_limit: u64,
}

impl Status {
    async fn read(&self, principal: &Principal) -> Result<AiStatus, Error> {
        principal.require(&AI_VIEW)?;
        let capabilities = self.text.capabilities();
        Ok(AiStatus {
            enabled: capabilities.enabled,
            provider: capabilities.provider,
            model: capabilities.model,
            images: capabilities.images,
            embeddings_enabled: self.embeddings.enabled(),
            embedding_model: self.embeddings.model_id().to_owned(),
            embedding_dimensions: self.embeddings.dimensions(),
            tokens_today: self.book.used_today().await?,
            daily_token_limit: self.book.limit(),
        })
    }
}

pub(crate) fn routes(status: Arc<Status>) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(get_status)).with_state(status)
}

/// See what the AI features can do.
///
/// Requires `ai.view`. Tells whether text generation and embeddings are on, the provider and
/// models, whether images are read, and today's token use against the daily limit. Use it to
/// show or hide AI actions in a user interface. The operator turns AI on with the
/// `AUTOXIDE_AI_*` and `AUTOXIDE_EMBEDDING_*` settings.
#[utoipa::path(
    get,
    path = "/status",
    extensions(("x-permissions" = json!(["ai.view"]))),
    operation_id = "get_ai_status",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The AI status.", body = Data<AiStatus>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `ai.view`; code `permission_denied`.", body = Problem),
    )
)]
async fn get_status(
    State(status): State<Arc<Status>>,
    principal: Principal,
) -> Result<Data<AiStatus>, Error> {
    Ok(Data { data: status.read(&principal).await? })
}

pub(crate) fn tools(status: &Arc<Status>) -> Vec<Tool> {
    let status = status.clone();
    vec![
        Tool::new(
            "status",
            "Tell whether the server's AI features are on (text generation, image reading, \
             embeddings), which provider and models they use, and today's token use against the \
             daily limit.",
            move |principal: Principal, _: NoInput| {
                let status = status.clone();
                async move { status.read(&principal).await }
            },
        )
        .requires(AI_VIEW)
        .read_only(),
    ]
}
