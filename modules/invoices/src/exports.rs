//! Xuất dữ liệu kế toán theo kỳ (bước 36 mục 12, D-354): sổ nhật ký chung và chứng từ ra CSV / XLSX
//! để kế toán của khách nhập vào phần mềm kế toán, không gõ lại tay.
//!
//! - `POST /exports` ghi một dòng `invoices_exports` (`pending`) và xếp job `invoices.build_export`
//!   trong cùng transaction; job đọc sổ qua `dyn Ledger::journal` (từng trang) và chứng từ của kỳ,
//!   ghi file vào nơi lưu của khách (`invoices/exports/<id>.<csv|xlsx>`), rồi `ready` + event
//!   `invoices.export_ready`; lỗi → `failed` (yêu cầu bản mới). Tải bằng `GET /exports/{id}/download`.
//! - Hồ sơ xuất là dữ liệu ([`PROFILES`]): `generic_journal` (mỗi dòng bút toán một hàng, cột Nợ / Có),
//!   `vn_double_entry` (định khoản theo cặp TK Nợ / TK Có / Số tiền — hình dạng của chứng từ nghiệp vụ
//!   khác mà phần mềm kế toán Việt Nam phổ biến nhập được qua bước ghép cột; **chưa đối chiếu** được
//!   mẫu nhập của từng hãng — D-354), `generic_documents` (mỗi dòng chứng từ một hàng).
//! - CSV: UTF-8 có BOM (Excel đọc đúng tiếng Việt), RFC 4180, CRLF, ô bắt đầu bằng `= + - @` tab CR
//!   thêm `'` (chặn CSV injection). Số tiền là số thập phân theo đơn vị chính, dấu chấm, không nhóm.
//! - XLSX: `rust_xlsxwriter`; số tiền là ô số (Excel lưu số thực 64 bit — đúng tuyệt đối tới
//!   9×10^15 đơn vị nhỏ nhất; lớn hơn thì ghi dạng chữ).

use std::collections::HashMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{Order, Page, PageRequest, fetch_page};
use autoxide_kernel::storage::{Storage as _, StoredFile, TenantStorage};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use autoxide_ledger::api::{JournalLine, JournalQuery, Side};
use bytes::Bytes;
use chrono::NaiveDate;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::entity::{document, export, line, series};
use crate::events;
use crate::permissions::EXPORT;
use crate::service::{InvoicesService, now};

pub(crate) const BUILD_EXPORT: &str = "invoices.build_export";

/// Hồ sơ xuất: tên, mô tả (tiếng Anh, cho API), đã đối chiếu với mẫu công bố của phần mềm nào chưa.
pub(crate) const PROFILES: [(&str, &str); 3] = [
    (
        "generic_journal",
        "General journal: one row per entry line with debit and credit columns, account number, \
         currency, rate and amount in the book currency.",
    ),
    (
        "vn_double_entry",
        "General journal as debit/credit pairs (TK Nợ, TK Có, Số tiền), the shape Vietnamese \
         accounting software imports as general vouchers through column mapping. Not yet checked \
         against each vendor's published import template.",
    ),
    (
        "generic_documents",
        "Documents of the period: one row per document line with series, number, buyer, item, \
         quantity, price, VAT rate and amounts.",
    ),
];

pub(crate) const FORMATS: [&str; 2] = ["csv", "xlsx"];
/// Kỳ xuất dài nhất (ngày).
const MAX_DAYS: i64 = 366;

/// Payload của job: chỉ ID.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct BuildExport {
    pub export_id: Uuid,
}

impl Job for BuildExport {
    const KIND: &'static str = BUILD_EXPORT;
}

pub(crate) fn export_not_found() -> Error {
    Error::not_found("invoices.export_not_found", "There is no such export.")
}

fn export_not_ready(status: &str) -> Error {
    Error::conflict("invoices.export_not_ready", "The export is not ready (status `{status}`).")
        .with_param("status", status)
}

pub(crate) fn storage_unavailable() -> Error {
    Error::unavailable(
        "invoices.storage_unavailable",
        "Exports and archives need file storage; the `files` module is not installed.",
    )
}

/// Một ô của bảng xuất.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Cell {
    Text(String),
    /// Số thập phân dạng chữ (`-1234.50`).
    Number(String),
}

impl Cell {
    fn text(value: impl Into<String>) -> Cell {
        Cell::Text(value.into())
    }

