//! Bảng tổng hợp thuế GTGT đầu ra theo kỳ và thuế suất (đề bài giao diện, D-383): cộng từ các dòng
//! của chứng từ bán đã có số trong kỳ. Đây là số tham khảo để đối chiếu khi lập tờ khai — cách tính
//! (chứng từ nào vào, điều chỉnh giảm trừ thế nào) **cần người có chuyên môn kế toán / thuế đối chiếu**.
//! Thuế GTGT đầu vào cần chứng từ mua, chưa có (bước 42).

use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use axum::extract::{Query, State};
use chrono::NaiveDate;
use sea_orm::sea_query::{Expr, ExprTrait as _, Func};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect, QueryTrait};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::entity::{document, line};
use crate::permissions::READ;
use crate::service::InvoicesService;

/// Trạng thái của chứng từ được cộng: đã có số và còn hiệu lực. Bị thay thế, huỷ, bị cơ quan thuế từ
/// chối thì không cộng (chứng từ thay thế / phát hành lại được cộng thay).
const COUNTED: [&str; 4] = ["issued", "submitting", "accepted", "adjusted"];
/// Kỳ dài nhất (ngày).
const MAX_DAYS: i64 = 366;

/// (loại chứng từ, tiền tệ, thuế suất).
type RowKey = (String, String, Option<String>);
/// (trước thuế, thuế, số chứng từ) theo đơn vị nhỏ nhất.
type Totals = (i64, i64, i64);

/// One row of the VAT summary.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = InvoicesVatSummaryRow)]
pub(crate) struct VatRow {
    /// Kind of document, e.g. `vat_invoice`.
    kind: String,
    /// VAT rate in percent as written on the lines, e.g. `"10"`; `null` for lines without a rate
    /// (not subject to VAT).
    rate_percent: Option<String>,
    /// Amount before tax: increases minus decrease adjustments.
    net: Money,
    /// VAT: increases minus decrease adjustments.
    tax: Money,
    /// Number of documents with lines in this row.
    documents: i64,
}

/// Output VAT of a period.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = InvoicesVatSummary)]
pub(crate) struct VatSummary {
    from: NaiveDate,
    to: NaiveDate,
    /// The unit, or `null` for every unit in your scope.
    org_id: Option<Uuid>,
    /// One row per kind of document, currency and VAT rate.
    rows: Vec<VatRow>,
}

/// Which period.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct VatQuery {
    /// First day, `YYYY-MM-DD`.
    from: NaiveDate,
    /// Last day, `YYYY-MM-DD`; at most 366 days after `from`.
    to: NaiveDate,
    /// Only the documents of this unit (seller); omit for every unit in your scope.
    org_id: Option<Uuid>,
}

