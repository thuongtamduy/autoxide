//! Chuỗi băm của sổ cái (mượn D-183 của audit): sửa, xoá hay chèn một bút toán đã niêm phong đều
//! bị `autoxide doctor` phát hiện.
//!
//! Job `ledger.seal_entries` (10 giây) lấy các bút toán chưa niêm phong của từng sổ, khoá đầu chuỗi
//! (`ledger_chain_heads`) của sổ, gán số thứ tự và ghi vào `ledger_entry_seals`:
//!
//! - `entry_hash`: SHA-256 của bút toán và các dòng của nó;
//! - `chain_hash`: SHA-256 của `chain_hash` trước, số thứ tự, ID và `entry_hash`.
//!
//! Đường request không bị nối đuôi (niêm phong chạy sau, trong job). **Không băm `tenant_id`**: tách
//! / gộp khách (bước 38) đổi `tenant_id` mà không làm gãy chuỗi. Cách băm là định dạng lưu trữ —
//! đổi nó làm mọi chuỗi thành "bị sửa"; cần đổi thì thêm `ledger-v2`, không sửa `ledger-v1`.

use autoxide_kernel::time::{Precision, Timestamp};
use jiff::SignedDuration;
use std::collections::HashMap;

use autoxide_kernel::database::Database;
use autoxide_kernel::doctor::{Finding, Probe};
use autoxide_kernel::error::Error;
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::DatabaseRegistry;
use sea_orm::sea_query::{OnConflict, Query};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use uuid::Uuid;

use crate::entity::{entry, head, line, seal};
use crate::service::{LedgerService, now};

/// Số bút toán niêm phong mỗi lần cho một sổ.
const BATCH: u64 = 500;
/// Số sổ xét mỗi lần chạy job.
const BOOKS_PER_RUN: u64 = 200;
/// Bút toán chưa niêm phong lâu hơn ngần này thì doctor cảnh báo.
const UNSEALED_WARNING: SignedDuration = SignedDuration::from_mins(10);
/// Thời gian tối đa cho bước kiểm chuỗi của doctor: đọc lại cả sổ cái.
pub(crate) const VERIFY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// Nối một phần vào chuỗi đem băm: `|<độ dài>:<giá trị>` hoặc `|-` khi không có.
fn push(text: &mut String, part: Option<&str>) {
    match part {
        Some(value) => {
            text.push('|');
            text.push_str(&value.len().to_string());
            text.push(':');
            text.push_str(value);
        }
        None => text.push_str("|-"),
    }
}

fn instant(at: Timestamp) -> String {
    // Độ chính xác micro giây: đúng độ chính xác cả hai database lưu.
    at.to_rfc3339_opts(Precision::Micros, true)
}

/// Mã băm của bút toán `row` và các dòng `lines` (đã sắp theo `line_no`).
pub(crate) fn entry_hash(row: &entry::Model, lines: &[line::Model]) -> String {
    let mut text = String::from("ledger-entry-v1");
    let reverses = row.reverses_entry_id.map(|id| id.to_string());
    let actor = row.actor_user_id.map(|id| id.to_string());
    for part in [
        Some(row.id.to_string().as_str()),
        Some(row.book_id.to_string().as_str()),
        Some(row.org_id.to_string().as_str()),
        Some(row.period_id.to_string().as_str()),
        Some(row.effective_date.format("%Y-%m-%d").to_string().as_str()),
        Some(instant(row.posted_at).as_str()),
        Some(row.description.as_str()),
        Some(row.source_module.as_str()),
        Some(row.source_kind.as_str()),
        row.source_id.as_deref(),
        Some(row.reference.as_str()),
        reverses.as_deref(),
        actor.as_deref(),
        Some(row.actor_kind.as_str()),
    ] {
        push(&mut text, part);
    }
    push(&mut text, Some(lines.len().to_string().as_str()));
    for item in lines {
        for part in [
            Some(item.line_no.to_string().as_str()),
            Some(item.account_id.to_string().as_str()),
            Some(item.side.as_str()),
            Some(item.amount_minor.to_string().as_str()),
            Some(item.amount_currency.as_str()),
            Some(item.base_minor.to_string().as_str()),
            Some(item.base_currency.as_str()),
            item.exchange_rate.as_deref(),
            Some(item.effective_date.format("%Y-%m-%d").to_string().as_str()),
        ] {
            push(&mut text, part);
        }
    }
    autoxide_kernel::secret::sha256(&text)
}