    fn as_str(&self) -> &str {
        match self {
            Cell::Text(text) | Cell::Number(text) => text,
        }
    }
}

/// Số tiền theo đơn vị chính, dấu chấm, không nhóm: `1999 USD` → `19.99`, `50000 VND` → `50000`.
pub(crate) fn amount(money: Money) -> String {
    money.to_decimal().to_string()
}

/// Một yêu cầu xuất.
#[derive(Debug, Clone)]
pub(crate) struct ExportInput {
    pub org: Option<OrgId>,
    pub profile: String,
    pub format: String,
    pub from: NaiveDate,
    pub to: NaiveDate,
}

// ---- Ghi file -----------------------------------------------------------------------------------

/// CSV: BOM, mọi ô trong ngoặc kép, chặn công thức.
pub(crate) fn to_csv(header: &[&str], rows: &[Vec<Cell>]) -> Vec<u8> {
    let cell = |value: &str| {
        let guarded = if value.starts_with(['=', '+', '-', '@', '\t', '\r']) {
            format!("'{value}")
        } else {
            value.to_owned()
        };
        format!("\"{}\"", guarded.replace('"', "\"\""))
    };
    let mut out = String::from("\u{feff}");
    out.push_str(&header.iter().map(|title| cell(title)).collect::<Vec<_>>().join(","));
    out.push_str("\r\n");
    for row in rows {
        // Số âm là số, không phải công thức: không thêm `'`.
        let cells: Vec<String> = row
            .iter()
            .map(|item| match item {
                Cell::Number(text) => format!("\"{text}\""),
                Cell::Text(text) => cell(text),
            })
            .collect();
        out.push_str(&cells.join(","));
        out.push_str("\r\n");
    }
    out.into_bytes()
}

/// XLSX: một trang, hàng đầu là tiêu đề.
pub(crate) fn to_xlsx(sheet: &str, header: &[&str], rows: &[Vec<Cell>]) -> Result<Vec<u8>, Error> {
    let fail = |error: rust_xlsxwriter::XlsxError| Error::internal(format!("xlsx: {error}"));
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name(sheet).map_err(fail)?;
    for (col, title) in header.iter().enumerate() {
        let col = u16::try_from(col).map_err(Error::internal)?;
        worksheet.write_string(0, col, *title).map_err(fail)?;
    }
    for (index, row) in rows.iter().enumerate() {
        let row_no = u32::try_from(index + 1).map_err(Error::internal)?;
        for (col, item) in row.iter().enumerate() {
            let col = u16::try_from(col).map_err(Error::internal)?;
            // Số tiền đúng tuyệt đối khi |x| < 2^53 đơn vị; lớn hơn ghi dạng chữ để không lệch.
            let exact = match item {
                Cell::Number(text) => text
                    .replace(['-', '.'], "")
                    .parse::<u64>()
                    .ok()
                    .filter(|digits| *digits < (1u64 << 53))
                    .and_then(|_| text.parse::<f64>().ok()),
                Cell::Text(_) => None,
            };
            match exact {
                Some(number) => worksheet.write_number(row_no, col, number).map_err(fail)?,
                None => worksheet.write_string(row_no, col, item.as_str()).map_err(fail)?,
            };
        }
    }
    workbook.save_to_buffer().map_err(fail)
}

/// Tiêu đề cột của sổ nhật ký chung (`generic_journal`).
pub(crate) const JOURNAL_HEADER: [&str; 15] = [
    "Ngày hạch toán (posting date)",
    "Số bút toán (entry id)",
    "Ngày chứng từ (document date)",
    "Số chứng từ (document number)",
    "Diễn giải (description)",
    "Tài khoản (account)",
    "Nợ (debit)",
    "Có (credit)",
    "Loại tiền (currency)",
    "Tỉ giá (rate)",
    "Nợ quy đổi (debit, book currency)",
    "Có quy đổi (credit, book currency)",
    "Đối tượng (counterparty)",
    "Nguồn (source)",
    "Tham chiếu (reference)",
];

/// Tiêu đề cột của định khoản theo cặp (`vn_double_entry`).
pub(crate) const PAIRS_HEADER: [&str; 12] = [
    "Ngày hạch toán",
    "Ngày chứng từ",
    "Số chứng từ",
    "Diễn giải",
    "TK Nợ",
    "TK Có",
    "Số tiền",
    "Loại tiền",
    "Tỉ giá",
    "Số tiền quy đổi",
    "Đối tượng Nợ",
    "Đối tượng Có",
];

