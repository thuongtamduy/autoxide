//! Nơi lưu file riêng của placement (bước 33, "Thứ ngoài bảng"): đặt / bỏ bằng `Placements`, lưu mã
//! hoá, API không bao giờ trả khoá bí mật; khách trên placement đó lưu file ở nơi lưu riêng
//! (`TenantStorage` tự chọn); dời khách sang placement có nơi lưu riêng chép file sang đó và xoá ở
//! nguồn, `keep_source` giữ lại, `purge` xoá bản sao; hai cấu hình cùng một chỗ thì không chép, không
//! xoá. Chạy trên cả SQLite và Postgres (placement là một database thêm cùng loại).

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use autoxide_kernel::event::Actor;
use autoxide_kernel::i18n::Language;
use autoxide_kernel::storage::{
    ByteStream, S3Location, Storage, StorageLocation, StorageOpener, TenantStorage,
};
use autoxide_kernel::tenancy::{Placements, TenantContext, TenantMoves};
use autoxide_testkit::{MemoryStorage, MemoryStorageOpener, TestApp, TestDatabase};
use futures_util::{StreamExt as _, TryStreamExt as _};
use sea_orm::ConnectionTrait as _;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query};

struct World {
    app: TestApp,
    shared: Arc<MemoryStorage>,
    opener: MemoryStorageOpener,
}

async fn world(test: &TestDatabase) -> World {
    let shared = Arc::new(MemoryStorage::new());
    let opener = MemoryStorageOpener::new();
    let shared_api: Arc<dyn Storage> = shared.clone();
    let opener_api: Arc<dyn StorageOpener> = Arc::new(opener.clone());
    let app = TestApp::builder(test)
        // Dời chờ TTL cache + thời gian tối đa của request: test bỏ qua bằng `skip_move_wait`.
        .setting("AUTOXIDE_TENANCY_CACHE_SECONDS", "1")
        .provide::<dyn Storage>(shared_api)
        .provide::<dyn StorageOpener>(opener_api)
        .tenant("a")
        .placement("spare")
        .user("platform", 1, &["kernel.placements.read", "kernel.tenants.move"])
        .build()
        .await
        .unwrap();
    World { app, shared, opener }
}

impl World {
    fn placements(&self) -> Placements {
        self.app.modules().placements(self.app.registry()).unwrap()
    }

    fn moves(&self) -> Arc<TenantMoves> {
        self.app.api::<TenantMoves>().unwrap()
    }

    async fn tenant(&self, slug: &str) -> TenantContext {
        self.app.registry().invalidate(self.app.tenant(slug).await.unwrap().id());
        self.app.tenant(slug).await.unwrap()
    }

    /// Nơi lưu của khách `tenant`, như module dùng.
    fn store(&self, tenant: &TenantContext) -> TenantStorage {
        TenantStorage::new(self.shared.clone(), tenant)
    }

    async fn move_to(&self, to: &str, keep_source: bool) {
        let moves = self.moves();
        let started = moves.start(Actor::system(), "a", to, keep_source, false).await.unwrap();
        self.app.skip_move_wait().await.unwrap();
        assert_eq!(moves.run(started.id, "test").await.unwrap().state, "done");
    }
}

fn body(text: &'static str) -> ByteStream {
    futures_util::stream::iter([Ok(bytes::Bytes::from_static(text.as_bytes()))]).boxed()
}

async fn read(storage: &dyn Storage, key: &str) -> Option<String> {
    let file = storage.get(key).await.unwrap()?;
    let chunks: Vec<bytes::Bytes> = file.content.try_collect().await.unwrap();
    Some(String::from_utf8(chunks.concat()).unwrap())
}

fn local(dir: &str) -> Option<StorageLocation> {
    Some(StorageLocation::Local { dir: dir.to_owned() })
}

