//! Đổi câu chữ hiển thị theo khách (bước 51 P21, D-235): khách thay thuật ngữ, câu hướng tới người
//! dùng cuối ("Đơn hàng" → "Phiếu đặt") mà không sửa code.
//!
//! - Module đánh dấu câu **đổi được** bằng [`Module::overridable_texts`](crate::module::Module) —
//!   khoá `<module>.<…>`, câu tiếng Anh mặc định (dịch qua `text:` của module), `legal` (câu pháp
//!   lý: không đổi được). Mã lỗi và câu của lỗi không bao giờ đổi được: chúng là hợp đồng với máy.
//! - Không gian `ui.*` cho repo giao diện: core chỉ kiểm dạng khoá, độ dài và giữ — tối đa
//!   `AUTOXIDE_TEXT_OVERRIDES_UI_MAX` khoá mỗi khách.
//! - Bảng `kernel_text_overrides` (cấp khách hàng). Kiểm khi lưu: chữ thuần (không thẻ HTML), tối đa
//!   500 ký tự, tập chỗ điền `{tên}` trùng khớp câu mặc định.
//! - Đọc: [`TextOverrides::text`] tra câu của khách trong cache bộ nhớ theo khách (một map, xoá khi
//!   ghi, sống tối đa 30 giây — máy chủ khác thấy câu mới sau chừng đó), không query ở đường nóng.
//!
//! API: `GET /api/v1/kernel/texts?locale&prefix&overridden`, `PUT/DELETE
//! /api/v1/kernel/texts/{locale}/{key}` (`kernel.texts.manage`); `GET
//! /api/v1/kernel/public/texts?locale&prefix=ui.` (công khai, `ETag`) cho giao diện.

use crate::time::Timestamp;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, OnConflict, Query as SqlQuery};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::access::{Permission, Principal};
use crate::config::Setting;
use crate::error::{Error, ErrorKind, FieldError};
use crate::http::ValidJson;
use crate::i18n::{Language, Translations};
use crate::migration::prelude::*;
use crate::response::{Data, Problem};
use crate::tenancy::{TenantContext, TenantId};

const TABLE: &str = "kernel_text_overrides";
/// Câu của khách dài tối đa chừng này ký tự.
const TEXT_MAX: usize = 500;
/// Cache theo khách sống chừng này.
const CACHE_TTL: Duration = Duration::from_secs(30);

/// Đổi câu chữ hiển thị của khách.
pub const TEXTS_MANAGE: Permission = Permission::new(
    "kernel.texts.manage",
    "Replace the wording the tenant's users see: terms of modules and texts of the interface.",
);

pub const UI_MAX: Setting = Setting::new(
    "AUTOXIDE_TEXT_OVERRIDES_UI_MAX",
    "Most interface texts (`ui.*` keys) a tenant may replace (1 to 100000).",
)
.with_default("1000");

/// Một câu module cho khách đổi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct OverridableText {
    /// `<module>.<…>`, vd `orders.term.order`.
    pub key: &'static str,
    /// Câu tiếng Anh mặc định (dịch qua `text:` của module).
    pub default: &'static str,
    /// Câu pháp lý: không đổi được.
    pub legal: bool,
}

impl OverridableText {
    pub const fn new(key: &'static str, default: &'static str) -> Self {
        OverridableText { key, default, legal: false }
    }

    pub const fn legal(mut self) -> Self {
        self.legal = true;
        self
    }
}

fn is_key(key: &str) -> bool {
    (3..=200).contains(&key.len())
        && key.contains('.')
        && key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}

fn is_locale(locale: &str) -> bool {
    (2..=10).contains(&locale.len())
        && locale.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Chỗ điền `{tên}` của một câu.
fn placeholders(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else { break };
        let name = &after[..end];
        if !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            found.insert(name.to_owned());
        }
        rest = &after[end + 1..];
    }
    found
}

