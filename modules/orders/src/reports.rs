//! Báo cáo bán hàng (đặc tả 45 mục "Báo cáo bán hàng", D-213) và nguồn xuất đơn qua xuất chung của
//! bước 51 (P16) — không có endpoint xuất riêng.
//!
//! - Đơn được tính: đã đặt trong khoảng `[from, to)`, không còn chờ thanh toán, không bị huỷ; tiền
//!   theo một loại tiền (`currency`, mặc định VND). Phạm vi theo quyền `orders.reports.read` ở nút
//!   bán (nhân viên cửa hàng A không thấy số của B; vùng thấy cả nhánh).
//! - `org` / `channel`: `GROUP BY` trong SQL (tổng bằng `CAST(SUM(…) AS BIGINT)` — Postgres trả
//!   `numeric` cho tổng `bigint`); `product`: tổng các dòng của đơn được tính; `day` / `month`: gom
//!   theo ngày UTC trong Rust (hàm ngày tháng khác nhau giữa hai database), khoảng tối đa 366 ngày.
//!   Truy vấn thẳng bảng có index (`tenant_id`, `org_id`, `placed_at`) — chỉ thêm bảng tổng hợp khi
//!   đo thấy chậm (D-20261120-step45-reviews-risk-reports).
//! - Kho báo cáo gộp (bước 38) dùng event `orders.order_*` (payload có `tenant_id`, `org_id`,
//!   `channel`, tiền, dòng).

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::access::{Permission, Principal};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::list_exports::{ExportInfo, ExportPage, ExportQuery, ExportSource};
use autoxide_kernel::money::Currency;
use autoxide_kernel::pagination::{PageQuery, PageRequest};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::{Alias, Expr, Func, Query};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QuerySelect};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entity::{line, order};
use crate::permissions::{READ, REPORTS_READ};
use crate::service::OrdersService;

/// Cách gom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GroupBy {
    Org,
    Channel,
    Product,
    Day,
    Month,
}

impl GroupBy {
    pub(crate) fn parse(raw: &str) -> Option<GroupBy> {
        match raw {
            "org" => Some(GroupBy::Org),
            "channel" => Some(GroupBy::Channel),
            "product" => Some(GroupBy::Product),
            "day" => Some(GroupBy::Day),
            "month" => Some(GroupBy::Month),
            _ => None,
        }
    }
}

/// Một dòng báo cáo.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SalesRow {
    /// ID nút / mã kênh / ID sản phẩm / `YYYY-MM-DD` / `YYYY-MM`.
    pub key: String,
    pub orders: i64,
    /// Số đơn vị bán (chỉ khi gom theo sản phẩm).
    pub quantity: Option<i64>,
    /// Người mua trả (gồm thuế, sau giảm).
    pub gross: i64,
    pub discount: i64,
    pub tax: i64,
    pub refunded: i64,
}

/// (số đơn, tổng trả, giảm, thuế, đã hoàn, khoá gom).
type GroupRow<K> = (i64, Option<i64>, Option<i64>, Option<i64>, Option<i64>, K);
/// (sản phẩm, số đơn, số lượng, tổng trả, giảm, thuế).
type ProductRow = (Uuid, i64, Option<i64>, Option<i64>, Option<i64>, Option<i64>);

fn sum(column: order::Column) -> Expr {
    Func::cast_as(Func::sum(Expr::col(column)), Alias::new("BIGINT")).into()
}

fn sum_line(column: line::Column) -> Expr {
    Func::cast_as(Func::sum(Expr::col(column)), Alias::new("BIGINT")).into()
}

