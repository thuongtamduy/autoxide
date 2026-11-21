//! Nhãn nội bộ gắn bản ghi (bước 51 P20, D-235): "VIP", "cần gọi lại"… cho mọi loại bản ghi module
//! cho phép (`StartContext::taggable`). Nhãn là dữ liệu nội bộ của khách, không có trong đọc công
//! khai — khác thẻ công khai của `content` và nhãn của `media` (giữ nguyên).
//!
//! - `kernel_tags` (tên, tên đã chuẩn hoá — chữ thường, bỏ dấu, duy nhất trong khách —, màu trong
//!   bảng cố định, mô tả) và `kernel_record_tags` (loại, ID bản ghi, nhãn). Bảng của kernel vì lọc
//!   danh sách cần subquery trong câu SQL của module ([`TagIndex::condition`]).
//! - `GET/POST /api/v1/kernel/tags`, `PATCH/DELETE /api/v1/kernel/tags/{id}`, `POST …/{id}/merge`
//!   (`kernel.tags.manage`; đọc danh sách thì ai đăng nhập cũng được).
//! - `PUT /api/v1/kernel/records/{kind}/{id}/tags {tag_ids, new_tags}` — cần quyền GHI bản ghi
//!   (permission loại đó khai); tạo nhãn mới lúc gắn được, trừ khi khách bật `kernel.tags.restricted`
//!   (khi đó cần `kernel.tags.manage`). Tối đa 50 nhãn mỗi bản ghi. Event `kernel.record_tags_changed`.

use crate::time::Timestamp;
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, PoisonError, RwLock};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use sea_orm::sea_query::{Alias, Condition, Expr, Order, Query as SqlQuery};
use sea_orm::{ColumnTrait, ConnectionTrait};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::access::{Permission, Principal};
use crate::error::{Error, FieldError};
use crate::event::{Actor, EventInfo, Outbox};
use crate::http::ValidJson;
use crate::migration::prelude::*;
use crate::resource::ResourceDirectory;
use crate::response::{Created, Data, Problem};
use crate::tenancy::TenantContext;
use crate::tenancy::settings::{TenantSetting, TenantSettings};

const TAGS: &str = "kernel_tags";
const RECORD_TAGS: &str = "kernel_record_tags";
/// Tối đa chừng này nhãn trên một bản ghi.
const MAX_PER_RECORD: usize = 50;
/// Màu nhãn được dùng.
pub const COLORS: [&str; 9] =
    ["gray", "red", "orange", "yellow", "green", "teal", "blue", "purple", "pink"];

/// Quản lý nhãn của khách.
pub const TAGS_MANAGE: Permission = Permission::new(
    "kernel.tags.manage",
    "Create, rename, merge and delete the tenant's internal tags.",
);

/// Cài đặt khách: `true` thì chỉ người có `kernel.tags.manage` tạo nhãn mới.
pub const TAGS_RESTRICTED: TenantSetting = TenantSetting::new(
    "kernel.tags.restricted",
    "`true`: only people with `kernel.tags.manage` create new tags; others pick existing ones.",
);

/// Event: nhãn của một bản ghi đổi (tự động hoá, bước 58).
pub(crate) const RECORD_TAGS_CHANGED: EventInfo =
    EventInfo::new("kernel.record_tags_changed", "The internal tags of a record changed.");

#[derive(Debug, Serialize)]
struct RecordTagsChanged {
    kind: String,
    id: String,
    added: Vec<Uuid>,
    removed: Vec<Uuid>,
}

impl crate::event::Event for RecordTagsChanged {
    const NAME: &'static str = "kernel.record_tags_changed";
}

/// Sổ loại bản ghi gắn nhãn được: loại → permission ghi. Clone rẻ.
#[derive(Clone, Default)]
pub struct Taggables {
    entries: Arc<RwLock<Vec<(&'static str, Permission)>>>,
}

impl Taggables {
    pub(crate) fn register(
        &self,
        module: &'static str,
        kind: &'static str,
        write: Permission,
    ) -> Result<(), Error> {
        if !kind.starts_with(&format!("{module}.")) {
            return Err(Error::internal(format!(
                "taggable type `{kind}` of module `{module}` must be named `{module}.<type>`"
            )));
        }
        let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
        if entries.iter().any(|(known, _)| *known == kind) {
            return Err(Error::internal(format!("taggable type `{kind}` is registered twice")));
        }
        entries.push((kind, write));
        Ok(())
    }