/// Kiểm khai báo của module: khoá `<module>.<…>` đúng dạng, không trùng.
pub(crate) fn check(module: &str, texts: &[OverridableText], problems: &mut Vec<String>) {
    let mut seen = BTreeSet::new();
    for text in texts {
        if !text.key.starts_with(&format!("{module}."))
            || !is_key(text.key)
            || !seen.insert(text.key)
        {
            problems.push(format!(
                "overridable text `{}` of module `{module}` must be a unique key `{module}.<…>`",
                text.key
            ));
        }
    }
}

type Cached = (Instant, Arc<HashMap<(String, String), String>>);

/// Câu khách đã đổi: sổ câu đổi được của mọi module và cache theo khách. Clone rẻ.
#[derive(Clone, Default)]
pub struct TextOverrides {
    declared: Arc<OnceLock<BTreeMap<&'static str, OverridableText>>>,
    cache: Arc<Mutex<HashMap<TenantId, Cached>>>,
    ui_max: Arc<OnceLock<u64>>,
}

impl TextOverrides {
    pub(crate) fn fill(&self, texts: Vec<OverridableText>) {
        let _ = self.declared.set(texts.into_iter().map(|text| (text.key, text)).collect());
    }

    pub(crate) fn set_ui_max(&self, max: u64) {
        let _ = self.ui_max.set(max);
    }

    fn declared(&self) -> &BTreeMap<&'static str, OverridableText> {
        static EMPTY: BTreeMap<&'static str, OverridableText> = BTreeMap::new();
        self.declared.get().unwrap_or(&EMPTY)
    }

    /// Câu khách `tenant` đặt cho khoá `key` bằng ngôn ngữ `locale`; `None`: dùng câu mặc định.
    pub async fn text(
        &self,
        tenant: &TenantContext,
        locale: &str,
        key: &str,
    ) -> Result<Option<String>, Error> {
        let all = self.all(tenant).await?;
        Ok(all.get(&(locale.to_owned(), key.to_owned())).cloned())
    }

    async fn all(
        &self,
        tenant: &TenantContext,
    ) -> Result<Arc<HashMap<(String, String), String>>, Error> {
        {
            let cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((at, map)) = cache.get(&tenant.id())
                && at.elapsed() < CACHE_TTL
            {
                return Ok(map.clone());
            }
        }
        let select = SqlQuery::select()
            .columns(["locale", "key", "text"])
            .from(Alias::new(TABLE))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
            .to_owned();
        let mut map = HashMap::new();
        for row in tenant.database().read().query_all(&select).await? {
            map.insert(
                (row.try_get("", "locale")?, row.try_get("", "key")?),
                row.try_get("", "text")?,
            );
        }
        let map = Arc::new(map);
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.retain(|_, (at, _)| at.elapsed() < CACHE_TTL);
        cache.insert(tenant.id(), (Instant::now(), map.clone()));
        Ok(map)
    }

    fn forget(&self, tenant: TenantId) {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner).remove(&tenant);
    }
}

/// One text a tenant may replace.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct TextView {
    #[schema(example = "content.term.post")]
    pub key: String,
    pub locale: String,
    /// The default wording in `locale` (`null` for `ui.*` keys: the interface owns it).
    pub default: Option<String>,
    /// The tenant's wording, when set.
    pub text: Option<String>,
    /// Required by law: cannot be replaced.
    pub legal: bool,
}

/// Which texts.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct TextsQuery {
    /// Language of the texts, e.g. `vi`. Defaults to `en`.
    locale: Option<String>,
    /// Only keys starting with this, e.g. `ui.` or `orders.`.
    prefix: Option<String>,
    /// `true`: only the texts the tenant replaced.
    #[serde(default)]
    overridden: bool,
}

/// The tenant's wording.
#[derive(Debug, Clone, Deserialize, ToSchema, validator::Validate)]
#[serde(deny_unknown_fields)]
pub(crate) struct TextInput {
    /// Plain text (no HTML), 1 to 500 characters, with exactly the `{placeholders}` of the default.
    pub text: String,
}

