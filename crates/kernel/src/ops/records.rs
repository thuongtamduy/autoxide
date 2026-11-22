//! Ghi chú và dòng hoạt động trên mọi bản ghi (bước 51 P13, D-201): một chỗ dùng chung thay cho mỗi
//! module tự làm ghi chú.
//!
//! - `GET /api/v1/kernel/records/{kind}/{id}/activity` — ghi chú nội bộ và lịch sử event của bản ghi
//!   (do module nhật ký công bố `RecordHistory`), mới trước.
//! - `GET` / `POST /api/v1/kernel/records/{kind}/{id}/notes`, `DELETE …/notes/{note_id}` (chỉ người
//!   viết) — bảng `kernel_record_notes` (cấp khách hàng). `mentions` nhắc người khác: mỗi người phải
//!   đọc được bản ghi; event `kernel.record_note_added` mang danh sách để module thông báo gửi.
//!
//! "Đọc được bản ghi": bộ tra của sổ đối tượng (D-123) thấy bản ghi, và người gọi có permission
//! `<module>.read` của module sở hữu loại đó. Không thì 404 như không có (không lộ sự tồn tại).

use crate::time::Timestamp;
use std::sync::{Arc, OnceLock};

use axum::Extension;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, Order, Query as SqlQuery};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::access::{Authenticator, PermissionCatalog, Principal};
use crate::error::{Error, FieldError};
use crate::event::{Actor, EventInfo, Outbox};
use crate::http::ValidJson;
use crate::i18n::Language;
use crate::migration::prelude::*;
use crate::resource::{RecordHistory, ResourceDirectory};
use crate::response::{Created, Data, Problem};
use crate::tenancy::TenantContext;

const TABLE: &str = "kernel_record_notes";
/// Ghi chú dài tối đa chừng này ký tự.
const MAX_BODY: usize = 5_000;
/// Nhắc tối đa chừng này người mỗi ghi chú.
const MAX_MENTIONS: usize = 20;

/// Event: ai đó viết ghi chú trên một bản ghi (mang người được nhắc, để module thông báo gửi).
pub(crate) const RECORD_NOTE_ADDED: EventInfo =
    EventInfo::new("kernel.record_note_added", "An internal note was written on a record.");

#[derive(Debug, Serialize)]
struct RecordNoteAdded {
    kind: String,
    record_id: String,
    note_id: Uuid,
    mentions: Vec<Uuid>,
}

impl crate::event::Event for RecordNoteAdded {
    const NAME: &'static str = "kernel.record_note_added";
}

/// Chỗ giữ bộ lịch sử theo bản ghi: `Modules::start` điền sau khi mọi module khởi động.
#[derive(Clone, Default)]
pub(crate) struct HistorySlot(Arc<OnceLock<Option<Arc<dyn RecordHistory>>>>);

impl HistorySlot {
    pub(crate) fn fill(&self, history: Option<Arc<dyn RecordHistory>>) {
        let _ = self.0.set(history);
    }

    fn get(&self) -> Option<Arc<dyn RecordHistory>> {
        self.0.get().cloned().flatten()
    }
}

/// An internal note on a record.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct RecordNote {
    pub id: Uuid,
    pub author_id: Uuid,
    pub body: String,
    /// Users the note mentions.
    pub mentions: Vec<Uuid>,
    pub created_at: Timestamp,
}

/// A note to write.
#[derive(Debug, Clone, Deserialize, ToSchema, validator::Validate)]
#[serde(deny_unknown_fields)]
pub(crate) struct NewRecordNote {
    /// 1 to 5000 characters.
    pub body: String,
    /// Up to 20 users to notify; each must be able to read the record.
    #[serde(default)]
    pub mentions: Vec<Uuid>,
}

