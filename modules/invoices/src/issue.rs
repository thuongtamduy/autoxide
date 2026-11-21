//! Phát hành chứng từ, gửi hoá đơn điện tử, ghi sổ, huỷ, điều chỉnh, thay thế.
//!
//! Phát hành trong một transaction: khoá dòng chứng từ (đã phát hành thì trả nguyên — gọi lại không
//! cấp số mới), kiểm ký hiệu và uỷ nhiệm, cấp số bằng `numbering::next` (rollback thì số không bị
//! tiêu → không nhảy số), đặt hạn lưu, event, xếp job gửi hoá đơn điện tử. Gọi nhà cung cấp trong job
//! (ngoài transaction). Ghi sổ sau khi commit, theo `reference` ổn định (gọi lại không ghi trùng).
//!
//! Bị từ chối: sửa người mua rồi phát hành lại → gửi lại **cùng số**. Đã chấp nhận: chỉ sửa bằng chứng
//! từ điều chỉnh / thay thế, hay huỷ (NĐ 123/2020 Điều 19 sửa bởi NĐ 70/2025 — cần đối chiếu).

use autoxide_kernel::time::{Date, Timestamp};
use std::sync::Arc;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::einvoice::{
    EInvoice, EInvoiceCancel, EInvoiceFormat, EInvoiceKind, EInvoiceLine, EInvoiceStatus,
    EInvoiceTaxTotal, Party, RelatedInvoice, Relation,
};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::numbering;
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, TenantId};
use autoxide_ledger::api::{AccountKey, LineAccount, NewEntry, NewLine, NewReversal};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect, Set};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{Buyer, Document, NewDocument, NewDocumentLine, is_sales_kind};
use crate::entity::{document, line, series};
use crate::events;
use crate::permissions::{CORRECT, ISSUE, READ};
use crate::service::{
    InvoicesService, describe, document_not_found, mandate_missing, money, now, retention_until,
    series_inactive, series_not_found,
};

pub(crate) const SUBMIT_EINVOICE: &str = "invoices.submit_einvoice";
pub(crate) const POLL_EINVOICES: &str = "invoices.poll_einvoice_status";

/// Job gửi một hoá đơn điện tử.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SubmitEInvoice {
    pub document_id: Uuid,
}

impl Job for SubmitEInvoice {
    const KIND: &'static str = SUBMIT_EINVOICE;
}

fn einvoice_kind(kind: &str) -> Option<EInvoiceKind> {
    match kind {
        "vat_invoice" => Some(EInvoiceKind::Vat),
        "sales_invoice" | "retail_receipt" => Some(EInvoiceKind::Sales),
        "cash_register_invoice" => Some(EInvoiceKind::CashRegister),
        "withholding_certificate" => Some(EInvoiceKind::WithholdingCertificate),
        _ => None,
    }
}

