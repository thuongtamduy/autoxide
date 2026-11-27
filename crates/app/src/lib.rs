//! Crate `autoxide`: the server and command-line interface (CLI), usable as a LIBRARY.
//!
//! This is the ONLY place that wires the kernel together with the modules into one program. The official binary
//! (`main.rs`) only calls [`Autoxide::new`] and then [`Autoxide::run`]. Authors of modules in their own repo
//! write their binary the same way, adding `.module(…)` for each of their modules — no need to fork
//! this repo (step 70 P2, D-20261121-external-modules). Guide: `docs/guides/writing-a-module.md`.
//!
//! The public surface of this crate is ONLY [`Autoxide`]: everything else (commands, configuration, startup) is
//! private, so it can change without breaking anyone else's binary.

mod commands;
#[cfg(test)]
mod e2e;
mod modules;
mod observability;
mod registry;
mod releases;
mod runtime;
mod settings;
mod startup;

use std::ffi::OsString;
use std::process::ExitCode;

use autoxide_kernel::module::Module;
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};

/// Autoxide — a Rust backend core. Modules plug in on top.
// `version` lấy từ Cargo.toml lúc biên dịch; phần mô tả lấy từ dòng doc phía trên.
#[derive(Debug, Parser)]
#[command(name = "autoxide", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Check the AI settings against the provider (`autoxide ai check`).
    ///
    /// Sends one tiny request to the configured AI provider and embeddings service and says
    /// whether they work. Uses a few tokens.
    Ai(commands::ai::Arguments),

    /// Back up the data of every module and every stored file into a directory.
    ///
    /// Works the same for SQLite and PostgreSQL, local disk and S3: the backup holds a
    /// SQLite copy of the database (read in one consistent snapshot, so the server may
    /// keep running) and a copy of every stored file. Restore it with
    /// `autoxide restore`.
    Backup(commands::backup::BackupArguments),

    /// Try Autoxide in one command: a local server with sample data and an administrator.
    ///
    /// Needs no setup: creates a directory (default `autoxide-demo`), a SQLite database, random
    /// secrets and an administrator whose password is printed on screen, loads the reference
    /// data (addresses, taxes, promotion limits) and serves the API with its reference page
    /// at `/api/docs`. Run it again to continue with the same data. Not for production.
    Demo(commands::demo::Arguments),

    /// Check the configuration and show every setting with its current value.
    ///
    /// Settings are read from environment variables and from a `.env` file in
    /// the current directory; environment variables win. Secrets are never
    /// printed. Exits with status 2 when the configuration is invalid.
    Config(commands::config::Arguments),

    /// Copy the data of every module into another, empty database.
    ///
    /// Copies from this server's database (`DATABASE_URL`) to the target — SQLite to
    /// PostgreSQL or back. The target gets every migration first, must hold no data,
    /// and receives everything in one transaction. Stop the server first so that no
    /// change is missed.
    Copy(commands::copy::Arguments),

    /// Check this installation step by step and say how to fix what is wrong.
    ///
    /// Checks the configuration, the database (reading and writing), the migrations, the
    /// signing key, then what each module declares: file storage, the SMTP server, whether
    /// an administrator exists… Changes nothing (a test file written to the storage is
    /// deleted again). Exits with status 1 when a check failed, 2 when the configuration is
    /// invalid; warnings do not fail.
    Doctor(commands::doctor::Arguments),

    /// Ask the server running on this machine whether it is healthy (for container health checks).
    ///
    /// Sends `GET /health/ready` (or `/health/live` with `--live`) to the address of
    /// `AUTOXIDE_HTTP_ADDRESS` over plain HTTP (it reads only that one setting, so it needs no other configuration), and
    /// exits with status 0 when it answers 200, 1 when it does not answer or answers anything else,
    /// 2 when the configuration is invalid. It needs no shell, `curl` or `wget`, so it works in an
    /// image that has none. It does not touch the database itself.
    Healthcheck(commands::healthcheck::Arguments),

    /// Print the generated password of the first administrator, or delete the file that holds it.
    ///
    /// `autoxide serve` writes the email and the generated password to the file named by
    /// `AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD_OUT` (default `data/initial-admin-password`, readable only by the
    /// server's user) when it creates the first administrator without a password you gave. This prints that
    /// file (the email, then the password) and, with `--remove`, deletes it once you have changed the
    /// password. It exists because the Docker image has no shell, `cat` or `rm`. Exits with status 1 when the
    /// file does not exist, 2 when the configuration is invalid. The password never goes to the log.
    InitialPassword(commands::initial_password::Arguments),

    /// Inspect and retry background jobs that failed for good (the dead-letter queue).
    Jobs(commands::jobs::Arguments),

    /// Show, apply or revert the database migrations of the modules.
    ///
    /// Run `autoxide migrate up` once per deployment, before starting the servers. (For one
    /// server or a first try, AUTOXIDE_MIGRATE_ON_START=true makes `autoxide serve` apply them
    /// on start.) Only one process applies migrations at a time: the others wait for it.
    Migrate(commands::migrate::Arguments),

    /// List every module of this program: tier, on or off, what it depends on, leftover data.
    ///
    /// Shows the official modules and the ones this binary added, as chosen by AUTOXIDE_MODULES.
    /// For a module that is turned off it says whether the database still holds its data. Reads
    /// only; `--json` prints the same for programs and agents.
    Modules(commands::modules::Arguments),

    /// Print the OpenAPI document of the whole API as JSON.
    ///
    /// The same document is served at `/api/v1/openapi.json`. Connects to the
    /// database (modules build their services) but does not need migrations.
    Openapi,

    /// Print the contract of every MCP tool (name, permissions, input schema, deprecation) as JSON.
    ///
    /// Kept in `docs/mcp-tools.json`; `cargo xtask api-diff` refuses breaking changes to it.
    /// Connects to the database (modules build their services) but does not need migrations.
    McpManifest,

    /// Manage the placements: the databases that keep tenants' data (multi-tenant mode).
    ///
    /// `add` stores the connection URL encrypted; `check` compares each placement's migrations
    /// with the program. Run `autoxide migrate up --all` to bring placements up to date.
    Placement(commands::placement::Arguments),

    /// Roll the main database and every tenant database back to the migrations of an earlier release.
    ///
    /// Run it with the binary of the CURRENT release, then start the servers of the earlier one.
    /// `--dry-run` lists, per database, what would be reverted and the tables and columns that
    /// would be dropped with the rows they hold. A rollback that drops data needs
    /// `--accept-data-loss`; every real run needs `--yes` once the servers are stopped. Modules
    /// this binary adds with `.module(…)` are left as they are unless `--revert-untracked`.
    Rollback(commands::rollback::Arguments),

    /// Restore a backup made by `autoxide backup` into a new, empty database.
    ///
    /// Restores into the database of `DATABASE_URL` (which gets every migration first)
    /// and the file storage configured now — they may differ from the backed-up server.
    Restore(commands::backup::RestoreArguments),

    /// Run the HTTP server until it receives Ctrl-C or SIGTERM.
    ///
    /// Refuses to start while a migration has not been applied: run `autoxide migrate up`
    /// first, or set AUTOXIDE_MIGRATE_ON_START=true. On start it can also create the first
    /// administrator (AUTOXIDE_BOOTSTRAP_ADMIN_EMAIL) and run maintenance tasks
    /// (AUTOXIDE_STARTUP_TASKS). On shutdown, it first keeps handling requests for
    /// `AUTOXIDE_HTTP_DRAIN_SECONDS` while `/health/ready` answers 503, then stops accepting
    /// connections and gives requests in progress up to `AUTOXIDE_HTTP_SHUTDOWN_SECONDS` to
    /// finish.
    Serve,

    /// List or run the maintenance tasks that modules declare.
    ///
    /// `autoxide task` lists every task with its arguments; `autoxide task <module>.<task>
    /// [name=value …]` runs one, e.g. `autoxide task media.refresh_variants dry_run=true`.
    /// Runs with the rights of whoever operates the server. Exits with status 64 when the task
    /// or an argument is wrong, 1 when the task fails. Run `autoxide migrate up` first.
    Task(commands::task::Arguments),

    /// Encrypt stored secrets again with the current key (after rotating AUTOXIDE_SECRET_KEY).
    ///
    /// `autoxide secrets reseal` runs the `reseal_secrets` task of every module that keeps
    /// encrypted secrets. Run `autoxide migrate up` first.
    Secrets(commands::secrets::Arguments),

    /// Manage the tenants (customers) of the installation: create, list, show, suspend, resume.
    ///
    /// Runs with the rights of whoever operates the server. Creating tenants needs
    /// AUTOXIDE_TENANCY_MODE=multi.
    Tenant(commands::tenant::Arguments),

    /// Manage user accounts, e.g. create the first administrator.
    ///
    /// Runs with the rights of whoever operates the server: no sign-in needed.
    User(commands::user::Arguments),
}

