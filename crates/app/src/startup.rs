//! Việc `autoxide serve` làm lúc khởi động, trước khi nhận request — đều TẮT mặc định
//! (D-20261121-migrate-on-start):
//!
//! 1. `AUTOXIDE_MIGRATE_ON_START=true`: áp dụng migration còn chờ của database chính.
//! 2. `AUTOXIDE_BOOTSTRAP_ADMIN_EMAIL`: khách mặc định chưa có quản trị viên nào thì tạo một tài
//!    khoản quản trị (qua trait `Accounts` của module `auth`, như `autoxide user create`).
//! 3. `AUTOXIDE_STARTUP_TASKS`: chạy các lệnh quản trị của module (vd nạp đơn vị hành chính).
//!
//! Cả ba chạy dưới khoá migration ([`MigrationLock`]): nhiều bản máy chủ khởi động cùng lúc thì
//! lần lượt từng bản; bản sau thấy migration đã áp dụng, quản trị viên đã có, và không làm lại.
//! Production vẫn nên chạy `autoxide migrate up` riêng mỗi lần deploy: người vận hành thấy rõ cái
//! gì sắp đổi, và migration hỏng thì deploy dừng trước khi bản mới nhận request.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use autoxide_auth::api::{Accounts, NewAccount, TenantAdmins};
use autoxide_kernel::config::{ConfigReader, Invalid, Setting};
use autoxide_kernel::database::Database;
use autoxide_kernel::migration::MigrationLock;

use crate::runtime::Runtime;
use crate::settings::Settings;

pub const MIGRATE_ON_START: Setting = Setting::new(
    "AUTOXIDE_MIGRATE_ON_START",
    "Apply the pending migrations of the main database when `autoxide serve` starts (`true` or \
     `false`). Servers starting at the same time take turns (a lock), so only one applies them. \
     Handy for one server or a first try; in production prefer running `autoxide migrate up` \
     once per deployment, before starting the servers. Placements (AUTOXIDE_TENANCY_MODE=multi) \
     still need `autoxide migrate up --all`.",
)
.with_default("false");

pub const BOOTSTRAP_ADMIN_EMAIL: Setting = Setting::new(
    "AUTOXIDE_BOOTSTRAP_ADMIN_EMAIL",
    "When set, and the default tenant has no administrator yet, `autoxide serve` creates an \
     administrator account with this email on start; once an administrator exists it does \
     nothing. The password comes from AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD or \
     AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD_FILE; with neither, a random password is generated and \
     written once to AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD_OUT (never to the log). Needs the \
     migrations applied (see AUTOXIDE_MIGRATE_ON_START).",
)
.with_example("admin@example.com");

pub const BOOTSTRAP_ADMIN_PASSWORD: Setting = Setting::new(
    "AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD",
    "Password of the administrator AUTOXIDE_BOOTSTRAP_ADMIN_EMAIL creates. Only used while no \
     administrator exists; change it after the first sign-in.",
)
.secret();

pub const BOOTSTRAP_ADMIN_PASSWORD_FILE: Setting = Setting::new(
    "AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD_FILE",
    "File whose first line is the password of the administrator AUTOXIDE_BOOTSTRAP_ADMIN_EMAIL \
     creates (e.g. a Docker secret under /run/secrets). Use this or \
     AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD, not both.",
);

pub const BOOTSTRAP_ADMIN_PASSWORD_OUT: Setting = Setting::new(
    "AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD_OUT",
    "File where `autoxide serve` writes the email and generated password of the first \
     administrator when AUTOXIDE_BOOTSTRAP_ADMIN_EMAIL is set without a password; readable only \
     by the server's user. Relative paths start from the working directory (in the Docker image, \
     the data volume). Delete it once the password is changed.",
)
.with_default("data/initial-admin-password");

pub const STARTUP_TASKS: Setting = Setting::new(
    "AUTOXIDE_STARTUP_TASKS",
    "Maintenance tasks `autoxide serve` runs on start, after the migrations and the first \
     administrator: comma-separated `<module>.<task>` names, run without arguments (see \
     `autoxide task`). Each must be safe to run again: e.g. \
     `address.import,tax.import_pack,promotions.load_legal_data` loads the reference data shipped \
     with the server and changes nothing the second time. Empty: none.",
)
.with_example("address.import,tax.import_pack,promotions.load_legal_data");

/// Chờ tiến trình khác migrate xong tối đa chừng này: migration lớn trên bảng nhiều dòng có thể
/// mất vài phút; lâu hơn thì nhiều khả năng tiến trình kia treo, người vận hành cần biết.
pub(crate) const LOCK_WAIT: Duration = Duration::from_secs(15 * 60);

/// Tên hiển thị của quản trị viên đầu tiên; đổi được sau khi đăng nhập.
const ADMIN_NAME: &str = "Administrator";

