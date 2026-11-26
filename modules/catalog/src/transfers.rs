//! Nhập / xuất sản phẩm bằng CSV, chạy bằng job (D-223…D-227).
//!
//! - **Nhập**: `POST /imports` lưu file vào `dyn Storage` của khách (`catalog/imports/<id>.csv`),
//!   ghi một dòng `catalog_transfers` (`queued`) và xếp job [`IMPORT`] trong CÙNG transaction. Job
//!   chạy với quyền của người yêu cầu ([`PrincipalSnapshot`]) và đi qua đúng service của REST, nên
//!   cùng kiểm tra, cùng hạn mức, cùng event. Mỗi dòng một transaction ngắn: dòng sai ghi vào
//!   `errors` (`row`, `field`, `code`, `message`), không chặn dòng đúng. Khớp theo SKU: SKU đã có thì
//!   sửa nội dung / mã vạch / cân nặng, chưa có thì tạo sản phẩm **nháp** một biến thể — nhập lại
//!   cùng file không nhân đôi. Không bao giờ mở bán từ file.
//! - **Xuất**: `POST /exports` → job [`EXPORT`] ghi mọi biến thể của sản phẩm trong phạm vi người
//!   yêu cầu, cùng cột với file nhập (xuất ra nhập lại được), bằng `autoxide_kernel::csv` (chặn CSV
//!   injection: ô bắt đầu bằng `=`, `+`, `-`, `@` được thêm `'`). Đọc lại bỏ đúng dấu `'` đó.
//! - Trạng thái chỉ đi một chiều: `queued` → `running` → `succeeded` / `succeeded_with_errors` /
//!   `rejected` (file sai định dạng hay quá giới hạn — không dòng nào được ghi) / `failed` (lỗi hệ
//!   thống). Job chạy lại sau khi đã xong thì không làm gì.
//! - Lịch [`CLEANUP`] mỗi ngày xoá file của lần nhập / xuất cũ hơn [`KEEP_DAYS`] ngày.

use jiff::SignedDuration;
use std::collections::HashMap;
use std::sync::Arc;

use autoxide_kernel::access::{Principal, PrincipalSnapshot};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use autoxide_kernel::storage::{ByteStream, Storage, StoredFile, TenantStorage};
use autoxide_kernel::tenancy::{OrgId, OrgTree, ScopeExt, TenantContext, TenantId};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::api::ProductKind;
use crate::entity::{product, transfer, translation, variant};
use crate::events::{Change, ExportCompleted, ImportCompleted};
use crate::permissions::{EXPORT as EXPORT_PERMISSION, IMPORT as IMPORT_PERMISSION, PRODUCTS_READ};
use crate::products::{NewProduct, ProductChanges, TextInput, kind_str, parse_kind};
use crate::service::{CatalogService, now};
use crate::variants::{NewVariant, VariantChanges, sku_key};

/// Job nhập một file.
pub(crate) const IMPORT: &str = "catalog.import";
/// Job xuất một file.
pub(crate) const EXPORT: &str = "catalog.export";
/// Lịch mỗi ngày: dọn file cũ.
pub(crate) const CLEANUP: &str = "catalog.cleanup_transfers";
/// Số dòng tối đa của một file nhập (không kể dòng tiêu đề).
pub(crate) const MAX_IMPORT_ROWS: usize = 50_000;
/// Cỡ tối đa của một file nhập.
pub(crate) const MAX_IMPORT_BYTES: usize = 20 * 1024 * 1024;
/// Số lỗi theo dòng giữ lại.
const MAX_ERRORS: usize = 1_000;
/// Giữ file nhập / xuất bao nhiêu ngày.
pub(crate) const KEEP_DAYS: i64 = 7;
/// Số sản phẩm mỗi trang khi xuất.
const EXPORT_PAGE: u64 = 200;

/// Cột của file (cả nhập và xuất). Chỉ `sku` bắt buộc; sản phẩm mới cần thêm `name` và
/// `tax_category`.
pub(crate) const COLUMNS: [&str; 9] = [
    "sku",
    "name",
    "locale",
    "short_description",
    "description",
    "kind",
    "tax_category",
    "barcode",
    "weight_grams",
];

