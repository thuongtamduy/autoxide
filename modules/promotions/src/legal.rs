//! Dữ liệu luật khuyến mại (D-222, D-225, luật số hai): trần giảm giá, giới hạn số ngày, quy tắc
//! thông báo — dữ liệu có ngày hiệu lực ở bảng mặt điều khiển, không viết cứng trong code. Gói "vn"
//! đi kèm (`data/vn/legal_limits.json`, `data/vn/notice_rules.json`) ghi nguồn, ngày đối chiếu và
//! ghi chú "cần đối chiếu".
//!
//! Nạp: `autoxide task promotions.load_legal_data [pack=vn]`. Chạy lại cùng gói không đổi gì (so
//! SHA-256); một gói thay hẳn dữ liệu của các vùng (jurisdiction) nó nêu, trong một transaction.

use autoxide_kernel::time::{Date, Timestamp};
use std::collections::BTreeSet;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, page_of};
use autoxide_kernel::task::{Task, TaskArgs, TaskArgument, TaskRun};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::entity::{legal_limit, legal_pack, notice_rule};
use crate::events;
use crate::model::{Kind, Program, ProgramKind, Scope};
use crate::service::{PromotionsService, now};

/// Gói đi kèm.
pub(crate) const BUNDLED_LIMITS: &str = include_str!("../data/vn/legal_limits.json");
pub(crate) const BUNDLED_RULES: &str = include_str!("../data/vn/notice_rules.json");

/// Dữ liệu luật đọc lại sau tối đa chừng này (dữ liệu đổi rất hiếm; nạp gói thì xoá ngay).
const CACHE_FOR: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct LimitsFile {
    code: String,
    version: i64,
    checked_on: String,
    source_note: String,
    limits: Vec<LimitRow>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RulesFile {
    code: String,
    version: i64,
    #[allow(dead_code)]
    checked_on: String,
    #[allow(dead_code)]
    source_note: String,
    rules: Vec<RuleRow>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct LimitRow {
    jurisdiction: String,
    promotion_kind: String,
    program_kind: String,
    max_bp: i32,
    max_days_per_program: Option<i32>,
    max_days_per_year: Option<i32>,
    effective_from: String,
    effective_to: Option<String>,
    legal_basis: String,
    source_url: String,
    note: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleRow {
    jurisdiction: String,
    promotion_kind: String,
    notice_required: bool,
    days_before: i32,
    authority: String,
    effective_from: String,
    effective_to: Option<String>,
    legal_basis: String,
    note: String,
}

/// Một hạn mức pháp luật.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct LegalLimit {
    pub jurisdiction: String,
    pub promotion_kind: String,
    pub program_kind: String,
    pub max_bp: i32,
    pub max_days_per_program: Option<i32>,
    pub max_days_per_year: Option<i32>,
    pub effective_from: Date,
    pub effective_to: Option<Date>,
    pub legal_basis: String,
    pub source_url: String,
    pub note: String,
}

/// Một quy tắc thông báo khuyến mại.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct NoticeRule {
    pub jurisdiction: String,
    pub promotion_kind: String,
    pub notice_required: bool,
    pub days_before: i32,
    /// `province_of_store` (Sở Công Thương nơi tổ chức) hay mã cơ quan cố định.
    pub authority: String,
    pub effective_from: Date,
    pub effective_to: Option<Date>,
    pub legal_basis: String,
    pub note: String,
}

fn covers(from: Date, to: Option<Date>, day: Date) -> bool {
    from <= day && to.is_none_or(|to| day < to)
}

/// Toàn bộ dữ liệu luật đã nạp.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LegalData {
    pub limits: Vec<LegalLimit>,
    pub rules: Vec<NoticeRule>,
}

