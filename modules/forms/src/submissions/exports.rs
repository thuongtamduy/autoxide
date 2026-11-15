//! Xuất CSV lớn bằng job (đặc tả 32 B3, D-249). Form có nhiều bài thì bản xuất đồng bộ
//! (`GET /{id}/submissions/export`) giữ cả file trong bộ nhớ và giữ request lâu; ở đây:
//!
//! 1. `POST /{id}/submissions/exports` ghi một dòng `forms_exports` (`pending`) và xếp job
//!    `forms.export_submissions` trong CÙNG transaction, trả 201 ngay. Mỗi form chỉ một bản đang
//!    chạy: transaction khoá dòng form trước khi xét, nên hai yêu cầu cùng lúc thì một bên nhận
//!    409 `forms.export_in_progress` (cả hai database).
//! 2. Job đọc bài theo trang 200 và ghi theo luồng vào nơi lưu chung (`dyn Storage`, khoá
//!    `forms/exports/<id>.csv`), bằng đúng hàm của bản xuất đồng bộ (`csv::line`): cùng nội
//!    dung, từng byte. Xong thì `ready` kèm số dòng, số byte, hạn giữ, và event
//!    `forms.submissions_exported`. Lỗi thì `failed` (không thử lại: người dùng yêu cầu bản
//!    mới). Job chạy lại sau khi đã xong thì không làm gì.
//! 3. `GET /exports/{id}` cho trạng thái và, khi `ready`, link tải ký HMAC-SHA256 (khoá
//!    `SecretKey::derive("forms.export_download")`, trên ID và hạn dùng) sống
//!    `AUTOXIDE_FORMS_EXPORT_LINK_MINUTES` — dán vào trình duyệt được, không cần đăng nhập.
//! 4. Job dọn dẹp hằng ngày xoá file và dòng quá `expires_at`
//!    (`AUTOXIDE_FORMS_EXPORT_KEEP_HOURS`); xoá theo email và xoá form xoá luôn file xuất của
//!    form đó — file chứa dữ liệu cá nhân.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use autoxide_kernel::rows;
use autoxide_kernel::storage::{ByteStream, StoredFile};
use autoxide_kernel::tenancy::TenantContext;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures_util::StreamExt as _;
use hmac::{Hmac, KeyInit, Mac};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::Sha256;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::{export, form};
use crate::events::{self, Change};
use crate::permissions::FORMS_EXPORT;
use crate::service::{FormsService, form_not_found, now};
use crate::submissions::csv;
use crate::submissions::inbox::{ExportColumns, SubmissionStatus, export_file_name};

/// Loại job ghi file xuất.
pub(crate) const EXPORT: &str = "forms.export_submissions";

/// Nhãn của khoá ký link tải (dẫn từ khoá gốc của chương trình).
const LINK_KEY: &str = "forms.export_download";

type HmacSha256 = Hmac<Sha256>;

/// Payload của job: chỉ ID, không có dữ liệu cá nhân.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ExportJob {
    pub export_id: Uuid,
}

impl Job for ExportJob {
    const KIND: &'static str = EXPORT;
}

/// Where an export is: `pending` (asked for, waiting for the job), `running` (the file is being
/// written), `ready` (get `download_url`) or `failed` (ask for a new one).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExportState {
    Pending,
    Running,
    Ready,
    Failed,
}

impl ExportState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ExportState::Pending => "pending",
            ExportState::Running => "running",
            ExportState::Ready => "ready",
            ExportState::Failed => "failed",
        }
    }

    fn parse(raw: &str) -> Result<Self, Error> {
        match raw {
            "pending" => Ok(ExportState::Pending),
            "running" => Ok(ExportState::Running),
            "ready" => Ok(ExportState::Ready),
            "failed" => Ok(ExportState::Failed),
            other => Err(Error::internal(format!("unknown export state `{other}`"))),
        }
    }
}

/// Một bản xuất, kèm link tải khi đã sẵn sàng.
#[derive(Debug, Clone)]
pub(crate) struct ExportView {
    pub row: export::Model,
    pub state: ExportState,
    /// Link tải và lúc nó hết hạn (chỉ khi `ready`, file chưa hết hạn giữ).
    pub download: Option<(String, DateTime<Utc>)>,
}

/// Bộ chạy job `forms.export_submissions`.
pub(crate) struct Exporter(pub Arc<FormsService>);

