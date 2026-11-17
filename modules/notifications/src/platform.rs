//! Thông báo của nền tảng tới quản trị viên mọi khách (bước 35, D-348): nhân sự nền tảng gửi một lần;
//! job duyệt danh sách khách theo lô 50, mỗi khách gửi tới mọi quản trị viên (in-app + email). Khoá
//! chống trùng theo ID thông báo: job chạy lại không gửi trùng ai.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use autoxide_kernel::pagination::{PageQuery, PageRequest};
use autoxide_kernel::tenancy::{TenantContext, TenantFilter, TenantId};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::api::{Category, Notification, Recipient};
use crate::events;
use crate::permissions::PLATFORM_NOTICES_SEND;
use crate::registry::{PLATFORM_NOTICE, PLATFORM_SECURITY_NOTICE};
use crate::service::{NotificationsService, valid_link};

/// Số khách mỗi lượt job.
const BATCH: u32 = 50;

/// Một thông báo của nền tảng cần gửi.
#[derive(Debug, Clone)]
pub(crate) struct NewPlatformNotice {
    /// `security` hay `transactional`.
    pub category: String,
    pub title: String,
    pub body: String,
    pub link: Option<String>,
}

/// Job gửi thông báo của nền tảng tới một lô khách; còn khách thì xếp lượt kế.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PlatformNoticeJob {
    pub notice_id: Uuid,
    pub category: String,
    pub title: String,
    pub body: String,
    pub link: Option<String>,
    /// `next_cursor` của trang khách trước; `None`: trang đầu.
    pub cursor: Option<String>,
}

impl Job for PlatformNoticeJob {
    const KIND: &'static str = "notifications.platform_notice";
}

impl NotificationsService {
    /// Xếp hàng thông báo tới quản trị viên mọi khách: nhân sự nền tảng có
    /// `notifications.platform_notices.send`. Trả ID thông báo.
    pub(crate) async fn queue_platform_notice(
        &self,
        principal: &Principal,
        input: NewPlatformNotice,
    ) -> Result<Uuid, Error> {
        principal.require(&PLATFORM_NOTICES_SEND)?;
        autoxide_kernel::tenancy::require_platform(principal)?;
        let mut problems = Vec::new();
        if !["security", "transactional"].contains(&input.category.as_str()) {
            problems.push(FieldError::new(
                "category",
                "invalid",
                "must be `security` or `transactional`",
            ));
        }
        if input.title.trim().is_empty() || input.title.chars().count() > 200 {
            problems.push(FieldError::new("title", "length", "must be 1 to 200 characters"));
        }
        if input.body.trim().is_empty() || input.body.chars().count() > 5000 {
            problems.push(FieldError::new("body", "length", "must be 1 to 5000 characters"));
        }
        if let Some(link) = &input.link
            && !valid_link(link)
        {
            problems.push(FieldError::new(
                "link",
                "invalid",
                "must be an http(s) URL or a path starting with `/`, at most 2000 characters",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let notice_id = Uuid::now_v7();
        let platform = self.databases.default_tenant();
        let job = PlatformNoticeJob {
            notice_id,
            category: input.category.clone(),
            title: input.title,
            body: input.body,
            link: input.link,
            cursor: None,
        };
        let transaction = platform.database().begin_write().await?;
        self.jobs
            .enqueue(
                &transaction,
                &platform,
                &job,
                EnqueueOptions::default().unique_key(format!("platform_notice:{notice_id}")),
            )
            .await?;
        let event = events::PlatformNoticeQueued(
            json!({"notice_id": notice_id, "category": input.category}),
        );
        self.outbox
            .record(
                &transaction,
                &platform,
                Actor::from(principal),
                Some(format!("notifications.platform_notice:{notice_id}")),
                &event,
            )
            .await?;
        transaction.commit().await?;
        tracing::info!(notice_id = %notice_id, "platform notice queued");
        Ok(notice_id)
    }

    /// Gửi thông báo tới quản trị viên của một khách.
    async fn notice_to(
        &self,
        tenant: &TenantContext,
        job: &PlatformNoticeJob,
    ) -> Result<(), Error> {
        let kind = if job.category == Category::Security.as_str() {
            PLATFORM_SECURITY_NOTICE
        } else {
            PLATFORM_NOTICE
        };
        let mut notification = Notification::new(
            kind.key(),
            Recipient::TenantAdmins,
            json!({"title": job.title, "body": job.body}),
            format!("platform_notice:{}", job.notice_id),
        );
        notification.link = job.link.clone();
        self.notify(tenant, notification).await.map(|_| ())
    }

    /// Một lượt của job: một trang khách đang hoạt động; còn trang sau thì xếp lượt kế.
    pub(crate) async fn fan_out(&self, job: &PlatformNoticeJob) -> Result<(), Error> {
        let Some(tenants) = &self.tenants else {
            // Bản cài không có sổ khách (test tối giản): chỉ khách mặc định.
            return self.notice_to(&self.databases.default_tenant(), job).await;
        };
        let request = PageRequest::try_from(PageQuery::new(
            Some(BATCH.to_string()),
            job.cursor.clone(),
            Some("asc".to_owned()),
        ))?;
        let filter = TenantFilter { status: Some("active".to_owned()), ..TenantFilter::default() };
        let page = tenants.list(&filter, &request).await?;
        for tenant in &page.data {
            let context = match self.databases.tenant(TenantId::from_uuid(tenant.id)).await {
                Ok(context) => context,
                Err(error) => {
                    tracing::warn!(tenant_id = %tenant.id, error = %error, "platform notice skipped a tenant");
                    continue;
                }
            };
            // Một khách lỗi (database tạm không trả lời) không chặn các khách khác.
            if let Err(error) = self.notice_to(&context, job).await {
                tracing::warn!(tenant_id = %tenant.id, error = %error, "platform notice failed for a tenant");
            }
        }
        if let Some(next) = page.pagination.next_cursor {
            let platform = self.databases.default_tenant();
            let following = PlatformNoticeJob { cursor: Some(next.clone()), ..job.clone() };
            self.jobs
                .enqueue(
                    platform.database().write(),
                    &platform,
                    &following,
                    EnqueueOptions::default()
                        .unique_key(format!("platform_notice:{}:{next}", job.notice_id)),
                )
                .await?;
        }
        Ok(())
    }
}

/// Bộ chạy job `notifications.platform_notice`.
pub(crate) struct PlatformNoticeRunner(pub Arc<NotificationsService>);

#[async_trait::async_trait]
impl JobHandler for PlatformNoticeRunner {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let Ok(notice) = serde_json::from_value::<PlatformNoticeJob>(job.payload.clone()) else {
            return Ok(());
        };
        self.0.fan_out(&notice).await
    }
}
