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

use autoxide_kernel::db::{
    Cond, QUERY_TIMEOUT, Queries as _, Record, TenantKey, TenantRead, is_query_timeout,
};
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
    /// Chỉ bài gắn chuyên mục / thẻ này.
    pub term: Option<Uuid>,
    /// Chỉ bài của tác giả này.
    pub author: Option<Uuid>,
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

/// Bài gắn chuyên mục / thẻ (đọc qua lớp `db`, cho lọc trước khi cắt trần).
#[derive(Record, Clone, Debug)]
#[record(table = "content_entry_terms", scope = tenant)]
pub(crate) struct EntryTermLink {
    pub tenant_id: TenantKey,
    pub entry_id: Uuid,
    pub term_id: Uuid,
}

/// Bài của tác giả (đọc qua lớp `db`, cho lọc trước khi cắt trần).
#[derive(Record, Clone, Debug)]
#[record(table = "content_entry_authors", scope = tenant)]
pub(crate) struct EntryAuthorLink {
    pub tenant_id: TenantKey,
    pub entry_id: Uuid,
    pub author_id: Uuid,
}

/// ID của tối đa [`MATCH_CAP`] bài công khai (không bị ẩn) của khách `tenant` khớp `query`, mới đăng trước, theo
/// `filter`. Câu rỗng thì không chạm database. Mọi câu của một lần tìm chung trần [`DEADLINE`], database dừng câu
/// quá trần: lỗi `503 content.public_search_timeout`.
///
/// Kết quả luôn là các bài mới đăng nhất khớp mọi từ (không cắt theo số bài đã duyệt); hết hạn trước khi xong thì là
/// lỗi, không trả thiếu. Mỗi câu đọc có trần số dòng, để từ rất phổ biến không làm một câu đọc hết chỉ mục của khách:
///
/// 1. Mỗi từ của câu, trần nhỏ trước: ID bài có từ đó, tối đa `DIRECT_MAX` dòng chỉ mục (index `(tenant_id, locale,
///    word, entry_id)`). Từ nào không có dòng nào, hay giao rỗng: không có kết quả. Có từ đọc đủ dưới trần nhỏ ("từ
///    hiếm") thì sang bước 2 với các từ còn lại. Không có thì đọc lại các từ với trần [`TERM_ROWS_MAX`], giao dần trong
///    bộ nhớ (tập giao đã nhỏ thì các từ sau sang bước 2); từ vượt trần là "phổ biến".
/// 2. Ít ứng viên (tối đa `DIRECT_MAX`): kiểm các từ còn lại bằng khoá chính `(tenant_id, entry_id, word)` trên chính
///    các bài đó, rồi lấy bài theo `filter`, mới đăng trước.
/// 3. Nhiều ứng viên, hay mọi từ đều phổ biến: duyệt bài theo `filter`, mới đăng trước, từng trang [`SCAN_PAGE`] bài
///    (index `(tenant_id, locale, published_at, entry_id)`), giữ bài thuộc tập ứng viên (trong bộ nhớ) và có các từ
///    phổ biến (khoá chính), tới khi đủ [`MATCH_CAP`] bài hay hết bài.
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