#[derive(Clone)]
pub(crate) struct TextsState {
    pub overrides: TextOverrides,
    pub translations: Translations,
}

impl TextsState {
    async fn views(
        &self,
        tenant: &TenantContext,
        locale: &str,
        prefix: Option<&str>,
        overridden: bool,
    ) -> Result<Vec<TextView>, Error> {
        let all = self.overrides.all(tenant).await?;
        let language = Language::from_tag(locale).unwrap_or(Language::En);
        let wanted = |key: &str| prefix.is_none_or(|prefix| key.starts_with(prefix));
        let mut views: Vec<TextView> = self
            .overrides
            .declared()
            .values()
            .filter(|declared| wanted(declared.key))
            .map(|declared| TextView {
                key: declared.key.to_owned(),
                locale: locale.to_owned(),
                default: Some(self.translations.describe(declared.default, language)),
                text: all.get(&(locale.to_owned(), declared.key.to_owned())).cloned(),
                legal: declared.legal,
            })
            .collect();
        for ((text_locale, key), text) in all.iter() {
            if text_locale == locale && key.starts_with("ui.") && wanted(key) {
                views.push(TextView {
                    key: key.clone(),
                    locale: locale.to_owned(),
                    default: None,
                    text: Some(text.clone()),
                    legal: false,
                });
            }
        }
        if overridden {
            views.retain(|view| view.text.is_some());
        }
        views.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(views)
    }
}

fn not_overridable(key: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "kernel.text_not_overridable",
        "`{key}` cannot be replaced: only texts modules mark as replaceable, and `ui.*` keys.",
    )
    .with_param("key", key.to_owned())
}

/// List the texts a tenant may replace.
///
/// Requires `kernel.texts.manage`. The texts modules let tenants replace (with their default in
/// `locale`) and the tenant's interface texts (`ui.*`), with the tenant's wording when set.
#[utoipa::path(
    get,
    path = "/texts",
    extensions(("x-permissions" = json!(["kernel.texts.manage"]))),
    operation_id = "list_texts",
    security(("bearer" = [])),
    params(TextsQuery),
    responses(
        (status = 200, description = "The texts.", body = Data<Vec<TextView>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `kernel.texts.manage`; code `permission_denied`.", body = Problem),
    )
)]
async fn list_texts(
    State(state): State<Arc<TextsState>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<TextsQuery>,
) -> Result<Data<Vec<TextView>>, Error> {
    principal.require(&TEXTS_MANAGE)?;
    let locale = query.locale.unwrap_or_else(|| "en".to_owned());
    Ok(Data {
        data: state.views(&tenant, &locale, query.prefix.as_deref(), query.overridden).await?,
    })
}

