//! Runs the real `acme` binary the way an operator does, to show that a module from outside the
//! Autoxide repository is part of every command: settings, migrations, the OpenAPI document and
//! the MCP tool manifest. On SQLite, or on PostgreSQL when `TEST_DATABASE_URL` says so.

// A file under `tests/` is test code: a failure must stop at once and say where.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::process::{Command, Output};

use autoxide_testkit::TestDatabase;
use serde_json::Value;

/// Runs `acme <arguments>` in an empty directory of its own with exactly this environment, so
/// that no `.env` file or variable of the machine changes the result.
fn acme(label: &str, arguments: &[&str], environment: &[(&str, &str)]) -> Output {
    let directory: PathBuf =
        std::env::temp_dir().join(format!("acme-cli-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    // Cargo sets this variable when it compiles the test: the path of the binary it just built.
    let output = Command::new(env!("CARGO_BIN_EXE_acme"))
        .args(arguments)
        .current_dir(&directory)
        .env_clear()
        .envs(environment.iter().copied())
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&directory);
    output
}

fn stdout(output: &Output) -> String {
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn the_program_has_its_own_name_and_version() {
    let printed = stdout(&acme("version", &["--version"], &[]));
    assert_eq!(printed.trim(), concat!("acme ", env!("CARGO_PKG_VERSION")));
}

#[test]
fn settings_of_the_module_are_listed_with_those_of_the_core() {
    let printed = stdout(&acme("env", &["config", "--env-example"], &[]));
    assert!(printed.contains("ACME_BOOKMARKS_RETENTION_DAYS"), "{printed}");
    assert!(printed.contains("DATABASE_URL"), "{printed}");

    // A wrong value of the module stops every command, with the name of the setting.
    let wrong = acme("wrong", &["config"], &[("ACME_BOOKMARKS_RETENTION_DAYS", "forever")]);
    assert_eq!(wrong.status.code(), Some(2));
    let message = String::from_utf8_lossy(&wrong.stderr).into_owned();
    assert!(message.contains("ACME_BOOKMARKS_RETENTION_DAYS"), "{message}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn migrations_openapi_and_mcp_include_the_module() {
    TestDatabase::run(async |test| {
        let environment = [("DATABASE_URL", test.url())];

        let pending = stdout(&acme("status", &["migrate", "status"], &environment));
        assert!(pending.contains("m0001_create_bookmark"), "{pending}");
        stdout(&acme("up", &["migrate", "up"], &environment));
        // Applied together with the migrations of the official modules: nothing is left to do.
        let applied = stdout(&acme("applied", &["migrate", "status"], &environment));
        assert!(applied.contains(", 0 pending"), "{applied}");

        let openapi: Value =
            serde_json::from_str(&stdout(&acme("openapi", &["openapi"], &environment))).unwrap();
        let create = &openapi["paths"]["/api/v1/bookmarks"]["post"];
        // The core prefixes operation ids with the module name, so they are unique across modules.
        assert_eq!(create["operationId"], "bookmarks_create_bookmark", "{create}");
        // Official modules are still there.
        assert!(openapi["paths"]["/api/v1/auth/me"].is_object());

        let manifest: Value =
            serde_json::from_str(&stdout(&acme("mcp", &["mcp-manifest"], &environment))).unwrap();
        let tools: Vec<&str> = manifest["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert!(tools.contains(&"bookmarks.create_bookmark"), "{tools:?}");
    })
    .await
    .unwrap();
}
