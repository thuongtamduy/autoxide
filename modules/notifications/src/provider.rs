//! Trait module `notifications` công bố: `Notifier`.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantId;

use crate::api::{Notification, NotificationKind, Notifier, Outcome};
use crate::service::NotificationsService;

pub(crate) struct Provider(pub Arc<NotificationsService>);

#[async_trait::async_trait]
impl Notifier for Provider {
    fn declare(&self, kind: NotificationKind) -> Result<(), Error> {
        self.0.kinds.declare(kind)
    }

    async fn notify(&self, tenant: TenantId, notification: Notification) -> Result<Outcome, Error> {
        let tenant = self.0.tenant(tenant).await?;
        self.0.notify(&tenant, notification).await
    }
}