#[async_trait::async_trait]
impl JobHandler for Exporter {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let job: ExportJob =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        FormsService::run_export(&self.0, job.export_id).await
    }
}

impl FormsService {
    /// Yêu cầu xuất bài của form `form_id` qua job. Cần `forms.export`. Form đang có bản xuất
    /// chưa xong: `forms.export_in_progress`.
    pub(crate) async fn request_export(
        &self,
        principal: &Principal,
        form_id: Uuid,
        status: Option<SubmissionStatus>,
    ) -> Result<ExportView, Error> {
        principal.require(&FORMS_EXPORT)?;
        let transaction = self.database.begin_write().await?;
        // Khoá dòng form tới hết transaction: hai yêu cầu cùng lúc cho một form xét lần lượt,
        // bên sau thấy bản của bên trước (Postgres chờ khoá; SQLite chỉ một bên ghi).
        if rows::lock_one(&transaction, form::Entity::find_by_id(form_id)).await?.is_none() {
            return Err(form_not_found());
        }
        let running = export::Entity::find()
            .filter(export::Column::FormId.eq(form_id))
            .filter(
                export::Column::State
                    .is_in([ExportState::Pending.as_str(), ExportState::Running.as_str()]),
            )
            .one(&transaction)
            .await?;
        if let Some(running) = running {
            return Err(in_progress(running.id));
        }
        let row = export::ActiveModel {
            id: Set(Uuid::now_v7()),
            form_id: Set(form_id),
            requested_by: Set(Some(principal.user_id())),
            status_filter: Set(status.map(|status| status.as_str().to_owned())),
            state: Set(ExportState::Pending.as_str().to_owned()),
            row_count: Set(None),
            size: Set(None),
            storage_key: Set(None),
            created_at: Set(now()),
            finished_at: Set(None),
            expires_at: Set(None),
        }
        .insert(&transaction)
        .await?;
        self.jobs
            .enqueue(
                &transaction,
                &TenantContext::legacy_default(&self.database),
                &ExportJob { export_id: row.id },
                EnqueueOptions::default(),
            )
            .await?;
        transaction.commit().await?;
        tracing::info!(form_id = %form_id, export_id = %row.id, "forms export requested");
        self.view(row).await
    }

    /// Bản xuất `id`. Cần `forms.export`.
    pub(crate) async fn get_export(
        &self,
        principal: &Principal,
        id: Uuid,
    ) -> Result<ExportView, Error> {
        principal.require(&FORMS_EXPORT)?;
        let row = export::Entity::find_by_id(id)
            .one(self.database.read())
            .await?
            .ok_or_else(export_not_found)?;
        self.view(row).await
    }

    async fn view(&self, row: export::Model) -> Result<ExportView, Error> {
        let state = ExportState::parse(&row.state)?;
        let download = match (state, row.expires_at) {
            (ExportState::Ready, Some(kept)) if kept > now() => {
                let minutes = i64::from(self.config.export_link_minutes);
                // Link không sống lâu hơn file.
                let expires = (now() + chrono::Duration::minutes(minutes)).min(kept);
                let signature = self.signature(row.id, expires.timestamp()).await?;
                let path = format!(
                    "/api/v1/forms/exports/{}/file?expires={}&signature={signature}",
                    row.id,
                    expires.timestamp()
                );
                // Có `AUTOXIDE_HTTP_PUBLIC_URL` thì link tuyệt đối; không thì đường dẫn trên API.
                Some((self.public_url.link(&path).unwrap_or(path), expires))
            }
            _ => None,
        };
        Ok(ExportView { row, state, download })
    }

    /// Chữ ký (hex) của link tải bản xuất `id` hết hạn lúc `expires` (giây Unix).
    pub(crate) async fn signature(&self, id: Uuid, expires: i64) -> Result<String, Error> {
        let mac = self.link_mac(id, expires).await?;
        Ok(mac.finalize().into_bytes().iter().map(|byte| format!("{byte:02x}")).collect())
    }

    async fn link_mac(&self, id: Uuid, expires: i64) -> Result<HmacSha256, Error> {
        let key = self.secret.derive(LINK_KEY).await?;
        let mut mac = <HmacSha256 as KeyInit>::new_from_slice(&key)
            .map_err(|error| Error::internal(format!("hmac: {error}")))?;
        mac.update(id.as_bytes());
        mac.update(&expires.to_be_bytes());
        Ok(mac)
    }

