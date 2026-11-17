//! Mẫu in theo khách (bước 36 mục 11, D-353): lưu, kiểm theo hợp đồng biến, chọn mẫu khi in, dựng
//! HTML / PDF. Mẫu là dữ liệu không tin cậy — dựng trong hộp cát của `autoxide_kernel::template`.

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::template::Output;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, root_of};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};
use uuid::Uuid;

use crate::api::{Document, KINDS, RenderFormat, Rendered};
use crate::entity::template;
use crate::permissions::{READ, TEMPLATES_MANAGE};
use crate::print::{self, DEFAULT_TEMPLATE};
use crate::service::{InvoicesService, describe, now};

pub(crate) const LOCALES: [&str; 2] = ["en", "vi"];

pub(crate) fn template_not_found() -> Error {
    Error::not_found("invoices.template_not_found", "There is no such print template.")
}

fn template_exists() -> Error {
    Error::conflict(
        "invoices.template_exists",
        "This unit already has a print template for this kind and language; change it with `PUT`.",
    )
}

/// Một mẫu cần lưu.
#[derive(Debug, Clone)]
pub(crate) struct TemplateInput {
    /// Bỏ trống: mẫu của cả khách.
    pub org: Option<OrgId>,
    pub kind: String,
    pub locale: String,
    pub body: String,
}

impl InvoicesService {
    /// Kiểm `invoices.templates_manage` ở nút của mẫu (mẫu của cả khách: ở gốc).
    async fn require_templates(
        tenant: &TenantContext,
        principal: &Principal,
        org: Uuid,
    ) -> Result<(), Error> {
        let target = if org.is_nil() { root_of(tenant.id()).as_uuid() } else { org };
        Self::require_at(tenant, principal, &TEMPLATES_MANAGE, target, template_not_found).await
    }

    /// Kiểm loại, ngôn ngữ và thân mẫu; mọi lỗi gom một lần (lỗi của thân mẫu mang dòng, cột).
    fn check_template(
        &self,
        kind: Option<&str>,
        locale: Option<&str>,
        body: &str,
    ) -> Result<(), Error> {
        let mut problems = Vec::new();
        if let Some(kind) = kind
            && !KINDS.contains(&kind)
        {
            problems.push(
                FieldError::new("kind", "invalid", "must be one of {values}")
                    .with_param("values", KINDS.join(", ")),
            );
        }
        if let Some(locale) = locale
            && !LOCALES.contains(&locale)
        {
            problems.push(FieldError::new("locale", "invalid", "must be `en` or `vi`"));
        }
        if let Err(error) = self.templates.validate("body", body, &print::schema()) {
            if error.fields().is_empty() {
                return Err(error);
            }
            problems.extend(error.fields().iter().cloned());
        }
        if problems.is_empty() { Ok(()) } else { Err(Error::validation(problems)) }
    }

