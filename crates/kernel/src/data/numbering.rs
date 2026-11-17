//! Đánh số tuần tự không nhảy số (bước 36): số chứng từ, số hoá đơn, số phiếu… phải liên tục
//! 1, 2, 3… theo từng dãy, không trùng và không bỏ số — kể cả khi nhiều request cấp số cùng lúc.
//!
//! Một dãy là một dòng của bảng `kernel_sequences`, theo khách (`tenant_id`) và tên dãy (`scope`, do
//! module tự đặt, vd `invoices:<org>:<loại>:<ký hiệu>:<năm>`). Bảng là bảng cấp khách hàng của module
//! nội bộ `kernel`, nên có ở mọi database dữ liệu: số được cấp **trong cùng transaction** với bản ghi
//! mang số đó.
//!
//! Vì sao không nhảy số: [`next`] là MỘT câu
//! `INSERT … ON CONFLICT (tenant_id, scope) DO UPDATE SET last_value = last_value + 1 RETURNING last_value`
//! chạy trong transaction ghi của bên gọi. Postgres khoá dòng của dãy tới hết transaction — bên cấp số
//! thứ hai chờ bên đầu xong rồi mới cộng tiếp; SQLite vốn chỉ một bên ghi (`BEGIN IMMEDIATE`).
//! Transaction rollback thì phép cộng cũng rollback: số không bị tiêu. (Khác `SEQUENCE` của Postgres —
//! nhanh hơn nhưng nhảy số khi rollback, và SQLite không có.)
//!
//! Giá phải trả: các bên cấp số trong cùng một dãy nối đuôi nhau tới khi commit — giữ transaction
//! ngắn, cấp số ở cuối, ngay trước khi ghi bản ghi.

use chrono::Utc;
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, OnConflict, Query};

use crate::error::Error;
use crate::tenancy::TenantId;

/// Tên bảng (cấp khách hàng, module `kernel`).
pub(crate) const TABLE: &str = "kernel_sequences";

/// Độ dài tối đa của tên dãy.
pub const MAX_SCOPE_LEN: usize = 200;

/// Cấp số kế tiếp của dãy `scope` của khách `tenant`: dãy mới bắt đầu từ 1. `transaction` phải là
/// transaction GHI (`Database::begin_write`) trên database của khách, cùng transaction với bản ghi mang
/// số — ngoài transaction thì số vẫn đúng thứ tự nhưng bản ghi hỏng sau đó sẽ để lại một số bị bỏ.
pub async fn next<C: ConnectionTrait>(
    transaction: &C,
    tenant: TenantId,
    scope: &str,
) -> Result<i64, Error> {
    check_scope(scope)?;
    let table = Alias::new(TABLE);
    let now = Utc::now();
    let upsert = Query::insert()
        .into_table(table.clone())
        .columns(["tenant_id", "scope", "last_value", "updated_at"].map(Alias::new))
        .values_panic([tenant.as_uuid().into(), scope.into(), 1i64.into(), now.into()])
        .on_conflict(
            OnConflict::columns([Alias::new("tenant_id"), Alias::new("scope")])
                // Ghi kèm tên bảng: trong `DO UPDATE` tên cột trần lẫn với dòng `excluded`.
                .value(
                    Alias::new("last_value"),
                    Expr::col((table.clone(), Alias::new("last_value"))).add(1i64),
                )
                .value(Alias::new("updated_at"), now)
                .to_owned(),
        )
        .returning_col(Alias::new("last_value"))
        .to_owned();
    let row = transaction
        .query_one(&upsert)
        .await?
        .ok_or_else(|| Error::internal(format!("numbering `{scope}` returned no row")))?;
    Ok(row.try_get::<i64>("", "last_value")?)
}

/// Số đã cấp cuối cùng của dãy (0 khi chưa cấp số nào). Chỉ để xem — muốn cấp số thì gọi [`next`],
/// không đọc rồi cộng một.
pub async fn current<C: ConnectionTrait>(
    connection: &C,
    tenant: TenantId,
    scope: &str,
) -> Result<i64, Error> {
    check_scope(scope)?;
    let select = Query::select()
        .column(Alias::new("last_value"))
        .from(Alias::new(TABLE))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.as_uuid()))
        .and_where(Expr::col(Alias::new("scope")).eq(scope))
        .to_owned();
    match connection.query_one(&select).await? {
        Some(row) => Ok(row.try_get::<i64>("", "last_value")?),
        None => Ok(0),
    }
}

fn check_scope(scope: &str) -> Result<(), Error> {
    if scope.is_empty() || scope.len() > MAX_SCOPE_LEN {
        // Lỗi của lập trình viên module (tên dãy do code đặt), không phải của người gọi API.
        return Err(Error::internal(format!(
            "a numbering scope must be 1 to {MAX_SCOPE_LEN} bytes, got {}",
            scope.len()
        )));
    }
    Ok(())
}

/// Migration tạo bảng `kernel_sequences`.
pub(crate) struct CreateSequences;

mod migration {
    use crate::migration::prelude::*;

    impl MigrationName for super::CreateSequences {
        fn name(&self) -> &str {
            "m0016_create_sequences"
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for super::CreateSequences {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .create_table(
                    Table::create()
                        .table(super::TABLE)
                        .col(schema::uuid("tenant_id"))
                        .col(schema::string_len("scope", 200))
                        .col(schema::big_integer("last_value"))
                        .col(schema::timestamp_with_time_zone("updated_at"))
                        .primary_key(Index::create().col("tenant_id").col("scope"))
                        .to_owned(),
                )
                .await
        }

        async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager.drop_table(Table::drop().table(super::TABLE).to_owned()).await
        }
    }
}