/// Mã băm nối bút toán thứ `sequence` sau mã băm `previous` (`""` cho bút toán đầu tiên).
pub(crate) fn chain_hash(
    previous: &str,
    sequence: i64,
    entry_id: Uuid,
    entry_hash: &str,
) -> String {
    let mut text = String::from("ledger-v1");
    for part in [previous, &sequence.to_string(), &entry_id.to_string(), entry_hash] {
        push(&mut text, Some(part));
    }
    autoxide_kernel::secret::sha256(&text)
}

/// Dòng của các bút toán `ids`, theo bút toán, sắp theo `line_no`.
async fn lines_of<C: ConnectionTrait>(
    connection: &C,
    tenant: Uuid,
    ids: Vec<Uuid>,
) -> Result<HashMap<Uuid, Vec<line::Model>>, Error> {
    let rows = line::Entity::find()
        .filter(line::Column::TenantId.eq(tenant))
        .filter(line::Column::EntryId.is_in(ids))
        .order_by_asc(line::Column::EntryId)
        .order_by_asc(line::Column::LineNo)
        .all(connection)
        .await?;
    let mut grouped: HashMap<Uuid, Vec<line::Model>> = HashMap::new();
    for row in rows {
        grouped.entry(row.entry_id).or_default().push(row);
    }
    Ok(grouped)
}

/// Câu con: ID các bút toán đã niêm phong.
fn sealed_ids() -> sea_orm::sea_query::SelectStatement {
    Query::select().column(seal::Column::EntryId).from(seal::Entity).to_owned()
}

impl LedgerService {
    /// Niêm phong bút toán chưa niêm phong trên `database` (job `ledger.seal_entries`). Trả số bút
    /// toán vừa niêm phong.
    pub(crate) async fn seal_entries(database: &Database) -> Result<u64, Error> {
        let books: Vec<(Uuid, Uuid)> = entry::Entity::find()
            .select_only()
            .column(entry::Column::TenantId)
            .column(entry::Column::BookId)
            .filter(entry::Column::Id.not_in_subquery(sealed_ids()))
            .distinct()
            .limit(BOOKS_PER_RUN)
            .into_tuple()
            .all(database.read())
            .await?;
        let mut sealed = 0;
        for (tenant, book) in books {
            sealed += database
                .write_with_retry(move || async move {
                    let transaction = database.begin_write().await?;
                    let count = seal_book(&transaction, tenant, book).await?;
                    transaction.commit().await?;
                    Ok(count)
                })
                .await?;
        }
        Ok(sealed)
    }
}

/// Niêm phong tối đa [`BATCH`] bút toán của sổ `book` trong `transaction`.
async fn seal_book<C: ConnectionTrait>(
    transaction: &C,
    tenant: Uuid,
    book: Uuid,
) -> Result<u64, Error> {
    head::Entity::insert(head::ActiveModel {
        tenant_id: Set(tenant),
        book_id: Set(book),
        last_sequence: Set(0),
        last_hash: Set(String::new()),
    })
    .on_conflict(
        OnConflict::columns([head::Column::TenantId, head::Column::BookId]).do_nothing().to_owned(),
    )
    .exec_without_returning(transaction)
    .await?;
    // Khoá đầu chuỗi: hai lần chạy cùng lúc nối đuôi nhau; lần sau đọc lại và thấy đã niêm phong.
    let current = rows::lock_one(transaction, head::Entity::find_by_id((tenant, book)))
        .await?
        .ok_or_else(|| Error::internal("ledger chain head vanished"))?;
    let pending = entry::Entity::find()
        .filter(entry::Column::TenantId.eq(tenant))
        .filter(entry::Column::BookId.eq(book))
        .filter(entry::Column::Id.not_in_subquery(sealed_ids()))
        .order_by_asc(entry::Column::Id)
        .limit(BATCH)
        .all(transaction)
        .await?;
    if pending.is_empty() {
        return Ok(0);
    }
    let mut lines =
        lines_of(transaction, tenant, pending.iter().map(|row| row.id).collect()).await?;
    let (mut sequence, mut previous) = (current.last_sequence, current.last_hash.clone());
    let at = now();
    let mut seals = Vec::with_capacity(pending.len());
    for row in &pending {
        sequence += 1;
        let entry_lines = lines.remove(&row.id).unwrap_or_default();
        let entry_hash = entry_hash(row, &entry_lines);
        let chained = chain_hash(&previous, sequence, row.id, &entry_hash);
        seals.push(seal::ActiveModel {
            tenant_id: Set(tenant),
            book_id: Set(book),
            sequence: Set(sequence),
            entry_id: Set(row.id),
            entry_hash: Set(entry_hash),
            chain_hash: Set(chained.clone()),
            sealed_at: Set(at),
        });
        previous = chained;
    }
    let count = seals.len() as u64;
    seal::Entity::insert_many(seals).exec_without_returning(transaction).await?;
    head::Entity::update(head::ActiveModel {
        tenant_id: Set(tenant),
        book_id: Set(book),
        last_sequence: Set(sequence),
        last_hash: Set(previous),
    })
    .exec(transaction)
    .await?;
    Ok(count)
}

