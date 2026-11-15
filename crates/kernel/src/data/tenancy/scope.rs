//! Lọc theo khách cho SeaORM (đặc tả 33, D-253): entity cấp khách hàng cài [`TenantScoped`] (một
//! dòng), service viết `.scoped(&tenant)` ở mọi truy vấn của nó.
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
//! `INSERT` không có bộ lọc: service đặt `tenant_id` khi dựng `ActiveModel` (cột NOT NULL không
//! `DEFAULT` — quên là lỗi ngay, D-253 mục 5).

use sea_orm::sea_query::IntoCondition;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DeleteMany, EntityTrait, QueryFilter, Select, SelectTwo,
    SelectTwoMany, UpdateMany, ValidatedDeleteOne, ValidatedUpdateOne,
};

use super::TenantContext;

/// Entity cấp khách hàng: có cột `tenant_id`.
pub trait TenantScoped: EntityTrait {
    /// Cột `tenant_id` của entity.
    fn tenant_column() -> Self::Column;
}

/// Thêm `WHERE <bảng>.tenant_id = <khách>` vào một truy vấn của entity cấp khách hàng.
pub trait ScopeExt: Sized {
    /// Chỉ các dòng của khách `tenant`.
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
