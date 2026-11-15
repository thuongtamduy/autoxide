//! Gửi event tới các endpoint.
//!
//! 1. [`Fanout`] nhận mọi event (đăng ký `*`). Với mỗi endpoint đang bật muốn event đó:
//!    ghi một dòng `webhooks_delivery` và đưa một job vào hàng đợi, cùng transaction.
//!    Event được giao lại thì index unique (endpoint, event) chặn bản trùng.
//! 2. [`Sender`] chạy job: POST thân đã dựng tới endpoint, ký theo Standard Webhooks.
//!    Bên nhận trả 2xx là xong; khác đi (hay quá giờ) thì job lỗi và job queue thử lại
//!    với backoff, tối đa [`MAX_ATTEMPTS`] lần — rồi lần gửi thành `failed`.
//! 3. [`Cleanup`] chạy mỗi ngày, xoá lần gửi cũ hơn [`KEEP_DELIVERIES_DAYS`] ngày.

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler, JobQueue};
use autoxide_kernel::tenancy::TenantContext;
use sea_orm::sea_query::{Expr, ExprTrait as _};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, SqlErr};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::entity::{delivery, endpoint};
use crate::service::{Endpoint, now};
use crate::signature;

/// Số lần thử gửi tối đa: backoff 2, 4, 8… giây tới một giờ, tổng gần hai giờ.
pub(crate) const MAX_ATTEMPTS: i32 = 12;

/// Lần gửi được giữ để xem lại trong ngần này ngày.
pub(crate) const KEEP_DELIVERIES_DAYS: i64 = 30;

/// Độ dài tối đa của lỗi được lưu.
const MAX_ERROR_LENGTH: usize = 500;

pub(crate) const SEND_KIND: &str = "webhooks.send";
pub(crate) const CLEANUP_KIND: &str = "webhooks.cleanup";

/// Job gửi một lần gửi.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Send {
    pub delivery_id: Uuid,
}

impl Job for Send {
    const KIND: &'static str = SEND_KIND;
}

/// Nhận mọi event, tạo lần gửi cho các endpoint muốn nó.
pub(crate) struct Fanout {
    pub database: Database,
    pub jobs: JobQueue,
}

#[async_trait::async_trait]
impl EventHandler for Fanout {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        // Một câu đọc mỗi event: bảng endpoint nhỏ. Đọc trên kết nối ghi để endpoint
        // vừa tạo nhận ngay event kế tiếp.
        let rows = endpoint::Entity::find()
            .filter(endpoint::Column::DisabledAt.is_null())
            .all(self.database.write())
            .await?;
        let mut targets = Vec::new();
        for row in rows {
            let endpoint = Endpoint::from_row(row)?;
            if endpoint.wants(&event.name) {
                targets.push(endpoint);
            }
        }
        if targets.is_empty() {
            return Ok(());
        }
        let body = body(event).to_string();
        for target in targets {
            self.create(&target, event, &body).await?;
        }
        Ok(())
    }
}

impl Fanout {
    async fn create(&self, target: &Endpoint, event: &Delivered, body: &str) -> Result<(), Error> {
        let transaction = self.database.begin_write().await?;
        let id = Uuid::now_v7();
        let inserted = delivery::ActiveModel {
            id: Set(id),
            endpoint_id: Set(target.id),
            event_id: Set(event.id),
            event_name: Set(event.name.clone()),
            body: Set(body.to_owned()),
            status: Set(STATUS_PENDING.to_owned()),
            attempts: Set(0),
            response_status: Set(None),
            last_error: Set(None),
            created_at: Set(now()),
            last_attempt_at: Set(None),
            delivered_at: Set(None),
        }
        .insert(&transaction)
        .await;
        match inserted {
            Ok(_) => {}
            // Đã tạo ở lần giao trước của cùng event.
            Err(error) if matches!(error.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))) => {
                transaction.rollback().await?;
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        }
        let options = EnqueueOptions::default().max_attempts(MAX_ATTEMPTS);
        self.jobs
            .enqueue(
                &transaction,
                &TenantContext::legacy_default(&self.database),
                &Send { delivery_id: id },
                options,
            )
            .await?;
        transaction.commit().await?;
        Ok(())
    }
}

/// Thân request theo dạng khuyến nghị của Standard Webhooks: `type`, `timestamp`, `data`.
fn body(event: &Delivered) -> serde_json::Value {
    json!({
        "type": event.name,
        "timestamp": event.occurred_at,
        "data": {
            "id": event.id,
            "module": event.module,
            "subject": event.subject,
            "actor": {
                "user_id": event.actor.user_id,
                "api_key_id": event.actor.api_key_id,
                "kind": event.actor.kind(),
            },
            "payload": event.payload,
        },
    })
}

