//! Vị trí hiển thị (bước 55, D-236): cấu hình chung của khách, cần `content.manage`. Phiên bản là
//! `updated_at` (ETag / `If-Match`).

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::time::Timestamp;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter,
    QueryOrder, SqlErr,
};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::{
    ACCEPTS, DEVICES, DISPLAYS, MAX_ITEMS, check_choice, check_display_settings, check_page_kinds,
    invalid, is_key, placement_not_found,
};
use crate::entity::{banner, placement};
use crate::events::{self, Change};
use crate::permissions::{CONTENT_BANNERS, CONTENT_MANAGE};
use crate::service::{ContentService, check_length, now};

pub(crate) type Placement = placement::Model;

/// Dữ liệu của một vị trí (tạo hay thay toàn bộ).
#[derive(Debug, Clone, Default)]
pub(crate) struct PlacementInput {
    pub key: String,
    pub name: String,
    pub description: String,
    /// Không gửi: 10.
    pub max_items: Option<i32>,
    pub display: String,
    pub display_settings: Map<String, Value>,
    /// Không gửi: `text` với `strip`, còn lại `image`.
    pub accepts: Option<String>,
    pub ratio: Option<(i32, i32)>,
    pub mobile_ratio: Option<(i32, i32)>,
    pub min_width: Option<i32>,
    pub min_height: Option<i32>,
    /// Không gửi: `all`.
    pub devices: Option<String>,
    /// Không gửi: `any`.
    pub page_kinds: Vec<String>,
    /// Không gửi: bật.
    pub active: Option<bool>,
}

impl ContentService {
    /// Mọi vị trí của khách, theo khoá. Ai quản banner (`content.banners`) cũng đọc được: họ cần
    /// biết vị trí nhận gì.
    pub(crate) async fn list_placements(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
    ) -> Result<Vec<Placement>, Error> {
        require_read(principal)?;
        Ok(placement::Entity::find()
            .scoped(tenant)
            .order_by_asc(placement::Column::Key)
            .all(tenant.database().read())
            .await?)
    }

    pub(crate) async fn get_placement(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        key: &str,
    ) -> Result<Placement, Error> {
        require_read(principal)?;
        self.placement_by_key(tenant, key).await
    }

    /// ID → khoá của mọi vị trí của khách (vài chục dòng: đọc một lần cho cả trang banner).
    pub(crate) async fn placement_keys(
        &self,
        tenant: &TenantContext,
    ) -> Result<std::collections::HashMap<Uuid, String>, Error> {
        Ok(placement::Entity::find()
            .scoped(tenant)
            .all(tenant.database().read())
            .await?
            .into_iter()
            .map(|row| (row.id, row.key))
            .collect())
    }