/// Kết quả kiểm chuỗi của một sổ.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Chuỗi còn nguyên; số bút toán đã kiểm.
    Intact(u64),
    /// Chỗ hỏng đầu tiên (số thứ tự mong đợi, ID bút toán nếu biết, lý do).
    Broken { sequence: i64, entry: Option<Uuid>, reason: &'static str },
}

/// Kiểm chuỗi của sổ `book` (khách `tenant`) từ đầu tới đầu chuỗi.
pub(crate) async fn verify<C: ConnectionTrait>(
    connection: &C,
    tenant: Uuid,
    book: Uuid,
) -> Result<Verdict, Error> {
    let mut previous = String::new();
    let mut expected: i64 = 1;
    loop {
        let seals = seal::Entity::find()
            .filter(seal::Column::TenantId.eq(tenant))
            .filter(seal::Column::BookId.eq(book))
            .filter(seal::Column::Sequence.gte(expected))
            .order_by_asc(seal::Column::Sequence)
            .limit(BATCH)
            .all(connection)
            .await?;
        let count = seals.len();
        let ids: Vec<Uuid> = seals.iter().map(|row| row.entry_id).collect();
        let entries: HashMap<Uuid, entry::Model> = entry::Entity::find()
            .filter(entry::Column::TenantId.eq(tenant))
            .filter(entry::Column::Id.is_in(ids.clone()))
            .all(connection)
            .await?
            .into_iter()
            .map(|row| (row.id, row))
            .collect();
        let mut lines = lines_of(connection, tenant, ids).await?;
        for stored in &seals {
            let broken = |reason| Verdict::Broken {
                sequence: expected,
                entry: Some(stored.entry_id),
                reason,
            };
            if stored.sequence != expected {
                return Ok(Verdict::Broken {
                    sequence: expected,
                    entry: None,
                    reason: "a seal is missing (deleted)",
                });
            }
            let Some(row) = entries.get(&stored.entry_id) else {
                return Ok(broken("the entry was deleted"));
            };
            if row.book_id != book {
                return Ok(broken("the entry was moved to another book"));
            }
            let entry_lines = lines.remove(&row.id).unwrap_or_default();
            if entry_hash(row, &entry_lines) != stored.entry_hash {
                return Ok(broken("the entry or its lines were changed"));
            }
            let chained = chain_hash(&previous, expected, row.id, &stored.entry_hash);
            if chained != stored.chain_hash {
                return Ok(broken("the seal was changed"));
            }
            previous = chained;
            expected += 1;
        }
        if (count as u64) < BATCH {
            break;
        }
    }
    let current = head::Entity::find_by_id((tenant, book)).one(connection).await?;
    let (last, hash) = current.map_or((0, String::new()), |row| (row.last_sequence, row.last_hash));
    if last != expected - 1 || hash != previous {
        return Ok(Verdict::Broken {
            sequence: expected,
            entry: None,
            reason: "seals at the end of the chain are missing (deleted)",
        });
    }
    Ok(Verdict::Intact(u64::try_from(expected - 1).unwrap_or_default()))
}

