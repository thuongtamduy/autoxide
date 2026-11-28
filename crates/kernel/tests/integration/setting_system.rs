//! Một hệ cài đặt (D-20261128-settings-one-system): luật chéo lúc lưu cho cài đặt cấp khách (REST) và theo cây
//! (đường của module giữ kho), cache giá trị theo cây (trúng cache không đọc kho, ghi thì bỏ cache), và hai lần
//! lưu đồng thời không cùng lọt luật. Chạy trên cả SQLite và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use autoxide_kernel::access::{Authenticator, Principal};
use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig, HttpConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::http;
use autoxide_kernel::module::{Module, Modules, StartContext};
use autoxide_kernel::tenancy::{
    OrgSetting, OrgSettingStore, SettingRule, SettingSystem, StoredOrgSetting, TenantContext,
    TenantHint, TenantSetting, root_of,
};
use autoxide_testkit::TestDatabase;
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt as _;
use uuid::Uuid;

const LOCALES: TenantSetting = TenantSetting::new("rulez.locales", "Languages, comma-separated.");
const DEFAULT_LOCALE: TenantSetting =
    TenantSetting::new("rulez.default_locale", "Default language.");
const LIMIT: OrgSetting = OrgSetting::new("rulez.limit", "Items per order.").with_default("5");
const CAP: OrgSetting = OrgSetting::new("rulez.cap", "Most items per order.").with_default("10");

/// Kho giá trị theo nút giả: giữ trong bộ nhớ, đếm số lần kernel đọc.
#[derive(Default)]
struct Store {
    rows: Mutex<Vec<StoredOrgSetting>>,
    reads: AtomicUsize,
    /// Khi bật: lần đọc chụp giá trị rồi chờ `resume` mới trả (mô phỏng câu đọc chậm).
    hold: std::sync::atomic::AtomicBool,
    resume: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl OrgSettingStore for Store {
    async fn stored(&self, _: &TenantContext) -> Result<Vec<StoredOrgSetting>, Error> {
        let rows = self.rows.lock().unwrap().clone();
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.hold.load(Ordering::SeqCst) {
            self.resume.notified().await;
        }
        Ok(rows)
    }
}

struct Rulez {
    store: Arc<Store>,
    system: Arc<OnceLock<SettingSystem>>,
}

fn problem(message: &'static str) -> FieldError {
    FieldError::new("value", "invalid", message)
}

impl Module for Rulez {
    fn name(&self) -> &'static str {
        "rulez"
    }

    fn tenant_settings(&self) -> Vec<TenantSetting> {
        vec![LOCALES, DEFAULT_LOCALE]
    }

    fn org_settings(&self) -> Vec<OrgSetting> {
        vec![LIMIT, CAP]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let system = context.require::<SettingSystem>()?;
        system.use_org_store(self.store.clone())?;
        // Cấp khách: mặc định đã đặt phải thuộc danh sách (chưa đặt danh sách thì `en`).
        system.rule(SettingRule::new(&[LOCALES.name(), DEFAULT_LOCALE.name()], |view| {
            let Some(default) = view.text(DEFAULT_LOCALE.name()) else { return Ok(()) };
            let locales = view.text(LOCALES.name()).unwrap_or("en");
            if locales.split(',').any(|locale| locale.trim() == default) {
                Ok(())
            } else {
                Err(problem("the default language must be one of the languages"))
            }
        }))?;
        // Theo cây: mức tại nút không vượt trần hiệu lực tại nút đó.
        system.rule(SettingRule::new(&[LIMIT.key(), CAP.key()], |view| {
            let number = |key| view.value(key).and_then(Value::as_i64).unwrap_or(0);
            if number(LIMIT.key()) <= number(CAP.key()) {
                Ok(())
            } else {
                Err(problem("the limit must not be above the cap"))
            }
        }))?;
        let _ = self.system.set(system.as_ref().clone());
        Ok(())
    }
}

struct Admin;

#[async_trait::async_trait]
impl Authenticator for Admin {
    async fn authenticate(&self, _: Option<&str>, _: &TenantHint) -> Result<Principal, Error> {
        Ok(Principal::new(Uuid::nil(), ["kernel.settings.manage".to_owned()]))
    }
}

struct Setup {
    database: Database,
    tenant: TenantContext,
    router: Router,
    store: Arc<Store>,
    system: SettingSystem,
}

