//! Test của module `menus`, trên database gate đang chọn (gate chạy cả SQLite và Postgres).
//!
//! - `service.rs`: gọi service thẳng, với sổ đối tượng giả (`shop.item`, `shop.category`).
//! - `http.rs`: qua REST và MCP thật (`TestApp`).

mod http;
mod service;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use autoxide_content::api::{LegacyMenu, LegacyMenus};
use autoxide_kernel::access::Principal;
use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Outbox;
use autoxide_kernel::migration::{Migrator, ModuleMigrations};
use autoxide_kernel::resource::{Resource, ResourceDirectory, ResourceRef, ResourceResolver};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_media::api::{Media, MediaRef, MediaVisibility};
use autoxide_testkit::TestDatabase;
use uuid::Uuid;

use crate::config::MenusConfig;
use crate::service::{Dependencies, MenuService};

/// Ảnh công khai và ảnh riêng tư của thư viện media giả.
pub(crate) const PUBLIC_IMAGE: Uuid = Uuid::from_u128(0x1);
pub(crate) const PRIVATE_IMAGE: Uuid = Uuid::from_u128(0x2);

/// `shop.item`: ID bắt đầu bằng `p` là công khai, `h` là đang ẩn, còn lại không có. Tiêu đề
/// theo ngôn ngữ (`<id>@<locale>`); `p-img` có ảnh công khai, `p-private` có ảnh riêng tư.
pub(crate) struct Items;

#[async_trait::async_trait]
impl ResourceResolver for Items {
    async fn resolve(&self, tenant: &TenantContext, id: &str) -> Result<Option<Resource>, Error> {
        Ok(self.resolve_many(tenant, &[id.to_owned()], None).await?.remove(id))
    }

    async fn resolve_many(
        &self,
        _tenant: &TenantContext,
        ids: &[String],
        locale: Option<&str>,
    ) -> Result<HashMap<String, Resource>, Error> {
        let locale = locale.unwrap_or("-");
        Ok(ids
            .iter()
            .filter(|id| id.starts_with('p') || id.starts_with('h'))
            .map(|id| {
                let public = id.starts_with('p');
                let image = match id.as_str() {
                    "p-img" => Some(PUBLIC_IMAGE),
                    "p-private" => Some(PRIVATE_IMAGE),
                    _ => None,
                };
                let resource =
                    Resource::new(if public { format!("{id}@{locale}") } else { String::new() })
                        .public(public)
                        .summary(format!("About {id}"))
                        .url(Some(format!("/items/{id}")))
                        .image(image);
                (id.clone(), resource)
            })
            .collect())
    }
}

/// `shop.category`: có cây. `c` có năm con `c1`…`c5`, trong đó `c3` đang ẩn (`h3`).
pub(crate) struct Categories;

#[async_trait::async_trait]
impl ResourceResolver for Categories {
    async fn resolve(&self, _tenant: &TenantContext, id: &str) -> Result<Option<Resource>, Error> {
        let public = !id.starts_with('h');
        Ok((id.starts_with('c') || id.starts_with('h')).then(|| {
            Resource::new(if public { format!("Category {id}") } else { String::new() })
                .public(public)
                .url(Some(format!("/c/{id}")))
        }))
    }

    fn has_children(&self) -> bool {
        true
    }

    async fn children(
        &self,
        _tenant: &TenantContext,
        id: &str,
        _: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ResourceRef>, Error> {
        Ok(if id == "c" {
            ["c1", "c2", "h3", "c4", "c5"]
                .iter()
                .take(limit)
                .map(|child| ResourceRef::new("shop.category", *child))
                .collect()
        } else {
            Vec::new()
        })
    }
}

/// Mọi ID là bài viết / danh mục có thật (cho test nhập menu cũ).
pub(crate) struct Anything;

#[async_trait::async_trait]
impl ResourceResolver for Anything {
    async fn resolve(&self, _tenant: &TenantContext, id: &str) -> Result<Option<Resource>, Error> {
        Ok(Some(Resource::new(format!("Old {id}")).public(true).url(Some(format!("/old/{id}")))))
    }
}

pub(crate) fn directory() -> ResourceDirectory {
    let directory = ResourceDirectory::default();
    directory.register("shop", "shop.item", Arc::new(Items)).unwrap();
    directory.register("shop", "shop.category", Arc::new(Categories)).unwrap();
    directory.register("content", "content.entry", Arc::new(Anything)).unwrap();
    directory.register("content", "content.term", Arc::new(Anything)).unwrap();
    directory
}

/// Thư viện media giả: một ảnh công khai, một ảnh riêng tư.
#[derive(Default)]
pub(crate) struct FakeMedia;

#[async_trait::async_trait]
impl Media for FakeMedia {
    async fn find_many(
        &self,
        _tenant: &TenantContext,
        ids: &[Uuid],
    ) -> Result<Vec<MediaRef>, Error> {
        Ok(ids
            .iter()
            .filter(|id| [PUBLIC_IMAGE, PRIVATE_IMAGE].contains(id))
            .map(|id| MediaRef {
                id: *id,
                visibility: if *id == PUBLIC_IMAGE {
                    MediaVisibility::Public
                } else {
                    MediaVisibility::Private
                },
                content_type: "image/webp".to_owned(),
                width: Some(640),
                height: Some(480),
                alt: String::new(),
                url: format!("/api/v1/media/{id}/content?v=1"),
                variants: Vec::new(),
            })
            .collect())
    }

    async fn find_many_in(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
        locale: &str,
    ) -> Result<Vec<MediaRef>, Error> {
        let mut found = self.find_many(tenant, ids).await?;
        for media in &mut found {
            media.alt = format!("[{locale}]");
        }
        Ok(found)
    }
}

/// Menu cũ của `content` giả.
#[derive(Default)]
pub(crate) struct FakeLegacy(pub Mutex<Vec<LegacyMenu>>);

#[async_trait::async_trait]
impl LegacyMenus for FakeLegacy {
    async fn legacy_menus(&self, _tenant: &TenantContext) -> Result<Vec<LegacyMenu>, Error> {
        Ok(self.0.lock().unwrap().clone())
    }
}

pub(crate) struct Harness {
    pub service: Arc<MenuService>,
    pub legacy: Arc<FakeLegacy>,
}

/// Service trên database (đã migrate) của test, ngôn ngữ `en` (mặc định) và `vi`.
pub(crate) async fn harness(test: &TestDatabase) -> Harness {
    let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let kernel = Config::read(&mut reader);
    let (kernel, _) = reader.finish(kernel).unwrap();
    let database = Database::connect(&kernel.database).await.unwrap();
    Migrator::new(vec![ModuleMigrations::new("menus", crate::migration::all())])
        .unwrap()
        .up(&database)
        .await
        .unwrap();
    let legacy = Arc::new(FakeLegacy::default());
    let dependencies = Dependencies {
        resources: directory(),
        media: Arc::new(FakeMedia),
        legacy: Some(legacy.clone()),
    };
    let service = MenuService::new(
        database,
        Outbox::standalone("menus"),
        dependencies,
        MenusConfig::default(),
    );
    Harness { service: Arc::new(service), legacy }
}

/// Người quản trị menu.
pub(crate) fn admin() -> Principal {
    Principal::new(Uuid::now_v7(), ["menus.read".to_owned(), "menus.manage".to_owned()])
}
