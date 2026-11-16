//! Mọi bảng của `auth` thuộc một khách (bước 33 B12, D-253): email, tên role, API key, passkey…
//! duy nhất trong một khách; index quét hết hạn (job dọn dẹp) quét cả database. Bảng cha
//! (`auth_user`, `auth_role`, `auth_api_key`) trước bảng con: foreign key trong module giữ nguyên,
//! trỏ theo `id`.
//!
//! Nâng bảng bằng `tenancy::migration::adopt`. `down` của nó dựng lại bảng với index hiện có (bỏ
//! `tenant_id`); ở đây đổi thêm index quét về tên cũ và bỏ index mới thêm, để schema sau `down`
//! đúng như trước — `down` của m0021 / m0014 xoá index theo tên cũ.

use autoxide_kernel::migration::prelude::*;
use autoxide_kernel::tenancy::migration::TenantTable;

pub(crate) struct Migration {
    adopt: Box<dyn MigrationTrait>,
}

impl Migration {
    pub(crate) fn new() -> Self {
        let adopt = autoxide_kernel::tenancy::migration::adopt(
            "m0026_tenancy",
            vec![
                TenantTable::new("auth_user")
                    .unique("auth_user_email_unique", ["tenant_id", "email"]),
                TenantTable::new("auth_role")
                    .unique("auth_role_name_unique", ["tenant_id", "name"]),
                TenantTable::new("auth_api_key")
                    .unique("auth_api_key_hash_unique", ["tenant_id", "key_hash"])
                    .index("auth_api_key_user_idx", ["tenant_id", "user_id"]),
                TenantTable::new("auth_role_permission")
                    .unique("auth_role_permission_unique", ["tenant_id", "role_id", "permission"]),
                TenantTable::new("auth_user_role")
                    .unique("auth_user_role_unique", ["tenant_id", "user_id", "role_id"])
                    .index("auth_user_role_role_idx", ["tenant_id", "role_id"]),
                TenantTable::new("auth_api_key_permission").unique(
                    "auth_api_key_permission_unique",
                    ["tenant_id", "api_key_id", "permission"],
                ),
                TenantTable::new("auth_refresh_token")
                    .unique("auth_refresh_token_hash_unique", ["tenant_id", "token_hash"])
                    .index("auth_refresh_token_family_idx", ["tenant_id", "family_id"])
                    .index("auth_refresh_token_user_idx", ["tenant_id", "user_id"])
                    .drop_index("auth_refresh_token_expires_idx")
                    .sweep_index("auth_refresh_token_expires_sweep_idx", ["expires_at"])
                    .drop_index("auth_refresh_token_revoked_idx")
                    .sweep_index("auth_refresh_token_revoked_sweep_idx", ["revoked_at"]),
                TenantTable::new("auth_mfa_challenge")
                    .unique("auth_mfa_challenge_hash_unique", ["tenant_id", "token_hash"])
                    // Đổi mật khẩu bỏ mọi lượt chờ của người dùng; job dọn quét lượt hết hạn.
                    .index("auth_mfa_challenge_user_idx", ["tenant_id", "user_id"])
                    .sweep_index("auth_mfa_challenge_expires_sweep_idx", ["expires_at"]),
                TenantTable::new("auth_totp")
                    .unique("auth_totp_user_unique", ["tenant_id", "user_id"]),
                TenantTable::new("auth_recovery_code")
                    .index("auth_recovery_code_user_idx", ["tenant_id", "user_id"]),
                TenantTable::new("auth_passkey")
                    .unique("auth_passkey_credential_unique", ["tenant_id", "credential_id"])
                    .index("auth_passkey_user_idx", ["tenant_id", "user_id"]),
                TenantTable::new("auth_passkey_ceremony")
                    // Đổi mật khẩu bỏ nghi thức đang dở của người dùng.
                    .index("auth_passkey_ceremony_user_idx", ["tenant_id", "user_id"])
                    .drop_index("auth_passkey_ceremony_expires_idx")
                    .sweep_index("auth_passkey_ceremony_expires_sweep_idx", ["expires_at"]),
            ],
        );
        Migration { adopt }
    }
}

/// Index trước bước 33 mà `up` đổi tên (bảng, tên mới, tên cũ, cột).
const RENAMED: [(&str, &str, &str, &str); 3] = [
    (
        "auth_refresh_token",
        "auth_refresh_token_expires_sweep_idx",
        "auth_refresh_token_expires_idx",
        "expires_at",
    ),
    (
        "auth_refresh_token",
        "auth_refresh_token_revoked_sweep_idx",
        "auth_refresh_token_revoked_idx",
        "revoked_at",
    ),
    (
        "auth_passkey_ceremony",
        "auth_passkey_ceremony_expires_sweep_idx",
        "auth_passkey_ceremony_expires_idx",
        "expires_at",
    ),
];

/// Index `up` thêm mới (bảng, tên).
const ADDED: [(&str, &str); 3] = [
    ("auth_mfa_challenge", "auth_mfa_challenge_user_idx"),
    ("auth_mfa_challenge", "auth_mfa_challenge_expires_sweep_idx"),
    ("auth_passkey_ceremony", "auth_passkey_ceremony_user_idx"),
];

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0026_tenancy"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        self.adopt.up(manager).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        self.adopt.down(manager).await?;
        for (table, name) in ADDED {
            manager.drop_index(Index::drop().name(name).table(table).to_owned()).await?;
        }
        for (table, new, old, column) in RENAMED {
            manager.drop_index(Index::drop().name(new).table(table).to_owned()).await?;
            manager
                .create_index(Index::create().name(old).table(table).col(column).to_owned())
                .await?;
        }
        Ok(())
    }
}