    pub(crate) async fn create_template(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: TemplateInput,
    ) -> Result<template::Model, Error> {
        let org = input.org.map_or(Uuid::nil(), |org| org.as_uuid());
        principal.require(&TEMPLATES_MANAGE)?;
        Self::require_templates(tenant, principal, org).await?;
        self.check_template(Some(&input.kind), Some(&input.locale), &input.body)?;
        let transaction = tenant.database().begin_write().await?;
        let target = if org.is_nil() { root_of(tenant.id()) } else { OrgId::from_uuid(org) };
        principal
            .require_write_at(&transaction, &TEMPLATES_MANAGE, OrgTarget::new(target, None))
            .await?;
        let at = now();
        let inserted = template::Entity::insert(template::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(org),
            kind: Set(input.kind.clone()),
            locale: Set(input.locale.clone()),
            version: Set(1),
            body: Set(input.body),
            updated_by: Set(Some(principal.user_id())),
            created_at: Set(at),
            updated_at: Set(at),
        })
        .on_conflict(
            OnConflict::columns([
                template::Column::TenantId,
                template::Column::OrgId,
                template::Column::Kind,
                template::Column::Locale,
            ])
            .do_nothing()
            .to_owned(),
        )
        .exec_without_returning(&transaction)
        .await?;
        if inserted == 0 {
            return Err(template_exists());
        }
        let saved = template::Entity::find()
            .scoped(tenant)
            .filter(template::Column::OrgId.eq(org))
            .filter(template::Column::Kind.eq(input.kind))
            .filter(template::Column::Locale.eq(input.locale))
            .one(&transaction)
            .await?
            .ok_or_else(|| Error::internal("a print template vanished"))?;
        transaction.commit().await?;
        Ok(saved)
    }

    pub(crate) async fn list_templates(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<template::Model>, Error> {
        principal.require(&READ)?;
        let select = template::Entity::find().scoped(tenant);
        fetch_page(select, template::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    pub(crate) async fn get_template(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<template::Model, Error> {
        principal.require(&READ)?;
        template::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(template_not_found)
    }

    /// Sửa thân mẫu (phiên bản tăng 1).
    pub(crate) async fn update_template(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        body: String,
    ) -> Result<template::Model, Error> {
        principal.require(&TEMPLATES_MANAGE)?;
        let found = template::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(template_not_found)?;
        Self::require_templates(tenant, principal, found.org_id).await?;
        self.check_template(None, None, &body)?;
        let transaction = tenant.database().begin_write().await?;
        let row = autoxide_kernel::rows::lock_one(
            &transaction,
            template::Entity::find_by_id(id).scoped(tenant),
        )
        .await?
        .ok_or_else(template_not_found)?;
        let version = row.version.saturating_add(1);
        let mut active: template::ActiveModel = row.into();
        active.body = Set(body);
        active.version = Set(version);
        active.updated_by = Set(Some(principal.user_id()));
        active.updated_at = Set(now());
        let saved = template::Entity::update(active).exec(&transaction).await?;
        transaction.commit().await?;
        Ok(saved)
    }

    /// Xoá mẫu: chứng từ in lại bằng mẫu của cả khách hay mẫu mặc định.
    pub(crate) async fn delete_template(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        principal.require(&TEMPLATES_MANAGE)?;
        let found = template::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(template_not_found)?;
        Self::require_templates(tenant, principal, found.org_id).await?;
        let transaction = tenant.database().begin_write().await?;
        template::Entity::delete_many()
            .filter(template::Column::Id.eq(id))
            .filter(template::Column::TenantId.eq(tenant.id().as_uuid()))
            .exec(&transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Mẫu dùng để in `document` bằng ngôn ngữ `locale`: của nút lập → của cả khách → mặc định.
    async fn template_for(
        &self,
        tenant: &TenantContext,
        document: &Document,
        locale: &str,
    ) -> Result<String, Error> {
        let found = template::Entity::find()
            .scoped(tenant)
            .filter(template::Column::OrgId.is_in([document.org_id, Uuid::nil()]))
            .filter(template::Column::Kind.eq(document.kind.as_str()))
            .filter(template::Column::Locale.eq(locale))
            .all(tenant.database().read())
            .await?;
        let own = found.iter().find(|row| row.org_id == document.org_id);
        let shared = found.iter().find(|row| row.org_id.is_nil());
        Ok(own.or(shared).map_or_else(|| DEFAULT_TEMPLATE.to_owned(), |row| row.body.clone()))
    }

    /// Bản in của chứng từ (đã kiểm quyền hay gọi qua `dyn Invoices`).
    pub(crate) async fn render_document(
        &self,
        tenant: &TenantContext,
        document: &Document,
        format: RenderFormat,
        locale: &str,
    ) -> Result<Rendered, Error> {
        let locale = print::locale_of(locale);
        let view = print::view_model(document, locale);
        let name = format!(
            "{}-{}",
            document.kind,
            document.number.map_or_else(|| document.id.to_string(), |number| number.to_string())
        );
        match format {
            RenderFormat::Html => {
                let body = self.template_for(tenant, document, locale).await?;
                let html = self.templates.render(&body, Output::Html, locale, &view)?;
                Ok(Rendered::new(
                    "text/html; charset=utf-8",
                    format!("{name}.html"),
                    html.into_bytes(),
                ))
            }
            RenderFormat::Pdf => {
                // Dựng PDF tốn CPU (đo chữ, nén font): chạy ở thread chặn, không giữ thread async.
                let bytes = tokio_blocking(move || print::render_pdf(&view)).await?;
                Ok(Rendered::new("application/pdf", format!("{name}.pdf"), bytes))
            }
        }
    }

    /// Bản in qua REST / MCP: cần `invoices.read` ở nút lập chứng từ.
    pub(crate) async fn render_manual(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        format: RenderFormat,
        locale: &str,
    ) -> Result<Rendered, Error> {
        let row = self.document_for(tenant, principal, &READ, id).await?;
        let document = describe(tenant.database().read(), tenant, row).await?;
        self.render_document(tenant, &document, format, locale).await
    }

    /// Xem trước một thân mẫu (chưa lưu) với chứng từ `document` (hay chứng từ mẫu dựng sẵn).
    pub(crate) async fn preview_template(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        kind: &str,
        locale: &str,
        body: &str,
        document: Option<Uuid>,
    ) -> Result<String, Error> {
        principal.require(&TEMPLATES_MANAGE)?;
        self.check_template(Some(kind), Some(locale), body)?;
        let sample = match document {
            Some(id) => {
                let row = self.document_for(tenant, principal, &READ, id).await?;
                describe(tenant.database().read(), tenant, row).await?
            }
            None => sample_document(kind),
        };
        let view = print::view_model(&sample, print::locale_of(locale));
        self.templates.render(body, Output::Html, print::locale_of(locale), &view)
    }
}

/// Chạy `work` ở thread chặn của tokio.
async fn tokio_blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, Error> + Send + 'static,
) -> Result<T, Error> {
    tokio::task::spawn_blocking(work).await.map_err(Error::internal)?
}

/// Chứng từ mẫu để xem trước mẫu in khi chưa có chứng từ thật.
pub(crate) fn sample_document(kind: &str) -> Document {
    use autoxide_kernel::money::{Currency, Money};
    let vnd = |minor| Money::new(minor, Currency::VND);
    let at = now();
    Document {
        id: Uuid::nil(),
        org_id: Uuid::nil(),
        on_behalf_of_org_id: None,
        kind: kind.to_owned(),
        relation: None,
        related_document_id: None,
        direction: "increase".to_owned(),
        series_id: None,
        symbol: Some("1C26TAA".to_owned()),
        number: Some(123),
        status: "issued".to_owned(),
        issue_date: Some(at.date_naive()),
        prices_include_tax: false,
        currency: "VND".to_owned(),
        subtotal: vnd(1_000_000),
        tax: vnd(80_000),
        total: vnd(1_080_000),
        buyer: crate::api::Buyer::business("Example Buyer Co., Ltd", "0101234567")
            .address("1 Example Street"),
        seller: serde_json::json!({"legal_name": "Example Shop", "tax_code": "0109999999", "address": "2 Sample Road"}),
        lines: vec![crate::api::DocumentLine {
            line_no: 1,
            kind: "goods".to_owned(),
            item_ref: Some("SKU-1".to_owned()),
            description: "Example item".to_owned(),
            unit: Some("pcs".to_owned()),
            quantity: "2".to_owned(),
            unit_price: vnd(500_000),
            discount: vnd(0),
            net: vnd(1_000_000),
            tax_category: Some("vat.standard_reducible".to_owned()),
            rate_percent: Some("8".to_owned()),
            tax: vnd(80_000),
            total: vnd(1_080_000),
        }],
        einvoice_provider: None,
        provider_ref: None,
        provider_number: None,
        authority_code: None,
        lookup_code: None,
        rejection_reason: None,
        retention_until: None,
        exchange_rate: None,
        exchange_rate_id: None,
        base_total: None,
        source_module: "invoices".to_owned(),
        source_id: None,
        reference: "sample".to_owned(),
        issued_at: Some(at),
        created_at: at,
        updated_at: at,
    }
}