/// Trạng thái của một lần nhập / xuất.
pub(crate) mod status {
    pub(crate) const QUEUED: &str = "queued";
    pub(crate) const RUNNING: &str = "running";
    pub(crate) const SUCCEEDED: &str = "succeeded";
    pub(crate) const WITH_ERRORS: &str = "succeeded_with_errors";
    pub(crate) const REJECTED: &str = "rejected";
    pub(crate) const FAILED: &str = "failed";
}

/// Payload của job: lần nào, chạy với quyền của ai.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TransferJob {
    pub transfer_id: Uuid,
    pub principal: PrincipalSnapshot,
}

/// Job nhập.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct ImportJob(pub TransferJob);

impl Job for ImportJob {
    const KIND: &'static str = IMPORT;
}

/// Job xuất.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct ExportJob(pub TransferJob);

impl Job for ExportJob {
    const KIND: &'static str = EXPORT;
}

/// Một lỗi của một dòng (dòng 2 là dòng dữ liệu đầu tiên, sau tiêu đề).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RowError {
    pub row: usize,
    pub field: String,
    pub code: String,
    pub message: String,
}

fn transfer_not_found(id: Uuid) -> Error {
    Error::not_found("catalog.transfer_not_found", "There is no import or export `{id}` of yours.")
        .with_param("id", id)
}

pub(crate) fn storage_unavailable() -> Error {
    Error::unavailable(
        "catalog.storage_unavailable",
        "Imports, exports and digital products need file storage: install module `files`.",
    )
}

fn import_too_large() -> Error {
    Error::validation(vec![
        FieldError::new("file", "size", "must be at most {max} bytes")
            .with_param("max", MAX_IMPORT_BYTES),
    ])
}

/// Bỏ dấu `'` mà bộ ghi CSV thêm trước ô bắt đầu bằng ký tự công thức (`'=1` → `=1`).
fn unguard(cell: &str) -> &str {
    match cell.strip_prefix('\'') {
        Some(rest) if rest.starts_with(['=', '+', '-', '@', '\t', '\r']) => rest,
        _ => cell,
    }
}

impl CatalogService {
    fn store(&self, tenant: &TenantContext) -> Result<TenantStorage, Error> {
        let storage = self.storage.clone().ok_or_else(storage_unavailable)?;
        Ok(TenantStorage::new(storage, tenant))
    }

    /// Bắt đầu nhập `content` (CSV UTF-8). `org_id`: nút nhận sản phẩm mới.
    pub(crate) async fn start_import(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org_id: Option<Uuid>,
        content: bytes::Bytes,
    ) -> Result<transfer::Model, Error> {
        principal.require(&IMPORT_PERMISSION)?;
        if content.len() > MAX_IMPORT_BYTES {
            return Err(import_too_large());
        }
        // Nút nhận phải là nút của khách (PT-8); phạm vi của người gọi kiểm lúc job tạo sản phẩm.
        if let Some(org) = org_id {
            OrgTree::ensure_exists(tenant.database().read(), tenant.id(), OrgId::from_uuid(org))
                .await?;
        }
        let store = self.store(tenant)?;
        let id = Uuid::now_v7();
        let key = format!("catalog/imports/{id}.csv");
        let stream: ByteStream =
            Box::pin(futures_util::stream::once(async move { Ok::<_, Error>(content) }));
        store.put(&key, stream).await?;
        self.queue_transfer(tenant, principal, id, "import", org_id, Some(key)).await
    }

    /// Bắt đầu xuất sản phẩm trong phạm vi người yêu cầu (`org_id`: chỉ nhánh này).
    pub(crate) async fn start_export(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org_id: Option<Uuid>,
    ) -> Result<transfer::Model, Error> {
        principal.require(&EXPORT_PERMISSION)?;
        if let Some(org) = org_id {
            OrgTree::ensure_exists(tenant.database().read(), tenant.id(), OrgId::from_uuid(org))
                .await?;
        }
        self.store(tenant)?;
        self.queue_transfer(tenant, principal, Uuid::now_v7(), "export", org_id, None).await
    }