/// Cấu hình của các việc lúc khởi động.
#[derive(Debug, Clone, Default)]
pub struct StartupConfig {
    pub migrate: bool,
    pub admin: Option<AdminBootstrap>,
    pub tasks: Vec<String>,
}

/// Quản trị viên đầu tiên cần tạo.
#[derive(Debug, Clone)]
pub struct AdminBootstrap {
    pub email: String,
    pub password: PasswordSource,
    /// Nơi ghi mật khẩu tự sinh (`AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD_OUT`).
    pub password_out: PathBuf,
}

/// Mật khẩu của quản trị viên đầu tiên lấy từ đâu.
#[derive(Clone)]
pub enum PasswordSource {
    Value(String),
    File(PathBuf),
    Generate,
}

// Tự viết `Debug` thay vì `derive`: không bao giờ in mật khẩu, kể cả khi in cả cấu hình để gỡ lỗi.
impl fmt::Debug for PasswordSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PasswordSource::Value(_) => f.write_str("Value(***)"),
            PasswordSource::File(path) => f.debug_tuple("File").field(path).finish(),
            PasswordSource::Generate => f.write_str("Generate"),
        }
    }
}

impl StartupConfig {
    pub fn read(reader: &mut ConfigReader<'_>) -> Result<Self, Invalid> {
        let migrate = reader.get::<bool>(&MIGRATE_ON_START);
        let email = reader.optional::<String>(&BOOTSTRAP_ADMIN_EMAIL);
        let password = reader.optional::<String>(&BOOTSTRAP_ADMIN_PASSWORD);
        let password_file = reader.optional::<PathBuf>(&BOOTSTRAP_ADMIN_PASSWORD_FILE);
        let tasks = reader.optional_with(&STARTUP_TASKS, parse_tasks);
        let password_out = reader.get::<PathBuf>(&BOOTSTRAP_ADMIN_PASSWORD_OUT);
        let (migrate, email, password, password_file, tasks, password_out) =
            (migrate?, email?, password?, password_file?, tasks?, password_out?);
        let email = email.map(|email| email.trim().to_owned()).filter(|email| !email.is_empty());
        let password = match (password, password_file) {
            (Some(_), Some(_)) => {
                return reader.reject(
                    &BOOTSTRAP_ADMIN_PASSWORD_FILE,
                    "set AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD or AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD_FILE, \
                     not both",
                );
            }
            (Some(value), None) => Some(PasswordSource::Value(value)),
            (None, Some(path)) => Some(PasswordSource::File(path)),
            (None, None) => None,
        };
        let admin = match (email, password) {
            (Some(email), password) => Some(AdminBootstrap {
                email,
                password: password.unwrap_or(PasswordSource::Generate),
                password_out,
            }),
            (None, Some(_)) => {
                return reader.reject(
                    &BOOTSTRAP_ADMIN_EMAIL,
                    "a bootstrap administrator password is set but not the email; set \
                     AUTOXIDE_BOOTSTRAP_ADMIN_EMAIL too",
                );
            }
            (None, None) => None,
        };
        Ok(StartupConfig { migrate, admin, tasks: tasks.unwrap_or_default() })
    }

    /// Có việc nào phải làm dưới khoá không.
    fn has_work(&self) -> bool {
        self.migrate || self.admin.is_some() || !self.tasks.is_empty()
    }
}

/// `a.b, c.d` → `["a.b", "c.d"]`; mỗi tên phải có dạng `<module>.<task>`.
fn parse_tasks(raw: &str) -> Result<Vec<String>, String> {
    let mut tasks = Vec::new();
    for name in raw.split(',').map(str::trim).filter(|name| !name.is_empty()) {
        match name.split_once('.') {
            Some((module, task)) if !module.is_empty() && !task.is_empty() => {
                tasks.push(name.to_owned());
            }
            _ => return Err(format!("`{name}` is not a task name; write `<module>.<task>`")),
        }
    }
    Ok(tasks)
}

/// Khoá đang giữ trong lúc chuẩn bị; `None` khi không có việc gì.
pub struct Prepared(Option<MigrationLock>);

