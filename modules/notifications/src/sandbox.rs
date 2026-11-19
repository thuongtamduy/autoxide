//! Mẫu thông báo trong sandbox (bước 38, D-232, D-20261119-step38-sandbox-copy): mẫu khách tự soạn
//! đang dùng ở khách thật được sao sang sandbox và xuất bản ở đó, để thử thông báo đúng như thật.
//! Chỉ nội dung mẫu — không sao hộp thư, đăng ký thiết bị hay lịch sử gửi (dữ liệu cá nhân).

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::ScopeExt;
use autoxide_kernel::tenancy::sandbox::{CopyContext, SandboxCopy, copied, link_copy};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use crate::custom::Content;
use crate::entity::{template_live, template_version};
use crate::service::NotificationsService;

/// Loại bản ghi (bảng nối của kernel): một phiên bản mẫu đã xuất bản.
const TEMPLATE_KIND: &str = "notifications.template";

pub(crate) struct Templates {
    pub(crate) service: Arc<NotificationsService>,
}

#[async_trait::async_trait]
impl SandboxCopy for Templates {
    async fn copy(&self, context: &CopyContext<'_>) -> Result<u64, Error> {
        let source = context.source;
        let lives = template_live::Entity::find()
            .filter(template_live::Column::TenantId.eq(source.id().as_uuid()))
            .all(source.database().read())
            .await?;
        let mut count = 0;
        for live in lives {
            let version = template_version::Entity::find()
                .scoped(source)
                .filter(template_version::Column::Kind.eq(live.kind.as_str()))
                .filter(template_version::Column::Channel.eq(live.channel.as_str()))
                .filter(template_version::Column::Locale.eq(live.locale.as_str()))
                .filter(template_version::Column::Version.eq(live.version))
                .one(source.database().read())
                .await?;
            let Some(version) = version else { continue };
            if copied(context.sandbox, TEMPLATE_KIND, version.id).await?.is_some() {
                continue;
            }
            let target = (live.kind.as_str(), live.channel.as_str(), live.locale.as_str());
            let content = Content::from(&version);
            // Loại hay kênh không còn trong chương trình: bỏ qua mẫu đó, không dừng cả job.
            let saved = self.service.put_draft(context.sandbox, context.principal, target, content);
            if let Err(error) = saved.await {
                tracing::warn!(code = error.code(), "template not copied into the sandbox");
                continue;
            }
            let published =
                self.service.publish_template(context.sandbox, context.principal, target).await?;
            link_copy(context.sandbox, TEMPLATE_KIND, published.id, version.id).await?;
            count += 1;
        }
        Ok(count)
    }
}