/// Số dòng chỉ mục tối đa đọc cho một từ của câu tìm (tập ID trong bộ nhớ: tối đa chừng 800 KB một từ); từ có nhiều
/// dòng hơn là từ "phổ biến", kiểm bằng khoá chính. Test dùng trần nhỏ để mọi đường đi chạy với ít dữ liệu.
#[cfg(not(test))]
pub(crate) const TERM_ROWS_MAX: u64 = 50_000;
#[cfg(test)]
pub(crate) const TERM_ROWS_MAX: u64 = 2_000;
/// Tập bài ứng viên tối đa chừng này thì lấy bài mới nhất bằng danh sách `IN (…)`; nhiều hơn thì duyệt bài mới nhất và
/// kiểm thành viên trong bộ nhớ.
#[cfg(not(test))]
pub(crate) const DIRECT_MAX: usize = 2_000;
#[cfg(test)]
pub(crate) const DIRECT_MAX: usize = 200;
/// Số bài mỗi trang khi duyệt bài mới nhất (mọi từ đều phổ biến).
const SCAN_PAGE: u64 = 1_000;
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
        // Bước 1a: mọi từ với trần nhỏ (`DIRECT_MAX`) — rẻ dù câu có nhiều từ phổ biến. Có từ hiếm (đọc đủ dưới trần
        // nhỏ) thì giao các tập hiếm, kiểm các từ còn lại bằng khoá chính: không đọc tập lớn nào.
        let mut rare: Option<HashSet<Uuid>> = None;
        let mut others: Vec<&Term> = Vec::new();
        for term in &query.terms {
            let ids = self.term_ids(term, DIRECT_MAX as u64).await?;
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            if ids.len() > DIRECT_MAX {
                others.push(term);
                continue;
            }
            let set: HashSet<Uuid> = ids.into_iter().collect();
            let kept = match rare {
                None => set,
                Some(mut kept) => {
                    kept.retain(|id| set.contains(id));
                    kept
                }
            };
            if kept.is_empty() {
                return Ok(Vec::new());
            }
            rare = Some(kept);
        }
        if let Some(kept) = rare {
            // Bước 2: ít ứng viên — kiểm các từ còn lại bằng khoá chính, rồi lấy bài mới nhất trong danh sách.
            let mut ids: Vec<Uuid> = kept.into_iter().collect();
            for term in &others {
                ids = self.having(term, &ids).await?;
            }
            return self.newest(&ids).await;
        }
        // Bước 1b: mọi từ đều hơn `DIRECT_MAX` bài — đọc tập tới `TERM_ROWS_MAX`, giao dần trong bộ nhớ; từ vượt trần
        // ("phổ biến") kiểm sau bằng khoá chính. Tập giao đã nhỏ thì các từ sau kiểm bằng khoá chính, không đọc nữa.
        let mut candidates: Option<HashSet<Uuid>> = None;
        let mut common: Vec<&Term> = Vec::new();
        for term in others {
            if candidates.as_ref().is_some_and(|kept| kept.len() <= DIRECT_MAX) {
                common.push(term);
                continue;
            }
            let ids = self.term_ids(term, TERM_ROWS_MAX).await?;
            if ids.len() as u64 > TERM_ROWS_MAX {
                common.push(term);
                continue;
            }
            let set: HashSet<Uuid> = ids.into_iter().collect();
            let kept = match candidates {
                None => set,
                Some(mut kept) => {
                    kept.retain(|id| set.contains(id));
                    kept
                }
            };
            if kept.is_empty() {
                return Ok(Vec::new());
            }
            candidates = Some(kept);
        }
        match candidates {
            // Bước 2: ít ứng viên — kiểm từ phổ biến bằng khoá chính, rồi lấy bài mới nhất trong danh sách.
            Some(kept) if kept.len() <= DIRECT_MAX => {
                let mut ids: Vec<Uuid> = kept.into_iter().collect();
                for term in &common {
                    ids = self.having(term, &ids).await?;
                }
                self.newest(&ids).await
            }
            // Bước 3: nhiều ứng viên, hay mọi từ đều phổ biến — duyệt bài mới nhất.
            kept => self.scan(kept.as_ref(), &common).await,
        }
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

    /// Điều kiện của từ `term` trên cột `word` (khớp nguyên từ, hay khoảng tiền tố).
    fn word(term: &Term) -> Cond<IndexedWord> {
        if term.prefix {
            let from = IndexedWord::WORD.gte(term.word.clone());
            match next_prefix(&term.word) {
                Some(next) => Cond::all().with(from).with(IndexedWord::WORD.lt(next)),
                None => from,
            }
        } else {
            IndexedWord::WORD.eq(term.word.clone())
        }
    }

    /// ID bài có từ `term`, tối đa `cap + 1` dòng chỉ mục (một bài có thể có nhiều từ cùng tiền tố). Dài hơn `cap`:
    /// tập chưa đủ.
    async fn term_ids(&self, term: &Term, cap: u64) -> Result<Vec<Uuid>, Error> {
        let db = self.tenant.db();
        let rows: Vec<(Uuid,)> = db
            .read()
            .find::<IndexedWord>()
            .filter(IndexedWord::LOCALE.eq(self.filter.locale.to_owned()))
            .filter(Self::word(term))
            .limit(cap + 1)
            .within(self.remaining()?)
            .select((IndexedWord::ENTRY_ID,))
            .all()
            .await?;
        if rows.len() as u64 > cap {
            // Tập chưa đủ: báo bằng một danh sách dài hơn trần, không cần bỏ trùng.
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
                // Không lọc ngôn ngữ ở đây (mỗi bài chỉ một ngôn ngữ, các bài trong `ids` đã đúng ngôn ngữ): không có
                // `locale` thì index `(tenant_id, locale, word, entry_id)` không dùng được cho câu này, chỉ còn khoá
                // chính `(tenant_id, entry_id, word)` — mỗi ID một lần dò. Có `locale`, Postgres có lúc chọn index kia
                // và quét cả khoảng từ (đo được: tới 640 ms một câu với 50 000 bài).
                .filter(IndexedWord::ENTRY_ID.is_in(chunk.iter().copied()))
                .filter(Self::word(term))
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
    fn visible(&self, read: &TenantRead<'_>) -> Cond<PublishedHit> {
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
        // Chuyên mục và tác giả lọc ở đây, trước khi cắt `MATCH_CAP`: không thì bài khớp mới hơn ở chuyên mục khác lấp
        // hết trần (index `(tenant_id, term_id, entry_id)` / `(tenant_id, author_id, entry_id)`).
        if let Some(term) = self.filter.term {
            let linked = read.sub(EntryTermLink::ENTRY_ID).filter(EntryTermLink::TERM_ID.eq(term));
            visible = visible.with(PublishedHit::ENTRY_ID.in_sub(linked));
        }
        if let Some(author) = self.filter.author {
            let linked =
                read.sub(EntryAuthorLink::ENTRY_ID).filter(EntryAuthorLink::AUTHOR_ID.eq(author));
            visible = visible.with(PublishedHit::ENTRY_ID.in_sub(linked));
        }
        visible
    }

    /// Trong `ids`, tối đa `MATCH_CAP` bài hiện theo `filter`, mới đăng trước.
    async fn newest(&self, ids: &[Uuid]) -> Result<Vec<Uuid>, Error> {
        let mut found: Vec<(Timestamp, Uuid)> = Vec::new();
        for chunk in ids.chunks(IDS_PER_QUERY) {
            let db = self.tenant.db();
            let read = db.read();
            let rows: Vec<(Uuid, Timestamp)> = read
                .find::<PublishedHit>()
                .filter(self.visible(&read))
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

    /// Duyệt bài theo `filter`, mới đăng trước, giữ bài thuộc `candidates` (nếu có) và có mọi từ `terms`, tới khi đủ
    /// `MATCH_CAP` hay hết bài.
    async fn scan(
        &self,
        candidates: Option<&HashSet<Uuid>>,
        terms: &[&Term],
    ) -> Result<Vec<Uuid>, Error> {
        let cap = usize::try_from(MATCH_CAP).unwrap_or(usize::MAX);
        let mut found = Vec::new();
        let mut after: Option<(Timestamp, Uuid)> = None;
        // Không trần số bài duyệt: dừng khi đủ hay hết bài; hạn chung của lần tìm chặn chi phí (hết hạn là lỗi 503).
        while found.len() < cap {
            let db = self.tenant.db();
            let read = db.read();
            let mut page = read
                .find::<PublishedHit>()
                .filter(self.visible(&read))
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
            // Thành viên trong bộ nhớ trước (không tốn câu), rồi từ phổ biến bằng khoá chính trên số còn lại.
            let mut kept: Vec<Uuid> = rows
                .iter()
                .map(|(id, _)| *id)
                .filter(|id| candidates.is_none_or(|candidates| candidates.contains(id)))
                .collect();
            for term in terms {
                if kept.is_empty() {
                    break;
                }
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
