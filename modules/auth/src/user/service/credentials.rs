//! Mật khẩu: kiểm khi đăng nhập (hash lại hash cũ), đặt, khôi phục, mật khẩu đầu tiên.

use super::*;

impl UserService {
    /// Kiểm email và mật khẩu; đúng thì trả người dùng.
    ///
    /// Email không tồn tại và mật khẩu sai trả CÙNG một lỗi, sau CÙNG một khoảng
    /// thời gian: không ai dò được email nào đã đăng ký. Hash lưu theo tham số cũ
    /// được hash lại theo tham số hiện hành.
    #[cfg(test)]
    pub(crate) async fn check_credentials(
        &self,
        tenant: &TenantContext,
        email: &str,
        password: &str,
    ) -> Result<User, Error> {
        self.verify_credentials(tenant, email, password).await.map(|(user, _)| user)
    }

    /// Như [`check_credentials`](Self::check_credentials), kèm hash mật khẩu đang lưu lúc
    /// kiểm (hash mới nếu vừa hash lại). Bên cấp phiên / API key đưa hash này vào
    /// [`SessionService::lock_grant`]: mật khẩu đổi giữa lúc kiểm và lúc cấp thì không cấp.
    #[cfg(test)]
    pub(crate) async fn verify_credentials(
        &self,
        tenant: &TenantContext,
        email: &str,
        password: &str,
    ) -> Result<(User, String), Error> {
        self.verify_credentials_in(tenant, Realm::Staff, email, password).await
    }

    /// Như [`verify_credentials`](Self::verify_credentials), cho tài khoản `realm` (D-309).
    pub(crate) async fn verify_credentials_in(
        &self,
        tenant: &TenantContext,
        realm: Realm,
        email: &str,
        password: &str,
    ) -> Result<(User, String), Error> {
        if password.chars().count() > password::MAX_LENGTH {
            return Err(invalid_credentials());
        }
        let row = Entity::find()
            .filter(Column::Realm.eq(entity::realm_str(realm)))
            .filter(Column::Email.eq(autoxide_kernel::email::normalize(email)))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        let stored = row.as_ref().and_then(|row| row.password_hash.clone());
        let verification = self.passwords.verify(password.to_owned(), stored.clone()).await?;

        let (row, rehash) = match (row, verification) {
            (Some(row), Verification::Valid { rehash }) => (row, rehash),
            _ => return Err(invalid_credentials()),
        };
        if row.disabled_at.is_some() {
            return Err(user_disabled());
        }
        let Some(mut current) = stored else { return Err(invalid_credentials()) };
        if rehash
            && let Some(upgraded) = self.upgrade_hash(tenant, row.id, &current, password).await
        {
            current = upgraded;
        }
        Ok((row.into(), current))
    }

    /// Hash lại mật khẩu theo tham số hiện hành. Hỏng thì chỉ ghi log: người dùng
    /// đã đăng nhập đúng, lần sau thử lại.
    /// Trả hash mới khi đã thay; `None` khi giữ hash cũ.
    pub(super) async fn upgrade_hash(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        stored: &str,
        password: &str,
    ) -> Option<String> {
        let result = async {
            let hash = self.passwords.hash(password.to_owned()).await?;
            // Chỉ thay nếu hash vẫn là hash vừa kiểm: không đè mật khẩu vừa được
            // đổi bởi một request khác.
            let updated = Entity::update_many()
                .col_expr(Column::PasswordHash, Expr::value(Some(hash.clone())))
                .filter(Column::Id.eq(id))
                .filter(Column::PasswordHash.eq(stored))
                .scoped(tenant)
                .exec(tenant.database().write())
                .await?;
            Ok::<_, Error>((updated.rows_affected == 1).then_some(hash))
        }
        .await;
        match result {
            Ok(upgraded) => upgraded,
            Err(error) => {
                tracing::warn!(user_id = %id, error = %error, "cannot upgrade the password hash");
                None
            }
        }
    }

    /// Đặt mật khẩu mới cho người dùng `id`.
    #[cfg(test)]
    pub(crate) async fn set_password(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
        password: String,
    ) -> Result<(), Error> {
        self.write_password(tenant, actor, id, "password", password, PasswordWrite::Change).await
    }

    /// Chủ tài khoản lấy lại tài khoản bằng mail đặt lại mật khẩu: đặt mật khẩu mới, và trong
    /// CÙNG transaction xoá mọi passkey, API key, token đặt lại khác — kẻ chiếm phiên không
    /// giữ lại được cửa nào (D-079, D-156).
    pub(crate) async fn recover_password(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
        password: String,
    ) -> Result<(), Error> {
        self.write_password(tenant, actor, id, "password", password, PasswordWrite::Recovery).await
    }

    /// Quản trị viên đặt mật khẩu cho tài khoản tạo không kèm mật khẩu. Tài khoản đã
    /// có mật khẩu thì từ chối (`auth.password_already_set`): người giữ quyền quản lý
    /// người dùng không được chiếm tài khoản của người khác bằng cách đổi mật khẩu của họ.
    pub(crate) async fn set_initial_password(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
        password: String,
    ) -> Result<(), Error> {
        self.write_password(tenant, actor, id, "password", password, PasswordWrite::Initial).await
    }