/// Phần trước khi dựng chương trình: lấy khoá (nếu có việc), rồi migrate (nếu bật).
pub async fn before_start(settings: &Settings) -> Result<Prepared, String> {
    let startup = &settings.startup;
    if !startup.has_work() {
        return Ok(Prepared(None));
    }
    let database_config = &settings.config.database;
    let lock = MigrationLock::acquire(database_config, LOCK_WAIT)
        .await
        .map_err(|error| error.to_string())?;
    if startup.migrate {
        let migrated = async {
            let migrator = settings.modules.migrator().map_err(|error| error.to_string())?;
            let database =
                Database::connect(database_config).await.map_err(|error| error.to_string())?;
            let applied = migrator.up(&database).await.map_err(|error| error.to_string());
            let closed = database.close().await.map_err(|error| format!("database: {error}"));
            let applied = applied?;
            closed?;
            if applied.is_empty() {
                tracing::info!("migrations: nothing to apply");
            } else {
                let names: Vec<String> = applied
                    .iter()
                    .map(|state| format!("{}/{}", state.module, state.name))
                    .collect();
                tracing::info!(count = applied.len(), applied = ?names, "migrations applied");
            }
            Ok::<(), String>(())
        }
        .await;
        if let Err(error) = migrated {
            release(Prepared(Some(lock))).await;
            return Err(error);
        }
    }
    Ok(Prepared(Some(lock)))
}

/// Phần sau khi dựng chương trình: quản trị viên đầu tiên, rồi các task. Nhả khoá dù thành công
/// hay không.
pub async fn after_start(
    runtime: &Runtime,
    startup: &StartupConfig,
    prepared: Prepared,
) -> Result<(), String> {
    let outcome = async {
        if let Some(admin) = &startup.admin {
            bootstrap_admin(runtime, admin).await?;
        }
        run_tasks(runtime, &startup.tasks).await
    }
    .await;
    release(prepared).await;
    outcome
}

/// Nhả khoá; lỗi lúc nhả chỉ ghi log (khoá cũng tự nhả khi kết nối / file đóng).
pub async fn release(prepared: Prepared) {
    if let Some(lock) = prepared.0
        && let Err(error) = lock.release().await
    {
        tracing::warn!(error = %error, "cannot release the migration lock");
    }
}

async fn bootstrap_admin(runtime: &Runtime, admin: &AdminBootstrap) -> Result<(), String> {
    let missing = || {
        "AUTOXIDE_BOOTSTRAP_ADMIN_EMAIL is set but no registered module manages user accounts \
         (`auth`)"
            .to_owned()
    };
    let admins = runtime.modules.api::<dyn TenantAdmins>().ok_or_else(missing)?;
    let accounts = runtime.modules.api::<dyn Accounts>().ok_or_else(missing)?;
    let registry = runtime
        .modules
        .tenancy()
        .ok_or_else(|| "the modules were not started with a tenancy registry".to_owned())?;
    let tenant = registry.default_tenant();
    let existing = admins.admins(&tenant).await.map_err(|error| describe(&error))?;
    if !existing.is_empty() {
        tracing::info!(
            "an administrator exists already; AUTOXIDE_BOOTSTRAP_ADMIN_EMAIL changes nothing"
        );
        return Ok(());
    }
    let (password, generated) = match &admin.password {
        PasswordSource::Value(value) => (value.clone(), false),
        PasswordSource::File(path) => (read_password_file(path)?, false),
        PasswordSource::Generate => (crate::commands::user::generate_password()?, true),
    };
    let account = NewAccount {
        email: admin.email.clone(),
        name: ADMIN_NAME.to_owned(),
        password: Some(password.clone()),
        admin: true,
    };
    let user = accounts
        .create(&tenant, account)
        .await
        .map_err(|error| format!("cannot create the first administrator: {}", describe(&error)))?;
    if generated {
        // Mật khẩu tự sinh không vào log (log hay được gom về hệ thống khác): ghi vào một file chỉ
        // chủ tiến trình đọc được, trong thư mục dữ liệu; log chỉ nói đường dẫn
        // (D-20261121-admin-password-file, thay phần "ghi vào log" của D-20261121-migrate-on-start).
        let path = admin.password_out.as_path();
        write_private(path, &format!("{}\n{password}\n", user.email)).map_err(|error| {
            format!(
                "created the first administrator `{}` but cannot write its generated password to \
                 `{}`: {error}; reset the password of that account",
                user.email,
                path.display()
            )
        })?;
        // ID chứ không phải email: log không mang dữ liệu cá nhân (rà soát bảo mật 2026-11).
        tracing::warn!(
            user_id = %user.id,
            file = %path.display(),
            "created the first administrator with a generated password, written to the file \
             (readable only by this server's user): sign in, change the password, then delete \
             the file"
        );
    } else {
        tracing::info!(user_id = %user.id, "created the first administrator");
    }
    Ok(())
}

/// Ghi `text` vào `path` (tạo thư mục cha), quyền `0600` trên Unix: chỉ chủ file đọc được.
fn write_private(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    // File có sẵn (lần chạy trước) giữ quyền cũ: đặt lại cho chắc.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(text.as_bytes())
}

