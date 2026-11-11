//! Tài khoản người dùng: bảng `auth_user` và service làm việc với nó.

mod entity;
mod service;

#[cfg(test)]
pub(crate) use service::NewUser;
pub(crate) use service::{UserService, user_disabled};

#[cfg(test)]
pub(crate) mod entity_for_tests {
    //! Thao tác thẳng trên bảng cho test của phần khác trong module (chưa có API
    //! khoá tài khoản ở bước này).

    use sea_orm::sea_query::Expr;
    use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
    use uuid::Uuid;

    use super::entity::{Column, Entity};

    /// Khoá tài khoản `id`.
    pub async fn disable<C: ConnectionTrait>(connection: &C, id: Uuid) {
        Entity::update_many()
            .col_expr(Column::DisabledAt, Expr::value(Some(chrono::Utc::now())))
            .filter(Column::Id.eq(id))
            .exec(connection)
            .await
            .unwrap();
    }
}