async fn start(test: &TestDatabase) -> Setup {
    let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let database = DatabaseConfig::read(&mut reader);
    let http_config = HttpConfig::read(&mut reader);
    let store = Arc::new(Store::default());
    let system = Arc::new(OnceLock::new());
    let module = Rulez { store: store.clone(), system: system.clone() };
    let mut modules = Modules::new(vec![Box::new(module)]).unwrap();
    let configured = modules.configure(&mut reader);
    let (database, http_config) = reader
        .finish(database.and_then(|database| {
            http_config.and_then(|config| configured.map(|()| (database, config)))
        }))
        .unwrap()
        .0;
    let db = Database::connect(&database).await.unwrap();
    modules.migrator().unwrap().up(&db).await.unwrap();
    modules.start(&db).unwrap();
    let tenancy = modules.tenancy().unwrap();
    let tenant = tenancy.default_tenant();
    let extras = http::Extras::default()
        .authenticator(Some(Arc::new(Admin)))
        .translations(modules.translations().clone())
        .tenancy(tenancy);
    let router = http::build(
        &http_config,
        http::health_routes(db.clone(), http::Draining::default()),
        modules.routes(),
        extras,
    )
    .router;
    let system = system.get().unwrap().clone();
    Setup { database: db, tenant, router, store, system }
}

