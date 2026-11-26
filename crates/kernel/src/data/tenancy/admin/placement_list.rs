//! `GET /api/v1/kernel/placements`: one page of the placements (step 70 P9), split from `admin.rs` by
//! responsibility.

use std::sync::Arc;

use axum::extract::{Query, State};
use sea_orm::EntityTrait;

use super::{
    PLACEMENTS_READ, Placement, TenantAdmin, describe_placement, placement, require_platform,
};
use crate::access::Principal;
use crate::error::Error;
use crate::pagination::{NaturalOrderPageQuery, Page, PageRequest, SortValue, fetch_page_by};
use crate::response::Problem;

impl TenantAdmin {
    /// One page of the placements, by name (unique), keyset in the database on the unique index of the name.
    pub(crate) async fn placements_page(
        &self,
        page: &PageRequest,
    ) -> Result<Page<Placement>, Error> {
        let rows = fetch_page_by(
            placement::Entity::find(),
            placement::Column::Name,
            |row| SortValue::Text(row.name.clone()),
            placement::Column::Id,
            |row| row.id,
            page,
            self.control().read(),
        )
        .await?;
        let mut data = Vec::with_capacity(rows.data.len());
        for row in rows.data {
            data.push(describe_placement(&self.registry, row).await);
        }
        Ok(Page { data, pagination: rows.pagination })
    }
}

/// List the placements.
///
/// Requires `kernel.placements.read` and an account of the platform tenant. Where tenants' data
/// is kept; connection URLs are never shown (placements are added with `autoxide placement add`).
/// By name (`order=desc` reverses it); paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/placements",
    extensions(("x-permissions" = json!(["kernel.placements.read"]))),
    operation_id = "list_placements",
    security(("bearer" = [])),
    params(NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of placements, by name.", body = Page<Placement>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.placements.read` (`permission_denied`), or not an account of the platform tenant (`tenant_platform_only`).", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(super) async fn list_placements(
    State(admin): State<Arc<TenantAdmin>>,
    principal: Principal,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<Placement>, Error> {
    principal.require(&PLACEMENTS_READ)?;
    require_platform(&principal)?;
    let page = PageRequest::try_from(page)?;
    admin.placements_page(&page).await
}