pub(crate) const STATUS_PENDING: &str = "pending";
pub(crate) const STATUS_SUCCEEDED: &str = "succeeded";
pub(crate) const STATUS_FAILED: &str = "failed";

/// Chạy job gửi.
pub(crate) struct Sender {
    pub database: Database,
    pub client: reqwest::Client,
    /// `AUTOXIDE_WEBHOOKS_ALLOW_PRIVATE_NETWORKS`.
    pub allow_private_networks: bool,
}

impl Sender {
    /// HTTP client dùng chung: không theo chuyển hướng (địa chỉ đã kiểm là địa chỉ được
    /// gọi), giới hạn thời gian chờ, và — trừ khi cấu hình cho phép — chỉ kết nối tới IP
    /// public (D-149).
    pub(crate) fn client(
        timeout: Duration,
        allow_private_networks: bool,
    ) -> Result<reqwest::Client, Error> {
        autoxide_kernel::tls::install_crypto_provider();
        let mut builder = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            // Proxy của hệ thống sẽ tự phân giải tên, vượt qua bộ lọc: không dùng.
            .no_proxy()
            .user_agent(concat!("Autoxide-Webhooks/", env!("CARGO_PKG_VERSION")));
        if !allow_private_networks {
            builder = builder.dns_resolver(crate::network::PublicOnly);
        }
        builder.build().map_err(|error| Error::internal(format!("webhook HTTP client: {error}")))
    }
}

#[async_trait::async_trait]
impl JobHandler for Sender {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let input: Send = serde_json::from_value(job.payload.clone())
            .map_err(|error| Error::internal(format!("webhook job payload: {error}")))?;
        let Some(mut row) =
            delivery::Entity::find_by_id(input.delivery_id).one(self.database.write()).await?
        else {
            // Endpoint đã bị xoá (cùng lịch sử gửi): không còn gì để gửi.
            return Ok(());
        };
        // Lần gửi đã hỏng hẳn mà job chạy lại từ lần đầu: người vận hành vừa đưa job từ
        // dead-letter queue về (`autoxide jobs retry`) — mở lại lần gửi rồi gửi thật (D-161).
        // Lần đầu bình thường thì lần gửi luôn đang `pending`.
        if row.status == STATUS_FAILED && job.attempt == 1 && self.reopen(&row).await? {
            row.status = STATUS_PENDING.to_owned();
        }
        if row.status != STATUS_PENDING {
            return Ok(());
        }
        let target =
            endpoint::Entity::find_by_id(row.endpoint_id).one(self.database.write()).await?;
        let Some(target) = target.filter(|target| target.disabled_at.is_none()) else {
            self.finish(&row, STATUS_FAILED, None, Some("the endpoint was disabled")).await?;
            return Ok(());
        };

        // Địa chỉ là IP thì không qua bộ phân giải: kiểm ở đây (cấu hình có thể đã đổi từ
        // lúc tạo webhook). Không lành lại khi thử lại nên thôi luôn.
        let public =
            reqwest::Url::parse(&target.url).is_ok_and(|url| crate::network::is_public_host(&url));
        if !self.allow_private_networks && !public {
            let problem = format!("the address {}", crate::network::PRIVATE_ADDRESS);
            self.finish(&row, STATUS_FAILED, None, Some(&problem)).await?;
            return Ok(());
        }

        let timestamp = now().timestamp();
        let message_id = row.id.to_string();
        let signature = signature::sign(&target.secret, &message_id, timestamp, &row.body)?;
        let sent = self
            .client
            .post(&target.url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header("webhook-id", &message_id)
            .header("webhook-timestamp", timestamp.to_string())
            .header("webhook-signature", signature)
            .body(row.body.clone())
            .send()
            .await;
        let last = job.attempt >= MAX_ATTEMPTS;
        match sent {
            Ok(response) if response.status().is_success() => {
                let status = i32::from(response.status().as_u16());
                self.finish(&row, STATUS_SUCCEEDED, Some(status), None).await?;
                metrics::counter!("webhooks_delivered_total").increment(1);
                Ok(())
            }
            Ok(response) => {
                let status = response.status();
                let problem = format!("the endpoint answered {status}");
                let outcome = if last { STATUS_FAILED } else { STATUS_PENDING };
                self.finish(&row, outcome, Some(i32::from(status.as_u16())), Some(&problem))
                    .await?;
                Err(failed(&problem, last))
            }
            Err(error) => {
                // Lý do chi tiết (chuỗi lỗi của reqwest) chỉ vào log của máy chủ, không vào
                // lịch sử gửi mà API trả ra: nó giúp dò cổng và máy trong mạng (D-149). Không
                // ghi URL (có thể chứa token trong query).
                let detail = std::error::Error::source(&error).map(ToString::to_string);
                tracing::warn!(delivery_id = %row.id, detail, "webhook request failed");
                let problem = describe(&error);
                let outcome = if last { STATUS_FAILED } else { STATUS_PENDING };
                self.finish(&row, outcome, None, Some(&problem)).await?;
                Err(failed(&problem, last))
            }
        }
    }
}

