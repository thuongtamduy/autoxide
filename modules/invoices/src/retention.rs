//! Lưu trữ chứng từ kế toán (bước 36 mục 7, D-190, D-355).
//!
//! Căn cứ (cần đối chiếu văn bản gốc / người có chuyên môn pháp lý): Luật Kế toán 88/2015/QH13 Điều 41
//! (chứng từ, sổ kế toán lưu ít nhất 10 năm); NĐ 174/2016/NĐ-CP Điều 12 (tính từ khi kết thúc kỳ kế
//! toán năm); Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15 + NĐ 356/2025 (giữ dữ liệu khi có nghĩa vụ pháp
//! lý khác, xoá / ẩn danh khi hết nghĩa vụ).
//!
//! - **Giữ**: bảng chứng từ nằm trong `Module::retained_tables` — xoá hẳn khách (D-337) không xoá
//!   chúng; chứng từ đã phát hành không xoá được qua API (`invoices.retention_hold`).
//! - **Bản lưu trữ** ([`InvoicesService::archive_tenant`]): ghi vào nơi lưu CHUNG dưới
//!   `archives/<khách>/<id>/` — ngoài `tenants/<khách>/` nên xoá hẳn khách không xoá — các file
//!   `ledger/journal.csv` (sổ nhật ký chung, qua `dyn Ledger::journal`), `documents.jsonl` (mỗi chứng từ
//!   một dòng JSON kèm dòng hàng, người mua, tỉ giá) và `manifest.json` (SHA-256, cỡ, số dòng từng
//!   file); một dòng `invoices_archives` (cũng giữ khi xoá khách). Người có `invoices.archive` gọi
//!   tay; job dọn dẹp tự làm cho khách đã xoá hẳn chưa có bản lưu.
//! - **Hết hạn** (job `invoices.purge_expired`, hằng ngày, mọi khách của database): chứng từ quá
//!   `retention_until` → ẩn danh ảnh chụp người mua cá nhân (tên, email, địa chỉ, mã định danh); khách
//!   đã xoá hẳn → xoá chứng từ quá hạn (và nháp, không phải chứng từ kế toán). Không bao giờ chạm chứng
//!   từ còn trong hạn lưu. Sổ cái (`ledger_*`) của khách đã xoá giữ nguyên: chuỗi băm không cắt được
//!   từng phần (D-355).

use autoxide_kernel::time::{Date, Timestamp};
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::pagination::{Order, Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext, TenantId, TenantStatus};
use bytes::Bytes;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::api::Buyer;
use crate::entity::{archive, document, line};
use crate::exports::{JOURNAL_HEADER, journal_rows, storage_unavailable, to_csv};
use crate::permissions::ARCHIVE;
use crate::service::{InvoicesService, describe, now};

pub(crate) const PURGE_EXPIRED: &str = "invoices.purge_expired";

/// Mỗi lượt dọn xử lý tối đa chừng này chứng từ / khách (lượt sau làm tiếp).
const BATCH: u64 = 500;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Người mua cá nhân sau khi hết hạn lưu: chỉ còn loại.
fn anonymized(buyer: &str) -> Option<String> {
    let parsed: Buyer = serde_json::from_str(buyer).unwrap_or_default();
    (parsed.kind != "business").then(|| json!({"kind": parsed.kind}).to_string())
}

/// Kết quả một lượt dọn.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Purged {
    pub anonymized: u64,
    pub deleted: u64,
    pub archived: u64,
}

