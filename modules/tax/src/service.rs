//! Service của module `tax`: tra bảng thuế (bộ nhớ đệm), tính thuế, khấu trừ thay, hồ sơ thuế theo
//! nút (kế thừa theo cây). Handler REST, tool MCP và bên cung cấp `dyn TaxEngine` chỉ gọi vào đây.

use autoxide_kernel::time::{Date, Timestamp};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::decimal::{Decimal, Rounding, from_column, to_column};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{DatabaseRegistry, OrgId, OrgTree, ScopeExt, TenantContext};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};
use serde_json::json;

use crate::api::{
    InvoiceRequirements, InvoiceType, TaxComponent, TaxProfile, TaxQuote, TaxQuoteRequest,
    WithholdingQuote, WithholdingRequest,
};
use crate::engine::{Basis, ResolvedRate, compute};
use crate::entity::{category, jurisdiction, profile, rate, rule, withholding};
use crate::events;
use crate::permissions::{PROFILES_MANAGE, READ};

pub(crate) fn now() -> Timestamp {
    Timestamp::now()
}

/// Mọi bảng thuế (mặt điều khiển) trong bộ nhớ: vài trăm dòng.
#[derive(Debug, Default)]
pub(crate) struct Tables {
    pub jurisdictions: Vec<jurisdiction::Model>,
    pub categories: Vec<category::Model>,
    pub rates: Vec<rate::Model>,
    pub withholding: Vec<withholding::Model>,
    pub rules: Vec<rule::Model>,
}

/// Bộ nhớ đệm bảng thuế. Nhập gói ở máy chủ này thì xoá ngay; máy chủ khác thấy sau tối đa [`TTL`].
pub(crate) struct TablesCache(Mutex<Option<(Instant, Arc<Tables>)>>);

const TTL: Duration = Duration::from_secs(60);

impl TablesCache {
    pub(crate) fn new() -> Self {
        TablesCache(Mutex::new(None))
    }

    pub(crate) fn clear(&self) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = None;
        }
    }

    fn get(&self) -> Option<Arc<Tables>> {
        let slot = self.0.lock().ok()?;
        slot.as_ref().filter(|(at, _)| at.elapsed() < TTL).map(|(_, tables)| tables.clone())
    }

    fn put(&self, tables: Arc<Tables>) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some((Instant::now(), tables));
        }
    }
}

pub(crate) struct TaxService {
    pub databases: DatabaseRegistry,
    pub outbox: Outbox,
    pub tables: TablesCache,
    /// Kiểm địa chỉ của hồ sơ thuế (module `address`).
    pub units: Option<Arc<dyn autoxide_address::api::AdministrativeUnits>>,
    /// Múi giờ của nút (`organizations.timezone`) cho vùng thuế không khai múi giờ trong gói.
    pub orgs: Option<Arc<dyn autoxide_organizations::api::OrgDirectory>>,
    /// `AUTOXIDE_TAX_EXPIRY_WARNING_DAYS`.
    pub warning_days: i64,
}

/// Mã quy tắc trong gói thuế khai múi giờ của vùng thuế: ngày thuế, ngày lập hoá đơn tính theo nó.
pub(crate) const TIME_ZONE_RULE: &str = "tax_date.time_zone";

/// Mã quy tắc trong gói thuế: có được huỷ hoá đơn điện tử đã gửi cơ quan thuế không (`{"allowed": bool}`).
pub(crate) const CANCEL_RULE: &str = "einvoice.cancel_after_submission";

pub(crate) const REGIMES: [&str; 4] = ["vat_credit", "vat_direct", "household", "exempt"];
pub(crate) const BUSINESS_LINES: [&str; 4] = ["goods", "services", "production_transport", "other"];

pub(crate) fn jurisdiction_unknown(code: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "tax.jurisdiction_unknown",
        "There is no tax jurisdiction `{jurisdiction}`. See `GET /api/v1/tax/jurisdictions`.",
    )
    .with_param("jurisdiction", code)
}

fn category_unknown(jurisdiction: &str, category: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "tax.category_unknown",
        "There is no tax category `{category}` in `{jurisdiction}`.",
    )
    .with_param("category", category)
    .with_param("jurisdiction", jurisdiction)
}

fn rate_missing(category: &str, on: Date) -> Error {
    Error::new(
        ErrorKind::Validation,
        "tax.rate_missing",
        "There is no tax rate for category `{category}` on {date}. Import a newer tax data pack.",
    )
    .with_param("category", category)
    .with_param("date", on.format("%Y-%m-%d").to_string())
}

