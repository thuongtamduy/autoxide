//! Bộ phát event: đọc event chưa giao trong bảng `<module>_events` của các module và
//! giao cho bên nhận.
//!
//! Nhiều tiến trình chạy bộ phát cùng lúc vẫn đúng: mỗi event được "nhận việc"
//! (`claimed_until`) bằng một câu `UPDATE` có điều kiện — chỉ một tiến trình nhận được,
//! các tiến trình khác bỏ qua. Câu lệnh chạy giống nhau trên SQLite và Postgres. Tiến
//! trình nhận việc rồi chết giữa chừng thì hết hạn nhận việc, tiến trình khác giao lại.
//!
//! Nhiều database (bước 33): mỗi lượt duyệt mọi database được giao (`DatabaseRegistry::databases`,
//! lọc theo `AUTOXIDE_WORKER_PLACEMENTS`); event mang khách (`tenant_id`) và bên nhận nhận ngữ cảnh
//! khách đó trong [`Delivered::tenant`].

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, SubsecRound, Utc};
use futures_util::FutureExt as _;
use sea_orm::sea_query::{Alias, Condition, Expr, ExprTrait as _, Order, Query};
use sea_orm::{ConnectionTrait, QueryResult};
use tokio::sync::Notify;
use uuid::Uuid;

use super::{Actor, Delivered, Subscription, table_name};
use crate::database::Database;
use crate::error::Error;
use crate::tenancy::{DatabaseRegistry, PlacementId, TenantId, TenantStatus};

/// Số lần giao tối đa của một event; quá thì đánh dấu hỏng (`failed_at`) — trừ event xoá
/// tài khoản ([`USER_DELETED`](crate::privacy::USER_DELETED)), thử lại mãi.
pub const MAX_ATTEMPTS: i32 = 10;
/// Số event mỗi bảng xử lý trong một lượt.
const BATCH: u64 = 100;
/// Một tiến trình giữ việc giao một event tối đa chừng này.
const CLAIM_SECONDS: i64 = 60;
/// Một lần giao (mọi bên nhận của một event) tối đa chừng này — ngắn hơn thời gian giữ, nên
/// không tiến trình nào khác nhận lại event đang giao (D-158). Bên nhận cần lâu hơn thì làm
/// từng phần (lần sau làm tiếp) hoặc chuyển việc vào job.
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(50);
/// Không có event mới thì hỏi lại sau chừng này.
const IDLE_POLL: Duration = Duration::from_secs(1);
/// Độ dài tối đa của lỗi lưu lại.
const MAX_ERROR_LENGTH: usize = 2000;
/// Giữ event đã giao chừng này rồi xoá (audit log đã có bản của nó).
const KEEP_DELIVERED: chrono::Duration = chrono::Duration::days(7);
/// Dọn event đã giao mỗi chừng này.
const CLEANUP_EVERY: Duration = Duration::from_secs(3600);