impl LegalData {
    /// Hạn mức đang hiệu lực ngày `day` cho loại khuyến mại và loại chương trình.
    pub(crate) fn limit(
        &self,
        jurisdiction: &str,
        promotion_kind: &str,
        program_kind: ProgramKind,
        day: Date,
    ) -> Option<&LegalLimit> {
        self.limits.iter().find(|limit| {
            limit.jurisdiction == jurisdiction
                && limit.promotion_kind == promotion_kind
                && limit.program_kind == program_kind.as_str()
                && covers(limit.effective_from, limit.effective_to, day)
        })
    }

    /// Quy tắc thông báo đang hiệu lực ngày `day`.
    pub(crate) fn rule(
        &self,
        jurisdiction: &str,
        promotion_kind: &str,
        day: Date,
    ) -> Option<&NoticeRule> {
        self.rules.iter().find(|rule| {
            rule.jurisdiction == jurisdiction
                && rule.promotion_kind == promotion_kind
                && covers(rule.effective_from, rule.effective_to, day)
        })
    }

    /// Có hạn mức nào hiệu lực cho `jurisdiction` ngày `day` không.
    pub(crate) fn has_limits(&self, jurisdiction: &str, day: Date) -> bool {
        self.limits.iter().any(|limit| {
            limit.jurisdiction == jurisdiction
                && covers(limit.effective_from, limit.effective_to, day)
        })
    }
}

/// Loại khuyến mại theo luật của một chương trình (để tra hạn mức / quy tắc thông báo).
pub(crate) fn legal_kind(program: &Program) -> &'static str {
    match (program.kind, program.scope) {
        (Kind::GiftChoice, _) => "gift_value",
        (Kind::FreeShipping, _) | (_, Scope::Shipping) => "free_shipping",
        _ => "price_reduction",
    }
}

/// Bộ nhớ đệm dữ liệu luật của máy chủ.
#[derive(Clone, Default)]
pub(crate) struct LegalCache(Arc<RwLock<Option<Cached>>>);

/// Dữ liệu đã đọc kèm lúc đọc.
type Cached = (Instant, Arc<LegalData>);

impl LegalCache {
    pub(crate) fn clear(&self) {
        if let Ok(mut slot) = self.0.write() {
            *slot = None;
        }
    }

    fn fresh(&self) -> Option<Arc<LegalData>> {
        let slot = self.0.read().ok()?;
        let (loaded, data) = slot.as_ref()?;
        (loaded.elapsed() < CACHE_FOR).then(|| data.clone())
    }

    fn store(&self, data: Arc<LegalData>) {
        if let Ok(mut slot) = self.0.write() {
            *slot = Some((Instant::now(), data));
        }
    }
}

fn pack_invalid(reason: impl Into<String>) -> Error {
    Error::new(
        ErrorKind::Validation,
        "promotions.legal_pack_invalid",
        "The promotion law data pack is invalid: {reason}.",
    )
    .with_param("reason", reason.into())
}

fn date(text: &str) -> Option<Date> {
    Date::parse_from_str(text, "%Y-%m-%d").ok()
}

/// Gói đã đọc và kiểm.
struct Pack {
    code: String,
    version: i64,
    checked_on: String,
    source_note: String,
    limits: Vec<LegalLimit>,
    rules: Vec<NoticeRule>,
    sha: String,
}

