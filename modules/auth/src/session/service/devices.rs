//! Phiên đang mở của người dùng: liệt kê, thu hồi một hay mọi phiên, đăng xuất, ghi quốc gia.

use super::*;

impl SessionService {
    /// Đăng xuất: thu hồi họ của `refresh_token`. Token lạ hay đã thu hồi cũng coi
    /// như xong — gọi lại nhiều lần vẫn cùng kết quả. Khách lấy từ token như [`refresh`].
    ///
    /// [`refresh`]: Self::refresh
    pub(crate) async fn logout(
        &self,
        request: Option<&TenantContext>,
        refresh_token: &str,
    ) -> Result<(), Error> {
        let Some((tenant, refresh_token)) =
            token::resolve(&self.databases, request, refresh_token).await?
        else {
            return Ok(());
        };
        let tenant = &tenant;
        let row = Entity::find()
            .filter(Column::TokenHash.eq(token::hash_refresh_token(refresh_token)))
            .scoped(tenant)
            .one(tenant.database().write())
            .await?;
        if let Some(row) = row {
            let transaction = tenant.database().begin_write().await?;
            revoke_family(&transaction, tenant, row.family_id).await?;
            let event = events::SignedOut::default();
            let subject = events::user_subject(row.user_id);
            self.outbox
                .record(&transaction, tenant, Actor::user(row.user_id), subject, &event)
                .await?;
            transaction.commit().await?;
            tracing::info!(user_id = %row.user_id, "signed out");
        }
        Ok(())
    }

    /// Thu hồi mọi refresh token của người dùng (vd sau khi đổi mật khẩu). Chạy
    /// trong `connection` để đi cùng transaction của bên gọi.
    /// Bỏ mọi lượt chờ mã MFA của `user_id` trong `connection`: đổi mật khẩu thì lượt mở bằng
    /// mật khẩu cũ hết dùng được.
    pub(crate) async fn drop_mfa_challenges<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<(), Error> {
        challenge::drop_for_user(connection, tenant, user_id).await
    }

    pub(crate) async fn revoke_user<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<(), Error> {
        // Chỉ token chưa dùng: token đã đổi lấy token mới thì không dùng lại được nữa, và bảng
        // giữ hàng nghìn dòng như vậy mỗi phiên (một dòng mỗi lần refresh). Gửi lại token đã dùng
        // vẫn bị phát hiện (dòng còn đó với `used_at`) và thu hồi cả họ.
        Entity::update_many()
            .col_expr(Column::RevokedAt, Expr::value(Some(now())))
            .filter(Column::UserId.eq(user_id))
            .filter(Column::UsedAt.is_null())
            .filter(Column::RevokedAt.is_null())
            .scoped(tenant)
            .exec(connection)
            .await?;
        Ok(())
    }

    /// Một trang phiên còn hiệu lực của người dùng `user_id`, mới bắt đầu trước (mặc
    /// định). Mỗi phiên là refresh token còn dùng được của một họ.
    /// Ghi quốc gia gần đúng `country` (đã kiểm: hai chữ hoa ISO 3166-1) cho phiên `session_id`
    /// vừa mở (D-272). Chỉ ghi khi phiên chưa có: phiên không đổi quốc gia giữa chừng. Thông tin
    /// để người dùng nhận ra phiên, không dùng để quyết định gì — lỗi ghi thì bỏ qua ở bên gọi.
    pub(crate) async fn note_country(
        &self,
        tenant: &TenantContext,
        session_id: Uuid,
        country: &str,
    ) -> Result<(), Error> {
        entity::Entity::update_many()
            .col_expr(entity::Column::Country, Expr::value(Some(country.to_owned())))
            .filter(entity::Column::FamilyId.eq(session_id))
            .filter(entity::Column::Country.is_null())
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        Ok(())
    }

