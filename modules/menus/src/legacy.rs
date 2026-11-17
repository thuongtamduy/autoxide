//! Nhập menu cũ của `content` (bước 17) sang module này (D-238):
//! `autoxide task menus.import_content_menus`, hay `POST /api/v1/menus/import-content` (có chạy thử).
//!
//! - Đọc qua trait `autoxide_content::api::LegacyMenus`, không đọc bảng của `content`.
//! - Chạy lại nhiều lần không nhân đôi: menu đã có cùng (`key`, `locale`) thì bỏ qua — kể cả khi
//!   nó đã được sửa sau lần nhập trước, để không đè việc của người quản trị.
//! - Mục trỏ bài viết thành `content.entry`, mục trỏ danh mục / thẻ thành `content.term`.
//! - Theo khách (bước 33): mỗi lần chạy nhập menu cũ của MỘT khách vào chính khách đó — tham số
//!   `tenant=<tên rút gọn>`, bỏ trống là khách mặc định (nơi có mọi menu cũ làm trước bước 33).
//!   Không duyệt mọi khách: menu cũ nằm ở bảng của `content`, module này không biết khách nào có.

use std::sync::Arc;

use autoxide_content::api::{LegacyMenuItem, LegacyTarget};
use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::task::{Task, TaskArgs, TaskArgument, TaskRun};
use autoxide_kernel::tenancy::{DatabaseRegistry, Site, TenantContext, root_of};

use crate::config::canonical_locale;
use crate::items::{ItemInput, RefInput};
use crate::permissions::MENUS_MANAGE;
use crate::service::{MenuService, NewMenu};

/// Kết quả một lần nhập.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ImportReport {
    /// (`key`, `locale`) đã nhập.
    pub imported: Vec<(String, String)>,
    /// Đã có, bỏ qua.
    pub skipped: Vec<(String, String)>,
    /// Không nhập được, kèm lý do (vd ngôn ngữ không có trong `AUTOXIDE_MENUS_LOCALES`).
    pub failed: Vec<(String, String, String)>,
}

impl MenuService {
    /// Nhập mọi menu cũ của khách `tenant` ở `content` chưa có ở khách đó. `principal` `None`:
    /// lệnh của người vận hành máy chủ (không kiểm permission, event của hệ thống); có: người gọi
    /// qua REST cần `menus.manage` ở cả khách (menu cũ là menu của cả khách) và là người làm của
    /// event. `dry_run`: chỉ báo sẽ nhập gì, không ghi (`imported` là danh sách sẽ nhập).
    pub(crate) async fn import_content_menus(
        &self,
        tenant: &TenantContext,
        principal: Option<&Principal>,
        dry_run: bool,
    ) -> Result<ImportReport, Error> {
        if let Some(principal) = principal {
            principal.require(&MENUS_MANAGE)?;
            let whole = OrgTarget::new(root_of(tenant.id()), None);
            principal.require_at(tenant.database().read(), &MENUS_MANAGE, whole).await?;
        }
        let legacy = self.legacy.as_ref().ok_or_else(|| {
            Error::bad_request(
                "menus.import_source_missing",
                "Module `content` is not installed, so there are no old menus to import.",
            )
        })?;
        let mut report = ImportReport::default();
        for old in legacy.legacy_menus(tenant).await? {
            let pair = (old.key.clone(), old.locale.clone());
            let Some(locale) =
                canonical_locale(&old.locale).filter(|locale| self.config.locales.contains(locale))
            else {
                let reason = format!(
                    "the language is not in AUTOXIDE_MENUS_LOCALES ({})",
                    self.config.locales.join(", ")
                );
                report.failed.push((pair.0, pair.1, reason));
                continue;
            };
            if self.find_by_key(tenant, &Site::whole(tenant), &old.key, &locale).await?.is_some() {
                report.skipped.push(pair);
                continue;
            }
            if dry_run {
                report.imported.push(pair);
                continue;
            }
            let input = NewMenu {
                key: old.key.clone(),
                locale: Some(locale),
                org_id: None,
                items: old.items.iter().map(convert).collect(),
            };
            match self.create_checked(tenant, principal, input, false).await {
                Ok(_) => report.imported.push(pair),
                // Hai lần nhập chạy cùng lúc: bên kia vừa tạo xong menu này.
                Err(error) if error.code() == "menus.menu_exists" => report.skipped.push(pair),
                Err(error) => report.failed.push((pair.0, pair.1, error.message())),
            }
        }
        Ok(report)
    }
}

fn convert(old: &LegacyMenuItem) -> ItemInput {
    let (url, target) = match &old.target {
        LegacyTarget::Entry(id) => {
            (None, Some(RefInput { kind: "content.entry".to_owned(), id: id.to_string() }))
        }
        LegacyTarget::Term(id) => {
            (None, Some(RefInput { kind: "content.term".to_owned(), id: id.to_string() }))
        }
        LegacyTarget::Url(url) => (Some(url.clone()), None),
    };
    ItemInput {
        id: None,
        label: Some(old.label.clone()),
        url,
        target,
        children: old.children.iter().map(convert).collect(),
        ..ItemInput::default()
    }
}

/// Lệnh `menus.import_content_menus`.
pub(crate) fn task(service: Arc<MenuService>, databases: DatabaseRegistry) -> Task {
    Task::new(
        "import_content_menus",
        "Copy the old website menus of module `content` into module `menus`, for one tenant. \
         Menus that already exist there (same key and language) are left as they are, so running \
         it again is safe.",
        Arc::new(Import { service, databases }),
    )
    .argument(TaskArgument::optional(
        "tenant",
        "Short name (slug) of the tenant whose menus to copy; omitted: the default tenant.",
    ))
}

struct Import {
    service: Arc<MenuService>,
    databases: DatabaseRegistry,
}

#[async_trait::async_trait]
impl TaskRun for Import {
    async fn run(&self, args: &TaskArgs) -> Result<String, Error> {
        let tenant = match args.get("tenant") {
            Some(slug) => self.databases.tenant_by_slug(slug.trim()).await?,
            None => self.databases.default_tenant(),
        };
        let report = self.service.import_content_menus(&tenant, None, false).await?;
        let list = |pairs: &[(String, String)]| {
            pairs
                .iter()
                .map(|(key, locale)| format!("{key} ({locale})"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut lines = vec![format!(
            "Tenant `{}`: imported {} menu(s), skipped {} already here, {} failed.",
            tenant.slug(),
            report.imported.len(),
            report.skipped.len(),
            report.failed.len()
        )];
        if !report.imported.is_empty() {
            lines.push(format!("Imported: {}", list(&report.imported)));
        }
        for (key, locale, reason) in &report.failed {
            lines.push(format!("Failed: {key} ({locale}): {reason}"));
        }
        Ok(lines.join("\n"))
    }
}
