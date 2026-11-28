//! `StartContext::contribute::<dyn Trait>` là một đường cắm vào điểm mở rộng của kernel (D-20261128-contribute): seed
//! cho khách mới, dữ liệu mẫu, dữ liệu cá nhân… đóng góp trong `start` đến đúng registry như hàm đăng ký cũ; module làm
//! cả hai cách cho một khái niệm thì khởi động lỗi. Chạy trên cả SQLite và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::module::{Manifest, Module, Modules, StartContext};
use autoxide_kernel::privacy::PersonalData;
use autoxide_kernel::tenancy::sandbox::{CopyContext, SandboxCopy};
use autoxide_kernel::tenancy::{
    DatabaseRegistry, NewTenant, Relocation, RelocationSwitch, SampleData, SwitchHook,
    TenancyConfig, TenancyMode, TenantAdmin, TenantContext, TenantId, TenantSeed,
};
use autoxide_testkit::TestDatabase;
use serde_json::{Value, json};
use uuid::Uuid;

/// Đếm số lần seed / dữ liệu mẫu chạy.
#[derive(Default)]
struct Counter {
    seeds: AtomicUsize,
    samples: AtomicUsize,
}

#[async_trait::async_trait]
impl TenantSeed for Counter {
    async fn seed(&self, _tenant: &TenantContext) -> Result<(), Error> {
        self.seeds.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[async_trait::async_trait]
impl SampleData for Counter {
    async fn sample(&self, _tenant: &TenantContext) -> Result<(), Error> {
        self.samples.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[async_trait::async_trait]
impl PersonalData for Counter {
    async fn export(&self, _tenant: &TenantContext, _user: Uuid) -> Result<Value, Error> {
        Ok(json!({"plants": 0}))
    }
}

struct NoCopy;

#[async_trait::async_trait]
impl SandboxCopy for NoCopy {
    async fn copy(&self, _context: &CopyContext<'_>) -> Result<u64, Error> {
        Ok(0)
    }
}

/// Module kiểu mới: mọi điểm mở rộng đóng góp bằng `contribute`.
struct Garden(Arc<Counter>);

impl Module for Garden {
    fn manifest(&self) -> Manifest {
        Manifest::new("garden", "Plants.")
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.contribute::<dyn TenantSeed>(self.0.clone())?;
        context.contribute::<dyn SampleData>(self.0.clone())?;
        context.contribute::<dyn PersonalData>(self.0.clone())?;
        context.contribute::<dyn SandboxCopy>(Arc::new(NoCopy))
    }
}

/// Module kiểu cũ khai seed bằng `Module::seed_tenant` và còn đóng góp `dyn TenantSeed`.
struct TwiceSeed;

impl Module for TwiceSeed {
    fn name(&self) -> &'static str {
        "twice_seed"
    }

    fn seed_tenant(&self) -> Option<Arc<dyn TenantSeed>> {
        Some(Arc::new(Counter::default()))
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.contribute::<dyn TenantSeed>(Arc::new(Counter::default()))
    }
}

/// Như trên với bản sao sandbox (khái niệm thử nghiệm).
struct TwiceCopy;

impl Module for TwiceCopy {
    fn name(&self) -> &'static str {
        "twice_copy"
    }

    fn sandbox_copy(&self) -> Option<Arc<dyn SandboxCopy>> {
        Some(Arc::new(NoCopy))
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.contribute::<dyn SandboxCopy>(Arc::new(NoCopy))
    }
}

/// Hook của bước chuyển không làm gì.
struct NoHook;

#[async_trait::async_trait]
impl SwitchHook for NoHook {
    async fn switch(
        &self,
        _control: &sea_orm::DatabaseTransaction,
        _relocation: &RelocationSwitch,
    ) -> Result<(), Error> {
        Ok(())
    }
}

/// Module kiểu cũ thêm hook bằng `Relocation::on_switch` và còn đóng góp `dyn SwitchHook`.
struct TwiceHook;

impl Module for TwiceHook {
    fn name(&self) -> &'static str {
        "twice_hook"
    }

    fn relocation(&self) -> Relocation {
        Relocation::new().on_switch(Arc::new(NoHook))
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.contribute::<dyn SwitchHook>(Arc::new(NoHook))
    }
}

/// Gọi `contribute` thiếu `::<dyn …>`: kiểu cụ thể, không ai đọc — bị chặn.
struct Untyped;

impl Module for Untyped {
    fn manifest(&self) -> Manifest {
        Manifest::new("untyped", "Forgets the trait.")
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.contribute(Arc::new(Counter::default()))
    }
}

/// Đóng góp một trait của kernel hai lần: registry của kernel báo lỗi như hàm đăng ký cũ.
struct TwicePersonal;

impl Module for TwicePersonal {
    fn manifest(&self) -> Manifest {
        Manifest::new("twice_personal", "Registers twice.")
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let counter = Arc::new(Counter::default());
        context.contribute::<dyn PersonalData>(counter.clone())?;
        context.contribute::<dyn PersonalData>(counter)
    }
}

/// Mở database, chạy migration, khởi động `modules` ở chế độ nhiều khách.
async fn started(
    test: &TestDatabase,
    modules: Vec<Box<dyn Module>>,
) -> (Result<Modules, String>, DatabaseRegistry) {
    let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = Config::read(&mut reader);
    let mut modules = Modules::new(modules).unwrap();
    let configured = modules.configure(&mut reader);
    let config = reader.finish(config.and_then(|config| configured.map(|()| config))).unwrap().0;
    let database = Database::connect(&config.database).await.unwrap();
    modules.migrator().unwrap().up(&database).await.unwrap();
    let registry = DatabaseRegistry::new(
        database,
        config.database,
        TenancyConfig::with_mode(TenancyMode::Multi),
    );
    let result = modules.start_in(&registry).map(|()| modules).map_err(|error| error.to_string());
    (result, registry)
}

#[tokio::test]
async fn dong_gop_bang_contribute_den_dung_registry_cua_kernel() {
    TestDatabase::run(async |test| {
        let counter = Arc::new(Counter::default());
        let (modules, _registry) = started(test, vec![Box::new(Garden(counter.clone()))]).await;
        let modules = modules.unwrap();

        // Seed chạy khi tạo khách, đúng một lần (không bị gom hai lần).
        let admin = modules.api::<TenantAdmin>().unwrap();
        let input: NewTenant =
            serde_json::from_value(json!({"slug": "acme", "name": "Acme"})).unwrap();
        let tenant = admin.create(Actor::system(), input).await.unwrap();
        assert_eq!(counter.seeds.load(Ordering::SeqCst), 1);
        assert_eq!(counter.samples.load(Ordering::SeqCst), 0);

        // Dữ liệu mẫu: module có tên trong danh sách và chạy khi được gọi.
        assert_eq!(admin.sample_modules(), ["garden"]);
        let seeded = admin.seed_samples(TenantId::from_uuid(tenant.id), &["garden"]).await.unwrap();
        assert_eq!(seeded, ["garden"]);
        assert_eq!(counter.samples.load(Ordering::SeqCst), 1);

        // Dữ liệu cá nhân vào registry xuất dữ liệu như `StartContext::personal_data`.
        assert!(modules.personal_data().modules().unwrap().contains(&"garden"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ham_cu_va_contribute_cung_khai_niem_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let (seed, _) = started(test, vec![Box::new(TwiceSeed)]).await;
        let error = seed.err().unwrap();
        assert!(
            error.contains("`twice_seed` implements `Module::seed_tenant` and also contributes"),
            "{error}"
        );
    })
    .await
    .unwrap();
    TestDatabase::run(async |test| {
        let (copy, _) = started(test, vec![Box::new(TwiceCopy)]).await;
        let error = copy.err().unwrap();
        assert!(
            error.contains("`twice_copy` implements `Module::sandbox_copy` and also contributes"),
            "{error}"
        );
    })
    .await
    .unwrap();
    TestDatabase::run(async |test| {
        let (hook, _) = started(test, vec![Box::new(TwiceHook)]).await;
        let error = hook.err().unwrap();
        assert!(
            error.contains(
                "`twice_hook` adds a switch hook with `Relocation::on_switch` and also contributes"
            ),
            "{error}"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dong_gop_trung_vao_registry_cua_kernel_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let (modules, _) = started(test, vec![Box::new(TwicePersonal)]).await;
        let error = modules.err().unwrap();
        assert!(error.contains("twice_personal"), "{error}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn contribute_thieu_dyn_trait_bi_chan() {
    TestDatabase::run(async |test| {
        let (modules, _) = started(test, vec![Box::new(Untyped)]).await;
        let error = modules.err().unwrap();
        assert!(
            error.contains("module `untyped` contributes") && error.contains("a concrete type"),
            "{error}"
        );
    })
    .await
    .unwrap();
}