pub(crate) fn profile_not_found() -> Error {
    Error::not_found(
        "tax.profile_not_found",
        "Neither this unit nor any unit above it has a tax profile. Create one first.",
    )
}

/// Ngày của lúc `at` theo múi giờ trong giá trị JSON (dạng chuỗi) của quy tắc [`TIME_ZONE_RULE`]
/// (`{"time_zone": "…"}`); `None` khi giá trị không có múi giờ hợp lệ. Kiểu múi giờ của kernel không lộ ra đây:
/// module không phụ thuộc thẳng thư viện thời gian (D-20261126-kernel-data-layer).
pub(crate) fn rule_date(raw: &str, at: Timestamp) -> Option<Date> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let zone = value["time_zone"].as_str().and_then(autoxide_kernel::time::zone)?;
    Some(at.date_in(&zone))
}

/// `from <= on < to` (nửa mở), ngày dạng `YYYY-MM-DD` so sánh như chữ.
fn in_force(from: &str, to: Option<&str>, on: &str) -> bool {
    from <= on && to.is_none_or(|to| on < to)
}

/// Mã số thuế: 10 hay 13 chữ số (`10 chữ số-3 chữ số` cho đơn vị phụ thuộc), hay số định danh cá
/// nhân 12 chữ số dùng làm mã số thuế (TT 86/2024/TT-BTC — cần đối chiếu văn bản gốc).
pub(crate) fn valid_tax_code(code: &str) -> bool {
    let bytes = code.as_bytes();
    let digits = |part: &[u8]| part.iter().all(u8::is_ascii_digit);
    match bytes.len() {
        10 | 12 | 13 => digits(bytes),
        14 => digits(&bytes[..10]) && bytes[10] == b'-' && digits(&bytes[11..]),
        _ => false,
    }
}

/// Dữ liệu để đặt hồ sơ thuế.
#[derive(Debug, Clone)]
pub(crate) struct ProfileInput {
    pub jurisdiction: String,
    pub tax_code: Option<String>,
    pub regime: String,
    pub business_lines: Vec<String>,
    pub annual_revenue: Option<Money>,
    pub legal_name: Option<String>,
    pub address: Option<autoxide_address::api::AddressInput>,
}

pub(crate) fn describe_profile(row: &profile::Model) -> TaxProfile {
    let annual_revenue = match (row.annual_revenue_minor, &row.annual_revenue_currency) {
        (Some(minor), Some(currency)) => Money::from_columns(minor, currency).ok(),
        _ => None,
    };
    TaxProfile {
        org_id: row.org_id,
        jurisdiction: row.jurisdiction.clone(),
        tax_code: row.tax_code.clone(),
        regime: row.regime.clone(),
        business_lines: serde_json::from_str(&row.business_lines).unwrap_or_default(),
        annual_revenue,
        legal_name: row.legal_name.clone(),
        address: row.address.as_deref().and_then(|raw| serde_json::from_str(raw).ok()),
    }
}

impl TaxService {
    /// Bảng thuế (bộ nhớ đệm, đọc lại sau [`TTL`]).
    pub(crate) async fn tables(&self) -> Result<Arc<Tables>, Error> {
        if let Some(found) = self.tables.get() {
            return Ok(found);
        }
        let read = self.databases.control().read();
        let tables = Arc::new(Tables {
            jurisdictions: jurisdiction::Entity::find().all(read).await?,
            categories: category::Entity::find().all(read).await?,
            rates: rate::Entity::find().all(read).await?,
            withholding: withholding::Entity::find().all(read).await?,
            rules: rule::Entity::find().all(read).await?,
        });
        self.tables.put(tables.clone());
        Ok(tables)
    }

