//! Tool MCP của `ledger`: cùng service, cùng permission với REST.

use autoxide_kernel::time::Date;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::decimal::Rounding;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::mcp::{PageInput, Tool};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::Page;
use autoxide_kernel::tenancy::{OrgId, TenantContext, root_of};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::api::{Entry, LineAccount, NewEntry, NewLine, Side};
use crate::http::{TrialBalanceResponse, describe_trial};
use crate::permissions::{POST, READ, RECONCILE, SETTINGS_MANAGE};
use crate::service::{EntryFilter, LedgerService};

/// Which account and day.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BalanceInput {
    /// Id of the account.
    account_id: Uuid,
    /// End of this day, `YYYY-MM-DD`; omit for the current balance.
    as_of: Option<Date>,
}

/// The balance of an account.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct BalanceItem {
    account_id: Uuid,
    purpose: String,
    currency: String,
    as_of: Option<Date>,
    /// Balance on the normal side of the account.
    balance: Money,
}

/// Which entries.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct EntriesInput {
    /// Only the book of this unit.
    org_id: Option<Uuid>,
    /// Only entries written by this module (`ledger` for manual entries).
    source_module: Option<String>,
    /// Only entries for this source document.
    source_id: Option<String>,
    /// Only entries dated on or after this day.
    from: Option<Date>,
    /// Only entries dated on or before this day.
    to: Option<Date>,
    /// Maximum number of entries, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `true` to also get `pagination.total` (the number of matching items) on the first page.
    count: Option<bool>,
}

/// Which book and days.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TrialInput {
    /// Unit of the book; default the tenant's root.
    org_id: Option<Uuid>,
    /// First day, `YYYY-MM-DD`.
    from: Date,
    /// Last day, `YYYY-MM-DD`.
    to: Date,
}

// Trường phẳng (client MCP không đọc `$ref`): dòng khai inline, số tiền là chữ thập phân theo đơn
// vị chính trong `currency` của bút toán.
/// One line of an entry.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct LineInput {
    account_id: Uuid,
    /// `debit` or `credit`.
    side: String,
    /// Amount as a decimal string in the entry currency, e.g. `"150000"`; greater than zero.
    amount: String,
}

/// A manual entry.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PostInput {
    /// Unit of the book; default the tenant's root.
    org_id: Option<Uuid>,
    /// Accounting day `YYYY-MM-DD`; default today. Must be in an open period.
    effective_date: Option<Date>,
    /// Fixed text, 1 to 200 characters, without personal data (kept for good).
    description: String,
    /// Your id of this entry: sending it again returns the entry already posted.
    reference: String,
    /// Currency code of every line, e.g. `VND`.
    currency: String,
    /// Two or more lines; debits must equal credits.
    #[schema(inline)]
    lines: Vec<LineInput>,
}

/// Which payment.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PaymentInput {
    /// Id of the payment.
    payment_id: Uuid,
}

/// The partner's answer to simulate for a sandbox payment.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SimulateInput {
    /// Id of the payment: for an order, find it with
    /// `GET /api/v1/ledger/payments?source_module=orders&source_id=<order id>`.
    payment_id: Uuid,
    /// `succeeded` (the partner collected the money) or `failed` (the buyer did not pay).
    outcome: String,
    /// The amount the partner reports collected, in the smallest unit of the payment's currency
    /// (e.g. `150000` for 150,000 VND); omit for exactly the amount due. Another amount puts the
    /// payment in `needs_review`.
    amount_minor: Option<i64>,
}

/// Which items.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExceptionsInput {
    /// `open` (default), `resolved` or `ignored`.
    status: Option<String>,
    /// Maximum number of items, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `true` to also get `pagination.total` (the number of matching items) on the first page.
    count: Option<bool>,
}

/// How to resolve an item.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResolveInput {
    /// Id of the item.
    exception_id: Uuid,
    /// `match` (record the money for `payment_id` when it is exactly the amount due),
    /// `accept_received` (record the amount collected even when it differs), `ignore` or
    /// `refunded_outside`.
    action: String,
    /// With `match` and `accept_received`.
    payment_id: Option<Uuid>,
    /// Up to 500 characters.
    note: Option<String>,
}

/// A difference waiting for a person.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ExceptionItem {
    id: Uuid,
    kind: String,
    subject: String,
    #[schema(value_type = Object)]
    details: serde_json::Value,
    status: String,
    resolution: Option<String>,
}