    pub(crate) async fn list_sessions(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<Session>, Error> {
        let select = live_tokens(tenant, user_id, now());
        // Họ token sinh bằng UUIDv7 lúc đăng nhập: xếp theo `family_id` là xếp theo lúc
        // phiên bắt đầu, và mỗi họ chỉ có một token còn sống — cursor ổn định.
        let read = tenant.database().read();
        let page = fetch_page(select, Column::FamilyId, |row| row.family_id, page, read).await?;
        Ok(page.map(Session::from))
    }

    /// Kết thúc phiên `session_id` của người dùng `user_id` (đăng xuất một thiết bị).
    /// Access token đã cấp cho phiên đó vẫn chạy tới khi hết hạn (mặc định 15 phút).
    pub(crate) async fn revoke_session(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        session_id: Uuid,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        // Khoá dòng người dùng trước, như refresh: refresh đang chạy dở xong hẳn rồi mới
        // tới lượt thu hồi, nên token nó vừa cấp cũng bị thu hồi.
        lock_user(&transaction, tenant, user_id).await?;
        let revoked = Entity::update_many()
            .col_expr(Column::RevokedAt, Expr::value(Some(now())))
            .filter(Column::UserId.eq(user_id))
            .filter(Column::FamilyId.eq(session_id))
            .filter(Column::RevokedAt.is_null())
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if revoked.rows_affected == 0 {
            transaction.rollback().await?;
            return Err(session_not_found(session_id));
        }
        let event =
            events::SignedOut { before: None, after: Some(json!({"session_id": session_id})) };
        let subject = events::user_subject(user_id);
        self.outbox.record(&transaction, tenant, Actor::user(user_id), subject, &event).await?;
        transaction.commit().await?;
        tracing::info!(user_id = %user_id, "session ended");
        Ok(())
    }

    /// Kết thúc mọi phiên của người dùng `user_id`, trừ phiên `keep` (nếu có). Trả số
    /// phiên đã kết thúc. Dùng cho "đăng xuất mọi thiết bị khác" và cho quản trị viên.
    pub(crate) async fn revoke_sessions(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        user_id: Uuid,
        keep: Option<Uuid>,
    ) -> Result<u64, Error> {
        let now = now();
        let transaction = tenant.database().begin_write().await?;
        lock_user(&transaction, tenant, user_id).await?;
        let mut live = live_tokens(tenant, user_id, now);
        let mut revoke = Entity::update_many()
            .col_expr(Column::RevokedAt, Expr::value(Some(now)))
            .filter(Column::UserId.eq(user_id))
            .filter(Column::RevokedAt.is_null())
            .scoped(tenant);
        if let Some(keep) = keep {
            live = live.filter(Column::FamilyId.ne(keep));
            revoke = revoke.filter(Column::FamilyId.ne(keep));
        }
        let ended = live.count(&transaction).await?;
        revoke.exec(&transaction).await?;
        if ended > 0 {
            let event = events::SessionsRevoked {
                before: None,
                after: Some(json!({"count": ended, "kept": keep})),
            };
            let subject = events::user_subject(user_id);
            self.outbox.record(&transaction, tenant, actor, subject, &event).await?;
        }
        transaction.commit().await?;
        tracing::info!(user_id = %user_id, sessions = ended, "sessions ended");
        Ok(ended)
    }
}

pub(super) fn session_not_found(id: Uuid) -> Error {
    Error::not_found("auth.session_not_found", "You have no active session `{id}`.")
        .with_param("id", id)
}

/// Thu hồi mọi refresh token của họ `family_id`.
pub(super) async fn revoke_family<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    family_id: Uuid,
) -> Result<(), Error> {
    Entity::update_many()
        .col_expr(Column::RevokedAt, Expr::value(Some(now())))
        .filter(Column::FamilyId.eq(family_id))
        .filter(Column::RevokedAt.is_null())
        .scoped(tenant)
        .exec(connection)
        .await?;
    Ok(())
}
