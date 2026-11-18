//! Tin cho việc của kernel (bước 51): job người dùng yêu cầu đã xong (bản xuất, thao tác hàng
//! loạt), người được nhắc trong ghi chú nội bộ. Chỉ tin trong ứng dụng, không email: việc vận hành
//! hằng ngày. Event giao lại thì `dedupe_key` (ID của event) chặn tin thứ hai.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use serde_json::json;
use uuid::Uuid;

use crate::api::{Notification, Recipient};
use crate::registry::{JOB_FINISHED as JOB_KIND, MENTIONED};
use crate::service::NotificationsService;

pub(crate) const JOB_FINISHED: &str = "kernel.job_finished";
pub(crate) const NOTE_ADDED: &str = "kernel.record_note_added";

pub(crate) struct KernelListener(pub Arc<NotificationsService>);

#[async_trait::async_trait]
impl EventHandler for KernelListener {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let payload = &event.payload;
        match event.name.as_str() {
            JOB_FINISHED => {
                let Some(user) =
                    payload["requested_by"].as_str().and_then(|id| id.parse::<Uuid>().ok())
                else {
                    return Ok(());
                };
                let notification = Notification::new(
                    JOB_KIND.key(),
                    Recipient::User(user),
                    json!({"label": payload["label"], "status": payload["status"]}),
                    format!("job_finished:{}", event.id),
                );
                self.0.notify(&event.tenant, notification).await?;
            }
            NOTE_ADDED => {
                let mentions = payload["mentions"].as_array().cloned().unwrap_or_default();
                for user in mentions.iter().filter_map(|id| id.as_str()?.parse::<Uuid>().ok()) {
                    let notification = Notification::new(
                        MENTIONED.key(),
                        Recipient::User(user),
                        json!({"kind": payload["kind"], "id": payload["record_id"]}),
                        format!("mentioned:{}:{user}", event.id),
                    );
                    self.0.notify(&event.tenant, notification).await?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