/// Đọc và kiểm một gói (hai file); báo mọi chỗ sai một lần (tối đa 20).
fn parse_pack(limits_text: &str, rules_text: &str) -> Result<Pack, Error> {
    let limits: LimitsFile = serde_json::from_str(limits_text)
        .map_err(|error| pack_invalid(format!("legal_limits.json: {error}")))?;
    let rules: RulesFile = serde_json::from_str(rules_text)
        .map_err(|error| pack_invalid(format!("notice_rules.json: {error}")))?;
    let mut problems: Vec<String> = Vec::new();
    if limits.code != rules.code || limits.version != rules.version {
        problems.push("both files must have the same `code` and `version`".to_owned());
    }
    if limits.code.is_empty() || limits.code.len() > 32 || limits.version < 1 {
        problems.push("`code` must be 1 to 32 characters and `version` at least 1".to_owned());
    }
    let window = |from: &str, to: Option<&str>, what: &str, problems: &mut Vec<String>| match (
        date(from),
        to.map(date),
    ) {
        (Some(from), Some(Some(to))) if to <= from => {
            problems.push(format!("{what}: `effective_to` must be after `effective_from`"));
            None
        }
        (Some(from), Some(Some(to))) => Some((from, Some(to))),
        (Some(from), None) => Some((from, None)),
        _ => {
            problems.push(format!("{what}: dates must be `YYYY-MM-DD`"));
            None
        }
    };
    let mut parsed_limits = Vec::new();
    let mut seen = BTreeSet::new();
    for (index, row) in limits.limits.iter().enumerate() {
        let what = format!("limit {index} ({} {})", row.promotion_kind, row.program_kind);
        if !(0..=10_000).contains(&row.max_bp) {
            problems.push(format!("{what}: `max_bp` must be 0 to 10000"));
        }
        if ProgramKind::parse(&row.program_kind).is_none() {
            problems.push(format!("{what}: `program_kind` must be regular or national_focus"));
        }
        if row.max_days_per_program.is_some_and(|days| days < 1)
            || row.max_days_per_year.is_some_and(|days| days < 1)
        {
            problems.push(format!("{what}: day limits must be at least 1"));
        }
        if !seen.insert((
            &row.jurisdiction,
            &row.promotion_kind,
            &row.program_kind,
            &row.effective_from,
        )) {
            problems.push(format!("{what}: listed twice"));
        }
        if let Some((from, to)) =
            window(&row.effective_from, row.effective_to.as_deref(), &what, &mut problems)
        {
            parsed_limits.push(LegalLimit {
                jurisdiction: row.jurisdiction.clone(),
                promotion_kind: row.promotion_kind.clone(),
                program_kind: row.program_kind.clone(),
                max_bp: row.max_bp,
                max_days_per_program: row.max_days_per_program,
                max_days_per_year: row.max_days_per_year,
                effective_from: from,
                effective_to: to,
                legal_basis: row.legal_basis.clone(),
                source_url: row.source_url.clone(),
                note: row.note.clone(),
            });
        }
    }
    // Hai hạn mức cùng loại không được chồng ngày.
    for (index, a) in parsed_limits.iter().enumerate() {
        for b in parsed_limits.iter().skip(index + 1) {
            if a.jurisdiction == b.jurisdiction
                && a.promotion_kind == b.promotion_kind
                && a.program_kind == b.program_kind
                && a.effective_to.is_none_or(|to| to > b.effective_from)
                && b.effective_to.is_none_or(|to| to > a.effective_from)
            {
                problems.push(format!(
                    "limits of {} {} overlap in time",
                    a.promotion_kind, a.program_kind
                ));
            }
        }
    }
    let mut parsed_rules = Vec::new();
    for (index, row) in rules.rules.iter().enumerate() {
        let what = format!("rule {index} ({})", row.promotion_kind);
        if row.days_before < 0 || row.authority.is_empty() {
            problems
                .push(format!("{what}: `days_before` must not be negative, `authority` not empty"));
        }
        if let Some((from, to)) =
            window(&row.effective_from, row.effective_to.as_deref(), &what, &mut problems)
        {
            parsed_rules.push(NoticeRule {
                jurisdiction: row.jurisdiction.clone(),
                promotion_kind: row.promotion_kind.clone(),
                notice_required: row.notice_required,
                days_before: row.days_before,
                authority: row.authority.clone(),
                effective_from: from,
                effective_to: to,
                legal_basis: row.legal_basis.clone(),
                note: row.note.clone(),
            });
        }
    }
    if !problems.is_empty() {
        let fields = problems
            .into_iter()
            .take(20)
            .map(|problem| {
                FieldError::new("pack", "invalid", "{problem}").with_param("problem", problem)
            })
            .collect();
        return Err(Error::validation(fields));
    }
    let canonical = |text: &str| {
        serde_json::from_str::<serde_json::Value>(text)
            .map(|value| value.to_string())
            .unwrap_or_default()
    };
    let sha = autoxide_kernel::secret::sha256(&format!(
        "{}\n{}",
        canonical(limits_text),
        canonical(rules_text)
    ));
    Ok(Pack {
        code: limits.code,
        version: limits.version,
        checked_on: limits.checked_on,
        source_note: limits.source_note,
        limits: parsed_limits,
        rules: parsed_rules,
        sha,
    })
}

