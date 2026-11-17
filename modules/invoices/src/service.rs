//! Service của module `invoices`: lỗi, mô tả chứng từ, ký hiệu, uỷ nhiệm. Chứng từ nháp ở
//! [`crate::documents`], phát hành / gửi hoá đơn điện tử / sửa sai ở [`crate::issue`].

use std::sync::Arc;

use autoxide_kernel::access::{OrgTarget, Permission, Principal};
use autoxide_kernel::einvoice::EInvoiceProvider;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Outbox;
use autoxide_kernel::job::JobQueue;
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::{DatabaseRegistry, OrgId, ScopeExt, TenantContext};
use autoxide_ledger::api::Ledger;
use autoxide_tax::api::TaxEngine;
use chrono::{DateTime, Datelike, NaiveDate, SubsecRound, Utc};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set};

use crate::api::{Buyer, Document, DocumentLine, KINDS};
use crate::entity::{document, line, mandate, series};
use crate::permissions::{MANDATES_MANAGE, READ, SERIES_MANAGE};

pub(crate) fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

pub(crate) struct InvoicesService {
    pub databases: DatabaseRegistry,
    pub outbox: Outbox,
    pub jobs: JobQueue,
    pub tax: Arc<dyn TaxEngine>,
    pub ledger: Arc<dyn Ledger>,
    pub providers: Vec<Arc<dyn EInvoiceProvider>>,
}

// ---- Lỗi ----------------------------------------------------------------------------------------

pub(crate) fn document_not_found() -> Error {
    Error::not_found("invoices.document_not_found", "There is no such document.")
}

pub(crate) fn not_draft() -> Error {
    Error::conflict(
        "invoices.not_draft",
        "Only draft documents can be changed or deleted. Correct an issued document with an \
         adjustment or a replacement.",
    )
}

pub(crate) fn retention_hold(until: Option<NaiveDate>) -> Error {
    Error::conflict(
        "invoices.retention_hold",
        "Accounting documents must be kept until {until}; this one cannot be deleted.",
    )
    .with_param(
        "until",
        until.map_or_else(|| "-".to_owned(), |day| day.format("%Y-%m-%d").to_string()),
    )
}

pub(crate) fn series_not_found() -> Error {
    Error::not_found("invoices.series_not_found", "There is no such document series.")
}

pub(crate) fn series_inactive(symbol: &str) -> Error {
    Error::conflict("invoices.series_inactive", "The series `{symbol}` is no longer in use.")
        .with_param("symbol", symbol)
}

pub(crate) fn mandate_missing() -> Error {
    Error::new(
        ErrorKind::Forbidden,
        "invoices.mandate_missing",
        "There is no mandate in force to issue invoices on behalf of this seller.",
    )
}

pub(crate) fn provider_not_configured(provider: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "invoices.provider_not_configured",
        "The e-invoice provider `{provider}` is not set up on this server.",
    )
    .with_param("provider", provider)
}

pub(crate) fn mandate_not_found() -> Error {
    Error::not_found("invoices.mandate_not_found", "There is no such mandate.")
}

/// Ngoài phạm vi thì như không có.
pub(crate) fn hide(error: Error, missing: fn() -> Error) -> Error {
    if error.code() == "org_out_of_scope" { missing() } else { error }
}

/// Tiền từ hai cột.
pub(crate) fn money(minor: i64, currency: &str) -> Result<Money, Error> {
    Money::from_columns(minor, currency)
}

/// Mã số thuế người mua: 10 hay 13 chữ số (`10-3`), hay số định danh cá nhân 12 chữ số dùng làm mã số
/// thuế (TT 86/2024/TT-BTC — cần đối chiếu văn bản gốc).
pub(crate) fn valid_tax_code(code: &str) -> bool {
    let bytes = code.as_bytes();
    let digits = |part: &[u8]| part.iter().all(u8::is_ascii_digit);
    match bytes.len() {
        10 | 12 | 13 => digits(bytes),
        14 => digits(&bytes[..10]) && bytes[10] == b'-' && digits(&bytes[11..]),
        _ => false,
    }
}

