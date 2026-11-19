//! Service của module `reporting`: ai thấy gì (phạm vi theo cây khách hàng và cây tổ chức), đọc kho
//! của cây, quy đổi nhiều tiền tệ. REST và tool MCP cùng gọi vào đây (luật kiến trúc 8).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use autoxide_kernel::access::{Principal, Scope};
use autoxide_kernel::decimal::Rounding;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::report::{FactKind, FactSource};
use autoxide_kernel::tenancy::{
    DatabaseRegistry, OrgId, OrgTree, TenantContext, TenantId, root_of,
};
use autoxide_ledger::api::Ledger;
use autoxide_organizations::api::{OrgDirectory, OrgGroups};
use chrono::{Datelike, Duration, NaiveDate, Utc};
use sea_orm::sea_query::{Expr, Func};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect, QueryTrait};
use uuid::Uuid;

use crate::config::ReportingConfig;
use crate::entity::{daily, dimension, fact, local_fact};
use crate::permissions::{REPORTING_MANAGE, REPORTING_READ};

/// Một loại số liệu đã khai, kèm module khai.
#[derive(Debug, Clone)]
pub(crate) struct KindInfo {
    pub module: &'static str,
    pub kind: FactKind,
}

pub(crate) struct Reporting {
    pub databases: DatabaseRegistry,
    pub directory: Option<Arc<dyn OrgDirectory>>,
    /// Nhóm nút ngoài cây (D-232); `None`: app không có module `organizations`.
    pub groups: Option<Arc<dyn OrgGroups>>,
    pub ledger: Option<Arc<dyn Ledger>>,
    pub kinds: Vec<KindInfo>,
    pub config: ReportingConfig,
}

impl Reporting {
    pub(crate) fn new(
        databases: DatabaseRegistry,
        directory: Option<Arc<dyn OrgDirectory>>,
        ledger: Option<Arc<dyn Ledger>>,
        sources: &[(&'static str, FactSource)],
        config: ReportingConfig,
    ) -> Self {
        let kinds = sources
            .iter()
            .flat_map(|(module, source)| {
                source.kinds().iter().map(|kind| KindInfo { module, kind: *kind })
            })
            .collect();
        Reporting { databases, directory, groups: None, ledger, kinds, config }
    }
}

/// Cách nhóm của tổng.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum GroupBy {
    #[default]
    Day,
    Week,
    Month,
    Tenant,
    Org,
    /// Theo nhóm nút (D-232): cần `group_ids`; một nút ở hai nhóm được tính ở cả hai.
    Group,
}

impl GroupBy {
    pub(crate) fn parse(text: &str) -> Option<Self> {
        match text {
            "day" => Some(GroupBy::Day),
            "week" => Some(GroupBy::Week),
            "month" => Some(GroupBy::Month),
            "tenant" => Some(GroupBy::Tenant),
            "org" => Some(GroupBy::Org),
            "group" => Some(GroupBy::Group),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            GroupBy::Day => "day",
            GroupBy::Week => "week",
            GroupBy::Month => "month",
            GroupBy::Tenant => "tenant",
            GroupBy::Org => "org",
            GroupBy::Group => "group",
        }
    }

    /// Khoảng ngày tối đa cho cách nhóm này.
    fn max_days(self) -> i64 {
        if self == GroupBy::Day { MAX_DAYS_BY_DAY } else { MAX_DAYS }
    }
}

const MAX_DAYS_BY_DAY: i64 = 366;
const MAX_DAYS: i64 = 3660;

/// Bộ lọc chung của `/summary` và `/facts`, đã đọc từ đầu vào.
#[derive(Debug, Clone, Default)]
pub(crate) struct Filter {
    pub kind: Option<String>,
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub tenant_ids: Vec<Uuid>,
    pub org_ids: Vec<Uuid>,
    /// Nhóm nút (D-232): chỉ số liệu của chính khách người xem ở các nút thuộc nhóm, giao phạm vi.
    pub group_ids: Vec<Uuid>,
    pub dimensions: BTreeMap<String, String>,
}

/// Câu hỏi của `/summary`.
#[derive(Debug, Clone, Default)]
pub(crate) struct SummaryQuery {
    pub filter: Filter,
    pub group_by: GroupBy,
    pub currency: Option<Currency>,
}

/// Tổng của một loại tiền (hay của số liệu không có tiền: `currency = None`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CurrencyTotal {
    pub currency: Option<Currency>,
    pub amount_minor: i64,
    pub quantity: i64,
    pub count: i64,
}