/// One line of a record's activity: a note, or an event recorded about it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct ActivityItem {
    /// `note` or `event`.
    #[schema(example = "event")]
    pub kind: String,
    pub id: Uuid,
    pub at: Timestamp,
    pub actor_id: Option<Uuid>,
    pub actor_name: Option<String>,
    /// With `event`: its name, e.g. `content.entry_published`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// With `event`: what happened, in the language of `Accept-Language`, when the event has a
    /// readable sentence; with `note`: the note.
    pub text: Option<String>,
    /// With `note`: users it mentions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mentions: Option<Vec<Uuid>>,
}

/// How much activity.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ActivityQuery {
    /// At most this many lines, 1 to 200. Defaults to 50.
    limit: Option<u32>,
}

#[derive(Clone)]
pub(crate) struct RecordsState {
    pub resources: ResourceDirectory,
    pub permissions: PermissionCatalog,
    pub outbox: Outbox,
    pub history: HistorySlot,
    pub references: crate::ops::usages::ReferenceSources,
}

fn now() -> Timestamp {
    Timestamp::now()
}

fn not_found(kind: &str, id: &str) -> Error {
    Error::not_found("kernel.record_not_found", "There is no `{kind}` record `{id}` for you.")
        .with_param("kind", kind.to_owned())
        .with_param("id", id.to_owned())
}

impl RecordsState {
    /// `principal` đọc được bản ghi `kind` / `id` không (xem đầu file).
    async fn can_read(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        kind: &str,
        id: &str,
    ) -> Result<bool, Error> {
        let module = kind.split('.').next().unwrap_or_default();
        let reader = format!("{module}.read");
        let Some(permission) =
            self.permissions.iter().find(|permission| permission.name() == reader).cloned()
        else {
            return Ok(false);
        };
        if principal.require(&permission).is_err() || !self.resources.knows(kind) {
            return Ok(false);
        }
        Ok(self.resources.resolve(tenant, kind, id).await?.is_some())
    }

    async fn require_read(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        kind: &str,
        id: &str,
    ) -> Result<(), Error> {
        if self.can_read(tenant, principal, kind, id).await? {
            Ok(())
        } else {
            Err(not_found(kind, id))
        }
    }
}

const COLUMNS: [&str; 5] = ["id", "author_id", "body", "mentions", "created_at"];

fn note(row: &sea_orm::QueryResult) -> Result<RecordNote, Error> {
    let mentions: String = row.try_get("", "mentions")?;
    Ok(RecordNote {
        id: row.try_get("", "id")?,
        author_id: row.try_get("", "author_id")?,
        body: row.try_get("", "body")?,
        mentions: serde_json::from_str(&mentions).unwrap_or_default(),
        created_at: row.try_get("", "created_at")?,
    })
}

async fn notes_of(
    tenant: &TenantContext,
    kind: &str,
    id: &str,
    limit: u64,
) -> Result<Vec<RecordNote>, Error> {
    let select = SqlQuery::select()
        .columns(COLUMNS)
        .from(Alias::new(TABLE))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .and_where(Expr::col(Alias::new("kind")).eq(kind))
        .and_where(Expr::col(Alias::new("record_id")).eq(id))
        .order_by(Alias::new("id"), Order::Desc)
        .limit(limit)
        .to_owned();
    let rows = tenant.database().read().query_all(&select).await?;
    rows.iter().map(note).collect()
}

/// List the notes of a record.
///
/// Requires being able to read the record (`<module>.read` of the module that owns `kind`). Newest
/// first, at most 200.
#[utoipa::path(
    get,
    path = "/records/{kind}/{id}/notes",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_record_notes",
    security(("bearer" = [])),
    params(
        ("kind" = String, Path, description = "Type of the record, e.g. `content.entry`."),
        ("id" = String, Path, description = "Id of the record."),
    ),
    responses(
        (status = 200, description = "The notes.", body = Data<Vec<RecordNote>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "No such record you may read; code `kernel.record_not_found`.", body = Problem),
    )
)]
async fn list_record_notes(
    State(state): State<Arc<RecordsState>>,
    tenant: TenantContext,
    principal: Principal,
    Path((kind, id)): Path<(String, String)>,
) -> Result<Data<Vec<RecordNote>>, Error> {
    state.require_read(&tenant, &principal, &kind, &id).await?;
    Ok(Data { data: notes_of(&tenant, &kind, &id, 200).await? })
}