    pub(crate) async fn placement_by_key(
        &self,
        tenant: &TenantContext,
        key: &str,
    ) -> Result<Placement, Error> {
        placement::Entity::find()
            .scoped(tenant)
            .filter(placement::Column::Key.eq(key))
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| placement_not_found(key))
    }

    /// Tạo vị trí. Khoá đã có → 409 `content.placement_exists`.
    pub(crate) async fn create_placement(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: PlacementInput,
    ) -> Result<Placement, Error> {
        principal.require(&CONTENT_MANAGE)?;
        let at = now();
        let row = checked(tenant, input, Uuid::now_v7(), at, None)?;
        let transaction = tenant.database().begin_write().await?;
        let saved = row.into_active_model().insert(&transaction).await.map_err(|error| {
            if matches!(error.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))) {
                placement_exists()
            } else {
                error.into()
            }
        })?;
        self.record_placement(&transaction, tenant, principal, None, Some(&saved)).await?;
        transaction.commit().await?;
        tracing::info!(placement_id = %saved.id, "display placement created");
        Ok(saved)
    }

    /// Thay toàn bộ vị trí `key` (khoá giữ nguyên). `expected`: phiên bản người gọi đã đọc.
    pub(crate) async fn replace_placement(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        key: &str,
        input: PlacementInput,
        expected: Option<Timestamp>,
    ) -> Result<Placement, Error> {
        principal.require(&CONTENT_MANAGE)?;
        let found = self.placement_by_key(tenant, key).await?;
        let at = now();
        let input = PlacementInput { key: found.key.clone(), ..input };
        let row = checked(tenant, input, found.id, at, Some(&found))?;
        let transaction = tenant.database().begin_write().await?;
        // `reset_all`: mọi cột thành `Set` để câu UPDATE ghi hết; khoá chính giữ `Unchanged`.
        let mut active = row.clone().into_active_model().reset_all();
        active.id = sea_orm::ActiveValue::Unchanged(found.id);
        active.tenant_id = sea_orm::ActiveValue::Unchanged(found.tenant_id);
        active.created_at = sea_orm::ActiveValue::Unchanged(found.created_at);
        // Kiểm phiên bản trong chính câu UPDATE: hai người sửa cùng lúc thì một người nhận 412.
        let updated = placement::Entity::update_many()
            .set(active)
            .scoped(tenant)
            .filter(placement::Column::Id.eq(found.id))
            .filter(placement::Column::UpdatedAt.eq(expected.unwrap_or(found.updated_at)))
            .exec(&transaction)
            .await?;
        if updated.rows_affected == 0 {
            return Err(autoxide_kernel::version::changed());
        }
        self.record_placement(&transaction, tenant, principal, Some(&found), Some(&row)).await?;
        transaction.commit().await?;
        Ok(row)
    }

    /// Xoá vị trí `key`. Còn banner (kể cả đã lưu trữ) → 409 `content.placement_in_use`.
    pub(crate) async fn delete_placement(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        key: &str,
    ) -> Result<(), Error> {
        principal.require(&CONTENT_MANAGE)?;
        let found = self.placement_by_key(tenant, key).await?;
        let transaction = tenant.database().begin_write().await?;
        let banners = banner::Entity::find()
            .scoped(tenant)
            .filter(banner::Column::PlacementId.eq(found.id))
            .count(&transaction)
            .await?;
        if banners > 0 {
            return Err(Error::conflict(
                "content.placement_in_use",
                "The placement still has {count} banners; move or delete them first.",
            )
            .with_param("count", banners));
        }
        placement::Entity::delete_many()
            .scoped(tenant)
            .filter(placement::Column::Id.eq(found.id))
            .exec(&transaction)
            .await?;
        self.record_placement(&transaction, tenant, principal, Some(&found), None).await?;
        transaction.commit().await?;
        tracing::info!(placement_id = %found.id, "display placement deleted");
        Ok(())
    }

    /// Đánh dấu vị trí đã đổi (đặt lại thứ tự banner): `updated_at` mới nếu còn đúng `expected`.
    pub(crate) async fn touch_placement<C: sea_orm::ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        found: &Placement,
        expected: Option<Timestamp>,
        at: Timestamp,
    ) -> Result<(), Error> {
        let updated = placement::Entity::update_many()
            .col_expr(placement::Column::UpdatedAt, Expr::value(at))
            .scoped(tenant)
            .filter(placement::Column::Id.eq(found.id))
            .filter(placement::Column::UpdatedAt.eq(expected.unwrap_or(found.updated_at)))
            .exec(connection)
            .await?;
        if updated.rows_affected == 0 {
            return Err(autoxide_kernel::version::changed());
        }
        Ok(())
    }

    async fn record_placement<C: sea_orm::ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        principal: &Principal,
        before: Option<&Placement>,
        after: Option<&Placement>,
    ) -> Result<(), Error> {
        let id = after.or(before).map(|row| row.id).unwrap_or_default();
        let event = events::PlacementChanged(Change {
            before: before.map(snapshot),
            after: after.map(snapshot),
            published: None,
        });
        self.outbox
            .record(
                connection,
                tenant,
                Actor::from(principal),
                events::subject("placement", id),
                &event,
            )
            .await
    }
}

/// Đọc vị trí: `content.manage` hay `content.banners`.
fn require_read(principal: &Principal) -> Result<(), Error> {
    if principal.has(&CONTENT_BANNERS) {
        return Ok(());
    }
    principal.require(&CONTENT_MANAGE)
}