/// Bộ phát event của chương trình.
pub struct Dispatcher {
    databases: DatabaseRegistry,
    /// Các module có bảng event.
    modules: Vec<&'static str>,
    /// (module nhận, đăng ký).
    subscriptions: Vec<(&'static str, Subscription)>,
    wake: Arc<Notify>,
    last_cleanup: std::sync::Mutex<Option<std::time::Instant>>,
    /// Quyền dùng theo khách (bước 35): bên nhận thuộc module gói của khách không có bị bỏ qua.
    entitlements: crate::entitlement::EntitlementsHandle,
}

impl Dispatcher {
    pub(crate) fn new(
        databases: DatabaseRegistry,
        modules: Vec<&'static str>,
        subscriptions: Vec<(&'static str, Subscription)>,
        wake: Arc<Notify>,
    ) -> Self {
        Dispatcher {
            databases,
            modules,
            subscriptions,
            wake,
            last_cleanup: std::sync::Mutex::new(None),
            entitlements: crate::entitlement::EntitlementsHandle::default(),
        }
    }

    /// Bỏ qua bên nhận thuộc module mà gói của khách của event không có (bước 35).
    pub(crate) fn with_entitlements(
        mut self,
        entitlements: crate::entitlement::EntitlementsHandle,
    ) -> Self {
        self.entitlements = entitlements;
        self
    }

    /// Bên nhận thuộc module `subscriber` có được nhận event của khách `tenant` không: module
    /// `always_enabled`, hay gói của khách có module và khách không chỉ đọc vì thuê bao.
    async fn admits(
        &self,
        subscriber: &str,
        tenant: &crate::tenancy::TenantContext,
    ) -> Result<bool, Error> {
        if !self.entitlements.is_enforced()
            || self.entitlements.catalog().is_always_enabled(subscriber)
        {
            return Ok(true);
        }
        let snapshot = self.entitlements.snapshot(tenant.id()).await?;
        Ok(snapshot.has_module(subscriber)
            && snapshot.access == crate::entitlement::TenantAccess::Full)
    }

    /// Giao một lượt mọi event đang chờ; trả về số event đã xử lý (giao xong hoặc
    /// hẹn lại).
    pub async fn run_once(&self) -> Result<usize, Error> {
        let databases = self.databases.databases().await?;
        self.cleanup(&databases).await?;
        let mut handled = 0;
        for (placement, _, database) in &databases {
            for module in &self.modules {
                handled += self.run_table(*placement, database, module).await?;
            }
        }
        Ok(handled)
    }

    /// Giao event tới khi `shutdown` hoàn thành. Lỗi database thì ghi log rồi thử lại
    /// ở lượt sau — bộ phát không được chết vì một lần database chập chờn.
    pub async fn run(self, shutdown: impl Future<Output = ()>) {
        tokio::pin!(shutdown);
        loop {
            let handled = match self.run_once().await {
                Ok(handled) => handled,
                Err(error) => {
                    tracing::error!(error = %error, "event dispatcher failed; retrying");
                    0
                }
            };
            if handled > 0 {
                continue;
            }
            tokio::select! {
                () = &mut shutdown => return,
                () = self.wake.notified() => {}
                () = tokio::time::sleep(IDLE_POLL) => {}
            }
        }
    }

    /// Xoá event đã giao hơn [`KEEP_DELIVERED`], mỗi giờ một lần. Event hỏng hẳn được
    /// giữ lại để người vận hành xem.
    async fn cleanup(&self, databases: &[(PlacementId, String, Database)]) -> Result<(), Error> {
        {
            let mut last = self.last_cleanup.lock().map_err(|_| Error::internal("cleanup lock"))?;
            if last.is_some_and(|at| at.elapsed() < CLEANUP_EVERY) {
                return Ok(());
            }
            *last = Some(std::time::Instant::now());
        }
        let before = now() - KEEP_DELIVERED;
        for (_, _, database) in databases {
            for module in &self.modules {
                let mut delete = Query::delete();
                delete
                    .from_table(Alias::new(table_name(module)))
                    .and_where(Expr::col(Alias::new("delivered_at")).lt(before));
                database.write().execute(&delete).await?;
            }
        }
        Ok(())
    }

    /// Giao event chờ trong bảng event của `module` ở `database` (placement `placement`).
    ///
    /// Khách đang dời (`moving`): hẹn event lại sau, không tính lần giao — event đi theo dữ liệu
    /// sang đích. Khách đã ở placement khác: dòng này là bản sao còn sót ở nguồn, không giao.
    async fn run_table(
        &self,
        placement: PlacementId,
        database: &Database,
        module: &'static str,
    ) -> Result<usize, Error> {
        let table = Alias::new(table_name(module));
        let mut select = Query::select();
        select
            .columns([
                "tenant_id",
                "id",
                "name",
                "actor_user_id",
                "actor_api_key_id",
                "actor_kind",
                "actor_access",
                "actor_access_id",
                "subject",
                "payload",
                "occurred_at",
                "attempts",
            ])
            .from(table.clone())
            .cond_where(pending(now()))
            .order_by(Alias::new("id"), Order::Asc)
            .limit(BATCH);
        let rows = database.write().query_all(&select).await?;
        let mut handled = 0;
        for row in rows {
            let id: Uuid = row.try_get("", "id")?;
            let attempts: i32 = row.try_get("", "attempts")?;
            let event = match self.read_event(module, &row).await {
                Ok(event) => event,
                Err(error) => {
                    // Dòng hỏng (sửa tay, dữ liệu lỗi, khách không còn): đánh dấu hỏng hẳn rồi đi
                    // tiếp — không để nó chặn mọi event sau nó mỗi giây (D-158).
                    mark_unreadable(database, &table, id, &error).await?;
                    continue;
                }
            };
            if event.tenant.status() == TenantStatus::Moving {
                postpone(database, &table, event.id).await?;
                continue;
            }
            if event.tenant.placement() != placement {
                let error = Error::internal(format!(
                    "the tenant's data moved to placement {}; this event is a left-over copy at \
                     the old placement and is not delivered",
                    event.tenant.placement()
                ));
                mark_unreadable(database, &table, id, &error).await?;
                continue;
            }
            // Giờ mới cho mỗi lần nhận: giao các event trước có thể đã mất thời gian.
            if !claim(database, &table, event.id, now()).await? {
                continue; // tiến trình khác vừa nhận việc giao event này
            }
            handled += 1;
            match self.deliver(&event).await {
                Ok(()) => mark_delivered(database, &table, event.id).await?,
                Err(error) => mark_failed(database, &table, &event, attempts + 1, &error).await?,
            }
        }
        Ok(handled)
    }

    /// Giao cho mọi bên nhận khớp tên event, lần lượt; bên đầu tiên lỗi thì dừng. Bên nhận
    /// panic hay chạy quá [`DELIVERY_TIMEOUT`] tính là một lần giao lỗi (thử lại theo backoff),
    /// không làm chết dispatcher của cả tiến trình (D-158).
    async fn deliver(&self, event: &Delivered) -> Result<(), Error> {
        let all = async {
            for (subscriber, subscription) in &self.subscriptions {
                if !subscription.matches(&event.name) {
                    continue;
                }
                // Gói của khách không có module nhận: bỏ qua (coi như đã giao), không thử lại.
                if !self.admits(subscriber, &event.tenant).await? {
                    metrics::counter!("entitlement_denials_total", "kind" => "event").increment(1);
                    continue;
                }
                let handled = AssertUnwindSafe(subscription.handler.handle(event)).catch_unwind();
                match handled.await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        return Err(Error::internal(format!(
                            "module `{subscriber}`: {}",
                            describe(&error)
                        )));
                    }
                    Err(_) => {
                        return Err(Error::internal(format!(
                            "module `{subscriber}`: the event handler panicked"
                        )));
                    }
                }
            }
            Ok(())
        };
        // Span của lượt giao mang khách của event (bước 39 B7, D-190).
        let span = tracing::info_span!(
            "event_delivery",
            event_id = %event.id,
            name = %event.name,
            tenant_id = %event.tenant.id(),
        );
        let all = tracing::Instrument::instrument(all, span);
        match tokio::time::timeout(DELIVERY_TIMEOUT, all).await {
            Ok(outcome) => outcome,
            Err(_) => Err(Error::internal(format!(
                "delivery took longer than {} seconds",
                DELIVERY_TIMEOUT.as_secs()
            ))),
        }
    }