/// Replace a text.
///
/// Requires `kernel.texts.manage`. Sets the tenant's wording of `key` in `locale`: plain text of
/// 1 to 500 characters with exactly the `{placeholders}` of the default. Error codes and texts
/// required by law cannot be replaced.
#[utoipa::path(
    put,
    path = "/texts/{locale}/{key}",
    extensions(("x-permissions" = json!(["kernel.texts.manage"]))),
    operation_id = "set_text",
    security(("bearer" = [])),
    params(
        ("locale" = String, Path, description = "Language, e.g. `vi`."),
        ("key" = String, Path, description = "The text's key, e.g. `content.term.post` or `ui.orders.title`."),
    ),
    request_body = TextInput,
    responses(
        (status = 200, description = "The text.", body = Data<TextView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `kernel.texts.manage`; code `permission_denied`.", body = Problem),
        (status = 409, description = "Too many interface texts; code `kernel.text_limit_reached`.", body = Problem),
        (status = 422, description = "The key cannot be replaced (`kernel.text_not_overridable`), the placeholders differ from the default (`kernel.text_placeholders_mismatch`), or the text is invalid; see `errors`.", body = Problem),
    )
)]
async fn set_text(
    State(state): State<Arc<TextsState>>,
    tenant: TenantContext,
    principal: Principal,
    Path((locale, key)): Path<(String, String)>,
    ValidJson(input): ValidJson<TextInput>,
) -> Result<Data<TextView>, Error> {
    principal.require(&TEXTS_MANAGE)?;
    let declared = state.overrides.declared().get(key.as_str()).copied();
    let ui = key.starts_with("ui.") && is_key(&key);
    if declared.is_some_and(|declared| declared.legal) || (declared.is_none() && !ui) {
        return Err(not_overridable(&key));
    }
    let text = input.text.trim().to_owned();
    let mut problems = Vec::new();
    if !is_locale(&locale) {
        problems.push(FieldError::new("locale", "invalid", "must be a language code, e.g. `vi`"));
    }
    if text.is_empty() || text.chars().count() > TEXT_MAX || text.contains(['<', '>']) {
        problems.push(
            FieldError::new("text", "invalid", "must be plain text of 1 to {max} characters")
                .with_param("max", TEXT_MAX),
        );
    }
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    if let Some(declared) = declared {
        let (wanted, got) = (placeholders(declared.default), placeholders(&text));
        if wanted != got {
            let list = |set: &BTreeSet<String>| {
                set.iter().map(|name| format!("{{{name}}}")).collect::<Vec<_>>().join(", ")
            };
            return Err(Error::new(
                ErrorKind::Validation,
                "kernel.text_placeholders_mismatch",
                "The text must use exactly the placeholders of the default: {wanted} (it has: {got}).",
            )
            .with_param("wanted", list(&wanted))
            .with_param("got", list(&got)));
        }
    }
    if ui {
        let all = state.overrides.all(&tenant).await?;
        let keys: BTreeSet<&String> = all
            .keys()
            .filter(|(_, known)| known.starts_with("ui."))
            .map(|(_, known)| known)
            .collect();
        let max = state.overrides.ui_max.get().copied().unwrap_or(1000);
        if !keys.contains(&key)
            && <u64 as TryFrom<usize>>::try_from(keys.len()).unwrap_or(u64::MAX) >= max
        {
            return Err(Error::conflict(
                "kernel.text_limit_reached",
                "This tenant already replaces {max} interface texts.",
            )
            .with_param("max", max));
        }
    }
    let now: Timestamp = Timestamp::now();
    let insert = SqlQuery::insert()
        .into_table(Alias::new(TABLE))
        .columns(["id", "tenant_id", "locale", "key", "text", "updated_by", "updated_at"])
        .values_panic([
            Uuid::now_v7().into(),
            tenant.id().as_uuid().into(),
            locale.clone().into(),
            key.clone().into(),
            text.into(),
            principal.user_id().into(),
            now.into(),
        ])
        .on_conflict(
            OnConflict::columns([Alias::new("tenant_id"), Alias::new("locale"), Alias::new("key")])
                .update_columns([
                    Alias::new("text"),
                    Alias::new("updated_by"),
                    Alias::new("updated_at"),
                ])
                .to_owned(),
        )
        .to_owned();
    tenant.database().write().execute(&insert).await?;
    state.overrides.forget(tenant.id());
    let views = state.views(&tenant, &locale, Some(&key), false).await?;
    let view =
        views.into_iter().find(|view| view.key == key).ok_or_else(|| not_overridable(&key))?;
    Ok(Data { data: view })
}

/// Go back to the default text.
///
/// Requires `kernel.texts.manage`. Answers 204 even when the tenant had not replaced it.
#[utoipa::path(
    delete,
    path = "/texts/{locale}/{key}",
    extensions(("x-permissions" = json!(["kernel.texts.manage"]))),
    operation_id = "delete_text",
    security(("bearer" = [])),
    params(
        ("locale" = String, Path, description = "Language, e.g. `vi`."),
        ("key" = String, Path, description = "The text's key."),
    ),
    responses(
        (status = 204, description = "Back to the default."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `kernel.texts.manage`; code `permission_denied`.", body = Problem),
    )
)]
async fn delete_text(
    State(state): State<Arc<TextsState>>,
    tenant: TenantContext,
    principal: Principal,
    Path((locale, key)): Path<(String, String)>,
) -> Result<StatusCode, Error> {
    principal.require(&TEXTS_MANAGE)?;
    let delete = SqlQuery::delete()
        .from_table(Alias::new(TABLE))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .and_where(Expr::col(Alias::new("locale")).eq(locale))
        .and_where(Expr::col(Alias::new("key")).eq(key))
        .to_owned();
    tenant.database().write().execute(&delete).await?;
    state.overrides.forget(tenant.id());
    Ok(StatusCode::NO_CONTENT)
}

