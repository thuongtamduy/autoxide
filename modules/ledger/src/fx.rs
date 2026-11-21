//! Tỉ giá có ngày hiệu lực theo cặp tiền (bước 36 mục 10, D-237, D-352).
//!
//! - Một dòng `ledger_fx_rates` = "từ ngày `effective_date`, 1 `base` = `rate` `quote`", tới khi có
//!   dòng mới hơn của cùng cặp. Nguồn: `manual` (người nhập) hay `provider:<tên>` (lấy qua
//!   `ExchangeRateSource` của kernel). Cùng ngày có cả hai thì tỉ giá nhập tay thắng.
//! - Tỉ giá đã ghi không sửa (chứng từ, bút toán đã dùng nó giữ ảnh chụp, nhưng sổ tỉ giá vẫn phải
//!   cho biết tỉ giá nào có hiệu lực lúc đó): ghi lại cùng giá trị → trả bản cũ; khác giá trị →
//!   `ledger.exchange_rate_exists` (nhập tỉ giá của ngày khác).
//! - Tỉ giá là chữ thập phân (`rust_decimal`), không bao giờ là số thực.
//! - Lấy từ nguồn: gọi đối tác NGOÀI transaction, rồi ghi một transaction ngắn.

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::decimal::{self, Decimal, DecimalString};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::money::Currency;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext, root_of};
use autoxide_kernel::time::Date;
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set};
use uuid::Uuid;

use crate::api::ExchangeRate;
use crate::entity::fx_rate;
use crate::permissions::{READ, SETTINGS_MANAGE};
use crate::service::{LedgerService, now};

/// Nguồn của tỉ giá nhập tay.
pub(crate) const MANUAL: &str = "manual";

fn rate_exists() -> Error {
    Error::conflict(
        "ledger.exchange_rate_exists",
        "A different rate for this pair, day and source is already recorded; recorded rates are not \
         changed. Record the new rate from another day.",
    )
}

pub(crate) fn rate_not_found(base: Currency, quote: Currency, on: Date) -> Error {
    Error::not_found(
        "ledger.exchange_rate_not_found",
        "There is no {base}/{quote} exchange rate in force on {date}. Record one with `POST \
         /api/v1/ledger/exchange-rates`.",
    )
    .with_param("base", base.code())
    .with_param("quote", quote.code())
    .with_param("date", on.format("%Y-%m-%d").to_string())
}

fn source_not_configured(provider: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "ledger.rate_source_not_configured",
        "No exchange-rate source `{provider}` is set up on this server.",
    )
    .with_param("provider", provider)
}

pub(crate) fn describe_rate(row: fx_rate::Model) -> Result<ExchangeRate, Error> {
    Ok(ExchangeRate {
        rate: DecimalString(decimal::from_column(&row.rate)?),
        id: row.id,
        base: row.base,
        quote: row.quote,
        effective_date: row.effective_date,
        source: row.source,
        created_at: row.created_at,
    })
}

/// Một tỉ giá cần ghi tay.
#[derive(Debug, Clone)]
pub(crate) struct RateInput {
    pub base: String,
    pub quote: String,
    pub rate: Decimal,
    pub effective_date: Date,
}

/// Kiểm cặp tiền và tỉ giá; mọi lỗi gom một lần.
fn checked(input: &RateInput) -> Result<(Currency, Currency), Error> {
    let mut problems = Vec::new();
    let base = Currency::parse(&input.base);
    let quote = Currency::parse(&input.quote);
    if base.is_err() {
        problems.push(FieldError::new("base", "currency", "must be an ISO 4217 currency code"));
    }
    if quote.is_err() {
        problems.push(FieldError::new("quote", "currency", "must be an ISO 4217 currency code"));
    }
    if let (Ok(base), Ok(quote)) = (&base, &quote)
        && base == quote
    {
        problems.push(FieldError::new("quote", "same", "must differ from `base`"));
    }
    if input.rate <= Decimal::ZERO {
        problems.push(FieldError::new("rate", "range", "must be greater than zero"));
    }
    if decimal::to_column(input.rate).len() > 40 {
        problems.push(FieldError::new("rate", "length", "must have at most 40 characters"));
    }
    match (base, quote, problems.is_empty()) {
        (Ok(base), Ok(quote), true) => Ok((base, quote)),
        _ => Err(Error::validation(problems)),
    }
}