    fn write_permission(&self, kind: &str) -> Option<Permission> {
        self.entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|(known, _)| *known == kind)
            .map(|(_, permission)| *permission)
    }
}

/// A tag, as attached to a record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TagRef {
    pub id: Uuid,
    pub name: String,
    /// One of `gray`, `red`, `orange`, `yellow`, `green`, `teal`, `blue`, `purple`, `pink`.
    pub color: String,
}

/// Nhãn của bản ghi và điều kiện lọc theo nhãn, cho module dùng trong danh sách của mình.
#[derive(Clone, Copy, Default)]
pub struct TagIndex;

impl TagIndex {
    /// Nhãn của các bản ghi `ids` loại `kind`, theo lô (một câu cho cả trang): ID → nhãn.
    pub async fn of_records(
        &self,
        tenant: &TenantContext,
        kind: &str,
        ids: &[String],
    ) -> Result<HashMap<String, Vec<TagRef>>, Error> {
        let mut found: HashMap<String, Vec<TagRef>> = HashMap::new();
        if ids.is_empty() {
            return Ok(found);
        }
        for chunk in ids.chunks(500) {
            let select = SqlQuery::select()
                .column((Alias::new(RECORD_TAGS), Alias::new("record_id")))
                .column((Alias::new(TAGS), Alias::new("id")))
                .column((Alias::new(TAGS), Alias::new("name")))
                .column((Alias::new(TAGS), Alias::new("color")))
                .from(Alias::new(RECORD_TAGS))
                .inner_join(
                    Alias::new(TAGS),
                    Expr::col((Alias::new(TAGS), Alias::new("id")))
                        .equals((Alias::new(RECORD_TAGS), Alias::new("tag_id"))),
                )
                .and_where(
                    Expr::col((Alias::new(RECORD_TAGS), Alias::new("tenant_id")))
                        .eq(tenant.id().as_uuid()),
                )
                .and_where(Expr::col((Alias::new(RECORD_TAGS), Alias::new("kind"))).eq(kind))
                .and_where(
                    Expr::col((Alias::new(RECORD_TAGS), Alias::new("record_id")))
                        .is_in(chunk.iter().cloned()),
                )
                .order_by((Alias::new(TAGS), Alias::new("name")), Order::Asc)
                .to_owned();
            for row in tenant.database().read().query_all(&select).await? {
                let record: String = row.try_get("", "record_id")?;
                found.entry(record).or_default().push(TagRef {
                    id: row.try_get("", "id")?,
                    name: row.try_get("", "name")?,
                    color: row.try_get("", "color")?,
                });
            }
        }
        Ok(found)
    }

    /// Điều kiện "cột `column` (ID bản ghi, kiểu UUID) có một trong nhãn `any` và mọi nhãn `all`",
    /// bằng subquery trên bảng nhãn của kernel — câu SQL thường, như nhau trên hai database.
    pub fn condition<Col: ColumnTrait>(
        &self,
        tenant: &TenantContext,
        kind: &str,
        column: Col,
        any: &[Uuid],
        all: &[Uuid],
    ) -> Condition {
        let records = |tags: &[Uuid]| {
            SqlQuery::select()
                .column(Alias::new("record_uuid"))
                .from(Alias::new(RECORD_TAGS))
                .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
                .and_where(Expr::col(Alias::new("kind")).eq(kind))
                .and_where(Expr::col(Alias::new("tag_id")).is_in(tags.iter().copied()))
                .to_owned()
        };
        let mut condition = Condition::all();
        if !any.is_empty() {
            condition = condition.add(column.in_subquery(records(any)));
        }
        for tag in all {
            condition = condition.add(column.in_subquery(records(std::slice::from_ref(tag))));
        }
        condition
    }
}

fn now() -> Timestamp {
    Timestamp::now()
}

/// A tag of the tenant.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct TagView {
    pub id: Uuid,
    pub name: String,
    pub color: String,
    pub description: Option<String>,
    /// Records carrying it.
    pub uses: i64,
    pub created_at: Timestamp,
}

