//! Choosing which modules run (`AUTOXIDE_MODULES`, step 70 P1 and P3, D-20261121-module-selection).
//!
//! The binary has every module, but an installation runs only the modules it needs. [`Modules::select`]
//! takes a LIST of modules (from this repo or written outside it) together with the selection read from
//! configuration, and registers only the enabled modules. A disabled module does not enter [`Modules`]: no
//! routes, no runtime OpenAPI documentation, no MCP tools, no jobs, no event receiving, no migrations.
//! Tables it already has in the database are untouched - the data stays intact and re-enabling it picks up
//! where it left off.
//!
//! Three things a module declares itself (all have defaults, D-038):
//!
//! - [`Module::stability`]: tier `Official` (default) or `Experimental` (may change, must be enabled
//!   explicitly);
//! - [`Module::essential`]: the core cannot run without it (login, idempotency) - it cannot be disabled;
//! - [`Module::depends_on`]: modules it MUST have (`context.require::<…>()?`). Enabling a module also
//!   enables the modules it needs.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use sea_orm::sea_query::{Alias, Expr, Query};
use sea_orm::{ConnectionTrait, DbErr};

use super::{Module, ModuleError, Modules, module_name_problems};
use crate::config::Setting;
use crate::database::Database;

/// The module selection variable. Default `official`: a new installation runs only official modules.
pub const MODULES: Setting = Setting::new(
    "AUTOXIDE_MODULES",
    "Which modules run. A comma-separated list of: `core` (only the modules the core cannot run \
     without), `official` (every official module), `all` (official and experimental modules), a \
     module name to add it, `-name` to leave it out. A module that an enabled module depends on \
     is enabled with it. A module that is left out serves no routes, MCP tools, jobs or events; \
     its migrations run only on a database that already has its tables, and it still exports and \
     deletes a person's data there. See docs/MODULES.md for the list of modules.",
)
.with_default("official")
.with_example("official,bookings,-plans");

const CORE: &str = "core";
const OFFICIAL: &str = "official";
const ALL: &str = "all";
/// Keywords of the list; a module must not carry these names.
const KEYWORDS: [&str; 3] = [CORE, OFFICIAL, ALL];

/// The tier of a module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stability {
    /// Official: runs by default; the API stays compatible under the versioning policy.
    Official,
    /// Experimental: the API and tables may change between releases; runs only when explicitly enabled.
    Experimental,
}

impl Stability {
    /// The name for machines and documentation: `official` / `experimental`.
    pub fn as_str(self) -> &'static str {
        match self {
            Stability::Official => OFFICIAL,
            Stability::Experimental => "experimental",
        }
    }
}

impl fmt::Display for Stability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The starting set of a selection; each later set contains the one before.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Base {
    /// Only the modules the core must have.
    Core,
    Official,
    All,
}

/// The module selection read from `AUTOXIDE_MODULES` (not yet matched against the module list).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// `None`: the list has no keyword.
    base: Option<Base>,
    add: Vec<String>,
    remove: Vec<String>,
    /// Verbatim as the installer wrote it; `None` when it is the program's default.
    text: Option<String>,
}

impl Selection {
    /// Every module, including experimental ones - for the full contract (`docs/openapi.json`) and tests.
    pub fn all() -> Self {
        Selection { base: Some(Base::All), add: Vec::new(), remove: Vec::new(), text: None }
    }

    /// The program's default when the variable is not set: official modules.
    pub fn official() -> Self {
        Selection { base: Some(Base::Official), add: Vec::new(), remove: Vec::new(), text: None }
    }

