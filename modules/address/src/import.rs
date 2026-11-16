//! Nhập bộ dữ liệu đơn vị hành chính (bước 34 mục 5–6, D-314): `autoxide task address.import
//! path=<thư mục>`. Khi nhà nước đổi danh mục, người vận hành nhập bộ mới — không sửa code.
//!
//! Thư mục có hai file:
//!
//! - `manifest.json`: `{"country", "title", "source_url", "retrieved_on", "effective_from"}` — nguồn
//!   chính thức (văn bản, URL, ngày tải) và ngày hiệu lực của danh mục;
//! - `units.csv` (UTF-8, dấu phẩy, dòng đầu là tên cột): `level,code,name,kind,parent_code` —
//!   `level` 1 (tỉnh) hay 2 (xã); mã giữ số 0 đầu; `parent_code` của xã là mã tỉnh.
//!
//! Nhập lại cùng một bộ (cùng SHA-256 của `units.csv`) không đổi gì. Bộ mới: đơn vị không còn hay
//! đổi tên / loại / cấp trên thì đóng (`valid_to` = ngày hiệu lực của bộ mới) và mở dòng mới;
//! đơn vị giữ nguyên thì giữ dòng cũ.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::task::{Task, TaskArgs, TaskArgument, TaskRun};
use chrono::{SubsecRound, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::entity::{dataset, unit};
use crate::events;
use crate::service::AddressService;

/// Bộ dữ liệu đóng gói sẵn (D-318): danh mục Việt Nam sau sáp nhập, nhập khi lệnh không nêu `path`.
pub(crate) const BUNDLED_MANIFEST: &str = include_str!("../data/vn-2025/manifest.json");
pub(crate) const BUNDLED_UNITS: &str = include_str!("../data/vn-2025/units.csv");

/// Manifest của bộ đóng gói và SHA-256 của `units.csv` của nó.
pub(crate) fn bundled() -> Result<(Manifest, String), Error> {
    let manifest: Manifest = serde_json::from_str(BUNDLED_MANIFEST).map_err(Error::internal)?;
    Ok((manifest, autoxide_kernel::secret::sha256(BUNDLED_UNITS)))
}

/// Loại đơn vị nhận được.
const KINDS: [&str; 5] = ["province", "city", "commune", "ward", "special_zone"];

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Manifest {
    pub country: String,
    pub title: String,
    pub source_url: String,
    pub retrieved_on: String,
    pub effective_from: String,
}

/// Một dòng của `units.csv`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    pub level: i32,
    pub code: String,
    pub name: String,
    pub kind: String,
    pub parent_code: Option<String>,
}

/// Kết quả một lần nhập.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Imported {
    pub dataset_id: Uuid,
    /// `false`: bộ này đã nhập trước đó, không đổi gì.
    pub changed: bool,
    pub opened: usize,
    pub closed: usize,
    pub provinces: usize,
    pub communes: usize,
}

fn invalid(problems: Vec<String>) -> Error {
    let fields = problems
        .into_iter()
        .take(20)
        .map(|problem| {
            FieldError::new("units.csv", "invalid", "{problem}").with_param("problem", problem)
        })
        .collect();
    Error::validation(fields)
}

