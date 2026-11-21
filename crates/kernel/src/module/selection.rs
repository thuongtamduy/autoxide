//! Chọn module nào chạy (`AUTOXIDE_MODULES`, bước 70 P1 và P3, D-20261121-module-selection).
//!
//! Binary có sẵn mọi module, nhưng một bản cài chỉ chạy những module nó cần. [`Modules::select`]
//! nhận DANH SÁCH module (của repo này hay viết ngoài repo) cùng lựa chọn đã đọc từ cấu hình, và
//! chỉ đăng ký những module được bật. Module tắt không vào [`Modules`]: không route, không tài liệu
//! OpenAPI lúc chạy, không tool MCP, không job, không nhận event, không migration. Bảng nó đã có
//! trong database không ai chạm tới — dữ liệu còn nguyên, bật lại là dùng tiếp.
//!
//! Ba điều module tự khai (đều có mặc định, D-038):
//!
//! - [`Module::stability`]: hạng `Official` (mặc định) hay `Experimental` (có thể đổi, phải bật rõ);
//! - [`Module::essential`]: core không chạy được nếu thiếu (đăng nhập, idempotency) — không tắt được;
//! - [`Module::depends_on`]: module mà nó BUỘC phải có (`context.require::<…>()?`). Bật một module
//!   thì module nó cần được bật kèm.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use sea_orm::sea_query::{Alias, Expr, Query};
use sea_orm::{ConnectionTrait, DbErr};

use super::{Module, ModuleError, Modules, module_name_problems};
use crate::config::Setting;
use crate::database::Database;

/// Biến chọn module. Mặc định `official`: bản cài mới chỉ chạy module chính thức.
pub const MODULES: Setting = Setting::new(
    "AUTOXIDE_MODULES",
    "Which modules run. A comma-separated list of: `core` (only the modules the core cannot run \
     without), `official` (every official module), `all` (official and experimental modules), a \
     module name to add it, `-name` to leave it out. A module that an enabled module depends on \
     is enabled with it. A module that is left out serves no routes, MCP tools, jobs or events \
     and its migrations do not run; data it already has in the database is kept untouched. See \
     docs/MODULES.md for the list of modules.",
)
.with_default("official")
.with_example("official,bookings,-plans");

const CORE: &str = "core";
const OFFICIAL: &str = "official";
const ALL: &str = "all";
/// Từ khoá của danh sách; module không được mang các tên này.
const KEYWORDS: [&str; 3] = [CORE, OFFICIAL, ALL];

/// Hạng của một module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stability {
    /// Chính thức: chạy mặc định; API giữ tương thích theo chính sách phiên bản.
    Official,
    /// Thử nghiệm: API và bảng có thể đổi giữa các bản; chỉ chạy khi được bật rõ.
    Experimental,
}

impl Stability {
    /// Tên cho máy và tài liệu: `official` / `experimental`.
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

/// Tập khởi đầu của một lựa chọn; tập sau chứa tập trước.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Base {
    /// Chỉ module core buộc phải có.
    Core,
    Official,
    All,
}

/// Lựa chọn module đã đọc từ `AUTOXIDE_MODULES` (chưa đối chiếu với danh sách module).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// `None`: danh sách không có từ khoá nào.
    base: Option<Base>,
    add: Vec<String>,
    remove: Vec<String>,
    /// Nguyên văn người cài viết; `None` khi là mặc định của chương trình.
    text: Option<String>,
}

impl Selection {
    /// Mọi module, kể cả thử nghiệm — cho hợp đồng đầy đủ (`docs/openapi.json`) và test.
    pub fn all() -> Self {
        Selection { base: Some(Base::All), add: Vec::new(), remove: Vec::new(), text: None }
    }

    /// Mặc định của chương trình khi biến không được đặt: module chính thức.
    pub fn official() -> Self {
        Selection { base: Some(Base::Official), add: Vec::new(), remove: Vec::new(), text: None }
    }

    /// Đọc danh sách do người cài viết. Lỗi nói rõ phải viết thế nào.
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

    /// Người cài tự đặt `AUTOXIDE_MODULES` (không phải mặc định).
    pub fn is_explicit(&self) -> bool {
        self.text.is_some()
    }
}

/// Một module có trong binary nhưng không chạy ở bản cài này.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InactiveModule {
    pub name: &'static str,
    pub description: &'static str,
    pub stability: Stability,
}