/// An Autoxide program: the official modules plus the ones you add, behind the standard
/// command line (`serve`, `migrate`, `doctor`, `openapi`, `mcp-manifest`, `config`…).
///
/// The official binary is exactly `Autoxide::new().run()`. To ship your own modules, write
/// a binary of your own and add them:
///
/// ```no_run
/// # use autoxide_kernel::module::Module;
/// # struct Bookmarks;
/// # impl Module for Bookmarks {
/// #     fn name(&self) -> &'static str {
/// #         "bookmarks"
/// #     }
/// # }
/// # fn module() -> impl Module {
/// #     Bookmarks
/// # }
/// fn main() -> std::process::ExitCode {
///     autoxide::Autoxide::new()
///         .name(env!("CARGO_PKG_NAME"))
///         .version(env!("CARGO_PKG_VERSION"))
///         .module(module) // `pub fn module() -> impl Module` of your crate
///         .run()
/// }
/// ```
///
/// Added modules get everything a built-in module gets: migrations, permissions, events,
/// jobs, routes in the OpenAPI document, MCP tools, translations and settings. They are
/// registered after the official ones, in the order added, so they can `require` any trait
/// an official module provides.
#[must_use = "call `.run()` to parse the command line and run the program"]
pub struct Autoxide {
    name: &'static str,
    version: &'static str,
    extra: Vec<registry::Factory>,
}