fn snapshot(row: &Placement) -> Value {
    json!({"key": row.key, "display": row.display, "active": row.active})
}

fn placement_exists() -> Error {
    Error::conflict(
        "content.placement_exists",
        "A placement with this key already exists; replace it with `PUT` instead.",
    )
}

/// Kiểm `input` và dựng dòng (gom mọi lỗi, D-083).
pub(crate) fn checked(
    tenant: &TenantContext,
    input: PlacementInput,
    id: Uuid,
    at: Timestamp,
    before: Option<&Placement>,
) -> Result<Placement, Error> {
    let mut problems = Vec::new();
    let key = input.key.trim().to_owned();
    if !is_key(&key) {
        problems.push(FieldError::new(
            "key",
            "invalid",
            "must be 1 to 64 lower-case letters, digits or `_`",
        ));
    }
    let name = input.name.trim().to_owned();
    if name.is_empty() {
        problems.push(FieldError::new("name", "required", "must not be empty"));
    }
    check_length("name", Some(&name), 120, &mut problems);
    check_length("description", Some(&input.description), 500, &mut problems);
    let max_items = input.max_items.unwrap_or(10);
    if !(1..=MAX_ITEMS).contains(&max_items) {
        problems.push(
            FieldError::new("max_items", "range", "must be 1 to {max}")
                .with_param("max", MAX_ITEMS),
        );
    }
    let display = input.display.trim().to_owned();
    let settings = if check_choice("display", &display, &DISPLAYS, &mut problems) {
        check_display_settings(&display, &input.display_settings, &mut problems)
    } else {
        Map::new()
    };
    let accepts = input
        .accepts
        .unwrap_or_else(|| if display == "strip" { "text" } else { "image" }.to_owned());
    check_choice("accepts", &accepts, &ACCEPTS, &mut problems);
    if display == "strip" && accepts != "text" {
        problems.push(FieldError::new("accepts", "invalid", "a `strip` only takes `text` banners"));
    }
    for (field, pair) in [("ratio", input.ratio), ("mobile_ratio", input.mobile_ratio)] {
        if let Some((w, h)) = pair
            && !((1..=100).contains(&w) && (1..=100).contains(&h))
        {
            problems.push(FieldError::new(field, "range", "both sides must be 1 to 100"));
        }
    }
    for (field, value) in [("min_width", input.min_width), ("min_height", input.min_height)] {
        if value.is_some_and(|value| !(1..=10_000).contains(&value)) {
            problems.push(FieldError::new(field, "range", "must be 1 to 10000 pixels"));
        }
    }
    let devices = input.devices.unwrap_or_else(|| "all".to_owned());
    check_choice("devices", &devices, &DEVICES, &mut problems);
    let page_kinds = if input.page_kinds.is_empty() {
        vec!["any".to_owned()]
    } else {
        check_page_kinds("page_kinds", &input.page_kinds, false, &mut problems)
    };
    if !problems.is_empty() {
        return Err(invalid(problems));
    }
    Ok(Placement {
        id,
        tenant_id: tenant.id().as_uuid(),
        key,
        name,
        description: input.description.trim().to_owned(),
        max_items,
        audience: before.map_or_else(|| "site".to_owned(), |row| row.audience.clone()),
        display,
        display_settings: Value::Object(settings).to_string(),
        accepts,
        ratio_w: input.ratio.map(|ratio| ratio.0),
        ratio_h: input.ratio.map(|ratio| ratio.1),
        mobile_ratio_w: input.mobile_ratio.map(|ratio| ratio.0),
        mobile_ratio_h: input.mobile_ratio.map(|ratio| ratio.1),
        min_width: input.min_width,
        min_height: input.min_height,
        devices,
        page_kinds: serde_json::to_string(&page_kinds).unwrap_or_else(|_| "[]".to_owned()),
        preset_key: before.and_then(|row| row.preset_key.clone()),
        active: input.active.unwrap_or(true),
        created_at: before.map_or(at, |row| row.created_at),
        updated_at: at,
    })
}