/// Module đang tắt mà database vẫn còn bảng của nó.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DormantModule {
    pub name: &'static str,
    pub stability: Stability,
    /// Bảng của module đang có dòng (không tính bảng job, event và lịch sử migration); rỗng nghĩa
    /// là bảng có đó nhưng chưa từng được dùng.
    pub tables_with_data: Vec<String>,
}

/// Kết quả chọn: module chạy, module nghỉ, và module được bật kèm vì module khác cần.
/// Vì sao không chọn được module.
#[derive(Debug)]
pub enum SelectionError {
    /// `AUTOXIDE_MODULES` viết sai (lỗi của người cài): câu nói rõ phải sửa gì.
    Setting(String),
    /// Module khai sai (lỗi của người viết module).
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
    /// `(module được bật kèm, module cần nó)`.
    pub(super) implied: Vec<(&'static str, &'static str)>,
}

/// Lỗi trong khai báo của module (lỗi của người viết module, không phải của người cài).
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

/// Chọn trong `modules` theo `selection`, giữ thứ tự đăng ký.
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
    for module in modules {
        if enabled.contains(module.name()) {
            active.push(module);
        } else {
            inactive.push(InactiveModule {
                name: module.name(),
                description: module.description(),
                stability: module.stability(),
            });
        }
    }
    implied.sort_unstable();
    Ok(Selected { modules: active, inactive, implied })
}

impl Modules {
    /// Đăng ký những module của `modules` mà `selection` bật (xem đầu file). Đây là cửa DUY NHẤT
    /// binary dùng để đăng ký: danh sách module do bên gọi đưa vào, nên module viết ngoài repo đi
    /// qua đúng bộ lọc này.
    pub fn select(
        modules: Vec<Box<dyn Module>>,
        selection: &Selection,
    ) -> Result<Self, SelectionError> {
        let selected = select(modules, selection)?;
        let mut registered = Modules::new(selected.modules).map_err(SelectionError::Modules)?;
        registered.selection_text = selection.text.clone();
        registered.inactive = selected.inactive;
        registered.implied = selected.implied;
        Ok(registered)
    }

    /// Module có trong binary nhưng đang tắt, theo thứ tự đăng ký.
    pub fn inactive(&self) -> &[InactiveModule] {
        &self.inactive
    }

    /// `(module được bật kèm, module cần nó)`: module người cài không nêu nhưng chạy vì một module
    /// đang bật phụ thuộc vào nó.
    pub fn implied(&self) -> &[(&'static str, &'static str)] {
        &self.implied
    }

    /// Module đang tắt mà `database` còn bảng của nó — bản cài từng chạy module đó. Chỉ đọc; không
    /// đổi gì. Với mỗi module, nêu bảng đang có dữ liệu để người cài biết tắt nó có làm mất chức
    /// năng đang dùng không.
    pub async fn dormant(&self, database: &Database) -> Result<Vec<DormantModule>, DbErr> {
        let connection = database.read();
        let mut dormant = Vec::new();
        for module in &self.inactive {
            let history = format!("{}_migrations", module.name);
            if !crate::migration::table_exists(connection, &history).await? {
                continue;
            }
            let tables = crate::transfer::module_tables(connection, &[module.name])
                .await
                .map_err(|error| DbErr::Custom(error.to_string()))?;
            // Bảng hạ tầng của kernel có dòng không có nghĩa là người dùng có dữ liệu ở module.
            let infrastructure =
                [crate::event::table_name(module.name), crate::job::table_name(module.name)];
            let mut tables_with_data = Vec::new();
            for table in tables {
                if infrastructure.contains(&table) {
                    continue;
                }
                let mut query = Query::select();
                query
                    .expr_as(Expr::val(1), Alias::new("present"))
                    .from(Alias::new(&table))
                    .limit(1);
                if connection.query_one(&query).await?.is_some() {
                    tables_with_data.push(table);
                }
            }
            dormant.push(DormantModule {
                name: module.name,
                stability: module.stability,
                tables_with_data,
            });
        }
        Ok(dormant)
    }

    /// Người cài tự đặt `AUTOXIDE_MODULES` (không phải mặc định của chương trình).
    pub fn selection_is_explicit(&self) -> bool {
        self.selection_text.is_some()
    }

    /// Dòng cấu hình bật thêm các module `names` so với lựa chọn hiện tại — để thông báo đưa đúng
    /// thứ người cài cần thêm vào `.env`.
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

    /// `mail` ← `auth` (core); `catalog` ← `orders`; `support` thử nghiệm.
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