impl OrdersService {
    /// `GET /reports/sales`.
    pub(crate) async fn sales_report(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        group_by: GroupBy,
        from: Timestamp,
        to: Timestamp,
        currency: Currency,
    ) -> Result<Vec<SalesRow>, Error> {
        principal.require(&REPORTS_READ)?;
        if to <= from || to - from > SignedDuration::from_hours(24 * 366) {
            return Err(Error::validation(vec![FieldError::new(
                "to",
                "range",
                "must be after `from`, at most 366 days later",
            )]));
        }
        let scope = principal.scope_condition(&REPORTS_READ, order::Column::OrgId, None)?;
        let counted = Condition::all()
            .add(scope)
            .add(order::Column::PlacedAt.gte(from))
            .add(order::Column::PlacedAt.lt(to))
            .add(order::Column::Currency.eq(currency.code()))
            .add(order::Column::Status.is_not_in(["pending_payment", "cancelled"]));
        let read = tenant.database().read();
        let rows = match group_by {
            GroupBy::Org | GroupBy::Channel => {
                let column = if group_by == GroupBy::Org {
                    order::Column::OrgId
                } else {
                    order::Column::Channel
                };
                let select = order::Entity::find()
                    .select_only()
                    .filter(counted)
                    .scoped(tenant)
                    .column_as(order::Column::Id.count(), "orders")
                    .column_as(sum(order::Column::TotalMinor), "gross")
                    .column_as(sum(order::Column::DiscountMinor), "discount")
                    .column_as(sum(order::Column::TaxMinor), "tax")
                    .column_as(sum(order::Column::RefundedMinor), "refunded")
                    .group_by(column);
                if group_by == GroupBy::Org {
                    let found: Vec<GroupRow<Uuid>> =
                        select.column(order::Column::OrgId).into_tuple().all(read).await?;
                    found
                        .into_iter()
                        .map(|(orders, gross, discount, tax, refunded, org)| SalesRow {
                            key: org.to_string(),
                            orders,
                            quantity: None,
                            gross: gross.unwrap_or(0),
                            discount: discount.unwrap_or(0),
                            tax: tax.unwrap_or(0),
                            refunded: refunded.unwrap_or(0),
                        })
                        .collect()
                } else {
                    let found: Vec<GroupRow<String>> =
                        select.column(order::Column::Channel).into_tuple().all(read).await?;
                    found
                        .into_iter()
                        .map(|(orders, gross, discount, tax, refunded, channel)| SalesRow {
                            key: channel,
                            orders,
                            quantity: None,
                            gross: gross.unwrap_or(0),
                            discount: discount.unwrap_or(0),
                            tax: tax.unwrap_or(0),
                            refunded: refunded.unwrap_or(0),
                        })
                        .collect()
                }
            }
            GroupBy::Product => {
                let ids = Query::select()
                    .column(order::Column::Id)
                    .from(order::Entity)
                    .and_where(order::Column::TenantId.eq(tenant.id().as_uuid()))
                    .cond_where(counted)
                    .to_owned();
                let found: Vec<ProductRow> = line::Entity::find()
                    .select_only()
                    .column(line::Column::ProductId)
                    .column_as(
                        Expr::from(Func::cast_as(
                            Func::count_distinct(Expr::col(line::Column::OrderId)),
                            Alias::new("BIGINT"),
                        )),
                        "orders",
                    )
                    .column_as(sum_line(line::Column::Quantity), "quantity")
                    .column_as(sum_line(line::Column::TotalMinor), "gross")
                    .column_as(sum_line(line::Column::DiscountMinor), "discount")
                    .column_as(sum_line(line::Column::TaxMinor), "tax")
                    .filter(line::Column::OrderId.in_subquery(ids))
                    .scoped(tenant)
                    .group_by(line::Column::ProductId)
                    .into_tuple()
                    .all(read)
                    .await?;
                found
                    .into_iter()
                    .map(|(product, orders, quantity, gross, discount, tax)| SalesRow {
                        key: product.to_string(),
                        orders,
                        quantity: Some(quantity.unwrap_or(0)),
                        gross: gross.unwrap_or(0),
                        discount: discount.unwrap_or(0),
                        tax: tax.unwrap_or(0),
                        refunded: 0,
                    })
                    .collect()
            }
            GroupBy::Day | GroupBy::Month => {
                let found: Vec<(Timestamp, i64, i64, i64, i64)> = order::Entity::find()
                    .select_only()
                    .column(order::Column::PlacedAt)
                    .column(order::Column::TotalMinor)
                    .column(order::Column::DiscountMinor)
                    .column(order::Column::TaxMinor)
                    .column(order::Column::RefundedMinor)
                    .filter(counted)
                    .scoped(tenant)
                    .into_tuple()
                    .all(read)
                    .await?;
                let format = if group_by == GroupBy::Day { "%Y-%m-%d" } else { "%Y-%m" };
                let mut buckets: BTreeMap<String, SalesRow> = BTreeMap::new();
                for (at, total, discount, tax, refunded) in found {
                    let key = at.format(format).to_string();
                    let row = buckets
                        .entry(key.clone())
                        .or_insert_with(|| SalesRow { key, ..SalesRow::default() });
                    row.orders += 1;
                    row.gross += total;
                    row.discount += discount;
                    row.tax += tax;
                    row.refunded += refunded;
                }
                buckets.into_values().collect()
            }
        };
        let mut rows: Vec<SalesRow> = rows;
        if !matches!(group_by, GroupBy::Day | GroupBy::Month) {
            rows.sort_by(|a, b| b.gross.cmp(&a.gross).then_with(|| a.key.cmp(&b.key)));
        }
        Ok(rows)
    }
}