/// Một nhóm của tổng.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SummaryGroup {
    pub key: String,
    pub by_currency: Vec<CurrencyTotal>,
    pub converted: Option<Money>,
    /// Số nút của nhóm ngoài phạm vi người xem (khi lọc / gom theo nhóm, D-232).
    pub excluded_org_count: Option<u64>,
}

/// Một cặp (tiền, ngày) không có tỉ giá sang tiền xin quy đổi.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct MissingRate {
    pub currency: String,
    pub day: NaiveDate,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Summary {
    pub kind: String,
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub group_by: GroupBy,
    pub groups: Vec<SummaryGroup>,
    pub totals: SummaryGroup,
    pub missing_rates: Vec<MissingRate>,
}

/// Độ trễ của một khách trong cây.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Lag {
    pub tenant_id: TenantId,
    pub pending: u64,
    pub oldest_pending_at: Option<chrono::DateTime<Utc>>,
    pub last_received_at: Option<chrono::DateTime<Utc>>,
}

/// Phần người xem được thấy: kho của khách gốc nào, khách nguồn nào, nút nào (`None`: mọi nút).
#[derive(Debug, Clone)]
struct Visible {
    root: TenantId,
    tenants: Vec<Uuid>,
    orgs: Option<Vec<Uuid>>,
    /// Nhóm đã hỏi: thành viên trong phạm vi, số thành viên ngoài phạm vi.
    groups: Vec<VisibleGroup>,
    /// Số nút (không trùng) của các nhóm đã hỏi nằm ngoài phạm vi; `None` khi không hỏi nhóm.
    excluded_org_count: Option<u64>,
}

#[derive(Debug, Clone)]
struct VisibleGroup {
    id: Uuid,
    members: BTreeSet<Uuid>,
    excluded: u64,
}

/// `reporting.tenant_not_found` (404): khách ngoài cây hay ngoài phạm vi — không lộ có tồn tại.
pub(crate) fn tenant_not_found() -> Error {
    Error::not_found(
        "reporting.tenant_not_found",
        "There is no such tenant within what you may see.",
    )
}

/// `reporting.store_unavailable` (503): database của khách gốc đang bảo trì.
pub(crate) fn store_unavailable() -> Error {
    Error::unavailable(
        "reporting.store_unavailable",
        "The report store of this customer tree is under maintenance. Retry in a few minutes.",
    )
    .with_retry_after(60)
}

/// `reporting.group_not_found` (404): khách người xem không có nhóm này.
fn group_not_found(id: Uuid) -> Error {
    Error::not_found("reporting.group_not_found", "There is no unit group `{id}` in this tenant.")
        .with_param("id", id)
}

fn kind_unknown(kind: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "reporting.kind_unknown",
        "There is no fact kind `{kind}`. List them with `GET /api/v1/reporting/kinds`.",
    )
    .with_param("kind", kind.chars().take(100).collect::<String>())
}

impl Reporting {
    /// Khách gốc của cây chứa `tenant` (chính nó khi không có khách cha).
    pub(crate) async fn root_of(&self, tenant: TenantId) -> Result<TenantId, Error> {
        Ok(self.databases.tenant_ancestors(tenant).await?.last().copied().unwrap_or(tenant))
    }

    pub(crate) fn kinds(&self) -> &[KindInfo] {
        &self.kinds
    }

    fn kind(&self, kind: &str) -> Result<&KindInfo, Error> {
        self.kinds.iter().find(|known| known.kind.kind == kind).ok_or_else(|| kind_unknown(kind))
    }