    /// File của bản xuất `id`, qua link ký (không cần đăng nhập): tên file và nội dung. Link
    /// sai hay hết hạn: `forms.export_link_invalid`; file đã bị xoá: `forms.export_not_found`.
    pub(crate) async fn download_export(
        &self,
        id: Uuid,
        expires: i64,
        signature: &str,
    ) -> Result<(String, StoredFile), Error> {
        let signature = hex(signature).ok_or_else(link_invalid)?;
        // `verify_slice` so sánh trong thời gian không đổi: không lộ dần chữ ký đúng.
        self.link_mac(id, expires).await?.verify_slice(&signature).map_err(|_| link_invalid())?;
        if expires < now().timestamp() {
            return Err(link_invalid());
        }
        let row = export::Entity::find_by_id(id)
            .one(self.database.read())
            .await?
            .ok_or_else(export_not_found)?;
        let key = row.storage_key.as_deref().ok_or_else(export_not_found)?;
        if row.state != ExportState::Ready.as_str() {
            return Err(export_not_found());
        }
        let file = self.storage.get(key).await?.ok_or_else(export_not_found)?;
        let name = match self.find_form(row.form_id).await {
            Ok(form) => export_file_name(&form),
            Err(_) => format!("{}-submissions.csv", row.form_id),
        };
        Ok((name, file))
    }

    /// Chạy job xuất `id`: ghi file, đổi trạng thái. Chạy lại khi đã xong thì không làm gì.
    pub(crate) async fn run_export(service: &Arc<FormsService>, id: Uuid) -> Result<(), Error> {
        let Some(row) = export::Entity::find_by_id(id).one(service.database.read()).await? else {
            // Bản xuất đã bị xoá (form bị xoá, xoá theo email) trước khi job chạy.
            return Ok(());
        };
        if matches!(ExportState::parse(&row.state)?, ExportState::Ready | ExportState::Failed) {
            return Ok(());
        }
        let mut running = row.clone().into_active_model();
        running.state = Set(ExportState::Running.as_str().to_owned());
        running.update(service.database.write()).await?;
        let result = Self::write_export(service, &row).await;
        let finished = now();
        let keep = chrono::Duration::hours(i64::from(service.config.export_keep_hours));
        let transaction = service.database.begin_write().await?;
        let mut done = row.clone().into_active_model();
        done.finished_at = Set(Some(finished));
        done.expires_at = Set(Some(finished + keep));
        match &result {
            Ok((count, size, key)) => {
                done.state = Set(ExportState::Ready.as_str().to_owned());
                done.row_count = Set(Some(i64::try_from(*count).map_err(Error::internal)?));
                done.size = Set(Some(i64::try_from(*size).map_err(Error::internal)?));
                done.storage_key = Set(Some(key.clone()));
                let change = Change {
                    before: None,
                    after: Some(json!({"export_id": row.id, "row_count": count})),
                };
                let actor = row.requested_by.map_or_else(Actor::system, Actor::user);
                service
                    .outbox
                    .record(
                        &transaction,
                        &TenantContext::legacy_default(&service.database),
                        actor,
                        events::form_subject(row.form_id),
                        &events::SubmissionsExported(change),
                    )
                    .await?;
            }
            Err(error) => {
                tracing::warn!(export_id = %row.id, error = %error, "forms export failed");
                done.state = Set(ExportState::Failed.as_str().to_owned());
            }
        }
        done.update(&transaction).await?;
        transaction.commit().await?;
        if let Ok((count, size, _)) = result {
            tracing::info!(export_id = %row.id, rows = count, bytes = size, "forms export ready");
        }
        Ok(())
    }

    /// Ghi file của bản xuất `row` theo luồng: (số dòng, số byte, khoá).
    async fn write_export(
        service: &Arc<FormsService>,
        row: &export::Model,
    ) -> Result<(u64, u64, String), Error> {
        let form = service.find_form(row.form_id).await?;
        let columns = service.export_columns(&form).await?;
        let status = row.status_filter.as_deref().and_then(SubmissionStatus::parse);
        let count = Arc::new(AtomicU64::new(0));
        let stream = lines(service.clone(), row.form_id, status, columns, count.clone());
        let key = storage_key(row.id);
        // Chạy lại sau lần hỏng giữa chừng: ghi đè cùng khoá.
        let size = service.storage.put(&key, stream).await?;
        Ok((count.load(Ordering::SeqCst), size, key))
    }