impl LedgerService {
    /// Tỉ giá hiệu lực ngày `on` của cặp (`base`, `quote`): ngày gần nhất ≤ `on`, nhập tay trước.
    pub(crate) async fn exchange_rate(
        &self,
        tenant: &TenantContext,
        base: Currency,
        quote: Currency,
        on: Date,
    ) -> Result<Option<fx_rate::Model>, Error> {
        // Vài dòng mới nhất là đủ: một ngày chỉ có một dòng mỗi nguồn.
        let rows = fx_rate::Entity::find()
            .scoped(tenant)
            .filter(fx_rate::Column::Base.eq(base.code()))
            .filter(fx_rate::Column::Quote.eq(quote.code()))
            .filter(fx_rate::Column::EffectiveDate.lte(on))
            .order_by_desc(fx_rate::Column::EffectiveDate)
            .order_by_asc(fx_rate::Column::Source)
            .limit(16)
            .all(tenant.database().read())
            .await?;
        let Some(latest) = rows.first().map(|row| row.effective_date) else { return Ok(None) };
        let same_day: Vec<fx_rate::Model> =
            rows.into_iter().filter(|row| row.effective_date == latest).collect();
        let manual = same_day.iter().position(|row| row.source == MANUAL).unwrap_or(0);
        Ok(same_day.into_iter().nth(manual))
    }

    /// Tỉ giá hiệu lực qua REST / MCP (`ledger.read`); không có → `ledger.exchange_rate_not_found`.
    pub(crate) async fn effective_rate(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        base: &str,
        quote: &str,
        on: Date,
    ) -> Result<ExchangeRate, Error> {
        principal.require(&READ)?;
        let mut problems = Vec::new();
        let parsed_base = Currency::parse(base);
        let parsed_quote = Currency::parse(quote);
        if parsed_base.is_err() {
            problems.push(FieldError::new("base", "currency", "must be an ISO 4217 currency code"));
        }
        if parsed_quote.is_err() {
            problems.push(FieldError::new(
                "quote",
                "currency",
                "must be an ISO 4217 currency code",
            ));
        }
        let (Ok(base), Ok(quote)) = (parsed_base, parsed_quote) else {
            return Err(Error::validation(problems));
        };
        let found = self.exchange_rate(tenant, base, quote, on).await?;
        describe_rate(found.ok_or_else(|| rate_not_found(base, quote, on))?)
    }

    /// Danh sách tỉ giá (`ledger.read`), lọc theo cặp.
    pub(crate) async fn list_rates(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        base: Option<String>,
        quote: Option<String>,
        page: &PageRequest,
    ) -> Result<Page<ExchangeRate>, Error> {
        principal.require(&READ)?;
        let mut select = fx_rate::Entity::find().scoped(tenant);
        if let Some(base) = base {
            select = select.filter(fx_rate::Column::Base.eq(base.to_ascii_uppercase()));
        }
        if let Some(quote) = quote {
            select = select.filter(fx_rate::Column::Quote.eq(quote.to_ascii_uppercase()));
        }
        let found =
            fetch_page(select, fx_rate::Column::Id, |row| row.id, page, tenant.database().read())
                .await?;
        let data = found.data.into_iter().map(describe_rate).collect::<Result<Vec<_>, _>>()?;
        Ok(Page { data, pagination: found.pagination })
    }

    /// Kiểm `ledger.settings_manage` ở gốc khách: tỉ giá dùng chung cả khách.
    async fn require_rates_manage(
        tenant: &TenantContext,
        principal: &Principal,
    ) -> Result<(), Error> {
        principal.require(&SETTINGS_MANAGE)?;
        principal
            .require_at(
                tenant.database().read(),
                &SETTINGS_MANAGE,
                OrgTarget::new(root_of(tenant.id()), None),
            )
            .await
    }