    /// Đọc một dòng event, kèm ngữ cảnh khách của nó.
    async fn read_event(
        &self,
        module: &'static str,
        row: &QueryResult,
    ) -> Result<Delivered, Error> {
        let tenant = TenantId::from_uuid(row.try_get("", "tenant_id")?);
        let tenant = self.databases.tenant(tenant).await?;
        let payload: String = row.try_get("", "payload")?;
        let actor_kind: Option<String> = row.try_get("", "actor_kind")?;
        Ok(Delivered {
            id: row.try_get("", "id")?,
            tenant,
            module,
            name: row.try_get("", "name")?,
            actor: Actor {
                user_id: row.try_get("", "actor_user_id")?,
                api_key_id: row.try_get("", "actor_api_key_id")?,
                public: actor_kind.as_deref() == Some("public"),
                kind: actor_kind.as_deref().and_then(crate::access::ActorKind::parse).filter(
                    |kind| {
                        !matches!(
                            kind,
                            crate::access::ActorKind::Human | crate::access::ActorKind::System
                        )
                    },
                ),
                access: access_of(
                    row.try_get::<Option<String>>("", "actor_access")?.as_deref(),
                    row.try_get("", "actor_access_id")?,
                ),
            },
            subject: row.try_get("", "subject")?,
            payload: serde_json::from_str(&payload).map_err(Error::internal)?,
            occurred_at: row.try_get("", "occurred_at")?,
        })
    }
}