    /// Xoá các bản xuất `rows`: file trước, dòng sau; xoá file lỗi thì để dòng lại cho lần dọn
    /// sau. Trả số bản đã xoá.
    pub(crate) async fn purge_exports(&self, rows: Vec<export::Model>) -> Result<u64, Error> {
        let mut purged = 0;
        for row in rows {
            if let Some(key) = &row.storage_key
                && let Err(error) = self.storage.delete(key).await
            {
                tracing::warn!(export_id = %row.id, error = %error, "forms export file not deleted");
                continue;
            }
            purged += export::Entity::delete_by_id(row.id)
                .exec(self.database.write())
                .await?
                .rows_affected;
        }
        Ok(purged)
    }

    /// Mọi bản xuất của các form `form_ids`.
    pub(crate) async fn exports_of(
        &self,
        form_ids: Vec<Uuid>,
    ) -> Result<Vec<export::Model>, Error> {
        Ok(export::Entity::find()
            .filter(export::Column::FormId.is_in(form_ids))
            .order_by_asc(export::Column::Id)
            .all(self.database.read())
            .await?)
    }

    /// Bản xuất quá hạn giữ lúc `now`.
    pub(crate) async fn expired_exports(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<export::Model>, Error> {
        Ok(export::Entity::find()
            .filter(export::Column::ExpiresAt.lt(now))
            .order_by_asc(export::Column::Id)
            .all(self.database.read())
            .await?)
    }
}

/// Khoá ở nơi lưu của file xuất `id`.
pub(crate) fn storage_key(id: Uuid) -> String {
    format!("forms/exports/{id}.csv")
}

/// Trạng thái của luồng ghi file: tiêu đề, rồi từng trang.
enum Next {
    Header(Vec<String>),
    Page(Option<Uuid>),
    Done,
}

/// Nội dung file dưới dạng luồng: mỗi khúc là tiêu đề hay một trang bài. Không bao giờ giữ cả
/// file trong bộ nhớ.
fn lines(
    service: Arc<FormsService>,
    form_id: Uuid,
    status: Option<SubmissionStatus>,
    columns: ExportColumns,
    count: Arc<AtomicU64>,
) -> ByteStream {
    let ExportColumns { header, keys } = columns;
    // `try_unfold`: mỗi lần được đọc, tính khúc tiếp theo từ trạng thái và trả trạng thái mới.
    // Trạng thái sở hữu mọi thứ nó cần (`Arc` của service, các khoá), nên luồng sống độc lập
    // với hàm đã tạo ra nó — điều `Storage::put` đòi (`'static`).
    futures_util::stream::try_unfold(Next::Header(header), move |next| {
        let service = service.clone();
        let keys = keys.clone();
        let count = count.clone();
        async move {
            match next {
                Next::Done => Ok(None),
                Next::Header(header) => {
                    let chunk = format!("{}{}", csv::BOM, csv::line(&header));
                    Ok(Some((Bytes::from(chunk), Next::Page(None))))
                }
                Next::Page(after) => {
                    let (rows, cursor) = service.export_page(form_id, status, &keys, after).await?;
                    count.fetch_add(rows.len() as u64, Ordering::SeqCst);
                    let chunk: String = rows.iter().map(|row| csv::line(row)).collect();
                    let next = cursor.map_or(Next::Done, |cursor| Next::Page(Some(cursor)));
                    Ok(Some((Bytes::from(chunk), next)))
                }
            }
        }
    })
    .boxed()
}

/// Byte của chuỗi hex; sai dạng thì `None`.
fn hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(text.get(index..index + 2)?, 16).ok())
        .collect()
}

pub(crate) fn export_not_found() -> Error {
    Error::not_found("forms.export_not_found", "No export has this id, or it was deleted.")
}

fn in_progress(id: Uuid) -> Error {
    Error::conflict(
        "forms.export_in_progress",
        "This form already has an export that is not finished: `{export_id}`. Wait for it \
         (`GET /api/v1/forms/exports/{export_id}`) before asking for another.",
    )
    .with_param("export_id", id)
}

fn link_invalid() -> Error {
    Error::forbidden(
        "forms.export_link_invalid",
        "This download link is wrong or has expired; get a new one from \
         `GET /api/v1/forms/exports/<export id>`.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_hex() {
        assert_eq!(hex("00ff10"), Some(vec![0, 255, 16]));
        assert_eq!(hex("0"), None);
        assert_eq!(hex("zz"), None);
        assert_eq!(hex("é0"), None);
    }
}