/// Kết quả một lần nạp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Imported {
    pub changed: bool,
    pub version: i64,
    pub limits: usize,
}

impl PromotionsService {
    /// Dữ liệu luật (bộ nhớ đệm, đọc lại sau [`CACHE_FOR`]).
    pub(crate) async fn legal(&self) -> Result<Arc<LegalData>, Error> {
        if let Some(data) = self.legal_cache.fresh() {
            return Ok(data);
        }
        let read = self.databases.control().read();
        let limits = legal_limit::Entity::find()
            .order_by_asc(legal_limit::Column::EffectiveFrom)
            .all(read)
            .await?
            .into_iter()
            .filter_map(|row| {
                Some(LegalLimit {
                    effective_from: date(&row.effective_from)?,
                    effective_to: row.effective_to.as_deref().and_then(date),
                    jurisdiction: row.jurisdiction,
                    promotion_kind: row.promotion_kind,
                    program_kind: row.program_kind,
                    max_bp: row.max_bp,
                    max_days_per_program: row.max_days_per_program,
                    max_days_per_year: row.max_days_per_year,
                    legal_basis: row.legal_basis,
                    source_url: row.source_url,
                    note: row.note,
                })
            })
            .collect();
        let rules = notice_rule::Entity::find()
            .order_by_asc(notice_rule::Column::EffectiveFrom)
            .all(read)
            .await?
            .into_iter()
            .filter_map(|row| {
                Some(NoticeRule {
                    effective_from: date(&row.effective_from)?,
                    effective_to: row.effective_to.as_deref().and_then(date),
                    jurisdiction: row.jurisdiction,
                    promotion_kind: row.promotion_kind,
                    notice_required: row.notice_required,
                    days_before: row.days_before,
                    authority: row.authority,
                    legal_basis: row.legal_basis,
                    note: row.note,
                })
            })
            .collect();
        let data = Arc::new(LegalData { limits, rules });
        self.legal_cache.store(data.clone());
        Ok(data)
    }

