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
use std::time::Duration;

use autoxide_kernel::db::{Queries as _, Record, TenantKey, is_query_timeout};
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

/// Các từ của `text` để đưa vào chỉ mục: [`fold`](autoxide_kernel::search::fold) (bỏ dấu, chữ thường), cắt ở mọi ký
/// tự không phải chữ hay số, bỏ từ dài quá [`WORD_MAX`], mỗi từ một lần (theo lần đầu xuất hiện), tối đa
/// [`DISTINCT_WORDS_MAX`] từ và [`WORDS_BYTES_MAX`] byte. `Đường đến Đà-Lạt!` → `[duong, den, da, lat]`.
pub(crate) fn words(text: &str) -> Vec<String> {
    let folded = autoxide_kernel::search::fold(text);
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut bytes = 0;
    for word in split(&folded) {
        if out.len() == DISTINCT_WORDS_MAX || bytes + word.len() > WORDS_BYTES_MAX {
            break;
        }
        if seen.insert(word) {
            bytes += word.len();
            out.push(word.to_owned());
        }
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
/// `filter`. Câu rỗng thì không chạm database. Database dừng câu ở [`DEADLINE`]: lỗi `503
/// content.public_search_timeout`.
pub(crate) async fn matching_ids(
    tenant: &TenantContext,
    filter: Matching<'_>,
    query: &WordQuery,
) -> Result<Vec<Uuid>, Error> {
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let db = tenant.db();
    let read = db.read();
    let mut find = read
        .find::<PublishedHit>()
        .filter(PublishedHit::LOCALE.eq(filter.locale.to_owned()))
        .filter(PublishedHit::RESTRICTED_AT.is_null());
    if let Some(kind) = filter.kind {
        find = find.filter(PublishedHit::KIND.eq(kind.to_owned()));
    }
    if filter.indexable_only {
        find = find.filter(PublishedHit::NOINDEX.eq(false));
    }
    if let Some(lineage) = filter.site {
        find = find.filter(PublishedHit::ORG_ID.is_in(lineage.iter().copied().map(Some)));
    }
    // Mỗi từ của câu một điều kiện `entry_id IN (…)`: mọi từ phải có (AND).
    for term in &query.terms {
        let words = read
            .sub(IndexedWord::ENTRY_ID)
            .filter(IndexedWord::LOCALE.eq(filter.locale.to_owned()));
        let words = if term.prefix {
            let from = words.filter(IndexedWord::WORD.gte(term.word.clone()));
            match next_prefix(&term.word) {
                Some(next) => from.filter(IndexedWord::WORD.lt(next)),
                None => from,
            }
        } else {
            words.filter(IndexedWord::WORD.eq(term.word.clone()))
        };
        find = find.filter(PublishedHit::ENTRY_ID.in_sub(words));
    }
    let found = find
        .order_desc(PublishedHit::PUBLISHED_AT)
        .order_desc(PublishedHit::ENTRY_ID)
        .limit(MATCH_CAP)
        .within(DEADLINE)
        .select((PublishedHit::ENTRY_ID,))
        .all()
        .await;
    match found {
        Ok(rows) => Ok(rows.into_iter().map(|(id,)| id).collect()),
        Err(error) if is_query_timeout(&error) => {
            tracing::warn!("public word search stopped at its deadline");
            Err(crate::ai::public_limits::timed_out())
        }
        Err(error) => Err(error),
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
    // Tối đa `DISTINCT_WORDS_MAX` dòng × 4 tham số: dưới trần tham số của cả hai database.
    if !rows.is_empty() {
        published_word::Entity::insert_many(rows).exec_without_returning(connection).await?;
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
