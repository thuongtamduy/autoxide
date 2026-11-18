//! Sổ nhật ký chung cho xuất kế toán (bước 36 mục 12): dòng bút toán theo khoảng ngày, kèm bút toán
//! và tài khoản (ba câu: một trang dòng, rồi bút toán và tài khoản của trang đó theo ID). Số hiệu tài
//! khoản: mã khách đặt trên tài khoản (`external_code`), hay ánh xạ mục đích → số hiệu của chế độ kế
//! toán Việt Nam (`data/vn-accounts.json` — cần đối chiếu TT 99/2025/TT-BTC, TT 133/2016/TT-BTC),
//! hay chính mục đích.

use std::collections::HashMap;
use std::sync::OnceLock;

use autoxide_kernel::error::Error;
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{Page, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect, QueryTrait};

use crate::api::{JournalLine, JournalQuery, Side};
use crate::entity::{account, entry, line};
use crate::service::LedgerService;

/// Ánh xạ mục đích → số hiệu tài khoản (đọc một lần từ file dữ liệu nhúng trong binary).
fn vn_accounts() -> &'static HashMap<String, String> {
    static MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    MAP.get_or_init(|| {
        let raw: serde_json::Value =
            serde_json::from_str(include_str!("../data/vn-accounts.json")).unwrap_or_default();
        raw["accounts"]
            .as_object()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|(purpose, code)| {
                        Some((purpose.clone(), code.as_str()?.to_owned()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// Số hiệu tài khoản để xuất: mã riêng, hay ánh xạ, hay mục đích.
pub(crate) fn account_code(row: &account::Model) -> String {
    row.external_code
        .clone()
        .filter(|code| !code.is_empty())
        .or_else(|| vn_accounts().get(&row.purpose).cloned())
        .unwrap_or_else(|| row.purpose.clone())
}

impl LedgerService {
    /// Một trang sổ nhật ký chung (`dyn Ledger::journal`).
    pub(crate) async fn journal(
        &self,
        tenant: &TenantContext,
        query: JournalQuery,
    ) -> Result<Page<JournalLine>, Error> {
        let read = tenant.database().read();
        let mut select = line::Entity::find()
            .scoped(tenant)
            .filter(line::Column::EffectiveDate.gte(query.from))
            .filter(line::Column::EffectiveDate.lte(query.to));
        if let Some(org) = query.book {
            match Self::find_book(read, tenant, org).await? {
                Some(book) => {
                    let accounts = account::Entity::find()
                        .scoped(tenant)
                        .filter(account::Column::BookId.eq(book.id));
                    select = select.filter(line::Column::AccountId.in_subquery(
                        accounts.select_only().column(account::Column::Id).into_query(),
                    ));
                }
                None => {
                    return Ok(Page {
                        data: Vec::new(),
                        pagination: autoxide_kernel::pagination::Pagination::new(
                            query.page.limit(),
                            None,
                        )
                        .with_total(query.page.wants_total().then_some(0)),
                    });
                }
            }
        }
        let found = fetch_page(select, line::Column::Id, |row| row.id, &query.page, read).await?;
        let entry_ids: Vec<uuid::Uuid> = found.data.iter().map(|row| row.entry_id).collect();
        let account_ids: Vec<uuid::Uuid> = found.data.iter().map(|row| row.account_id).collect();
        let entries: HashMap<uuid::Uuid, entry::Model> = if entry_ids.is_empty() {
            HashMap::new()
        } else {
            entry::Entity::find()
                .scoped(tenant)
                .filter(entry::Column::Id.is_in(entry_ids))
                .all(read)
                .await?
                .into_iter()
                .map(|row| (row.id, row))
                .collect()
        };
        let accounts: HashMap<uuid::Uuid, account::Model> = if account_ids.is_empty() {
            HashMap::new()
        } else {
            account::Entity::find()
                .scoped(tenant)
                .filter(account::Column::Id.is_in(account_ids))
                .all(read)
                .await?
                .into_iter()
                .map(|row| (row.id, row))
                .collect()
        };
        let mut data = Vec::with_capacity(found.data.len());
        for row in found.data {
            let (Some(head), Some(owner)) =
                (entries.get(&row.entry_id), accounts.get(&row.account_id))
            else {
                return Err(Error::internal("a ledger line has no entry or account"));
            };
            let blank = |text: &str| (!text.is_empty()).then(|| text.to_owned());
            data.push(JournalLine {
                line_id: row.id,
                entry_id: row.entry_id,
                org_id: head.org_id,
                effective_date: row.effective_date,
                posted_at: head.posted_at,
                description: head.description.clone(),
                source_module: head.source_module.clone(),
                source_kind: head.source_kind.clone(),
                source_id: head.source_id.clone(),
                reference: head.reference.clone(),
                reverses_entry_id: head.reverses_entry_id,
                line_no: row.line_no,
                account_id: row.account_id,
                purpose: owner.purpose.clone(),
                counterparty_kind: blank(&owner.counterparty_kind),
                counterparty_id: blank(&owner.counterparty_id),
                account_code: account_code(owner),
                side: Side::parse(&row.side)
                    .ok_or_else(|| Error::internal("ledger line has a bad side"))?,
                amount: Money::from_columns(row.amount_minor, &row.amount_currency)?,
                base_amount: Money::from_columns(row.base_minor, &row.base_currency)?,
                exchange_rate: row.exchange_rate,
            });
        }
        Ok(Page { data, pagination: found.pagination })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn anh_xa_tai_khoan_doc_duoc_va_du_muc_dich_co_ban() {
        let map = super::vn_accounts();
        for (purpose, _) in crate::api::PURPOSES {
            assert!(map.contains_key(purpose), "{purpose}");
        }
        assert_eq!(map["revenue.sales"], "5111");
    }
}