    /// Nạp một gói (dạng chữ của hai file). Cùng nội dung đã nạp: không đổi gì.
    pub(crate) async fn import_legal(
        &self,
        actor: Actor,
        limits_text: &str,
        rules_text: &str,
    ) -> Result<Imported, Error> {
        let pack = parse_pack(limits_text, rules_text)?;
        let control = self.databases.control();
        let current = legal_pack::Entity::find()
            .filter(legal_pack::Column::Code.eq(pack.code.clone()))
            .order_by_desc(legal_pack::Column::Version)
            .one(control.read())
            .await?;
        if let Some(current) = &current {
            if current.content_sha256 == pack.sha {
                return Ok(Imported { changed: false, version: current.version, limits: 0 });
            }
            if pack.version <= current.version {
                return Err(Error::new(
                    ErrorKind::Conflict,
                    "promotions.legal_pack_outdated",
                    "Pack `{code}` version {current} is already loaded; only a higher version is \
                     accepted.",
                )
                .with_param("code", &pack.code)
                .with_param("current", current.version));
            }
        }
        let jurisdictions: Vec<String> = pack
            .limits
            .iter()
            .map(|limit| limit.jurisdiction.clone())
            .chain(pack.rules.iter().map(|rule| rule.jurisdiction.clone()))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let transaction = control.begin_write().await?;
        legal_limit::Entity::delete_many()
            .filter(legal_limit::Column::Jurisdiction.is_in(jurisdictions.clone()))
            .exec(&transaction)
            .await?;
        notice_rule::Entity::delete_many()
            .filter(notice_rule::Column::Jurisdiction.is_in(jurisdictions))
            .exec(&transaction)
            .await?;
        for limit in &pack.limits {
            legal_limit::Entity::insert(legal_limit::ActiveModel {
                id: Set(Uuid::now_v7()),
                jurisdiction: Set(limit.jurisdiction.clone()),
                promotion_kind: Set(limit.promotion_kind.clone()),
                program_kind: Set(limit.program_kind.clone()),
                max_bp: Set(limit.max_bp),
                max_days_per_program: Set(limit.max_days_per_program),
                max_days_per_year: Set(limit.max_days_per_year),
                effective_from: Set(limit.effective_from.to_string()),
                effective_to: Set(limit.effective_to.map(|to| to.to_string())),
                legal_basis: Set(limit.legal_basis.clone()),
                source_url: Set(limit.source_url.clone()),
                note: Set(limit.note.clone()),
                pack_version: Set(pack.version),
            })
            .exec_without_returning(&transaction)
            .await?;
        }
        for rule in &pack.rules {
            notice_rule::Entity::insert(notice_rule::ActiveModel {
                id: Set(Uuid::now_v7()),
                jurisdiction: Set(rule.jurisdiction.clone()),
                promotion_kind: Set(rule.promotion_kind.clone()),
                notice_required: Set(rule.notice_required),
                days_before: Set(rule.days_before),
                authority: Set(rule.authority.clone()),
                effective_from: Set(rule.effective_from.to_string()),
                effective_to: Set(rule.effective_to.map(|to| to.to_string())),
                legal_basis: Set(rule.legal_basis.clone()),
                note: Set(rule.note.clone()),
                pack_version: Set(pack.version),
            })
            .exec_without_returning(&transaction)
            .await?;
        }
        legal_pack::Entity::insert(legal_pack::ActiveModel {
            id: Set(Uuid::now_v7()),
            code: Set(pack.code.clone()),
            version: Set(pack.version),
            checked_on: Set(pack.checked_on.clone()),
            source_note: Set(pack.source_note.clone()),
            content_sha256: Set(pack.sha.clone()),
            imported_at: Set(now()),
        })
        .exec_without_returning(&transaction)
        .await?;
        let platform = self.databases.default_tenant();
        self.outbox
            .record(
                &transaction,
                &platform,
                actor,
                Some(format!("promotions.legal_pack:{}", pack.code)),
                &events::LegalDataImported(events::Change::after(json!({
                    "code": pack.code, "version": pack.version, "limits": pack.limits.len(),
                }))),
            )
            .await?;
        transaction.commit().await?;
        self.legal_cache.clear();
        tracing::info!(code = %pack.code, version = pack.version, "promotion law data pack loaded");
        Ok(Imported { changed: true, version: pack.version, limits: pack.limits.len() })
    }
}

/// `autoxide task promotions.load_legal_data [pack=vn] [limits=<file> rules=<file>]`.
pub(crate) fn task(service: Arc<PromotionsService>) -> Task {
    Task::new(
        "load_legal_data",
        "Load the promotion law data (discount limits, day limits, notice rules with their \
         effective dates). Loading the same pack again changes nothing; only a higher version \
         than the one loaded is accepted.",
        Arc::new(LoadTask(service)),
    )
    .argument(TaskArgument::optional("pack", "Pack shipped with the server: `vn` (the default)."))
    .argument(TaskArgument::optional("limits", "JSON file of the limits of your own pack."))
    .argument(TaskArgument::optional("rules", "JSON file of the notice rules of your own pack."))
}