/// Which public texts.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct PublicTextsQuery {
    /// Language of the texts, e.g. `vi`. Defaults to `en`.
    locale: Option<String>,
    /// Only keys starting with this, e.g. `ui.`.
    prefix: Option<String>,
}

/// Get the tenant's wording.
///
/// No sign-in needed. The texts the tenant of the request replaced in `locale` (key → text), for
/// an interface to use instead of its own. Carries an `ETag`: send it in `If-None-Match` to get
/// `304` when nothing changed.
#[utoipa::path(
    get,
    path = "/public/texts",
    extensions(("x-public" = json!(true))),
    operation_id = "get_public_texts",
    security((), ("bearer" = [])),
    params(PublicTextsQuery),
    responses(
        (status = 200, description = "Key → text.", body = Data<BTreeMap<String, String>>, headers(("ETag" = String, description = "Version of these texts."))),
        (status = 304, description = "Not modified since the `ETag` sent in `If-None-Match`."),
        (status = 400, description = "The request names no tenant on a server with many; code `tenant_required`.", body = Problem),
    )
)]
async fn get_public_texts(
    State(state): State<Arc<TextsState>>,
    tenant: TenantContext,
    Query(query): Query<PublicTextsQuery>,
) -> Result<Response, Error> {
    let locale = query.locale.unwrap_or_else(|| "en".to_owned());
    let all = state.overrides.all(&tenant).await?;
    let texts: BTreeMap<String, String> = all
        .iter()
        .filter(|((text_locale, key), _)| {
            text_locale == &locale
                && query.prefix.as_deref().is_none_or(|prefix| key.starts_with(prefix))
        })
        .map(|((_, key), text)| (key.clone(), text.clone()))
        .collect();
    let version = crate::secret::sha256(&serde_json::to_string(&texts).map_err(Error::internal)?);
    let mut response = Data { data: texts }.into_response();
    if let Ok(value) = HeaderValue::from_str(&format!("\"{}\"", &version[..22])) {
        response.headers_mut().insert(header::ETAG, value);
    }
    Ok(response)
}

pub(crate) fn routes(state: TextsState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_texts))
        .routes(routes!(set_text, delete_text))
        .routes(routes!(get_public_texts))
        .with_state(Arc::new(state))
}

/// Migration tạo bảng `kernel_text_overrides` (cấp khách hàng, mọi database dữ liệu).
pub(crate) struct CreateTextOverrides;

impl MigrationName for CreateTextOverrides {
    fn name(&self) -> &str {
        "m0028_create_text_overrides"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateTextOverrides {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(TABLE))
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("locale", 16))
                    .col(schema::string_len("key", 200))
                    .col(schema::string_len("text", 2000))
                    .col(schema::uuid("updated_by"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("kernel_text_overrides_key")
                    .table(Alias::new(TABLE))
                    .col(Alias::new("tenant_id"))
                    .col(Alias::new("locale"))
                    .col(Alias::new("key"))
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Alias::new(TABLE)).to_owned()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cho_dien_cua_cau() {
        let names: Vec<String> =
            placeholders("Hi {name}, order {order_id} {}{ x}").into_iter().collect();
        assert_eq!(names, ["name", "order_id"]);
        assert!(is_key("ui.orders.title") && !is_key("UI.x") && !is_key("ui"));
    }
}
