//! Lệnh `autoxide modules`: mọi module có trong binary này — kể cả module binary thêm bằng
//! `Autoxide::module` — với hạng, đang bật hay tắt, phụ thuộc, và module đang tắt còn dữ liệu trong
//! database không. `--json` cho agent và script.
//!
//! Chỉ đọc: không khởi động module, không chạy migration. Không kết nối được database thì vẫn liệt
//! kê, cột dữ liệu ghi "unknown".

use std::collections::BTreeMap;
use std::process::ExitCode;

use autoxide_kernel::database::Database;
use autoxide_kernel::module::Modules;
use serde::Serialize;

use super::{report_failure, report_invalid_configuration, run_with_observability};
use crate::{registry, settings};

#[derive(Debug, clap::Args)]
pub struct Arguments {
    /// Print JSON instead of a table: `{"setting": …, "modules": [{"name", "description", "tier",
    /// "enabled", "essential", "depends_on", "enabled_because_of", "data"}]}`. `data` is given
    /// for modules that are turned off: the tables that still hold rows, or `null` when the
    /// database cannot be read.
    #[arg(long)]
    json: bool,
}

/// Một dòng của bảng. Tên trường là hợp đồng của `--json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Row {
    name: &'static str,
    description: &'static str,
    /// `official` hay `experimental`.
    tier: &'static str,
    enabled: bool,
    /// Core không chạy được nếu thiếu: không tắt được.
    essential: bool,
    depends_on: Vec<&'static str>,
    /// Module đang bật cần module này, nên nó chạy dù người cài không nêu.
    enabled_because_of: Vec<&'static str>,
    /// Chỉ cho module đang tắt: bảng còn dòng trong database (rỗng: không còn dữ liệu); `None` khi
    /// module đang bật hoặc không đọc được database.
    data: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct Report {
    /// Giá trị `AUTOXIDE_MODULES` người cài đặt; `None`: mặc định (`official`).
    setting: Option<String>,
    modules: Vec<Row>,
}

pub fn run(arguments: &Arguments) -> ExitCode {
    let settings = match settings::load() {
        Ok(settings) => settings,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    let config = settings.config.clone();
    let outcome = run_with_observability(&config, async move {
        // Database chỉ để biết module tắt còn dữ liệu không: không kết nối được thì bỏ cột đó.
        let data = match Database::connect(&config.database).await {
            Ok(database) => {
                let data = dormant_data(&settings.modules, &database).await;
                let _ = database.close().await;
                data
            }
            Err(_) => None,
        };
        Ok(Report {
            setting: settings.modules.selection_text().map(str::to_owned),
            modules: rows(&settings.modules, data.as_ref()),
        })
    });
    let report = match outcome {
        Ok(report) => report,
        Err(error) => return report_failure(&error),
    };
    if arguments.json {
        match serde_json::to_string_pretty(&report) {
            Ok(text) => println!("{text}"),
            Err(error) => return report_failure(&error.to_string()),
        }
    } else {
        print!("{}", table(&report.modules));
    }
    ExitCode::SUCCESS
}

/// Bảng còn dòng của từng module đang tắt; `None` khi không đọc được database.
async fn dormant_data(
    modules: &Modules,
    database: &Database,
) -> Option<BTreeMap<&'static str, Vec<String>>> {
    let dormant = modules.dormant(database).await.ok()?;
    Some(dormant.into_iter().map(|module| (module.name, module.tables_with_data)).collect())
}

/// Một dòng cho mỗi module của binary, theo thứ tự đăng ký. `data`: kết quả của [`dormant_data`].
fn rows(modules: &Modules, data: Option<&BTreeMap<&'static str, Vec<String>>>) -> Vec<Row> {
    let enabled = modules.names();
    registry::all()
        .iter()
        .map(|module| {
            let name = module.name();
            let is_enabled = enabled.contains(&name);
            Row {
                name,
                description: module.description(),
                tier: module.stability().as_str(),
                enabled: is_enabled,
                essential: module.essential(),
                depends_on: module.depends_on(),
                enabled_because_of: modules
                    .implied()
                    .iter()
                    .filter(|(implied, _)| *implied == name)
                    .map(|(_, needed_by)| *needed_by)
                    .collect(),
                // Module tắt mà database không có bảng của nó: không còn dữ liệu.
                data: (!is_enabled)
                    .then(|| data.map(|data| data.get(name).cloned().unwrap_or_default()))
                    .flatten(),
            }
        })
        .collect()
}

