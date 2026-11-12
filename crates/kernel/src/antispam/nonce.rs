//! Pha hai của chống spam: nonce của token form chèn vào `<module>_antispam_nonces` TRONG
//! transaction của module. Đụng unique nghĩa là bài này đã gửi rồi (trình duyệt bấm hai lần,
//! mạng di động gửi lại): module trả lại kết quả cũ thay vì tạo bản ghi thứ hai — idempotency
//! cho người lạ, vì `Idempotency-Key` chỉ áp cho request đã xác thực.

use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, OnConflict, Query};
use sea_orm_migration::prelude::{
    DbErr, Index, MigrationName, MigrationTrait, SchemaManager, Table,
};
use sea_orm_migration::schema;

use crate::error::Error;
use crate::secret::sha256;

/// Kết quả chèn nonce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// Lần đầu: bài gửi mới.
    New,
    /// Đã gửi rồi; kèm `result` lần đầu đã lưu (vd ID bản ghi) để trả lại.
    Replay { result: Option<String> },
}

pub(crate) fn table_name(module: &str) -> String {
    format!("{module}_antispam_nonces")
}

/// Chèn `nonce` (đã băm) kèm `result` của lần gửi này; giữ tới `expires_at`.
pub(crate) async fn claim<C: ConnectionTrait>(
    connection: &C,
    module: &str,
    nonce: &str,
    result: Option<&str>,
    expires_at: DateTime<Utc>,
) -> Result<Claim, Error> {
    let table = Alias::new(table_name(module));
    let hash = sha256(nonce);
    let insert = Query::insert()
        .into_table(table.clone())
        .columns([Alias::new("nonce_hash"), Alias::new("result"), Alias::new("expires_at")])
        .values_panic([
            hash.as_str().into(),
            result.map(str::to_owned).into(),
            expires_at.trunc_subsecs(6).into(),
        ])
        .on_conflict(OnConflict::column(Alias::new("nonce_hash")).do_nothing().to_owned())
        .to_owned();
    if connection.execute(&insert).await?.rows_affected() == 1 {
        return Ok(Claim::New);
    }
    let select = Query::select()
        .column(Alias::new("result"))
        .from(table)
        .and_where(Expr::col(Alias::new("nonce_hash")).eq(hash.as_str()))
        .to_owned();
    let result = match connection.query_one(&select).await? {
        Some(row) => row.try_get::<Option<String>>("", "result")?,
        None => None,
    };
    Ok(Claim::Replay { result })
}

/// Xoá nonce đã hết hạn trước `before`.
pub(crate) async fn purge<C: ConnectionTrait>(
    connection: &C,
    module: &str,
    before: DateTime<Utc>,
) -> Result<u64, Error> {
    let delete = Query::delete()
        .from_table(Alias::new(table_name(module)))
        .and_where(Expr::col(Alias::new("expires_at")).lt(before))
        .to_owned();
    Ok(connection.execute(&delete).await?.rows_affected())
}

/// Migration tạo bảng `<module>_antispam_nonces`. Module nhận bài gửi công khai qua
/// [`super::SpamGuard`] thêm nó vào danh sách migration của mình.
pub fn antispam_migration(name: &'static str, module: &'static str) -> Box<dyn MigrationTrait> {
    Box::new(NoncesMigration { name, module })
}

struct NoncesMigration {
    name: &'static str,
    module: &'static str,
}

impl MigrationName for NoncesMigration {
    fn name(&self) -> &str {
        self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for NoncesMigration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table = table_name(self.module);
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(&table))
                    .col(schema::string_len("nonce_hash", 64).primary_key())
                    .col(schema::string_len_null("result", 255))
                    .col(schema::timestamp_with_time_zone("expires_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name(format!("{table}_expires_idx"))
                    .table(Alias::new(&table))
                    .col("expires_at")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Alias::new(table_name(self.module))).to_owned())
            .await
    }
}
