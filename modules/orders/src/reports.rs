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
use autoxide_kernel::pagination::{
    Order, Page, PageQuery, PageRequest, Pagination, page_of, text_key_cursor,
};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Func, Query};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
    QueryTrait as _, Select,
};
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
    /// `GET /reports/sales`: một trang.
    ///
    /// - `org` / `channel` / `product`: keyset ngay trong SQL trên tổng của nhóm (`HAVING`, như xếp hạng của
    ///   `analytics`, D-20261126-list-pages-cluster-3) — tổng trả nhiều nhất trước (`order=asc`, mặc định), cùng
    ///   tổng thì theo khoá; `order=desc` đảo lại. Database chỉ trả `limit + 1` nhóm.
    /// - `day` / `month`: gom trong Rust (hàm ngày tháng khác nhau giữa hai database) rồi `page_of`: số nhóm bị
    ///   chặn theo cấu trúc — khoảng tối đa 366 ngày nên tối đa 367 ngày / 13 tháng; khoá `YYYY-MM-DD` /
    ///   `YYYY-MM` so theo byte đúng thứ tự thời gian.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn sales_report(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        group_by: GroupBy,
        from: Timestamp,
        to: Timestamp,
        currency: Currency,
        page: &PageRequest,
    ) -> Result<Page<SalesRow>, Error> {
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
        match group_by {
            GroupBy::Org | GroupBy::Channel => {
                let column = if group_by == GroupBy::Org {
                    order::Column::OrgId
                } else {
                    order::Column::Channel
                };
                let groups = order::Entity::find()
                    .select_only()
                    .filter(counted)
                    .scoped(tenant)
                    .column_as(order::Column::Id.count(), "orders")
                    .column_as(sum(order::Column::TotalMinor), "gross")
                    .column_as(sum(order::Column::DiscountMinor), "discount")
                    .column_as(sum(order::Column::TaxMinor), "tax")
                    .column_as(sum(order::Column::RefundedMinor), "refunded")
                    .column(column)
                    .group_by(column);
                let cursor = cursor_name(group_by);
                let after = after_key(page, &cursor, group_by)?;
                let total = groups_total(groups.clone(), page, read).await?;
                let select = ranked(groups, sum(order::Column::TotalMinor), column, after, page);
                let rows: Vec<SalesRow> = if group_by == GroupBy::Org {
                    let found: Vec<GroupRow<Uuid>> = select.into_tuple().all(read).await?;
                    found
                        .into_iter()
                        .map(|row| group_row(row, |org: Uuid| org.to_string()))
                        .collect()
                } else {
                    let found: Vec<GroupRow<String>> = select.into_tuple().all(read).await?;
                    found.into_iter().map(|row| group_row(row, |channel| channel)).collect()
                };
                Ok(ranked_page(rows, &cursor, page, total))
            }
            GroupBy::Product => {
                let ids = Query::select()
                    .column(order::Column::Id)
                    .from(order::Entity)
                    .and_where(order::Column::TenantId.eq(tenant.id().as_uuid()))
                    .cond_where(counted)
                    .to_owned();
                let groups = line::Entity::find()
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
                    .group_by(line::Column::ProductId);
                let cursor = cursor_name(group_by);
                let after = after_key(page, &cursor, group_by)?;
                let total = groups_total(groups.clone(), page, read).await?;
                let select = ranked(
                    groups,
                    sum_line(line::Column::TotalMinor),
                    line::Column::ProductId,
                    after,
                    page,
                );
                let found: Vec<ProductRow> = select.into_tuple().all(read).await?;
                let rows = found
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
                    .collect();
                Ok(ranked_page(rows, &cursor, page, total))
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
                page_of(buckets.into_values().collect(), |row| row.key.clone(), page)
            }
        }
    }
}

/// Tên thứ tự trong con trỏ của báo cáo theo nhóm: mỗi cách gom một tên, con trỏ của cách gom khác bị từ chối.
fn cursor_name(group_by: GroupBy) -> String {
    let name = match group_by {
        GroupBy::Org => "org",
        GroupBy::Channel => "channel",
        GroupBy::Product => "product",
        GroupBy::Day => "day",
        GroupBy::Month => "month",
    };
    format!("sales:{name}")
}

