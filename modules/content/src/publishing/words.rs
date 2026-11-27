//! Chỉ mục từ của bản công khai (PT-15, D-20261126-content-word-index): tìm chữ công khai không quét mọi bài bằng
//! `LIKE '%q%'` mà đi qua chỉ mục đảo `content_published_words` — bảng thường, mỗi từ của một bài đã đăng một dòng,
//! index `(tenant_id, locale, word, entry_id)`. Giống nhau trên hai database, không SQL viết tay.
//!
//! - Từ do Rust tách ([`words`]), cả khi ghi lẫn khi tìm: gấp dấu, chữ thường, cắt ở mọi ký tự không phải chữ / số.
//! - Ghi cùng transaction với bản đăng ([`write`], [`remove`]): xoá dòng cũ của bài rồi chèn lại.
//! - Câu tìm ([`WordQuery`]): tối đa [`TERMS_MAX`] từ, mọi từ phải có; từ từ [`PREFIX_MIN`] ký tự khớp tiền tố
//!   (khoảng `word >= w AND word < w_next`, cột so sánh theo byte), ngắn hơn khớp nguyên từ.
//! - [`matching_ids`] trả ID (tối đa [`MATCH_CAP`], mới đăng trước) của bài hiện trên các nút của website người gọi,
//!   bỏ bài bị ẩn; bên gọi lọc tiếp bằng câu thường với `entry_id IN (…)`. Database dừng câu ở [`DEADLINE`]
//!   (`Find::within` của kernel), kể cả khi người gọi thôi chờ.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use autoxide_kernel::db::{Cond, QUERY_TIMEOUT, Queries as _, Record, TenantKey, is_query_timeout};
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{ScopeExt as _, TenantContext};
use autoxide_kernel::time::Timestamp;
use sea_orm::{ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _, Set};
use uuid::Uuid;

use crate::entity::published_word;

/// Số ID tối đa một lần tìm trả về (mới đăng trước): kết quả tìm chữ phủ chừng này bài khớp mới nhất.
pub(crate) const MATCH_CAP: u64 = 1_000;
/// Trần thời gian của câu chạm chỉ mục.
pub(crate) const DEADLINE: Duration = Duration::from_secs(2);
/// Từ ít nhất chừng này ký tự thì khớp tiền tố; ngắn hơn thì khớp nguyên từ (tiền tố một hai ký tự khớp quá nhiều từ).
pub(crate) const PREFIX_MIN: usize = 3;
/// Số từ tối đa của một câu tìm; từ sau đó bị bỏ.
pub(crate) const TERMS_MAX: usize = 8;
/// Từ dài hơn chừng này ký tự không vào chỉ mục và không được tìm (cột `word` dài 64).
const WORD_MAX: usize = 64;
/// Số từ khác nhau tối đa của một bài trong chỉ mục: bài rất dài chỉ đưa chừng này từ đầu tiên vào (một bài 5 000 chữ
/// có chừng 1 500 từ khác nhau). Chặn số dòng ghi mỗi lần đăng.
pub(crate) const DISTINCT_WORDS_MAX: usize = 4_000;
/// Tổng số byte tối đa các từ của một bài trong chỉ mục.
pub(crate) const WORDS_BYTES_MAX: usize = 64 * 1024;
/// Số dòng tối đa của một câu `INSERT` vào chỉ mục: 4 tham số mỗi dòng, 800 tham số một câu — dưới trần tham số bind
/// của SQLite (999 ở bản cũ, 32 766 từ 3.32) và của Postgres (65 535).
pub(crate) const INSERT_ROWS: usize = 200;