impl From<crate::entity::exception::Model> for ExceptionItem {
    fn from(row: crate::entity::exception::Model) -> Self {
        ExceptionItem {
            id: row.id,
            details: serde_json::from_str(&row.details).unwrap_or_default(),
            kind: row.kind,
            subject: row.subject,
            status: row.status,
            resolution: row.resolution,
        }
    }
}

pub(crate) fn all(service: &Arc<LedgerService>) -> Vec<Tool> {
    let (balance, entries, trial, post) =
        (service.clone(), service.clone(), service.clone(), service.clone());
    let (payment, exceptions, resolve) = (service.clone(), service.clone(), service.clone());
    let (rate, record, simulate) = (service.clone(), service.clone(), service.clone());
    vec![
        Tool::new(
            "simulate_payment",
            "Inside a sandbox only (and when the server allows it): play the payment partner for a \
             pending sandbox payment, reporting it `succeeded` or `failed`. Goes through the same path \
             as a partner notification (entry posted, the order told); no money moves.",
            move |tenant: TenantContext, principal: Principal, input: SimulateInput| {
                let service = simulate.clone();
                async move {
                    let amount = input.amount_minor.map(crate::simulate::Reported::Minor);
                    service
                        .simulate_payment(&tenant, &principal, input.payment_id, &input.outcome, amount)
                        .await
                }
            },
        )
        .requires(RECONCILE),
        Tool::new(
            "payment",
            "A payment collected through a payment partner, with its status and refunded amount.",
            move |tenant: TenantContext, principal: Principal, input: PaymentInput| {
                let service = payment.clone();
                async move {
                    let row = service.payment_for(&tenant, &principal, &READ, input.payment_id).await?;
                    crate::payments::describe_payment(&row)
                }
            },
        )
        .requires(READ)
        .read_only(),
        Tool::new(
            "exceptions",
            "Differences between the book and the payment partners that wait for a person \
             (amount mismatches, unknown or missing transactions, late payments).",
            move |tenant: TenantContext, principal: Principal, input: ExceptionsInput| {
                let service = exceptions.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None).with_count(input.count.unwrap_or(false)).page()?;
                    let status = Some(input.status.unwrap_or_else(|| "open".to_owned()));
                    let found = service.list_exceptions(&tenant, &principal, status, &page).await?;
                    Ok::<Page<ExceptionItem>, Error>(found.map(ExceptionItem::from))
                }
            },
        )
        .requires(READ)
        .read_only(),
        Tool::new(
            "resolve_exception",
            "Resolve a difference: `match` records the money the partner collected for a payment \
             when it is exactly the amount due, `accept_received` records the amount collected even \
             when it differs, `ignore` closes it, `refunded_outside` notes it was refunded outside \
             Autoxide.",
            move |tenant: TenantContext, principal: Principal, input: ResolveInput| {
                let service = resolve.clone();
                async move {
                    use crate::reconcile::Resolution;
                    let resolution = match (input.action.as_str(), input.payment_id) {
                        ("match", Some(payment)) => Resolution::Match(payment),
                        ("accept_received", Some(payment)) => {
                            Resolution::AcceptReceived(payment)
                        }
                        ("ignore", _) => Resolution::Ignore,
                        ("refunded_outside", _) => Resolution::RefundedOutside,
                        _ => {
                            return Err(Error::validation(vec![FieldError::new(
                                "action",
                                "invalid",
                                "must be `match` or `accept_received` (with `payment_id`), `ignore` or `refunded_outside`",
                            )]));
                        }
                    };
                    let row = service
                        .resolve_exception(&tenant, &principal, input.exception_id, resolution, input.note)
                        .await?;
                    Ok::<_, Error>(ExceptionItem::from(row))
                }
            },
        )
        .requires(RECONCILE),
        Tool::new(
            "balance",
            "The balance of a ledger account, now or at the end of a day.",
            move |tenant: TenantContext, principal: Principal, input: BalanceInput| {
                let service = balance.clone();
                async move {
                    let (found, amount) = service
                        .balance_for(&tenant, &principal, input.account_id, input.as_of)
                        .await?;
                    Ok::<_, Error>(BalanceItem {
                        account_id: found.id,
                        purpose: found.purpose,
                        currency: found.currency,
                        as_of: input.as_of,
                        balance: amount,
                    })
                }
            },
        )
        .requires(READ)
        .read_only(),
        Tool::new(
            "entries",
            "List ledger entries in your scope with their lines, newest first; follow \
             `pagination.next_cursor` for more.",
            move |tenant: TenantContext, principal: Principal, input: EntriesInput| {
                let service = entries.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None).with_count(input.count.unwrap_or(false)).page()?;
                    let filter = EntryFilter {
                        book_org: input.org_id.map(OrgId::from_uuid),
                        source_module: input.source_module,
                        source_id: input.source_id,
                        from: input.from,
                        to: input.to,
                    };
                    let found: Page<Entry> =
                        service.list_entries(&tenant, &principal, filter, &page).await?;
                    Ok::<_, Error>(found)
                }
            },
        )
        .requires(READ)
        .read_only(),
        Tool::new(
            "trial_balance",
            "The trial balance of a book for a range of days: opening balance, debits, credits and \
             closing balance per account.",
            move |tenant: TenantContext, principal: Principal, input: TrialInput| {
                let service = trial.clone();
                async move {
                    let org = input.org_id.map_or_else(|| root_of(tenant.id()), OrgId::from_uuid);
                    let book = service.book_of(&tenant, &principal, &READ, org).await?;
                    let (book, rows) = service
                        .trial_balance(&tenant, &principal, book.id, input.from, input.to)
                        .await?;
                    let found: TrialBalanceResponse =
                        describe_trial(book, input.from, input.to, rows)?;
                    Ok::<_, Error>(found)
                }
            },
        )
        .requires(READ)
        .read_only(),
        Tool::new(
            "post_entry",
            "Post a manual double entry (debits equal credits). Entries are never changed: correct \
             one by reversing it. The same reference and content again returns the entry already \
             posted.",
            move |tenant: TenantContext, principal: Principal, input: PostInput| {
                let service = post.clone();
                async move {
                    let currency = Currency::parse(&input.currency)?;
                    let mut problems = Vec::new();
                    let mut lines = Vec::with_capacity(input.lines.len());
                    for (index, item) in input.lines.into_iter().enumerate() {
                        let Some(side) = Side::parse(&item.side) else {
                            problems.push(FieldError::new(
                                format!("lines[{index}].side"),
                                "invalid",
                                "must be `debit` or `credit`",
                            ));
                            continue;
                        };
                        let amount = Money::from_decimal(
                            autoxide_kernel::decimal::parse(&item.amount)?,
                            currency,
                            Rounding::HalfUp,
                        )?;
                        lines.push(NewLine {
                            account: LineAccount::Id(item.account_id),
                            side,
                            amount,
                        });
                    }
                    if !problems.is_empty() {
                        return Err(Error::validation(problems));
                    }
                    let org = input.org_id.map_or_else(|| root_of(tenant.id()), OrgId::from_uuid);
                    let mut entry = NewEntry::new(
                        org,
                        "ledger",
                        "manual",
                        input.reference,
                        input.description,
                        lines,
                    );
                    entry.effective_date = input.effective_date;
                    let found = service.post_manual(&tenant, &principal, entry).await?.into_inner();
                    Ok::<_, Error>(found)
                }
            },
        )
        .requires(POST),
        Tool::new(
            "exchange_rate",
            "The exchange rate in force on a day for a currency pair (1 base = rate quote): the latest \
             rate on or before that day, a rate entered by hand first.",
            move |tenant: TenantContext, principal: Principal, input: RateLookupInput| {
                let service = rate.clone();
                async move { service.effective_rate(&tenant, &principal, &input.base, &input.quote, input.on).await }
            },
        )
        .requires(READ)
        .read_only(),
        Tool::new(
            "record_exchange_rate",
            "Record an exchange rate by hand for the whole tenant, used from its day until a later \
             rate of the pair. Recorded rates are never changed.",
            move |tenant: TenantContext, principal: Principal, input: RateRecordInput| {
                let service = record.clone();
                async move {
                    let rate = autoxide_kernel::decimal::parse(&input.rate)?;
                    let input = crate::fx::RateInput {
                        base: input.base,
                        quote: input.quote,
                        rate,
                        effective_date: input.effective_date,
                    };
                    Ok::<_, Error>(service.record_rate(&tenant, &principal, input).await?.into_inner())
                }
            },
        )
        .requires(SETTINGS_MANAGE),
    ]
}

/// A currency pair and day.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RateLookupInput {
    /// e.g. `USD`.
    base: String,
    /// e.g. `VND`.
    quote: String,
    /// The day, `YYYY-MM-DD`.
    on: Date,
}

/// An exchange rate to record.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RateRecordInput {
    /// e.g. `USD`.
    base: String,
    /// e.g. `VND`.
    quote: String,
    /// Units of `quote` for one `base`, decimal string, e.g. `"25345.5"`.
    rate: String,
    /// First day the rate is used, `YYYY-MM-DD`.
    effective_date: Date,
}