/// Đường vào đọc từ hai cột của outbox; dòng cũ (không có giá trị) là trực tiếp.
fn access_of(kind: Option<&str>, id: Option<Uuid>) -> crate::access::AccessPath {
    match (kind, id) {
        (Some("delegation"), Some(id)) => crate::access::AccessPath::Delegation(id),
        (Some("support"), Some(id)) => crate::access::AccessPath::Support(id),
        (Some("automation"), Some(id)) => crate::access::AccessPath::Automation(id),
        _ => crate::access::AccessPath::Direct,
    }
}

/// Nhận việc giao event `id`. `false` nếu tiến trình khác đã nhận.
async fn claim(
    database: &Database,
    table: &Alias,
    id: Uuid,
    now: DateTime<Utc>,
) -> Result<bool, Error> {
    let mut update = Query::update();
    update
        .table(table.clone())
        .value(Alias::new("claimed_until"), now + chrono::Duration::seconds(CLAIM_SECONDS))
        .cond_where(Condition::all().add(Expr::col(Alias::new("id")).eq(id)).add(pending(now)));
    Ok(database.write().execute(&update).await?.rows_affected() == 1)
}

/// Khách của event `id` đang dời: thử lại sau, không tính lần giao.
async fn postpone(database: &Database, table: &Alias, id: Uuid) -> Result<(), Error> {
    let now = now();
    let mut update = Query::update();
    update
        .table(table.clone())
        .value(Alias::new("next_attempt_at"), now + chrono::Duration::seconds(30))
        .cond_where(Condition::all().add(Expr::col(Alias::new("id")).eq(id)).add(pending(now)));
    database.write().execute(&update).await?;
    Ok(())
}

/// Dòng event `id` không đọc được: hỏng hẳn, kèm lý do.
async fn mark_unreadable(
    database: &Database,
    table: &Alias,
    id: Uuid,
    error: &Error,
) -> Result<(), Error> {
    let message: String = describe(error).chars().take(MAX_ERROR_LENGTH).collect();
    tracing::error!(event_id = %id, error = %message, "an event row cannot be read; it will not be delivered");
    let mut update = Query::update();
    update
        .table(table.clone())
        .value(Alias::new("failed_at"), now())
        .value(Alias::new("last_error"), message)
        .and_where(Expr::col(Alias::new("id")).eq(id));
    database.write().execute(&update).await?;
    metrics::counter!("events_failed_total").increment(1);
    Ok(())
}