impl InvoicesService {
    pub(crate) async fn vat_summary(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        from: NaiveDate,
        to: NaiveDate,
        org: Option<Uuid>,
    ) -> Result<Vec<VatRow>, Error> {
        principal.require(&READ)?;
        if to < from {
            return Err(Error::validation(vec![FieldError::new(
                "to",
                "range",
                "must not be before `from`",
            )]));
        }
        if (to - from).num_days() >= MAX_DAYS {
            return Err(Error::validation(vec![FieldError::new(
                "to",
                "range",
                "the period must be at most 366 days long",
            )]));
        }
        let condition = principal.scope_condition(&READ, document::Column::OrgId, None)?;
        // `None`: mọi chứng từ được cộng; `Some(true)`: chỉ điều chỉnh giảm; `Some(false)`: còn lại.
        let documents = |decrease: Option<bool>| {
            let mut select = document::Entity::find()
                .scoped(tenant)
                .filter(condition.clone())
                .filter(document::Column::IssueDate.gte(from))
                .filter(document::Column::IssueDate.lte(to))
                .filter(document::Column::Number.is_not_null())
                .filter(document::Column::Status.is_in(COUNTED));
            select = match decrease {
                Some(true) => select.filter(document::Column::Direction.eq("decrease")),
                Some(false) => select.filter(document::Column::Direction.ne("decrease")),
                None => select,
            };
            if let Some(org) = org {
                select = select.filter(document::Column::OrgId.eq(org));
            }
            select
        };
        let read = tenant.database().read();
        // Loại chứng từ và tiền tệ có trong kỳ (ít), rồi một câu cộng gộp theo thuế suất cho mỗi cặp
        // và mỗi chiều (tăng / giảm) — câu con lọc chứng từ, không join.
        let pairs: Vec<(String, String)> = documents(None)
            .select_only()
            .column(document::Column::Kind)
            .column(document::Column::Currency)
            .distinct()
            .into_tuple()
            .all(read)
            .await?;
        let mut rows: BTreeMap<RowKey, Totals> = BTreeMap::new();
        for (kind, currency) in pairs {
            for decrease in [false, true] {
                let ids = documents(Some(decrease))
                    .filter(document::Column::Kind.eq(kind.as_str()))
                    .filter(document::Column::Currency.eq(currency.as_str()))
                    .select_only()
                    .column(document::Column::Id)
                    .into_query();
                let sums: Vec<(Option<String>, i64, i64, i64)> = line::Entity::find()
                    .scoped(tenant)
                    .select_only()
                    .column(line::Column::RatePercent)
                    // Postgres trả `numeric` cho `SUM(bigint)`: ép về BIGINT cho cả hai database.
                    .column_as(
                        Expr::from(Func::cast_as(
                            Func::sum(Expr::col(line::Column::NetMinor)),
                            "BIGINT",
                        )),
                        "net",
                    )
                    .column_as(
                        Expr::from(Func::cast_as(
                            Func::sum(Expr::col(line::Column::TaxMinor)),
                            "BIGINT",
                        )),
                        "tax",
                    )
                    .column_as(Expr::col(line::Column::DocumentId).count_distinct(), "documents")
                    .filter(line::Column::DocumentId.in_subquery(ids))
                    .group_by(line::Column::RatePercent)
                    .into_tuple()
                    .all(read)
                    .await?;
                let sign = if decrease { -1 } else { 1 };
                for (rate, net, tax, count) in sums {
                    let entry = rows.entry((kind.clone(), currency.clone(), rate)).or_default();
                    entry.0 += sign * net;
                    entry.1 += sign * tax;
                    entry.2 += count;
                }
            }
        }
        rows.into_iter()
            .map(|((kind, currency, rate_percent), (net, tax, documents))| {
                let currency = Currency::parse(&currency)?;
                Ok(VatRow {
                    kind,
                    rate_percent,
                    net: Money::new(net, currency),
                    tax: Money::new(tax, currency),
                    documents,
                })
            })
            .collect()
    }
}

/// Output VAT of a period by kind of document and VAT rate.
///
/// Requires `invoices.read`; counts the documents of the units in your scope. Adds up the lines of
/// the sales documents numbered in the period (`issued`, `submitting`, `accepted`, `adjusted`):
/// replaced, cancelled and rejected documents are left out, decrease adjustments are subtracted.
/// A figure to check the VAT return against, not the return itself — have an accountant review
/// how it is counted. Input VAT (purchases) is not included.
#[utoipa::path(
    get,
    path = "/vat-summary",
    extensions(("x-permissions" = json!(["invoices.read"]))),
    operation_id = "invoice_vat_summary",
    security(("bearer" = [])),
    params(VatQuery),
    responses(
        (status = 200, description = "The summary.", body = Data<VatSummary>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A parameter is invalid (missing date, period longer than 366 days…); see `errors`.", body = Problem),
    )
)]
async fn vat_summary(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<VatQuery>,
) -> Result<Data<VatSummary>, Error> {
    let rows = service.vat_summary(&tenant, &principal, query.from, query.to, query.org_id).await?;
    Ok(Data { data: VatSummary { from: query.from, to: query.to, org_id: query.org_id, rows } })
}

pub(crate) fn routes(service: Arc<InvoicesService>) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(vat_summary)).with_state(service)
}