/// Các từ của `text` để đưa vào chỉ mục: [`fold`](autoxide_kernel::search::fold) (bỏ dấu, chữ thường), cắt ở mọi ký
/// tự không phải chữ hay số, bỏ từ dài quá [`WORD_MAX`], mỗi từ một lần (theo lần đầu xuất hiện), tối đa
/// [`DISTINCT_WORDS_MAX`] từ và [`WORDS_BYTES_MAX`] byte. `Đường đến Đà-Lạt!` → `[duong, den, da, lat]`.
pub(crate) fn words(text: &str) -> Vec<String> {
    let folded = autoxide_kernel::search::fold(text);
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut bytes = 0;
    for word in split(&folded) {
        if out.len() == DISTINCT_WORDS_MAX {
            break;
        }
        // Từ đã có không tốn thêm byte: kiểm trần byte sau khi bỏ trùng.
        if seen.contains(word) {
            continue;
        }
        if bytes + word.len() > WORDS_BYTES_MAX {
            break;
        }
        seen.insert(word);
        bytes += word.len();
        out.push(word.to_owned());
    }
    out
}

fn split(folded: &str) -> impl Iterator<Item = &str> {
    folded
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty() && word.chars().count() <= WORD_MAX)
}

/// Chuỗi nhỏ nhất lớn hơn mọi chuỗi bắt đầu bằng `prefix` (thứ tự điểm mã, cũng là thứ tự byte của UTF-8): tăng ký tự
/// cuối lên ký tự kế tiếp (bỏ qua vùng surrogate); ký tự cuối đã là `char::MAX` thì bỏ nó và tăng ký tự trước. `None`:
/// không có cận trên (mọi ký tự đều là `char::MAX`).
pub(crate) fn next_prefix(prefix: &str) -> Option<String> {
    let mut characters: Vec<char> = prefix.chars().collect();
    while let Some(last) = characters.pop() {
        let next = (u32::from(last) + 1..=u32::from(char::MAX)).find_map(char::from_u32);
        if let Some(next) = next {
            characters.push(next);
            return Some(characters.into_iter().collect());
        }
    }
    None
}

/// Một từ của câu tìm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Term {
    /// Chỉ có chữ và số (đã qua [`words`]).
    pub word: String,
    /// Khớp mọi từ bắt đầu bằng `word`.
    pub prefix: bool,
}

/// Câu tìm đã tách từ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WordQuery {
    terms: Vec<Term>,
}

impl WordQuery {
    /// Tách `q` như [`words`], bỏ từ trùng, giữ tối đa [`TERMS_MAX`] từ.
    pub(crate) fn parse(q: &str) -> Self {
        let folded = autoxide_kernel::search::fold(q);
        let mut terms: Vec<Term> = Vec::new();
        for word in split(&folded) {
            if terms.len() == TERMS_MAX {
                break;
            }
            if terms.iter().any(|term| term.word == word) {
                continue;
            }
            terms.push(Term { word: word.to_owned(), prefix: word.chars().count() >= PREFIX_MIN });
        }
        WordQuery { terms }
    }

    /// Câu không còn từ nào (chỉ có dấu câu, hay rỗng): không có kết quả.
    pub(crate) fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }
}

/// Lọc của một lần tìm, áp ngay trong câu chạm chỉ mục: trần [`MATCH_CAP`] tính trên bài người gọi thấy được, không bị
/// bài của website khác hay bài ẩn lấp chỗ.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Matching<'a> {
    pub locale: &'a str,
    /// `post`, `page`…; `None`: mọi loại.
    pub kind: Option<&'a str>,
    /// Chuỗi nút của website (gần trước, như `Site::lineage_ids`): chỉ bài ở các nút đó; `None`: mọi nút của khách.
    /// Bài ở nút xa bị bài cùng đường dẫn ở nút gần hơn che thì bên gọi bỏ ở bước sau
    /// ([`on_site`](super::reading::on_site)).
    pub site: Option<&'a [Uuid]>,
    /// Bỏ bài đặt `noindex` (gợi ý tìm kiếm).
    pub indexable_only: bool,
}

