//! Gợi ý alt text bằng AI (D-109). Không ghi gì; người xem rồi lưu bằng `PATCH /{id}`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Data, Problem};
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::images::alt::AltSuggestion;
use crate::service::MediaService;

/// What the alt text is for.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AltRequest {
    /// Language of the alt text, a BCP 47 code.
    #[schema(example = "vi")]
    pub locale: String,
    /// Where the image is used (for example the title of the post), up to 500 characters; helps
    /// the model say what matters.
    pub context: Option<String>,
}

/// A suggested alt text; nothing was saved.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct AltSuggestionResponse {
    /// Up to 250 characters; empty when `decorative`.
    #[schema(example = "Xuan Huong lake in Da Lat at sunrise, with pine trees on the far shore.")]
    alt: String,
    /// The model thinks the image is purely decorative: leave its alt text empty.
    decorative: bool,
    /// The model that answered.
    model: String,
}

impl From<AltSuggestion> for AltSuggestionResponse {
    fn from(suggestion: AltSuggestion) -> Self {
        AltSuggestionResponse {
            alt: suggestion.alt,
            decorative: suggestion.decorative,
            model: suggestion.model,
        }
    }
}

/// Suggest alt text with AI.
///
/// Requires `media.write`. The AI model looks at a small copy of the image (at most 1280
/// pixels, never the original file) and suggests a text alternative in `locale`. Nothing is
/// saved: show it, then store what a person accepts with `PATCH /api/v1/media/{id}` (or in the
/// `alt` of the image node of a post body, which is per language). Private images are sent
/// only when the server allows it (`AUTOXIDE_MEDIA_AI_SEND_PRIVATE`). Uses AI tokens; see
/// `GET /api/v1/ai/status`.
#[utoipa::path(
    post,
    path = "/{id}/alt-suggestions",
    extensions(("x-permissions" = json!(["media.write"]))),
    operation_id = "suggest_media_alt",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the media item.")),
    request_body = AltRequest,
    responses(
        (status = 200, description = "The suggestion; nothing was saved.", body = Data<AltSuggestionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `media.write` (code `permission_denied`), or the image is private and may not be sent (code `media.ai_private_not_allowed`).", body = Problem),
        (status = 404, description = "No media item has this id; code `media.item_not_found`.", body = Problem),
        (status = 409, description = "The item is in the trash; code `media.item_in_trash`. Restore it with `POST /api/v1/media/{id}/restore`.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), the item is not a readable JPEG, PNG, GIF or WebP image (code `media.ai_unsupported_image`), the model reads no images (code `ai.unsupported_input`), or it declined the image (code `ai.refused`).", body = Problem),
        (status = 429, description = "Today's AI token limit is used up; code `ai.quota_exceeded`, with `Retry-After`.", body = Problem),
        (status = 503, description = "AI is off (code `ai.disabled`), misconfigured (code `ai.misconfigured`), the provider is busy (code `ai.provider_unavailable`, maybe with `Retry-After`), or its answer had the wrong shape twice (code `ai.output_invalid`).", body = Problem),
    )
)]
pub(crate) async fn suggest_alt(
    State(service): State<Arc<MediaService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<AltRequest>,
) -> Result<Data<AltSuggestionResponse>, Error> {
    let suggestion =
        service.suggest_alt(&principal, id, &input.locale, input.context.as_deref()).await?;
    Ok(Data { data: suggestion.into() })
}