    async fn queue_transfer(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        kind: &str,
        org_id: Option<Uuid>,
        file_key: Option<String>,
    ) -> Result<transfer::Model, Error> {
        let row = transfer::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            kind: Set(kind.to_owned()),
            status: Set(status::QUEUED.to_owned()),
            org_id: Set(org_id),
            file_key: Set(file_key),
            total_rows: Set(0),
            ok_rows: Set(0),
            errors: Set("[]".to_owned()),
            created_by: Set(Some(principal.user_id())),
            created_at: Set(now()),
            finished_at: Set(None),
        };
        let job = TransferJob { transfer_id: id, principal: principal.snapshot() };
        let transaction = tenant.database().begin_write().await?;
        let saved = row.insert(&transaction).await?;
        let options = EnqueueOptions::default().max_attempts(1);
        if kind == "import" {
            self.jobs.enqueue(&transaction, tenant, &ImportJob(job), options).await?;
        } else {
            self.jobs.enqueue(&transaction, tenant, &ExportJob(job), options).await?;
        }
        transaction.commit().await?;
        Ok(saved)
    }

    /// Lần nhập / xuất `id` của chính người gọi (`kind`: `import` hay `export`).
    pub(crate) async fn get_transfer(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        kind: &str,
        id: Uuid,
    ) -> Result<transfer::Model, Error> {
        let permission = if kind == "import" { &IMPORT_PERMISSION } else { &EXPORT_PERMISSION };
        principal.require(permission)?;
        transfer::Entity::find_by_id(id)
            .filter(transfer::Column::Kind.eq(kind))
            .filter(transfer::Column::CreatedBy.eq(principal.user_id()))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| transfer_not_found(id))
    }

    /// File CSV của lần xuất `id` đã xong.
    pub(crate) async fn export_content(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<StoredFile, Error> {
        let row = self.get_transfer(tenant, principal, "export", id).await?;
        let key = row
            .file_key
            .filter(|_| row.status == status::SUCCEEDED)
            .ok_or_else(|| transfer_not_found(id))?;
        self.store(tenant)?.get(&key).await?.ok_or_else(|| transfer_not_found(id))
    }

    /// Chuyển lần `id` sang `running`; `None` khi nó không còn ở `queued` (job chạy lại).
    async fn begin_transfer(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<Option<transfer::Model>, Error> {
        let updated = transfer::Entity::update_many()
            .col_expr(transfer::Column::Status, sea_orm::sea_query::Expr::value(status::RUNNING))
            .filter(transfer::Column::Id.eq(id))
            .filter(transfer::Column::Status.eq(status::QUEUED))
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        if updated.rows_affected == 0 {
            return Ok(None);
        }
        Ok(transfer::Entity::find_by_id(id).scoped(tenant).one(tenant.database().write()).await?)
    }

    /// Ghi kết quả cuối và event `catalog.import_completed` / `catalog.export_completed`.
    async fn finish_transfer(
        &self,
        tenant: &TenantContext,
        row: transfer::Model,
        outcome: Outcome,
    ) -> Result<(), Error> {
        let actor = row.created_by.map_or_else(Actor::system, Actor::user);
        let kind = row.kind.clone();
        let mut edit = row.into_active_model();
        edit.status = Set(outcome.status.to_owned());
        edit.total_rows = Set(i64::try_from(outcome.total).unwrap_or(i64::MAX));
        edit.ok_rows = Set(i64::try_from(outcome.ok).unwrap_or(i64::MAX));
        edit.errors = Set(serde_json::to_string(&outcome.errors).map_err(Error::internal)?);
        if let Some(key) = outcome.file_key {
            edit.file_key = Set(Some(key));
        }
        edit.finished_at = Set(Some(now()));
        let transaction = tenant.database().begin_write().await?;
        let saved = edit.update(&transaction).await?;
        let after = json!({
            "transfer_id": saved.id, "status": saved.status, "total_rows": saved.total_rows,
            "ok_rows": saved.ok_rows, "error_count": outcome.errors.len(),
        });
        let change = Change { before: None, after: Some(after) };
        let subject = Some(format!("catalog.transfer:{}", saved.id));
        if kind == "import" {
            self.outbox
                .record(&transaction, tenant, actor, subject, &ImportCompleted(change))
                .await?;
        } else {
            self.outbox
                .record(&transaction, tenant, actor, subject, &ExportCompleted(change))
                .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Đọc và kiểm cả file; lỗi định dạng thì từ chối cả file.
    fn parse_import(text: &str) -> Result<(Vec<String>, Vec<Vec<String>>), RowError> {
        let reject = |message: String| RowError {
            row: 0,
            field: "file".to_owned(),
            code: "format".to_owned(),
            message,
        };
        let mut rows =
            autoxide_kernel::csv::read(text, MAX_IMPORT_ROWS + 1).map_err(|error| match error {
                autoxide_kernel::csv::CsvError::TooManyRows { .. } => RowError {
                    row: 0,
                    field: "file".to_owned(),
                    code: "catalog.import_too_many_rows".to_owned(),
                    message: format!("The file has more than {MAX_IMPORT_ROWS} rows."),
                },
                other => reject(other.to_string()),
            })?;
        if rows.is_empty() {
            return Err(reject("the file is empty".to_owned()));
        }
        let header: Vec<String> =
            rows.remove(0).iter().map(|cell| cell.trim().to_ascii_lowercase()).collect();
        if let Some(unknown) = header.iter().find(|name| !COLUMNS.contains(&name.as_str())) {
            return Err(reject(format!(
                "unknown column `{unknown}`; the columns are {}",
                COLUMNS.join(", ")
            )));
        }
        if !header.iter().any(|name| name == "sku") {
            return Err(reject("the column `sku` is required".to_owned()));
        }
        Ok((header, rows))
    }

    /// Chạy lần nhập `job`.
    async fn run_import(&self, tenant: &TenantContext, job: &TransferJob) -> Result<(), Error> {
        let Some(row) = self.begin_transfer(tenant, job.transfer_id).await? else {
            return Ok(());
        };
        let principal = job.principal.restore();
        let outcome = match self.import_file(tenant, &principal, &row).await {
            Ok(outcome) => outcome,
            Err(error) => {
                tracing::warn!(transfer_id = %row.id, code = error.code(), "catalog import failed");
                Outcome::failed(&error)
            }
        };
        self.finish_transfer(tenant, row, outcome).await
    }

    async fn import_file(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        row: &transfer::Model,
    ) -> Result<Outcome, Error> {
        let key = row.file_key.clone().ok_or_else(|| Error::internal("import without a file"))?;
        let store = self.store(tenant)?;
        let file = store.get(&key).await?.ok_or_else(|| Error::internal("import file is gone"))?;
        let bytes = collect(file.content).await?;
        let Ok(text) = String::from_utf8(bytes) else {
            return Ok(Outcome::rejected(RowError {
                row: 0,
                field: "file".to_owned(),
                code: "format".to_owned(),
                message: "the file is not UTF-8 text".to_owned(),
            }));
        };
        let (header, rows) = match Self::parse_import(&text) {
            Ok(parsed) => parsed,
            Err(error) => return Ok(Outcome::rejected(error)),
        };
        let mut outcome = Outcome { status: status::SUCCEEDED, ..Outcome::default() };
        for (index, cells) in rows.iter().enumerate() {
            // Dòng 1 là tiêu đề: dòng dữ liệu đầu tiên là dòng 2.
            let number = index + 2;
            let mut values: HashMap<&str, &str> = HashMap::new();
            for (name, cell) in header.iter().zip(cells) {
                let cell = unguard(cell.trim());
                if !cell.is_empty() {
                    values.insert(name.as_str(), cell);
                }
            }
            if values.is_empty() {
                continue;
            }
            outcome.total += 1;
            match self.import_row(tenant, principal, row.org_id, &values).await {
                Ok(()) => outcome.ok += 1,
                Err(error) => outcome.push(number, &error),
            }
        }
        if !outcome.errors.is_empty() || outcome.dropped > 0 {
            outcome.status = status::WITH_ERRORS;
        }
        Ok(outcome)
    }

    /// Một dòng: SKU đã có thì sửa, chưa có thì tạo sản phẩm nháp.
    async fn import_row(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org_id: Option<Uuid>,
        values: &HashMap<&str, &str>,
    ) -> Result<(), Error> {
        let field = |name: &str| values.get(name).map(|value| (*value).to_owned());
        let sku = field("sku").ok_or_else(|| {
            Error::validation(vec![FieldError::new("sku", "required", "must not be empty")])
        })?;
        let weight = match values.get("weight_grams") {
            Some(raw) => Some(raw.parse::<i64>().map_err(|_| {
                Error::validation(vec![FieldError::new(
                    "weight_grams",
                    "format",
                    "must be a whole number of grams",
                )])
            })?),
            None => None,
        };
        let kind = match values.get("kind") {
            Some(raw) if ["physical", "digital", "service", "bundle"].contains(raw) => {
                Some(parse_kind(raw))
            }
            Some(_) => {
                return Err(Error::validation(vec![FieldError::new(
                    "kind",
                    "one_of",
                    "must be physical, digital, service or bundle",
                )]));
            }
            None => None,
        };
        let locale = match values.get("locale") {
            Some(raw) => self.locale(tenant, raw).await?,
            None => self.locales(tenant).await?.first().cloned().unwrap_or_else(|| "en".to_owned()),
        };
        let existing = variant::Entity::find()
            .filter(variant::Column::SkuKey.eq(sku_key(&sku)))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        let Some(found) = existing else {
            let name = field("name").ok_or_else(|| {
                Error::validation(vec![FieldError::new(
                    "name",
                    "required",
                    "is required for a new product",
                )])
            })?;
            let tax_category = field("tax_category").ok_or_else(|| {
                Error::validation(vec![FieldError::new(
                    "tax_category",
                    "required",
                    "is required for a new product",
                )])
            })?;
            let input = NewProduct {
                org_id,
                kind: kind.unwrap_or(ProductKind::Physical),
                tax_category,
                locale,
                text: TextInput {
                    name,
                    short_description: field("short_description").unwrap_or_default(),
                    description: field("description").unwrap_or_default(),
                    slug_generated: true,
                    ..TextInput::default()
                },
                warranty_months: None,
                lot_tracked: false,
                serial_tracked: false,
                category_ids: Vec::new(),
                variant: NewVariant {
                    sku,
                    barcode: field("barcode"),
                    weight_grams: weight,
                    ..NewVariant::default()
                },
            };
            self.create_product(tenant, principal, input).await?;
            return Ok(());
        };
        let product_id = found.product_id;
        if let Some(kind) = kind {
            let current = product::Entity::find_by_id(product_id)
                .scoped(tenant)
                .one(tenant.database().read())
                .await?;
            if current.is_some_and(|row| row.kind != kind_str(kind)) {
                return Err(Error::validation(vec![FieldError::new(
                    "kind",
                    "immutable",
                    "cannot change the kind of an existing product",
                )]));
            }
        }
        if let Some(tax_category) = field("tax_category") {
            let changes = ProductChanges { tax_category: Some(tax_category), ..Default::default() };
            self.update_product(tenant, principal, product_id, changes, None).await?;
        }
        if values.contains_key("name")
            || values.contains_key("short_description")
            || values.contains_key("description")
        {
            let old = translation::Entity::find()
                .filter(translation::Column::ProductId.eq(product_id))
                .filter(translation::Column::Locale.eq(locale.as_str()))
                .scoped(tenant)
                .one(tenant.database().read())
                .await?;
            let text = TextInput {
                name: field("name")
                    .or_else(|| old.as_ref().map(|row| row.name.clone()))
                    .unwrap_or_default(),
                // Nhập lại không đổi đường dẫn công khai đã có.
                slug: old.as_ref().map(|row| row.slug.clone()),
                short_description: field("short_description")
                    .or_else(|| old.as_ref().map(|row| row.short_description.clone()))
                    .unwrap_or_default(),
                description: field("description")
                    .or_else(|| old.as_ref().map(|row| row.description.clone()))
                    .unwrap_or_default(),
                meta_title: old.as_ref().and_then(|row| row.meta_title.clone()),
                meta_description: old.as_ref().and_then(|row| row.meta_description.clone()),
                slug_generated: old.is_none(),
            };
            self.put_translation(tenant, principal, product_id, &locale, text).await?;
        }
        if values.contains_key("barcode") || weight.is_some() {
            let changes = VariantChanges {
                barcode: values.contains_key("barcode").then(|| field("barcode")),
                weight_grams: weight.map(Some),
                ..VariantChanges::default()
            };
            self.update_variant(tenant, principal, found.id, changes, None).await?;
        }
        Ok(())
    }

    /// Chạy lần xuất `job`.
    async fn run_export(&self, tenant: &TenantContext, job: &TransferJob) -> Result<(), Error> {
        let Some(row) = self.begin_transfer(tenant, job.transfer_id).await? else {
            return Ok(());
        };
        let principal = job.principal.restore();
        let outcome = match self.export_file(tenant, &principal, &row).await {
            Ok(outcome) => outcome,
            Err(error) => {
                tracing::warn!(transfer_id = %row.id, code = error.code(), "catalog export failed");
                Outcome::failed(&error)
            }
        };
        self.finish_transfer(tenant, row, outcome).await
    }

    async fn export_file(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        row: &transfer::Model,
    ) -> Result<Outcome, Error> {
        principal.require(&PRODUCTS_READ)?;
        let read = tenant.database().read();
        let header: Vec<String> = COLUMNS.iter().map(|name| (*name).to_owned()).collect();
        let mut out = autoxide_kernel::csv::write(&header, &[]);
        let mut total = 0usize;
        let mut after: Option<Uuid> = None;
        loop {
            let mut query = product::Entity::find()
                .filter(principal.scope_condition(&PRODUCTS_READ, product::Column::OrgId, None)?)
                .scoped(tenant);
            if let Some(org) = row.org_id {
                query = query.filter(autoxide_kernel::tenancy::OrgTree::subtree_condition(
                    tenant.id(),
                    &[OrgId::from_uuid(org)],
                    product::Column::OrgId,
                ));
            }
            if let Some(last) = after {
                query = query.filter(product::Column::Id.gt(last));
            }
            let products =
                query.order_by_asc(product::Column::Id).limit(EXPORT_PAGE).all(read).await?;
            let Some(last) = products.last() else { break };
            after = Some(last.id);
            let ids: Vec<Uuid> = products.iter().map(|row| row.id).collect();
            let texts: HashMap<(Uuid, String), translation::Model> = translation::Entity::find()
                .filter(translation::Column::ProductId.is_in(ids.clone()))
                .scoped(tenant)
                .all(read)
                .await?
                .into_iter()
                .map(|row| ((row.product_id, row.locale.clone()), row))
                .collect();
            let variants = variant::Entity::find()
                .filter(variant::Column::ProductId.is_in(ids))
                .scoped(tenant)
                .order_by_asc(variant::Column::ProductId)
                .order_by_asc(variant::Column::Position)
                .all(read)
                .await?;
            let by_id: HashMap<Uuid, &product::Model> =
                products.iter().map(|row| (row.id, row)).collect();
            for item in &variants {
                let Some(owner) = by_id.get(&item.product_id) else { continue };
                let text = texts.get(&(owner.id, owner.default_locale.clone()));
                let cells = vec![
                    item.sku.clone(),
                    text.map(|row| row.name.clone()).unwrap_or_default(),
                    owner.default_locale.clone(),
                    text.map(|row| row.short_description.clone()).unwrap_or_default(),
                    text.map(|row| row.description.clone()).unwrap_or_default(),
                    owner.kind.clone(),
                    owner.tax_category.clone(),
                    item.barcode.clone().unwrap_or_default(),
                    item.weight_grams.map(|grams| grams.to_string()).unwrap_or_default(),
                ];
                out.push_str(&autoxide_kernel::csv::line(&cells));
                total += 1;
            }
        }
        let key = format!("catalog/exports/{}.csv", row.id);
        let content = bytes::Bytes::from(out);
        let stream: ByteStream =
            Box::pin(futures_util::stream::once(async move { Ok::<_, Error>(content) }));
        self.store(tenant)?.put(&key, stream).await?;
        Ok(Outcome {
            status: status::SUCCEEDED,
            total,
            ok: total,
            file_key: Some(key),
            ..Outcome::default()
        })
    }
}

/// Kết quả của một lần nhập / xuất.
#[derive(Debug, Default)]
struct Outcome {
    status: &'static str,
    total: usize,
    ok: usize,
    errors: Vec<RowError>,
    /// Lỗi vượt [`MAX_ERRORS`], không giữ chi tiết.
    dropped: usize,
    file_key: Option<String>,
}

impl Outcome {
    fn rejected(error: RowError) -> Self {
        Outcome { status: status::REJECTED, errors: vec![error], ..Outcome::default() }
    }

    fn failed(error: &Error) -> Self {
        let error = RowError {
            row: 0,
            field: String::new(),
            code: error.code().to_owned(),
            message: error.message(),
        };
        Outcome { status: status::FAILED, errors: vec![error], ..Outcome::default() }
    }

    /// Lỗi của dòng `row`: mỗi lỗi trường một mục; lỗi không theo trường thì một mục.
    fn push(&mut self, row: usize, error: &Error) {
        let mut found: Vec<RowError> = error
            .fields()
            .iter()
            .map(|field| RowError {
                row,
                field: field.field.clone(),
                code: field.code.clone(),
                message: field.message.clone(),
            })
            .collect();
        if found.is_empty() {
            found.push(RowError {
                row,
                field: String::new(),
                code: error.code().to_owned(),
                message: error.message(),
            });
        }
        for item in found {
            if self.errors.len() < MAX_ERRORS {
                self.errors.push(item);
            } else {
                self.dropped += 1;
            }
        }
    }
}

/// Đọc hết luồng byte (file nhập đã giới hạn [`MAX_IMPORT_BYTES`]).
async fn collect(mut stream: ByteStream) -> Result<Vec<u8>, Error> {
    use futures_util::StreamExt as _;
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        bytes.extend_from_slice(&chunk?);
        if bytes.len() > MAX_IMPORT_BYTES {
            return Err(import_too_large());
        }
    }
    Ok(bytes)
}

/// Bộ chạy job [`IMPORT`] và [`EXPORT`].
pub(crate) struct Transfers(pub(crate) Arc<CatalogService>);

#[async_trait::async_trait]
impl JobHandler for Transfers {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let tenant = job
            .tenant
            .as_ref()
            .ok_or_else(|| Error::internal("a catalog transfer job must belong to a tenant"))?;
        let input: TransferJob =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        if job.kind == IMPORT {
            self.0.run_import(tenant, &input).await
        } else {
            self.0.run_export(tenant, &input).await
        }
    }
}

/// Lịch [`CLEANUP`]: xoá file của lần nhập / xuất cũ hơn [`KEEP_DAYS`] ngày (dòng vẫn giữ, làm
/// nhật ký).
pub(crate) struct Cleanup(pub(crate) Arc<CatalogService>);

#[async_trait::async_trait]
impl JobHandler for Cleanup {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let Some(storage) = self.0.storage.clone() else { return Ok(()) };
        let before = now() - SignedDuration::from_hours(24 * KEEP_DAYS);
        // tenant-scope: việc nền / kiểm tra quét mọi khách của database, chủ ý không lọc khách (rà soát bảo mật 2026-11)
        let rows = transfer::Entity::find()
            .filter(transfer::Column::CreatedAt.lt(before))
            .filter(transfer::Column::FileKey.is_not_null())
            .limit(1_000)
            .all(job.database.read())
            .await?;
        for row in rows {
            let tenant = self.0.databases.tenant(TenantId::from_uuid(row.tenant_id)).await?;
            if let Some(key) = &row.file_key {
                TenantStorage::new(storage.clone(), &tenant).delete(key).await?;
            }
            let mut edit = row.into_active_model();
            edit.file_key = Set(None);
            edit.update(tenant.database().write()).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bo_dau_chan_cong_thuc_khi_doc_lai() {
        assert_eq!(unguard("'=SUM(A1)"), "=SUM(A1)");
        assert_eq!(unguard("'-5"), "-5");
        assert_eq!(unguard("'abc"), "'abc");
        assert_eq!(unguard("abc"), "abc");
    }

    #[test]
    fn tu_choi_cot_la_va_thieu_sku() {
        let (header, rows) = CatalogService::parse_import("sku,name\r\nA-1,Áo\r\n").unwrap();
        assert_eq!(header, vec!["sku", "name"]);
        assert_eq!(rows, vec![vec!["A-1".to_owned(), "Áo".to_owned()]]);
        assert!(CatalogService::parse_import("name,price\nÁo,1\n").is_err());
        assert!(CatalogService::parse_import("name\nÁo\n").is_err());
        assert!(CatalogService::parse_import("").is_err());
    }
}
