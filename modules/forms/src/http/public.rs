//! Endpoint công khai (không cần đăng nhập): định nghĩa form để website hiển thị, và nhận bài
//! gửi. Không bao giờ trả `notify_emails`, giới hạn, thời hạn giữ hay danh sách origin.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::http::{ClientInfo, ValidJson};
use autoxide_kernel::response::{Created, Problem};
use autoxide_kernel::version::Tagged;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use crate::fields::{Field, FieldType, Locale};
use crate::forms::{Form, FormStatus};
use crate::public::PublicSubmission;
use crate::service::FormsService;

/// A choice, in the asked language.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PublicOption {
    /// What to send.
    value: String,
    /// What to show.
    label: String,
}

/// A field, in the asked language.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PublicField {
    /// Send the answer under this key in `data`.
    key: String,
    #[serde(rename = "type")]
    kind: FieldType,
    /// For `consent`: the exact text to show next to the box (it is stored with the
    /// submission as proof).
    label: String,
    help: Option<String>,
    placeholder: Option<String>,
    required: bool,
    /// Choices of `select`, `radio` and `checkbox`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    options: Vec<PublicOption>,
    min_length: Option<u32>,
    max_length: Option<u32>,
    min: Option<f64>,
    max: Option<f64>,
}

impl PublicField {
    fn new(field: &Field, locale: Locale) -> Self {
        PublicField {
            key: field.key.clone(),
            kind: field.kind,
            label: field.label.get(locale).to_owned(),
            help: field.help.as_ref().map(|text| text.get(locale).to_owned()),
            placeholder: field.placeholder.as_ref().map(|text| text.get(locale).to_owned()),
            required: field.required,
            options: field
                .options
                .iter()
                .map(|option| {
                    let label = option.label.get(locale);
                    PublicOption {
                        value: option.value.clone(),
                        label: if label.is_empty() {
                            option.value.clone()
                        } else {
                            label.to_owned()
                        },
                    }
                })
                .collect(),
            min_length: field.min_length,
            max_length: field.max_length,
            min: field.min,
            max: field.max,
        }
    }
}

/// A form, as a website shows it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PublicFormResponse {
    #[schema(example = "contact")]
    key: String,
    name: String,
    /// `closed`: show that the form is closed; sending is refused with `forms.closed`.
    status: FormStatus,
    /// Version of the fields.
    version: i32,
    /// Language of the texts below.
    locale: Locale,
    fields: Vec<PublicField>,
    /// Ask `GET /api/v1/antispam/token?scope=<token_scope>` for a form token each time the
    /// form is shown, and send it as `form_token`.
    #[schema(example = "forms:contact")]
    token_scope: String,
    /// Shown after sending (also returned by the submit endpoint).
    success_message: String,
}

impl PublicFormResponse {
    fn new(form: &Form, locale: Locale) -> Self {
        PublicFormResponse {
            key: form.row.key.clone(),
            name: form.row.name.clone(),
            status: form.status(),
            version: form.row.version,
            locale,
            fields: form.fields.iter().map(|field| PublicField::new(field, locale)).collect(),
            token_scope: form.scope(),
            success_message: form.success_message.get(locale).to_owned(),
        }
    }
}

/// Language of the texts.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct LocaleQuery {
    /// `en` (default) or `vi`. A text missing in that language falls back to the other.
    #[param(value_type = Option<Locale>, inline)]
    locale: Option<String>,
}