/// Một dòng địa chỉ từ JSON `Address` của hồ sơ thuế.
fn address_line(address: &Value) -> Option<String> {
    let parts: Vec<&str> = ["line", "commune_name", "province_name"]
        .iter()
        .filter_map(|key| address[key].as_str())
        .filter(|part| !part.is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

fn document_payload(row: &document::Model, symbol: Option<&str>) -> Result<Value, Error> {
    Ok(json!({
        "document_id": row.id, "org_id": row.org_id, "kind": row.kind, "symbol": symbol,
        "number": row.number, "total": money(row.total_minor, &row.currency)?,
        "source_module": row.source_module, "source_id": row.source_id,
    }))
}

/// Ảnh chụp tỉ giá của một chứng từ ngoại tệ.
pub(crate) struct FxSnapshot {
    pub rate: autoxide_kernel::decimal::Decimal,
    pub rate_id: Uuid,
    /// Tổng tiền của chứng từ quy ra tiền của sổ.
    pub base_total: Money,
}

/// Trạng thái của chứng từ còn thay thế / điều chỉnh được.
const CORRECTABLE: [&str; 3] = ["issued", "submitting", "accepted"];

fn not_correctable(status: &str) -> Error {
    Error::conflict(
        "invoices.not_correctable",
        "Only an issued document can be adjusted or replaced (this one is `{status}`).",
    )
    .with_param("status", status)
}

fn rate_missing(base: Currency, quote: Currency, on: Date) -> Error {
    Error::new(
        autoxide_kernel::error::ErrorKind::Validation,
        "invoices.rate_missing",
        "There is no {base}/{quote} exchange rate for {date}. Record the rate before issuing.",
    )
    .with_param("base", base.code())
    .with_param("quote", quote.code())
    .with_param("date", on.format("%Y-%m-%d").to_string())
}

impl InvoicesService {
    /// Tỉ giá cho chứng từ `row` lập ngày `on` khi tiền của chứng từ khác tiền của sổ người bán
    /// (Luật Kế toán 88/2015 Điều 11: ngoại tệ quy đổi ra đồng Việt Nam). Không có tỉ giá →
    /// `invoices.rate_missing`.
    async fn fx_snapshot(
        &self,
        tenant: &TenantContext,
        row: &document::Model,
        seller: OrgId,
        on: Date,
    ) -> Result<Option<FxSnapshot>, Error> {
        let currency = Currency::parse(&row.currency)?;
        let base = self.ledger.book_currency(tenant, seller).await?;
        if base == currency {
            return Ok(None);
        }
        let Some(found) = self.ledger.exchange_rate(tenant, currency, base, on).await? else {
            return Err(rate_missing(currency, base, on));
        };
        let rate = found.rate.0;
        let base_total = Money::new(row.total_minor, currency).convert(
            rate,
            base,
            autoxide_kernel::decimal::Rounding::HalfUp,
        )?;
        Ok(Some(FxSnapshot { rate, rate_id: found.id, base_total }))
    }

    /// Ảnh chụp người bán (hồ sơ thuế gần nhất trên cây).
    async fn seller_snapshot(&self, tenant: &TenantContext, seller: OrgId) -> Result<Value, Error> {
        Ok(match self.tax.profile(tenant, seller).await? {
            Some(profile) => json!({
                "org_id": seller.as_uuid(), "legal_name": profile.legal_name,
                "tax_code": profile.tax_code, "regime": profile.regime,
                "address": profile.address.as_ref().and_then(address_line),
            }),
            None => json!({"org_id": seller.as_uuid()}),
        })
    }

    /// Phát hành chứng từ `id` (`dyn Invoices`). Đã phát hành → trả nguyên; bị từ chối → gửi lại
    /// cùng số.
    pub(crate) async fn issue(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
    ) -> Result<Document, Error> {
        self.issue_inner(tenant, actor, id, None).await
    }

    /// Phát hành qua REST / MCP: cần `invoices.issue` ở nút lập.
    pub(crate) async fn issue_manual(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Document, Error> {
        self.document_for(tenant, principal, &ISSUE, id).await?;
        self.issue_inner(tenant, Actor::from(principal), id, Some(principal)).await
    }

    async fn issue_inner(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
        guard: Option<&Principal>,
    ) -> Result<Document, Error> {
        let database = tenant.database();
        let current = document::Entity::find_by_id(id)
            .scoped(tenant)
            .one(database.read())
            .await?
            .ok_or_else(document_not_found)?;
        let seller_org = OrgId::from_uuid(current.on_behalf_of_org_id.unwrap_or(current.org_id));
        // Ngoài transaction: ảnh chụp người bán (đọc database điều khiển qua `TaxEngine`).
        let seller = if current.status == "draft" {
            self.seller_snapshot(tenant, seller_org).await?
        } else {
            Value::Null
        };
        let today = Timestamp::now().date_utc();
        // Chứng từ ngoại tệ: chụp tỉ giá ngày lập (đọc `ledger` NGOÀI transaction ghi).
        let fx = if current.status == "draft" {
            self.fx_snapshot(tenant, &current, seller_org, current.issue_date.unwrap_or(today))
                .await?
        } else {
            None
        };
        let (outbox, jobs, seller, fx) = (&self.outbox, &self.jobs, &seller, &fx);
        let issued = database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let row = rows::lock_one(&transaction, document::Entity::find_by_id(id).scoped(tenant))
                    .await?
                    .ok_or_else(document_not_found)?;
                if let Some(principal) = guard {
                    let target = OrgTarget::new(OrgId::from_uuid(row.org_id), None);
                    principal.require_write_at(&transaction, &ISSUE, target).await?;
                }
                match row.status.as_str() {
                    "draft" => {}
                    "rejected" => {
                        // Gửi lại cùng số.
                        let mut active: document::ActiveModel = row.clone().into();
                        active.status = Set("issued".to_owned());
                        active.rejection_reason = Set(None);
                        active.updated_at = Set(now());
                        let saved = document::Entity::update(active).exec(&transaction).await?;
                        jobs.enqueue(
                            &transaction,
                            tenant,
                            &SubmitEInvoice { document_id: id },
                            EnqueueOptions::default().unique_key(format!("{id}:{}", saved.updated_at.as_unix_micros())),
                        )
                        .await?;
                        transaction.commit().await?;
                        return Ok(saved);
                    }
                    _ => return Ok(row),
                }
                let issue_date = row.issue_date.unwrap_or(today);
                let symbol_row = match row.series_id {
                    Some(series_id) => series::Entity::find_by_id(series_id)
                        .scoped(tenant)
                        .one(&transaction)
                        .await?
                        .ok_or_else(series_not_found)?,
                    None => {
                        let found = series::Entity::find()
                            .scoped(tenant)
                            .filter(series::Column::OrgId.eq(row.org_id))
                            .filter(series::Column::Kind.eq(row.kind.as_str()))
                            .filter(series::Column::Year.eq(issue_date.year()))
                            .filter(series::Column::Status.eq("active"))
                            .limit(2)
                            .all(&transaction)
                            .await?;
                        match found.as_slice() {
                            [only] => only.clone(),
                            [] => return Err(series_not_found()),
                            _ => {
                                return Err(Error::validation(vec![FieldError::new(
                                    "series_id",
                                    "required",
                                    "is required: the unit has several series for this kind and year",
                                )]));
                            }
                        }
                    }
                };
                if symbol_row.status != "active" {
                    return Err(series_inactive(&symbol_row.symbol));
                }
                if symbol_row.org_id != row.org_id
                    || symbol_row.kind != row.kind
                    || symbol_row.year != i32::from(issue_date.year())
                {
                    return Err(Error::validation(vec![FieldError::new(
                        "series_id",
                        "mismatch",
                        "must be a series of the same unit, kind and year as the document",
                    )]));
                }
                if let Some(principal_org) = row.on_behalf_of_org_id
                    && !Self::has_mandate(&transaction, tenant, principal_org, row.org_id, &row.kind, issue_date).await?
                {
                    return Err(mandate_missing());
                }
                let number = numbering::next(&transaction, tenant.id(), &format!("invoices:{}", symbol_row.id)).await?;
                let at = now();
                let mut active: document::ActiveModel = row.clone().into();
                active.series_id = Set(Some(symbol_row.id));
                active.number = Set(Some(number));
                active.status = Set("issued".to_owned());
                active.issue_date = Set(Some(issue_date));
                active.retention_until = Set(Some(retention_until(issue_date)));
                active.seller = Set(seller.to_string());
                if let Some(fx) = fx {
                    active.exchange_rate = Set(Some(autoxide_kernel::decimal::to_column(fx.rate)));
                    active.exchange_rate_id = Set(Some(fx.rate_id));
                    active.base_currency = Set(Some(fx.base_total.currency().code().to_owned()));
                    active.base_total_minor = Set(Some(fx.base_total.minor()));
                }
                active.issued_by = Set(actor.user_id);
                active.issued_at = Set(Some(at));
                active.updated_at = Set(at);
                let saved = document::Entity::update(active).exec(&transaction).await?;
                // Chứng từ sửa sai: đánh dấu chứng từ gốc.
                if let (Some(original), Some(relation)) = (saved.related_document_id, saved.relation.as_deref()) {
                    let status = if relation == "replacement" { "replaced" } else { "adjusted" };
                    // Chỉ đánh dấu khi gốc còn sửa được (D-181): hai nháp thay thế cùng một chứng
                    // từ, hay thay thế đua với huỷ, thì bên sau thấy gốc đã `replaced` /
                    // `cancelled` và không phát hành — không thì doanh thu bị ghi hai lần. Câu
                    // `UPDATE` có điều kiện khoá dòng gốc trên Postgres: bên sau chờ bên trước
                    // commit rồi xét lại điều kiện.
                    let corrected = document::Entity::update_many()
                        .col_expr(document::Column::Status, status.into())
                        .col_expr(document::Column::UpdatedAt, at.into())
                        .filter(document::Column::Id.eq(original))
                        .filter(document::Column::TenantId.eq(tenant.id().as_uuid()))
                        .filter(document::Column::Status.is_in(CORRECTABLE))
                        .exec(&transaction)
                        .await?;
                    if corrected.rows_affected == 0 {
                        let now_status = document::Entity::find_by_id(original)
                            .filter(document::Column::TenantId.eq(tenant.id().as_uuid()))
                            .one(&transaction)
                            .await?
                            .map(|row| row.status)
                            .unwrap_or_default();
                        return Err(not_correctable(&now_status));
                    }
                    let event = events::DocumentCorrected(json!({
                        "document_id": original, "relation": relation, "by_document_id": saved.id,
                    }));
                    outbox.record(&transaction, tenant, actor, Some(format!("invoices.document:{original}")), &event).await?;
                }
                let event = events::DocumentIssued(document_payload(&saved, Some(&symbol_row.symbol))?);
                outbox.record(&transaction, tenant, actor, Some(format!("invoices.document:{id}")), &event).await?;
                if symbol_row.einvoice_provider.is_some() {
                    jobs.enqueue(
                        &transaction,
                        tenant,
                        &SubmitEInvoice { document_id: id },
                        EnqueueOptions::default().unique_key(id.to_string()),
                    )
                    .await?;
                }
                transaction.commit().await?;
                Ok(saved)
            })
            .await?;
        self.post_issue(tenant, &issued).await?;
        describe(database.write(), tenant, issued).await
    }

    /// Ghi sổ cho chứng từ bán hàng đã phát hành (gọi lại không ghi trùng). Chứng từ thay thế đảo
    /// bút toán của chứng từ gốc trước.
    async fn post_issue(&self, tenant: &TenantContext, row: &document::Model) -> Result<(), Error> {
        if row.relation.as_deref() == Some("replacement")
            && let Some(original) = row.related_document_id
        {
            self.reverse_posting(tenant, original, &format!("invoices:{original}:replaced"))
                .await?;
        }
        if let Some(entry) = issue_entry(row)? {
            self.ledger.post(tenant, Actor::system(), entry).await?;
        }
        Ok(())
    }

    /// Đảo bút toán phát hành của chứng từ `original` (huỷ, thay thế).
    async fn reverse_posting(
        &self,
        tenant: &TenantContext,
        original: Uuid,
        reference: &str,
    ) -> Result<(), Error> {
        let found = document::Entity::find_by_id(original)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(document_not_found)?;
        // Cùng `reference` + cùng nội dung → `dyn Ledger::post` trả lại bút toán đã ghi (không ghi
        // thêm): lấy ID bút toán gốc mà không đọc bảng của `ledger`.
        let Some(entry) = issue_entry(&found)? else { return Ok(()) };
        let posted = self.ledger.post(tenant, Actor::system(), entry).await?;
        let entry = match posted {
            Submitted::Created(entry) | Submitted::Replayed(entry) => entry.id,
        };
        match self
            .ledger
            .reverse(tenant, Actor::system(), entry, NewReversal::new("invoices", reference))
            .await
        {
            Ok(_) => Ok(()),
            Err(error) if error.code() == "ledger.already_reversed" => Ok(()),
            Err(error) => Err(error),
        }
    }

    // ---- Hoá đơn điện tử ------------------------------------------------------------------------

    /// Dựng hoá đơn điện tử từ chứng từ đã phát hành.
    async fn einvoice_of(
        &self,
        tenant: &TenantContext,
        row: &document::Model,
        symbol: &str,
    ) -> Result<EInvoice, Error> {
        let Some(kind) = einvoice_kind(&row.kind) else {
            return Err(Error::internal("this document kind has no e-invoice"));
        };
        let read = tenant.database().read();
        let currency = Currency::parse(&row.currency)?;
        let seller: Value = serde_json::from_str(&row.seller).unwrap_or_default();
        let party = |value: &Value| {
            Party::new(value["legal_name"].as_str().unwrap_or_default())
                .tax_code(value["tax_code"].as_str().map(str::to_owned))
                .address(value["address"].as_str().map(str::to_owned))
        };
        let buyer: Buyer = serde_json::from_str(&row.buyer).unwrap_or_default();
        let buyer = Party::new(buyer.name.unwrap_or_default())
            .tax_code(buyer.tax_code)
            .address(buyer.address)
            .email(buyer.email);
        let lines = line::Entity::find()
            .scoped(tenant)
            .filter(line::Column::DocumentId.eq(row.id))
            .all(read)
            .await?;
        let mut items = Vec::with_capacity(lines.len());
        let mut totals: Vec<EInvoiceTaxTotal> = Vec::new();
        for item in lines.iter().filter(|item| item.kind != "note") {
            let net = Money::new(item.net_minor, currency);
            let tax = Money::new(item.tax_minor, currency);
            items.push(
                EInvoiceLine::new(
                    item.line_no,
                    &item.description,
                    &item.quantity,
                    Money::new(item.unit_price_minor, currency),
                    net,
                    tax,
                )
                .unit(item.unit.clone())
                .rate_percent(item.rate_percent.clone()),
            );
            match totals.iter_mut().find(|total| total.rate_percent == item.rate_percent) {
                Some(total) => {
                    total.net = total.net.checked_add(net)?;
                    total.tax = total.tax.checked_add(tax)?;
                }
                None => totals.push(EInvoiceTaxTotal::new(item.rate_percent.clone(), net, tax)),
            }
        }
        let (seller_party, on_behalf_of) = match row.on_behalf_of_org_id {
            // Lập thay: bên lập là nút lập (sàn); người bán là bên uỷ nhiệm.
            Some(_) => {
                let agent = self.seller_snapshot(tenant, OrgId::from_uuid(row.org_id)).await?;
                (party(&agent), Some(party(&seller)))
            }
            None => (party(&seller), None),
        };
        let related = match (row.related_document_id, row.relation.as_deref()) {
            (Some(original), Some(relation)) => {
                let found = document::Entity::find_by_id(original).scoped(tenant).one(read).await?;
                match found {
                    Some(found) => {
                        let symbol = match found.series_id {
                            Some(id) => series::Entity::find_by_id(id)
                                .scoped(tenant)
                                .one(read)
                                .await?
                                .map(|row| row.symbol),
                            None => None,
                        };
                        let relation = if relation == "replacement" {
                            Relation::Replacement
                        } else {
                            Relation::Adjustment
                        };
                        Some(
                            RelatedInvoice::new(
                                relation,
                                symbol.unwrap_or_default(),
                                found.number.unwrap_or_default(),
                                found.issue_date.unwrap_or_default(),
                            )
                            .provider_ref(found.provider_ref),
                        )
                    }
                    None => None,
                }
            }
            _ => None,
        };
        Ok(EInvoice::new(
            row.id,
            kind,
            seller_party,
            buyer,
            row.issue_date.unwrap_or_default(),
            currency,
        )
        .series(symbol, row.number)
        .lines(items, totals)
        .on_behalf_of(on_behalf_of)
        .related(related))
    }

    /// Gửi hoá đơn điện tử của chứng từ `id` (job), rồi hỏi trạng thái ngay một lần.
    pub(crate) async fn submit_einvoice(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<(), Error> {
        let database = tenant.database();
        let Some(row) =
            document::Entity::find_by_id(id).scoped(tenant).one(database.write()).await?
        else {
            return Ok(());
        };
        if row.status != "issued" {
            return Ok(());
        }
        let Some(found) = (match row.series_id {
            Some(series_id) => {
                series::Entity::find_by_id(series_id).scoped(tenant).one(database.read()).await?
            }
            None => None,
        }) else {
            return Ok(());
        };
        let Some(name) = &found.einvoice_provider else { return Ok(()) };
        let provider = self.provider(name)?;
        let invoice = self.einvoice_of(tenant, &row, &found.symbol).await?;
        let answer = provider.issue(tenant.id(), &invoice).await?;
        document::Entity::update_many()
            .col_expr(document::Column::Status, "submitting".into())
            .col_expr(document::Column::ProviderRef, Some(answer.provider_ref.clone()).into())
            .col_expr(document::Column::ProviderNumber, answer.number.into())
            .col_expr(document::Column::LookupCode, answer.lookup_code.clone().into())
            .col_expr(document::Column::AuthorityCode, answer.authority_code.clone().into())
            .col_expr(document::Column::UpdatedAt, now().into())
            .filter(document::Column::Id.eq(id))
            .filter(document::Column::Status.eq("issued"))
            .exec(database.write())
            .await?;
        self.check_status(tenant, id).await
    }

    /// Hỏi nhà cung cấp ngay (REST, `invoices.issue`) thay vì chờ lịch hỏi định kỳ. Chứng từ không ở
    /// `submitting` thì trả nguyên — không có gì để hỏi.
    pub(crate) async fn refresh_manual(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Document, Error> {
        self.document_for(tenant, principal, &ISSUE, id).await?;
        self.check_status(tenant, id).await?;
        self.document_detail(tenant, principal, id).await
    }

    /// Hỏi nhà cung cấp trạng thái của chứng từ `submitting` `id`.
    pub(crate) async fn check_status(&self, tenant: &TenantContext, id: Uuid) -> Result<(), Error> {
        let database = tenant.database();
        let Some(row) =
            document::Entity::find_by_id(id).scoped(tenant).one(database.write()).await?
        else {
            return Ok(());
        };
        let (Some(provider_ref), Some(series_id)) = (row.provider_ref.clone(), row.series_id)
        else {
            return Ok(());
        };
        if row.status != "submitting" {
            return Ok(());
        }
        let Some(found) =
            series::Entity::find_by_id(series_id).scoped(tenant).one(database.read()).await?
        else {
            return Ok(());
        };
        let Some(name) = found.einvoice_provider else { return Ok(()) };
        let status = self.provider(&name)?.status(tenant.id(), &provider_ref).await?;
        let (next, authority, reason) = match status {
            EInvoiceStatus::Pending => return Ok(()),
            EInvoiceStatus::Accepted { authority_code } => ("accepted", authority_code, None),
            EInvoiceStatus::Rejected { reason } => ("rejected", None, Some(reason)),
        };
        let outbox = &self.outbox;
        let (authority, reason) = (&authority, &reason);
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let Some(row) =
                    rows::lock_one(&transaction, document::Entity::find_by_id(id).scoped(tenant))
                        .await?
                else {
                    return Ok(());
                };
                if row.status != "submitting" {
                    return Ok(());
                }
                let mut active: document::ActiveModel = row.clone().into();
                active.status = Set(next.to_owned());
                if authority.is_some() {
                    active.authority_code = Set(authority.clone());
                }
                active.rejection_reason = Set(reason.clone());
                active.updated_at = Set(now());
                document::Entity::update(active).exec(&transaction).await?;
                let subject = Some(format!("invoices.document:{id}"));
                if next == "accepted" {
                    let event = events::DocumentAccepted(
                        json!({"document_id": id, "authority_code": authority}),
                    );
                    outbox.record(&transaction, tenant, Actor::system(), subject, &event).await?;
                } else {
                    let event =
                        events::DocumentRejected(json!({"document_id": id, "reason": reason}));
                    outbox.record(&transaction, tenant, Actor::system(), subject, &event).await?;
                }
                transaction.commit().await?;
                Ok(())
            })
            .await
    }

    /// Lịch: hỏi trạng thái mọi chứng từ `submitting` trên `database`.
    pub(crate) async fn poll_einvoices(
        &self,
        database: &autoxide_kernel::database::Database,
    ) -> Result<u64, Error> {
        let pending: Vec<(Uuid, Uuid)> = document::Entity::find()
            .select_only()
            .column(document::Column::TenantId)
            .column(document::Column::Id)
            .filter(document::Column::Status.eq("submitting"))
            .limit(200)
            .into_tuple()
            .all(database.read())
            .await?;
        let mut checked = 0;
        for (tenant, id) in pending {
            let tenant = self.databases.tenant(TenantId::from_uuid(tenant)).await?;
            if let Err(error) = self.check_status(&tenant, id).await {
                tracing::warn!(document_id = %id, error = %error, "e-invoice status check failed");
            }
            checked += 1;
        }
        Ok(checked)
    }

    /// Tải hoá đơn điện tử (XML / PDF) từ nhà cung cấp.
    pub(crate) async fn download_einvoice(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        format: EInvoiceFormat,
    ) -> Result<axum::body::Bytes, Error> {
        let row = self.document_for(tenant, principal, &READ, id).await?;
        let (Some(provider_ref), Some(series_id)) = (row.provider_ref, row.series_id) else {
            return Err(Error::conflict(
                "invoices.einvoice_not_ready",
                "The e-invoice is not sent yet (status `{status}`).",
            )
            .with_param("status", row.status));
        };
        let found = series::Entity::find_by_id(series_id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(series_not_found)?;
        let name = found.einvoice_provider.unwrap_or_default();
        self.provider(&name)?.download(tenant.id(), &provider_ref, format).await
    }

    // ---- Huỷ, điều chỉnh, thay thế ---------------------------------------------------------------

    /// Huỷ chứng từ đã phát hành (`dyn Invoices`).
    pub(crate) async fn cancel(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
        reason: String,
    ) -> Result<Document, Error> {
        let reason = reason.trim().to_owned();
        if !(1..=500).contains(&reason.chars().count()) {
            return Err(Error::validation(vec![FieldError::new(
                "reason",
                "length",
                "must be 1 to 500 characters long",
            )]));
        }
        let database = tenant.database();
        let row = document::Entity::find_by_id(id)
            .scoped(tenant)
            .one(database.read())
            .await?
            .ok_or_else(document_not_found)?;
        match row.status.as_str() {
            "cancelled" => return describe(database.read(), tenant, row).await,
            "issued" | "submitting" | "accepted" | "rejected" => {}
            _ => {
                return Err(Error::conflict(
                    "invoices.not_cancellable",
                    "A document in status `{status}` cannot be cancelled.",
                )
                .with_param("status", row.status));
            }
        }
        // Ngoài transaction: báo nhà cung cấp.
        if let (Some(provider_ref), Some(series_id)) = (&row.provider_ref, row.series_id)
            && let Some(found) =
                series::Entity::find_by_id(series_id).scoped(tenant).one(database.read()).await?
            && let Some(name) = found.einvoice_provider
        {
            self.provider(&name)?
                .cancel(tenant.id(), &EInvoiceCancel::new(provider_ref.clone(), reason.clone()))
                .await?;
        }
        let outbox = &self.outbox;
        let saved = database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let row = rows::lock_one(&transaction, document::Entity::find_by_id(id).scoped(tenant))
                    .await?
                    .ok_or_else(document_not_found)?;
                if row.status == "cancelled" {
                    return Ok(row);
                }
                let mut active: document::ActiveModel = row.into();
                active.status = Set("cancelled".to_owned());
                active.updated_at = Set(now());
                let saved = document::Entity::update(active).exec(&transaction).await?;
                let event = events::DocumentCancelled(json!({
                    "document_id": id, "source_module": saved.source_module, "source_id": saved.source_id,
                }));
                outbox.record(&transaction, tenant, actor, Some(format!("invoices.document:{id}")), &event).await?;
                transaction.commit().await?;
                Ok(saved)
            })
            .await?;
        self.reverse_posting(tenant, id, &format!("invoices:{id}:cancelled")).await?;
        describe(database.write(), tenant, saved).await
    }

    /// Huỷ qua REST / MCP: cần `invoices.correct`.
    pub(crate) async fn cancel_manual(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        reason: String,
    ) -> Result<Document, Error> {
        self.document_for(tenant, principal, &CORRECT, id).await?;
        self.cancel(tenant, Actor::from(principal), id, reason).await
    }

    /// Lập nháp điều chỉnh (`relation = adjustment`, `direction` tăng / giảm) hay thay thế chứng từ
    /// đã phát hành `id`. Phát hành nháp đó mới đổi chứng từ gốc.
    pub(crate) async fn correct(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        correction: Correction,
    ) -> Result<Submitted<Document>, Error> {
        let original = self.document_for(tenant, principal, &CORRECT, id).await?;
        self.correct_document(tenant, Actor::from(principal), original, correction).await
    }

    /// Lập nháp sửa sai của chứng từ `original` (đã kiểm quyền ở bên gọi): REST qua [`correct`],
    /// module khác qua `dyn Invoices::adjust`. Nháp kế thừa cờ `booked_by_source` của chứng từ gốc.
    pub(crate) async fn correct_document(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        original: document::Model,
        correction: Correction,
    ) -> Result<Submitted<Document>, Error> {
        let Correction { relation, direction, buyer, lines, reference } = correction;
        if !CORRECTABLE.contains(&original.status.as_str()) {
            return Err(not_correctable(&original.status));
        }
        let direction = direction.unwrap_or_else(|| "increase".to_owned());
        if relation == "adjustment" && !["increase", "decrease"].contains(&direction.as_str()) {
            return Err(Error::validation(vec![FieldError::new(
                "direction",
                "invalid",
                "must be `increase` or `decrease`",
            )]));
        }
        let buyer =
            buyer.unwrap_or_else(|| serde_json::from_str(&original.buyer).unwrap_or_default());
        let mut draft = NewDocument::new(
            OrgId::from_uuid(original.org_id),
            original.kind.clone(),
            Currency::parse(&original.currency)?,
            buyer,
            lines,
            "invoices",
            reference,
        )
        .source_id(original.id.to_string());
        draft.on_behalf_of = original.on_behalf_of_org_id.map(OrgId::from_uuid);
        draft.series = original.series_id;
        draft.prices_include_tax = original.prices_include_tax;
        draft.booked_by_source = original.booked_by_source;
        let direction = if relation == "replacement" { "increase".to_owned() } else { direction };
        self.create_inner(tenant, actor, draft, Some((original.id, relation, direction))).await
    }
}