#[tokio::test]
async fn noi_luu_rieng_luu_ma_hoa_va_khong_bao_gio_lo_khoa() {
    TestDatabase::run(async |test| {
        let world = world(test).await;
        let placements = world.placements();
        let s3 = StorageLocation::S3(S3Location {
            bucket: "spare-files".to_owned(),
            prefix: Some("eu".to_owned()),
            region: "eu-west-1".to_owned(),
            endpoint: Some("https://s3.example.test".to_owned()),
            access_key_id: Some("KEY-ID-1234".to_owned()),
            secret_access_key: Some("SECRET-VALUE-5678".to_owned()),
        });
        let placement = placements.set_storage("spare", Some(s3), false).await.unwrap();
        let storage = placement.storage.unwrap();
        assert_eq!(
            (storage.kind.as_str(), storage.bucket.as_deref(), storage.prefix.as_deref()),
            ("s3", Some("spare-files"), Some("eu"))
        );
        assert!(storage.credentials);

        // Lưu mã hoá; danh sách qua REST có phần không bí mật, không có khoá.
        let select = Query::select()
            .column(Alias::new("storage_sealed"))
            .from(Alias::new("kernel_placements"))
            .and_where(Expr::col(Alias::new("name")).eq("spare"))
            .to_owned();
        let row = world.app.database().read().query_one(&select).await.unwrap().unwrap();
        let sealed: String = row.try_get("", "storage_sealed").unwrap();
        assert!(sealed.starts_with("sealed:"), "{sealed}");
        assert!(!sealed.contains("SECRET") && !sealed.contains("spare-files"));
        let answer = world.app.get("/api/v1/kernel/placements").token("platform").send().await;
        let answer = answer.unwrap();
        assert_eq!(answer.status(), 200);
        let text = answer.text();
        assert!(!text.contains("SECRET-VALUE-5678") && !text.contains("KEY-ID-1234"), "{text}");
        let listed = answer.json()["data"].as_array().unwrap().clone();
        let spare = listed.iter().find(|placement| placement["name"] == "spare").unwrap();
        assert_eq!(spare["storage"]["endpoint"], "https://s3.example.test");
        let main = listed.iter().find(|placement| placement["name"] == "main").unwrap();
        assert_eq!(main["storage"], serde_json::Value::Null);

        // Sai: `main`, placement không có, giá trị không hợp lệ (báo mọi trường một lần).
        let error = placements.set_storage("main", local("x"), false).await.unwrap_err();
        assert_eq!(error.fields()[0].field, "placement");
        let error = placements.set_storage("nope", local("x"), false).await.unwrap_err();
        assert_eq!(error.code(), "tenant_placement_not_found");
        let bad = StorageLocation::S3(S3Location {
            bucket: "No".to_owned(),
            prefix: None,
            region: " ".to_owned(),
            endpoint: Some("ftp://x".to_owned()),
            access_key_id: None,
            secret_access_key: None,
        });
        let error = placements.set_storage("spare", Some(bad), false).await.unwrap_err();
        let fields: Vec<&str> = error.fields().iter().map(|field| field.field.as_str()).collect();
        assert_eq!(fields, ["s3_bucket", "s3_region", "s3_endpoint"]);

        // Bỏ: về nơi lưu chung.
        let cleared = placements.set_storage("spare", None, false).await.unwrap();
        assert!(cleared.storage.is_none());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doi_khach_sang_placement_co_noi_luu_rieng_chep_file() {
    TestDatabase::run(async |test| {
        let world = world(test).await;
        world.placements().set_storage("spare", local("spare-dir"), false).await.unwrap();
        let spare = world.opener.store("spare-dir");

        // Khách `a` trên `main`: file ở nơi lưu chung, dưới tiền tố của khách.
        let a = world.tenant("a").await;
        assert!(a.own_storage().is_none());
        world.store(&a).put("files/one", body("one")).await.unwrap();
        let key = format!("tenants/{}/files/one", a.id());
        assert!(world.shared.bytes(&key).is_some());

        // Kế hoạch đếm file sẽ chép.
        let plan = world.moves().plan("a", "spare", false, Language::En).await.unwrap();
        assert!(plan.ready, "{plan:?}");
        assert_eq!(plan.files, 1);

        world.move_to("spare", false).await;
        let a = world.tenant("a").await;
        assert!(a.own_storage().is_some());
        assert_eq!(a.storage_placement(), Some(a.placement()));
        assert_eq!(read(&world.store(&a), "files/one").await.as_deref(), Some("one"));
        assert!(spare.bytes(&key).is_some());
        assert!(world.shared.bytes(&key).is_none(), "the source copy is deleted");
        // File mới của khách vào nơi lưu riêng.
        world.store(&a).put("files/two", body("two")).await.unwrap();
        let two = format!("tenants/{}/files/two", a.id());
        assert!(spare.bytes(&two).is_some() && world.shared.bytes(&two).is_none());

        // Placement đang có khách: đổi nơi lưu cần `force`.
        let error =
            world.placements().set_storage("spare", local("other"), false).await.unwrap_err();
        assert_eq!(error.code(), "tenant_placement_storage_in_use");

        // Dời về `main`, giữ bản sao: file chép về nơi lưu chung, nơi lưu riêng còn nguyên; xoá bản
        // sao thì xoá cả file ở nơi lưu riêng.
        let back = world.moves().plan("a", "main", true, Language::En).await.unwrap();
        assert_eq!(back.files, 2);
        world.move_to("main", true).await;
        let a = world.tenant("a").await;
        assert!(a.own_storage().is_none());
        assert_eq!(read(&world.store(&a), "files/two").await.as_deref(), Some("two"));
        assert!(spare.bytes(&key).is_some(), "keep_source keeps the files");
        world.moves().purge("a", "spare").await.unwrap();
        assert!(spare.keys_with_prefix(&format!("tenants/{}/", a.id())).await.unwrap().is_empty());
        assert!(world.shared.bytes(&key).is_some() && world.shared.bytes(&two).is_some());
    })
    .await
    .unwrap();
}

/// Nơi lưu riêng cấu hình khác nhưng trỏ đúng nơi lưu chung: dời không chép, và nhất là không xoá
/// file đang dùng ở "nguồn".
#[tokio::test]
async fn noi_luu_rieng_trung_noi_luu_chung_thi_khong_xoa() {
    TestDatabase::run(async |test| {
        let world = world(test).await;
        world.opener.alias("same-as-shared", world.shared.clone());
        world.placements().set_storage("spare", local("same-as-shared"), false).await.unwrap();
        let a = world.tenant("a").await;
        world.store(&a).put("files/one", body("one")).await.unwrap();
        let key = format!("tenants/{}/files/one", a.id());

        world.move_to("spare", false).await;
        let a = world.tenant("a").await;
        assert_eq!(read(&world.store(&a), "files/one").await.as_deref(), Some("one"));
        assert!(world.shared.bytes(&key).is_some());
        // Không còn file đánh dấu nào.
        let left = world.shared.keys_with_prefix(&format!("tenants/{}/", a.id())).await.unwrap();
        assert_eq!(left, [key]);
    })
    .await
    .unwrap();
}
