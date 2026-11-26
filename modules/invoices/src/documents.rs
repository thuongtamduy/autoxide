//! Chứng từ nháp: lập (thuế tính sẵn qua `TaxEngine`), sửa, xoá, đọc. Nháp không có số; số chỉ cấp
//! khi phát hành ([`crate::issue`]).

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::decimal::{Decimal, Rounding, to_column};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::reference::{self, Reference, Submitted};
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use autoxide_kernel::time::{Date, Timestamp};
use autoxide_tax::api::{PriceMode, TaxQuoteRequest, TaxableLine};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{Buyer, Document, KINDS, NewDocument, NewDocumentLine, is_sales_kind};
use crate::entity::{document, line};
use crate::permissions::{READ, WRITE};
use crate::service::{
    InvoicesService, describe, document_not_found, not_draft, now, retention_hold, valid_tax_code,
};

/// Một dòng đã tính.
#[derive(Debug, Clone)]
pub(crate) struct ComputedLine {
    pub input: NewDocumentLine,
    pub quantity: Decimal,
    pub unit_price: Money,
    pub discount: Money,
    pub net: Money,
    pub rate_percent: Option<String>,
    pub tax: Money,
    pub total: Money,
}

/// Những gì cần để tính một chứng từ.
pub(crate) struct Spec<'a> {
    /// Người bán (nút có hồ sơ thuế quyết định chế độ thuế).
    pub seller: OrgId,
    pub kind: &'a str,
    pub prices_include_tax: bool,
    pub currency: Currency,
    pub buyer: &'a Buyer,
    pub on: Date,
    pub lines: &'a [NewDocumentLine],
}

/// Chứng từ đã tính thuế.
#[derive(Debug, Clone)]
pub(crate) struct Computed {
    pub lines: Vec<ComputedLine>,
    pub subtotal: Money,
    pub tax: Money,
    pub total: Money,
    pub snapshot: Value,
}

/// Thay đổi một chứng từ nháp (`PATCH`): trường `None` giữ nguyên.
#[derive(Debug, Clone, Default)]
pub(crate) struct DraftChange {
    pub buyer: Option<Buyer>,
    pub lines: Option<Vec<NewDocumentLine>>,
    pub series: Option<Option<Uuid>>,
    pub issue_date: Option<Option<Date>>,
    pub prices_include_tax: Option<bool>,
}

/// Bộ lọc danh sách chứng từ.
#[derive(Debug, Clone, Default)]
pub(crate) struct DocumentFilter {
    pub org: Option<OrgId>,
    pub kind: Option<String>,
    pub status: Option<String>,
    pub source_module: Option<String>,
    pub source_id: Option<String>,
}

/// Kiểm người mua; lỗi vào `problems`.
pub(crate) fn check_buyer(buyer: &Buyer, kind: &str, problems: &mut Vec<FieldError>) {
    if !["consumer", "business"].contains(&buyer.kind.as_str()) {
        problems.push(FieldError::new("buyer.kind", "invalid", "must be `consumer` or `business`"));
    }
    if buyer.name.as_ref().is_some_and(|name| !(1..=200).contains(&name.chars().count())) {
        problems.push(FieldError::new("buyer.name", "length", "must be 1 to 200 characters long"));
    }
    if buyer.address.as_ref().is_some_and(|text| text.chars().count() > 400) {
        problems.push(FieldError::new(
            "buyer.address",
            "length",
            "must be at most 400 characters long",
        ));
    }
    if let Some(email) = &buyer.email {
        autoxide_kernel::email::checked(email, "buyer.email", problems);
    }
    match &buyer.tax_code {
        Some(code) if !valid_tax_code(code) => problems.push(FieldError::new(
            "buyer.tax_code",
            "format",
            "must have 10 or 13 digits (10-3), or be a 12-digit personal id",
        )),
        None if buyer.kind == "business" && kind == "vat_invoice" => problems.push(
            FieldError::new("buyer.tax_code", "required", "is required for a business buyer"),
        ),
        _ => {}
    }
}