/// Bảng chữ cho người đọc.
fn table(rows: &[Row]) -> String {
    let width = rows.iter().map(|row| row.name.len()).max().unwrap_or(0).max("MODULE".len());
    let mut text =
        format!("{:width$}  {:12}  {:5}  {:24}  DEPENDS ON\n", "MODULE", "TIER", "STATE", "DATA");
    for row in rows {
        let state = if row.enabled { "on" } else { "off" };
        let data = match (&row.data, row.enabled) {
            (_, true) => "-".to_owned(),
            (None, false) => "unknown".to_owned(),
            (Some(tables), false) if tables.is_empty() => "none".to_owned(),
            (Some(tables), false) => format!("{} table(s) with rows", tables.len()),
        };
        let mut depends = row.depends_on.join(", ");
        if !row.enabled_because_of.is_empty() {
            depends.push_str(&format!("  (on because of: {})", row.enabled_because_of.join(", ")));
        }
        text.push_str(&format!(
            "{:width$}  {:12}  {:5}  {:24}  {}\n",
            row.name,
            row.tier,
            state,
            data,
            depends.trim()
        ));
    }
    let on = rows.iter().filter(|row| row.enabled).count();
    text.push_str(&format!(
        "\n{on} on, {} off. Choose with AUTOXIDE_MODULES (see docs/MODULES.md).\n",
        rows.len() - on
    ));
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::config::ConfigSource;
    use autoxide_testkit::TestDatabase;

    fn settings_with(database: &TestDatabase, modules: &str) -> settings::Settings {
        let source = ConfigSource::from_pairs([
            ("DATABASE_URL", database.url()),
            ("AUTOXIDE_MODULES", modules),
        ]);
        settings::read(&source).unwrap()
    }

    #[tokio::test]
    async fn liet_ke_module_bat_tat_phu_thuoc_va_du_lieu() {
        TestDatabase::run(async |test| {
            let settings = settings_with(test, "official,-newsletter");
            let database = Database::connect(&settings.config.database).await.unwrap();
            let data = dormant_data(&settings.modules, &database).await;
            let rows = rows(&settings.modules, data.as_ref());

            let auth = rows.iter().find(|row| row.name == "auth").unwrap();
            assert!(auth.enabled && auth.essential && auth.data.is_none(), "{auth:?}");
            assert_eq!(auth.tier, "official");
            let newsletter = rows.iter().find(|row| row.name == "newsletter").unwrap();
            assert!(!newsletter.enabled, "{newsletter:?}");
            // Database trống: module tắt không có bảng nào, tức không còn dữ liệu.
            assert_eq!(newsletter.data, Some(Vec::new()));
            assert!(newsletter.depends_on.contains(&"mail"), "{newsletter:?}");
            // Mỗi module của binary đúng một dòng, theo thứ tự đăng ký.
            let names: Vec<&str> = rows.iter().map(|row| row.name).collect();
            let registered: Vec<&str> =
                registry::all().iter().map(|module| module.name()).collect();
            assert_eq!(names, registered);

            let printed = table(&rows);
            assert!(printed.starts_with("MODULE"), "{printed}");
            assert!(printed.contains("newsletter"), "{printed}");
            let json = serde_json::to_value(&rows).unwrap();
            assert_eq!(json[0]["name"], names[0]);
            assert!(json[0]["depends_on"].is_array());
            database.close().await.unwrap();
        })
        .await
        .unwrap();
    }

    #[test]
    fn khong_doc_duoc_database_thi_du_lieu_la_unknown() {
        let row = Row {
            name: "notes",
            description: "",
            tier: "official",
            enabled: false,
            essential: false,
            depends_on: Vec::new(),
            enabled_because_of: Vec::new(),
            data: None,
        };
        assert!(table(&[row]).contains("unknown"));
    }
}
