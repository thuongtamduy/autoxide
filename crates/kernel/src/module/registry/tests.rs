//! Test của bộ đăng ký module.

use super::*;
use crate::config::{ConfigSource, Setting};

const LIMIT: Setting = Setting::new("AUTOXIDE_NOTES_LIMIT", "Limit.").with_default("10");
const KEY: Setting = Setting::new("AUTOXIDE_TAGS_KEY", "Key.");

/// Module không khai gì ngoài tên: dùng mọi cài đặt mặc định của hợp đồng.
struct Bare(&'static str);

impl Module for Bare {
    fn name(&self) -> &'static str {
        self.0
    }
}

/// Module đọc một biến có mặc định.
#[derive(Default)]
struct Notes {
    limit: u32,
}

impl Module for Notes {
    fn name(&self) -> &'static str {
        "notes"
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.limit = reader.get(&LIMIT)?;
        Ok(())
    }
}

/// Module sở hữu đối tượng `shop.item`, khai event vòng đời.
struct Shop(Vec<crate::resource::ResourceEvent>);

impl Module for Shop {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn events(&self) -> Vec<crate::event::EventInfo> {
        vec![crate::event::EventInfo::new("shop.item_deleted", "An item was deleted.")]
    }

    fn resource_events(&self) -> Vec<crate::resource::ResourceEvent> {
        self.0.clone()
    }
}

#[test]
fn event_vong_doi_duoc_kiem_luc_khoi_dong() {
    use crate::resource::{ResourceChange, ResourceEvent};
    let good = ResourceEvent::new("shop.item_deleted", "shop.item", ResourceChange::Removed);
    let modules = Modules::new(vec![Box::new(Shop(vec![good]))]).unwrap();
    assert_eq!(modules.resource_events, [good]);
    for bad in [
        ResourceEvent::new("shop.item_gone", "shop.item", ResourceChange::Removed),
        ResourceEvent::new("shop.item_deleted", "media.item", ResourceChange::Removed),
    ] {
        let error = Modules::new(vec![Box::new(Shop(vec![bad]))]).unwrap_err();
        assert!(error.to_string().contains("lifecycle event"), "{error}");
    }
    let error = Modules::new(vec![Box::new(Shop(vec![good, good]))]).unwrap_err();
    assert!(error.to_string().contains("declared twice"), "{error}");
}

/// Module đọc một biến bắt buộc.
struct Tags;

impl Module for Tags {
    fn name(&self) -> &'static str {
        "tags"
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        reader.get::<String>(&KEY).map(|_| ())
    }
}