    /// Hash và lưu mật khẩu. `field`: tên trường báo trong lỗi luật mật khẩu.
    /// [`PasswordWrite::Initial`]: chỉ ghi khi tài khoản chưa có mật khẩu — kiểm ngay trong câu
    /// `UPDATE` nên hai request đồng thời không cùng ghi được.
    ///
    /// Cùng transaction với việc đổi mật khẩu, mọi thứ đã mở bằng mật khẩu cũ hết dùng được
    /// (D-156): phiên đăng nhập, lượt chờ mã MFA, nghi thức thêm passkey đang dở, token đổi
    /// email đang chờ.
    pub(crate) async fn write_password(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
        field: &str,
        password: String,
        mode: PasswordWrite,
    ) -> Result<(), Error> {
        let only_if_unset = mode == PasswordWrite::Initial;
        if let Some(problem) = password::check_rules(field, &password) {
            return Err(Error::validation(vec![problem]));
        }
        let hash = self.passwords.hash(password).await?;
        // Đổi mật khẩu và thu hồi mọi phiên đăng nhập trong CÙNG một transaction: kẻ
        // giữ mật khẩu cũ (hay refresh token cũ) mất quyền ngay khi mật khẩu đổi.
        let transaction = tenant.database().begin_write().await?;
        let mut update = Entity::update_many()
            .col_expr(Column::PasswordHash, Expr::value(Some(hash)))
            .col_expr(Column::UpdatedAt, Expr::value(now()))
            .filter(Column::Id.eq(id))
            .scoped(tenant);
        if only_if_unset {
            update = update.filter(Column::PasswordHash.is_null());
        }
        let updated = update.exec(&transaction).await?;
        if updated.rows_affected == 0 {
            transaction.rollback().await?;
            if only_if_unset && self.get(tenant, id).await?.is_some() {
                return Err(password_already_set());
            }
            return Err(user_not_found(id));
        }
        SessionService::revoke_user(&transaction, tenant, id).await?;
        SessionService::drop_mfa_challenges(&transaction, tenant, id).await?;
        crate::passkey::drop_ceremonies_in(&transaction, tenant, id).await?;
        let subject = id.to_string();
        // Link đăng nhập và token xác nhận lại đang chờ: phát trước khi đổi mật khẩu, không còn
        // hiệu lực sau đó.
        for purpose in ["email_verification", "login_link", "reauth"] {
            OneTimeTokens::new("auth").revoke(&transaction, tenant, purpose, &subject).await?;
        }
        if matches!(mode, PasswordWrite::Recovery | PasswordWrite::ChangeRevokingAll) {
            crate::passkey::remove_all_in(&transaction, tenant, &self.outbox, actor, id, false)
                .await?;
            crate::api_key::revoke_all_in(&transaction, tenant, &self.outbox, actor, id).await?;
            OneTimeTokens::new("auth")
                .revoke(&transaction, tenant, "password_reset", &subject)
                .await?;
            // Liên kết đăng nhập bằng nhà cung cấp ngoài cũng là một cửa: gỡ hết, chủ tài khoản
            // liên kết lại sau khi đã lấy lại tài khoản.
            self.forget_identities(&transaction, tenant, actor, id).await?;
            // Uỷ quyền bền tài khoản tạo (token SCIM, vai người cấu hình nhà cung cấp đăng nhập): kẻ
            // đã chiếm tài khoản có thể tạo chúng; chủ lấy lại tài khoản thì chúng mất theo
            // (D-20261125-durable-grants).
            super::revoke_durable_grants_in(&transaction, tenant, &self.outbox, actor, id).await?;
        }
        let event = events::PasswordChanged::default();
        self.outbox.record(&transaction, tenant, actor, events::user_subject(id), &event).await?;
        transaction.commit().await?;
        tracing::info!(user_id = %id, "password changed");
        Ok(())
    }
}

impl UserService {
    /// Gỡ mọi liên kết OIDC của tài khoản `id` trong `transaction`, mỗi liên kết một event
    /// `auth.identity_unlinked`.
    pub(super) async fn forget_identities(
        &self,
        transaction: &sea_orm::DatabaseTransaction,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
    ) -> Result<(), Error> {
        let linked = crate::oidc::identity::for_user_in(transaction, tenant, id).await?;
        if linked.is_empty() {
            return Ok(());
        }
        crate::oidc::identity::delete_for_user(transaction, tenant, id).await?;
        for identity in linked {
            let event = events::IdentityUnlinked {
                before: Some(serde_json::json!({
                    "identity_id": identity.id,
                    "provider_id": identity.provider_id,
                })),
                after: None,
            };
            let subject = events::user_subject(id);
            self.outbox.record(transaction, tenant, actor, subject, &event).await?;
        }
        Ok(())
    }
}

pub(super) fn password_already_set() -> Error {
    Error::conflict(
        "auth.password_already_set",
        "This account already has a password. Only its owner can change it, or reset it by email.",
    )
}
