//! Tài khoản người dùng: bảng `auth_user` và service làm việc với nó.

pub(crate) mod entity;
mod service;

pub(crate) use service::NewUser;
pub(crate) use service::{
    PasswordWrite, UserChanges, UserService, check_email_field, email_taken, invalid_credentials,
    user_disabled, user_not_found,
};

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