/// Write a note on a record.
///
/// Requires being able to read the record. An internal note, seen by everyone who can read the
/// record; `mentions` notify users (each must be able to read the record), through the event
/// `kernel.record_note_added`.
#[utoipa::path(
    post,
    path = "/records/{kind}/{id}/notes",
    extensions(("x-permissions" = json!([]))),
    operation_id = "create_record_note",
    security(("bearer" = [])),
    params(
        ("kind" = String, Path, description = "Type of the record, e.g. `content.entry`."),
        ("id" = String, Path, description = "Id of the record."),
    ),
    request_body = NewRecordNote,
    responses(
        (status = 201, description = "Written.", body = Data<RecordNote>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "No such record you may read; code `kernel.record_not_found`.", body = Problem),
        (status = 422, description = "Empty or too long, or a mentioned user cannot read the record; see `errors`.", body = Problem),
    )
)]
async fn create_record_note(
    State(state): State<Arc<RecordsState>>,
    tenant: TenantContext,
    principal: Principal,
    Path((kind, id)): Path<(String, String)>,
    // Bộ xác thực của chương trình (lớp HTTP gắn vào mọi request): tra quyền người được nhắc.
    authenticator: Option<Extension<Arc<dyn Authenticator>>>,
    ValidJson(input): ValidJson<NewRecordNote>,
) -> Result<Created<RecordNote>, Error> {
    state.require_read(&tenant, &principal, &kind, &id).await?;
    let body = input.body.trim().to_owned();
    let mut problems = Vec::new();
    if body.is_empty() || body.chars().count() > MAX_BODY {
        problems.push(
            FieldError::new("body", "invalid", "must be 1 to {max} characters long")
                .with_param("max", MAX_BODY),
        );
    }
    let mut mentions = input.mentions.clone();
    mentions.sort();
    mentions.dedup();
    if mentions.len() > MAX_MENTIONS {
        problems.push(
            FieldError::new("mentions", "invalid", "may name at most {max} users")
                .with_param("max", MAX_MENTIONS),
        );
    } else {
        for (index, user) in mentions.iter().enumerate() {
            let them = match &authenticator {
                Some(Extension(authenticator)) => {
                    authenticator.principal_of(&tenant, *user).await?
                }
                None => None,
            };
            let readable = match them {
                Some(them) => state.can_read(&tenant, &them, &kind, &id).await?,
                None => false,
            };
            if !readable {
                problems.push(FieldError::new(
                    format!("mentions[{index}]"),
                    "invalid",
                    "is not an active user who can read this record",
                ));
            }
        }
    }
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    let note_id = Uuid::now_v7();
    let created_at = now();
    let transaction = tenant.database().begin_write().await?;
    let insert = SqlQuery::insert()
        .into_table(Alias::new(TABLE))
        .columns([
            "id",
            "tenant_id",
            "kind",
            "record_id",
            "author_id",
            "body",
            "mentions",
            "created_at",
        ])
        .values_panic([
            note_id.into(),
            tenant.id().as_uuid().into(),
            kind.clone().into(),
            id.clone().into(),
            principal.user_id().into(),
            body.clone().into(),
            serde_json::to_string(&mentions).map_err(Error::internal)?.into(),
            created_at.into(),
        ])
        .to_owned();
    transaction.execute(&insert).await?;
    state
        .outbox
        .record(
            &transaction,
            &tenant,
            Actor::from(&principal),
            Some(format!("{kind}:{id}")),
            &RecordNoteAdded {
                kind: kind.clone(),
                record_id: id.clone(),
                note_id,
                mentions: mentions.clone(),
            },
        )
        .await?;
    transaction.commit().await?;
    let created =
        RecordNote { id: note_id, author_id: principal.user_id(), body, mentions, created_at };
    Ok(Created::new(format!("/api/v1/kernel/records/{kind}/{id}/notes/{note_id}"), created))
}