    /// Phần người `principal` ở khách `viewer` được xem, sau khi giao với bộ lọc của client (bộ lọc
    /// chỉ thu hẹp, không bao giờ mở rộng).
    async fn visible(
        &self,
        viewer: &TenantContext,
        principal: &Principal,
        filter: &Filter,
    ) -> Result<Visible, Error> {
        principal.require(&REPORTING_READ)?;
        let grants = principal.grants(&REPORTING_READ);
        let tenant_wide = grants.iter().any(|grant| grant.scope == Scope::Tenant);
        let fence = principal.fence();
        let (tenants, mut orgs) = if tenant_wide && fence.is_none() {
            // Độ sâu `tenant`: khách mình và mọi khách cấp dưới (uỷ quyền theo cây của bước 34 —
            // cha thấy mọi thứ của con).
            let mut tenants = vec![viewer.id().as_uuid()];
            tenants.extend(
                self.databases
                    .tenant_descendants(viewer.id())
                    .await?
                    .into_iter()
                    .map(|child| child.id.as_uuid()),
            );
            (tenants, None)
        } else {
            // Độ sâu hẹp hơn: chỉ số liệu của chính khách này ở các nút của người xem. Cây tổ chức
            // ở database của khách này, kho ở database của khách gốc: đổ danh sách nút ra trước.
            let read = viewer.database().read();
            let mut allowed: BTreeSet<Uuid> = BTreeSet::new();
            if tenant_wide {
                allowed.extend(
                    OrgTree::descendants(read, viewer.id(), &[root_of(viewer.id())])
                        .await?
                        .iter()
                        .map(OrgId::as_uuid),
                );
            } else {
                let subtrees: Vec<OrgId> = grants
                    .iter()
                    .filter(|grant| grant.scope == Scope::Subtree)
                    .map(|grant| grant.org)
                    .collect();
                allowed.extend(
                    OrgTree::descendants(read, viewer.id(), &subtrees)
                        .await?
                        .iter()
                        .map(OrgId::as_uuid),
                );
                allowed.extend(
                    grants
                        .iter()
                        .filter(|grant| matches!(grant.scope, Scope::Unit | Scope::Own))
                        .map(|grant| grant.org.as_uuid()),
                );
            }
            if let Some(fence) = fence {
                let fenced: BTreeSet<Uuid> = OrgTree::descendants(read, viewer.id(), &[fence])
                    .await?
                    .iter()
                    .map(OrgId::as_uuid)
                    .collect();
                allowed = allowed.intersection(&fenced).copied().collect();
            }
            (vec![viewer.id().as_uuid()], Some(allowed.into_iter().collect::<Vec<_>>()))
        };
        let tenants = if filter.tenant_ids.is_empty() {
            tenants
        } else {
            if filter.tenant_ids.iter().any(|asked| !tenants.contains(asked)) {
                return Err(tenant_not_found());
            }
            filter.tenant_ids.clone()
        };
        // Phạm vi theo nút (trước bộ lọc của client): nhóm giao với nó.
        let scope = orgs.clone();
        if !filter.org_ids.is_empty() {
            orgs = Some(match orgs {
                None => filter.org_ids.clone(),
                Some(allowed) => {
                    filter.org_ids.iter().filter(|org| allowed.contains(org)).copied().collect()
                }
            });
        }
        let mut visible = Visible {
            root: self.root_of(viewer.id()).await?,
            tenants,
            orgs,
            groups: Vec::new(),
            excluded_org_count: None,
        };
        if !filter.group_ids.is_empty() {
            self.narrow_to_groups(viewer, filter, scope.as_deref(), &mut visible).await?;
        }
        Ok(visible)
    }

