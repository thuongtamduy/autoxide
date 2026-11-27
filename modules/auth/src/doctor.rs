//! Bước kiểm của `auth` cho `autoxide doctor` (D-166): đã có quản trị viên đang hoạt động chưa —
//! bản cài mới chưa có ai thì không ai đăng nhập quản lý được, lỗi hay gặp nhất của người tự cài;
//! và đã đặt khoá ký access token (`AUTOXIDE_AUTH_TOKEN_SECRET`) chưa.

use autoxide_kernel::doctor::{Finding, Probe};
use autoxide_kernel::tenancy::DatabaseRegistry;

use crate::role::count_active_admins_unlocked;

/// Kiểm quản trị viên của khách mặc định (khách nền tảng, bước 33): người vận hành bản cài quản
/// lý nó; khách khác có role `admin` riêng, gieo khi tạo khách.
pub(crate) struct Administrator {
    pub databases: DatabaseRegistry,
}

#[async_trait::async_trait]
impl Probe for Administrator {
    async fn probe(&self) -> Finding {
        let tenant = self.databases.default_tenant();
        match count_active_admins_unlocked(tenant.database().read(), &tenant).await {
            Ok(0) => Finding::warning("No active account has the `admin` role, so nobody can sign in to manage this server.")
                .with_hint("Run `autoxide user create --email <email> --name <name> --admin`."),
            Ok(count) => Finding::ok(format!("{count} active administrator(s).")),
            Err(error) => Finding::failed(format!("cannot count administrators: {error}")),
        }
    }
}

/// Khoá ký access token có đặt trong cấu hình không. Giá trị đọc một lần lúc `configure` (qua
/// `ConfigReader`, như mọi biến khác của `auth`); bước kiểm chỉ giữ kết quả, không giữ khoá.
pub(crate) struct TokenSecret {
    pub configured: bool,
}

#[async_trait::async_trait]
impl Probe for TokenSecret {
    async fn probe(&self) -> Finding {
        if self.configured {
            Finding::ok("Access tokens are signed with the configured AUTOXIDE_AUTH_TOKEN_SECRET.")
        } else {
            Finding::warning(
                "AUTOXIDE_AUTH_TOKEN_SECRET is not set: the server signs access tokens with a \
                 random key made at startup, so signed-in clients must refresh their token after \
                 every restart and several servers cannot share tokens.",
            )
            .with_hint(
                "Set AUTOXIDE_AUTH_TOKEN_SECRET to a long random value (`openssl rand -base64 \
                 48`), the same on every server, before going live.",
            )
        }
    }
}

/// Đăng nhập OAuth cho client MCP có bật không (D-20261126-mcp-oauth). Tắt không phải lỗi (client vẫn dùng API key)
/// nên chỉ nêu cách bật.
pub(crate) struct McpOAuth {
    pub issuer: Option<String>,
}

#[async_trait::async_trait]
impl Probe for McpOAuth {
    async fn probe(&self) -> Finding {
        match &self.issuer {
            Some(issuer) => Finding::ok(format!("MCP clients sign in with OAuth at issuer {issuer}.")),
            None => Finding::ok("OAuth sign-in for MCP clients is off: they connect with an API key.").with_hint(
                "To let Claude, ChatGPT and other MCP clients connect without pasting a key, set \
                 AUTOXIDE_HTTP_PUBLIC_URL (no path) and AUTOXIDE_AUTH_OAUTH_CONSENT_URL (the consent page of your \
                 interface).",
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::doctor::Status;
    use autoxide_kernel::event::Actor;
    use autoxide_testkit::TestDatabase;

    use super::*;
    use crate::user::NewUser;

    /// Chưa có admin thì cảnh báo kèm lệnh tạo; có rồi thì đạt — trên cả hai database.
    #[tokio::test]
    async fn bao_khi_chua_co_quan_tri_vien() {
        TestDatabase::run(async |test| {
            let services = crate::testing::services(test).await;
            let tenant = services.tenant.clone();
            let databases =
                autoxide_kernel::tenancy::DatabaseRegistry::single(services.database.clone());
            let probe = Administrator { databases };

            let finding = probe.probe().await;
            assert_eq!(finding.status, Status::Warning, "{finding:?}");
            assert!(finding.hint.unwrap().contains("--admin"));

            let user = |email: &str| NewUser {
                email: email.to_owned(),
                name: "Ada".to_owned(),
                password: None,
            };
            services
                .users
                .create(&tenant, Actor::system(), user("member@example.com"))
                .await
                .unwrap();
            assert_eq!(probe.probe().await.status, Status::Warning);
            let admin = user("admin@example.com");
            services.users.create_with_roles(&tenant, Actor::system(), admin, true).await.unwrap();
            let finding = probe.probe().await;
            assert_eq!(
                (finding.status, finding.message.as_str()),
                (Status::Ok, "1 active administrator(s).")
            );
        })
        .await
        .unwrap();
    }

    /// Không đặt khoá ký thì cảnh báo kèm cách tạo khoá; đặt rồi thì đạt.
    #[tokio::test]
    async fn bao_khi_chua_dat_khoa_ky_access_token() {
        let missing = TokenSecret { configured: false }.probe().await;
        assert_eq!(missing.status, Status::Warning, "{missing:?}");
        assert!(missing.message.contains("AUTOXIDE_AUTH_TOKEN_SECRET"), "{missing:?}");
        assert!(missing.hint.unwrap().contains("openssl rand"));
        assert_eq!(TokenSecret { configured: true }.probe().await.status, Status::Ok);
    }

    #[tokio::test]
    async fn bao_cach_bat_oauth_cho_mcp() {
        let off = McpOAuth { issuer: None }.probe().await;
        assert!(off.hint.unwrap().contains("AUTOXIDE_AUTH_OAUTH_CONSENT_URL"));
        let on = McpOAuth { issuer: Some("https://api.example.com".to_owned()) }.probe().await;
        assert!(on.message.contains("https://api.example.com"));
    }
}