/// Delete your note on a record.
///
/// Only the note's author, who must still be able to read the record.
#[utoipa::path(
    delete,
    path = "/records/{kind}/{id}/notes/{note_id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "delete_record_note",
    security(("bearer" = [])),
    params(
        ("kind" = String, Path, description = "Type of the record, e.g. `content.entry`."),
        ("id" = String, Path, description = "Id of the record."),
        ("note_id" = Uuid, Path, description = "Id of the note."),
    ),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "No such record you may read, or no note of yours with this id; code `kernel.record_not_found` or `kernel.note_not_found`.", body = Problem),
    )
)]
async fn delete_record_note(
    State(state): State<Arc<RecordsState>>,
    tenant: TenantContext,
    principal: Principal,
    Path((kind, id, note_id)): Path<(String, String, Uuid)>,
) -> Result<StatusCode, Error> {
    state.require_read(&tenant, &principal, &kind, &id).await?;
    let delete = SqlQuery::delete()
        .from_table(Alias::new(TABLE))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .and_where(Expr::col(Alias::new("kind")).eq(kind.as_str()))
        .and_where(Expr::col(Alias::new("record_id")).eq(id.as_str()))
        .and_where(Expr::col(Alias::new("id")).eq(note_id))
        .and_where(Expr::col(Alias::new("author_id")).eq(principal.user_id()))
        .to_owned();
    let deleted = tenant.database().write().execute(&delete).await?;
    if deleted.rows_affected() == 0 {
        return Err(Error::not_found("kernel.note_not_found", "You have no note `{id}` here.")
            .with_param("id", note_id));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Get the activity of a record.
///
/// Requires being able to read the record. Its internal notes and the events recorded about it
/// (when the program keeps an audit log), newest first.
#[utoipa::path(
    get,
    path = "/records/{kind}/{id}/activity",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_record_activity",
    security(("bearer" = [])),
    params(
        ("kind" = String, Path, description = "Type of the record, e.g. `content.entry`."),
        ("id" = String, Path, description = "Id of the record."),
        ActivityQuery,
    ),
    responses(
        (status = 200, description = "The activity.", body = Data<Vec<ActivityItem>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "No such record you may read; code `kernel.record_not_found`.", body = Problem),
        (status = 422, description = "`limit` is not between 1 and 200; see `errors`.", body = Problem),
    )
)]
async fn get_record_activity(
    State(state): State<Arc<RecordsState>>,
    tenant: TenantContext,
    principal: Principal,
    language: Language,
    Path((kind, id)): Path<(String, String)>,
    Query(query): Query<ActivityQuery>,
) -> Result<Data<Vec<ActivityItem>>, Error> {
    let limit = query.limit.unwrap_or(50);
    if !(1..=200).contains(&limit) {
        return Err(Error::validation(vec![FieldError::new(
            "limit",
            "invalid",
            "must be between 1 and 200",
        )]));
    }
    state.require_read(&tenant, &principal, &kind, &id).await?;
    let mut items: Vec<ActivityItem> = notes_of(&tenant, &kind, &id, u64::from(limit))
        .await?
        .into_iter()
        .map(|note| ActivityItem {
            kind: "note".to_owned(),
            id: note.id,
            at: note.created_at,
            actor_id: Some(note.author_id),
            actor_name: None,
            action: None,
            text: Some(note.body),
            mentions: Some(note.mentions),
        })
        .collect();
    if let Some(history) = state.history.get() {
        let subject = format!("{kind}:{id}");
        items.extend(history.history(&tenant, &subject, limit, language).await?.into_iter().map(
            |line| ActivityItem {
                kind: "event".to_owned(),
                id: line.id,
                at: line.at,
                actor_id: line.actor_user_id,
                actor_name: line.actor_name,
                action: Some(line.action),
                text: line.sentence,
                mentions: None,
            },
        ));
    }
    items.sort_by_key(|item| std::cmp::Reverse((item.at, item.id)));
    items.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    Ok(Data { data: items })
}