/// Các cột của `content_published` mà câu tìm cần (lớp `db` của kernel; ghi vẫn qua entity SeaORM của module).
#[derive(Record, Clone, Debug)]
#[record(table = "content_published", scope = tenant)]
pub(crate) struct PublishedHit {
    pub tenant_id: TenantKey,
    pub entry_id: Uuid,
    pub org_id: Option<Uuid>,
    pub kind: String,
    pub locale: String,
    pub noindex: bool,
    pub restricted_at: Option<Timestamp>,
    pub published_at: Timestamp,
}

/// Một dòng của chỉ mục đảo `content_published_words` (đọc qua lớp `db`).
#[derive(Record, Clone, Debug)]
#[record(table = "content_published_words", scope = tenant)]
pub(crate) struct IndexedWord {
    pub tenant_id: TenantKey,
    pub entry_id: Uuid,
    pub word: String,
    pub locale: String,
}

/// ID của tối đa [`MATCH_CAP`] bài công khai (không bị ẩn) của khách `tenant` khớp `query`, mới đăng trước, theo
/// `filter`. Câu rỗng thì không chạm database. Mọi câu của một lần tìm chung trần [`DEADLINE`], database dừng câu
/// quá trần: lỗi `503 content.public_search_timeout`.
///
/// Mỗi bước đọc có trần số dòng, để từ rất phổ biến không làm câu đọc hết chỉ mục của khách:
///
/// 1. Mỗi từ của câu: ID bài có từ đó, tối đa [`TERM_ROWS_MAX`] dòng chỉ mục (index `(tenant_id, locale, word,
///    entry_id)`). Từ nào không có dòng nào: không có kết quả. Từ nào vượt trần là "phổ biến".
/// 2. Có ít nhất một từ không phổ biến: giao các tập đã đủ, rồi kiểm từ phổ biến bằng khoá chính `(tenant_id,
///    entry_id, word)` trên chính các bài còn lại; cuối cùng lọc bài theo `filter`, mới đăng trước.
/// 3. Mọi từ đều phổ biến: duyệt bài theo `filter`, mới đăng trước, từng trang [`SCAN_PAGE`] bài, kiểm mọi từ bằng khoá
///    chính, tới khi đủ [`MATCH_CAP`] bài hay đã duyệt [`SCAN_ENTRIES_MAX`] bài — kết quả là các bài khớp trong chừng
///    ấy bài mới nhất (từ phổ biến khớp phần lớn bài).
pub(crate) async fn matching_ids(
    tenant: &TenantContext,
    filter: Matching<'_>,
    query: &WordQuery,
) -> Result<Vec<Uuid>, Error> {
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let search = Search { tenant, filter, deadline: Instant::now() + DEADLINE };
    match search.run(query).await {
        Err(error) if is_query_timeout(&error) => {
            tracing::warn!("public word search stopped at its deadline");
            Err(crate::ai::public_limits::timed_out())
        }
        other => other,
    }
}

/// Số dòng chỉ mục tối đa đọc cho một từ của câu tìm; từ có nhiều dòng hơn là từ phổ biến.
pub(crate) const TERM_ROWS_MAX: u64 = 5_000;
/// Số bài mỗi trang khi duyệt bài mới nhất (mọi từ đều phổ biến).
const SCAN_PAGE: u64 = 1_000;
/// Số bài mới nhất tối đa duyệt khi mọi từ đều phổ biến.
pub(crate) const SCAN_ENTRIES_MAX: u64 = 5_000;
/// Số ID tối đa của một danh sách `IN (…)`: dưới trần tham số bind của cả hai database.
const IDS_PER_QUERY: usize = 500;

/// Một lần tìm: khách, bộ lọc và hạn chung cho mọi câu.
struct Search<'a> {
    tenant: &'a TenantContext,
    filter: Matching<'a>,
    deadline: Instant,
}

