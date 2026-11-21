//! Lệnh `autoxide user`: quản lý tài khoản từ dòng lệnh, chạy với quyền của người
//! vận hành máy chủ. Dùng để tạo người dùng đầu tiên — lúc chưa ai đăng nhập được
//! để tạo qua API.

use std::io::BufRead;
use std::process::ExitCode;

use autoxide_auth::api::{AccountRecovery, Accounts, Invitations, NewAccount, NewInvitation};
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use super::{report_failure, report_invalid_configuration, run_with_observability};
use crate::runtime::{self, Migrations};
use crate::settings::{self, Settings};

#[derive(Debug, clap::Args)]
pub struct Arguments {
    #[command(subcommand)]
    action: Action,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Action {
    /// Create a user account and print its password.
    ///
    /// Without `--password-stdin`, a strong random password is generated and printed
    /// once. Run `autoxide migrate up` first.
    Create {
        /// Email of the account; it is also the sign-in name.
        #[arg(long)]
        email: String,
        /// Display name.
        #[arg(long)]
        name: String,
        /// Give the account the built-in `admin` role (every permission). Use this for
        /// the first user.
        #[arg(long)]
        admin: bool,
        /// Read the password from the first line of standard input instead of
        /// generating one: `printf '%s' "$PASSWORD" | autoxide user create …`.
        #[arg(long)]
        password_stdin: bool,
        /// Short name (slug) of the tenant the account belongs to. Defaults to the default
        /// tenant (the only one in `single` mode).
        #[arg(long)]
        tenant: Option<String>,
    },
    /// Invite someone by email, with roles of the tenant.
    ///
    /// The mail goes out shortly (the server must be running its worker, or run `autoxide jobs`);
    /// the link can be used once, within `AUTOXIDE_AUTH_INVITATION_DAYS` days. Run
    /// `autoxide migrate up` first.
    Invite {
        /// Email to invite.
        #[arg(long)]
        email: String,
        /// Name of a role given when the invitation is accepted; repeat for several.
        #[arg(long = "role")]
        roles: Vec<String>,
        /// Id of the organization unit the roles apply at. Defaults to the root unit.
        #[arg(long)]
        org: Option<uuid::Uuid>,
        /// Short name (slug) of the tenant. Defaults to the default tenant.
        #[arg(long)]
        tenant: Option<String>,
    },
    /// Turn off two-step sign-in (authenticator app and recovery codes) of an account.
    ///
    /// For an administrator who lost their device when no other administrator can do it
    /// through the API (`DELETE /api/v1/auth/users/{id}/mfa`). The account signs in with its
    /// password alone until it turns two-step sign-in on again.
    ResetMfa {
        /// Email of the account.
        #[arg(long)]
        email: String,
        /// Short name (slug) of the tenant of the account. Defaults to the default tenant.
        #[arg(long)]
        tenant: Option<String>,
    },
    /// Remove every passkey of an account.
    ///
    /// For an administrator who lost the devices holding their passkeys when no other
    /// administrator can do it through the API (`DELETE /api/v1/auth/users/{id}/passkeys`).
    /// The account signs in with its password and can add new passkeys.
    RemovePasskeys {
        /// Email of the account.
        #[arg(long)]
        email: String,
        /// Short name (slug) of the tenant of the account. Defaults to the default tenant.
        #[arg(long)]
        tenant: Option<String>,
    },
}

pub fn run(arguments: &Arguments) -> ExitCode {
    let (email, name, admin, password_stdin, tenant) = match arguments.action.clone() {
        Action::Create { email, name, admin, password_stdin, tenant } => {
            (email, name, admin, password_stdin, tenant)
        }
        Action::ResetMfa { email, tenant } => {
            return recovery_command(Recovery::ResetMfa(email), tenant);
        }
        Action::Invite { email, roles, org, tenant } => {
            let mut invitation = NewInvitation::new(email, roles);
            if let Some(org) = org {
                invitation = invitation.at(org);
            }
            return invite_command(invitation, tenant);
        }
        Action::RemovePasskeys { email, tenant } => {
            return recovery_command(Recovery::RemovePasskeys(email), tenant);
        }
    };
    let settings = match settings::load() {
        Ok(settings) => settings,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    // Đọc stdin TRƯỚC khi dựng runtime: lỗi đọc thì báo ngay, chưa kết nối gì.
    let (password, generated) = if password_stdin {
        match read_password(std::io::stdin().lock()) {
            Ok(password) => (password, false),
            Err(error) => return report_failure(&error),
        }
    } else {
        match generate_password() {
            Ok(password) => (password, true),
            Err(error) => return report_failure(&error),
        }
    };
    let account = NewAccount { email, name, password: Some(password.clone()), admin };
    let config = settings.config.clone();
    match run_with_observability(&config, create(settings, account, tenant)) {
        Ok(report) => {
            print!("{report}");
            if generated {
                println!("Password: {password}");
                println!("Store this password now: it is not shown again.");
            }
            ExitCode::SUCCESS
        }
        Err(error) => report_failure(&error),
    }
}

/// Khách `slug` (bỏ trống: khách mặc định) qua sổ database của chương trình (bước 33).
async fn tenant_of(
    runtime: &runtime::Runtime,
    slug: Option<&str>,
) -> Result<TenantContext, String> {
    let registry = runtime
        .modules
        .tenancy()
        .ok_or_else(|| "the modules were not started with a tenancy registry".to_owned())?;
    match slug.map(str::trim) {
        Some(slug) => registry.tenant_by_slug(slug).await.map_err(|error| describe(&error)),
        None => Ok(registry.default_tenant()),
    }
}

/// Khởi động chương trình, tạo tài khoản, trả về nội dung in ra (chưa có mật khẩu).
async fn create(
    settings: Settings,
    account: NewAccount,
    tenant: Option<String>,
) -> Result<String, String> {
    let admin = account.admin;
    let runtime = runtime::start(settings, Migrations::RequireApplied).await?;
    let outcome =
        match (runtime.modules.api::<dyn Accounts>(), tenant_of(&runtime, tenant.as_deref()).await)
        {
            (Some(accounts), Ok(tenant)) => {
                accounts.create(&tenant, account).await.map_err(|error| describe(&error))
            }
            (None, _) => Err("no registered module manages user accounts (`auth`)".to_owned()),
            (_, Err(error)) => Err(error),
        };
    // Đóng kết nối cả khi tạo hỏng; lỗi tạo quan trọng hơn lỗi lúc đóng.
    let closed = runtime.database.close().await.map_err(|error| format!("database: {error}"));
    let user = outcome?;
    closed?;
    let role = if admin { " with the `admin` role" } else { " without any role" };
    Ok(format!(
        "Created user {} (id {}){role}.\nSign in with `POST /api/v1/auth/login`.\n",
        user.email, user.id
    ))
}

/// Lệnh `invite`: đọc cấu hình, mời qua `Invitations` của module `auth`, in kết quả.
fn invite_command(invitation: NewInvitation, tenant: Option<String>) -> ExitCode {
    let settings = match settings::load() {
        Ok(settings) => settings,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    let config = settings.config.clone();
    match run_with_observability(&config, invite(settings, invitation, tenant)) {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => report_failure(&error),
    }
}

async fn invite(
    settings: Settings,
    invitation: NewInvitation,
    tenant: Option<String>,
) -> Result<String, String> {
    let runtime = runtime::start(settings, Migrations::RequireApplied).await?;
    let found = tenant_of(&runtime, tenant.as_deref()).await;
    let outcome = match (runtime.modules.api::<dyn Invitations>(), found) {
        (Some(invitations), Ok(tenant)) => {
            invitations.invite(&tenant, invitation).await.map_err(|error| describe(&error))
        }
        (None, _) => Err("no registered module manages invitations (`auth`)".to_owned()),
        (_, Err(error)) => Err(error),
    };
    let closed = runtime.database.close().await.map_err(|error| format!("database: {error}"));
    let summary = outcome?;
    closed?;
    Ok(format!(
        "Invited {} (invitation {}); it can be accepted until {}.\n",
        summary.email, summary.id, summary.expires_at
    ))
}

/// `autoxide user reset-mfa`.
/// Việc khôi phục tài khoản của người vận hành, qua `AccountRecovery` của module `auth`.
enum Recovery {
    ResetMfa(String),
    RemovePasskeys(String),
}

/// Lệnh `reset-mfa` / `remove-passkeys`: đọc cấu hình, làm `action` ở khách `tenant`, in kết quả.
fn recovery_command(action: Recovery, tenant: Option<String>) -> ExitCode {
    let settings = match settings::load() {
        Ok(settings) => settings,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    let config = settings.config.clone();
    match run_with_observability(&config, recover(settings, action, tenant)) {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => report_failure(&error),
    }
}

/// Khởi động chương trình, làm `action`, đóng database. Trả câu báo cho người vận hành.
async fn recover(
    settings: Settings,
    action: Recovery,
    tenant: Option<String>,
) -> Result<String, String> {
    let runtime = runtime::start(settings, Migrations::RequireApplied).await?;
    let found = tenant_of(&runtime, tenant.as_deref()).await;
    let outcome = match (runtime.modules.api::<dyn AccountRecovery>(), found) {
        (Some(recovery), Ok(tenant)) => match action {
            Recovery::ResetMfa(email) => recovery.reset_mfa(&tenant, &email).await.map(|user| {
                format!(
                    "Two-step sign-in of {} (id {}) is off. Sign in with the password, then turn \
                     it on again.\n",
                    user.email, user.id
                )
            }),
            Recovery::RemovePasskeys(email) => {
                recovery.remove_passkeys(&tenant, &email).await.map(|(user, removed)| {
                    format!(
                        "Removed {removed} passkey(s) of {} (id {}). Sign in with the password, \
                         then add new passkeys.\n",
                        user.email, user.id
                    )
                })
            }
        }
        .map_err(|error| describe(&error)),
        (None, _) => Err("no registered module manages user accounts (`auth`)".to_owned()),
        (_, Err(error)) => Err(error),
    };
    let closed = runtime.database.close().await.map_err(|error| format!("database: {error}"));
    let report = outcome?;
    closed?;
    Ok(report)
}

/// Mật khẩu là dòng đầu của `input`, bỏ ký tự xuống dòng cuối (không bỏ khoảng trắng:
/// nó có thể là một phần của mật khẩu).
fn read_password(mut input: impl BufRead) -> Result<String, String> {
    let mut line = String::new();
    input
        .read_line(&mut line)
        .map_err(|error| format!("cannot read the password from standard input: {error}"))?;
    let password = line.strip_suffix('\n').unwrap_or(&line);
    let password = password.strip_suffix('\r').unwrap_or(password);
    if password.is_empty() {
        return Err("standard input is empty; send the password on its first line".to_owned());
    }
    Ok(password.to_owned())
}

/// 18 byte ngẫu nhiên (144 bit) thành 24 ký tự base64url.
pub(crate) fn generate_password() -> Result<String, String> {
    let mut bytes = [0u8; 18];
    getrandom::fill(&mut bytes).map_err(|error| format!("cannot generate a password: {error}"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// Thông báo của lỗi kèm lỗi từng trường, cho người đọc ở dòng lệnh.
fn describe(error: &Error) -> String {
    let mut text = format!("{} ({})", error.message(), error.code());
    for field in error.fields() {
        text.push_str(&format!("\n  - {}: {}", field.field, field.message));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::error::FieldError;

    #[test]
    fn mat_khau_doc_tu_dong_dau_tien() {
        assert_eq!(read_password("s3cret pass \nignored\n".as_bytes()).unwrap(), "s3cret pass ");
        assert_eq!(read_password("windows\r\n".as_bytes()).unwrap(), "windows");
        assert_eq!(read_password("no newline".as_bytes()).unwrap(), "no newline");
        assert!(read_password("".as_bytes()).unwrap_err().contains("standard input is empty"));
    }

    #[test]
    fn mat_khau_sinh_ra_du_dai_va_khac_nhau() {
        let first = generate_password().unwrap();

        assert_eq!(first.len(), 24);
        assert_ne!(first, generate_password().unwrap());
    }

    #[test]
    fn loi_kem_tung_truong() {
        let error = Error::validation(vec![FieldError::new("password", "length", "too short")]);

        assert_eq!(
            describe(&error),
            "The request contains invalid values. See `errors` for each field. \
             (validation_failed)\n  - password: too short"
        );
    }
}