/// Which record's events.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct RecordEventsQuery {
    /// The record, as `<kind>:<id>`, e.g. `content.entry:0190…`.
    #[param(example = "content.entry:0190a4c2-0000-7000-8000-000000000001")]
    object: String,
    /// At most this many events, 1 to 200. Defaults to 50.
    limit: Option<u32>,
}

/// An event recorded about a record.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct RecordEvent {
    pub id: Uuid,
    pub at: Timestamp,
    /// Name of the event, e.g. `content.entry_published`.
    #[schema(example = "content.entry_published")]
    pub name: String,
    pub actor_id: Option<Uuid>,
    pub actor_name: Option<String>,
    /// What happened, in the language of `Accept-Language`, when the event has a readable
    /// sentence.
    pub text: Option<String>,
}

/// List the events of a record.
///
/// Requires `kernel.devtools.read`. The events recorded about the record in the audit log
/// (events themselves are removed once delivered), newest first; empty when the program keeps no
/// audit log.
#[utoipa::path(
    get,
    path = "/devtools/events",
    extensions(("x-permissions" = json!(["kernel.devtools.read"]))),
    operation_id = "list_record_events",
    security(("bearer" = [])),
    params(RecordEventsQuery),
    responses(
        (status = 200, description = "The events, newest first.", body = Data<Vec<RecordEvent>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `kernel.devtools.read`.", body = Problem),
        (status = 422, description = "`object` is not `<kind>:<id>` or `limit` is not between 1 and 200; see `errors`.", body = Problem),
    )
)]
async fn list_record_events(
    State(state): State<Arc<RecordsState>>,
    tenant: TenantContext,
    principal: Principal,
    language: Language,
    Query(query): Query<RecordEventsQuery>,
) -> Result<Data<Vec<RecordEvent>>, Error> {
    principal.require(&crate::ops::request_log::DEVTOOLS_READ)?;
    let mut problems = Vec::new();
    let valid = query.object.len() <= 300
        && query.object.split_once(':').is_some_and(|(kind, id)| {
            !kind.is_empty()
                && !id.is_empty()
                && kind
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._".contains(&b))
        });
    if !valid {
        problems.push(FieldError::new(
            "object",
            "invalid",
            "must be `<record kind>:<id>`, e.g. `content.entry:0190…`",
        ));
    }
    let limit = query.limit.unwrap_or(50);
    if !(1..=200).contains(&limit) {
        problems.push(FieldError::new("limit", "invalid", "must be between 1 and 200"));
    }
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    let Some(history) = state.history.get() else {
        return Ok(Data { data: Vec::new() });
    };
    let lines = history.history(&tenant, &query.object, limit, language).await?;
    Ok(Data {
        data: lines
            .into_iter()
            .map(|line| RecordEvent {
                id: line.id,
                at: line.at,
                name: line.action,
                actor_id: line.actor_user_id,
                actor_name: line.actor_name,
                text: line.sentence,
            })
            .collect(),
    })
}

/// How many usages.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct UsagesQuery {
    /// At most this many, 1 to 200. Defaults to 50.
    limit: Option<u32>,
}