    /// Nhóm không cấp quyền (D-232): thành viên của nhóm GIAO với phạm vi `reporting.read` của người
    /// xem; nút ngoài phạm vi bị bỏ và chỉ lộ thành số đếm. Nhóm chỉ lọc số liệu của chính khách sở
    /// hữu nhóm (khách người xem), không lọc khách con.
    async fn narrow_to_groups(
        &self,
        viewer: &TenantContext,
        filter: &Filter,
        scope: Option<&[Uuid]>,
        visible: &mut Visible,
    ) -> Result<(), Error> {
        let viewer_id = viewer.id().as_uuid();
        visible.tenants.retain(|tenant| *tenant == viewer_id);
        let mut union: BTreeSet<Uuid> = BTreeSet::new();
        let mut everyone: BTreeSet<Uuid> = BTreeSet::new();
        for id in &filter.group_ids {
            let members = match &self.groups {
                Some(groups) => groups.members(viewer, *id).await?,
                None => None,
            }
            .ok_or_else(|| group_not_found(*id))?;
            let members: BTreeSet<Uuid> = members.iter().map(OrgId::as_uuid).collect();
            let inside: BTreeSet<Uuid> = match scope {
                None => members.clone(),
                Some(allowed) => {
                    members.iter().filter(|org| allowed.contains(org)).copied().collect()
                }
            };
            let excluded = u64::try_from(members.len() - inside.len()).unwrap_or(0);
            union.extend(inside.iter().copied());
            everyone.extend(members.iter().copied());
            visible.groups.push(VisibleGroup { id: *id, members: inside, excluded });
        }
        visible.excluded_org_count = Some(u64::try_from(everyone.len() - union.len()).unwrap_or(0));
        visible.orgs = Some(match visible.orgs.take() {
            None => union.into_iter().collect(),
            Some(orgs) => orgs.into_iter().filter(|org| union.contains(org)).collect(),
        });
        Ok(())
    }

    /// Khoảng ngày của câu hỏi: mặc định 30 ngày gần nhất (theo UTC).
    fn range(filter: &Filter, group_by: GroupBy) -> Result<(NaiveDate, NaiveDate), Error> {
        let to = filter.to.unwrap_or_else(|| Utc::now().date_naive());
        let from = filter.from.unwrap_or(to - Duration::days(29));
        if from > to {
            return Err(Error::new(
                ErrorKind::Validation,
                "reporting.range_invalid",
                "The start date must be on or before the end date.",
            ));
        }
        let max = group_by.max_days();
        if (to - from).num_days() + 1 > max {
            return Err(Error::new(
                ErrorKind::Validation,
                "reporting.range_too_large",
                "The period is too long for grouping by `{group_by}` (at most {max} days).",
            )
            .with_param("group_by", group_by.as_str())
            .with_param("max", max));
        }
        Ok((from, to))
    }