fn bare(names: &[&'static str]) -> Vec<Box<dyn Module>> {
    names.iter().map(|name| Box::new(Bare(name)) as Box<dyn Module>).collect()
}

fn error(names: &[&'static str]) -> String {
    Modules::new(bare(names)).unwrap_err().to_string()
}

#[test]
fn giu_thu_tu_dang_ky() {
    let modules = Modules::new(bare(&["notes", "auth", "tags"])).unwrap();
    assert_eq!(modules.names(), ["kernel", "antispam", "notes", "auth", "tags"]);
    assert!(!modules.is_empty());
    assert_eq!(
        format!("{modules:?}"),
        r#"Modules(["kernel", "antispam", "notes", "auth", "tags"])"#
    );
    assert!(Modules::new(Vec::new()).unwrap().is_empty());
}

#[test]
fn ten_module_sai_dang_bi_tu_choi() {
    for name in ["", "Auth", "1auth", "auth-keys", "auth_", "au th"] {
        let message = error(&[name]);
        assert!(message.contains("must be lowercase letters, digits and `_`"), "{message}");
    }
}

#[test]
fn ten_danh_rieng_cua_database_bi_tu_choi() {
    for name in ["sqlite", "pg"] {
        assert!(error(&[name]).contains("is reserved"), "{name}");
    }
    // Chỉ đúng hai tên đó; tên chứa chúng thì không sao.
    assert!(Modules::new(bare(&["pgqueue", "sqlitex"])).is_ok());
}

#[test]
fn dang_ky_trung_hoac_ten_long_nhau_bi_tu_choi() {
    assert!(error(&["auth", "auth"]).contains("module `auth` is registered twice"));
    assert!(error(&["auth", "auth_keys"]).contains("`auth` and `auth_keys` are ambiguous"));
    // `auth` và `authz`: tiền tố bảng là `auth_` và `authz_`, không đụng nhau.
    assert!(Modules::new(bare(&["auth", "authz"])).is_ok());
}

#[test]
fn moi_loi_ten_duoc_bao_cung_luc() {
    let message = error(&["Auth", "notes", "notes"]);
    assert!(message.starts_with("modules: "), "{message}");
    assert!(message.contains("`Auth`") && message.contains("registered twice"), "{message}");
}

#[test]
fn tung_module_doc_cau_hinh_cua_minh() {
    let source = ConfigSource::from_pairs([("AUTOXIDE_NOTES_LIMIT", "25")]);
    let mut reader = ConfigReader::new(&source);
    let mut notes = Notes::default();

    assert_eq!(notes.configure(&mut reader), Ok(()));

    assert_eq!(notes.limit, 25);
}

#[test]
fn loi_cau_hinh_cua_moi_module_duoc_gom_chung() {
    // `notes` nhận giá trị sai, `tags` thiếu biến bắt buộc: cả hai phải có mặt
    // trong cùng một thông báo, dù `notes` đứng trước và đã hỏng.
    let source = ConfigSource::from_pairs([("AUTOXIDE_NOTES_LIMIT", "many")]);
    let mut reader = ConfigReader::new(&source);
    let mut modules =
        Modules::new(vec![Box::new(Notes::default()), Box::new(Tags), Box::new(Bare("auth"))])
            .unwrap();

    let configured = modules.configure(&mut reader);

    assert_eq!(configured, Err(Invalid));
    let message = reader.finish(configured).unwrap_err().to_string();
    assert!(message.contains("invalid configuration (2 problems):"), "{message}");
    assert!(message.contains("AUTOXIDE_NOTES_LIMIT:"), "{message}");
    assert!(message.contains("AUTOXIDE_TAGS_KEY: is required but not set"), "{message}");
}

/// Chế độ `multi` mà không đặt `AUTOXIDE_SECRET_KEY`: lỗi cấu hình nói rõ phải làm gì;
/// `single` thì không bắt buộc.
#[test]
fn che_do_multi_bat_buoc_khoa_goc() {
    let check = |pairs: &[(&str, &str)]| {
        let source = ConfigSource::from_pairs(pairs.iter().copied());
        let mut reader = ConfigReader::new(&source);
        let kernel = crate::config::Config::read(&mut reader);
        let mut modules = Modules::new(bare(&["auth"])).unwrap();
        let configured = modules.configure(&mut reader).and(kernel.map(|_| ()));
        reader.finish(configured).map(|_| ()).map_err(|error| error.to_string())
    };
    let error = check(&[("AUTOXIDE_TENANCY_MODE", "multi")]).unwrap_err();
    assert!(error.contains("AUTOXIDE_SECRET_KEY: is required when"), "{error}");
    let key = "k".repeat(40);
    check(&[("AUTOXIDE_TENANCY_MODE", "multi"), ("AUTOXIDE_SECRET_KEY", key.as_str())]).unwrap();
    check(&[]).unwrap();
}

#[test]
fn module_khong_khai_gi_thi_khong_co_migration_va_cau_hinh() {
    let source = ConfigSource::default();
    let mut reader = ConfigReader::new(&source);
    let mut modules = Modules::new(bare(&["auth"])).unwrap();

    assert_eq!(modules.configure(&mut reader), Ok(()));
    // Chỉ biến của hai module nội bộ `kernel` và `antispam`.
    let names: Vec<&str> = reader.values().iter().map(|value| value.setting.name).collect();
    let internal = |name: &&str| {
        [
            "AUTOXIDE_SECRET_KEY",
            "AUTOXIDE_SECRET_KEY_PREVIOUS",
            "AUTOXIDE_HTTP_PUBLIC_URL",
            "AUTOXIDE_USER_FLAGS_MAX",
            "AUTOXIDE_TENANCY_EXPORT_RETENTION_DAYS",
            "AUTOXIDE_JOBS_FILE_RETENTION_DAYS",
            "AUTOXIDE_TEXT_OVERRIDES_UI_MAX",
            "AUTOXIDE_TEMPLATE_MAX_BYTES",
            "AUTOXIDE_SEARCH_BACKEND",
            "AUTOXIDE_RELOCATION_DRAIN_SECONDS",
            "AUTOXIDE_TENANCY_SANDBOX_DAYS",
            "AUTOXIDE_TENANCY_SANDBOX_MAX_PRODUCTS",
        ]
        .contains(name)
            || name.starts_with("AUTOXIDE_ANTISPAM_")
            || name.starts_with("AUTOXIDE_EVENTS_STREAM")
            || name.starts_with("AUTOXIDE_BACKUP_")
            || name.starts_with("AUTOXIDE_SYNC_")
    };
    assert!(names.iter().all(internal), "{names:?}");
    assert!(modules.migrator().is_ok());
}