impl Default for Autoxide {
    fn default() -> Self {
        Self::new()
    }
}

impl Autoxide {
    /// The official program: every official module, named `autoxide`, with the version of
    /// this crate.
    pub fn new() -> Self {
        Self { name: "autoxide", version: env!("CARGO_PKG_VERSION"), extra: Vec::new() }
    }

    /// The program name shown by `--help` and `--version` (default `autoxide`).
    pub fn name(mut self, name: &'static str) -> Self {
        self.name = name;
        self
    }

    /// The version shown by `--version` (default: the version of the `autoxide` crate).
    pub fn version(mut self, version: &'static str) -> Self {
        self.version = version;
        self
    }

    /// Adds a module. `factory` builds a new instance each time it is called (the program
    /// may build the module list more than once); by convention pass the `module` function
    /// of the module's crate. Two modules with the same name are a startup error.
    // Nhận hàm tạo thay vì giá trị: `Module` không `Clone`, mà chương trình dựng danh sách
    // module nhiều hơn một lần (vd một lần để liệt kê cấu hình, một lần để chạy).
    pub fn module<M>(mut self, factory: impl Fn() -> M + Send + Sync + 'static) -> Self
    where
        M: Module + 'static,
    {
        self.extra.push(Box::new(move || Box::new(factory())));
        self
    }

    /// Parses the command line of the process and runs the chosen command to its end.
    /// Return the result from `main`.
    pub fn run(self) -> ExitCode {
        self.run_from(std::env::args_os())
    }

    /// Like [`Autoxide::run`], with the given arguments (the first is the program name)
    /// instead of those of the process.
    pub fn run_from<I, T>(self, arguments: I) -> ExitCode
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        // `get_matches_from` tự xử lý `--help`, `--version` và tham số sai (in rồi thoát).
        let matches =
            Cli::command().name(self.name).version(self.version).get_matches_from(arguments);
        let cli = match Cli::from_arg_matches(&matches) {
            Ok(cli) => cli,
            Err(error) => error.exit(),
        };
        if let Err(message) = registry::install(self.extra) {
            eprintln!("{}: {message}", self.name);
            return ExitCode::FAILURE;
        }
        dispatch(cli.command)
    }
}

fn dispatch(command: Command) -> ExitCode {
    match command {
        Command::Ai(arguments) => commands::ai::run(&arguments),
        Command::Backup(arguments) => commands::backup::backup(&arguments),
        Command::Config(arguments) => commands::config::run(&arguments),
        Command::Copy(arguments) => commands::copy::run(&arguments),
        Command::Demo(arguments) => commands::demo::run(&arguments),
        Command::Doctor(arguments) => commands::doctor::run(&arguments),
        Command::Healthcheck(arguments) => commands::healthcheck::run(&arguments),
        Command::InitialPassword(arguments) => commands::initial_password::run(&arguments),
        Command::Jobs(arguments) => commands::jobs::run(&arguments),
        Command::Migrate(arguments) => commands::migrate::run(&arguments),
        Command::Rollback(arguments) => commands::rollback::run(&arguments),
        Command::Modules(arguments) => commands::modules::run(&arguments),
        Command::Openapi => commands::openapi::run(),
        Command::McpManifest => commands::mcp_manifest::run(),
        Command::Placement(arguments) => commands::placement::run(&arguments),
        Command::Restore(arguments) => commands::backup::restore(&arguments),
        Command::Secrets(arguments) => commands::secrets::run(&arguments),
        Command::Serve => commands::serve::run(),
        Command::Task(arguments) => commands::task::run(&arguments),
        Command::Tenant(arguments) => commands::tenant::run(&arguments),
        Command::User(arguments) => commands::user::run(&arguments),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use clap::error::ErrorKind;

    #[test]
    fn khai_bao_cli_hop_le() {
        // clap tự kiểm các khai báo mâu thuẫn (trùng tên tham số, thiếu giá trị mặc định…).
        Cli::command().debug_assert();
    }

    #[test]
    fn version_lay_tu_cargo_toml() {
        let error = Cli::try_parse_from(["autoxide", "--version"]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::DisplayVersion);
        assert_eq!(error.to_string().trim(), concat!("autoxide ", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn tham_so_la_bi_tu_choi() {
        let error = Cli::try_parse_from(["autoxide", "--no-such-flag"]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn khong_co_lenh_thi_in_huong_dan() {
        let error = Cli::try_parse_from(["autoxide"]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand);
    }
}