    /// Chuỗi thuế suất của nhóm `category` vào ngày `on`, theo tầng tăng dần.
    fn chain(
        tables: &Tables,
        jurisdiction: &str,
        category: &str,
        on: Date,
    ) -> Result<Vec<ResolvedRate>, Error> {
        if !tables
            .categories
            .iter()
            .any(|found| found.jurisdiction == jurisdiction && found.code == category)
        {
            return Err(category_unknown(jurisdiction, category));
        }
        let day = on.format("%Y-%m-%d").to_string();
        let mut chain = Vec::new();
        for found in tables.rates.iter().filter(|found| {
            found.jurisdiction == jurisdiction
                && found.category_code == category
                && in_force(&found.effective_from, found.effective_to.as_deref(), &day)
        }) {
            let basis = Basis::parse(&found.basis).ok_or_else(|| {
                Error::internal(format!("tax rate basis `{}` is unknown", found.basis))
            })?;
            let per_unit = match (found.per_unit_minor, &found.per_unit_currency) {
                (Some(minor), Some(currency)) => Some(Money::from_columns(minor, currency)?),
                _ => None,
            };
            chain.push(ResolvedRate {
                id: found.id,
                tax_kind: found.tax_kind.clone(),
                basis,
                rate_percent: from_column(&found.rate_percent)?,
                per_unit,
                stage: found.stage,
            });
        }
        if chain.is_empty() {
            return Err(rate_missing(category, on));
        }
        chain.sort_by_key(|found| found.stage);
        Ok(chain)
    }

    /// Hồ sơ thuế của nút `org` hay của nút cấp trên gần nhất (không kiểm quyền).
    pub(crate) async fn nearest_profile(
        &self,
        tenant: &TenantContext,
        org: OrgId,
    ) -> Result<Option<profile::Model>, Error> {
        let read = tenant.database().read();
        let mut lineage = OrgTree::ancestors(read, tenant.id(), org).await?;
        if lineage.is_empty() {
            lineage.push(org);
        }
        let found = profile::Entity::find()
            .scoped(tenant)
            .filter(profile::Column::OrgId.is_in(lineage.iter().map(|id| id.as_uuid())))
            .all(read)
            .await?;
        Ok(autoxide_kernel::tenancy::nearest(
            &lineage,
            found.into_iter().map(|row| (OrgId::from_uuid(row.org_id), row)),
        )
        .map(|(_, row)| row))
    }

    /// Người gọi có `tax.read` ở nút bán `seller`.
    pub(crate) async fn require_seller(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        seller: OrgId,
    ) -> Result<(), Error> {
        principal.require_at(tenant.database().read(), &READ, OrgTarget::new(seller, None)).await
    }

    pub(crate) async fn quote(
        &self,
        tenant: &TenantContext,
        request: TaxQuoteRequest,
    ) -> Result<TaxQuote, Error> {
        let tables = self.tables().await?;
        if !tables.jurisdictions.iter().any(|found| found.code == request.jurisdiction) {
            return Err(jurisdiction_unknown(&request.jurisdiction));
        }
        if request.lines.is_empty() || request.lines.len() > 1000 {
            return Err(Error::validation(vec![FieldError::new(
                "lines",
                "length",
                "must have 1 to 1000 lines",
            )]));
        }
        let regime = self
            .nearest_profile(tenant, request.seller)
            .await?
            .map_or_else(|| "vat_credit".to_owned(), |row| row.regime);
        let mut lines = Vec::with_capacity(request.lines.len());
        for line in &request.lines {
            let chain =
                Self::chain(&tables, &request.jurisdiction, &line.category, request.tax_point)?;
            // Người bán không khấu trừ GTGT: hoá đơn bán hàng không tách GTGT trên dòng; miễn thuế:
            // không thuế nào.
            let chain: Vec<ResolvedRate> = match regime.as_str() {
                "exempt" => Vec::new(),
                "vat_direct" | "household" => {
                    chain.into_iter().filter(|found| found.tax_kind != "vat").collect()
                }
                _ => chain,
            };
            lines.push((line.clone(), chain));
        }
        let computed = compute(request.price_mode, request.currency, &lines)?;
        let total = |pick: fn(&crate::api::TaxedLine) -> Money| {
            computed
                .lines
                .iter()
                .try_fold(Money::zero(request.currency), |sum, line| sum.checked_add(pick(line)))
        };
        let net = total(|line| line.net)?;
        let gross = total(|line| line.gross)?;
        let tax = gross.checked_sub(net)?;
        let pack_versions: Vec<i64> = tables
            .jurisdictions
            .iter()
            .filter(|found| found.code == request.jurisdiction)
            .map(|found| found.pack_version)
            .collect();
        let used: Vec<serde_json::Value> = lines
            .iter()
            .flat_map(|(line, chain)| {
                chain.iter().map(move |found| {
                    json!({
                        "line_id": line.line_id, "rate_id": found.id, "tax_kind": found.tax_kind,
                        "rate_percent": to_column(found.rate_percent), "stage": found.stage,
                    })
                })
            })
            .collect();
        Ok(TaxQuote {
            lines: computed.lines,
            totals: computed.totals,
            net,
            tax,
            gross,
            invoice_type: if regime == "vat_credit" {
                InvoiceType::VatInvoice
            } else {
                InvoiceType::SalesInvoice
            },
            snapshot: json!({
                "jurisdiction": request.jurisdiction,
                "tax_point": request.tax_point.format("%Y-%m-%d").to_string(),
                "pack_version": pack_versions.first(),
                "regime": regime,
                "rounding": "half_up_per_rate_group",
                "rates": used,
            }),
            regime,
        })
    }