    /// Tổng của loại `query.filter.kind`, đọc từ `reporting_daily` ở kho của cây.
    pub(crate) async fn summary(
        &self,
        viewer: &TenantContext,
        principal: &Principal,
        query: &SummaryQuery,
    ) -> Result<Summary, Error> {
        principal.require(&REPORTING_READ)?;
        let kind = query.filter.kind.as_deref().ok_or_else(|| {
            Error::validation(vec![FieldError::new(
                "kind",
                "required",
                "Give the fact kind to sum, e.g. `invoices.issued_total`.",
            )])
        })?;
        let kind = self.kind(kind)?.kind.kind;
        let (from, to) = Self::range(&query.filter, query.group_by)?;
        if query.group_by == GroupBy::Group && query.filter.group_ids.is_empty() {
            return Err(Error::validation(vec![FieldError::new(
                "group_ids",
                "required",
                "Give the unit groups to group by (`group_ids`).",
            )]));
        }
        let visible = self.visible(viewer, principal, &query.filter).await?;
        let store = self.store(visible.root).await?;
        let mut select = daily::Entity::find()
            .select_only()
            .filter(daily::Column::TenantId.eq(visible.root.as_uuid()))
            .filter(daily::Column::Kind.eq(kind))
            .filter(daily::Column::SourceTenantId.is_in(visible.tenants.clone()))
            .filter(daily::Column::LocalDay.gte(from))
            .filter(daily::Column::LocalDay.lte(to));
        if let Some(orgs) = &visible.orgs {
            select = select.filter(daily::Column::OrgId.is_in(orgs.clone()));
        }
        // Cột nhóm thêm: khách nguồn hay nút; theo ngày / tuần / tháng thì chỉ cần (ngày, tiền).
        let extra = match query.group_by {
            GroupBy::Tenant => Some(daily::Column::SourceTenantId),
            GroupBy::Org | GroupBy::Group => Some(daily::Column::OrgId),
            _ => None,
        };
        // `extra` vắng: chọn lại `tenant_id` (một giá trị) để kiểu dòng giữ nguyên.
        let extra_column = extra.unwrap_or(daily::Column::TenantId);
        // Postgres trả `numeric` cho `SUM(bigint)`: ép về BIGINT cho cả hai database.
        let sum = |column: daily::Column| {
            Expr::from(Func::cast_as(Func::sum(Expr::col(column)), "BIGINT"))
        };
        let rows: Vec<(Uuid, NaiveDate, String, i64, i64, i64)> = select
            .column(extra_column)
            .column(daily::Column::LocalDay)
            .column(daily::Column::Currency)
            .column_as(sum(daily::Column::AmountMinor), "amount")
            .column_as(sum(daily::Column::Quantity), "quantity")
            .column_as(sum(daily::Column::FactCount), "facts")
            .group_by(extra_column)
            .group_by(daily::Column::LocalDay)
            .group_by(daily::Column::Currency)
            .into_tuple()
            .all(store.database().read())
            .await?;
        // Tỉ giá đọc trước, ngoài transaction, theo khách của NGƯỜI XEM (sổ tỉ giá của họ).
        let mut rates: HashMap<(String, NaiveDate), Option<rust_decimal::Decimal>> = HashMap::new();
        if let Some(target) = query.currency {
            for (_, day, currency, ..) in &rows {
                if currency.is_empty() || currency == target.code() {
                    continue;
                }
                let key = (currency.clone(), *day);
                if rates.contains_key(&key) {
                    continue;
                }
                let rate = self.rate(viewer, currency, target, *day).await?;
                rates.insert(key, rate);
            }
        }
        let mut groups: BTreeMap<String, Accumulator> = BTreeMap::new();
        let mut totals = Accumulator::default();
        let mut missing: BTreeSet<MissingRate> = BTreeSet::new();
        for (extra, day, currency, amount, quantity, facts) in rows {
            let key = match query.group_by {
                GroupBy::Day => day.to_string(),
                GroupBy::Week => (day
                    - Duration::days(i64::from(day.weekday().num_days_from_monday())))
                .to_string(),
                GroupBy::Month => format!("{:04}-{:02}-01", day.year(), day.month()),
                GroupBy::Tenant | GroupBy::Org | GroupBy::Group => extra.to_string(),
            };
            let converted = match query.currency {
                None => None,
                Some(_) if currency.is_empty() => None,
                Some(target) if currency == target.code() => Some(Ok(amount)),
                Some(target) => match rates.get(&(currency.clone(), day)).copied().flatten() {
                    Some(rate) => {
                        let money = Money::from_columns(amount, &currency)?;
                        Some(Ok(money.convert(rate, target, Rounding::HalfUp)?.minor()))
                    }
                    None => {
                        missing.insert(MissingRate { currency: currency.clone(), day });
                        Some(Err(()))
                    }
                },
            };
            // Theo nhóm: một nút ở hai nhóm được tính ở cả hai; tổng chung tính một lần.
            let keys: Vec<String> = if query.group_by == GroupBy::Group {
                visible
                    .groups
                    .iter()
                    .filter(|group| group.members.contains(&extra))
                    .map(|group| group.id.to_string())
                    .collect()
            } else {
                vec![key]
            };
            for key in keys {
                groups
                    .entry(key)
                    .or_default()
                    .add(&currency, amount, quantity, facts, converted)?;
            }
            totals.add(&currency, amount, quantity, facts, converted)?;
        }
        if query.group_by == GroupBy::Group {
            for group in &visible.groups {
                groups.entry(group.id.to_string()).or_default();
            }
        }
        let excluded: BTreeMap<String, u64> =
            visible.groups.iter().map(|group| (group.id.to_string(), group.excluded)).collect();
        let total_excluded = visible.excluded_org_count;
        let finish =
            |key: String, accumulator: Accumulator| accumulator.finish(key, query.currency);
        Ok(Summary {
            kind: kind.to_owned(),
            from,
            to,
            group_by: query.group_by,
            groups: groups
                .into_iter()
                .map(|(key, accumulator)| {
                    let excluded_org_count = if query.group_by == GroupBy::Group {
                        excluded.get(&key).copied()
                    } else {
                        None
                    };
                    finish(key, accumulator)
                        .map(|group| SummaryGroup { excluded_org_count, ..group })
                })
                .collect::<Result<_, _>>()?,
            totals: SummaryGroup {
                excluded_org_count: total_excluded,
                ..finish("total".to_owned(), totals)?
            },
            missing_rates: missing.into_iter().collect(),
        })
    }