    /// Ghi một tỉ giá nhập tay (`ledger.settings_manage` cho cả khách).
    pub(crate) async fn record_rate(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: RateInput,
    ) -> Result<Submitted<ExchangeRate>, Error> {
        Self::require_rates_manage(tenant, principal).await?;
        let (base, quote) = checked(&input)?;
        let by = Some(principal.user_id());
        let (stored, inserted) =
            Self::store_rate(tenant, base, quote, input.rate, input.effective_date, MANUAL, by)
                .await?;
        let rate = describe_rate(stored.ok_or_else(rate_exists)?)?;
        Ok(if inserted { Submitted::Created(rate) } else { Submitted::Replayed(rate) })
    }

    /// Ghi tỉ giá (một transaction ngắn). `Some(dòng)` khi mới ghi hoặc đã có đúng giá trị đó; `None`
    /// khi đã có giá trị khác (tỉ giá không sửa). Kèm `true` khi vừa ghi mới.
    async fn store_rate(
        tenant: &TenantContext,
        base: Currency,
        quote: Currency,
        rate: Decimal,
        on: Date,
        source: &str,
        by: Option<Uuid>,
    ) -> Result<(Option<fx_rate::Model>, bool), Error> {
        let text = decimal::to_column(rate);
        let transaction = tenant.database().begin_write().await?;
        let inserted = fx_rate::Entity::insert(fx_rate::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            base: Set(base.code().to_owned()),
            quote: Set(quote.code().to_owned()),
            rate: Set(text.clone()),
            effective_date: Set(on),
            source: Set(source.to_owned()),
            created_by: Set(by),
            created_at: Set(now()),
        })
        .on_conflict(
            OnConflict::columns([
                fx_rate::Column::TenantId,
                fx_rate::Column::Base,
                fx_rate::Column::Quote,
                fx_rate::Column::EffectiveDate,
                fx_rate::Column::Source,
            ])
            .do_nothing()
            .to_owned(),
        )
        .exec_without_returning(&transaction)
        .await?;
        let stored = fx_rate::Entity::find()
            .scoped(tenant)
            .filter(fx_rate::Column::Base.eq(base.code()))
            .filter(fx_rate::Column::Quote.eq(quote.code()))
            .filter(fx_rate::Column::EffectiveDate.eq(on))
            .filter(fx_rate::Column::Source.eq(source))
            .one(&transaction)
            .await?
            .ok_or_else(|| Error::internal("an exchange rate vanished after it was written"))?;
        transaction.commit().await?;
        Ok(((stored.rate == text).then_some(stored), inserted > 0))
    }

    /// Lấy tỉ giá của `base` ngày `on` từ nguồn `provider` (bỏ trống: nguồn duy nhất đã cấu hình) và
    /// ghi lại (`ledger.settings_manage` cho cả khách). Trả các tỉ giá đã ghi.
    pub(crate) async fn fetch_rates(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        provider: Option<String>,
        base: &str,
        on: Option<Date>,
    ) -> Result<Vec<ExchangeRate>, Error> {
        Self::require_rates_manage(tenant, principal).await?;
        let base = Currency::parse(base).map_err(|_| {
            Error::validation(vec![FieldError::new(
                "base",
                "currency",
                "must be an ISO 4217 currency code",
            )])
        })?;
        let source = match provider.as_deref() {
            Some(name) => self
                .rate_sources
                .iter()
                .find(|source| source.provider() == name)
                .cloned()
                .ok_or_else(|| source_not_configured(name))?,
            None => match self.rate_sources.as_slice() {
                [only] => only.clone(),
                _ => return Err(source_not_configured(provider.as_deref().unwrap_or("-"))),
            },
        };
        let on = match on {
            Some(day) => day,
            None => self.today(tenant, root_of(tenant.id())).await,
        };
        // Gọi đối tác ngoài mọi transaction.
        let quoted = source.rates(tenant.id(), base, on).await?;
        let name = format!("provider:{}", source.provider());
        let mut stored = Vec::with_capacity(quoted.len());
        for item in quoted {
            if item.base == item.quote || item.rate <= Decimal::ZERO {
                continue;
            }
            if let (Some(row), _) = Self::store_rate(
                tenant,
                item.base,
                item.quote,
                item.rate,
                item.on,
                &name,
                Some(principal.user_id()),
            )
            .await?
            {
                stored.push(describe_rate(row)?);
            }
        }
        Ok(stored)
    }
}