    pub(crate) async fn withholding(
        &self,
        request: WithholdingRequest,
    ) -> Result<WithholdingQuote, Error> {
        let tables = self.tables().await?;
        if !tables.jurisdictions.iter().any(|found| found.code == request.jurisdiction) {
            return Err(jurisdiction_unknown(&request.jurisdiction));
        }
        if request.amount.is_negative() {
            return Err(Error::validation(vec![FieldError::new(
                "amount",
                "range",
                "must not be negative",
            )]));
        }
        let day = request.on.format("%Y-%m-%d").to_string();
        let mut components = Vec::new();
        let mut used = Vec::new();
        let mut total = Money::zero(request.amount.currency());
        for found in tables.withholding.iter().filter(|found| {
            found.jurisdiction == request.jurisdiction
                && found.payer_kind == request.payer_kind
                && found.payee_regime == request.payee_regime
                && found.business_line == request.business_line
                && in_force(&found.effective_from, found.effective_to.as_deref(), &day)
        }) {
            // Dưới ngưỡng (nếu gói đặt) thì không khấu trừ.
            if let (Some(minor), Some(currency)) =
                (found.threshold_minor, &found.threshold_currency)
                && let Ok(threshold) = Money::from_columns(minor, currency)
                && threshold.currency() == request.amount.currency()
                && request.amount.minor() < threshold.minor()
            {
                continue;
            }
            let rate = from_column(&found.rate_percent)?;
            let amount =
                request.amount.apply_rate(rate / Decimal::new(100, 0), Rounding::HalfUp)?;
            total = total.checked_add(amount)?;
            used.push(json!({"rate_id": found.id, "tax_kind": found.tax_kind, "rate_percent": found.rate_percent, "legal_basis": found.legal_basis}));
            components.push(TaxComponent {
                tax_kind: found.tax_kind.clone(),
                rate_percent: found.rate_percent.clone(),
                exempt: false,
                amount,
            });
        }
        components.sort_by(|a, b| a.tax_kind.cmp(&b.tax_kind));
        Ok(WithholdingQuote {
            components,
            total,
            snapshot: json!({"on": day, "payer_kind": request.payer_kind, "payee_regime": request.payee_regime, "business_line": request.business_line, "rates": used}),
        })
    }

    pub(crate) async fn invoice_requirements(
        &self,
        tenant: &TenantContext,
        org: OrgId,
        on: Date,
    ) -> Result<InvoiceRequirements, Error> {
        let Some(found) = self.nearest_profile(tenant, org).await? else {
            return Ok(InvoiceRequirements { cash_register_required: false, legal_basis: None });
        };
        let tables = self.tables().await?;
        let day = on.format("%Y-%m-%d").to_string();
        let revenue = describe_profile(&found).annual_revenue;
        for rule in tables.rules.iter().filter(|rule| {
            rule.jurisdiction == found.jurisdiction
                && rule.code == "einvoice.cash_register_required"
                && in_force(&rule.effective_from, rule.effective_to.as_deref(), &day)
        }) {
            let value: serde_json::Value = serde_json::from_str(&rule.value).unwrap_or_default();
            let regimes: Vec<String> =
                serde_json::from_value(value["regimes"].clone()).unwrap_or_default();
            let threshold = value["annual_revenue_minor"].as_i64();
            let currency = value["currency"].as_str().and_then(|code| Currency::parse(code).ok());
            let over = match (revenue, threshold, currency) {
                (Some(revenue), Some(threshold), Some(currency)) => {
                    revenue.currency() == currency && revenue.minor() >= threshold
                }
                _ => false,
            };
            if regimes.contains(&found.regime) && over {
                return Ok(InvoiceRequirements {
                    cash_register_required: true,
                    legal_basis: Some(rule.legal_basis.clone()),
                });
            }
        }
        Ok(InvoiceRequirements { cash_register_required: false, legal_basis: None })
    }