impl Search<'_> {
    async fn run(&self, query: &WordQuery) -> Result<Vec<Uuid>, Error> {
        // Bước 1: tập ID của từng từ (None: từ phổ biến, tập chưa đủ).
        let mut complete: Vec<HashSet<Uuid>> = Vec::new();
        let mut common: Vec<&Term> = Vec::new();
        for term in &query.terms {
            let ids = self.term_ids(term).await?;
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            if ids.len() as u64 > TERM_ROWS_MAX {
                common.push(term);
            } else {
                complete.push(ids.into_iter().collect());
            }
        }
        // Bước 2: giao các tập đủ (bắt đầu từ tập nhỏ nhất), kiểm từ phổ biến trên bài còn lại.
        complete.sort_by_key(HashSet::len);
        let mut sets = complete.into_iter();
        if let Some(first) = sets.next() {
            let mut candidates: Vec<Uuid> = sets
                .fold(first, |kept, set| kept.intersection(&set).copied().collect())
                .into_iter()
                .collect();
            for term in &common {
                candidates = self.having(term, &candidates).await?;
            }
            return self.newest(&candidates).await;
        }
        // Bước 3: mọi từ đều phổ biến.
        self.scan(&common).await
    }

    /// Thời gian còn lại của lần tìm; hết thì là lỗi quá hạn như khi database dừng câu.
    fn remaining(&self) -> Result<Duration, Error> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(Error::unavailable(
                QUERY_TIMEOUT,
                "The query ran longer than its time limit.",
            ));
        }
        Ok(left)
    }

    fn word(&self, term: &Term) -> Cond<IndexedWord> {
        let locale = IndexedWord::LOCALE.eq(self.filter.locale.to_owned());
        let word = if term.prefix {
            let from = IndexedWord::WORD.gte(term.word.clone());
            match next_prefix(&term.word) {
                Some(next) => Cond::all().with(from).with(IndexedWord::WORD.lt(next)),
                None => from,
            }
        } else {
            IndexedWord::WORD.eq(term.word.clone())
        };
        Cond::all().with(locale).with(word)
    }

    /// ID bài có từ `term`, tối đa `TERM_ROWS_MAX + 1` dòng chỉ mục (một bài có thể có nhiều từ cùng tiền tố).
    async fn term_ids(&self, term: &Term) -> Result<Vec<Uuid>, Error> {
        let db = self.tenant.db();
        let rows: Vec<(Uuid,)> = db
            .read()
            .find::<IndexedWord>()
            .filter(self.word(term))
            .limit(TERM_ROWS_MAX + 1)
            .within(self.remaining()?)
            .select((IndexedWord::ENTRY_ID,))
            .all()
            .await?;
        if rows.len() as u64 > TERM_ROWS_MAX {
            // Từ phổ biến: báo bằng một tập dài hơn trần, không cần bỏ trùng.
            return Ok(rows.into_iter().map(|(id,)| id).collect());
        }
        let unique: HashSet<Uuid> = rows.into_iter().map(|(id,)| id).collect();
        Ok(unique.into_iter().collect())
    }

    /// Các bài trong `ids` có từ `term` (khoá chính của chỉ mục), giữ thứ tự của `ids`.
    async fn having(&self, term: &Term, ids: &[Uuid]) -> Result<Vec<Uuid>, Error> {
        let mut found = HashSet::new();
        for chunk in ids.chunks(IDS_PER_QUERY) {
            let db = self.tenant.db();
            let rows: Vec<(Uuid,)> = db
                .read()
                .find::<IndexedWord>()
                .filter(IndexedWord::ENTRY_ID.is_in(chunk.iter().copied()))
                .filter(self.word(term))
                .within(self.remaining()?)
                .select((IndexedWord::ENTRY_ID,))
                .all()
                .await?;
            found.extend(rows.into_iter().map(|(id,)| id));
        }
        Ok(ids.iter().copied().filter(|id| found.contains(id)).collect())
    }

    /// Bài công khai theo `filter`: đúng ngôn ngữ, không bị ẩn, đúng loại, ở các nút của website, bỏ `noindex` khi là
    /// gợi ý.
    fn visible(&self) -> Cond<PublishedHit> {
        let mut visible = Cond::all()
            .with(PublishedHit::LOCALE.eq(self.filter.locale.to_owned()))
            .with(PublishedHit::RESTRICTED_AT.is_null());
        if let Some(kind) = self.filter.kind {
            visible = visible.with(PublishedHit::KIND.eq(kind.to_owned()));
        }
        if self.filter.indexable_only {
            visible = visible.with(PublishedHit::NOINDEX.eq(false));
        }
        if let Some(lineage) = self.filter.site {
            visible = visible.with(PublishedHit::ORG_ID.is_in(lineage.iter().copied().map(Some)));
        }
        visible
    }

    /// Trong `ids`, tối đa `MATCH_CAP` bài hiện theo `filter`, mới đăng trước.
    async fn newest(&self, ids: &[Uuid]) -> Result<Vec<Uuid>, Error> {
        let mut found: Vec<(Timestamp, Uuid)> = Vec::new();
        for chunk in ids.chunks(IDS_PER_QUERY) {
            let db = self.tenant.db();
            let rows: Vec<(Uuid, Timestamp)> = db
                .read()
                .find::<PublishedHit>()
                .filter(self.visible())
                .filter(PublishedHit::ENTRY_ID.is_in(chunk.iter().copied()))
                .order_desc(PublishedHit::PUBLISHED_AT)
                .order_desc(PublishedHit::ENTRY_ID)
                .limit(MATCH_CAP)
                .within(self.remaining()?)
                .select((PublishedHit::ENTRY_ID, PublishedHit::PUBLISHED_AT))
                .all()
                .await?;
            found.extend(rows.into_iter().map(|(id, at)| (at, id)));
        }
        found.sort_unstable_by(|a, b| b.cmp(a));
        found.truncate(usize::try_from(MATCH_CAP).unwrap_or(usize::MAX));
        Ok(found.into_iter().map(|(_, id)| id).collect())
    }

    /// Mọi từ đều phổ biến: duyệt bài theo `filter`, mới đăng trước, giữ bài có mọi từ.
    async fn scan(&self, terms: &[&Term]) -> Result<Vec<Uuid>, Error> {
        let cap = usize::try_from(MATCH_CAP).unwrap_or(usize::MAX);
        let mut found = Vec::new();
        let mut after: Option<(Timestamp, Uuid)> = None;
        let mut scanned = 0;
        while scanned < SCAN_ENTRIES_MAX && found.len() < cap {
            let db = self.tenant.db();
            let read = db.read();
            let mut page = read
                .find::<PublishedHit>()
                .filter(self.visible())
                .order_desc(PublishedHit::PUBLISHED_AT)
                .order_desc(PublishedHit::ENTRY_ID)
                .limit(SCAN_PAGE);
            if let Some((at, id)) = after {
                // Keyset: sau bài cuối của trang trước theo (ngày đăng, ID) giảm dần.
                page = page.filter(
                    Cond::any().with(PublishedHit::PUBLISHED_AT.lt(at)).with(
                        Cond::all()
                            .with(PublishedHit::PUBLISHED_AT.eq(at))
                            .with(PublishedHit::ENTRY_ID.lt(id)),
                    ),
                );
            }
            let rows: Vec<(Uuid, Timestamp)> = page
                .within(self.remaining()?)
                .select((PublishedHit::ENTRY_ID, PublishedHit::PUBLISHED_AT))
                .all()
                .await?;
            let Some(&(last_id, last_at)) = rows.last() else { break };
            scanned += rows.len() as u64;
            let mut kept: Vec<Uuid> = rows.iter().map(|(id, _)| *id).collect();
            for term in terms {
                kept = self.having(term, &kept).await?;
            }
            found.extend(kept);
            if (rows.len() as u64) < SCAN_PAGE {
                break;
            }
            after = Some((last_at, last_id));
        }
        found.truncate(cap);
        Ok(found)
    }
}
/// Ghi lại chỉ mục của bài `entry_id` (ngôn ngữ `locale`) theo `search_key` của bản đăng — cùng transaction với bản
/// đăng: xoá dòng cũ của bài rồi chèn các từ mới.
pub(crate) async fn write<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    entry_id: Uuid,
    locale: &str,
    search_key: &str,
) -> Result<(), Error> {
    remove(connection, tenant, entry_id).await?;
    let rows: Vec<published_word::ActiveModel> = words(search_key)
        .into_iter()
        .map(|word| published_word::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            entry_id: Set(entry_id),
            word: Set(word),
            locale: Set(locale.to_owned()),
        })
        .collect();
    // Chia câu theo `INSERT_ROWS`: một bài tới `DISTINCT_WORDS_MAX` từ, mỗi từ 4 tham số bind.
    let mut rows = rows.into_iter().peekable();
    while rows.peek().is_some() {
        let chunk: Vec<_> = rows.by_ref().take(INSERT_ROWS).collect();
        published_word::Entity::insert_many(chunk).exec_without_returning(connection).await?;
    }
    Ok(())
}