/// Đọc `units.csv`; báo mọi dòng sai một lần (tối đa 20).
pub(crate) fn parse(csv: &str) -> Result<Vec<Row>, Error> {
    let mut rows = Vec::new();
    let mut problems = Vec::new();
    let mut lines = csv.lines().enumerate();
    let header =
        lines.next().map(|(_, line)| line.trim().trim_start_matches('\u{feff}').to_owned());
    if header.as_deref() != Some("level,code,name,kind,parent_code") {
        return Err(invalid(vec![
            "the first line must be `level,code,name,kind,parent_code`".to_owned(),
        ]));
    }
    for (index, line) in lines {
        if line.trim().is_empty() {
            continue;
        }
        let number = index + 1;
        let parts: Vec<&str> = line.split(',').map(str::trim).collect();
        let [level, code, name, kind, parent] = parts[..] else {
            problems.push(format!("line {number}: must have 5 fields"));
            continue;
        };
        let level = match level {
            "1" => 1,
            "2" => 2,
            _ => {
                problems.push(format!("line {number}: level must be 1 or 2"));
                continue;
            }
        };
        if code.is_empty() || code.len() > 10 || !code.chars().all(|c| c.is_ascii_digit()) {
            problems.push(format!("line {number}: code must be 1 to 10 digits"));
        }
        if name.is_empty() || name.chars().count() > 200 {
            problems.push(format!("line {number}: name must be 1 to 200 characters"));
        }
        if !KINDS.contains(&kind) {
            problems.push(format!("line {number}: kind must be one of {}", KINDS.join(", ")));
        }
        let parent_code = (!parent.is_empty()).then(|| parent.to_owned());
        if (level == 1) != parent_code.is_none() {
            problems
                .push(format!("line {number}: a province has no parent_code, a commune has one"));
        }
        rows.push(Row {
            level,
            code: code.to_owned(),
            name: name.to_owned(),
            kind: kind.to_owned(),
            parent_code,
        });
    }
    let provinces: BTreeSet<&str> =
        rows.iter().filter(|row| row.level == 1).map(|row| row.code.as_str()).collect();
    let mut seen = BTreeSet::new();
    for row in &rows {
        if !seen.insert(row.code.as_str()) {
            problems.push(format!("code {} appears twice", row.code));
        }
        if let Some(parent) = &row.parent_code
            && !provinces.contains(parent.as_str())
        {
            problems.push(format!(
                "commune {} names province {parent}, which is not in the list",
                row.code
            ));
        }
    }
    if !problems.is_empty() {
        return Err(invalid(problems));
    }
    Ok(rows)
}

fn check_manifest(manifest: &Manifest) -> Result<(), Error> {
    let mut problems = Vec::new();
    let date = |raw: &str| chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d").is_ok();
    if !crate::service::COUNTRIES.contains(&manifest.country.as_str()) {
        problems.push(format!("country `{}` is not supported", manifest.country));
    }
    if !date(&manifest.effective_from) || !date(&manifest.retrieved_on) {
        problems
            .push("effective_from and retrieved_on must be dates written YYYY-MM-DD".to_owned());
    }
    if manifest.title.trim().is_empty() || manifest.source_url.trim().is_empty() {
        problems.push("title and source_url must name the official source".to_owned());
    }
    if problems.is_empty() { Ok(()) } else { Err(invalid(problems)) }
}

impl AddressService {
    /// Nhập bộ `rows` (đã đọc từ `units.csv` có SHA-256 `sha256`) theo `manifest`.
    pub(crate) async fn import(
        &self,
        manifest: &Manifest,
        rows: &[Row],
        sha256: &str,
    ) -> Result<Imported, Error> {
        check_manifest(manifest)?;
        let provinces = rows.iter().filter(|row| row.level == 1).count();
        let communes = rows.len() - provinces;
        let control = self.control();
        if let Some(existing) = dataset::Entity::find()
            .filter(dataset::Column::Sha256.eq(sha256))
            .one(control.read())
            .await?
        {
            return Ok(Imported {
                dataset_id: existing.id,
                changed: false,
                opened: 0,
                closed: 0,
                provinces,
                communes,
            });
        }
        let country = manifest.country.as_str();
        let from = manifest.effective_from.as_str();
        let transaction = control.begin_write().await?;
        let current: Vec<unit::Model> = unit::Entity::find()
            .filter(unit::Column::Country.eq(country))
            .filter(unit::Column::ValidTo.is_null())
            .all(&transaction)
            .await?;
        let current: BTreeMap<&str, &unit::Model> =
            current.iter().map(|row| (row.code.as_str(), row)).collect();
        let wanted: BTreeMap<&str, &Row> =
            rows.iter().map(|row| (row.code.as_str(), row)).collect();
        let dataset_id = Uuid::now_v7();
        dataset::ActiveModel {
            id: Set(dataset_id),
            country: Set(country.to_owned()),
            effective_from: Set(from.to_owned()),
            title: Set(manifest.title.trim().to_owned()),
            source_url: Set(manifest.source_url.trim().to_owned()),
            retrieved_on: Set(manifest.retrieved_on.clone()),
            sha256: Set(sha256.to_owned()),
            province_count: Set(i32::try_from(provinces).map_err(Error::internal)?),
            commune_count: Set(i32::try_from(communes).map_err(Error::internal)?),
            imported_at: Set(Utc::now().trunc_subsecs(6)),
        }
        .insert(&transaction)
        .await?;
        let same = |old: &unit::Model, new: &Row| {
            old.name == new.name
                && old.kind == new.kind
                && old.parent_code == new.parent_code
                && old.level == new.level
        };
        let mut closed = 0;
        for (code, old) in &current {
            if wanted.get(code).is_none_or(|new| !same(old, new)) {
                unit::Entity::update_many()
                    .col_expr(unit::Column::ValidTo, Expr::value(Some(from.to_owned())))
                    .filter(unit::Column::Id.eq(old.id))
                    .exec(&transaction)
                    .await?;
                closed += 1;
            }
        }
        let mut opened = 0;
        // Theo mã: ID (UUIDv7) tăng cùng thứ tự của văn bản — danh sách phân trang theo ID ra theo mã.
        for (code, new) in &wanted {
            if current.get(code).is_some_and(|old| same(old, new)) {
                continue;
            }
            unit::ActiveModel {
                id: Set(Uuid::now_v7()),
                country: Set(country.to_owned()),
                level: Set(new.level),
                code: Set(new.code.clone()),
                name: Set(new.name.clone()),
                name_folded: Set(autoxide_kernel::search::fold(&new.name)),
                kind: Set(new.kind.clone()),
                parent_code: Set(new.parent_code.clone()),
                valid_from: Set(from.to_owned()),
                valid_to: Set(None),
                dataset_id: Set(dataset_id),
            }
            .insert(&transaction)
            .await?;
            opened += 1;
        }
        let event = events::DatasetImported {
            dataset: json!({
                "id": dataset_id,
                "country": country,
                "title": manifest.title,
                "effective_from": from,
                "provinces": provinces,
                "communes": communes,
                "opened": opened,
                "closed": closed,
            }),
        };
        let platform = self.databases.default_tenant();
        self.outbox
            .record(
                &transaction,
                &platform,
                Actor::system(),
                Some(format!("address.dataset:{dataset_id}")),
                &event,
            )
            .await?;
        transaction.commit().await?;
        tracing::info!(provinces, communes, opened, closed, "administrative units imported");
        Ok(Imported { dataset_id, changed: true, opened, closed, provinces, communes })
    }
}