    /// Căn cứ cấm huỷ hoá đơn điện tử đã gửi cơ quan thuế của vùng thuế người bán `org`, cho hoá đơn ngày
    /// `on` (quy tắc [`CANCEL_RULE`] của gói, `{"allowed": false}`); `None`: không cấm hay không có hồ sơ thuế.
    pub(crate) async fn einvoice_cancel_forbidden(
        &self,
        tenant: &TenantContext,
        org: OrgId,
        on: Date,
    ) -> Result<Option<String>, Error> {
        let Some(found) = self.nearest_profile(tenant, org).await? else { return Ok(None) };
        let tables = self.tables().await?;
        let day = on.format("%Y-%m-%d").to_string();
        Ok(tables
            .rules
            .iter()
            .filter(|rule| {
                rule.jurisdiction == found.jurisdiction
                    && rule.code == CANCEL_RULE
                    && in_force(&rule.effective_from, rule.effective_to.as_deref(), &day)
            })
            .find(|rule| {
                let value: serde_json::Value =
                    serde_json::from_str(&rule.value).unwrap_or_default();
                value["allowed"] == serde_json::Value::Bool(false)
            })
            .map(|rule| rule.legal_basis.clone()))
    }

    /// Ngày thuế của thời điểm `at` với người bán `org` (LG-2, D-20261126-tax-date-time-zone): ngày theo
    /// múi giờ của vùng thuế trong hồ sơ thuế (quy tắc [`TIME_ZONE_RULE`] của gói, vd `Asia/Ho_Chi_Minh`);
    /// vùng không khai thì theo `organizations.timezone` của nút; không có gì thì UTC.
    //
    // Vùng thuế thắng cài đặt của nút: luật Việt Nam tính ngày theo giờ Việt Nam bất kể nút đặt múi giờ
    // nào (mặc định của `organizations.timezone` là UTC — chính là lỗi LG-2).
    pub(crate) async fn tax_date(
        &self,
        tenant: &TenantContext,
        org: OrgId,
        at: Timestamp,
    ) -> Result<Date, Error> {
        if let Some(found) = self.nearest_profile(tenant, org).await?
            && let Some(day) = self.jurisdiction_date(&found.jurisdiction, at).await?
        {
            return Ok(day);
        }
        if let Some(orgs) = &self.orgs {
            match orgs.setting(tenant, org, "organizations.timezone").await {
                Ok(found) => {
                    let zone = found
                        .and_then(|found| found.value)
                        .and_then(|value| value.as_str().and_then(autoxide_kernel::time::zone));
                    if let Some(zone) = zone {
                        return Ok(at.date_in(&zone));
                    }
                }
                Err(error) => {
                    tracing::warn!(error = %error, "cannot read the unit time zone; tax date in UTC");
                }
            }
        }
        Ok(at.date_utc())
    }

    /// Ngày của thời điểm `at` theo múi giờ của vùng thuế `jurisdiction` (quy tắc [`TIME_ZONE_RULE`]);
    /// `None`: vùng không khai múi giờ.
    pub(crate) async fn jurisdiction_date(
        &self,
        jurisdiction: &str,
        at: Timestamp,
    ) -> Result<Option<Date>, Error> {
        let tables = self.tables().await?;
        // Tìm quy tắc theo ngày UTC: múi giờ của một vùng không đổi sát ranh giới ngày.
        let day = at.date_utc().format("%Y-%m-%d").to_string();
        Ok(tables
            .rules
            .iter()
            .filter(|rule| {
                rule.jurisdiction == jurisdiction
                    && rule.code == TIME_ZONE_RULE
                    && in_force(&rule.effective_from, rule.effective_to.as_deref(), &day)
            })
            .find_map(|rule| rule_date(&rule.value, at)))
    }

    /// Vùng thuế đã nhập không có quy tắc [`TIME_ZONE_RULE`] đang hiệu lực vào lúc `at`.
    pub(crate) async fn jurisdictions_without_time_zone(
        &self,
        at: Timestamp,
    ) -> Result<Vec<String>, Error> {
        let codes: Vec<String> =
            self.tables().await?.jurisdictions.iter().map(|found| found.code.clone()).collect();
        let mut missing = Vec::new();
        for code in codes {
            if self.jurisdiction_date(&code, at).await?.is_none() {
                missing.push(code);
            }
        }
        Ok(missing)
    }