/// Dòng đầu của file, bỏ ký tự xuống dòng cuối.
fn read_password_file(path: &std::path::Path) -> Result<String, String> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        format!("AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD_FILE: cannot read `{}`: {error}", path.display())
    })?;
    let line = text.lines().next().unwrap_or_default();
    if line.is_empty() {
        return Err(format!(
            "AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD_FILE: the first line of `{}` is empty",
            path.display()
        ));
    }
    Ok(line.to_owned())
}

async fn run_tasks(runtime: &Runtime, names: &[String]) -> Result<(), String> {
    if names.is_empty() {
        return Ok(());
    }
    let tasks = runtime.modules.tasks().map_err(|error| error.to_string())?;
    for name in names {
        let Some((_, task)) = tasks.iter().find(|(full, _)| full == name) else {
            // Task của module đang tắt (bước 70 P1): bỏ qua kèm cảnh báo, để người chọn tập module
            // nhỏ hơn không phải sửa thêm danh sách task (vd của quickstart). Tên sai thật vẫn là lỗi.
            let owner = name.split('.').next().unwrap_or_default();
            if runtime.modules.inactive().iter().any(|module| module.name == owner) {
                tracing::warn!(
                    task = %name,
                    module = owner,
                    "startup task skipped: its module is turned off (AUTOXIDE_MODULES)"
                );
                continue;
            }
            return Err(format!(
                "AUTOXIDE_STARTUP_TASKS: there is no task `{name}`; run `autoxide task` to list \
                 them"
            ));
        };
        let args = task
            .parse(&[])
            .map_err(|error| format!("AUTOXIDE_STARTUP_TASKS: task `{name}`: {error}"))?;
        let report = task
            .run(&args)
            .await
            .map_err(|error| format!("AUTOXIDE_STARTUP_TASKS: task `{name}` failed: {error}"))?;
        tracing::info!(task = %name, report = %report.trim(), "startup task done");
    }
    Ok(())
}

fn describe(error: &autoxide_kernel::error::Error) -> String {
    crate::commands::describe_error(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::config::ConfigSource;

    fn read(pairs: &[(&str, &str)]) -> Result<StartupConfig, String> {
        let source = ConfigSource::from_pairs(pairs.iter().copied());
        let mut reader = ConfigReader::new(&source);
        let config = StartupConfig::read(&mut reader);
        reader.finish(config).map(|(config, _)| config).map_err(|errors| errors.to_string())
    }

    #[test]
    fn mat_khau_tu_sinh_ghi_file_chi_chu_doc_duoc() {
        let dir = std::env::temp_dir().join(format!("autoxide-admin-{}", uuid::Uuid::now_v7()));
        let path = dir.join("data").join("initial-admin-password");
        write_private(&path, "a@example.com\nfirst\n").unwrap();
        // Ghi lại (lần chạy sau trên database trống) thay nội dung, vẫn chỉ chủ đọc được.
        write_private(&path, "a@example.com\nsecond\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a@example.com\nsecond\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mac_dinh_khong_lam_gi() {
        let config = read(&[]).unwrap();

        assert!(!config.migrate);
        assert!(config.admin.is_none());
        assert!(config.tasks.is_empty());
        assert!(!config.has_work());
    }

    #[test]
    fn doc_du_ba_viec() {
        let config = read(&[
            ("AUTOXIDE_MIGRATE_ON_START", "true"),
            ("AUTOXIDE_BOOTSTRAP_ADMIN_EMAIL", " admin@example.com "),
            ("AUTOXIDE_STARTUP_TASKS", "address.import, tax.import_pack,,"),
        ])
        .unwrap();

        assert!(config.migrate);
        let admin = config.admin.unwrap();
        assert_eq!(admin.email, "admin@example.com");
        assert!(matches!(admin.password, PasswordSource::Generate));
        assert_eq!(config.tasks, ["address.import", "tax.import_pack"]);
    }

    #[test]
    fn mat_khau_khong_bao_gio_in_ra() {
        let source = PasswordSource::Value("s3cret".to_owned());

        assert_eq!(format!("{source:?}"), "Value(***)");
    }

    #[test]
    fn cau_hinh_mau_thuan_bi_tu_choi() {
        let both = read(&[
            ("AUTOXIDE_BOOTSTRAP_ADMIN_EMAIL", "admin@example.com"),
            ("AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD", "x"),
            ("AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD_FILE", "/run/secrets/x"),
        ])
        .unwrap_err();
        let no_email = read(&[("AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD", "x")]).unwrap_err();
        let bad_task = read(&[("AUTOXIDE_STARTUP_TASKS", "import")]).unwrap_err();

        assert!(both.contains("not both"), "{both}");
        assert!(no_email.contains("set AUTOXIDE_BOOTSTRAP_ADMIN_EMAIL too"), "{no_email}");
        assert!(bad_task.contains("write `<module>.<task>`"), "{bad_task}");
    }
}