/// Lệnh `address.import`.
pub(crate) fn task(service: Arc<AddressService>) -> Task {
    Task::new(
        "import",
        "Import an official list of administrative units from a directory holding `manifest.json` \
         and `units.csv` (see the module guide). Importing the same list again changes nothing; a \
         new list closes the units it no longer has and opens the new ones.",
        Arc::new(ImportTask(service)),
    )
    .argument(TaskArgument::optional(
        "path",
        "Directory holding `manifest.json` and `units.csv`. Omit to import the list shipped with \
         the server (Vietnam, in force from 2025-07-01).",
    ))
}

struct ImportTask(Arc<AddressService>);

#[async_trait::async_trait]
impl TaskRun for ImportTask {
    async fn run(&self, args: &TaskArgs) -> Result<String, Error> {
        let Some(path) = args.get("path") else {
            let (manifest, sha256) = bundled()?;
            let rows = parse(BUNDLED_UNITS)?;
            let imported = self.0.import(&manifest, &rows, &sha256).await?;
            return Ok(report(&manifest, &imported));
        };
        let path = std::path::PathBuf::from(path);
        let read = |name: &str| {
            std::fs::read_to_string(path.join(name)).map_err(|error| {
                Error::bad_request("address.import_unreadable", "Cannot read `{file}`: {reason}.")
                    .with_param("file", path.join(name).display().to_string())
                    .with_param("reason", error.to_string())
            })
        };
        let manifest: Manifest =
            serde_json::from_str(&read("manifest.json")?).map_err(|error| {
                Error::bad_request("address.import_unreadable", "Cannot read `{file}`: {reason}.")
                    .with_param("file", "manifest.json")
                    .with_param("reason", error.to_string())
            })?;
        let csv = read("units.csv")?;
        let rows = parse(&csv)?;
        let sha256 = autoxide_kernel::secret::sha256(&csv);
        let imported = self.0.import(&manifest, &rows, &sha256).await?;
        Ok(report(&manifest, &imported))
    }
}

fn report(manifest: &Manifest, imported: &Imported) -> String {
    if imported.changed {
        format!(
            "Imported {} provinces and {} communes ({}, in force from {}): {} units opened, {} closed.",
            imported.provinces,
            imported.communes,
            manifest.title,
            manifest.effective_from,
            imported.opened,
            imported.closed
        )
    } else {
        "This list was already imported; nothing changed.".to_owned()
    }
}