struct LoadTask(Arc<PromotionsService>);

#[async_trait::async_trait]
impl TaskRun for LoadTask {
    async fn run(&self, args: &TaskArgs) -> Result<String, Error> {
        let read = |path: &str| {
            std::fs::read_to_string(path).map_err(|error| {
                Error::bad_request(
                    "promotions.legal_pack_unreadable",
                    "Cannot read `{file}`: {reason}.",
                )
                .with_param("file", path)
                .with_param("reason", error.to_string())
            })
        };
        let (limits, rules) = match (args.get("limits"), args.get("rules")) {
            (Some(limits), Some(rules)) => (read(limits)?, read(rules)?),
            (None, None) => match args.get("pack").unwrap_or("vn") {
                "vn" => (BUNDLED_LIMITS.to_owned(), BUNDLED_RULES.to_owned()),
                other => {
                    return Err(Error::bad_request(
                        "promotions.legal_pack_unknown",
                        "There is no bundled pack `{pack}`; the server ships `vn`.",
                    )
                    .with_param("pack", other));
                }
            },
            _ => {
                return Err(Error::bad_request(
                    "promotions.legal_pack_unreadable",
                    "Give both `limits` and `rules` files, or neither.",
                ));
            }
        };
        let imported = self.0.import_legal(Actor::system(), &limits, &rules).await?;
        Ok(if imported.changed {
            format!(
                "Loaded promotion law pack version {} ({} limits).",
                imported.version, imported.limits
            )
        } else {
            format!("Promotion law pack version {} is already loaded.", imported.version)
        })
    }
}

/// `autoxide doctor`: cảnh báo khi chưa có hạn mức nào hiệu lực, hay hạn mức hết hiệu lực trong
/// 30 ngày mà chưa có dòng kế tiếp.
pub(crate) struct LegalCheck(pub Arc<PromotionsService>);

#[async_trait::async_trait]
impl autoxide_kernel::doctor::Probe for LegalCheck {
    async fn probe(&self) -> autoxide_kernel::doctor::Finding {
        use autoxide_kernel::doctor::Finding;
        let jurisdiction = self.0.config.jurisdiction.clone();
        let today = Timestamp::now().date_utc();
        match self.0.legal().await {
            Err(error) => Finding::failed(format!("Cannot read the promotion law data: {error}.")),
            Ok(data) if !data.has_limits(&jurisdiction, today) => Finding::warning(format!(
                "No promotion law limit is in force for `{jurisdiction}`: promotions cannot be \
                 turned on."
            ))
            .with_hint("Run `autoxide task promotions.load_legal_data`."),
            Ok(data) => {
                let soon = today.checked_add_days(30).unwrap_or(today);
                let ending = data.limits.iter().find(|limit| {
                    limit.jurisdiction == jurisdiction
                        && limit.effective_to.is_some_and(|to| to > today && to <= soon)
                        && !data.limits.iter().any(|next| {
                            next.promotion_kind == limit.promotion_kind
                                && next.program_kind == limit.program_kind
                                && Some(next.effective_from) == limit.effective_to
                        })
                });
                match ending {
                    Some(limit) => Finding::warning(format!(
                        "The promotion limit `{} {}` ends on {} with nothing after it.",
                        limit.promotion_kind,
                        limit.program_kind,
                        limit.effective_to.map(|to| to.to_string()).unwrap_or_default()
                    ))
                    .with_hint("Load a newer promotion law data pack."),
                    None => Finding::ok("Promotion law limits are loaded."),
                }
            }
        }
    }
}