/// Tiêu đề cột của chứng từ (`generic_documents`).
pub(crate) const DOCUMENTS_HEADER: [&str; 20] = [
    "Ngày chứng từ (date)",
    "Loại (kind)",
    "Ký hiệu (series)",
    "Số (number)",
    "Trạng thái (status)",
    "Người mua (buyer)",
    "MST người mua (buyer tax code)",
    "Địa chỉ (address)",
    "Mã hàng (item)",
    "Tên hàng (description)",
    "ĐVT (unit)",
    "Số lượng (quantity)",
    "Đơn giá (unit price)",
    "Thành tiền (amount)",
    "Thuế suất (VAT %)",
    "Tiền thuế (VAT)",
    "Tổng (total)",
    "Loại tiền (currency)",
    "Tỉ giá (rate)",
    "Mã tra cứu (lookup code)",
];

/// Số / ngày của chứng từ nguồn (khi bút toán do `invoices` ghi).
type DocumentRefs = HashMap<String, (String, String)>;

fn party(line: &JournalLine) -> String {
    match (&line.counterparty_kind, &line.counterparty_id) {
        (Some(kind), Some(id)) => format!("{kind}:{id}"),
        _ => String::new(),
    }
}

fn document_ref(refs: &DocumentRefs, line: &JournalLine) -> (String, String) {
    line.source_id
        .as_ref()
        .filter(|_| line.source_module == "invoices")
        .and_then(|id| refs.get(id).cloned())
        .unwrap_or_default()
}

/// Hàng của `generic_journal`: mỗi dòng bút toán một hàng.
pub(crate) fn journal_rows(lines: &[JournalLine], refs: &DocumentRefs) -> Vec<Vec<Cell>> {
    lines
        .iter()
        .map(|line| {
            let (doc_date, doc_number) = document_ref(refs, line);
            let (debit, credit, base_debit, base_credit) = match line.side {
                Side::Debit => {
                    (amount(line.amount), String::new(), amount(line.base_amount), String::new())
                }
                Side::Credit => {
                    (String::new(), amount(line.amount), String::new(), amount(line.base_amount))
                }
            };
            let number =
                |text: String| if text.is_empty() { Cell::Text(text) } else { Cell::Number(text) };
            vec![
                Cell::text(line.effective_date.format("%Y-%m-%d").to_string()),
                Cell::text(line.entry_id.to_string()),
                Cell::text(doc_date),
                Cell::text(doc_number),
                Cell::text(line.description.clone()),
                Cell::text(line.account_code.clone()),
                number(debit),
                number(credit),
                Cell::text(line.amount.currency().code()),
                Cell::text(line.exchange_rate.clone().unwrap_or_default()),
                number(base_debit),
                number(base_credit),
                Cell::text(party(line)),
                Cell::text(format!("{}:{}", line.source_module, line.source_kind)),
                Cell::text(line.reference.clone()),
            ]
        })
        .collect()
}

/// Hàng của `vn_double_entry`: mỗi bút toán thành các cặp (TK Nợ, TK Có, số tiền) — ghép dần dòng
/// nợ với dòng có theo thứ tự, mỗi cặp lấy phần nhỏ hơn; tổng các cặp = tổng nợ = tổng có (ở cả tiền
/// gốc và tiền sổ).
pub(crate) fn pair_rows(lines: &[JournalLine], refs: &DocumentRefs) -> Vec<Vec<Cell>> {
    let mut entries: Vec<(Uuid, Vec<&JournalLine>)> = Vec::new();
    for line in lines {
        match entries.iter_mut().find(|(id, _)| *id == line.entry_id) {
            Some((_, group)) => group.push(line),
            None => entries.push((line.entry_id, vec![line])),
        }
    }
    let mut rows = Vec::new();
    for (_, group) in entries {
        let pick = |side: Side| -> Vec<(&JournalLine, i64, i64)> {
            group
                .iter()
                .filter(|line| line.side == side)
                .map(|line| (*line, line.amount.minor(), line.base_amount.minor()))
                .collect()
        };
        let (mut debits, mut credits) = (pick(Side::Debit), pick(Side::Credit));
        let (mut d, mut c) = (0, 0);
        while d < debits.len() && c < credits.len() {
            let take = debits[d].1.min(credits[c].1);
            // Tiền sổ: dòng hết trước thì lấy hết phần tiền sổ còn lại của nó (không làm tròn lại).
            let base = if debits[d].1 == take {
                debits[d].2
            } else if credits[c].1 == take {
                credits[c].2
            } else {
                0
            };
            let (debit, credit) = (debits[d].0, credits[c].0);
            let (doc_date, doc_number) = document_ref(refs, debit);
            rows.push(vec![
                Cell::text(debit.effective_date.format("%Y-%m-%d").to_string()),
                Cell::text(doc_date),
                Cell::text(doc_number),
                Cell::text(debit.description.clone()),
                Cell::text(debit.account_code.clone()),
                Cell::text(credit.account_code.clone()),
                Cell::Number(amount(Money::new(take, debit.amount.currency()))),
                Cell::text(debit.amount.currency().code()),
                Cell::text(debit.exchange_rate.clone().unwrap_or_default()),
                Cell::Number(amount(Money::new(base, debit.base_amount.currency()))),
                Cell::text(party(debit)),
                Cell::text(party(credit)),
            ]);
            debits[d].1 -= take;
            credits[c].1 -= take;
            debits[d].2 -= base;
            credits[c].2 -= base;
            if debits[d].1 == 0 {
                d += 1;
            }
            if credits[c].1 == 0 {
                c += 1;
            }
        }
    }
    rows
}