/// A tag to create.
#[derive(Debug, Clone, Deserialize, ToSchema, validator::Validate)]
pub(crate) struct NewTag {
    /// 1 to 50 characters; unique in the tenant, ignoring case and accents.
    pub name: String,
    /// A color of the fixed palette; `gray` by default.
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Changes to a tag; omitted fields stay.
#[derive(Debug, Clone, Default, Deserialize, ToSchema, validator::Validate)]
pub(crate) struct TagChanges {
    pub name: Option<String>,
    pub color: Option<String>,
    /// `""` clears it.
    pub description: Option<String>,
}

/// The tag to merge into.
#[derive(Debug, Clone, Deserialize, ToSchema, validator::Validate)]
pub(crate) struct MergeTag {
    /// Every record of the merged tag gets this tag; the merged tag is deleted.
    pub into: Uuid,
}

/// The tags of a record.
#[derive(Debug, Clone, Deserialize, ToSchema, validator::Validate)]
pub(crate) struct RecordTagsInput {
    /// The tags the record has after the call (replaces the current ones).
    #[serde(default)]
    pub tag_ids: Vec<Uuid>,
    /// New tags to create and attach by name (an existing name is reused).
    #[serde(default)]
    pub new_tags: Vec<String>,
}

#[derive(Clone)]
pub(crate) struct TagsState {
    pub taggables: Taggables,
    pub resources: ResourceDirectory,
    pub outbox: Outbox,
    pub settings: Option<Arc<dyn TenantSettings>>,
}

fn check_name(name: &str, problems: &mut Vec<FieldError>, field: &str) {
    if !(1..=50).contains(&name.trim().chars().count()) {
        problems.push(FieldError::new(field, "invalid", "must be 1 to 50 characters long"));
    }
}

fn check_color(color: Option<&str>, problems: &mut Vec<FieldError>) {
    if let Some(color) = color
        && !COLORS.contains(&color)
    {
        problems.push(
            FieldError::new("color", "invalid", "must be one of {colors}")
                .with_param("colors", COLORS.join(", ")),
        );
    }
}

fn tag_not_found(id: Uuid) -> Error {
    Error::not_found("kernel.tag_not_found", "There is no tag `{id}`.").with_param("id", id)
}

fn name_taken(name: &str) -> Error {
    Error::conflict("kernel.tag_name_taken", "A tag named `{name}` already exists.")
        .with_param("name", name.to_owned())
}

async fn list_views(tenant: &TenantContext) -> Result<Vec<TagView>, Error> {
    let uses = SqlQuery::select()
        .expr(Expr::col(Alias::new("tag_id")).count())
        .from(Alias::new(RECORD_TAGS))
        .and_where(
            Expr::col((Alias::new(RECORD_TAGS), Alias::new("tag_id")))
                .equals((Alias::new(TAGS), Alias::new("id"))),
        )
        .to_owned();
    let select = SqlQuery::select()
        .columns(
            ["id", "name", "color", "description", "created_at"]
                .map(|column| (Alias::new(TAGS), Alias::new(column))),
        )
        .expr_as(
            sea_orm::sea_query::SimpleExpr::SubQuery(
                None,
                Box::new(uses.into_sub_query_statement()),
            ),
            Alias::new("uses"),
        )
        .from(Alias::new(TAGS))
        .and_where(Expr::col((Alias::new(TAGS), Alias::new("tenant_id"))).eq(tenant.id().as_uuid()))
        .order_by((Alias::new(TAGS), Alias::new("normalized")), Order::Asc)
        .to_owned();
    let rows = tenant.database().read().query_all(&select).await?;
    rows.iter()
        .map(|row| {
            Ok(TagView {
                id: row.try_get("", "id")?,
                name: row.try_get("", "name")?,
                color: row.try_get("", "color")?,
                description: row.try_get("", "description")?,
                uses: row.try_get("", "uses")?,
                created_at: row.try_get("", "created_at")?,
            })
        })
        .collect()
}

async fn find_view(tenant: &TenantContext, id: Uuid) -> Result<TagView, Error> {
    list_views(tenant).await?.into_iter().find(|tag| tag.id == id).ok_or_else(|| tag_not_found(id))
}

/// Tạo nhãn `name` (hay trả nhãn có sẵn cùng tên chuẩn hoá khi `reuse`). Chạy được trên kết nối
/// của transaction.
async fn create<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    principal: &Principal,
    name: &str,
    color: &str,
    description: Option<&str>,
    reuse: bool,
) -> Result<Uuid, Error> {
    let name = name.trim();
    let normalized = crate::search::fold(name);
    let existing = SqlQuery::select()
        .column(Alias::new("id"))
        .from(Alias::new(TAGS))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .and_where(Expr::col(Alias::new("normalized")).eq(normalized.as_str()))
        .to_owned();
    if let Some(row) = connection.query_one(&existing).await? {
        return if reuse { Ok(row.try_get("", "id")?) } else { Err(name_taken(name)) };
    }
    let id = Uuid::now_v7();
    let insert = SqlQuery::insert()
        .into_table(Alias::new(TAGS))
        .columns([
            "id",
            "tenant_id",
            "name",
            "normalized",
            "color",
            "description",
            "created_by",
            "created_at",
        ])
        .values_panic([
            id.into(),
            tenant.id().as_uuid().into(),
            name.into(),
            normalized.clone().into(),
            color.into(),
            description.map(str::to_owned).into(),
            principal.user_id().into(),
            now().into(),
        ])
        .to_owned();
    match connection.execute(&insert).await {
        Ok(_) => Ok(id),
        // Hai người tạo cùng tên một lúc: khoá duy nhất giữ một dòng; bên thua dùng dòng đó.
        Err(error) => match connection.query_one(&existing).await? {
            Some(row) if reuse => Ok(row.try_get("", "id")?),
            Some(_) => Err(name_taken(name)),
            None => Err(error.into()),
        },
    }
}

