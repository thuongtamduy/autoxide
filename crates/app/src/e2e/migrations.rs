//! Migration thật của mọi module chạy được vòng up → down → up, trên database trống và trên database
//! có dữ liệu, trên cả hai database (đặc tả 31 B2). Trước đây chỉ có test cơ chế bằng bảng mẫu
//! (`crates/kernel/tests/migration.rs`) và một vòng không so schema.

use autoxide_kernel::config::ConfigSource;
use autoxide_kernel::database::Database;
use autoxide_kernel::migration::Migrator;
use autoxide_kernel::module::{Module, Modules};
use autoxide_testkit::{TestDatabase, schema_snapshot};
use serde_json::json;

use crate::e2e::media::{admin_token, call, start};

/// Mọi module của binary cộng module mẫu `notes` (không đăng ký mặc định).
fn modules() -> Modules {
    let mut all: Vec<Box<dyn Module>> = crate::modules::all();
    all.push(Box::new(autoxide_notes::module()));
    Modules::new(all).unwrap()
}

async fn connect(url: &str) -> Database {
    let source = ConfigSource::from_pairs([("DATABASE_URL", url)]);
    let settings = crate::settings::read(&source).unwrap();
    Database::connect(&settings.config.database).await.unwrap()
}

/// Bảng còn lại sau khi gỡ hết: chỉ bảng lịch sử `<module>_migrations` (sea-orm-migration không xoá
/// bảng lịch sử của chính nó).
async fn tables_after_full_down(database: &Database) -> Vec<String> {
    let snapshot = schema_snapshot(database.write()).await.unwrap();
    snapshot
        .table_names()
        .into_iter()
        .filter(|name| !name.ends_with("_migrations"))
        .map(str::to_owned)
        .collect()
}

async fn down_all(migrator: &Migrator, database: &Database, names: &[&'static str]) {
    for name in names.iter().rev() {
        migrator
            .down(database, name, u32::MAX)
            .await
            .unwrap_or_else(|error| panic!("down of module `{name}` failed: {error}"));
    }
}

#[tokio::test]
async fn moi_module_up_down_up_tren_database_trong() {
    TestDatabase::run(async |test| {
        let modules = modules();
        let names = modules.names();
        let migrator = modules.migrator().unwrap();
        let database = connect(test.url()).await;

        migrator.up(&database).await.unwrap();
        let first = schema_snapshot(database.write()).await.unwrap();
        down_all(&migrator, &database, &names).await;
        assert_eq!(tables_after_full_down(&database).await, Vec::<String>::new());
        migrator.up(&database).await.unwrap();
        let second = schema_snapshot(database.write()).await.unwrap();
        assert_eq!(first, second, "the schema after down → up differs from the first up");
        database.close().await.unwrap();
    })
    .await
    .unwrap();
}

/// Ghi dữ liệu mẫu của nhiều module qua REST (chương trình thật, đăng nhập thật).
async fn sample_data(url: &str) {
    let dir = std::env::temp_dir().join(format!("autoxide-migrations-{}", uuid::Uuid::now_v7()));
    let storage = [("AUTOXIDE_FILES_DIR", dir.display().to_string())];
    let runtime = start(url, &storage).await;
    let token = admin_token(&runtime, "admin@example.com").await;
    let auth = format!("Bearer {token}");
    let json_headers = [("authorization", auth.as_str()), ("content-type", "application/json")];
    let writes = [
        ("/api/v1/content/entries", json!({"kind": "post", "title": "Hello"})),
        ("/api/v1/content/terms", json!({"taxonomy": "category", "name": "Travel"})),
        ("/api/v1/menus", json!({"key": "main", "items": [{"label": "Home", "url": "/"}]})),
        ("/api/v1/newsletter/lists", json!({"key": "news", "name": {"en": "News"}})),
        ("/api/v1/webhooks", json!({"url": "https://example.com/hook", "events": ["*"]})),
        ("/api/v1/auth/roles", json!({"name": "editor", "permissions": ["content.read"]})),
    ];
    for (path, body) in writes {
        let (status, _, answer) =
            call(&runtime, "POST", path, &json_headers, body.to_string().into_bytes()).await;
        assert!(status.is_success(), "POST {path}: {status} {answer}");
    }
    let png = crate::e2e::media::png(40, 30);
    let media_headers = [("authorization", auth.as_str()), ("content-type", "image/png")];
    let (status, _, answer) =
        call(&runtime, "POST", "/api/v1/media?name=a.png", &media_headers, png).await;
    assert!(status.is_success(), "upload: {status} {answer}");
    runtime.database.close().await.unwrap();
}

#[tokio::test]
async fn moi_module_down_mot_buoc_roi_up_khi_co_du_lieu() {
    TestDatabase::run(async |test| {
        sample_data(test.url()).await;
        let modules = modules();
        let migrator = modules.migrator().unwrap();
        let database = connect(test.url()).await;
        // `notes` chưa được binary migrate: đưa lên trước.
        migrator.up(&database).await.unwrap();
        for name in modules.names().iter().rev() {
            migrator
                .down(&database, name, 1)
                .await
                .unwrap_or_else(|error| panic!("down 1 of `{name}` with data failed: {error}"));
            migrator
                .up(&database)
                .await
                .unwrap_or_else(|error| panic!("up of `{name}` after down 1 failed: {error}"));
        }
        database.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn down_het_roi_up_khi_co_du_lieu() {
    TestDatabase::run(async |test| {
        sample_data(test.url()).await;
        let modules = modules();
        let names = modules.names();
        let migrator = modules.migrator().unwrap();
        let database = connect(test.url()).await;
        migrator.up(&database).await.unwrap();
        down_all(&migrator, &database, &names).await;
        assert_eq!(tables_after_full_down(&database).await, Vec::<String>::new());
        migrator.up(&database).await.unwrap();
        assert!(migrator.pending(&database).await.unwrap().is_empty());
        database.close().await.unwrap();
    })
    .await
    .unwrap();
}