async fn put(router: &Router, name: &str, value: Option<&str>) -> (StatusCode, Value) {
    let (method, body) = match value {
        Some(value) => (Method::PUT, Body::from(json!({"value": value}).to_string())),
        None => (Method::DELETE, Body::empty()),
    };
    let request = Request::builder()
        .method(method)
        .uri(format!("/api/v1/kernel/settings/{name}"))
        .header(header::AUTHORIZATION, "Bearer admin")
        .header(header::CONTENT_TYPE, "application/json")
        .body(body)
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn luat_cheo_cap_khach_chan_luc_luu_va_luon_co_duong_doi() {
    TestDatabase::run(async |test| {
        let setup = start(test).await;
        let router = &setup.router;
        assert_eq!(put(router, "rulez.locales", Some("en,vi")).await.0, StatusCode::OK);
        // Mặc định ngoài danh sách: 422, nói rõ phải sửa gì; không lưu.
        let (status, body) = put(router, "rulez.default_locale", Some("fr")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["code"], "validation_failed");
        assert_eq!(body["errors"][0]["field"], "value", "{body}");
        assert_eq!(put(router, "rulez.default_locale", Some("vi")).await.0, StatusCode::OK);
        // Bỏ `vi` khỏi danh sách khi mặc định là `vi`: 422.
        assert_eq!(
            put(router, "rulez.locales", Some("en")).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        // Đường đổi bộ ngôn ngữ: bỏ mặc định, đổi danh sách, đặt lại mặc định.
        assert_eq!(put(router, "rulez.default_locale", None).await.0, StatusCode::NO_CONTENT);
        assert_eq!(put(router, "rulez.locales", Some("en")).await.0, StatusCode::OK);
        assert_eq!(put(router, "rulez.default_locale", Some("en")).await.0, StatusCode::OK);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cache_theo_cay_trung_thi_khong_doc_kho_ghi_thi_bo() {
    TestDatabase::run(async |test| {
        let setup = start(test).await;
        let (system, tenant, store) = (&setup.system, &setup.tenant, &setup.store);
        let root = root_of(tenant.id());
        let limit = system.effective(tenant, root, LIMIT.key()).await.unwrap().unwrap();
        assert_eq!((limit.value, limit.source), (Some(json!(5)), None));
        assert!(system.effective(tenant, root, "rulez.unknown").await.unwrap().is_none());
        let reads = store.reads.load(Ordering::SeqCst);
        for _ in 0..20 {
            system.effective(tenant, root, CAP.key()).await.unwrap();
        }
        assert_eq!(store.reads.load(Ordering::SeqCst), reads, "reads that hit the cache");
        // Module giữ kho ghi rồi báo: lần đọc sau thấy giá trị mới.
        store.rows.lock().unwrap().push(StoredOrgSetting::new(root, LIMIT.key(), json!(7)));
        system.changed(tenant.id());
        let limit = system.effective(tenant, root, LIMIT.key()).await.unwrap().unwrap();
        assert_eq!(
            (limit.value, limit.source, limit.inherited),
            (Some(json!(7)), Some(root), false)
        );
        assert_eq!(store.reads.load(Ordering::SeqCst), reads + 1);
        let all = system.effective_all(tenant, root).await.unwrap();
        assert_eq!(
            all.iter().map(|setting| setting.key.as_str()).collect::<Vec<_>>(),
            ["rulez.cap", "rulez.limit"]
        );

        // Đọc song song với ghi: sau lần ghi cuối và `changed`, mọi lần đọc thấy giá trị cuối.
        let readers = (0..8).map(|_| {
            let (system, tenant) = (system.clone(), tenant.clone());
            tokio::spawn(async move {
                for _ in 0..25 {
                    system.effective(&tenant, root, LIMIT.key()).await.unwrap();
                }
            })
        });
        let readers: Vec<_> = readers.collect();
        for value in 8..=12 {
            {
                let mut rows = store.rows.lock().unwrap();
                rows.retain(|row| row.key != LIMIT.key());
                rows.push(StoredOrgSetting::new(root, LIMIT.key(), json!(value)));
            }
            system.changed(tenant.id());
            tokio::task::yield_now().await;
        }
        for reader in readers {
            reader.await.unwrap();
        }
        let limit = system.effective(tenant, root, LIMIT.key()).await.unwrap().unwrap();
        assert_eq!(limit.value, Some(json!(12)));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn luat_cheo_theo_cay_tren_duong_cua_module_giu_kho() {
    TestDatabase::run(async |test| {
        let setup = start(test).await;
        let (system, tenant) = (&setup.system, &setup.tenant);
        let root = root_of(tenant.id());
        assert!(system.has_rules(LIMIT.key()) && !system.has_rules("rulez.other"));
        let stored = vec![StoredOrgSetting::new(root, CAP.key(), json!(6))];
        let tx = setup.database.begin_write().await.unwrap();
        system.lock(&tx, tenant).await.unwrap();
        // Trần 6 tại gốc: mức 7 bị chặn, mức 6 được; bỏ mức (về mặc định 5) được.
        let error = system
            .check_org_change(&tx, tenant, root, LIMIT.key(), Some(&json!(7)), stored.clone())
            .await
            .unwrap_err();
        assert_eq!(error.code(), "validation_failed");
        system
            .check_org_change(&tx, tenant, root, LIMIT.key(), Some(&json!(6)), stored.clone())
            .await
            .unwrap();
        system
            .check_org_change(&tx, tenant, root, LIMIT.key(), None, stored.clone())
            .await
            .unwrap();
        // Hạ trần dưới mức đang đặt: bị chặn.
        let with_limit = vec![
            StoredOrgSetting::new(root, CAP.key(), json!(6)),
            StoredOrgSetting::new(root, LIMIT.key(), json!(6)),
        ];
        assert!(
            system
                .check_org_change(&tx, tenant, root, CAP.key(), Some(&json!(4)), with_limit)
                .await
                .is_err()
        );
        tx.rollback().await.unwrap();
    })
    .await
    .unwrap();
}

// Hai lần lưu đồng thời, mỗi lần tự nó hợp lệ, gộp lại thì sai (`vi` bị bỏ khỏi danh sách trong khi `vi` thành mặc
// định): khoá theo khách xếp hai lần lưu nối tiếp, lần sau thấy lần trước đã commit nên bị chặn.
#[tokio::test]
async fn hai_lan_luu_dong_thoi_khong_cung_lot_luat() {
    TestDatabase::run(async |test| {
        let setup = start(test).await;
        let router = &setup.router;
        for _ in 0..5 {
            assert_eq!(put(router, "rulez.default_locale", None).await.0, StatusCode::NO_CONTENT);
            assert_eq!(put(router, "rulez.locales", Some("en,vi")).await.0, StatusCode::OK);
            let (narrow, default) = tokio::join!(
                put(router, "rulez.locales", Some("en")),
                put(router, "rulez.default_locale", Some("vi")),
            );
            let passed =
                [narrow.0, default.0].iter().filter(|status| **status == StatusCode::OK).count();
            assert_eq!(passed, 1, "{narrow:?} {default:?}");
        }
    })
    .await
    .unwrap();
}

// Lần đọc trượt cache chụp giá trị cũ, trong lúc đó module ghi giá trị mới và gọi `changed`: bản cũ không được chèn
// vào cache, lần đọc sau thấy giá trị mới ngay (không phải chờ hết thời gian sống của cache).
#[tokio::test]
async fn doc_cham_khong_de_gia_tri_moi_bang_ban_cu() {
    TestDatabase::run(async |test| {
        let setup = start(test).await;
        let (system, tenant, store) =
            (setup.system.clone(), setup.tenant.clone(), setup.store.clone());
        let root = root_of(tenant.id());
        store.rows.lock().unwrap().push(StoredOrgSetting::new(root, LIMIT.key(), json!(7)));
        system.changed(tenant.id());
        store.hold.store(true, Ordering::SeqCst);
        let reads = store.reads.load(Ordering::SeqCst);
        let slow = {
            let (system, tenant) = (system.clone(), tenant.clone());
            tokio::spawn(async move { system.effective(&tenant, root, LIMIT.key()).await.unwrap() })
        };
        while store.reads.load(Ordering::SeqCst) == reads {
            tokio::task::yield_now().await;
        }
        // Lần đọc chậm đã chụp `7`; giờ ghi `9`.
        {
            let mut rows = store.rows.lock().unwrap();
            rows.clear();
            rows.push(StoredOrgSetting::new(root, LIMIT.key(), json!(9)));
        }
        system.changed(tenant.id());
        store.hold.store(false, Ordering::SeqCst);
        store.resume.notify_one();
        assert_eq!(slow.await.unwrap().unwrap().value, Some(json!(7)));
        let fresh = system.effective(&tenant, root, LIMIT.key()).await.unwrap().unwrap();
        assert_eq!(fresh.value, Some(json!(9)), "the slow read did not put the old value back");
    })
    .await
    .unwrap();
}