impl Sender {
    /// Lần gửi hỏng hẳn `row` thành `pending` lại; `false` nếu nó không còn `failed`.
    async fn reopen(&self, row: &delivery::Model) -> Result<bool, Error> {
        let reopened = delivery::Entity::update_many()
            .col_expr(delivery::Column::Status, Expr::value(STATUS_PENDING))
            .filter(delivery::Column::Id.eq(row.id))
            .filter(delivery::Column::Status.eq(STATUS_FAILED))
            .exec(self.database.write())
            .await?;
        if reopened.rows_affected == 1 {
            tracing::info!(delivery_id = %row.id, "failed webhook delivery reopened by a job retry");
        }
        Ok(reopened.rows_affected == 1)
    }

    /// Ghi kết quả của một lần thử.
    async fn finish(
        &self,
        row: &delivery::Model,
        status: &str,
        response_status: Option<i32>,
        error: Option<&str>,
    ) -> Result<(), Error> {
        let now = now();
        let mut update = delivery::Entity::update_many()
            .col_expr(delivery::Column::Status, Expr::value(status))
            .col_expr(delivery::Column::Attempts, Expr::col(delivery::Column::Attempts).add(1))
            .col_expr(delivery::Column::ResponseStatus, Expr::value(response_status))
            .col_expr(
                delivery::Column::LastError,
                Expr::value(
                    error.map(|error| error.chars().take(MAX_ERROR_LENGTH).collect::<String>()),
                ),
            )
            .col_expr(delivery::Column::LastAttemptAt, Expr::value(Some(now)));
        if status == STATUS_SUCCEEDED {
            update = update.col_expr(delivery::Column::DeliveredAt, Expr::value(Some(now)));
        }
        update.filter(delivery::Column::Id.eq(row.id)).exec(self.database.write()).await?;
        if status == STATUS_FAILED {
            tracing::warn!(delivery_id = %row.id, endpoint_id = %row.endpoint_id, "webhook delivery failed for good");
            metrics::counter!("webhooks_failed_total").increment(1);
        }
        Ok(())
    }
}

fn failed(problem: &str, last: bool) -> Error {
    let next = if last { "giving up" } else { "will retry" };
    Error::internal(format!("webhook delivery: {problem}; {next}"))
}

/// Lỗi gửi, chỉ là một trong vài loại cố định: không kèm URL, không kèm chi tiết của lỗi.
fn describe(error: &reqwest::Error) -> String {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(current) = source {
        if current.is::<crate::network::PrivateAddress>() {
            return format!("the address {}", crate::network::PRIVATE_ADDRESS);
        }
        source = current.source();
    }
    if error.is_timeout() {
        "the endpoint did not answer in time".to_owned()
    } else if error.is_connect() {
        "could not connect to the endpoint (DNS, network or TLS)".to_owned()
    } else {
        "the request failed".to_owned()
    }
}

/// Xoá lần gửi cũ.
pub(crate) struct Cleanup {
    pub database: Database,
}

#[async_trait::async_trait]
impl JobHandler for Cleanup {
    async fn run(&self, _: &JobContext) -> Result<(), Error> {
        let before = now() - chrono::Duration::days(KEEP_DELIVERIES_DAYS);
        let deleted = delivery::Entity::delete_many()
            .filter(delivery::Column::CreatedAt.lt(before))
            .filter(delivery::Column::Status.ne(STATUS_PENDING))
            .exec(self.database.write())
            .await?;
        tracing::info!(deleted = deleted.rows_affected, "old webhook deliveries removed");
        Ok(())
    }
}

/// Dùng trong `Arc<dyn …>`.
pub(crate) fn fanout(database: Database) -> Arc<Fanout> {
    Arc::new(Fanout { database, jobs: JobQueue::new("webhooks") })
}