impl InvoicesService {
    /// Ghi bản lưu trữ kế toán của khách `tenant` (`reason`: `manual` hay `tenant_deleted`).
    pub(crate) async fn archive_tenant(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        reason: &str,
    ) -> Result<archive::Model, Error> {
        let store = self.storage.clone().ok_or_else(storage_unavailable)?;
        let id = Uuid::now_v7();
        let prefix = format!("archives/{}/{id}/", tenant.id().as_uuid());
        let mut files: Vec<Value> = Vec::new();
        let mut put = async |path: &str, bytes: Vec<u8>, rows: usize| -> Result<(), Error> {
            let sha = hex(&Sha256::digest(&bytes));
            let size = bytes.len();
            let content =
                Box::pin(futures_util::stream::once(async move { Ok(Bytes::from(bytes)) }));
            store.put(&format!("{prefix}{path}"), content).await?;
            files.push(json!({"path": path, "sha256": sha, "size": size, "rows": rows}));
            Ok(())
        };
        // Sổ nhật ký chung: mọi ngày.
        let (first, last) = (
            Date::new(1900, 1, 1).unwrap_or_default(),
            Date::new(9999, 12, 31).unwrap_or_default(),
        );
        let lines = self.journal_of(tenant, None, first, last).await?;
        let rows = journal_rows(&lines, &Default::default());
        put("ledger/journal.csv", to_csv(&JOURNAL_HEADER, &rows), rows.len()).await?;
        // Chứng từ: mỗi chứng từ một dòng JSON, đọc theo trang.
        let mut jsonl = Vec::new();
        let (mut count, mut retain_until) = (0usize, None::<Date>);
        let mut page = PageRequest::first(200, Order::Asc);
        loop {
            let found = fetch_page(
                document::Entity::find().scoped(tenant),
                document::Column::Id,
                |row| row.id,
                &page,
                tenant.database().read(),
            )
            .await?;
            for row in found.data {
                retain_until = retain_until.max(row.retention_until);
                let described = describe(tenant.database().read(), tenant, row).await?;
                serde_json::to_writer(&mut jsonl, &described).map_err(Error::internal)?;
                jsonl.push(b'\n');
                count += 1;
            }
            match found.pagination.next_cursor.and_then(|cursor| cursor.parse::<Uuid>().ok()) {
                Some(cursor) => page = PageRequest::first(200, Order::Asc).after(cursor),
                None => break,
            }
        }
        put("documents.jsonl", jsonl, count).await?;
        // Giữ bản lưu ít nhất tới hết hạn của chứng từ muộn nhất, hay 10 năm từ nay.
        let floor = Date::new(Timestamp::now().date_utc().year() + 10, 12, 31).unwrap_or_default();
        let retain_until = retain_until.unwrap_or(floor).max(floor);
        let manifest = json!({
            "format": "autoxide-accounting-archive",
            "version": 1,
            "tenant_id": tenant.id().as_uuid(),
            "archive_id": id,
            "created_at": now(),
            "retain_until": retain_until,
            "legal_basis": "Accounting Law 88/2015/QH13 Article 41; Decree 174/2016/ND-CP Article 12",
            "files": files,
        });
        let manifest_bytes = serde_json::to_vec_pretty(&manifest).map_err(Error::internal)?;
        let manifest_sha = hex(&Sha256::digest(&manifest_bytes));
        let manifest_content =
            Box::pin(futures_util::stream::once(async move { Ok(Bytes::from(manifest_bytes)) }));
        store.put(&format!("{prefix}manifest.json"), manifest_content).await?;
        let transaction = tenant.database().begin_write().await?;
        let at = now();
        let saved = archive::Entity::insert(archive::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            status: Set("ready".to_owned()),
            reason: Set(reason.to_owned()),
            storage_prefix: Set(prefix),
            manifest_sha256: Set(Some(manifest_sha)),
            files: Set(Some(manifest["files"].to_string())),
            retain_until: Set(Some(retain_until)),
            error: Set(None),
            created_by: Set(actor.user_id),
            created_at: Set(at),
            updated_at: Set(at),
        })
        .exec_with_returning(&transaction)
        .await?;
        transaction.commit().await?;
        tracing::info!(tenant_id = %tenant.id(), archive_id = %id, documents = count, "accounting archive written");
        Ok(saved)
    }

    /// Bản lưu trữ qua REST / MCP (`invoices.archive`).
    pub(crate) async fn archive_manual(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
    ) -> Result<archive::Model, Error> {
        principal.require(&ARCHIVE)?;
        self.archive_tenant(tenant, Actor::from(principal), "manual").await
    }

    pub(crate) async fn list_archives(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<archive::Model>, Error> {
        principal.require(&ARCHIVE)?;
        let select = archive::Entity::find().scoped(tenant);
        fetch_page(select, archive::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Một lượt dọn chứng từ hết hạn lưu trên `database` (mọi khách), tính tới ngày `today`.
    pub(crate) async fn purge_expired(
        &self,
        database: &Database,
        today: Date,
    ) -> Result<Purged, Error> {
        let mut report = Purged::default();
        // 1. Ẩn danh người mua cá nhân của chứng từ quá hạn lưu (index `…_retention_sweep_idx`).
        let expired: Vec<document::Model> = document::Entity::find()
            .filter(document::Column::RetentionUntil.lt(today))
            .filter(document::Column::BuyerPurgedAt.is_null())
            .order_by_asc(document::Column::RetentionUntil)
            .limit(BATCH)
            .all(database.read())
            .await?;
        if !expired.is_empty() {
            let transaction = database.begin_write().await?;
            let at = now();
            for row in expired {
                let mut update = document::Entity::update_many()
                    .col_expr(document::Column::BuyerPurgedAt, Expr::value(at))
                    .filter(document::Column::Id.eq(row.id))
                    .filter(document::Column::TenantId.eq(row.tenant_id));
                if let Some(buyer) = anonymized(&row.buyer) {
                    update = update.col_expr(document::Column::Buyer, Expr::value(buyer));
                    report.anonymized += 1;
                }
                update.exec(&transaction).await?;
            }
            transaction.commit().await?;
        }
        // 2. Khách đã xoá hẳn: bản lưu trữ (nếu chưa có), rồi xoá chứng từ quá hạn và nháp.
        let tenants: Vec<Uuid> = document::Entity::find()
            .select_only()
            .column(document::Column::TenantId)
            .distinct()
            .limit(BATCH)
            .into_tuple()
            .all(database.read())
            .await?;
        for tenant_id in tenants {
            let tenant = match self.databases.tenant(TenantId::from_uuid(tenant_id)).await {
                Ok(tenant) if tenant.status() == TenantStatus::Deleted => tenant,
                _ => continue,
            };
            let archived = archive::Entity::find()
                .scoped(&tenant)
                .filter(archive::Column::Status.eq("ready"))
                .one(tenant.database().read())
                .await?;
            if archived.is_none() {
                match self.archive_tenant(&tenant, Actor::system(), "tenant_deleted").await {
                    Ok(_) => report.archived += 1,
                    Err(error) => {
                        // Không xoá gì khi chưa có bản lưu; lượt sau thử lại.
                        tracing::warn!(tenant_id = %tenant_id, error = %error, "cannot archive a deleted tenant");
                        continue;
                    }
                }
            }
            report.deleted += self.delete_expired(&tenant, today).await?;
        }
        Ok(report)
    }

    /// Xoá chứng từ đã quá hạn lưu (và nháp) của khách đã xoá hẳn `tenant`. Chứng từ còn trong hạn
    /// không bao giờ bị chạm.
    async fn delete_expired(&self, tenant: &TenantContext, today: Date) -> Result<u64, Error> {
        let doomed: Vec<Uuid> = document::Entity::find()
            .scoped(tenant)
            .filter(
                Condition::any()
                    .add(document::Column::RetentionUntil.lt(today))
                    .add(document::Column::Status.eq("draft")),
            )
            .select_only()
            .column(document::Column::Id)
            .limit(BATCH)
            .into_tuple()
            .all(tenant.database().read())
            .await?;
        if doomed.is_empty() {
            return Ok(0);
        }
        let transaction = tenant.database().begin_write().await?;
        line::Entity::delete_many()
            .filter(line::Column::TenantId.eq(tenant.id().as_uuid()))
            .filter(line::Column::DocumentId.is_in(doomed.clone()))
            .exec(&transaction)
            .await?;
        let deleted = document::Entity::delete_many()
            .filter(document::Column::TenantId.eq(tenant.id().as_uuid()))
            .filter(document::Column::Id.is_in(doomed))
            // Chặn lần cuối trong cùng câu: không xoá chứng từ còn hạn dù danh sách đọc trước đó cũ.
            .filter(
                Condition::any()
                    .add(document::Column::RetentionUntil.lt(today))
                    .add(document::Column::Status.eq("draft")),
            )
            .exec(&transaction)
            .await?
            .rows_affected;
        transaction.commit().await?;
        Ok(deleted)
    }
}

/// Job `invoices.purge_expired` (hằng ngày, mọi database).
pub(crate) struct PurgeExpiredJob(pub Arc<InvoicesService>);

#[async_trait::async_trait]
impl JobHandler for PurgeExpiredJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let report = self.0.purge_expired(&job.database, Timestamp::now().date_utc()).await?;
        if report != Purged::default() {
            tracing::info!(
                anonymized = report.anonymized,
                deleted = report.deleted,
                archived = report.archived,
                "expired accounting documents processed"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn chi_an_danh_nguoi_mua_ca_nhan() {
        let consumer = r#"{"kind":"consumer","name":"Nguyễn Văn A","email":"a@example.com"}"#;
        assert_eq!(super::anonymized(consumer).unwrap(), r#"{"kind":"consumer"}"#);
        let business = r#"{"kind":"business","name":"Công ty B","tax_code":"0101234567"}"#;
        assert!(super::anonymized(business).is_none());
    }
}