impl PromotionsService {
    /// Hạn mức hiệu lực ngày `day` (mặc định hôm nay theo giờ Việt Nam).
    ///
    /// Một trang (`page_of`): dữ liệu luật dùng chung mọi khách, nạp bằng task của người vận hành (một gói vài
    /// chục dòng), không lớn dần theo dữ liệu khách — như bảng thuế (D-20261126-list-pages-cluster-3). Theo loại
    /// khuyến mại, loại chương trình, vùng; khoá kèm id nên duy nhất.
    pub(crate) async fn legal_limits_at(
        &self,
        day: Option<Date>,
        page: &PageRequest,
    ) -> Result<Page<legal_limit::Model>, Error> {
        let day = day.unwrap_or_else(|| crate::compliance::legal_day(Timestamp::now())).to_string();
        let rows = legal_limit::Entity::find()
            .filter(legal_limit::Column::EffectiveFrom.lte(day.clone()))
            .filter(
                sea_orm::Condition::any()
                    .add(legal_limit::Column::EffectiveTo.is_null())
                    .add(legal_limit::Column::EffectiveTo.gt(day)),
            )
            .all(self.databases.control().read())
            .await?;
        page_of(
            rows,
            |row| {
                format!(
                    "{}\u{1f}{}\u{1f}{}\u{1f}{}",
                    row.promotion_kind, row.program_kind, row.jurisdiction, row.id
                )
            },
            page,
        )
    }

    /// Quy tắc thông báo hiệu lực ngày `day`: một trang (`page_of`, cùng lý do với
    /// [`Self::legal_limits_at`]), theo loại khuyến mại rồi vùng.
    pub(crate) async fn notice_rules_at(
        &self,
        day: Option<Date>,
        page: &PageRequest,
    ) -> Result<Page<notice_rule::Model>, Error> {
        let day = day.unwrap_or_else(|| crate::compliance::legal_day(Timestamp::now())).to_string();
        let rows = notice_rule::Entity::find()
            .filter(notice_rule::Column::EffectiveFrom.lte(day.clone()))
            .filter(
                sea_orm::Condition::any()
                    .add(notice_rule::Column::EffectiveTo.is_null())
                    .add(notice_rule::Column::EffectiveTo.gt(day)),
            )
            .all(self.databases.control().read())
            .await?;
        page_of(
            rows,
            |row| format!("{}\u{1f}{}\u{1f}{}", row.promotion_kind, row.jurisdiction, row.id),
            page,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goi_vn_hop_le_va_tran_doi_theo_ngay() {
        let pack = parse_pack(BUNDLED_LIMITS, BUNDLED_RULES).unwrap();
        let data = LegalData { limits: pack.limits, rules: pack.rules };
        let day = |text: &str| Date::parse_from_str(text, "%Y-%m-%d").unwrap();
        let before =
            data.limit("VN", "price_reduction", ProgramKind::Regular, day("2024-11-30")).unwrap();
        let after =
            data.limit("VN", "price_reduction", ProgramKind::Regular, day("2024-12-01")).unwrap();
        assert_eq!((before.max_days_per_year, after.max_days_per_year), (Some(90), Some(120)));
        assert_eq!(
            data.limit("VN", "price_reduction", ProgramKind::NationalFocus, day("2026-11-18"))
                .unwrap()
                .max_bp,
            10_000
        );
        assert!(data.rule("VN", "purchase_voucher", day("2026-11-18")).unwrap().notice_required);
    }

    #[test]
    fn goi_sai_bao_mot_lan() {
        let limits = r#"{"code":"x","version":1,"checked_on":"","source_note":"","limits":[
          {"jurisdiction":"XX","promotion_kind":"price_reduction","program_kind":"odd","max_bp":20000,
           "max_days_per_program":0,"max_days_per_year":null,"effective_from":"2026-13-01",
           "effective_to":null,"legal_basis":"","source_url":"","note":""}]}"#;
        let rules = r#"{"code":"x","version":2,"checked_on":"","source_note":"","rules":[]}"#;
        let error = parse_pack(limits, rules).err().unwrap();
        assert!(error.fields().len() >= 4, "{:?}", error.fields());
    }
}
