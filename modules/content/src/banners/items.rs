//! Banner của vị trí (bước 55, D-236): cần `content.banners` ở nút của banner (`org_id`, gốc = mọi
//! cửa hàng). Kiểm ảnh (thư viện `media`), link và trang được chọn (sổ đối tượng) chạy TRƯỚC
//! transaction ghi: transaction phải ngắn, không gọi module khác.

use std::collections::{HashMap, HashSet};

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, root_of};
use chrono::{DateTime, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use super::placements::Placement;
use super::{banner_not_found, check_choice, strings};
use crate::entity::{banner, placement};
use crate::events::{self, Change};
use crate::permissions::{CONTENT_BANNERS, CONTENT_READ};
use crate::service::{ContentService, now};

pub(crate) type Banner = banner::Model;

/// Trạng thái của banner.
pub(crate) const STATUSES: [&str; 3] = ["draft", "active", "archived"];

/// Một trang cụ thể banner hiện ở: đối tượng trong sổ đối tượng (vd `catalog.category`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PageTarget {
    pub kind: String,
    pub id: String,
}

/// Link của banner.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum Link {
    #[default]
    None,
    /// `https://`, `http://` hay đường dẫn `/…` trong website.
    Url(String),
    /// Đối tượng của sổ đối tượng.
    Ref { kind: String, id: String },
}

/// Mọi trường sửa được của banner (đã ghép bản cũ khi sửa).
#[derive(Debug, Clone, Default)]
pub(crate) struct BannerFields {
    pub locale: Option<String>,
    /// `image` (mặc định) hay `text`.
    pub kind: Option<String>,
    pub title: String,
    pub subtitle: String,
    pub cta_label: String,
    pub tone: Option<String>,
    pub media_id: Option<Uuid>,
    pub mobile_media_id: Option<Uuid>,
    pub alt: String,
    pub page_kinds: Vec<String>,
    pub page_targets: Vec<PageTarget>,
    pub link: Link,
    pub starts_at: Option<DateTime<Utc>>,
    pub ends_at: Option<DateTime<Utc>>,
    /// Không gửi: `draft`.
    pub status: Option<String>,
    pub ai_assisted: bool,
}

/// Banner mới.
#[derive(Debug, Clone, Default)]
pub(crate) struct NewBanner {
    /// Khoá vị trí.
    pub placement: String,
    /// Nút sở hữu; `None`: mọi cửa hàng của khách (nút gốc).
    pub org_id: Option<Uuid>,
    pub fields: BannerFields,
}

/// Sửa một phần: trường không gửi giữ nguyên; `Some(None)` xoá giá trị.
#[derive(Debug, Clone, Default)]
pub(crate) struct BannerPatch {
    pub kind: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub cta_label: Option<String>,
    pub tone: Option<Option<String>>,
    pub media_id: Option<Option<Uuid>>,
    pub mobile_media_id: Option<Option<Uuid>>,
    pub alt: Option<String>,
    pub page_kinds: Option<Vec<String>>,
    pub page_targets: Option<Vec<PageTarget>>,
    pub link: Option<Link>,
    pub starts_at: Option<Option<DateTime<Utc>>>,
    pub ends_at: Option<Option<DateTime<Utc>>>,
    pub status: Option<String>,
    pub ai_assisted: Option<bool>,
}

/// Bộ lọc danh sách.
#[derive(Debug, Clone, Default)]
pub(crate) struct BannerFilter {
    pub placement: Option<String>,
    pub org_id: Option<Uuid>,
    pub status: Option<String>,
    pub locale: Option<String>,
}

/// Ai gọi: người qua REST, hay agent qua tool MCP (chỉ tạo / sửa bản nháp — D-109).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Via {
    Rest,
    Tool,
}