impl InvoicesService {
    /// Tính tiền và thuế các dòng. Chỉ đọc; không gọi trong transaction ghi (`TaxEngine` đọc
    /// database điều khiển).
    pub(crate) async fn compute(
        &self,
        tenant: &TenantContext,
        spec: &Spec<'_>,
    ) -> Result<Computed, Error> {
        let Spec { seller, kind, prices_include_tax, currency, buyer, on, lines } = *spec;
        let mut problems = Vec::new();
        if lines.is_empty() || lines.len() > 1000 {
            problems.push(FieldError::new("lines", "count", "must have 1 to 1000 lines"));
        }
        let sales = is_sales_kind(kind);
        let mut computed = Vec::with_capacity(lines.len());
        for (index, item) in lines.iter().enumerate() {
            let field = |name: &str| format!("lines[{index}].{name}");
            if !["goods", "service", "note"].contains(&item.kind.as_str()) {
                problems.push(FieldError::new(
                    field("kind"),
                    "invalid",
                    "must be `goods`, `service` or `note`",
                ));
            }
            if !(1..=500).contains(&item.description.trim().chars().count()) {
                problems.push(FieldError::new(
                    field("description"),
                    "length",
                    "must be 1 to 500 characters long",
                ));
            }
            let zero = Money::zero(currency);
            if item.kind == "note" {
                computed.push(ComputedLine {
                    input: item.clone(),
                    quantity: Decimal::ZERO,
                    unit_price: zero,
                    discount: zero,
                    net: zero,
                    rate_percent: None,
                    tax: zero,
                    total: zero,
                });
                continue;
            }
            let quantity = item.quantity.map_or(Decimal::ONE, |quantity| quantity.0);
            if quantity <= Decimal::ZERO {
                problems.push(FieldError::new(
                    field("quantity"),
                    "range",
                    "must be greater than zero",
                ));
            }
            let Some(unit_price) = item.unit_price else {
                problems.push(FieldError::new(field("unit_price"), "required", "is required"));
                continue;
            };
            if unit_price.currency() != currency || unit_price.is_negative() {
                problems.push(FieldError::new(
                    field("unit_price"),
                    "invalid",
                    "must not be negative and must be in the document currency",
                ));
                continue;
            }
            let discount = item.discount.unwrap_or(zero);
            let gross = unit_price.apply_rate(quantity, Rounding::HalfUp)?;
            if discount.currency() != currency
                || discount.is_negative()
                || discount.minor() > gross.minor()
            {
                problems.push(FieldError::new(
                    field("discount"),
                    "range",
                    "must be between zero and the line amount, in the document currency",
                ));
                continue;
            }
            if sales && item.tax_category.is_none() {
                problems.push(FieldError::new(
                    field("tax_category"),
                    "required",
                    "is required on a sales document",
                ));
            }
            let net = gross.checked_sub(discount)?;
            computed.push(ComputedLine {
                input: item.clone(),
                quantity,
                unit_price,
                discount,
                net,
                rate_percent: None,
                tax: zero,
                total: net,
            });
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let zero = Money::zero(currency);
        if !sales {
            let subtotal = Money::sum(currency, computed.iter().map(|line| line.net))?;
            return Ok(Computed {
                lines: computed,
                subtotal,
                tax: zero,
                total: subtotal,
                snapshot: json!({}),
            });
        }
        let jurisdiction = self
            .tax
            .profile(tenant, seller)
            .await?
            .map_or_else(|| "VN".to_owned(), |profile| profile.jurisdiction);
        let taxable: Vec<TaxableLine> = computed
            .iter()
            .enumerate()
            .filter(|(_, line)| line.input.kind != "note")
            .map(|(index, line)| {
                let category = line.input.tax_category.clone().unwrap_or_default();
                TaxableLine::new(index.to_string(), category, line.net).with_quantity(line.quantity)
            })
            .collect();
        let mode = if prices_include_tax { PriceMode::Inclusive } else { PriceMode::Exclusive };
        let mut request = TaxQuoteRequest::new(jurisdiction, on, seller, mode, currency, taxable);
        if buyer.kind == "business" {
            request = request.for_business();
        }
        let quote = self.tax.quote(tenant, request).await?;
        for taxed in &quote.lines {
            let Ok(index) = taxed.line_id.parse::<usize>() else { continue };
            let Some(line) = computed.get_mut(index) else { continue };
            line.net = taxed.net;
            line.tax = Money::sum(currency, taxed.taxes.iter().map(|tax| tax.amount))?;
            line.total = taxed.gross;
            line.rate_percent = taxed
                .taxes
                .iter()
                .find(|tax| tax.tax_kind == "vat" && !tax.exempt)
                .map(|tax| tax.rate_percent.clone());
        }
        Ok(Computed {
            lines: computed,
            subtotal: quote.net,
            tax: quote.tax,
            total: quote.gross,
            snapshot: quote.snapshot,
        })
    }

    /// Ghi dòng của chứng từ `document_id` (thay hết dòng cũ).
    pub(crate) async fn write_lines<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        document_id: Uuid,
        computed: &Computed,
    ) -> Result<(), Error> {
        line::Entity::delete_many()
            .filter(line::Column::TenantId.eq(tenant.id().as_uuid()))
            .filter(line::Column::DocumentId.eq(document_id))
            .exec(connection)
            .await?;
        let rows: Vec<line::ActiveModel> = computed
            .lines
            .iter()
            .enumerate()
            .map(|(index, item)| line::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                document_id: Set(document_id),
                line_no: Set(i32::try_from(index + 1).unwrap_or(i32::MAX)),
                kind: Set(item.input.kind.clone()),
                item_ref: Set(item.input.item_ref.clone()),
                description: Set(item.input.description.trim().to_owned()),
                unit: Set(item.input.unit.clone()),
                quantity: Set(to_column(item.quantity)),
                unit_price_minor: Set(item.unit_price.minor()),
                discount_minor: Set(item.discount.minor()),
                net_minor: Set(item.net.minor()),
                tax_category: Set(item.input.tax_category.clone()),
                rate_percent: Set(item.rate_percent.clone()),
                tax_minor: Set(item.tax.minor()),
                total_minor: Set(item.total.minor()),
            })
            .collect();
        if !rows.is_empty() {
            line::Entity::insert_many(rows).exec_without_returning(connection).await?;
        }
        Ok(())
    }

    /// Lập chứng từ nháp (`dyn Invoices`, REST, MCP).
    pub(crate) async fn create_document(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        draft: NewDocument,
    ) -> Result<Submitted<Document>, Error> {
        self.create_inner(tenant, actor, draft, None).await
    }

    /// Lập nháp; `related`: (chứng từ gốc, `adjustment` / `replacement`, hướng).
    pub(crate) async fn create_inner(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        draft: NewDocument,
        related: Option<(Uuid, &'static str, String)>,
    ) -> Result<Submitted<Document>, Error> {
        let mut problems = Vec::new();
        if !KINDS.contains(&draft.kind.as_str()) {
            problems.push(
                FieldError::new("kind", "invalid", "must be one of {values}")
                    .with_param("values", KINDS.join(", ")),
            );
        }
        let reference = Reference::checked(&draft.reference, "reference", &mut problems);
        check_buyer(&draft.buyer, &draft.kind, &mut problems);
        if draft.on_behalf_of.is_some()
            && !is_sales_kind(&draft.kind)
            && draft.kind != "withholding_certificate"
        {
            problems.push(FieldError::new(
                "on_behalf_of_org_id",
                "kind",
                "only invoices can be issued on behalf of a seller",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let Some(reference) = reference else {
            return Err(Error::internal("a checked reference went missing"));
        };
        let seller = draft.on_behalf_of.unwrap_or(draft.org);
        // Ngày lập mặc định: hôm nay theo múi giờ của vùng thuế người bán, không theo UTC (LG-2).
        let on = match draft.issue_date {
            Some(day) => day,
            None => self.tax.tax_date(tenant, seller, Timestamp::now()).await?,
        };
        let computed = self
            .compute(
                tenant,
                &Spec {
                    seller,
                    kind: &draft.kind,
                    prices_include_tax: draft.prices_include_tax,
                    currency: draft.currency,
                    buyer: &draft.buyer,
                    on,
                    lines: &draft.lines,
                },
            )
            .await?;
        let fingerprint = reference::fingerprint(&json!({
            "org": draft.org.as_uuid(), "on_behalf_of": draft.on_behalf_of.map(|org| org.as_uuid()),
            "kind": draft.kind, "series": draft.series, "issue_date": draft.issue_date,
            "include_tax": draft.prices_include_tax, "currency": draft.currency.code(),
            "buyer": draft.buyer, "lines": draft.lines, "source_id": draft.source_id,
            "related": related.as_ref().map(|(id, relation, direction)| json!([id, relation, direction])),
        }))?;
        let database = tenant.database();
        let (reference, fingerprint, draft, computed, related) =
            (&reference, &fingerprint, &draft, &computed, &related);
        let outcome = reference::submit_once(database, move || async move {
            let transaction = database.begin_write().await?;
            let stored = document::Entity::find()
                .scoped(tenant)
                .filter(document::Column::SourceModule.eq(draft.source_module.as_str()))
                .filter(document::Column::Reference.eq(reference.as_str()))
                .one(&transaction)
                .await?
                .map(|row| {
                    let fingerprint = row.reference_fingerprint.clone();
                    (row, fingerprint)
                });
            if let Some(existing) = reference::replayed(stored, reference, fingerprint)? {
                return Ok(Submitted::Replayed(existing));
            }
            let at = now();
            let id = Uuid::now_v7();
            let saved = document::Entity::insert(document::ActiveModel {
                id: Set(id),
                tenant_id: Set(tenant.id().as_uuid()),
                org_id: Set(draft.org.as_uuid()),
                on_behalf_of_org_id: Set(draft.on_behalf_of.map(|org| org.as_uuid())),
                kind: Set(draft.kind.clone()),
                relation: Set(related.as_ref().map(|(_, relation, _)| (*relation).to_owned())),
                related_document_id: Set(related.as_ref().map(|(id, _, _)| *id)),
                direction: Set(related
                    .as_ref()
                    .map_or_else(|| "increase".to_owned(), |(_, _, direction)| direction.clone())),
                series_id: Set(draft.series),
                number: Set(None),
                status: Set("draft".to_owned()),
                issue_date: Set(draft.issue_date),
                prices_include_tax: Set(draft.prices_include_tax),
                currency: Set(draft.currency.code().to_owned()),
                subtotal_minor: Set(computed.subtotal.minor()),
                tax_minor: Set(computed.tax.minor()),
                total_minor: Set(computed.total.minor()),
                buyer: Set(serde_json::to_string(&draft.buyer).map_err(Error::internal)?),
                seller: Set("{}".to_owned()),
                tax_snapshot: Set(computed.snapshot.to_string()),
                provider_ref: Set(None),
                provider_number: Set(None),
                authority_code: Set(None),
                lookup_code: Set(None),
                rejection_reason: Set(None),
                retention_until: Set(None),
                source_module: Set(draft.source_module.clone()),
                source_id: Set(draft.source_id.clone()),
                reference: Set(reference.as_str().to_owned()),
                reference_fingerprint: Set(fingerprint.clone()),
                issued_by: Set(None),
                issued_at: Set(None),
                created_by: Set(actor.user_id),
                created_at: Set(at),
                updated_at: Set(at),
                exchange_rate: Set(None),
                exchange_rate_id: Set(None),
                base_currency: Set(None),
                base_total_minor: Set(None),
                buyer_purged_at: Set(None),
                booked_by_source: Set(draft.booked_by_source),
            })
            .exec_with_returning(&transaction)
            .await?;
            Self::write_lines(&transaction, tenant, id, computed).await?;
            transaction.commit().await?;
            Ok(Submitted::Created(saved))
        })
        .await?;
        let created = matches!(outcome, Submitted::Created(_));
        let described = describe(database.write(), tenant, outcome.into_inner()).await?;
        Ok(if created { Submitted::Created(described) } else { Submitted::Replayed(described) })
    }

    /// Lập nháp qua REST / MCP: cần `invoices.write` ở nút lập.
    pub(crate) async fn create_manual(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        draft: NewDocument,
    ) -> Result<Submitted<Document>, Error> {
        principal.require(&WRITE)?;
        principal
            .require_at(tenant.database().read(), &WRITE, OrgTarget::new(draft.org, None))
            .await?;
        self.create_document(tenant, Actor::from(principal), draft).await
    }

    /// Chứng từ `id` cho người có `permission` ở nút lập.
    pub(crate) async fn document_for(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        permission: &autoxide_kernel::access::Permission,
        id: Uuid,
    ) -> Result<document::Model, Error> {
        principal.require(permission)?;
        let found = document::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(document_not_found)?;
        Self::require_at(tenant, principal, permission, found.org_id, document_not_found).await?;
        Ok(found)
    }

    pub(crate) async fn list_documents(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: DocumentFilter,
        page: &PageRequest,
    ) -> Result<Page<Document>, Error> {
        let condition = principal.scope_condition(&READ, document::Column::OrgId, None)?;
        let mut select = document::Entity::find().scoped(tenant).filter(condition);
        if let Some(org) = filter.org {
            select = select.filter(document::Column::OrgId.eq(org.as_uuid()));
        }
        if let Some(kind) = filter.kind {
            select = select.filter(document::Column::Kind.eq(kind));
        }
        if let Some(status) = filter.status {
            select = select.filter(document::Column::Status.eq(status));
        }
        if let Some(module) = filter.source_module {
            select = select.filter(document::Column::SourceModule.eq(module));
        }
        if let Some(id) = filter.source_id {
            select = select.filter(document::Column::SourceId.eq(id));
        }
        let read = tenant.database().read();
        let Page { data, pagination } =
            fetch_page(select, document::Column::Id, |row| row.id, page, read).await?;
        let mut described = Vec::with_capacity(data.len());
        for row in data {
            described.push(describe(read, tenant, row).await?);
        }
        Ok(Page { data: described, pagination })
    }

    /// Sửa chứng từ nháp; chứng từ bị từ chối chỉ sửa được người mua (gửi lại cùng số).
    pub(crate) async fn update_draft(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        change: DraftChange,
    ) -> Result<Document, Error> {
        let found = self.document_for(tenant, principal, &WRITE, id).await?;
        let rejected = found.status == "rejected";
        if found.status != "draft" && !rejected {
            return Err(not_draft());
        }
        if rejected
            && (change.lines.is_some()
                || change.series.is_some()
                || change.issue_date.is_some()
                || change.prices_include_tax.is_some())
        {
            return Err(not_draft());
        }
        let buyer: Buyer = match &change.buyer {
            Some(buyer) => buyer.clone(),
            None => serde_json::from_str(&found.buyer).unwrap_or_default(),
        };
        let mut problems = Vec::new();
        check_buyer(&buyer, &found.kind, &mut problems);
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let issue_date = change.issue_date.unwrap_or(found.issue_date);
        let include_tax = change.prices_include_tax.unwrap_or(found.prices_include_tax);
        let lines = match change.lines.clone() {
            Some(lines) => Some(lines),
            None if change.buyer.is_some()
                || change.issue_date.is_some()
                || change.prices_include_tax.is_some() =>
            {
                if rejected {
                    None
                } else {
                    Some(self.stored_lines(tenant, found.id).await?)
                }
            }
            None => None,
        };
        let currency = Currency::parse(&found.currency)?;
        let seller = OrgId::from_uuid(found.on_behalf_of_org_id.unwrap_or(found.org_id));
        let on = match issue_date {
            Some(day) => day,
            None => self.tax.tax_date(tenant, seller, Timestamp::now()).await?,
        };
        let computed = match &lines {
            Some(lines) => Some(
                self.compute(
                    tenant,
                    &Spec {
                        seller,
                        kind: &found.kind,
                        prices_include_tax: include_tax,
                        currency,
                        buyer: &buyer,
                        on,
                        lines,
                    },
                )
                .await?,
            ),
            None => None,
        };
        let database = tenant.database();
        let transaction = database.begin_write().await?;
        principal
            .require_write_at(
                &transaction,
                &WRITE,
                OrgTarget::new(OrgId::from_uuid(found.org_id), None),
            )
            .await?;
        let row = rows::lock_one(&transaction, document::Entity::find_by_id(id).scoped(tenant))
            .await?
            .ok_or_else(document_not_found)?;
        if row.status != found.status {
            return Err(not_draft());
        }
        let mut active: document::ActiveModel = row.into();
        active.buyer = Set(serde_json::to_string(&buyer).map_err(Error::internal)?);
        if let Some(series) = change.series {
            active.series_id = Set(series);
        }
        active.issue_date = Set(issue_date);
        active.prices_include_tax = Set(include_tax);
        if let Some(computed) = &computed {
            active.subtotal_minor = Set(computed.subtotal.minor());
            active.tax_minor = Set(computed.tax.minor());
            active.total_minor = Set(computed.total.minor());
            active.tax_snapshot = Set(computed.snapshot.to_string());
            Self::write_lines(&transaction, tenant, id, computed).await?;
        }
        active.updated_at = Set(now());
        let saved = document::Entity::update(active).exec(&transaction).await?;
        let described = describe(&transaction, tenant, saved).await?;
        transaction.commit().await?;
        Ok(described)
    }

    /// Dòng đã lưu, dạng đầu vào (để tính lại khi đổi người mua / ngày).
    pub(crate) async fn stored_lines(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<Vec<NewDocumentLine>, Error> {
        let found = describe(
            tenant.database().read(),
            tenant,
            document::Entity::find_by_id(id)
                .scoped(tenant)
                .one(tenant.database().read())
                .await?
                .ok_or_else(document_not_found)?,
        )
        .await?;
        Ok(found
            .lines
            .into_iter()
            .map(|item| NewDocumentLine {
                kind: item.kind,
                item_ref: item.item_ref,
                description: item.description,
                unit: item.unit,
                quantity: autoxide_kernel::decimal::parse(&item.quantity)
                    .ok()
                    .map(autoxide_kernel::decimal::DecimalString),
                unit_price: Some(item.unit_price),
                discount: Some(item.discount),
                tax_category: item.tax_category,
            })
            .collect())
    }

    /// Xoá chứng từ nháp. Đã phát hành → `invoices.retention_hold`.
    pub(crate) async fn delete_draft(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        let found = self.document_for(tenant, principal, &WRITE, id).await?;
        if found.status != "draft" {
            return Err(retention_hold(found.retention_until));
        }
        let transaction = tenant.database().begin_write().await?;
        principal
            .require_write_at(
                &transaction,
                &WRITE,
                OrgTarget::new(OrgId::from_uuid(found.org_id), None),
            )
            .await?;
        let row = rows::lock_one(&transaction, document::Entity::find_by_id(id).scoped(tenant))
            .await?
            .ok_or_else(document_not_found)?;
        if row.status != "draft" {
            return Err(retention_hold(row.retention_until));
        }
        line::Entity::delete_many()
            .filter(line::Column::TenantId.eq(tenant.id().as_uuid()))
            .filter(line::Column::DocumentId.eq(id))
            .exec(&transaction)
            .await?;
        document::Entity::delete_by_id(id).scoped(tenant).exec(&transaction).await?;
        transaction.commit().await?;
        Ok(())
    }
}

impl InvoicesService {
    /// Chứng từ `id` cho người có `invoices.read` ở nút lập, đã mô tả.
    pub(crate) async fn document_detail(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Document, Error> {
        let row = self.document_for(tenant, principal, &READ, id).await?;
        describe(tenant.database().read(), tenant, row).await
    }
}