/// List the tags.
///
/// Any signed-in user. Every tag of the tenant, by name, with how many records carry it.
#[utoipa::path(
    get,
    path = "/tags",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_tags",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The tags.", body = Data<Vec<TagView>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
async fn list_tags(
    tenant: TenantContext,
    _principal: Principal,
) -> Result<Data<Vec<TagView>>, Error> {
    Ok(Data { data: list_views(&tenant).await? })
}

/// Create a tag.
///
/// Requires `kernel.tags.manage`.
#[utoipa::path(
    post,
    path = "/tags",
    extensions(("x-permissions" = json!(["kernel.tags.manage"]))),
    operation_id = "create_tag",
    security(("bearer" = [])),
    request_body = NewTag,
    responses(
        (status = 201, description = "Created.", body = Data<TagView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `kernel.tags.manage`; code `permission_denied`.", body = Problem),
        (status = 409, description = "The name is taken (ignoring case and accents); code `kernel.tag_name_taken`.", body = Problem),
        (status = 422, description = "Invalid name or color; see `errors`.", body = Problem),
    )
)]
async fn create_tag(
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<NewTag>,
) -> Result<Created<TagView>, Error> {
    principal.require(&TAGS_MANAGE)?;
    let mut problems = Vec::new();
    check_name(&input.name, &mut problems, "name");
    check_color(input.color.as_deref(), &mut problems);
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    let color = input.color.as_deref().unwrap_or("gray");
    let description = input.description.as_deref().map(str::trim).filter(|text| !text.is_empty());
    let id = create(
        tenant.database().write(),
        &tenant,
        &principal,
        &input.name,
        color,
        description,
        false,
    )
    .await?;
    Ok(Created::new(format!("/api/v1/kernel/tags/{id}"), find_view(&tenant, id).await?))
}

/// Change a tag.
///
/// Requires `kernel.tags.manage`.
#[utoipa::path(
    patch,
    path = "/tags/{id}",
    extensions(("x-permissions" = json!(["kernel.tags.manage"]))),
    operation_id = "update_tag",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the tag.")),
    request_body = TagChanges,
    responses(
        (status = 200, description = "The tag.", body = Data<TagView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `kernel.tags.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such tag; code `kernel.tag_not_found`.", body = Problem),
        (status = 409, description = "The name is taken; code `kernel.tag_name_taken`.", body = Problem),
        (status = 422, description = "Invalid name or color; see `errors`.", body = Problem),
    )
)]
async fn update_tag(
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(changes): ValidJson<TagChanges>,
) -> Result<Data<TagView>, Error> {
    principal.require(&TAGS_MANAGE)?;
    let current = find_view(&tenant, id).await?;
    let mut problems = Vec::new();
    if let Some(name) = &changes.name {
        check_name(name, &mut problems, "name");
    }
    check_color(changes.color.as_deref(), &mut problems);
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    let mut update = SqlQuery::update();
    update
        .table(Alias::new(TAGS))
        .value(Alias::new("color"), changes.color.unwrap_or(current.color))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .and_where(Expr::col(Alias::new("id")).eq(id));
    if let Some(name) = changes.name {
        let name = name.trim().to_owned();
        let normalized = crate::search::fold(&name);
        let taken = list_views(&tenant)
            .await?
            .iter()
            .any(|tag| tag.id != id && crate::search::fold(&tag.name) == normalized);
        if taken {
            return Err(name_taken(&name));
        }
        update.value(Alias::new("name"), name).value(Alias::new("normalized"), normalized);
    }
    if let Some(description) = changes.description {
        let description = description.trim().to_owned();
        update.value(Alias::new("description"), (!description.is_empty()).then_some(description));
    }
    tenant.database().write().execute(&update).await?;
    Ok(Data { data: find_view(&tenant, id).await? })
}

/// Delete a tag.
///
/// Requires `kernel.tags.manage`. Records that carried it lose it.
#[utoipa::path(
    delete,
    path = "/tags/{id}",
    extensions(("x-permissions" = json!(["kernel.tags.manage"]))),
    operation_id = "delete_tag",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the tag.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `kernel.tags.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such tag; code `kernel.tag_not_found`.", body = Problem),
    )
)]
async fn delete_tag(
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&TAGS_MANAGE)?;
    find_view(&tenant, id).await?;
    let transaction = tenant.database().begin_write().await?;
    for table in [RECORD_TAGS, TAGS] {
        let column = if table == TAGS { "id" } else { "tag_id" };
        let delete = SqlQuery::delete()
            .from_table(Alias::new(table))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
            .and_where(Expr::col(Alias::new(column)).eq(id))
            .to_owned();
        transaction.execute(&delete).await?;
    }
    transaction.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Merge a tag into another.