/// Tổng và khoá của nhóm cuối trang trước (con trỏ `<tổng>\u{1f}<khoá>`); hỏng thì 422 ở `cursor`.
fn after_key(
    page: &PageRequest,
    cursor: &str,
    group_by: GroupBy,
) -> Result<Option<(i64, sea_orm::Value)>, Error> {
    let Some(key) = page.text_key_after(cursor)? else { return Ok(None) };
    let invalid = || {
        Error::validation(vec![FieldError::new(
            "cursor",
            "invalid",
            "must be the `next_cursor` value of a previous page of the same list and sort, unchanged",
        )])
    };
    let (gross, value) = key.split_once('\u{1f}').ok_or_else(invalid)?;
    let gross = gross.parse::<i64>().map_err(|_| invalid())?;
    let value = match group_by {
        GroupBy::Channel => sea_orm::Value::from(value.to_owned()),
        _ => sea_orm::Value::from(Uuid::parse_str(value).map_err(|_| invalid())?),
    };
    Ok(Some((gross, value)))
}

/// Thêm keyset (`HAVING`), thứ tự và `LIMIT limit + 1` vào câu gom: tổng trả nhiều trước (`asc`), cùng tổng thì
/// khoá tăng dần; `desc` đảo cả hai.
fn ranked<E: EntityTrait>(
    groups: Select<E>,
    gross: Expr,
    key: E::Column,
    after: Option<(i64, sea_orm::Value)>,
    page: &PageRequest,
) -> Select<E> {
    let most_first = page.order() == Order::Asc;
    let mut select = groups;
    if let Some((total, value)) = after {
        let (beyond, tie) = if most_first {
            (gross.clone().lt(total), key.gt(value))
        } else {
            (gross.clone().gt(total), key.lt(value))
        };
        select = select.having(
            Condition::any()
                .add(beyond)
                .add(Condition::all().add(gross.clone().eq(total)).add(tie)),
        );
    }
    let (gross_order, key_order) = if most_first {
        (sea_orm::Order::Desc, sea_orm::Order::Asc)
    } else {
        (sea_orm::Order::Asc, sea_orm::Order::Desc)
    };
    select.order_by(gross, gross_order).order_by(key, key_order).limit(u64::from(page.limit()) + 1)
}

/// Số nhóm (với `count=true`, trang đầu): đếm trên subquery `GROUP BY`, chạy được trên cả hai database.
async fn groups_total<E: EntityTrait>(
    groups: Select<E>,
    page: &PageRequest,
    read: &impl ConnectionTrait,
) -> Result<Option<u64>, Error> {
    if !page.wants_total() {
        return Ok(None);
    }
    let count = Query::select()
        .expr(Expr::cust("COUNT(*)"))
        .from_subquery(groups.into_query(), Alias::new("groups"))
        .to_owned();
    let total: i64 = match read.query_one(&count).await? {
        Some(row) => row.try_get_by_index(0)?,
        None => 0,
    };
    Ok(Some(u64::try_from(total).unwrap_or_default()))
}

/// Cắt dòng thừa (`limit + 1`) và dựng con trỏ từ nhóm cuối.
fn ranked_page(
    mut rows: Vec<SalesRow>,
    cursor: &str,
    page: &PageRequest,
    total: Option<u64>,
) -> Page<SalesRow> {
    let limit = page.limit() as usize;
    let has_more = rows.len() > limit;
    rows.truncate(limit);
    let next = has_more
        .then(|| rows.last())
        .flatten()
        .map(|row| text_key_cursor(cursor, &format!("{}\u{1f}{}", row.gross, row.key)));
    Page { data: rows, pagination: Pagination::new(page.limit(), next).with_total(total) }
}

/// Dòng của câu gom theo nút / kênh.
fn group_row<K>(row: GroupRow<K>, key: impl Fn(K) -> String) -> SalesRow {
    let (orders, gross, discount, tax, refunded, value) = row;
    SalesRow {
        key: key(value),
        orders,
        quantity: None,
        gross: gross.unwrap_or(0),
        discount: discount.unwrap_or(0),
        tax: tax.unwrap_or(0),
        refunded: refunded.unwrap_or(0),
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
