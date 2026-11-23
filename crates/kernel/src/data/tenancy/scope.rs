//! Tenant filtering for SeaORM (spec 33, D-253): a tenant-level entity implements [`TenantScoped`] (one
//! line), and a service writes `.scoped(&tenant)` in every one of its queries.
//!
//! ```ignore
//! impl TenantScoped for Entity {
//!     fn tenant_column() -> Self::Column {
//!         Column::TenantId
//!     }
//! }
//!
//! let note = Entity::find_by_id(id).scoped(&tenant).one(tenant.database().read()).await?;
//! Entity::delete_many().filter(Column::OwnerId.eq(owner)).scoped(&tenant).exec(write).await?;
//! ```
//!
//! `INSERT` has no filter: the service sets `tenant_id` when building the `ActiveModel` (a NOT NULL column
//! with no `DEFAULT`, so forgetting it is an immediate error, D-253 item 5).

use sea_orm::sea_query::IntoCondition;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DeleteMany, EntityTrait, QueryFilter, Select, SelectTwo,
    SelectTwoMany, UpdateMany, ValidatedDeleteOne, ValidatedUpdateOne,
};

use super::TenantContext;

/// A tenant-level entity: has a `tenant_id` column.
pub trait TenantScoped: EntityTrait {
    /// The entity's `tenant_id` column.
    fn tenant_column() -> Self::Column;
}

/// Adds `WHERE <table>.tenant_id = <tenant>` to a query of a tenant-level entity.
pub trait ScopeExt: Sized {
    /// Only the rows of tenant `tenant`.
    #[must_use]
    fn scoped(self, tenant: &TenantContext) -> Self;
}

fn condition<E: TenantScoped>(tenant: &TenantContext) -> impl IntoCondition {
    E::tenant_column().eq(tenant.id().as_uuid())
}

impl<E: TenantScoped> ScopeExt for Select<E> {
    fn scoped(self, tenant: &TenantContext) -> Self {
        self.filter(condition::<E>(tenant))
    }
}

// Truy vấn kèm entity thứ hai: lọc theo khách của entity chính; entity đi kèm (cùng module, nối
// theo khoá ngoại trong module) thuộc cùng khách.
impl<E: TenantScoped, F: EntityTrait> ScopeExt for SelectTwo<E, F> {
    fn scoped(self, tenant: &TenantContext) -> Self {
        self.filter(condition::<E>(tenant))
    }
}

impl<E: TenantScoped, F: EntityTrait> ScopeExt for SelectTwoMany<E, F> {
    fn scoped(self, tenant: &TenantContext) -> Self {
        self.filter(condition::<E>(tenant))
    }
}

impl<E: TenantScoped> ScopeExt for UpdateMany<E> {
    fn scoped(self, tenant: &TenantContext) -> Self {
        self.filter(condition::<E>(tenant))
    }
}

impl<E: TenantScoped> ScopeExt for DeleteMany<E> {
    fn scoped(self, tenant: &TenantContext) -> Self {
        self.filter(condition::<E>(tenant))
    }
}

// `Entity::update(model)` / `Entity::delete(model)`: `.validate()?` rồi `.scoped(&tenant)`.
impl<A> ScopeExt for ValidatedUpdateOne<A>
where
    A: ActiveModelTrait,
    A::Entity: TenantScoped,
{
    fn scoped(self, tenant: &TenantContext) -> Self {
        self.filter(condition::<A::Entity>(tenant))
    }
}

impl<E: TenantScoped> ScopeExt for ValidatedDeleteOne<E> {
    fn scoped(self, tenant: &TenantContext) -> Self {
        self.filter(condition::<E>(tenant))
    }
}