/// Cột xuất của danh sách đơn.
const COLUMNS: [&str; 16] = [
    "id",
    "number",
    "org_id",
    "channel",
    "status",
    "payment_status",
    "fulfillment_status",
    "payment_method",
    "currency",
    "subtotal",
    "discount",
    "shipping_fee",
    "tax",
    "total",
    "refunded",
    "placed_at",
];

/// Một dòng xuất (phẳng, tiền là số nguyên đơn vị nhỏ nhất).
#[derive(Debug, Serialize)]
struct ExportRow {
    id: Uuid,
    number: String,
    org_id: Uuid,
    channel: String,
    status: String,
    payment_status: String,
    fulfillment_status: String,
    payment_method: String,
    currency: String,
    subtotal: i64,
    discount: i64,
    shipping_fee: i64,
    tax: i64,
    total: i64,
    refunded: i64,
    placed_at: Timestamp,
}

/// Bộ lọc của xuất đơn: như `GET /api/v1/orders`.
#[derive(Debug, Deserialize)]
struct ExportFilter {
    status: Option<String>,
    payment_status: Option<String>,
    org_id: Option<Uuid>,
    channel: Option<String>,
    number: Option<String>,
}

/// Xuất danh sách đơn theo bộ lọc của `GET /api/v1/orders` (P16, D-213): cùng phạm vi `orders.read`.
pub(crate) struct OrderExport(pub Arc<OrdersService>);

#[async_trait::async_trait]
impl ExportSource for OrderExport {
    fn info(&self) -> ExportInfo {
        ExportInfo::new("orders.list_orders", "orders", COLUMNS.to_vec())
    }

    fn permission(&self) -> Permission {
        READ
    }

    async fn page(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        query: &ExportQuery,
        cursor: Option<String>,
    ) -> Result<ExportPage, Error> {
        let mut query = query.with("limit", "200");
        if let Some(cursor) = cursor {
            query = query.with("cursor", cursor);
        }
        let filter: ExportFilter = query.parse()?;
        let paging: PageQuery = query.parse()?;
        let page = PageRequest::try_from(paging)?;
        let filter = crate::orders_read::OrderFilter {
            status: filter.status,
            payment_status: filter.payment_status,
            org_id: filter.org_id,
            channel: filter.channel,
            number: filter.number,
            ..crate::orders_read::OrderFilter::default()
        };
        let found = self.0.list_orders(tenant, principal, &filter, &page).await?;
        let rows: Vec<ExportRow> = found
            .data
            .into_iter()
            .map(|row| ExportRow {
                id: row.id,
                number: row.number,
                org_id: row.org_id,
                channel: row.channel,
                status: row.status,
                payment_status: row.payment_status,
                fulfillment_status: row.fulfillment_status,
                payment_method: row.payment_method,
                currency: row.currency,
                subtotal: row.subtotal_minor,
                discount: row.discount_minor,
                shipping_fee: row.shipping_fee_minor,
                tax: row.tax_minor,
                total: row.total_minor,
                refunded: row.refunded_minor,
                placed_at: row.placed_at,
            })
            .collect();
        ExportPage::from_json(&rows, &COLUMNS, found.pagination.next_cursor)
    }
}