impl InvoicesService {
    fn store(&self, tenant: &TenantContext) -> Result<TenantStorage, Error> {
        let shared = self.storage.clone().ok_or_else(storage_unavailable)?;
        Ok(TenantStorage::new(shared, tenant))
    }

    /// Yêu cầu một bản xuất (`invoices.export`).
    pub(crate) async fn request_export(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: ExportInput,
    ) -> Result<export::Model, Error> {
        principal.require(&EXPORT)?;
        let mut problems = Vec::new();
        if !PROFILES.iter().any(|(name, _)| *name == input.profile) {
            let names: Vec<&str> = PROFILES.iter().map(|(name, _)| *name).collect();
            problems.push(
                FieldError::new("profile", "invalid", "must be one of {values}")
                    .with_param("values", names.join(", ")),
            );
        }
        if !FORMATS.contains(&input.format.as_str()) {
            problems.push(FieldError::new("format", "invalid", "must be `csv` or `xlsx`"));
        }
        if input.to < input.from {
            problems.push(FieldError::new("to", "range", "must not be before `from`"));
        } else if (input.to - input.from).num_days() >= MAX_DAYS {
            problems.push(FieldError::new(
                "to",
                "range",
                "the period must be at most 366 days long",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        // Không có nơi lưu thì báo ngay, không xếp job sẽ hỏng.
        self.store(tenant)?;
        let org = input.org.unwrap_or_else(|| autoxide_kernel::tenancy::root_of(tenant.id()));
        let transaction = tenant.database().begin_write().await?;
        let at = now();
        let row = export::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(if input.org.is_some() { org.as_uuid() } else { Uuid::nil() }),
            profile: Set(input.profile),
            format: Set(input.format),
            period_from: Set(input.from),
            period_to: Set(input.to),
            status: Set("pending".to_owned()),
            file_key: Set(None),
            size: Set(None),
            row_count: Set(None),
            error: Set(None),
            created_by: Set(Some(principal.user_id())),
            created_at: Set(at),
            updated_at: Set(at),
        };
        let saved = export::Entity::insert(row).exec_with_returning(&transaction).await?;
        self.jobs
            .enqueue(
                &transaction,
                tenant,
                &BuildExport { export_id: saved.id },
                EnqueueOptions::default(),
            )
            .await?;
        transaction.commit().await?;
        Ok(saved)
    }

    pub(crate) async fn get_export(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<export::Model, Error> {
        principal.require(&EXPORT)?;
        export::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(export_not_found)
    }

    pub(crate) async fn list_exports(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<export::Model>, Error> {
        principal.require(&EXPORT)?;
        let select = export::Entity::find().scoped(tenant);
        fetch_page(select, export::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// File của bản xuất `id` (đã `ready`).
    pub(crate) async fn export_file(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(export::Model, StoredFile), Error> {
        let row = self.get_export(tenant, principal, id).await?;
        let key = match (row.status.as_str(), row.file_key.as_deref()) {
            ("ready", Some(key)) => key.to_owned(),
            (status, _) => return Err(export_not_ready(status)),
        };
        let file =
            self.store(tenant)?.get(&key).await?.ok_or_else(|| export_not_ready("missing"))?;
        Ok((row, file))
    }

    /// Toàn bộ sổ nhật ký chung của kỳ (đọc từng trang 500 dòng qua `dyn Ledger`).
    pub(crate) async fn journal_of(
        &self,
        tenant: &TenantContext,
        book: Option<OrgId>,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<JournalLine>, Error> {
        let mut all = Vec::new();
        let mut page = PageRequest::first(500, Order::Asc);
        loop {
            let mut query = JournalQuery::new(from, to).page(page.clone());
            if let Some(book) = book {
                query = query.book(book);
            }
            let found = self.ledger.journal(tenant, query).await?;
            all.extend(found.data);
            match found.pagination.next_cursor.and_then(|cursor| cursor.parse::<Uuid>().ok()) {
                Some(cursor) => page = PageRequest::first(500, Order::Asc).after(cursor),
                None => break,
            }
        }
        all.sort_by(|a, b| {
            (a.effective_date, a.posted_at, a.entry_id, a.line_no).cmp(&(
                b.effective_date,
                b.posted_at,
                b.entry_id,
                b.line_no,
            ))
        });
        Ok(all)
    }

    /// Số / ngày của các chứng từ nguồn nhắc tới trong `lines`.
    async fn document_refs(
        &self,
        tenant: &TenantContext,
        lines: &[JournalLine],
    ) -> Result<DocumentRefs, Error> {
        let ids: Vec<Uuid> = lines
            .iter()
            .filter(|line| line.source_module == "invoices")
            .filter_map(|line| line.source_id.as_deref()?.parse().ok())
            .collect();
        let mut refs = HashMap::new();
        for chunk in ids.chunks(500) {
            let rows = document::Entity::find()
                .scoped(tenant)
                .filter(document::Column::Id.is_in(chunk.to_vec()))
                .all(tenant.database().read())
                .await?;
            let series_ids: Vec<Uuid> = rows.iter().filter_map(|row| row.series_id).collect();
            let symbols: HashMap<Uuid, String> = series::Entity::find()
                .scoped(tenant)
                .filter(series::Column::Id.is_in(series_ids))
                .all(tenant.database().read())
                .await?
                .into_iter()
                .map(|row| (row.id, row.symbol))
                .collect();
            for row in rows {
                let symbol =
                    row.series_id.and_then(|id| symbols.get(&id).cloned()).unwrap_or_default();
                let number =
                    row.number.map(|number| format!("{symbol}-{number:07}")).unwrap_or_default();
                let date = row
                    .issue_date
                    .map(|day| day.format("%Y-%m-%d").to_string())
                    .unwrap_or_default();
                refs.insert(row.id.to_string(), (date, number));
            }
        }
        Ok(refs)
    }

    /// Hàng của `generic_documents`: dòng của chứng từ đã phát hành trong kỳ.
    async fn document_rows(
        &self,
        tenant: &TenantContext,
        org: Option<OrgId>,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<Vec<Cell>>, Error> {
        let read = tenant.database().read();
        let mut select = document::Entity::find()
            .scoped(tenant)
            .filter(document::Column::IssueDate.gte(from))
            .filter(document::Column::IssueDate.lte(to))
            .filter(document::Column::Number.is_not_null())
            .order_by_asc(document::Column::IssueDate)
            .order_by_asc(document::Column::Id);
        if let Some(org) = org {
            select = select.filter(document::Column::OrgId.eq(org.as_uuid()));
        }
        let documents = select.all(read).await?;
        let mut rows = Vec::new();
        for chunk in documents.chunks(200) {
            let ids: Vec<Uuid> = chunk.iter().map(|row| row.id).collect();
            let series_ids: Vec<Uuid> = chunk.iter().filter_map(|row| row.series_id).collect();
            let symbols: HashMap<Uuid, String> = series::Entity::find()
                .scoped(tenant)
                .filter(series::Column::Id.is_in(series_ids))
                .all(read)
                .await?
                .into_iter()
                .map(|row| (row.id, row.symbol))
                .collect();
            let mut lines: HashMap<Uuid, Vec<line::Model>> = HashMap::new();
            for item in line::Entity::find()
                .scoped(tenant)
                .filter(line::Column::DocumentId.is_in(ids))
                .order_by_asc(line::Column::LineNo)
                .all(read)
                .await?
            {
                lines.entry(item.document_id).or_default().push(item);
            }
            for row in chunk {
                let buyer: crate::api::Buyer = serde_json::from_str(&row.buyer).unwrap_or_default();
                let currency = autoxide_kernel::money::Currency::parse(&row.currency)?;
                let money = |minor| Cell::Number(amount(Money::new(minor, currency)));
                for item in lines.get(&row.id).map(Vec::as_slice).unwrap_or_default() {
                    rows.push(vec![
                        Cell::text(
                            row.issue_date
                                .map(|day| day.format("%Y-%m-%d").to_string())
                                .unwrap_or_default(),
                        ),
                        Cell::text(row.kind.clone()),
                        Cell::text(
                            row.series_id
                                .and_then(|id| symbols.get(&id).cloned())
                                .unwrap_or_default(),
                        ),
                        Cell::text(row.number.map(|number| number.to_string()).unwrap_or_default()),
                        Cell::text(row.status.clone()),
                        Cell::text(buyer.name.clone().unwrap_or_default()),
                        Cell::text(buyer.tax_code.clone().unwrap_or_default()),
                        Cell::text(buyer.address.clone().unwrap_or_default()),
                        Cell::text(item.item_ref.clone().unwrap_or_default()),
                        Cell::text(item.description.clone()),
                        Cell::text(item.unit.clone().unwrap_or_default()),
                        Cell::Number(item.quantity.clone()),
                        money(item.unit_price_minor),
                        money(item.net_minor),
                        Cell::text(item.rate_percent.clone().unwrap_or_default()),
                        money(item.tax_minor),
                        money(item.total_minor),
                        Cell::text(row.currency.clone()),
                        Cell::text(row.exchange_rate.clone().unwrap_or_default()),
                        Cell::text(row.lookup_code.clone().unwrap_or_default()),
                    ]);
                }
            }
        }
        Ok(rows)
    }

    /// Nội dung file của một bản xuất: (byte, số hàng, đuôi file).
    pub(crate) async fn build_file(
        &self,
        tenant: &TenantContext,
        row: &export::Model,
    ) -> Result<(Vec<u8>, usize), Error> {
        let org = (!row.org_id.is_nil()).then(|| OrgId::from_uuid(row.org_id));
        let (header, rows): (&[&str], Vec<Vec<Cell>>) = match row.profile.as_str() {
            "generic_documents" => (
                &DOCUMENTS_HEADER,
                self.document_rows(tenant, org, row.period_from, row.period_to).await?,
            ),
            profile => {
                let lines = self.journal_of(tenant, org, row.period_from, row.period_to).await?;
                let refs = self.document_refs(tenant, &lines).await?;
                if profile == "vn_double_entry" {
                    (&PAIRS_HEADER, pair_rows(&lines, &refs))
                } else {
                    (&JOURNAL_HEADER, journal_rows(&lines, &refs))
                }
            }
        };
        let count = rows.len();
        let bytes = if row.format == "xlsx" {
            let (header, sheet) = (header.to_vec(), row.profile.clone());
            tokio::task::spawn_blocking(move || to_xlsx(&sheet, &header, &rows))
                .await
                .map_err(Error::internal)??
        } else {
            to_csv(header, &rows)
        };
        Ok((bytes, count))
    }

    /// Chạy một bản xuất (job). Đã xong thì không làm gì.
    pub(crate) async fn run_export(&self, tenant: &TenantContext, id: Uuid) -> Result<(), Error> {
        let Some(row) =
            export::Entity::find_by_id(id).scoped(tenant).one(tenant.database().read()).await?
        else {
            return Ok(());
        };
        if matches!(row.status.as_str(), "ready" | "failed") {
            return Ok(());
        }
        self.set_export(tenant, id, |active| active.status = Set("running".to_owned())).await?;
        let result = async {
            let (bytes, count) = self.build_file(tenant, &row).await?;
            let key = format!("invoices/exports/{id}.{}", row.format);
            let size = bytes.len();
            let content =
                Box::pin(futures_util::stream::once(async move { Ok(Bytes::from(bytes)) }));
            self.store(tenant)?.put(&key, content).await?;
            Ok::<_, Error>((key, size, count))
        }
        .await;
        match result {
            Ok((key, size, count)) => {
                let transaction = tenant.database().begin_write().await?;
                let mut active = export::ActiveModel { id: Set(id), ..Default::default() };
                active.status = Set("ready".to_owned());
                active.file_key = Set(Some(key));
                active.size = Set(Some(i64::try_from(size).unwrap_or(i64::MAX)));
                active.row_count = Set(Some(i64::try_from(count).unwrap_or(i64::MAX)));
                active.updated_at = Set(now());
                export::Entity::update(active).exec(&transaction).await?;
                let event = events::ExportReady(json!({
                    "export_id": id, "profile": row.profile, "format": row.format, "rows": count,
                }));
                self.outbox
                    .record(
                        &transaction,
                        tenant,
                        Actor::system(),
                        Some(format!("invoices.export:{id}")),
                        &event,
                    )
                    .await?;
                transaction.commit().await?;
                Ok(())
            }
            Err(error) => {
                let message: String = error.to_string().chars().take(2000).collect();
                self.set_export(tenant, id, |active| {
                    active.status = Set("failed".to_owned());
                    active.error = Set(Some(message));
                })
                .await?;
                tracing::warn!(export_id = %id, error = %error, "accounting export failed");
                Ok(())
            }
        }
    }

    async fn set_export(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        edit: impl FnOnce(&mut export::ActiveModel),
    ) -> Result<(), Error> {
        let mut active = export::ActiveModel { id: Set(id), ..Default::default() };
        edit(&mut active);
        active.updated_at = Set(now());
        let transaction = tenant.database().begin_write().await?;
        export::Entity::update(active).exec(&transaction).await?;
        transaction.commit().await?;
        Ok(())
    }
}

/// Job `invoices.build_export`.
pub(crate) struct BuildExportJob(pub Arc<InvoicesService>);

#[async_trait::async_trait]
impl JobHandler for BuildExportJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let tenant = job
            .tenant
            .as_ref()
            .ok_or_else(|| Error::internal("an export job must belong to a tenant"))?;
        let payload: BuildExport =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        self.0.run_export(tenant, payload.export_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::money::Currency;
    use chrono::Utc;

    fn line(entry: Uuid, no: i32, side: Side, account: &str, minor: i64) -> JournalLine {
        JournalLine {
            line_id: Uuid::now_v7(),
            entry_id: entry,
            org_id: Uuid::nil(),
            effective_date: NaiveDate::from_ymd_opt(2026, 3, 2).unwrap(),
            posted_at: Utc::now(),
            description: "=cmd|' /C calc'!A0".to_owned(),
            source_module: "demo".to_owned(),
            source_kind: "sale".to_owned(),
            source_id: None,
            reference: "R".to_owned(),
            reverses_entry_id: None,
            line_no: no,
            account_id: Uuid::nil(),
            purpose: "x".to_owned(),
            counterparty_kind: None,
            counterparty_id: None,
            account_code: account.to_owned(),
            side,
            amount: Money::new(minor, Currency::VND),
            base_amount: Money::new(minor, Currency::VND),
            exchange_rate: None,
        }
    }

    #[test]
    fn dinh_khoan_theo_cap_can_va_chan_cong_thuc() {
        let entry = Uuid::now_v7();
        let lines = vec![
            line(entry, 1, Side::Debit, "131", 1_080_000),
            line(entry, 2, Side::Credit, "5111", 1_000_000),
            line(entry, 3, Side::Credit, "33311", 80_000),
        ];
        let rows = pair_rows(&lines, &HashMap::new());
        assert_eq!(rows.len(), 2);
        let total: i64 = rows.iter().map(|row| row[6].as_str().parse::<i64>().unwrap()).sum();
        assert_eq!(total, 1_080_000);
        assert_eq!((rows[0][4].as_str(), rows[0][5].as_str()), ("131", "5111"));
        assert_eq!((rows[1][4].as_str(), rows[1][5].as_str()), ("131", "33311"));
        let csv = String::from_utf8(to_csv(&PAIRS_HEADER, &rows)).unwrap();
        assert!(csv.starts_with('\u{feff}'));
        assert!(csv.contains("\"'=cmd|' /C calc'!A0\""), "{csv}");
        assert!(csv.ends_with("\r\n"));
        let xlsx = to_xlsx("journal", &PAIRS_HEADER, &rows).unwrap();
        assert_eq!(&xlsx[..2], b"PK");
    }

    #[test]
    fn so_tien_theo_don_vi_chinh() {
        assert_eq!(amount(Money::new(1999, Currency::USD)), "19.99");
        assert_eq!(amount(Money::new(-50_000, Currency::VND)), "-50000");
    }
}
