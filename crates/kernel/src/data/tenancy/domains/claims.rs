//! Migration of P-4 (2026-11 security review, D-20261125-domain-claims): several tenants may wait on the same
//! domain. The unique index on `domain` becomes two: one row per tenant and domain, and at most one
//! `verified` row per domain (a partial unique index, `WHERE status = 'verified'`, on both SQLite and
//! PostgreSQL).

use sea_orm::ConnectionTrait;

use crate::migration::prelude::*;

const TABLE: &str = "kernel_tenant_domains";
const OLD_UNIQUE: &str = "kernel_tenant_domains_domain_unique";
const PER_TENANT: &str = "kernel_tenant_domains_domain_tenant_unique";
const VERIFIED: &str = "kernel_tenant_domains_verified_unique";

pub(crate) struct ShareClaims;

impl MigrationName for ShareClaims {
    fn name(&self) -> &str {
        "m0040_domain_shared_claims"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for ShareClaims {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(Index::drop().name(OLD_UNIQUE).table(Alias::new(TABLE)).to_owned())
            .await?;
        manager
            .create_index(
                Index::create()
                    .name(PER_TENANT)
                    .table(Alias::new(TABLE))
                    .col(Alias::new("domain"))
                    .col(Alias::new("tenant_id"))
                    .unique()
                    .to_owned(),
            )
            .await?;
        // Chỉ một khách giữ tên miền ở trạng thái `verified`: index một phần (partial index), cú pháp
        // `CREATE UNIQUE INDEX … WHERE status = 'verified'` chạy được trên cả SQLite và Postgres.
        manager
            .create_index(
                Index::create()
                    .name(VERIFIED)
                    .table(Alias::new(TABLE))
                    .col(Alias::new("domain"))
                    .unique()
                    .and_where(Expr::col(Alias::new("status")).eq("verified"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Quay lại "một dòng mỗi tên miền" chỉ khi không tên miền nào có nhiều dòng: không xoá lượt chờ của khách.
        let shared = Query::select()
            .column(Alias::new("domain"))
            .from(Alias::new(TABLE))
            .group_by_col(Alias::new("domain"))
            .and_having(Expr::col(Alias::new("domain")).count().gt(1))
            .limit(1)
            .to_owned();
        let connection = manager.get_connection();
        let statement = connection.get_database_backend().build(&shared);
        if connection.query_one_raw(statement).await?.is_some() {
            return Err(DbErr::Migration(format!(
                "{TABLE}: a domain has rows of several tenants (claims in any status, revoked ones \
                 included); going back allows one row per domain, so delete the extra rows first - this \
                 migration does not delete tenants' claims itself"
            )));
        }
        manager
            .drop_index(Index::drop().name(VERIFIED).table(Alias::new(TABLE)).to_owned())
            .await?;
        manager
            .drop_index(Index::drop().name(PER_TENANT).table(Alias::new(TABLE)).to_owned())
            .await?;
        manager
            .create_index(
                Index::create()
                    .name(OLD_UNIQUE)
                    .table(Alias::new(TABLE))
                    .col(Alias::new("domain"))
                    .unique()
                    .to_owned(),
            )
            .await
    }
}
