//! Xoay vòng refresh token và phát hiện dùng lại (thu hồi cả họ phiên).

use super::*;

impl SessionService {
    /// Đổi refresh token lấy cặp token mới. Refresh token cũ hết dùng được. Khách lấy từ token
    /// (`request`: khách của request, nếu xác định được — xem [`token::resolve`]).
    pub(crate) async fn refresh(
        &self,
        request: Option<&TenantContext>,
        refresh_token: &str,
    ) -> Result<Tokens, Error> {
        let Some((tenant, raw)) = token::resolve(&self.databases, request, refresh_token).await?
        else {
            return Err(invalid_refresh_token());
        };
        self.refresh_in(&tenant, raw).await
    }

    pub(super) async fn refresh_in(
        &self,
        tenant: &TenantContext,
        refresh_token: &str,
    ) -> Result<Tokens, Error> {
        let now = now();
        // Đọc trên kết nối ghi: token vừa cấp có thể chưa kịp sang read replica.
        let row = Entity::find()
            .filter(Column::TokenHash.eq(token::hash_refresh_token(refresh_token)))
            .scoped(tenant)
            .one(tenant.database().write())
            .await?
            .ok_or_else(invalid_refresh_token)?;
        // Kiểm "đã dùng" TRƯỚC "đã thu hồi": token đã dùng luôn bị báo là dùng lại,
        // kể cả khi một request đồng thời khác vừa thu hồi cả họ — client nhận cùng
        // một mã cho cùng một tình huống.
        if row.used_at.is_some() {
            return Err(self.reuse_detected(tenant, &row).await);
        }
        if row.revoked_at.is_some() || row.expires_at <= now {
            return Err(invalid_refresh_token());
        }
        let user = self.users.get(tenant, row.user_id).await?.ok_or_else(invalid_refresh_token)?;
        if !user.is_active() {
            revoke_family(tenant.database().write(), tenant, row.family_id).await?;
            return Err(crate::user::user_disabled());
        }

        let expires_at = now + self.refresh_token_ttl;
        // Token mới mang theo thông tin của phiên.
        let started_at = row.session_started_at.unwrap_or(row.created_at);
        let authenticated_at = row.authenticated_at.unwrap_or(started_at);
        let session = SessionDetails {
            started_at,
            authenticated_at,
            user_agent: row.user_agent.clone(),
            sign_in_method: row.sign_in_method.clone(),
            country: row.country.clone(),
        };
        let (new_token, new_row) =
            new_refresh_token(tenant, user.id, row.family_id, now, expires_at, session)?;
        let transaction = tenant.database().begin_write().await?;
        // Khoá dòng người dùng TRƯỚC (Postgres khoá dòng này tới hết transaction): đổi mật
        // khẩu hay khoá tài khoản cũng ghi dòng này trước khi thu hồi phiên, nên hai việc
        // nối đuôi nhau — token mới không lọt qua lệnh thu hồi (D-079). Tài khoản vừa bị
        // khoá thì dừng ở đây.
        if !lock_active_user(&transaction, tenant, user.id).await? {
            transaction.rollback().await?;
            revoke_family(tenant.database().write(), tenant, row.family_id).await?;
            return Err(crate::user::user_disabled());
        }
        // Đánh dấu "đã dùng" CÓ ĐIỀU KIỆN: hai request cùng đổi một token thì chỉ một
        // bên cập nhật được dòng này (database khoá dòng), bên kia thấy 0 dòng.
        let claimed = Entity::update_many()
            .col_expr(Column::UsedAt, Expr::value(Some(now)))
            .filter(Column::Id.eq(row.id))
            .filter(Column::UsedAt.is_null())
            .filter(Column::RevokedAt.is_null())
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if claimed.rows_affected == 0 {
            transaction.rollback().await?;
            return Err(self.reuse_detected(tenant, &row).await);
        }
        new_row.insert(&transaction).await?;
        transaction.commit().await?;
        let session = Stamp { id: row.family_id, authenticated_at };
        self.tokens(tenant, user.id, session, &new_token, expires_at, now)
    }

    /// Thu hồi cả họ của `row` và trả lỗi báo token bị dùng lại.
    /// Phiên `session` của người dùng `user_id` vừa xác nhận lại danh tính (`POST /me/reauth`, bước
    /// 54 P5): ghi lần xác thực mới vào dòng refresh token còn dùng được của phiên (refresh sau đó
    /// giữ nó) và cấp ngay một access token mang `auth_time` mới.
    pub(crate) async fn reconfirm(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        session: Uuid,
    ) -> Result<String, Error> {
        let now = now();
        let updated = Entity::update_many()
            .col_expr(Column::AuthenticatedAt, Expr::value(Some(now)))
            .filter(Column::FamilyId.eq(session))
            .filter(Column::UserId.eq(user_id))
            .filter(Column::UsedAt.is_null())
            .filter(Column::RevokedAt.is_null())
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        if updated.rows_affected == 0 {
            // Phiên đã kết thúc (đăng xuất, thu hồi): không cấp token mới cho nó.
            return Err(crate::token::invalid_token());
        }
        self.access_tokens.issue(tenant.id(), user_id, now, session, now)
    }

    pub(super) async fn reuse_detected(
        &self,
        tenant: &TenantContext,
        row: &entity::Model,
    ) -> Error {
        tracing::warn!(
            user_id = %row.user_id,
            family_id = %row.family_id,
            "refresh token reused; revoking the whole session"
        );
        let ended = async {
            let transaction = tenant.database().begin_write().await?;
            revoke_family(&transaction, tenant, row.family_id).await?;
            let event = events::RefreshTokenReused::default();
            let subject = events::user_subject(row.user_id);
            self.outbox
                .record(&transaction, tenant, Actor::user(row.user_id), subject, &event)
                .await?;
            transaction.commit().await?;
            Ok::<(), Error>(())
        }
        .await;
        if let Err(error) = ended {
            return error;
        }
        Error::unauthenticated(
            "auth.refresh_token_reused",
            "This refresh token was already used. The session has been ended for safety; sign \
             in again. Use each refresh token once and keep only the newest one.",
        )
    }
}

pub(super) fn invalid_refresh_token() -> Error {
    Error::unauthenticated("auth.invalid_refresh_token", "Your session has ended. Sign in again.")
}