    /// Thuế suất hết hạn trong `warning_days` ngày tính từ `today` mà không có thuế suất nối tiếp
    /// (cùng vùng, nhóm, tầng, bắt đầu đúng ngày hết hạn). `None`: chưa nhập gói nào.
    pub(crate) async fn expiring_rates(&self, today: Date) -> Result<Option<Vec<String>>, Error> {
        let tables = self.tables().await?;
        if tables.rates.is_empty() {
            return Ok(None);
        }
        let start = today.format("%Y-%m-%d").to_string();
        let limit = today.checked_add_days(self.warning_days).unwrap_or(today).format("%Y-%m-%d");
        let mut found = Vec::new();
        for rate in &tables.rates {
            let Some(end) = rate.effective_to.as_deref() else { continue };
            if end <= start.as_str() || end > limit.as_str() {
                continue;
            }
            let followed = tables.rates.iter().any(|next| {
                next.jurisdiction == rate.jurisdiction
                    && next.category_code == rate.category_code
                    && next.stage == rate.stage
                    && next.effective_from.as_str() == end
            });
            if !followed {
                found.push(format!(
                    "{} {} ({}%) ends on {end}",
                    rate.jurisdiction, rate.category_code, rate.rate_percent
                ));
            }
        }
        found.sort();
        Ok(Some(found))
    }

    // ---- Hồ sơ thuế (REST) ----------------------------------------------------------------

    /// Hồ sơ của nút `org` cho người gọi (`tax.read` phủ nút): của chính nút hay nút cấp trên gần
    /// nhất. Ngoài phạm vi: như không có.
    pub(crate) async fn read_profile(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: OrgId,
    ) -> Result<profile::Model, Error> {
        let read = tenant.database().read();
        principal.require_at(read, &READ, OrgTarget::new(org, None)).await.map_err(|error| {
            if error.code() == "org_out_of_scope" { profile_not_found() } else { error }
        })?;
        self.nearest_profile(tenant, org).await?.ok_or_else(profile_not_found)
    }