/// Ngày phải lưu tới: hết năm (dương lịch) của ngày lập + 10 năm (Luật Kế toán 88/2015 Điều 41,
/// NĐ 174/2016 Điều 12 — thời hạn tính từ kết thúc kỳ kế toán năm; cần đối chiếu khi năm tài chính
/// không theo năm dương lịch).
pub(crate) fn retention_until(issued: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(issued.year() + 10, 12, 31).unwrap_or(issued)
}

fn describe_line(row: &line::Model, currency: &str) -> Result<DocumentLine, Error> {
    Ok(DocumentLine {
        line_no: row.line_no,
        kind: row.kind.clone(),
        item_ref: row.item_ref.clone(),
        description: row.description.clone(),
        unit: row.unit.clone(),
        quantity: row.quantity.clone(),
        unit_price: money(row.unit_price_minor, currency)?,
        discount: money(row.discount_minor, currency)?,
        net: money(row.net_minor, currency)?,
        tax_category: row.tax_category.clone(),
        rate_percent: row.rate_percent.clone(),
        tax: money(row.tax_minor, currency)?,
        total: money(row.total_minor, currency)?,
    })
}

/// Mô tả chứng từ `row` kèm dòng và ký hiệu.
pub(crate) async fn describe<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    row: document::Model,
) -> Result<Document, Error> {
    let lines = line::Entity::find()
        .scoped(tenant)
        .filter(line::Column::DocumentId.eq(row.id))
        .order_by_asc(line::Column::LineNo)
        .all(connection)
        .await?;
    let found = match row.series_id {
        Some(id) => series::Entity::find_by_id(id).scoped(tenant).one(connection).await?,
        None => None,
    };
    let lines = lines
        .iter()
        .map(|item| describe_line(item, &row.currency))
        .collect::<Result<Vec<_>, _>>()?;
    let buyer: Buyer = serde_json::from_str(&row.buyer).unwrap_or_default();
    Ok(Document {
        id: row.id,
        org_id: row.org_id,
        on_behalf_of_org_id: row.on_behalf_of_org_id,
        kind: row.kind,
        relation: row.relation,
        related_document_id: row.related_document_id,
        direction: row.direction,
        series_id: row.series_id,
        symbol: found.as_ref().map(|found| found.symbol.clone()),
        number: row.number,
        status: row.status,
        issue_date: row.issue_date,
        prices_include_tax: row.prices_include_tax,
        subtotal: money(row.subtotal_minor, &row.currency)?,
        tax: money(row.tax_minor, &row.currency)?,
        total: money(row.total_minor, &row.currency)?,
        currency: row.currency,
        buyer,
        seller: serde_json::from_str(&row.seller).unwrap_or_default(),
        lines,
        einvoice_provider: found.and_then(|found| found.einvoice_provider),
        provider_ref: row.provider_ref,
        provider_number: row.provider_number,
        authority_code: row.authority_code,
        lookup_code: row.lookup_code,
        rejection_reason: row.rejection_reason,
        retention_until: row.retention_until,
        source_module: row.source_module,
        source_id: row.source_id,
        reference: row.reference,
        issued_at: row.issued_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// Một ký hiệu cần tạo.
#[derive(Debug, Clone)]
pub(crate) struct SeriesInput {
    pub org: OrgId,
    pub kind: String,
    pub symbol: String,
    pub year: i32,
    pub einvoice_provider: Option<String>,
}

/// Một uỷ nhiệm cần ghi.
#[derive(Debug, Clone)]
pub(crate) struct MandateInput {
    pub principal: OrgId,
    pub agent: OrgId,
    pub kinds: Vec<String>,
    pub valid_from: NaiveDate,
    pub valid_to: Option<NaiveDate>,
    pub document_file_id: Option<uuid::Uuid>,
}

impl InvoicesService {
    pub(crate) fn provider(&self, name: &str) -> Result<Arc<dyn EInvoiceProvider>, Error> {
        self.providers
            .iter()
            .find(|provider| provider.provider() == name)
            .cloned()
            .ok_or_else(|| provider_not_configured(name))
    }

    /// Kiểm `permission` ở nút `org`; ngoài phạm vi trả `missing`.
    pub(crate) async fn require_at(
        tenant: &TenantContext,
        principal: &Principal,
        permission: &Permission,
        org: uuid::Uuid,
        missing: fn() -> Error,
    ) -> Result<(), Error> {
        principal.require(permission)?;
        principal
            .require_at(
                tenant.database().read(),
                permission,
                OrgTarget::new(OrgId::from_uuid(org), None),
            )
            .await
            .map_err(|error| hide(error, missing))
    }

    // ---- Ký hiệu -----------------------------------------------------------------------------

    pub(crate) async fn create_series(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: SeriesInput,
    ) -> Result<series::Model, Error> {
        principal.require(&SERIES_MANAGE)?;
        let mut problems = Vec::new();
        if !KINDS.contains(&input.kind.as_str()) {
            problems.push(
                FieldError::new("kind", "invalid", "must be one of {values}")
                    .with_param("values", KINDS.join(", ")),
            );
        }
        let symbol = input.symbol.trim().to_owned();
        if !(1..=16).contains(&symbol.len()) || !symbol.bytes().all(|b| b.is_ascii_alphanumeric()) {
            problems.push(FieldError::new(
                "symbol",
                "format",
                "must be 1 to 16 letters and digits, e.g. `1C26TAA`",
            ));
        }
        if !(2000..=2100).contains(&input.year) {
            problems.push(FieldError::new("year", "range", "must be 2000 to 2100"));
        }
        if let Some(name) = &input.einvoice_provider
            && self.provider(name).is_err()
        {
            problems.push(FieldError::new(
                "einvoice_provider",
                "unknown",
                "is not an e-invoice provider set up on this server",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let transaction = tenant.database().begin_write().await?;
        principal
            .require_write_at(&transaction, &SERIES_MANAGE, OrgTarget::new(input.org, None))
            .await?;
        let at = now();
        let inserted = series::Entity::insert(series::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(input.org.as_uuid()),
            kind: Set(input.kind.clone()),
            symbol: Set(symbol.clone()),
            year: Set(input.year),
            einvoice_provider: Set(input.einvoice_provider),
            status: Set("active".to_owned()),
            created_at: Set(at),
            updated_at: Set(at),
        })
        .on_conflict(
            OnConflict::columns([
                series::Column::TenantId,
                series::Column::OrgId,
                series::Column::Kind,
                series::Column::Symbol,
                series::Column::Year,
            ])
            .do_nothing()
            .to_owned(),
        )
        .exec_without_returning(&transaction)
        .await?;
        if inserted == 0 {
            return Err(Error::conflict(
                "invoices.series_exists",
                "This unit already has the series `{symbol}` for this kind and year.",
            )
            .with_param("symbol", symbol));
        }
        let saved = series::Entity::find()
            .scoped(tenant)
            .filter(series::Column::OrgId.eq(input.org.as_uuid()))
            .filter(series::Column::Kind.eq(input.kind))
            .filter(series::Column::Symbol.eq(symbol))
            .filter(series::Column::Year.eq(input.year))
            .one(&transaction)
            .await?
            .ok_or_else(|| Error::internal("a series vanished"))?;
        transaction.commit().await?;
        Ok(saved)
    }

    pub(crate) async fn list_series(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<series::Model>, Error> {
        let condition = principal.scope_condition(&READ, series::Column::OrgId, None)?;
        let select = series::Entity::find().scoped(tenant).filter(condition);
        fetch_page(select, series::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Đổi trạng thái ký hiệu (`active` / `inactive`).
    pub(crate) async fn set_series_status(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: uuid::Uuid,
        status: String,
    ) -> Result<series::Model, Error> {
        principal.require(&SERIES_MANAGE)?;
        if !["active", "inactive"].contains(&status.as_str()) {
            return Err(Error::validation(vec![FieldError::new(
                "status",
                "invalid",
                "must be `active` or `inactive`",
            )]));
        }
        let read = tenant.database().read();
        let found = series::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(series_not_found)?;
        Self::require_at(tenant, principal, &SERIES_MANAGE, found.org_id, series_not_found).await?;
        let transaction = tenant.database().begin_write().await?;
        principal
            .require_write_at(
                &transaction,
                &SERIES_MANAGE,
                OrgTarget::new(OrgId::from_uuid(found.org_id), None),
            )
            .await?;
        let mut row: series::ActiveModel = found.into();
        row.status = Set(status);
        row.updated_at = Set(now());
        let saved = series::Entity::update(row).exec(&transaction).await?;
        transaction.commit().await?;
        Ok(saved)
    }

    // ---- Uỷ nhiệm ----------------------------------------------------------------------------

    pub(crate) async fn create_mandate(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: MandateInput,
    ) -> Result<mandate::Model, Error> {
        principal.require(&MANDATES_MANAGE)?;
        let mut problems = Vec::new();
        if input.kinds.is_empty() {
            problems.push(FieldError::new("kinds", "count", "must name at least one kind"));
        }
        for (index, kind) in input.kinds.iter().enumerate() {
            if !crate::api::is_sales_kind(kind) && kind != "withholding_certificate" {
                problems.push(FieldError::new(
                    format!("kinds[{index}]"),
                    "invalid",
                    "must be a kind of invoice",
                ));
            }
        }
        if input.valid_to.is_some_and(|to| to < input.valid_from) {
            problems.push(FieldError::new("valid_to", "range", "must not be before `valid_from`"));
        }
        if input.principal == input.agent {
            problems.push(FieldError::new(
                "agent_org_id",
                "same",
                "must be another unit than the seller",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let transaction = tenant.database().begin_write().await?;
        // Ghi uỷ nhiệm là việc của bên uỷ nhiệm (người bán).
        principal
            .require_write_at(&transaction, &MANDATES_MANAGE, OrgTarget::new(input.principal, None))
            .await?;
        let saved = mandate::Entity::insert(mandate::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            principal_org_id: Set(input.principal.as_uuid()),
            agent_org_id: Set(input.agent.as_uuid()),
            kinds: Set(serde_json::to_string(&input.kinds).map_err(Error::internal)?),
            valid_from: Set(input.valid_from),
            valid_to: Set(input.valid_to),
            document_file_id: Set(input.document_file_id),
            status: Set("active".to_owned()),
            created_by: Set(Some(principal.user_id())),
            created_at: Set(now()),
            revoked_at: Set(None),
        })
        .exec_with_returning(&transaction)
        .await?;
        transaction.commit().await?;
        Ok(saved)
    }

    pub(crate) async fn list_mandates(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<mandate::Model>, Error> {
        let condition = principal.scope_condition(&READ, mandate::Column::PrincipalOrgId, None)?;
        let select = mandate::Entity::find().scoped(tenant).filter(condition);
        fetch_page(select, mandate::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    pub(crate) async fn revoke_mandate(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: uuid::Uuid,
    ) -> Result<mandate::Model, Error> {
        principal.require(&MANDATES_MANAGE)?;
        let read = tenant.database().read();
        let found = mandate::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(mandate_not_found)?;
        Self::require_at(
            tenant,
            principal,
            &MANDATES_MANAGE,
            found.principal_org_id,
            mandate_not_found,
        )
        .await?;
        let transaction = tenant.database().begin_write().await?;
        let row = rows::lock_one(&transaction, mandate::Entity::find_by_id(id).scoped(tenant))
            .await?
            .ok_or_else(mandate_not_found)?;
        if row.status == "revoked" {
            return Ok(row);
        }
        let mut active: mandate::ActiveModel = row.into();
        active.status = Set("revoked".to_owned());
        active.revoked_at = Set(Some(now()));
        let saved = mandate::Entity::update(active).exec(&transaction).await?;
        transaction.commit().await?;
        Ok(saved)
    }

    /// Có uỷ nhiệm còn hiệu lực cho `agent` lập `kind` thay `principal` vào ngày `on` không.
    pub(crate) async fn has_mandate<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        principal: uuid::Uuid,
        agent: uuid::Uuid,
        kind: &str,
        on: NaiveDate,
    ) -> Result<bool, Error> {
        let found = mandate::Entity::find()
            .scoped(tenant)
            .filter(mandate::Column::PrincipalOrgId.eq(principal))
            .filter(mandate::Column::AgentOrgId.eq(agent))
            .filter(mandate::Column::Status.eq("active"))
            .filter(mandate::Column::ValidFrom.lte(on))
            .all(connection)
            .await?;
        Ok(found.iter().any(|row| {
            row.valid_to.is_none_or(|to| on <= to)
                && serde_json::from_str::<Vec<String>>(&row.kinds)
                    .is_ok_and(|kinds| kinds.iter().any(|found| found == kind))
        }))
    }
}