/// Bút toán phát hành của chứng từ bán hàng `row`: Nợ `receivable.customer` / Có `revenue.sales`,
/// `tax.vat_output` (chứng từ điều chỉnh giảm thì ngược lại), vào sổ của người bán. `None`: chứng từ
/// không ghi sổ (không phải chứng từ bán hàng, chưa phát hành, tổng 0).
fn issue_entry(row: &document::Model) -> Result<Option<NewEntry>, Error> {
    // Bên lập (vd `orders`) đã tự ghi doanh thu theo nghiệp vụ của nó: chứng từ không ghi lần hai.
    if row.booked_by_source
        || !is_sales_kind(&row.kind)
        || row.number.is_none()
        || row.total_minor <= 0
    {
        return Ok(None);
    }
    let currency = Currency::parse(&row.currency)?;
    let (subtotal, tax, total) = (
        Money::new(row.subtotal_minor, currency),
        Money::new(row.tax_minor, currency),
        Money::new(row.total_minor, currency),
    );
    let book = OrgId::from_uuid(row.on_behalf_of_org_id.unwrap_or(row.org_id));
    let account = |purpose: &str| LineAccount::Key(AccountKey::new(book, purpose, currency));
    let decrease = row.direction == "decrease";
    let side = |debit: bool, account: LineAccount, amount: Money| {
        if debit != decrease {
            NewLine::debit(account, amount)
        } else {
            NewLine::credit(account, amount)
        }
    };
    let mut lines = vec![side(true, account("receivable.customer"), total)];
    if subtotal.is_positive() {
        lines.push(side(false, account("revenue.sales"), subtotal));
    }
    if tax.is_positive() {
        lines.push(side(false, account("tax.vat_output"), tax));
    }
    let mut entry = NewEntry::new(
        book,
        "invoices",
        "document",
        format!("invoices:{}:issued", row.id),
        "Sales document issued",
        lines,
    )
    .source_id(row.id.to_string());
    if let Some(day) = row.issue_date {
        entry = entry.on(day);
    }
    // Chứng từ ngoại tệ: bút toán quy đổi bằng đúng tỉ giá đã chụp trên chứng từ.
    if let Some(rate) = row.exchange_rate.as_deref() {
        entry = entry.exchange_rate(autoxide_kernel::decimal::from_column(rate)?);
    }
    Ok(Some(entry))
}

/// Chứng từ sửa sai cần lập.
pub(crate) struct Correction {
    /// `adjustment` hay `replacement`.
    pub relation: &'static str,
    /// Chứng từ điều chỉnh: `increase` hay `decrease`.
    pub direction: Option<String>,
    pub buyer: Option<Buyer>,
    pub lines: Vec<NewDocumentLine>,
    pub reference: String,
}

/// Job `invoices.submit_einvoice`.
pub(crate) struct SubmitJob(pub Arc<InvoicesService>);

#[async_trait::async_trait]
impl JobHandler for SubmitJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let payload: SubmitEInvoice =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        let tenant = job
            .tenant
            .as_ref()
            .ok_or_else(|| Error::internal("invoices.submit_einvoice needs a tenant"))?;
        self.0.submit_einvoice(tenant, payload.document_id).await
    }
}

/// Lịch `invoices.poll_einvoice_status` (5 phút, mọi database).
pub(crate) struct PollJob(pub Arc<InvoicesService>);

#[async_trait::async_trait]
impl JobHandler for PollJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.poll_einvoices(&job.database).await.map(|_| ())
    }
}