    pub(crate) async fn list_profiles(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<profile::Model>, Error> {
        let condition = principal.scope_condition(&READ, profile::Column::OrgId, None)?;
        let select = profile::Entity::find().scoped(tenant).filter(condition);
        fetch_page(select, profile::Column::OrgId, |row| row.org_id, page, tenant.database().read())
            .await
    }

    /// Đặt (thay) hồ sơ thuế của nút `org`; gom mọi lỗi báo một lần.
    pub(crate) async fn set_profile(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: OrgId,
        input: ProfileInput,
        expected: Option<Timestamp>,
    ) -> Result<profile::Model, Error> {
        // Không có permission ở đâu cả: từ chối trước khi kiểm dữ liệu (phạm vi nút kiểm trong
        // transaction bên dưới).
        principal.require(&PROFILES_MANAGE)?;
        let tables = self.tables().await?;
        let mut problems = Vec::new();
        if !tables.jurisdictions.iter().any(|found| found.code == input.jurisdiction) {
            problems.push(FieldError::new(
                "jurisdiction",
                "unknown",
                "there is no tax jurisdiction with this code",
            ));
        }
        let tax_code =
            input.tax_code.map(|code| code.trim().to_owned()).filter(|code| !code.is_empty());
        if tax_code.as_deref().is_some_and(|code| !valid_tax_code(code)) {
            problems.push(FieldError::new(
                "tax_code",
                "format",
                "must be 10, 12 or 13 digits, or 10 digits, `-` and 3 digits",
            ));
        }
        if !REGIMES.contains(&input.regime.as_str()) {
            problems.push(
                FieldError::new("regime", "invalid", "must be one of {values}")
                    .with_param("values", REGIMES.join(", ")),
            );
        }
        for (index, line) in input.business_lines.iter().enumerate() {
            if !BUSINESS_LINES.contains(&line.as_str()) {
                problems.push(
                    FieldError::new(
                        format!("business_lines[{index}]"),
                        "invalid",
                        "must be one of {values}",
                    )
                    .with_param("values", BUSINESS_LINES.join(", ")),
                );
            }
        }
        if input.annual_revenue.is_some_and(Money::is_negative) {
            problems.push(FieldError::new("annual_revenue", "range", "must not be negative"));
        }
        let legal_name =
            input.legal_name.map(|name| name.trim().to_owned()).filter(|name| !name.is_empty());
        if legal_name.as_ref().is_some_and(|name| name.chars().count() > 200) {
            problems.push(FieldError::new(
                "legal_name",
                "length",
                "must be 1 to 200 characters long",
            ));
        }
        let address = match input.address {
            Some(address) => match &self.units {
                Some(units) => match units.validate(address, "address.").await {
                    Ok(address) => Some(address),
                    Err(fields) => {
                        problems.extend(fields);
                        None
                    }
                },
                None => {
                    problems.push(FieldError::new(
                        "address",
                        "unavailable",
                        "addresses cannot be checked on this server",
                    ));
                    None
                }
            },
            None => None,
        };
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let address = match address {
            Some(address) => Some(serde_json::to_string(&address).map_err(Error::internal)?),
            None => None,
        };
        let mut lines = input.business_lines;
        lines.sort();
        lines.dedup();
        let transaction = tenant.database().begin_write().await?;
        principal
            .require_write_at(&transaction, &PROFILES_MANAGE, OrgTarget::new(org, None))
            .await?;
        // Khoá hồ sơ trước khi so `If-Match` (D-181): hai lần sửa cùng phiên bản từng cùng qua
        // trên Postgres, bản sau đè mất bản trước.
        let current = autoxide_kernel::rows::lock_one(
            &transaction,
            profile::Entity::find().scoped(tenant).filter(profile::Column::OrgId.eq(org.as_uuid())),
        )
        .await?;
        if let (Some(expected), Some(current)) = (expected, &current)
            && expected != current.updated_at
        {
            return Err(autoxide_kernel::version::changed());
        }
        let row = profile::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(org.as_uuid()),
            jurisdiction: Set(input.jurisdiction),
            tax_code: Set(tax_code),
            regime: Set(input.regime),
            business_lines: Set(serde_json::to_string(&lines).map_err(Error::internal)?),
            annual_revenue_minor: Set(input.annual_revenue.map(Money::minor)),
            annual_revenue_currency: Set(input
                .annual_revenue
                .map(|money| money.currency().code().to_owned())),
            legal_name: Set(legal_name),
            address: Set(address),
            updated_by: Set(Some(principal.user_id())),
            updated_at: Set(now()),
        };
        profile::Entity::insert(row)
            .on_conflict(
                OnConflict::columns([profile::Column::TenantId, profile::Column::OrgId])
                    .update_columns([
                        profile::Column::Jurisdiction,
                        profile::Column::TaxCode,
                        profile::Column::Regime,
                        profile::Column::BusinessLines,
                        profile::Column::AnnualRevenueMinor,
                        profile::Column::AnnualRevenueCurrency,
                        profile::Column::LegalName,
                        profile::Column::Address,
                        profile::Column::UpdatedBy,
                        profile::Column::UpdatedAt,
                    ])
                    .to_owned(),
            )
            .exec_without_returning(&transaction)
            .await?;
        let saved = profile::Entity::find()
            .scoped(tenant)
            .filter(profile::Column::OrgId.eq(org.as_uuid()))
            .one(&transaction)
            .await?
            .ok_or_else(|| Error::internal("tax profile vanished after it was written"))?;
        // Event không mang mã số thuế hay địa chỉ: chỉ nút và chế độ thuế.
        let event = events::ProfileChanged(json!({
            "org_id": org.as_uuid(), "regime": saved.regime, "created": current.is_none()
        }));
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                Some(format!("tax.profile:{}", org.as_uuid())),
                &event,
            )
            .await?;
        transaction.commit().await?;
        Ok(saved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ma_so_thue_10_12_13_so() {
        for good in ["0101234567", "001234567890", "0101234567001", "0101234567-001"] {
            assert!(valid_tax_code(good), "{good}");
        }
        for bad in ["010123456", "01012345678", "0101234567_001", "01012a4567"] {
            assert!(!valid_tax_code(bad), "{bad}");
        }
    }

    #[test]
    fn hieu_luc_nua_mo() {
        assert!(in_force("2025-07-01", Some("2027-01-01"), "2026-12-31"));
        assert!(!in_force("2025-07-01", Some("2027-01-01"), "2027-01-01"));
        assert!(in_force("2027-01-01", None, "2027-01-01"));
        assert!(!in_force("2027-01-01", None, "2026-12-31"));
    }
}