    async fn rate(
        &self,
        viewer: &TenantContext,
        currency: &str,
        target: Currency,
        day: NaiveDate,
    ) -> Result<Option<rust_decimal::Decimal>, Error> {
        let Some(ledger) = &self.ledger else { return Ok(None) };
        let base = Money::from_columns(0, currency)?.currency();
        Ok(ledger.exchange_rate(viewer, base, target, day).await?.map(|rate| rate.rate.0))
    }

    /// Một trang số liệu thô của kho, mới nhất trước (mặc định).
    pub(crate) async fn facts(
        &self,
        viewer: &TenantContext,
        principal: &Principal,
        filter: &Filter,
        page: &PageRequest,
    ) -> Result<Page<fact::Model>, Error> {
        principal.require(&REPORTING_READ)?;
        let kind = match filter.kind.as_deref() {
            Some(kind) => Some(self.kind(kind)?),
            None => None,
        };
        if let Some((name, _)) = filter.dimensions.iter().find(|(name, _)| {
            kind.is_none_or(|kind| !kind.kind.dimensions.contains(&name.as_str()))
        }) {
            return Err(Error::new(
                ErrorKind::Validation,
                "reporting.dimension_unknown",
                "The fact kind `{kind}` has no dimension `{dimension}`.",
            )
            .with_param("kind", kind.map_or("", |kind| kind.kind.kind))
            .with_param("dimension", name.chars().take(64).collect::<String>()));
        }
        let (from, to) = Self::range(filter, GroupBy::Day)?;
        let visible = self.visible(viewer, principal, filter).await?;
        let store = self.store(visible.root).await?;
        let root = visible.root.as_uuid();
        let mut select = fact::Entity::find()
            .filter(fact::Column::TenantId.eq(root))
            .filter(fact::Column::SourceTenantId.is_in(visible.tenants))
            .filter(fact::Column::LocalDay.gte(from))
            .filter(fact::Column::LocalDay.lte(to));
        if let Some(kind) = kind {
            select = select.filter(fact::Column::Kind.eq(kind.kind.kind));
        }
        if let Some(orgs) = visible.orgs {
            select = select.filter(fact::Column::OrgId.is_in(orgs));
        }
        for (name, value) in &filter.dimensions {
            let matching = dimension::Entity::find()
                .select_only()
                .column(dimension::Column::FactId)
                .filter(dimension::Column::TenantId.eq(root))
                .filter(dimension::Column::Name.eq(name.as_str()))
                .filter(dimension::Column::Value.eq(value.as_str()))
                .into_query();
            select = select.filter(fact::Column::Id.in_subquery(matching));
        }
        fetch_page(select, fact::Column::Id, |row| row.id, page, store.database().read()).await
    }

    /// Khách cây của người quản lý: khách mình và mọi khách cấp dưới.
    async fn tree(&self, viewer: &TenantContext) -> Result<Vec<TenantId>, Error> {
        let mut tenants = vec![viewer.id()];
        tenants.extend(
            self.databases.tenant_descendants(viewer.id()).await?.into_iter().map(|child| child.id),
        );
        Ok(tenants)
    }

