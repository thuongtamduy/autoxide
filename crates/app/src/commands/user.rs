//! Lệnh `autoxide user`: quản lý tài khoản từ dòng lệnh, chạy với quyền của người
//! vận hành máy chủ. Dùng để tạo người dùng đầu tiên — lúc chưa ai đăng nhập được
//! để tạo qua API.

use std::io::BufRead;
use std::process::ExitCode;

use autoxide_auth::api::{AccountRecovery, Accounts, NewAccount};
use autoxide_kernel::error::Error;
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
    },
}

pub fn run(arguments: &Arguments) -> ExitCode {
    let (email, name, admin, password_stdin) = match arguments.action.clone() {
        Action::Create { email, name, admin, password_stdin } => {
            (email, name, admin, password_stdin)
        }
        Action::ResetMfa { email } => return reset_mfa_command(&email),
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
    match run_with_observability(&config, create(settings, account)) {
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

/// Khởi động chương trình, tạo tài khoản, trả về nội dung in ra (chưa có mật khẩu).
async fn create(settings: Settings, account: NewAccount) -> Result<String, String> {
    let admin = account.admin;
    let runtime = runtime::start(settings, Migrations::RequireApplied).await?;
    let outcome = match runtime.modules.api::<dyn Accounts>() {
        Some(accounts) => accounts.create(account).await.map_err(|error| describe(&error)),
        None => Err("no registered module manages user accounts (`auth`)".to_owned()),
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

/// `autoxide user reset-mfa`.
fn reset_mfa_command(email: &str) -> ExitCode {
    let settings = match settings::load() {
        Ok(settings) => settings,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    let config = settings.config.clone();
    match run_with_observability(&config, reset_mfa(settings, email.to_owned())) {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => report_failure(&error),
    }
}

/// Khởi động chương trình, tắt xác thực hai bước của tài khoản `email`.
async fn reset_mfa(settings: Settings, email: String) -> Result<String, String> {
    let runtime = runtime::start(settings, Migrations::RequireApplied).await?;
    let outcome = match runtime.modules.api::<dyn AccountRecovery>() {
        Some(recovery) => recovery.reset_mfa(&email).await.map_err(|error| describe(&error)),
        None => Err("no registered module manages user accounts (`auth`)".to_owned()),
    };
    let closed = runtime.database.close().await.map_err(|error| format!("database: {error}"));
    let user = outcome?;
    closed?;
    Ok(format!(
        "Two-step sign-in of {} (id {}) is off. Sign in with the password, then turn it on again.\n",
        user.email, user.id
    ))
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
fn generate_password() -> Result<String, String> {
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