/// Get where a record is used.
///
/// Requires being able to read the record. The records of every module that point to it — posts
/// using an image, menu items linking to a page… — limited to modules whose `<module>.read` you
/// hold. Use it before deleting or making a record private.
#[utoipa::path(
    get,
    path = "/records/{kind}/{id}/usages",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_record_usages",
    security(("bearer" = [])),
    params(
        ("kind" = String, Path, description = "Type of the record, e.g. `media.item`."),
        ("id" = String, Path, description = "Id of the record."),
        UsagesQuery,
    ),
    responses(
        (status = 200, description = "The records using it.", body = Data<Vec<crate::ops::usages::Referrer>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "No such record you may read; code `kernel.record_not_found`.", body = Problem),
        (status = 422, description = "`limit` is not between 1 and 200; see `errors`.", body = Problem),
    )
)]
async fn get_record_usages(
    State(state): State<Arc<RecordsState>>,
    tenant: TenantContext,
    principal: Principal,
    Path((kind, id)): Path<(String, String)>,
    Query(query): Query<UsagesQuery>,
) -> Result<Data<Vec<crate::ops::usages::Referrer>>, Error> {
    let limit = query.limit.unwrap_or(50);
    if !(1..=200).contains(&limit) {
        return Err(Error::validation(vec![FieldError::new(
            "limit",
            "invalid",
            "must be between 1 and 200",
        )]));
    }
    state.require_read(&tenant, &principal, &kind, &id).await?;
    let limit = usize::try_from(limit).unwrap_or(200);
    let mut found = Vec::new();
    for (module, source) in state.references.all() {
        // Chỉ module người gọi đọc được: tiêu đề bản ghi của module khác không lộ.
        let reader = format!("{module}.read");
        let readable = state
            .permissions
            .iter()
            .find(|permission| permission.name() == reader)
            .is_some_and(|permission| principal.require(permission).is_ok());
        if !readable {
            continue;
        }
        found.extend(source.referrers(&tenant, &kind, &id, limit).await?);
        if found.len() >= limit {
            break;
        }
    }
    found.truncate(limit);
    Ok(Data { data: found })
}

/// A type of record in the resource directory.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct RecordType {
    /// `<module>.<type>`, e.g. `content.entry`.
    #[schema(example = "content.term")]
    kind: String,
    /// The module that owns the type.
    module: String,
    /// `true` when records of this type form a tree (e.g. categories): a menu item can list a
    /// record's children automatically.
    has_children: bool,
}

/// List the types of record that menus, embeds and notes can point to.
///
/// Any signed-in user; only the types of modules whose `<module>.read` you hold. To find records of
/// a type, use `GET /api/v1/kernel/find?q=…` with the operator `is:<type>`.
#[utoipa::path(
    get,
    path = "/records/types",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_record_types",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The types, sorted by name.", body = Data<Vec<RecordType>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
async fn list_record_types(
    State(state): State<Arc<RecordsState>>,
    principal: Principal,
) -> Result<Data<Vec<RecordType>>, Error> {
    let mut types: Vec<RecordType> = state
        .resources
        .types()
        .into_iter()
        .filter_map(|kind| {
            let module = kind.split_once('.').map_or(kind, |(module, _)| module);
            let reader = format!("{module}.read");
            let readable = state
                .permissions
                .iter()
                .find(|permission| permission.name() == reader)
                .is_some_and(|permission| principal.require(permission).is_ok());
            readable.then(|| RecordType {
                kind: kind.to_owned(),
                module: module.to_owned(),
                has_children: state.resources.has_children(kind),
            })
        })
        .collect();
    types.sort_by(|a, b| a.kind.cmp(&b.kind));
    Ok(Data { data: types })
}

pub(crate) fn routes(state: RecordsState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(get_record_usages))
        .routes(routes!(list_record_types))
        .routes(routes!(list_record_events))
        .routes(routes!(list_record_notes, create_record_note))
        .routes(routes!(delete_record_note))
        .routes(routes!(get_record_activity))
        .with_state(Arc::new(state))
}

/// Migration tạo bảng `kernel_record_notes` (cấp khách hàng, mọi database dữ liệu).
pub(crate) struct CreateRecordNotes;

impl MigrationName for CreateRecordNotes {
    fn name(&self) -> &str {
        "m0023_create_record_notes"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateRecordNotes {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(TABLE))
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("kind", 128))
                    .col(schema::string_len("record_id", 200))
                    .col(schema::uuid("author_id"))
                    .col(schema::text("body"))
                    .col(schema::text("mentions"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("kernel_record_notes_record_idx")
                    .table(Alias::new(TABLE))
                    .col(Alias::new("tenant_id"))
                    .col(Alias::new("kind"))
                    .col(Alias::new("record_id"))
                    .col(Alias::new("id"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Alias::new(TABLE)).to_owned()).await
    }
}