    /// Độ trễ của từng khách trong cây của `viewer`.
    pub(crate) async fn lag(
        &self,
        viewer: &TenantContext,
        principal: &Principal,
    ) -> Result<Vec<Lag>, Error> {
        principal.require(&REPORTING_MANAGE)?;
        let root = self.root_of(viewer.id()).await?;
        let store = self.store(root).await.ok();
        let mut lags = Vec::new();
        for tenant in self.tree(viewer).await? {
            let context = match self.databases.tenant(tenant).await {
                Ok(context) => context,
                Err(_) => continue,
            };
            let (pending, oldest): (i64, Option<chrono::DateTime<Utc>>) =
                local_fact::Entity::find()
                    .select_only()
                    .column_as(
                        Expr::from(Func::count(Expr::col(local_fact::Column::Id))),
                        "pending",
                    )
                    .column_as(
                        Expr::from(Func::min(Expr::col(local_fact::Column::CreatedAt))),
                        "oldest",
                    )
                    .filter(local_fact::Column::TenantId.eq(tenant.as_uuid()))
                    .filter(local_fact::Column::ForwardedAt.is_null())
                    .into_tuple()
                    .one(context.database().read())
                    .await?
                    .unwrap_or((0, None));
            let last = match &store {
                None => None,
                Some(store) => fact::Entity::find()
                    .select_only()
                    .column_as(Expr::from(Func::max(Expr::col(fact::Column::ReceivedAt))), "last")
                    .filter(fact::Column::TenantId.eq(root.as_uuid()))
                    .filter(fact::Column::SourceTenantId.eq(tenant.as_uuid()))
                    .into_tuple::<Option<chrono::DateTime<Utc>>>()
                    .one(store.database().read())
                    .await?
                    .flatten(),
            };
            lags.push(Lag {
                tenant_id: tenant,
                pending: u64::try_from(pending).unwrap_or(0),
                oldest_pending_at: oldest,
                last_received_at: last,
            });
        }
        Ok(lags)
    }

    /// Đẩy lại mọi số liệu cục bộ còn giữ của khách `tenant` (trong cây của `viewer`): đặt lại
    /// `forwarded_at`; unique ở kho chặn đếm hai lần. Trả số dòng sẽ đẩy lại.
    pub(crate) async fn rebuild(
        &self,
        viewer: &TenantContext,
        principal: &Principal,
        tenant: Option<Uuid>,
    ) -> Result<u64, Error> {
        principal.require(&REPORTING_MANAGE)?;
        let tenant = tenant.map_or(viewer.id(), TenantId::from_uuid);
        if !self.tree(viewer).await?.contains(&tenant) {
            return Err(tenant_not_found());
        }
        let context = self.databases.tenant(tenant).await.map_err(|_| tenant_not_found())?;
        let result = local_fact::Entity::update_many()
            .col_expr(local_fact::Column::ForwardedAt, Expr::value(None::<chrono::DateTime<Utc>>))
            .filter(local_fact::Column::TenantId.eq(tenant.as_uuid()))
            .filter(local_fact::Column::ForwardedAt.is_not_null())
            .exec(context.database().write())
            .await?;
        Ok(result.rows_affected)
    }
}

/// Cộng dồn một nhóm của tổng.
#[derive(Debug, Default)]
struct Accumulator {
    by_currency: BTreeMap<String, (i64, i64, i64)>,
    converted: i64,
}

impl Accumulator {
    fn add(
        &mut self,
        currency: &str,
        amount: i64,
        quantity: i64,
        facts: i64,
        converted: Option<Result<i64, ()>>,
    ) -> Result<(), Error> {
        let overflow = || Error::internal("a report total does not fit in 64 bits");
        let slot = self.by_currency.entry(currency.to_owned()).or_default();
        slot.0 = slot.0.checked_add(amount).ok_or_else(overflow)?;
        slot.1 = slot.1.checked_add(quantity).ok_or_else(overflow)?;
        slot.2 = slot.2.checked_add(facts).ok_or_else(overflow)?;
        if let Some(Ok(minor)) = converted {
            self.converted = self.converted.checked_add(minor).ok_or_else(overflow)?;
        }
        Ok(())
    }

    fn finish(self, key: String, target: Option<Currency>) -> Result<SummaryGroup, Error> {
        let by_currency = self
            .by_currency
            .into_iter()
            .map(|(currency, (amount_minor, quantity, count))| {
                let currency = if currency.is_empty() {
                    None
                } else {
                    Some(Money::from_columns(0, &currency)?.currency())
                };
                Ok(CurrencyTotal { currency, amount_minor, quantity, count })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let converted = target.map(|target| Money::new(self.converted, target));
        Ok(SummaryGroup { key, by_currency, converted, excluded_org_count: None })
    }
}