impl ContentService {
    /// Một trang banner trong phạm vi của người gọi, theo ID. Đọc được bằng `content.banners` hay
    /// `content.read` (người đọc bản nháp xem được banner của nút mình).
    pub(crate) async fn list_banners(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: BannerFilter,
        page: &PageRequest,
    ) -> Result<Page<Banner>, Error> {
        let permission =
            if principal.has(&CONTENT_BANNERS) { &CONTENT_BANNERS } else { &CONTENT_READ };
        principal.require(permission)?;
        let mut problems = Vec::new();
        if let Some(status) = &filter.status {
            check_choice("status", status, &STATUSES, &mut problems);
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let scope = principal.scope_condition(permission, banner::Column::OrgId, None)?;
        let mut select = banner::Entity::find().filter(scope).scoped(tenant);
        if let Some(key) = &filter.placement {
            let found = self.placement_by_key(tenant, key).await?;
            select = select.filter(banner::Column::PlacementId.eq(found.id));
        }
        if let Some(org) = filter.org_id {
            select = select.filter(banner::Column::OrgId.eq(org));
        }
        if let Some(status) = filter.status {
            select = select.filter(banner::Column::Status.eq(status));
        }
        if let Some(locale) = filter.locale {
            select = select.filter(banner::Column::Locale.eq(locale));
        }
        fetch_page(select, banner::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Banner `id` kèm vị trí của nó. Ngoài phạm vi của người gọi: như không có.
    pub(crate) async fn get_banner(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(Banner, Placement), Error> {
        principal.require(&CONTENT_BANNERS)?;
        let found = self.find_banner(tenant.database().read(), tenant, id).await?;
        hidden(
            principal.require_at(tenant.database().read(), &CONTENT_BANNERS, target(&found)).await,
            id,
        )?;
        let placement = self.placement_of(tenant, &found).await?;
        Ok((found, placement))
    }

    pub(crate) async fn create_banner(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewBanner,
        via: Via,
    ) -> Result<(Banner, Placement), Error> {
        principal.require(&CONTENT_BANNERS)?;
        let placement = self.placement_by_key(tenant, input.placement.trim()).await?;
        let org = input.org_id.map_or(root_of(tenant.id()), OrgId::from_uuid);
        let mut fields = input.fields;
        if via == Via::Tool {
            fields.status = Some("draft".to_owned());
        }
        let at = now();
        let mut row = self.check_banner(tenant, &placement, fields, None).await?;
        row.id = Uuid::now_v7();
        row.tenant_id = tenant.id().as_uuid();
        row.placement_id = placement.id;
        row.org_id = org.as_uuid();
        row.created_by = Actor::from(principal).user_id;
        row.created_at = at;
        row.updated_at = at;
        let transaction = tenant.database().begin_write().await?;
        principal.require_write_at(&transaction, &CONTENT_BANNERS, target(&row)).await?;
        lock_placement(&transaction, tenant, placement.id).await?;
        if row.status != "archived" {
            ensure_room(&transaction, tenant, &placement, &row, None).await?;
        }
        row.position = next_position(&transaction, tenant, placement.id).await?;
        let saved = row.into_active_model().insert(&transaction).await?;
        self.record_banner(&transaction, tenant, principal, None, Some(&saved)).await?;
        transaction.commit().await?;
        tracing::info!(banner_id = %saved.id, placement_id = %placement.id, "banner created");
        Ok((saved, placement))
    }

    /// Sửa banner `id`. `expected`: phiên bản người gọi đã đọc (`If-Match`).
    pub(crate) async fn update_banner(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        patch: BannerPatch,
        expected: Option<DateTime<Utc>>,
        via: Via,
    ) -> Result<(Banner, Placement), Error> {
        let (found, placement) = self.get_banner(tenant, principal, id).await?;
        if via == Via::Tool
            && patch.status.as_deref().is_some_and(|status| status != found.status)
            && patch.status.as_deref() == Some("active")
        {
            return Err(Error::forbidden(
                "content.banner_activation_needs_person",
                "An AI tool only drafts banners; a person turns them on with `PATCH /api/v1/content/banners/<id>`.",
            ));
        }
        let fields = merge(&found, patch);
        let mut row = self.check_banner(tenant, &placement, fields, Some(&found)).await?;
        let at = now();
        row.id = found.id;
        row.tenant_id = found.tenant_id;
        row.placement_id = found.placement_id;
        row.org_id = found.org_id;
        row.position = found.position;
        row.created_by = found.created_by;
        row.created_at = found.created_at;
        row.updated_at = at;
        let transaction = tenant.database().begin_write().await?;
        lock_placement(&transaction, tenant, placement.id).await?;
        principal.require_write_at(&transaction, &CONTENT_BANNERS, target(&found)).await?;
        if row.status != "archived" && (found.status == "archived" || row.locale != found.locale) {
            ensure_room(&transaction, tenant, &placement, &row, Some(found.id)).await?;
        }
        // `reset_all`: mọi cột thành `Set` để câu UPDATE ghi hết; khoá chính giữ `Unchanged`.
        let mut active = row.clone().into_active_model().reset_all();
        active.id = sea_orm::ActiveValue::Unchanged(found.id);
        let updated = banner::Entity::update_many()
            .set(active)
            .scoped(tenant)
            .filter(banner::Column::Id.eq(found.id))
            .filter(banner::Column::UpdatedAt.eq(expected.unwrap_or(found.updated_at)))
            .exec(&transaction)
            .await?;
        if updated.rows_affected == 0 {
            return Err(autoxide_kernel::version::changed());
        }
        self.record_banner(&transaction, tenant, principal, Some(&found), Some(&row)).await?;
        transaction.commit().await?;
        Ok((row, placement))
    }

    pub(crate) async fn delete_banner(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        principal.require(&CONTENT_BANNERS)?;
        let transaction = tenant.database().begin_write().await?;
        let found = self.find_banner(&transaction, tenant, id).await?;
        hidden(principal.require_at(&transaction, &CONTENT_BANNERS, target(&found)).await, id)?;
        principal.require_write_at(&transaction, &CONTENT_BANNERS, target(&found)).await?;
        banner::Entity::delete_many()
            .scoped(tenant)
            .filter(banner::Column::Id.eq(id))
            .exec(&transaction)
            .await?;
        self.record_banner(&transaction, tenant, principal, Some(&found), None).await?;
        transaction.commit().await?;
        tracing::info!(banner_id = %id, "banner deleted");
        Ok(())
    }

    /// Đặt lại thứ tự banner của vị trí `key`: `ids` lên đầu theo thứ tự gửi, banner còn lại giữ
    /// thứ tự cũ phía sau. `expected`: phiên bản (`ETag`) của vị trí — hai lần đặt cùng lúc thì một
    /// bên nhận 412. Cần `content.banners` ở nút của mọi banner được nêu.
    pub(crate) async fn order_placement(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        key: &str,
        ids: Vec<Uuid>,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Placement, Error> {
        principal.require(&CONTENT_BANNERS)?;
        let found = self.placement_by_key(tenant, key).await?;
        let mut seen = HashSet::new();
        let mut problems = Vec::new();
        for (index, id) in ids.iter().enumerate() {
            if !seen.insert(*id) {
                problems.push(FieldError::new(
                    format!("banner_ids[{index}]"),
                    "duplicate",
                    "is listed twice",
                ));
            }
        }
        let at = now();
        let transaction = tenant.database().begin_write().await?;
        let rows = banner::Entity::find()
            .scoped(tenant)
            .filter(banner::Column::PlacementId.eq(found.id))
            .order_by_asc(banner::Column::Position)
            .order_by_asc(banner::Column::Id)
            .all(&transaction)
            .await?;
        let by_id: HashMap<Uuid, &Banner> = rows.iter().map(|row| (row.id, row)).collect();
        for (index, id) in ids.iter().enumerate() {
            match by_id.get(id) {
                Some(row) => {
                    if principal
                        .require_at(&transaction, &CONTENT_BANNERS, target(row))
                        .await
                        .is_err()
                    {
                        problems.push(FieldError::new(
                            format!("banner_ids[{index}]"),
                            "not_found",
                            "is not a banner of this placement",
                        ));
                    }
                }
                None => problems.push(FieldError::new(
                    format!("banner_ids[{index}]"),
                    "not_found",
                    "is not a banner of this placement",
                )),
            }
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        for org in
            ids.iter().filter_map(|id| by_id.get(id)).map(|row| row.org_id).collect::<HashSet<_>>()
        {
            let at_org = OrgTarget::new(OrgId::from_uuid(org), None);
            principal.require_write_at(&transaction, &CONTENT_BANNERS, at_org).await?;
        }
        self.touch_placement(&transaction, tenant, &found, expected, at).await?;
        let order: Vec<Uuid> = ids
            .iter()
            .copied()
            .chain(rows.iter().map(|row| row.id).filter(|id| !seen.contains(id)))
            .collect();
        for (position, id) in order.iter().enumerate() {
            let position = i32::try_from(position).unwrap_or(i32::MAX);
            if by_id.get(id).is_some_and(|row| row.position == position) {
                continue;
            }
            banner::Entity::update_many()
                .col_expr(banner::Column::Position, Expr::value(position))
                .scoped(tenant)
                .filter(banner::Column::Id.eq(*id))
                .exec(&transaction)
                .await?;
        }
        let event = events::BannerChanged(Change {
            before: None,
            after: Some(json!({"placement": found.key, "order": order})),
            published: None,
        });
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::subject("placement", found.id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        Ok(Placement { updated_at: at, ..found })
    }

    async fn find_banner<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<Banner, Error> {
        banner::Entity::find_by_id(id)
            .scoped(tenant)
            .one(connection)
            .await?
            .ok_or_else(|| banner_not_found(id))
    }

    async fn placement_of(&self, tenant: &TenantContext, row: &Banner) -> Result<Placement, Error> {
        placement::Entity::find_by_id(row.placement_id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| Error::internal("a banner points to a missing placement"))
    }

    async fn record_banner<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        principal: &Principal,
        before: Option<&Banner>,
        after: Option<&Banner>,
    ) -> Result<(), Error> {
        let id = after.or(before).map(|row| row.id).unwrap_or_default();
        let event = events::BannerChanged(Change {
            before: before.map(snapshot),
            after: after.map(snapshot),
            published: None,
        });
        self.outbox
            .record(
                connection,
                tenant,
                Actor::from(principal),
                events::subject("banner", id),
                &event,
            )
            .await
    }
}

/// Ghép `patch` vào bản đang có thành bộ trường đầy đủ.
fn merge(found: &Banner, patch: BannerPatch) -> BannerFields {
    let link = patch.link.unwrap_or_else(|| link_of(found));
    BannerFields {
        locale: Some(found.locale.clone()),
        kind: Some(patch.kind.unwrap_or_else(|| found.kind.clone())),
        title: patch.title.unwrap_or_else(|| found.title.clone()),
        subtitle: patch.subtitle.unwrap_or_else(|| found.subtitle.clone()),
        cta_label: patch.cta_label.unwrap_or_else(|| found.cta_label.clone()),
        tone: patch.tone.unwrap_or_else(|| found.tone.clone()),
        media_id: patch.media_id.unwrap_or(found.media_id),
        mobile_media_id: patch.mobile_media_id.unwrap_or(found.mobile_media_id),
        alt: patch.alt.unwrap_or_else(|| found.alt.clone()),
        page_kinds: patch.page_kinds.unwrap_or_else(|| strings(&found.page_kinds)),
        page_targets: patch.page_targets.unwrap_or_else(|| page_targets(&found.page_targets)),
        link,
        starts_at: patch.starts_at.unwrap_or(found.starts_at),
        ends_at: patch.ends_at.unwrap_or(found.ends_at),
        status: Some(patch.status.unwrap_or_else(|| found.status.clone())),
        ai_assisted: patch.ai_assisted.unwrap_or(found.ai_assisted),
    }
}

/// Link đã lưu của banner.
pub(crate) fn link_of(row: &Banner) -> Link {
    match (row.link_kind.as_str(), &row.link_url, &row.link_id) {
        ("url", Some(url), _) => Link::Url(url.clone()),
        ("none", _, _) | ("url", None, _) => Link::None,
        (kind, _, Some(id)) => Link::Ref { kind: kind.to_owned(), id: id.clone() },
        _ => Link::None,
    }
}

/// Trang cụ thể đã lưu (JSON); hỏng thì rỗng.
pub(crate) fn page_targets(raw: &str) -> Vec<PageTarget> {
    serde_json::from_str(raw).unwrap_or_default()
}

/// Vị trí còn chỗ cho banner `row` (đếm banner chưa lưu trữ cùng nút và ngôn ngữ, trừ `except`).
async fn ensure_room<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    placement: &Placement,
    row: &Banner,
    except: Option<Uuid>,
) -> Result<(), Error> {
    let mut select = banner::Entity::find()
        .scoped(tenant)
        .filter(banner::Column::PlacementId.eq(placement.id))
        .filter(banner::Column::OrgId.eq(row.org_id))
        .filter(banner::Column::Locale.eq(row.locale.clone()))
        .filter(banner::Column::Status.ne("archived"));
    if let Some(id) = except {
        select = select.filter(banner::Column::Id.ne(id));
    }
    let count = select.count(connection).await?;
    if count >= u64::try_from(placement.max_items).unwrap_or(0) {
        return Err(Error::conflict(
            "content.placement_full",
            "The placement already has {max} banners; archive one first.",
        )
        .with_param("max", placement.max_items));
    }
    Ok(())
}

/// Khoá dòng vị trí banner tới hết `transaction` ghi (D-181): đếm số banner (`max_items`) và lấy
/// `position` kế tiếp rồi mới ghi — hai lần tạo cùng lúc từng cùng đếm thiếu trên Postgres, vượt
/// `max_items` và trùng `position`.
async fn lock_placement(
    transaction: &sea_orm::DatabaseTransaction,
    tenant: &TenantContext,
    placement_id: Uuid,
) -> Result<(), Error> {
    autoxide_kernel::rows::lock_one(
        transaction,
        placement::Entity::find_by_id(placement_id).scoped(tenant),
    )
    .await?;
    Ok(())
}

async fn next_position<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    placement_id: Uuid,
) -> Result<i32, Error> {
    let last: Option<i32> = banner::Entity::find()
        .scoped(tenant)
        .select_only()
        .column_as(banner::Column::Position.max(), "position")
        .filter(banner::Column::PlacementId.eq(placement_id))
        .into_tuple::<Option<i32>>()
        .one(connection)
        .await?
        .flatten();
    Ok(last.map_or(0, |last| last.saturating_add(1)))
}

fn snapshot(row: &Banner) -> Value {
    json!({
        "placement_id": row.placement_id,
        "org_id": row.org_id,
        "locale": row.locale,
        "status": row.status,
        "link_kind": row.link_kind,
        "starts_at": row.starts_at,
        "ends_at": row.ends_at,
    })
}

/// Đích kiểm quyền của banner: nút của nó.
fn target(row: &Banner) -> OrgTarget {
    OrgTarget::new(OrgId::from_uuid(row.org_id), None)
}

/// Banner ngoài phạm vi đọc báo như không có.
fn hidden(result: Result<(), Error>, id: Uuid) -> Result<(), Error> {
    result.map_err(
        |error| if error.code() == "org_out_of_scope" { banner_not_found(id) } else { error },
    )
}