    /// Reads the list the installer wrote. Errors say plainly how it must be written.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut selection = Selection {
            base: None,
            add: Vec::new(),
            remove: Vec::new(),
            text: Some(text.trim().to_owned()),
        };
        for entry in text.split(',').map(str::trim).filter(|entry| !entry.is_empty()) {
            match entry {
                CORE => selection.base = selection.base.max(Some(Base::Core)),
                OFFICIAL => selection.base = selection.base.max(Some(Base::Official)),
                ALL => selection.base = selection.base.max(Some(Base::All)),
                _ => match entry.strip_prefix('-') {
                    Some(name) => selection.remove.push(name.trim().to_owned()),
                    None => selection.add.push(entry.to_owned()),
                },
            }
        }
        if selection.base.is_none() && selection.add.is_empty() {
            return Err(if selection.remove.is_empty() {
                "is empty; write `core`, `official`, `all` or a list of module names".to_owned()
            } else {
                format!(
                    "only leaves modules out, from nothing; start the list with a set, e.g. \
                     `official,-{}`",
                    selection.remove.join(",-")
                )
            });
        }
        Ok(selection)
    }

    /// The installer set `AUTOXIDE_MODULES` explicitly (it is not the default).
    pub fn is_explicit(&self) -> bool {
        self.text.is_some()
    }
}

/// A module that is in the binary but does not run in this installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InactiveModule {
    pub name: &'static str,
    pub description: &'static str,
    pub stability: Stability,
}

/// A disabled module whose tables still exist in the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DormantModule {
    pub name: &'static str,
    pub stability: Stability,
    /// The module's tables that have rows (not counting job, event and migration history tables); empty
    /// means the tables exist but were never used.
    pub tables_with_data: Vec<String>,
    /// Placements (other than `main`) holding the module's data; only [`Modules::dormant_everywhere`] fills
    /// this in.
    pub placements: Vec<String>,
}

/// The result of selection: running modules, dormant modules, and modules enabled along because another
/// module needs them. Why modules could not be selected.
#[derive(Debug)]
pub enum SelectionError {
    /// `AUTOXIDE_MODULES` is written wrongly (the installer's error): the message says plainly what to fix.
    Setting(String),
    /// A module declares something wrong (the module author's error).
    Modules(ModuleError),
}