/// Kiểm mọi chuỗi trên `database`: (số bút toán đã kiểm, chỗ hỏng đầu tiên, số bút toán chưa
/// niêm phong quá [`UNSEALED_WARNING`]).
pub(crate) async fn check_database(
    database: &Database,
) -> Result<(u64, Option<String>, u64), Error> {
    // Một bức ảnh nhất quán: niêm phong mới giữa chừng không làm lệch đầu chuỗi với các dấu.
    let snapshot = database.begin_snapshot().await?;
    let heads: Vec<(Uuid, Uuid)> = head::Entity::find()
        .select_only()
        .column(head::Column::TenantId)
        .column(head::Column::BookId)
        .into_tuple()
        .all(&snapshot)
        .await?;
    let mut total = 0;
    for (tenant, book) in heads {
        match verify(&snapshot, tenant, book).await? {
            Verdict::Intact(count) => total += count,
            Verdict::Broken { sequence, entry, reason } => {
                let at = entry.map_or_else(String::new, |id| format!(" (entry {id})"));
                return Ok((
                    total,
                    Some(format!("tenant {tenant}, book {book}: seal #{sequence}{at} — {reason}")),
                    0,
                ));
            }
        }
    }
    let stale = entry::Entity::find()
        .filter(entry::Column::Id.not_in_subquery(sealed_ids()))
        .filter(entry::Column::PostedAt.lt(Timestamp::now() - UNSEALED_WARNING))
        .count(&snapshot)
        .await?;
    Ok((total, None, stale))
}

/// Bước kiểm `ledger.chain` của `autoxide doctor`.
pub(crate) struct ChainProbe {
    pub databases: DatabaseRegistry,
}

#[async_trait::async_trait]
impl Probe for ChainProbe {
    async fn probe(&self) -> Finding {
        let databases = match self.databases.databases().await {
            Ok(databases) => databases,
            Err(error) => return Finding::failed(format!("cannot list the databases: {error}")),
        };
        let (mut checked, mut stale) = (0, 0);
        for (_, name, database) in databases {
            match check_database(&database).await {
                Ok((count, None, unsealed)) => {
                    checked += count;
                    stale += unsealed;
                }
                Ok((_, Some(broken), _)) => {
                    return Finding::failed(format!("database `{name}`: {broken}")).with_hint(
                        "A ledger entry was changed or removed outside Autoxide. Restore the \
                         database from a backup taken before that, then post corrections as new \
                         entries.",
                    );
                }
                Err(error) => {
                    return Finding::failed(format!("cannot check database `{name}`: {error}"));
                }
            }
        }
        if stale > 0 {
            return Finding::warning(format!(
                "{stale} ledger entries are still unsealed after 10 minutes."
            ))
            .with_hint("Check that a worker runs (`autoxide worker`) and the job `ledger.seal_entries` succeeds.");
        }
        Finding::ok(format!("{checked} sealed ledger entries are intact."))
    }
}

/// Bước kiểm `ledger.balances` của `autoxide doctor`: số dư cộng dồn khớp tổng các dòng.
pub(crate) struct BalancesProbe {
    pub databases: DatabaseRegistry,
}

#[async_trait::async_trait]
impl Probe for BalancesProbe {
    async fn probe(&self) -> Finding {
        let databases = match self.databases.databases().await {
            Ok(databases) => databases,
            Err(error) => return Finding::failed(format!("cannot list the databases: {error}")),
        };
        for (_, name, database) in databases {
            match LedgerService::drifted(&database).await {
                Ok(drifted) if drifted.is_empty() => {}
                Ok(drifted) => {
                    let first: Vec<String> = drifted
                        .iter()
                        .take(5)
                        .map(|(tenant, account, stored, expected)| {
                            format!("tenant {tenant}, account {account}: {stored} stored, {expected} from the lines")
                        })
                        .collect();
                    return Finding::failed(format!(
                        "database `{name}`: {} ledger account balances differ from their lines: {}.",
                        drifted.len(),
                        first.join("; ")
                    ))
                    .with_hint("A balance was changed outside Autoxide. Restore from a backup, or contact support.");
                }
                Err(error) => {
                    return Finding::failed(format!("cannot check database `{name}`: {error}"));
                }
            }
        }
        Finding::ok("Every ledger account balance matches the sum of its lines.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chuoi_bam_phu_thuoc_moi_phan() {
        let id = Uuid::now_v7();
        let a = chain_hash("", 1, id, "h");
        assert_eq!(a, chain_hash("", 1, id, "h"));
        assert_ne!(a, chain_hash("x", 1, id, "h"));
        assert_ne!(a, chain_hash("", 2, id, "h"));
        assert_ne!(a, chain_hash("", 1, id, "g"));
        let mut text = String::new();
        push(&mut text, Some("ab"));
        push(&mut text, None);
        assert_eq!(text, "|2:ab|-");
    }
}
