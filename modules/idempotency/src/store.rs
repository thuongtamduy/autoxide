//! `IdempotencyStore` trên bảng `idempotency_record`.
//!
//! Hai request cùng key đến cùng lúc: index unique `(tenant_id, scope, key)` để đúng một bên chèn
//! được dòng và chạy (`ON CONFLICT DO NOTHING`: bên kia chèn được 0 dòng); bên kia đọc dòng đó và nhận
//! `InProgress` (hoặc kết quả, nếu bên đầu đã xong). Key thuộc khách của request (bước 33): mọi truy vấn qua
//! `tenant.db()`, handle tự lọc theo khách.

use std::time::Duration;

use autoxide_kernel::db::{Queries as _, TenantKey};
use autoxide_kernel::error::Error;
use autoxide_kernel::idempotency::{Begin, IdempotencyStore, StoredResponse};
use autoxide_kernel::job::{JobContext, JobHandler, Schedule};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::time::{SignedDuration, Timestamp};
use uuid::Uuid;

use crate::entity::IdempotencyRecord as Row;

/// Giữ một key chừng này: gửi lại trong khoảng đó nhận lại kết quả cũ.
const KEEP: SignedDuration = SignedDuration::from_hours(24);
/// Loại job dọn key hết hạn (cũng là tên lịch).
pub(crate) const CLEANUP: &str = "idempotency.cleanup";

/// Dọn mỗi giờ.
pub(crate) fn cleanup_schedule() -> Schedule {
    Schedule::new(CLEANUP, CLEANUP, Duration::from_secs(3600))
}

// Store không giữ database: database là của khách, đi theo `TenantContext` của từng lời gọi.
pub(crate) struct Store;

impl Store {
    /// Chèn dòng mới cho lần đầu. `None` nếu dòng của key đã có.
    async fn insert(
        &self,
        tenant: &TenantContext,
        scope: &str,
        key: &str,
        fingerprint: &str,
        locked_until: Timestamp,
    ) -> Result<Option<Uuid>, Error> {
        let now = now();
        let id = Uuid::now_v7();
        let row = Row {
            id,
            tenant_id: TenantKey::new(tenant.id()),
            scope: scope.to_owned(),
            key: key.to_owned(),
            fingerprint: fingerprint.to_owned(),
            status: None,
            headers: None,
            body: None,
            locked_until,
            expires_at: now + KEEP,
            created_at: now,
        };
        // Mỗi câu ghi một transaction ngắn, như trước (không giữ khoá trong lúc handler chạy).
        let transaction = tenant.db().begin_write().await?;
        let inserted = transaction
            .insert(row)
            .on_conflict_do_nothing([Row::TENANT_ID.key(), Row::SCOPE.key(), Row::KEY.key()])
            .exec()
            .await?;
        transaction.commit().await?;
        Ok((inserted == 1).then_some(id))
    }
}

#[autoxide_kernel::async_trait]
impl IdempotencyStore for Store {
    async fn begin(
        &self,
        tenant: &TenantContext,
        scope: &str,
        key: &str,
        fingerprint: &str,
        locked_until: Timestamp,
    ) -> Result<Begin, Error> {
        // Vòng lặp chạy lại nhiều nhất một lần: khi dòng cũ đã hết hạn và vừa bị xoá.
        for _ in 0..2 {
            if let Some(id) = self.insert(tenant, scope, key, fingerprint, locked_until).await? {
                return Ok(Begin::Started { id });
            }
            let transaction = tenant.db().begin_write().await?;
            // Đọc trong transaction ghi (kết nối ghi): thấy ngay dòng bên kia vừa chèn.
            let Some(row) = transaction
                .find::<Row>()
                .filter(Row::SCOPE.eq(scope.to_owned()))
                .filter(Row::KEY.eq(key.to_owned()))
                .one()
                .await?
            else {
                transaction.rollback().await?;
                continue; // vừa bị dọn giữa hai câu lệnh
            };
            let now = now();
            if row.expires_at <= now {
                transaction.delete::<Row>().filter(Row::ID.eq(row.id)).exec().await?;
                transaction.commit().await?;
                continue;
            }
            // Các nhánh chỉ đọc trả lời ngay và `rollback` rõ ràng: nhả khoá ghi của SQLite liền, không đợi kết nối về pool.
            if row.fingerprint != fingerprint {
                transaction.rollback().await?;
                return Ok(Begin::Mismatch);
            }
            if let Some(status) = row.status {
                let headers = row
                    .headers
                    .as_deref()
                    .map(serde_json::from_str::<Vec<(String, String)>>)
                    .transpose()
                    .map_err(Error::internal)?
                    .unwrap_or_default();
                let status = u16::try_from(status).map_err(Error::internal)?;
                let body = row.body.unwrap_or_default();
                transaction.rollback().await?;
                return Ok(Begin::Replay(StoredResponse::new(status, headers, body)));
            }
            if row.locked_until > now {
                transaction.rollback().await?;
                return Ok(Begin::InProgress);
            }
            // Lần đầu đã quá hạn mà chưa xong (tiến trình chết): nhận lại quyền chạy, có
            // điều kiện để chỉ một request nhận được.
            let taken = transaction
                .update::<Row>()
                .set(Row::LOCKED_UNTIL, locked_until)
                .filter(Row::ID.eq(row.id))
                .filter(Row::STATUS.is_null())
                .filter(Row::LOCKED_UNTIL.lte(now))
                .exec()
                .await?;
            transaction.commit().await?;
            return Ok(if taken == 1 { Begin::Started { id: row.id } } else { Begin::InProgress });
        }
        Ok(Begin::InProgress)
    }

    async fn complete(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        response: StoredResponse,
    ) -> Result<(), Error> {
        let headers = serde_json::to_string(&response.headers).map_err(Error::internal)?;
        let transaction = tenant.db().begin_write().await?;
        transaction
            .update::<Row>()
            .set(Row::STATUS, Some(i32::from(response.status)))
            .set(Row::HEADERS, Some(headers))
            .set(Row::BODY, Some(response.body))
            .filter(Row::ID.eq(id))
            .exec()
            .await?;
        transaction.commit().await
    }

    async fn abandon(&self, tenant: &TenantContext, id: Uuid) -> Result<(), Error> {
        let transaction = tenant.db().begin_write().await?;
        transaction.delete::<Row>().filter(Row::ID.eq(id)).exec().await?;
        transaction.commit().await
    }
}

/// Job dọn key đã hết hạn: chạy theo lịch trên từng database, dọn key của mọi khách trên đó
/// (index `_sweep_idx`).
pub(crate) struct Cleanup;

#[autoxide_kernel::async_trait]
impl JobHandler for Cleanup {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let transaction =
            job.all_tenants("expired idempotency keys of every tenant").begin_write().await?;
        let removed = transaction.delete::<Row>().filter(Row::EXPIRES_AT.lt(now())).exec().await?;
        transaction.commit().await?;
        tracing::info!(removed, "expired idempotency keys removed");
        Ok(())
    }
}

fn now() -> Timestamp {
    Timestamp::now()
}

// Test ở file riêng (`*_tests.rs`): luật `raw-database` miễn file test, và `TestApp::database()` cần cho test.
#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