///
/// Requires `kernel.tags.manage`. Every record carrying this tag carries `into` instead; this tag
/// is deleted.
#[utoipa::path(
    post,
    path = "/tags/{id}/merge",
    extensions(("x-permissions" = json!(["kernel.tags.manage"]))),
    operation_id = "merge_tag",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the tag to merge away.")),
    request_body = MergeTag,
    responses(
        (status = 200, description = "The tag merged into.", body = Data<TagView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `kernel.tags.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such tag; code `kernel.tag_not_found`.", body = Problem),
        (status = 422, description = "`into` is the same tag; see `errors`.", body = Problem),
    )
)]
async fn merge_tag(
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<MergeTag>,
) -> Result<Data<TagView>, Error> {
    principal.require(&TAGS_MANAGE)?;
    if input.into == id {
        return Err(Error::validation(vec![FieldError::new(
            "into",
            "invalid",
            "must be another tag",
        )]));
    }
    find_view(&tenant, id).await?;
    find_view(&tenant, input.into).await?;
    let transaction = tenant.database().begin_write().await?;
    let tenant_id = tenant.id().as_uuid();
    // Bản ghi đã có cả hai nhãn: bỏ dòng của nhãn bị gộp trước, rồi chuyển phần còn lại.
    let both = SqlQuery::select()
        .column(Alias::new("record_id"))
        .from(Alias::new(RECORD_TAGS))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant_id))
        .and_where(Expr::col(Alias::new("tag_id")).eq(input.into))
        .to_owned();
    let rows = transaction.query_all(&both).await?;
    let mut already = BTreeSet::new();
    for row in rows {
        already.insert(row.try_get::<String>("", "record_id")?);
    }
    if !already.is_empty() {
        let delete = SqlQuery::delete()
            .from_table(Alias::new(RECORD_TAGS))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant_id))
            .and_where(Expr::col(Alias::new("tag_id")).eq(id))
            .and_where(Expr::col(Alias::new("record_id")).is_in(already))
            .to_owned();
        transaction.execute(&delete).await?;
    }
    let moved = SqlQuery::update()
        .table(Alias::new(RECORD_TAGS))
        .value(Alias::new("tag_id"), input.into)
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant_id))
        .and_where(Expr::col(Alias::new("tag_id")).eq(id))
        .to_owned();
    transaction.execute(&moved).await?;
    let delete = SqlQuery::delete()
        .from_table(Alias::new(TAGS))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant_id))
        .and_where(Expr::col(Alias::new("id")).eq(id))
        .to_owned();
    transaction.execute(&delete).await?;
    transaction.commit().await?;
    Ok(Data { data: find_view(&tenant, input.into).await? })
}

/// Set the tags of a record.
///
/// Requires the permission that writes the record (each taggable type declares it). Replaces the
/// record's tags with `tag_ids`, plus `new_tags` created by name (an existing name is reused;
/// creating needs `kernel.tags.manage` when the tenant set `kernel.tags.restricted`). At most 50
/// tags per record. Emits `kernel.record_tags_changed`.
#[utoipa::path(
    put,
    path = "/records/{kind}/{id}/tags",
    extensions(("x-permissions" = json!([]))),
    operation_id = "set_record_tags",
    security(("bearer" = [])),
    params(
        ("kind" = String, Path, description = "Type of the record, e.g. `content.entry`."),
        ("id" = String, Path, description = "Id of the record."),
    ),
    request_body = RecordTagsInput,
    responses(
        (status = 200, description = "The record's tags.", body = Data<Vec<TagRef>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "You may not write the record (`permission_denied`), or create tags here (`kernel.tags_restricted`).", body = Problem),
        (status = 404, description = "No such taggable record (`kernel.record_not_found`) or tag (`kernel.tag_not_found`).", body = Problem),
        (status = 409, description = "More than 50 tags; code `kernel.tag_limit_reached`.", body = Problem),
        (status = 422, description = "A new tag's name is invalid; see `errors`.", body = Problem),
    )
)]
async fn set_record_tags(
    State(state): State<Arc<TagsState>>,
    tenant: TenantContext,
    principal: Principal,
    Path((kind, id)): Path<(String, String)>,
    ValidJson(input): ValidJson<RecordTagsInput>,
) -> Result<Data<Vec<TagRef>>, Error> {
    let missing = || {
        Error::not_found("kernel.record_not_found", "There is no `{kind}` record `{id}` for you.")
            .with_param("kind", kind.clone())
            .with_param("id", id.clone())
    };
    let write = state.taggables.write_permission(&kind).ok_or_else(missing)?;
    principal.require(&write)?;
    if !state.resources.knows(&kind)
        || state.resources.resolve(&tenant, &kind, &id).await?.is_none()
    {
        return Err(missing());
    }
    let mut problems = Vec::new();
    for (index, name) in input.new_tags.iter().enumerate() {
        check_name(name, &mut problems, &format!("new_tags[{index}]"));
    }
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    if !input.new_tags.is_empty() && principal.require(&TAGS_MANAGE).is_err() {
        let restricted = match &state.settings {
            Some(settings) => settings
                .get(&tenant, &TAGS_RESTRICTED)
                .await?
                .is_some_and(|value| value.as_str() == "true"),
            None => false,
        };
        if restricted {
            return Err(Error::forbidden(
                "kernel.tags_restricted",
                "Only people with `kernel.tags.manage` create tags in this tenant; pick existing ones.",
            ));
        }
    }
    let known: BTreeSet<Uuid> = list_views(&tenant).await?.into_iter().map(|tag| tag.id).collect();
    if let Some(unknown) = input.tag_ids.iter().find(|tag| !known.contains(tag)) {
        return Err(tag_not_found(*unknown));
    }
    let transaction = tenant.database().begin_write().await?;
    let mut wanted: BTreeSet<Uuid> = input.tag_ids.iter().copied().collect();
    for name in &input.new_tags {
        wanted.insert(create(&transaction, &tenant, &principal, name, "gray", None, true).await?);
    }
    if wanted.len() > MAX_PER_RECORD {
        transaction.rollback().await?;
        return Err(Error::conflict(
            "kernel.tag_limit_reached",
            "A record carries at most {max} tags.",
        )
        .with_param("max", MAX_PER_RECORD));
    }
    let tenant_id = tenant.id().as_uuid();
    let current = SqlQuery::select()
        .column(Alias::new("tag_id"))
        .from(Alias::new(RECORD_TAGS))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant_id))
        .and_where(Expr::col(Alias::new("kind")).eq(kind.as_str()))
        .and_where(Expr::col(Alias::new("record_id")).eq(id.as_str()))
        .to_owned();
    let mut had = BTreeSet::new();
    for row in transaction.query_all(&current).await? {
        had.insert(row.try_get::<Uuid>("", "tag_id")?);
    }
    let added: Vec<Uuid> = wanted.difference(&had).copied().collect();
    let removed: Vec<Uuid> = had.difference(&wanted).copied().collect();
    if !removed.is_empty() {
        let delete = SqlQuery::delete()
            .from_table(Alias::new(RECORD_TAGS))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant_id))
            .and_where(Expr::col(Alias::new("kind")).eq(kind.as_str()))
            .and_where(Expr::col(Alias::new("record_id")).eq(id.as_str()))
            .and_where(Expr::col(Alias::new("tag_id")).is_in(removed.clone()))
            .to_owned();
        transaction.execute(&delete).await?;
    }
    // ID bản ghi dạng UUID thì giữ thêm ở cột UUID, để module lọc bằng subquery cùng kiểu cột.
    let record_uuid = id.parse::<Uuid>().ok();
    for tag in &added {
        let insert = SqlQuery::insert()
            .into_table(Alias::new(RECORD_TAGS))
            .columns([
                "id",
                "tenant_id",
                "kind",
                "record_id",
                "record_uuid",
                "tag_id",
                "created_by",
                "created_at",
            ])
            .values_panic([
                Uuid::now_v7().into(),
                tenant_id.into(),
                kind.clone().into(),
                id.clone().into(),
                record_uuid.into(),
                (*tag).into(),
                principal.user_id().into(),
                now().into(),
            ])
            .to_owned();
        transaction.execute(&insert).await?;
    }
    if !added.is_empty() || !removed.is_empty() {
        state
            .outbox
            .record(
                &transaction,
                &tenant,
                Actor::from(&principal),
                Some(format!("{kind}:{id}")),
                &RecordTagsChanged { kind: kind.clone(), id: id.clone(), added, removed },
            )
            .await?;
    }
    transaction.commit().await?;
    let tags = TagIndex.of_records(&tenant, &kind, std::slice::from_ref(&id)).await?;
    Ok(Data { data: tags.get(&id).cloned().unwrap_or_default() })
}

/// Records to tag or untag in one call.
#[derive(Debug, Clone, Deserialize, ToSchema, validator::Validate)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApplyTagInput {
    /// Type of the records, e.g. `content.entry`.
    #[schema(example = "content.entry")]
    pub kind: String,
    /// Records that get the tag (those that have it already are skipped).
    #[serde(default)]
    pub add: Vec<String>,
    /// Records that lose the tag.
    #[serde(default)]
    pub remove: Vec<String>,
}

/// What a bulk tagging changed.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct ApplyTagResult {
    /// Records that got the tag.
    pub added: Vec<String>,
    /// Records that lost the tag.
    pub removed: Vec<String>,
    /// Records left as they were: already tagged, untagged, at 50 tags, or not found.
    pub skipped: Vec<String>,
}

/// Bản ghi tối đa của một lần gắn hàng loạt.
const MAX_APPLY: usize = 500;

/// Tag or untag many records at once.
///
/// Requires the write permission of the record type (as for `PUT /records/{kind}/{id}/tags`). Adds
/// the tag to the records of `add` and removes it from those of `remove` — up to 500 records in all,
/// in one transaction, one `kernel.record_tags_changed` event per changed record. Records not found,
/// already in the wanted state, or at 50 tags are `skipped`. To act on a filter, ask
/// `POST /api/v1/kernel/bulk/{resource}/{action}` with `dry_run` for the ids first.
#[utoipa::path(
    post,
    path = "/tags/{id}/apply",
    extensions(("x-permissions" = json!([]))),
    operation_id = "apply_tag",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the tag.")),
    request_body = ApplyTagInput,
    responses(
        (status = 200, description = "What changed.", body = Data<ApplyTagResult>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "You may not write records of this type; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such tag (`kernel.tag_not_found`) or taggable type (`kernel.record_not_found`).", body = Problem),
        (status = 422, description = "No records, or more than 500; see `errors`.", body = Problem),
    )
)]
async fn apply_tag(
    State(state): State<Arc<TagsState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(tag): Path<Uuid>,
    ValidJson(input): ValidJson<ApplyTagInput>,
) -> Result<Data<ApplyTagResult>, Error> {
    let kind = input.kind.clone();
    let write = state.taggables.write_permission(&kind).ok_or_else(|| {
        Error::not_found("kernel.record_not_found", "There is no `{kind}` record `{id}` for you.")
            .with_param("kind", kind.clone())
            .with_param("id", "*")
    })?;
    let total = input.add.len() + input.remove.len();
    if total == 0 || total > MAX_APPLY {
        return Err(Error::validation(vec![
            FieldError::new("add", "invalid", "must hold 1 to {max} ids together with `remove`")
                .with_param("max", MAX_APPLY),
        ]));
    }
    find_view(&tenant, tag).await?;
    principal.require(&write)?;
    // Tra bản ghi NGOÀI transaction ghi (có thể gọi module khác).
    let found = state.resources.resolve_many(&tenant, &kind, &input.add, None).await?;
    let tenant_id = tenant.id().as_uuid();
    let mut result = ApplyTagResult { added: Vec::new(), removed: Vec::new(), skipped: Vec::new() };
    let transaction = tenant.database().begin_write().await?;
    for id in &input.add {
        if !found.contains_key(id) {
            result.skipped.push(id.clone());
            continue;
        }
        let current = SqlQuery::select()
            .column(Alias::new("tag_id"))
            .from(Alias::new(RECORD_TAGS))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant_id))
            .and_where(Expr::col(Alias::new("kind")).eq(kind.as_str()))
            .and_where(Expr::col(Alias::new("record_id")).eq(id.as_str()))
            .to_owned();
        let had: Vec<Uuid> = transaction
            .query_all(&current)
            .await?
            .iter()
            .map(|row| row.try_get::<Uuid>("", "tag_id"))
            .collect::<Result<_, _>>()?;
        if had.contains(&tag) || had.len() >= MAX_PER_RECORD {
            result.skipped.push(id.clone());
            continue;
        }
        let insert = SqlQuery::insert()
            .into_table(Alias::new(RECORD_TAGS))
            .columns([
                "id",
                "tenant_id",
                "kind",
                "record_id",
                "record_uuid",
                "tag_id",
                "created_by",
                "created_at",
            ])
            .values_panic([
                Uuid::now_v7().into(),
                tenant_id.into(),
                kind.clone().into(),
                id.clone().into(),
                id.parse::<Uuid>().ok().into(),
                tag.into(),
                principal.user_id().into(),
                now().into(),
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
                &RecordTagsChanged {
                    kind: kind.clone(),
                    id: id.clone(),
                    added: vec![tag],
                    removed: Vec::new(),
                },
            )
            .await?;
        result.added.push(id.clone());
    }
    for id in &input.remove {
        let delete = SqlQuery::delete()
            .from_table(Alias::new(RECORD_TAGS))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant_id))
            .and_where(Expr::col(Alias::new("kind")).eq(kind.as_str()))
            .and_where(Expr::col(Alias::new("record_id")).eq(id.as_str()))
            .and_where(Expr::col(Alias::new("tag_id")).eq(tag))
            .to_owned();
        if transaction.execute(&delete).await?.rows_affected() == 0 {
            result.skipped.push(id.clone());
            continue;
        }
        state
            .outbox
            .record(
                &transaction,
                &tenant,
                Actor::from(&principal),
                Some(format!("{kind}:{id}")),
                &RecordTagsChanged {
                    kind: kind.clone(),
                    id: id.clone(),
                    added: Vec::new(),
                    removed: vec![tag],
                },
            )
            .await?;
        result.removed.push(id.clone());
    }
    transaction.commit().await?;
    Ok(Data { data: result })
}

pub(crate) fn routes(state: TagsState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(apply_tag))
        .routes(routes!(list_tags, create_tag))
        .routes(routes!(update_tag, delete_tag))
        .routes(routes!(merge_tag))
        .routes(routes!(set_record_tags))
        .with_state(Arc::new(state))
}

/// Migration tạo `kernel_tags` và `kernel_record_tags` (cấp khách hàng, mọi database dữ liệu).
pub(crate) struct CreateTags;

impl MigrationName for CreateTags {
    fn name(&self) -> &str {
        "m0027_create_tags"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateTags {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(TAGS))
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("name", 200))
                    .col(schema::string_len("normalized", 200))
                    .col(schema::string_len("color", 16))
                    .col(schema::string_len_null("description", 500))
                    .col(schema::uuid("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("kernel_tags_normalized")
                    .table(Alias::new(TAGS))
                    .col(Alias::new("tenant_id"))
                    .col(Alias::new("normalized"))
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(RECORD_TAGS))
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("kind", 128))
                    .col(schema::string_len("record_id", 200))
                    .col(schema::uuid_null("record_uuid"))
                    .col(schema::uuid("tag_id"))
                    .col(schema::uuid("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        for (name, columns, unique) in [
            ("kernel_record_tags_record", vec!["tenant_id", "kind", "record_id", "tag_id"], true),
            (
                "kernel_record_tags_tag_idx",
                vec!["tenant_id", "tag_id", "kind", "record_uuid"],
                false,
            ),
        ] {
            let mut index = Index::create();
            index.name(name).table(Alias::new(RECORD_TAGS));
            for column in columns {
                index.col(Alias::new(column));
            }
            if unique {
                index.unique();
            }
            manager.create_index(index.to_owned()).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Alias::new(RECORD_TAGS)).to_owned()).await?;
        manager.drop_table(Table::drop().table(Alias::new(TAGS)).to_owned()).await
    }
}