/// Get a form to show (public).
///
/// No sign-in needed. The fields with their labels, help and choices in one language, the
/// token scope and the success message. Admin settings (notification addresses, limits,
/// retention, allowed origins) are never included.
#[utoipa::path(
    get,
    path = "/public/{key}",
    operation_id = "get_public_form",
    security((), ("bearer" = [])),
    params(("key" = String, Path, description = "Key of the form."), LocaleQuery),
    responses(
        (status = 200, description = "The form.", body = autoxide_kernel::response::Data<PublicFormResponse>, headers(("ETag" = String, description = "Version; send it in `If-None-Match` to get 304."))),
        (status = 404, description = "No form has this key; code `forms.not_found`.", body = Problem),
        (status = 422, description = "`locale` is not `en` or `vi`; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn get_public_form(
    State(service): State<Arc<FormsService>>,
    _principal: Option<Principal>,
    Path(key): Path<String>,
    Query(query): Query<LocaleQuery>,
) -> Result<Tagged<PublicFormResponse>, Error> {
    let locale = match query.locale.as_deref().map(str::trim).filter(|raw| !raw.is_empty()) {
        None => Locale::default(),
        Some(raw) => Locale::parse(raw).ok_or_else(|| {
            Error::validation(vec![FieldError::new("locale", "invalid", "must be `en` or `vi`")])
        })?,
    };
    let form = service.public_form(&key).await?;
    Ok(Tagged { version: form.row.updated_at, data: PublicFormResponse::new(&form, locale) })
}

/// A submission of a public form.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SubmitRequest {
    /// Answers by field key: a string for most types, a number for `number`, an array of
    /// option values for `checkbox`, `true` for a ticked `consent`. Omit (or send `""` /
    /// `null`) unanswered fields. Unknown keys are refused.
    #[schema(value_type = Object, example = json!({"name": "Ada", "email": "ada@example.com", "message": "Hello"}))]
    data: Map<String, Value>,
    /// Language the form was shown in: `en` or `vi`.
    locale: Option<String>,
    /// Address of the page the form is on; omitted: the `Referer` header.
    page_url: Option<String>,
    /// Page that led the visitor there (`document.referrer`), if any.
    referrer: Option<String>,
    /// Token from `GET /api/v1/antispam/token?scope=forms:<key>`; needed unless the caller
    /// has the permission `forms.submit` (a website server with its own API key). Being
    /// signed in without it is not enough.
    form_token: Option<String>,
    /// The hidden trap field: people leave it empty.
    trap: Option<String>,
    /// The value of the `altcha` widget, when the server uses a challenge.
    challenge: Option<String>,
}

/// The answer to a submission.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct SubmitResponse {
    /// Id of the submission. Sending the same form token again returns the same id.
    id: Uuid,
    /// The form's success message, in the language sent.
    message: String,
}

/// Send a form (public).
///
/// No sign-in needed. Every answer is checked on the server and every problem is reported at
/// once in `errors` (paths `data.<key>`). Send `form_token` (and `challenge` when the server
/// uses one); only a caller with the permission `forms.submit` (a website server with its own
/// API key) may omit them — other signed-in callers go through the same checks as anonymous
/// visitors. The answer is the
/// same 201 whether the submission is accepted, held for review or dropped by the anti-spam
/// checks. Sending the same `form_token` again returns the first answer and stores nothing
/// new. The body may be at most `AUTOXIDE_FORMS_MAX_SUBMISSION_BYTES`.
#[utoipa::path(
    post,
    path = "/public/{key}/submissions",
    operation_id = "submit_form",
    security((), ("bearer" = [])),
    params(("key" = String, Path, description = "Key of the form.")),
    request_body = SubmitRequest,
    responses(
        (status = 201, description = "Received.", body = autoxide_kernel::response::Data<SubmitResponse>),
        (status = 400, description = "The form token or challenge is missing, invalid or expired (codes `antispam.token_missing`, `antispam.token_invalid`, `antispam.token_expired`, `antispam.challenge_required`, `antispam.challenge_failed`): reload the form.", body = Problem),
        (status = 403, description = "The page's `Origin` is not allowed for this form; code `forms.origin_not_allowed`.", body = Problem),
        (status = 404, description = "No form has this key; code `forms.not_found`.", body = Problem),
        (status = 409, description = "The form is closed; code `forms.closed`.", body = Problem),
        (status = 413, description = "The body is too large; code `payload_too_large`.", body = Problem),
        (status = 422, description = "An answer is invalid; see `errors`.", body = Problem),
        (status = 429, description = "A caller with `forms.submit` sent too many; code `antispam.rate_limited`.", body = Problem),
        (status = 503, description = "The anti-spam check cannot run; code `antispam.verifier_unavailable`.", body = Problem),
    )
)]
pub(crate) async fn submit_form(
    State(service): State<Arc<FormsService>>,
    principal: Option<Principal>,
    client: ClientInfo,
    Path(key): Path<String>,
    ValidJson(input): ValidJson<SubmitRequest>,
) -> Result<Created<SubmitResponse>, Error> {
    let submission = PublicSubmission {
        data: input.data,
        locale: input.locale,
        page_url: input.page_url,
        referrer: input.referrer,
        form_token: input.form_token,
        trap: input.trap,
        challenge: input.challenge,
    };
    let receipt = service.submit(&key, principal.as_ref(), &client, submission).await?;
    Ok(Created::new(
        format!("/forms/submissions/{}", receipt.id),
        SubmitResponse { id: receipt.id, message: receipt.message },
    ))
}