async fn mark_delivered(database: &Database, table: &Alias, id: Uuid) -> Result<(), Error> {
    let mut update = Query::update();
    update
        .table(table.clone())
        .value(Alias::new("delivered_at"), now())
        .value(Alias::new("claimed_until"), Option::<DateTime<Utc>>::None)
        .and_where(Expr::col(Alias::new("id")).eq(id));
    database.write().execute(&update).await?;
    metrics::counter!("events_delivered_total").increment(1);
    Ok(())
}

/// Ghi một lần giao hỏng: hẹn lần sau theo backoff, hoặc đánh dấu hỏng hẳn.
async fn mark_failed(
    database: &Database,
    table: &Alias,
    event: &Delivered,
    attempts: i32,
    error: &Error,
) -> Result<(), Error> {
    let now = now();
    let message: String = describe(error).chars().take(MAX_ERROR_LENGTH).collect();
    let mut update = Query::update();
    update
        .table(table.clone())
        .value(Alias::new("attempts"), attempts)
        .value(Alias::new("next_attempt_at"), now + backoff(attempts))
        .value(Alias::new("claimed_until"), Option::<DateTime<Utc>>::None)
        .value(Alias::new("last_error"), message.clone())
        .and_where(Expr::col(Alias::new("id")).eq(event.id));
    // Xoá dữ liệu của tài khoản đã xoá là nghĩa vụ, không phải việc phụ: không bỏ cuộc,
    // cứ thử lại (tối đa mỗi giờ một lần) tới khi mọi module xong (D-149). Tài khoản đã
    // mất nên người dùng không tự xin xoá lại được.
    let never_give_up = event.name == crate::privacy::USER_DELETED;
    if attempts >= MAX_ATTEMPTS && !never_give_up {
        update.value(Alias::new("failed_at"), now);
        tracing::error!(
            event_id = %event.id,
            event = %event.name,
            attempts,
            error = %message,
            "event delivery failed for good; it will not be retried"
        );
        metrics::counter!("events_failed_total").increment(1);
    } else if attempts >= MAX_ATTEMPTS {
        tracing::error!(
            event_id = %event.id,
            event = %event.name,
            attempts,
            error = %message,
            "erasing a deleted user's data keeps failing; it is retried every hour"
        );
    } else {
        tracing::warn!(
            event_id = %event.id,
            event = %event.name,
            attempts,
            error = %message,
            "event delivery failed; will retry"
        );
    }
    database.write().execute(&update).await?;
    Ok(())
}

/// Điều kiện "đang chờ giao": chưa giao, chưa hỏng hẳn, tới giờ thử, không ai đang giữ.
fn pending(now: DateTime<Utc>) -> Condition {
    Condition::all()
        .add(Expr::col(Alias::new("delivered_at")).is_null())
        .add(Expr::col(Alias::new("failed_at")).is_null())
        .add(Expr::col(Alias::new("next_attempt_at")).lte(now))
        .add(
            Condition::any()
                .add(Expr::col(Alias::new("claimed_until")).is_null())
                .add(Expr::col(Alias::new("claimed_until")).lt(now)),
        )
}

/// Chờ trước lần giao thứ `attempts + 1`: 2, 4, 8… giây, tối đa một giờ.
fn backoff(attempts: i32) -> chrono::Duration {
    let exponent = u32::try_from(attempts.clamp(1, 12)).unwrap_or(12);
    chrono::Duration::seconds(std::cmp::min(i64::from(2u32.pow(exponent)), 3600))
}

/// Lỗi kèm nguyên nhân gốc (lỗi nội bộ chỉ có câu chung; nguyên nhân mới có ích để sửa).
fn describe(error: &Error) -> String {
    match std::error::Error::source(error) {
        Some(source) => format!("{}: {source}", error.code()),
        None => format!("{}: {}", error.code(), error.message()),
    }
}

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_tang_dan_va_co_tran() {
        assert_eq!(backoff(1), chrono::Duration::seconds(2));
        assert_eq!(backoff(3), chrono::Duration::seconds(8));
        assert_eq!(backoff(30), chrono::Duration::seconds(3600));
    }
}
