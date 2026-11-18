//! Thứ ngoài bảng cũng theo khách (bước 33 A8): khoá file có tiền tố khách, khoá ký riêng mỗi
//! khách (khách mặc định giữ khoá cũ), token form của khách A bị từ chối ở khách B.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use autoxide_kernel::antispam::{AntispamConfig, SpamGuard, Submission};
use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::http::ClientInfo;
use autoxide_kernel::module::Modules;
use autoxide_kernel::secret::SecretKey;
use autoxide_kernel::storage::{Storage, TenantStorage};
use autoxide_kernel::tenancy::{
    DEFAULT_TENANT, MAIN_PLACEMENT, TenantContext, TenantId, TenantStatus,
};
use autoxide_testkit::{MemoryStorage, TestDatabase};
use bytes::Bytes;
use futures_util::{StreamExt as _, TryStreamExt as _};

async fn database(test: &TestDatabase) -> Database {
    let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    let database = Database::connect(&reader.finish(config).unwrap().0).await.unwrap();
    Modules::new(Vec::new()).unwrap().migrator().unwrap().up(&database).await.unwrap();
    database
}

fn other(database: &Database) -> TenantContext {
    TenantContext::new(
        TenantId::from_uuid(uuid::Uuid::now_v7()),
        "other",
        "Other",
        TenantStatus::Active,
        MAIN_PLACEMENT,
        database.clone(),
    )
}

fn body(text: &'static str) -> autoxide_kernel::storage::ByteStream {
    futures_util::stream::iter([Ok(Bytes::from_static(text.as_bytes()))]).boxed()
}

async fn read(storage: &dyn Storage, key: &str) -> Option<String> {
    let file = storage.get(key).await.unwrap()?;
    let bytes: Vec<Bytes> = file.content.try_collect().await.unwrap();
    Some(String::from_utf8(bytes.concat()).unwrap())
}

#[tokio::test]
async fn khoa_file_co_tien_to_khach() {
    TestDatabase::run(async |test| {
        let database = database(test).await;
        let shared = Arc::new(MemoryStorage::new());
        let ours = TenantStorage::new(shared.clone(), &TenantContext::default_on(database.clone()));
        let theirs_context = other(&database);
        let theirs = TenantStorage::new(shared.clone(), &theirs_context);

        ours.put("notes/a.txt", body("ours")).await.unwrap();
        theirs.put("notes/a.txt", body("theirs")).await.unwrap();
        // Khách mặc định: không tiền tố (file cũ giữ nguyên chỗ); khách khác: `tenants/<id>/`.
        assert!(shared.bytes("notes/a.txt").is_some());
        assert!(shared.bytes(&format!("tenants/{}/notes/a.txt", theirs_context.id())).is_some());
        assert_eq!(read(&ours, "notes/a.txt").await.as_deref(), Some("ours"));
        assert_eq!(read(&theirs, "notes/a.txt").await.as_deref(), Some("theirs"));
        // Liệt kê chỉ thấy file của mình, khoá đã bỏ tiền tố.
        assert_eq!(ours.keys().await.unwrap(), ["notes/a.txt"]);
        assert_eq!(theirs.keys_with_prefix("notes/").await.unwrap(), ["notes/a.txt"]);
        // Không với ra ngoài phần của mình.
        for bad in ["../x", "notes/../../x", "/etc/passwd", "a//b", "./a", ""] {
            assert!(theirs.get(bad).await.is_err(), "{bad}");
        }
        theirs.delete("notes/a.txt").await.unwrap();
        assert_eq!(read(&ours, "notes/a.txt").await.as_deref(), Some("ours"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn token_ky_cua_khach_a_bi_tu_choi_o_khach_b() {
    TestDatabase::run(async |test| {
        let database = database(test).await;
        let secret = SecretKey::new(database.clone(), None);
        // Khách mặc định giữ đúng khoá cũ; khách khác có khoá riêng.
        let purpose = "autoxide/demo/v1";
        let other = other(&database);
        assert_eq!(
            secret.derive_for(DEFAULT_TENANT, purpose).await.unwrap(),
            secret.derive(purpose).await.unwrap()
        );
        assert_ne!(
            secret.derive_for(other.id(), purpose).await.unwrap(),
            secret.derive(purpose).await.unwrap()
        );

        let config = AntispamConfig::default()
            .min_fill(std::time::Duration::ZERO)
            .challenge(autoxide_kernel::antispam::ChallengeKind::None);
        let guard = SpamGuard::new(secret, config);
        let ours = TenantContext::default_on(database.clone());
        let issued = guard.issue(&ours, "forms:contact").await.unwrap();
        let client = ClientInfo::from_ip("203.0.113.9".parse().unwrap());
        let submission = Submission::new("forms:contact", &client).form_token(Some(&issued.token));
        let limits = autoxide_kernel::antispam::Limits::default();
        assert!(guard.check(&ours, &submission, &limits).await.is_ok());
        let refused = guard.check(&other, &submission, &limits).await.unwrap_err();
        assert_eq!(refused.code(), "antispam.token_invalid");
    })
    .await
    .unwrap();
}
