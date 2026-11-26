//! Gói dữ liệu thuế (đặc tả 36 "Engine thuế"): thuế suất, khấu trừ thay, quy tắc có ngày hiệu lực —
//! luật đổi thì nhập gói mới, không sửa code. Gói "vn" đi kèm (`packs/vn.json`).
//!
//! Nhập: `autoxide task tax.import_pack [path=<file>]`, `POST /api/v1/tax/packs`, hay job hằng giờ tự
//! nhập gói đi kèm khi nó mới hơn bản trong database. Chỉ nhận `version` lớn hơn bản đang có
//! (`tax.pack_outdated`); cùng nội dung (SHA-256) đã nhập thì không đổi gì. Một gói thay hẳn dữ liệu
//! của các vùng thuế nó nêu, trong một transaction.

use autoxide_kernel::time::Date;
use std::collections::BTreeSet;
use std::sync::Arc;

use autoxide_kernel::decimal::{Decimal, parse, to_column};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::task::{Task, TaskArgs, TaskArgument, TaskRun};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::engine::Basis;
use crate::entity::{category, jurisdiction, pack, rate, rule, withholding};
use crate::events;
use crate::service::{TaxService, now};

/// Gói đi kèm.
pub(crate) const BUNDLED_VN: &str = include_str!("../packs/vn.json");

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Pack {
    pub code: String,
    pub version: i64,
    pub checked_on: String,
    pub source_note: String,
    pub jurisdictions: Vec<PackJurisdiction>,
    pub categories: Vec<PackCategory>,
    pub rates: Vec<PackRate>,
    #[serde(default)]
    pub withholding_rates: Vec<PackWithholding>,
    #[serde(default)]
    pub rules: Vec<PackRule>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PackJurisdiction {
    pub code: String,
    pub name: String,
    pub default_currency: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PackCategory {
    pub jurisdiction: String,
    pub code: String,
    pub name_en: String,
    pub name_vi: String,
    pub tax_kind: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PackRate {
    pub jurisdiction: String,
    pub category_code: String,
    pub tax_kind: String,
    pub basis: String,
    pub rate_percent: String,
    #[serde(default)]
    pub per_unit: Option<Money>,
    pub stage: i32,
    pub effective_from: String,
    pub effective_to: Option<String>,
    pub legal_basis: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PackWithholding {
    pub jurisdiction: String,
    pub payer_kind: String,
    pub payee_regime: String,
    pub business_line: String,
    pub tax_kind: String,
    pub rate_percent: String,
    #[serde(default)]
    pub threshold: Option<Money>,
    pub effective_from: String,
    pub effective_to: Option<String>,
    pub legal_basis: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PackRule {
    pub jurisdiction: String,
    pub code: String,
    pub value: serde_json::Value,
    pub effective_from: String,
    pub effective_to: Option<String>,
    pub legal_basis: String,
}

pub(crate) fn pack_invalid(reason: impl Into<String>) -> Error {
    Error::new(ErrorKind::Validation, "tax.pack_invalid", "The tax data pack is invalid: {reason}.")
        .with_param("reason", reason.into())
}

fn outdated(code: &str, current: i64) -> Error {
    Error::new(
        ErrorKind::Conflict,
        "tax.pack_outdated",
        "Pack `{code}` version {current} is already imported; only a higher version is accepted.",
    )
    .with_param("code", code)
    .with_param("current", current)
}

fn date(text: &str) -> Option<Date> {
    Date::parse_from_str(text, "%Y-%m-%d").ok()
}

/// Đọc và kiểm một gói; báo mọi chỗ sai một lần (tối đa 20).
/// Trần thuế suất trong gói (phần trăm). Thuế suất cao nhất trong luật đang biết là vài trăm phần trăm
/// (TTĐB); một con số lớn vô lý là lỗi nhập liệu và làm tràn số khi tính.
const MAX_RATE_PERCENT: i64 = 1_000;

fn rate_in_range(rate: Decimal) -> bool {
    !rate.is_sign_negative() && rate <= Decimal::from(MAX_RATE_PERCENT)
}

pub(crate) fn parse_pack(text: &str) -> Result<Pack, Error> {
    let pack: Pack =
        serde_json::from_str(text).map_err(|error| pack_invalid(format!("not a pack: {error}")))?;
    let mut problems: Vec<String> = Vec::new();
    if pack.code.is_empty() || pack.code.len() > 32 {
        problems.push("`code` must be 1 to 32 characters".to_owned());
    }
    if pack.version < 1 {
        problems.push("`version` must be at least 1".to_owned());
    }
    let jurisdictions: BTreeSet<&str> =
        pack.jurisdictions.iter().map(|found| found.code.as_str()).collect();
    for found in &pack.jurisdictions {
        if autoxide_kernel::money::Currency::parse(&found.default_currency).is_err() {
            problems.push(format!("jurisdiction {}: unknown currency", found.code));
        }
    }
    let mut categories = BTreeSet::new();
    for found in &pack.categories {
        if !jurisdictions.contains(found.jurisdiction.as_str()) {
            problems.push(format!("category {}: unknown jurisdiction", found.code));
        }
        if !categories.insert((found.jurisdiction.as_str(), found.code.as_str())) {
            problems.push(format!("category {}: listed twice", found.code));
        }
    }
    let window = |from: &str, to: Option<&str>, what: &str, problems: &mut Vec<String>| match (
        date(from),
        to.map(date),
    ) {
        (Some(from), Some(Some(to))) if to <= from => {
            problems.push(format!("{what}: `effective_to` must be after `effective_from`"));
        }
        (None, _) | (_, Some(None)) => {
            problems.push(format!("{what}: dates must be `YYYY-MM-DD`"));
        }
        _ => {}
    };
    for (index, found) in pack.rates.iter().enumerate() {
        let what = format!("rate {index} ({})", found.category_code);
        if !categories.contains(&(found.jurisdiction.as_str(), found.category_code.as_str())) {
            problems.push(format!("{what}: unknown category"));
        }
        let basis = Basis::parse(&found.basis);
        match basis {
            None => problems.push(format!("{what}: `basis` must be percent, per_unit or exempt")),
            Some(Basis::PerUnit) if found.per_unit.is_none() => {
                problems.push(format!("{what}: a per-unit rate needs `per_unit`"));
            }
            _ => {}
        }
        // Thuế theo đơn vị âm sẽ làm giá chưa thuế lớn hơn giá đã gồm thuế — không có ở luật nào.
        if found.per_unit.is_some_and(Money::is_negative) {
            problems.push(format!("{what}: `per_unit` must not be negative"));
        }
        if !parse(&found.rate_percent).is_ok_and(rate_in_range) {
            problems.push(format!(
                "{what}: `rate_percent` must be a decimal string from 0 to {MAX_RATE_PERCENT}"
            ));
        }
        window(&found.effective_from, found.effective_to.as_deref(), &what, &mut problems);
    }
    // Hai thuế suất cùng nhóm, cùng tầng không được chồng ngày.
    for (a_index, a) in pack.rates.iter().enumerate() {
        for b in pack.rates.iter().skip(a_index + 1) {
            if a.jurisdiction == b.jurisdiction
                && a.category_code == b.category_code
                && a.stage == b.stage
            {
                let (a_to, b_to) = (a.effective_to.as_deref(), b.effective_to.as_deref());
                let a_before_b = a_to.is_some_and(|to| to <= b.effective_from.as_str());
                let b_before_a = b_to.is_some_and(|to| to <= a.effective_from.as_str());
                if !a_before_b && !b_before_a {
                    problems.push(format!(
                        "rates of {} (stage {}) overlap in time",
                        a.category_code, a.stage
                    ));
                }
            }
        }
    }
    for (index, found) in pack.withholding_rates.iter().enumerate() {
        let what = format!("withholding rate {index}");
        if !jurisdictions.contains(found.jurisdiction.as_str()) {
            problems.push(format!("{what}: unknown jurisdiction"));
        }
        // Khấu trừ thay là một phần của số tiền: không quá 100%.
        if !parse(&found.rate_percent)
            .is_ok_and(|rate| !rate.is_sign_negative() && rate <= Decimal::ONE_HUNDRED)
        {
            problems.push(format!("{what}: `rate_percent` must be a decimal string from 0 to 100"));
        }
        window(&found.effective_from, found.effective_to.as_deref(), &what, &mut problems);
    }
    for (index, found) in pack.rules.iter().enumerate() {
        let what = format!("rule {index} ({})", found.code);
        if !jurisdictions.contains(found.jurisdiction.as_str()) {
            problems.push(format!("{what}: unknown jurisdiction"));
        }
        window(&found.effective_from, found.effective_to.as_deref(), &what, &mut problems);
    }
    if problems.is_empty() {
        return Ok(pack);
    }
    let fields = problems
        .into_iter()
        .take(20)
        .map(|problem| {
            FieldError::new("pack", "invalid", "{problem}").with_param("problem", problem)
        })
        .collect();
    Err(Error::validation(fields))
}

/// Kết quả một lần nhập.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Imported {
    pub pack_id: Uuid,
    /// `false`: gói này đã nhập trước đó.
    pub changed: bool,
    pub rates: usize,
}

impl TaxService {
    /// Gói mới nhất đã nhập của mã `code`.
    pub(crate) async fn latest_pack(&self, code: &str) -> Result<Option<pack::Model>, Error> {
        Ok(pack::Entity::find()
            .filter(pack::Column::Code.eq(code))
            .order_by_desc(pack::Column::Version)
            .one(self.databases.control().read())
            .await?)
    }

    /// Gói đã nhập, theo thứ tự nhập (ID sinh lúc nhập; bản cao hơn luôn nhập sau bản thấp), keyset
    /// theo khoá chính.
    pub(crate) async fn packs(&self, page: &PageRequest) -> Result<Page<pack::Model>, Error> {
        let read = self.databases.control().read();
        fetch_page(pack::Entity::find(), pack::Column::Id, |row| row.id, page, read).await
    }

    /// Nhập gói (đã đọc dạng chữ). Cùng nội dung đã nhập: không đổi gì.
    pub(crate) async fn import_pack(&self, actor: Actor, text: &str) -> Result<Imported, Error> {
        let parsed = parse_pack(text)?;
        // SHA-256 của dạng JSON chuẩn hoá: cùng gói qua file hay qua API là cùng một gói.
        let canonical: serde_json::Value =
            serde_json::from_str(text).map_err(|error| pack_invalid(error.to_string()))?;
        let sha = autoxide_kernel::secret::sha256(&canonical.to_string());
        if let Some(current) = self.latest_pack(&parsed.code).await? {
            if current.content_sha256 == sha {
                return Ok(Imported { pack_id: current.id, changed: false, rates: 0 });
            }
            if parsed.version <= current.version {
                return Err(outdated(&parsed.code, current.version));
            }
        }
        let version = parsed.version;
        let codes: Vec<String> =
            parsed.jurisdictions.iter().map(|found| found.code.clone()).collect();
        let transaction = self.databases.control().begin_write().await?;
        // Gói thay hẳn dữ liệu của các vùng thuế nó nêu.
        rate::Entity::delete_many()
            .filter(rate::Column::Jurisdiction.is_in(codes.clone()))
            .exec(&transaction)
            .await?;
        withholding::Entity::delete_many()
            .filter(withholding::Column::Jurisdiction.is_in(codes.clone()))
            .exec(&transaction)
            .await?;
        rule::Entity::delete_many()
            .filter(rule::Column::Jurisdiction.is_in(codes.clone()))
            .exec(&transaction)
            .await?;
        category::Entity::delete_many()
            .filter(category::Column::Jurisdiction.is_in(codes.clone()))
            .exec(&transaction)
            .await?;
        jurisdiction::Entity::delete_many()
            .filter(jurisdiction::Column::Code.is_in(codes))
            .exec(&transaction)
            .await?;
        for found in &parsed.jurisdictions {
            jurisdiction::Entity::insert(jurisdiction::ActiveModel {
                code: Set(found.code.clone()),
                name: Set(found.name.clone()),
                default_currency: Set(found.default_currency.clone()),
                pack_code: Set(parsed.code.clone()),
                pack_version: Set(version),
            })
            .exec_without_returning(&transaction)
            .await?;
        }
        for found in &parsed.categories {
            category::Entity::insert(category::ActiveModel {
                id: Set(Uuid::now_v7()),
                jurisdiction: Set(found.jurisdiction.clone()),
                code: Set(found.code.clone()),
                name_en: Set(found.name_en.clone()),
                name_vi: Set(found.name_vi.clone()),
                tax_kind: Set(found.tax_kind.clone()),
                pack_version: Set(version),
            })
            .exec_without_returning(&transaction)
            .await?;
        }
        for found in &parsed.rates {
            rate::Entity::insert(rate::ActiveModel {
                id: Set(Uuid::now_v7()),
                jurisdiction: Set(found.jurisdiction.clone()),
                category_code: Set(found.category_code.clone()),
                tax_kind: Set(found.tax_kind.clone()),
                basis: Set(found.basis.clone()),
                rate_percent: Set(to_column(parse(&found.rate_percent)?)),
                per_unit_minor: Set(found.per_unit.map(Money::minor)),
                per_unit_currency: Set(found
                    .per_unit
                    .map(|unit| unit.currency().code().to_owned())),
                stage: Set(found.stage),
                effective_from: Set(found.effective_from.clone()),
                effective_to: Set(found.effective_to.clone()),
                legal_basis: Set(found.legal_basis.clone()),
                pack_version: Set(version),
            })
            .exec_without_returning(&transaction)
            .await?;
        }
        for found in &parsed.withholding_rates {
            withholding::Entity::insert(withholding::ActiveModel {
                id: Set(Uuid::now_v7()),
                jurisdiction: Set(found.jurisdiction.clone()),
                payer_kind: Set(found.payer_kind.clone()),
                payee_regime: Set(found.payee_regime.clone()),
                business_line: Set(found.business_line.clone()),
                tax_kind: Set(found.tax_kind.clone()),
                rate_percent: Set(to_column(parse(&found.rate_percent)?)),
                threshold_minor: Set(found.threshold.map(Money::minor)),
                threshold_currency: Set(found
                    .threshold
                    .map(|limit| limit.currency().code().to_owned())),
                effective_from: Set(found.effective_from.clone()),
                effective_to: Set(found.effective_to.clone()),
                legal_basis: Set(found.legal_basis.clone()),
                pack_version: Set(version),
            })
            .exec_without_returning(&transaction)
            .await?;
        }
        for found in &parsed.rules {
            rule::Entity::insert(rule::ActiveModel {
                id: Set(Uuid::now_v7()),
                jurisdiction: Set(found.jurisdiction.clone()),
                code: Set(found.code.clone()),
                value: Set(found.value.to_string()),
                effective_from: Set(found.effective_from.clone()),
                effective_to: Set(found.effective_to.clone()),
                legal_basis: Set(found.legal_basis.clone()),
                pack_version: Set(version),
            })
            .exec_without_returning(&transaction)
            .await?;
        }
        let id = Uuid::now_v7();
        pack::Entity::insert(pack::ActiveModel {
            id: Set(id),
            code: Set(parsed.code.clone()),
            version: Set(version),
            checked_on: Set(parsed.checked_on.clone()),
            source_note: Set(parsed.source_note.clone()),
            content_sha256: Set(sha),
            imported_by: Set(actor.user_id),
            imported_at: Set(now()),
        })
        .exec_without_returning(&transaction)
        .await?;
        let platform = self.databases.default_tenant();
        let event = events::PackImported(json!({
            "code": parsed.code, "version": version, "rates": parsed.rates.len()
        }));
        self.outbox
            .record(
                &transaction,
                &platform,
                actor,
                Some(format!("tax.pack:{}", parsed.code)),
                &event,
            )
            .await?;
        transaction.commit().await?;
        self.tables.clear();
        tracing::info!(code = %parsed.code, version, "tax data pack imported");
        Ok(Imported { pack_id: id, changed: true, rates: parsed.rates.len() })
    }

    /// Nhập gói đi kèm khi nó mới hơn bản trong database (job hằng giờ; chạy lại vô hại).
    pub(crate) async fn import_bundled(&self) -> Result<bool, Error> {
        let bundled = parse_pack(BUNDLED_VN)?;
        let newer = match self.latest_pack(&bundled.code).await? {
            Some(current) => current.version < bundled.version,
            None => true,
        };
        if !newer {
            return Ok(false);
        }
        match self.import_pack(Actor::system(), BUNDLED_VN).await {
            Ok(imported) => Ok(imported.changed),
            // Máy chủ khác vừa nhập cùng lúc.
            Err(error) if error.code() == "tax.pack_outdated" => Ok(false),
            Err(error) => Err(error),
        }
    }
}

/// `autoxide task tax.import_pack [path=<file>]`.
pub(crate) fn task(service: Arc<TaxService>) -> Task {
    Task::new(
        "import_pack",
        "Import a tax data pack (rates, withholding rates and rules with their effective dates). \
         Importing the same pack again changes nothing; only a higher version than the one \
         imported is accepted.",
        Arc::new(ImportTask(service)),
    )
    .argument(TaskArgument::optional(
        "path",
        "JSON file of the pack. Omit to import the pack shipped with the server (Vietnam).",
    ))
}

struct ImportTask(Arc<TaxService>);

#[async_trait::async_trait]
impl TaskRun for ImportTask {
    async fn run(&self, args: &TaskArgs) -> Result<String, Error> {
        let text = match args.get("path") {
            Some(path) => std::fs::read_to_string(path).map_err(|error| {
                Error::bad_request("tax.pack_unreadable", "Cannot read `{file}`: {reason}.")
                    .with_param("file", path)
                    .with_param("reason", error.to_string())
            })?,
            None => BUNDLED_VN.to_owned(),
        };
        let imported = self.0.import_pack(Actor::system(), &text).await?;
        let parsed = parse_pack(&text)?;
        Ok(if imported.changed {
            format!(
                "Imported tax pack `{}` version {} ({} rates).",
                parsed.code, parsed.version, imported.rates
            )
        } else {
            format!("Tax pack `{}` version {} is already imported.", parsed.code, parsed.version)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goi_vn_hop_le() {
        let pack = parse_pack(BUNDLED_VN).unwrap();
        assert_eq!(pack.code, "vn");
        assert!(pack.rates.iter().any(|rate| rate.category_code == "vat.standard_reducible"));
    }

    #[test]
    fn goi_sai_bao_moi_cho_mot_lan() {
        let text = r#"{"code": "x", "version": 1, "checked_on": "2026-01-01", "source_note": "",
            "jurisdictions": [{"code": "XX", "name": "X", "default_currency": "VND"}],
            "categories": [{"jurisdiction": "XX", "code": "vat.a", "name_en": "a", "name_vi": "a", "tax_kind": "vat"}],
            "rates": [
              {"jurisdiction": "XX", "category_code": "vat.a", "tax_kind": "vat", "basis": "percent", "rate_percent": "10", "stage": 2, "effective_from": "2026-01-01", "effective_to": null, "legal_basis": ""},
              {"jurisdiction": "XX", "category_code": "vat.a", "tax_kind": "vat", "basis": "percent", "rate_percent": "8", "stage": 2, "effective_from": "2026-06-01", "effective_to": null, "legal_basis": ""},
              {"jurisdiction": "XX", "category_code": "vat.b", "tax_kind": "vat", "basis": "nope", "rate_percent": "-1", "stage": 2, "effective_from": "2026-13-01", "effective_to": null, "legal_basis": ""}
            ]}"#;
        let error = parse_pack(text).unwrap_err();
        assert_eq!(error.code(), "validation_failed");
        assert!(error.fields().len() >= 4, "{:?}", error.fields());
    }

    #[test]
    fn goi_tu_choi_thue_theo_don_vi_am_va_thue_suat_qua_tran() {
        let text = r#"{"code": "x", "version": 1, "checked_on": "2026-01-01", "source_note": "",
            "jurisdictions": [{"code": "XX", "name": "X", "default_currency": "VND"}],
            "categories": [{"jurisdiction": "XX", "code": "a", "name_en": "a", "name_vi": "a", "tax_kind": "env"}],
            "rates": [
              {"jurisdiction": "XX", "category_code": "a", "tax_kind": "env", "basis": "per_unit", "rate_percent": "0", "per_unit": {"amount": "-5", "currency": "VND"}, "stage": 0, "effective_from": "2026-01-01", "effective_to": null, "legal_basis": ""},
              {"jurisdiction": "XX", "category_code": "a", "tax_kind": "excise", "basis": "percent", "rate_percent": "240000000000000000000", "stage": 1, "effective_from": "2026-01-01", "effective_to": null, "legal_basis": ""}
            ]}"#;
        let error = parse_pack(text).unwrap_err();
        let messages: Vec<&str> =
            error.fields().iter().map(|field| field.message.as_str()).collect();
        assert!(
            messages.iter().any(|message| message.contains("`per_unit` must not be negative")),
            "{messages:?}"
        );
        assert!(messages.iter().any(|message| message.contains("from 0 to 1000")), "{messages:?}");
    }
}