/// Bỏ chỉ mục của bài `entry_id` (gỡ, xoá bài).
pub(crate) async fn remove<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    entry_id: Uuid,
) -> Result<(), Error> {
    published_word::Entity::delete_many()
        .filter(published_word::Column::EntryId.eq(entry_id))
        .scoped(tenant)
        .exec(connection)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tach_tu_bo_dau_va_dau_cau() {
        assert_eq!(words("Đường đến Đà-Lạt! (2026)"), ["duong", "den", "da", "lat", "2026"]);
        assert!(words("  ...  ").is_empty());
        assert!(words(&"a".repeat(65)).is_empty(), "a word over 64 characters is dropped");
        assert_eq!(words("東京 tower 東京"), ["東京", "tower"], "each word once");
    }

    #[test]
    fn bai_rat_dai_co_tran_so_tu_va_so_byte() {
        let many: Vec<String> = (0..10_000).map(|n| format!("w{n}")).collect();
        assert_eq!(words(&many.join(" ")).len(), DISTINCT_WORDS_MAX);
        let long: Vec<String> = (0..10_000).map(|n| format!("{n:0>60}")).collect();
        let indexed = words(&long.join(" "));
        let bytes: usize = indexed.iter().map(String::len).sum();
        assert!(bytes <= WORDS_BYTES_MAX, "{bytes}");
        assert!(bytes > WORDS_BYTES_MAX - 70);
    }

    #[test]
    fn cau_tim_tien_to_tu_ba_ky_tu() {
        let query = WordQuery::parse("Đà Lạt, đà");
        assert_eq!(
            query.terms,
            vec![
                Term { word: "da".into(), prefix: false },
                Term { word: "lat".into(), prefix: true },
            ]
        );
        assert!(WordQuery::parse("%_\"*'").is_empty());
        assert_eq!(WordQuery::parse("a b c d e f g h i j").terms.len(), TERMS_MAX);
    }

    #[test]
    fn can_tren_cua_tien_to() {
        assert_eq!(next_prefix("lat").as_deref(), Some("lau"));
        assert_eq!(next_prefix("caz").as_deref(), Some("ca{"));
        // Ký tự ngoài ASCII: điểm mã kế tiếp.
        assert_eq!(next_prefix("東京").as_deref(), Some("東亭"));
        // Ngay trước vùng surrogate: nhảy qua vùng đó.
        assert_eq!(next_prefix("a\u{D7FF}").as_deref(), Some("a\u{E000}"));
        // Ký tự cuối là `char::MAX`: bỏ nó, tăng ký tự trước.
        assert_eq!(next_prefix("ab\u{10FFFF}").as_deref(), Some("ac"));
        assert_eq!(next_prefix("\u{10FFFF}"), None);
        // Mọi chuỗi bắt đầu bằng tiền tố nằm trong [tiền tố, cận trên).
        let upper = next_prefix("lat").unwrap();
        for word in ["lat", "latte", "lat\u{10FFFF}", "lat東"] {
            assert!(word >= "lat" && word < upper.as_str(), "{word}");
        }
        assert!("lau" >= upper.as_str());
    }
}