impl fmt::Display for SelectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SelectionError::Setting(problem) => write!(f, "{}: {problem}", MODULES.name),
            SelectionError::Modules(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for SelectionError {}

pub(super) struct Selected {
    pub(super) modules: Vec<Box<dyn Module>>,
    pub(super) inactive: Vec<InactiveModule>,
    /// The turned-off modules themselves, in registration order (dormant mode).
    pub(super) dormant: Vec<Box<dyn Module>>,
    /// `(module enabled along, module that needs it)`.
    pub(super) implied: Vec<(&'static str, &'static str)>,
}

/// An error in a module's declaration (the module author's error, not the installer's).
fn declaration_problems(modules: &[Box<dyn Module>]) -> Vec<String> {
    let names: Vec<&str> = modules.iter().map(|module| module.name()).collect();
    let mut problems = module_name_problems(&names);
    for (position, module) in modules.iter().enumerate() {
        let name = module.name();
        if KEYWORDS.contains(&name) {
            problems.push(format!(
                "module name `{name}` is a keyword of {} and cannot name a module",
                MODULES.name
            ));
        }
        for dependency in module.depends_on() {
            let Some(found) = names.iter().position(|other| *other == dependency) else {
                problems.push(format!(
                    "module `{name}` depends on `{dependency}`, which is not registered"
                ));
                continue;
            };
            if found >= position {
                problems.push(format!(
                    "module `{name}` depends on `{dependency}`; register `{dependency}` before \
                     `{name}`"
                ));
            }
            if module.stability() == Stability::Official
                && modules[found].stability() == Stability::Experimental
            {
                problems.push(format!(
                    "official module `{name}` depends on experimental module `{dependency}`; an \
                     official module may only depend on official modules"
                ));
            }
        }
    }
    problems
}

/// Selects from `modules` according to `selection`, keeping registration order.
pub(super) fn select(
    modules: Vec<Box<dyn Module>>,
    selection: &Selection,
) -> Result<Selected, SelectionError> {
    let problems = declaration_problems(&modules);
    if !problems.is_empty() {
        return Err(SelectionError::Modules(ModuleError::new(problems.join("; "))));
    }
    let names: Vec<&'static str> = modules.iter().map(|module| module.name()).collect();
    let dependencies: BTreeMap<&str, Vec<&'static str>> =
        modules.iter().map(|module| (module.name(), module.depends_on())).collect();
    let mut problems = Vec::new();

    // Tên gõ sai: nói rõ tên nào hợp lệ.
    let unknown: Vec<&str> = selection
        .add
        .iter()
        .chain(&selection.remove)
        .map(String::as_str)
        .filter(|name| !names.contains(name))
        .collect();
    if !unknown.is_empty() {
        return Err(SelectionError::Setting(format!(
            "unknown module{} `{}`. Modules: {}. Sets: `core`, `official`, `all`",
            if unknown.len() == 1 { "" } else { "s" },
            unknown.join("`, `"),
            names.join(", ")
        )));
    }

    // Module core buộc phải có: module tự khai `essential`, cùng mọi thứ chúng cần.
    let mut essential: BTreeMap<&str, Option<&str>> = modules
        .iter()
        .filter(|module| module.essential())
        .map(|module| (module.name(), None))
        .collect();
    let mut queue: Vec<&str> = essential.keys().copied().collect();
    while let Some(name) = queue.pop() {
        for dependency in dependencies.get(name).into_iter().flatten() {
            if !essential.contains_key(dependency) {
                essential.insert(dependency, Some(name));
                queue.push(dependency);
            }
        }
    }
    let removed: BTreeSet<&str> = selection.remove.iter().map(String::as_str).collect();
    for name in &removed {
        match essential.get(name) {
            Some(None) => problems
                .push(format!("`{name}` cannot be turned off: the core does not run without it")),
            Some(Some(needed_by)) => problems.push(format!(
                "`{name}` cannot be turned off: `{needed_by}`, which the core does not run \
                 without, depends on it"
            )),
            None => {}
        }
    }

    let base = selection.base.unwrap_or(Base::Core);
    let mut enabled: BTreeSet<&str> = modules
        .iter()
        .filter(|module| {
            let name = module.name();
            let in_base = match base {
                Base::Core => false,
                Base::Official => module.stability() == Stability::Official,
                Base::All => true,
            };
            essential.contains_key(name)
                || ((in_base || selection.add.iter().any(|added| added == name))
                    && !removed.contains(name))
        })
        .map(|module| module.name())
        .collect();

    // Bật kèm module mà module đang bật cần; module đó bị `-tên` tắt thì là lỗi của người cài.
    let mut implied = Vec::new();
    let mut blocked: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut queue: Vec<&str> = enabled.iter().copied().collect();
    while let Some(name) = queue.pop() {
        for dependency in dependencies.get(name).into_iter().flatten() {
            if removed.contains(dependency) && !essential.contains_key(dependency) {
                blocked.entry(dependency).or_default().insert(name);
            } else if enabled.insert(dependency) {
                implied
                    .push((*dependency, names.iter().copied().find(|n| *n == name).unwrap_or("")));
                queue.push(dependency);
            }
        }
    }
    for (dependency, dependents) in blocked {
        let dependents: Vec<&str> = dependents.into_iter().collect();
        problems.push(format!(
            "`-{dependency}` turns off a module that `{}` depend{} on; remove `-{dependency}`, \
             or leave {} out too (`-{}`)",
            dependents.join("`, `"),
            if dependents.len() == 1 { "s" } else { "" },
            if dependents.len() == 1 { "it" } else { "them" },
            dependents.join(",-"),
        ));
    }
    if !problems.is_empty() {
        return Err(SelectionError::Setting(problems.join("; ")));
    }

    let mut active = Vec::new();
    let mut inactive = Vec::new();
    let mut dormant = Vec::new();
    for module in modules {
        if enabled.contains(module.name()) {
            active.push(module);
        } else {
            inactive.push(InactiveModule {
                name: module.name(),
                description: module.description(),
                stability: module.stability(),
            });
            dormant.push(module);
        }
    }
    implied.sort_unstable();
    Ok(Selected { modules: active, inactive, dormant, implied })
}

/// The module `module`'s tables in `connection` that have rows - of tenant `tenant` if given (a table
/// without a `tenant_id` column is skipped), of anyone otherwise. Job, event and migration history tables
/// are not counted: their having rows does not mean the user has data in the module. `None`: the database
/// never had the module's tables.
pub(crate) async fn tables_with_data<C: ConnectionTrait>(
    connection: &C,
    module: &str,
    tenant: Option<crate::tenancy::TenantId>,
) -> Result<Option<Vec<String>>, DbErr> {
    if !crate::migration::module_installed(connection, &format!("{module}_migrations")).await? {
        return Ok(None);
    }
    let custom = |error: crate::transfer::TransferError| DbErr::Custom(error.to_string());
    let tables = crate::transfer::module_tables(connection, &[module]).await.map_err(custom)?;
    let infrastructure = [crate::event::table_name(module), crate::job::table_name(module)];
    let mut found = Vec::new();
    for table in tables {
        if infrastructure.contains(&table) {
            continue;
        }
        let mut query = Query::select();
        query.expr_as(Expr::val(1), Alias::new("present")).from(Alias::new(&table)).limit(1);
        if let Some(tenant) = tenant {
            let columns = crate::transfer::columns(connection, &table).await.map_err(custom)?;
            if !columns.iter().any(|column| column.name == "tenant_id") {
                continue;
            }
            query.cond_where(crate::transfer::tenant_is(tenant));
        }
        if connection.query_one(&query).await?.is_some() {
            found.push(table);
        }
    }
    Ok(Some(found))
}

impl Modules {
    /// Registers the modules of `modules` that `selection` enables (see the top of the file). This is the
    /// ONLY door the binary uses to register: the caller supplies the module list, so modules written
    /// outside the repo go through this same filter.
    pub fn select(
        modules: Vec<Box<dyn Module>>,
        selection: &Selection,
    ) -> Result<Self, SelectionError> {
        let selected = select(modules, selection)?;
        let mut registered = Modules::new(selected.modules).map_err(SelectionError::Modules)?;
        registered.selection_text = selection.text.clone();
        registered.inactive = selected.inactive;
        registered.dormant = selected.dormant;
        registered.implied = selected.implied;
        Ok(registered)
    }

    /// Modules that are in the binary but disabled, in registration order.
    pub fn inactive(&self) -> &[InactiveModule] {
        &self.inactive
    }

    /// `(module enabled along, module that needs it)`: a module the installer did not name but that runs
    /// because an enabled module depends on it.
    pub fn implied(&self) -> &[(&'static str, &'static str)] {
        &self.implied
    }

    /// Disabled modules whose tables `database` still has - an installation that once ran that module.
    /// Read-only; it changes nothing. For each module, it lists the tables that hold data so the installer
    /// knows whether disabling it loses a feature in use. It looks at ONE database only; to look at the
    /// placements too, use
    pub async fn dormant(&self, database: &Database) -> Result<Vec<DormantModule>, DbErr> {
        let mut dormant = Vec::new();
        for module in &self.inactive {
            if let Some(tables_with_data) =
                tables_with_data(database.read(), module.name, None).await?
            {
                dormant.push(DormantModule {
                    name: module.name,
                    stability: module.stability,
                    tables_with_data,
                    placements: Vec::new(),
                });
            }
        }
        Ok(dormant)
    }

    /// Like [`dormant`](Self::dormant) but looks at the control database AND the database of every
    /// placement (a tenant with its own database keeps their data there). Call after
    /// [`start`](Self::start). A placement that cannot be connected to is skipped and named in
    /// `unreachable` - a reminder must not block startup.
    pub async fn dormant_everywhere(&self) -> Result<(Vec<DormantModule>, Vec<String>), DbErr> {
        let registry = self.tenancy().ok_or_else(|| {
            DbErr::Custom("start the modules before looking at the placements".to_owned())
        })?;
        let mut dormant = self.dormant(registry.control()).await?;
        let mut unreachable = Vec::new();
        if self.inactive.is_empty() {
            return Ok((dormant, unreachable));
        }
        let placements = match self.placements(&registry) {
            Ok(placements) => placements.list().await.map_err(|e| DbErr::Custom(e.message())),
            Err(error) => Err(DbErr::Custom(error.to_string())),
        }?;
        for placement in placements {
            let id = crate::tenancy::PlacementId::from_uuid(placement.id);
            if id == crate::tenancy::MAIN_PLACEMENT {
                continue;
            }
            let Ok(database) = registry.placement_database(id).await else {
                unreachable.push(placement.name);
                continue;
            };
            for module in &self.inactive {
                let Some(tables) = tables_with_data(database.read(), module.name, None).await?
                else {
                    continue;
                };
                let position = match dormant.iter().position(|found| found.name == module.name) {
                    Some(position) => position,
                    None => {
                        dormant.push(DormantModule {
                            name: module.name,
                            stability: module.stability,
                            tables_with_data: Vec::new(),
                            placements: Vec::new(),
                        });
                        dormant.len() - 1
                    }
                };
                if !tables.is_empty() {
                    let entry = &mut dormant[position];
                    entry.placements.push(placement.name.clone());
                    for table in tables {
                        if !entry.tables_with_data.contains(&table) {
                            entry.tables_with_data.push(table);
                        }
                    }
                }
            }
        }
        Ok((dormant, unreachable))
    }

    /// The `AUTOXIDE_MODULES` value the installer set, verbatim; `None`: the program's default.
    pub fn selection_text(&self) -> Option<&str> {
        self.selection_text.as_deref()
    }

    /// The installer set `AUTOXIDE_MODULES` explicitly (it is not the program's default).
    pub fn selection_is_explicit(&self) -> bool {
        self.selection_text.is_some()
    }

    /// The configuration line that enables the extra modules `names` beyond the current selection - so the
    /// message gives exactly what the installer needs to add to `.env`.
    pub fn setting_enabling(&self, names: &[&str]) -> String {
        setting_enabling(self.selection_text.as_deref(), names)
    }
}

fn setting_enabling(configured: Option<&str>, names: &[&str]) -> String {
    let mut entries: Vec<&str> = configured
        .unwrap_or(OFFICIAL)
        .split(',')
        .map(str::trim)
        .filter(|entry| {
            !entry.is_empty()
                && entry.strip_prefix('-').is_none_or(|removed| !names.contains(&removed.trim()))
        })
        .collect();
    for name in names {
        if !entries.contains(name) {
            entries.push(name);
        }
    }
    format!("{}={}", MODULES.name, entries.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        name: &'static str,
        stability: Stability,
        essential: bool,
        depends_on: Vec<&'static str>,
    }

    impl Module for Fake {
        fn name(&self) -> &'static str {
            self.name
        }
        fn stability(&self) -> Stability {
            self.stability
        }
        fn essential(&self) -> bool {
            self.essential
        }
        fn depends_on(&self) -> Vec<&'static str> {
            self.depends_on.clone()
        }
    }

    fn fake(name: &'static str, depends_on: &[&'static str]) -> Fake {
        Fake {
            name,
            stability: Stability::Official,
            essential: false,
            depends_on: depends_on.to_vec(),
        }
    }

    /// `mail` depends on `auth` (core); `catalog` depends on `orders`; `support` is experimental.
    fn modules() -> Vec<Box<dyn Module>> {
        vec![
            Box::new(fake("mail", &[])),
            Box::new(Fake { essential: true, ..fake("auth", &["mail"]) }),
            Box::new(fake("catalog", &[])),
            Box::new(fake("orders", &["catalog"])),
            Box::new(Fake { stability: Stability::Experimental, ..fake("support", &["orders"]) }),
        ]
    }

    fn enabled(text: &str) -> Result<Vec<&'static str>, String> {
        let selected =
            select(modules(), &Selection::parse(text)?).map_err(|error| error.to_string())?;
        Ok(selected.modules.iter().map(|module| module.name()).collect())
    }

    #[test]
    fn ba_tap_core_official_all() {
        assert_eq!(enabled("core").unwrap(), ["mail", "auth"]);
        assert_eq!(enabled("official").unwrap(), ["mail", "auth", "catalog", "orders"]);
        assert_eq!(enabled("all").unwrap(), ["mail", "auth", "catalog", "orders", "support"]);
        let official = select(modules(), &Selection::official()).unwrap();
        assert_eq!(official.inactive.len(), 1);
        assert_eq!(official.inactive[0].name, "support");
        assert_eq!(official.inactive[0].stability, Stability::Experimental);
    }

    #[test]
    fn ten_module_bat_kem_module_no_can() {
        assert_eq!(enabled("orders").unwrap(), ["mail", "auth", "catalog", "orders"]);
        let selected = select(modules(), &Selection::parse(" support ").unwrap()).unwrap();
        assert_eq!(selected.implied, [("catalog", "orders"), ("orders", "support")]);
        assert_eq!(enabled("official, support").unwrap().len(), 5);
        assert_eq!(enabled("official,-orders").unwrap(), ["mail", "auth", "catalog"]);
        assert_eq!(enabled("all,-support,-orders,-catalog").unwrap(), ["mail", "auth"]);
    }

    #[test]
    fn go_sai_ten_thi_neu_ten_hop_le() {
        let error = enabled("official,order").unwrap_err();
        assert_eq!(
            error,
            "AUTOXIDE_MODULES: unknown module `order`. Modules: mail, auth, catalog, orders, support. Sets: `core`, \
             `official`, `all`"
        );
    }

    #[test]
    fn tat_module_ma_module_dang_bat_can_la_loi() {
        let error = enabled("official,-catalog").unwrap_err();
        assert_eq!(
            error,
            "AUTOXIDE_MODULES: `-catalog` turns off a module that `orders` depends on; remove `-catalog`, or leave \
             it out too (`-orders`)"
        );
    }

    #[test]
    fn module_core_khong_tat_duoc() {
        let error = enabled("official,-auth,-mail").unwrap_err();
        assert!(error.contains("`auth` cannot be turned off: the core does not run without it"));
        assert!(error.contains("`mail` cannot be turned off: `auth`, which the core"), "{error}");
    }

    #[test]
    fn danh_sach_rong_hay_chi_co_dau_tru_la_loi() {
        assert!(Selection::parse(" , ").unwrap_err().starts_with("is empty"));
        let error = Selection::parse("-orders").unwrap_err();
        assert!(error.contains("`official,-orders`"), "{error}");
    }

    #[test]
    fn khai_bao_sai_cua_module_bi_chan() {
        let wrong: Vec<Box<dyn Module>> = vec![
            Box::new(fake("orders", &["catalog", "ghost"])),
            Box::new(Fake { stability: Stability::Experimental, ..fake("catalog", &[]) }),
            Box::new(fake("all", &[])),
        ];
        let error = select(wrong, &Selection::all()).err().unwrap().to_string();
        assert!(error.contains("register `catalog` before `orders`"), "{error}");
        assert!(error.contains("depends on `ghost`, which is not registered"), "{error}");
        assert!(error.contains("official module `orders` depends on experimental"), "{error}");
        assert!(error.contains("`all` is a keyword"), "{error}");
    }

    #[test]
    fn dong_cau_hinh_goi_y_them_module() {
        assert_eq!(
            setting_enabling(None, &["bookings", "support"]),
            "AUTOXIDE_MODULES=official,bookings,support"
        );
        assert_eq!(
            setting_enabling(Some("core, content,-support"), &["support"]),
            "AUTOXIDE_MODULES=core,content,support"
        );
    }
}
