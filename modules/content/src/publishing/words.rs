//! Chỉ mục từ của bản công khai (PT-15, D-20261126-content-word-index): tìm chữ công khai không quét mọi bài bằng
//! `LIKE '%q%'` mà đi qua chỉ mục tìm chữ của database — GIN trên `tsvector` ở Postgres, bảng ảo FTS5 ở SQLite.
//!
//! - Từ do Rust tách ([`words`]), cả khi ghi lẫn khi tìm: hai database chỉ thấy các từ chữ-số cách nhau bằng dấu cách,
//!   nên bộ tách từ của mỗi bên cho cùng một tập từ, và cùng một câu tìm cho cùng kết quả.
//! - Mỗi dòng mang thêm một từ riêng của khách ([`tenant_token`]) và câu tìm luôn đòi từ đó: chỉ mục (dùng chung mọi
//!   khách) chỉ trả dòng của khách gọi, nên một từ phổ biến ở khách khác không làm chậm hay lấp kết quả của khách này.
//! - Câu tìm ([`WordQuery`]): mọi từ phải có; từ từ [`PREFIX_MIN`] ký tự khớp tiền tố.
//! - [`matching_ids`] trả ID (tối đa [`MATCH_CAP`], mới đăng trước) của bài hiện trên website của người gọi, lọc
//!   ngay trong câu chạm chỉ mục; bên gọi lọc tiếp bằng câu thường với `entry_id IN (…)`.
//!
//! Câu chạm chỉ mục là SQL viết tay của lớp `db` (`tenant.db().read().sql(…)`, `:tenant` do kernel bind): toán tử
//! `@@` và `MATCH` không có trong bộ dựng câu. Database dừng câu ở [`DEADLINE`] (`Sql::within`: Postgres
//! `statement_timeout`, SQLite interrupt), kể cả khi người gọi thôi chờ.

use std::time::Duration;

use autoxide_kernel::config::DatabaseBackend;
use autoxide_kernel::db::{Queries as _, is_query_timeout};
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use uuid::Uuid;

/// Số ID tối đa một lần tìm trả về (mới đăng trước): kết quả tìm chữ phủ chừng này bài khớp mới nhất.
pub(crate) const MATCH_CAP: u32 = 1_000;
/// Trần thời gian của câu chạm chỉ mục.
pub(crate) const DEADLINE: Duration = Duration::from_secs(2);
/// Từ ít nhất chừng này ký tự thì khớp tiền tố; ngắn hơn thì khớp nguyên từ (tiền tố một hai ký tự khớp quá nhiều từ).
pub(crate) const PREFIX_MIN: usize = 3;
/// Số từ tối đa của một câu tìm; từ sau đó bị bỏ.
pub(crate) const TERMS_MAX: usize = 8;
/// Từ dài hơn chừng này ký tự không vào chỉ mục và không được tìm (Postgres bỏ từ quá 2 047 byte; hai bên phải giống).
const WORD_MAX: usize = 64;
/// Số từ khác nhau tối đa của một dòng trong chỉ mục: bài rất dài chỉ đưa chừng này từ đầu tiên vào. Đủ cho mọi bài
/// thường (một bài 5 000 chữ có chừng 1 500 từ khác nhau).
pub(crate) const DISTINCT_WORDS_MAX: usize = 4_000;
/// Số byte tối đa của `search_words`: dưới xa trần 1 MB của `tsvector` ở Postgres, nên một bài dài không làm ghi lỗi.
pub(crate) const WORDS_BYTES_MAX: usize = 64 * 1024;

/// Từ riêng của khách `tenant` trong chỉ mục: `zt` + 32 chữ số hex — chữ-số nên đi qua bộ tách từ của hai database
/// nguyên vẹn, và không trùng từ thường (người dùng gõ đúng chuỗi này cũng chỉ thấy bài của chính khách mình).
pub(crate) fn tenant_token(tenant: Uuid) -> String {
    format!("zt{}", tenant.simple())
}

/// Nội dung chỉ mục của một dòng của khách `tenant`: [`tenant_token`], rồi các từ của `text` —
/// [`fold`](autoxide_kernel::search::fold) (bỏ dấu, chữ thường), cắt ở mọi ký tự không phải chữ hay số, bỏ từ dài quá
/// [`WORD_MAX`], mỗi từ một lần (lần đầu xuất hiện), tối đa [`DISTINCT_WORDS_MAX`] từ và [`WORDS_BYTES_MAX`] byte,
/// nối bằng một dấu cách. `Đường đến Đà-Lạt!` → `zt… duong den da lat`.
pub(crate) fn words(tenant: Uuid, text: &str) -> String {
    let mut out = tenant_token(tenant);
    let mut seen = std::collections::HashSet::new();
    for word in split(&autoxide_kernel::search::fold(text)) {
        if seen.len() == DISTINCT_WORDS_MAX {
            break;
        }
        if out.len() + 1 + word.len() > WORDS_BYTES_MAX {
            break;
        }
        if seen.insert(word) {
            out.push(' ');
            out.push_str(word);
        }
    }
    out
}

fn split(folded: &str) -> impl Iterator<Item = &str> {
    folded
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty() && word.chars().count() <= WORD_MAX)
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

    /// Biểu thức `to_tsquery` của Postgres, luôn đòi từ của khách: `zt… & dala:* & lat`. Từ chỉ có chữ-số, nên
    /// không cần thoát ký tự.
    fn tsquery(&self, tenant: Uuid) -> String {
        std::iter::once(tenant_token(tenant))
            .chain(self.terms.iter().map(|term| {
                if term.prefix { format!("{}:*", term.word) } else { term.word.clone() }
            }))
            .collect::<Vec<_>>()
            .join(" & ")
    }

    /// Biểu thức `MATCH` của FTS5, luôn đòi từ của khách: `"zt…" AND "dala"* AND "lat"`. Từ trong ngoặc kép là
    /// chuỗi, không phải toán tử.
    fn fts5(&self, tenant: Uuid) -> String {
        std::iter::once(format!("\"{}\"", tenant_token(tenant)))
            .chain(self.terms.iter().map(|term| {
                if term.prefix {
                    format!("\"{}\"*", term.word)
                } else {
                    format!("\"{}\"", term.word)
                }
            }))
            .collect::<Vec<_>>()
            .join(" AND ")
    }
}

/// Lọc của một lần tìm trên `content_published`, áp ngay trong câu chạm chỉ mục: trần [`MATCH_CAP`] tính trên bài
/// người gọi thấy được, không bị bài của website khác hay bài ẩn lấp chỗ.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Matching<'a> {
    pub locale: &'a str,
    /// `post`, `page`…; `None`: mọi loại.
    pub kind: Option<&'a str>,
    /// Chuỗi nút của website (gần trước, như `Site::lineage_ids`): chỉ bài hiện trên website đó
    /// ([`on_site`](super::reading::on_site)); `None`: mọi nút của khách.
    pub site: Option<&'a [Uuid]>,
    /// Bỏ bài đặt `noindex` (gợi ý tìm kiếm).
    pub indexable_only: bool,
}

/// Chỉ mục từ của bản công khai — mỗi database một cài đặt (Luật số một: rẽ nhánh giấu sau trait). Chỉ dựng phần
/// câu riêng của database; chạy câu và trần thời gian là việc của kernel.
pub(crate) trait WordIndex: Send + Sync {
    /// Điều kiện khớp chỉ mục với tham số `:q`.
    fn match_clause(&self) -> &'static str;

    /// Biểu thức tìm của `query` (cho `:q`) theo cú pháp của database, đòi từ của khách `tenant`.
    fn expression(&self, query: &WordQuery, tenant: Uuid) -> String;
}

/// Cài đặt cho `backend`.
pub(crate) fn index_for(backend: DatabaseBackend) -> &'static dyn WordIndex {
    match backend {
        DatabaseBackend::Postgres => &PostgresWords,
        _ => &SqliteWords,
    }
}

/// Câu SQL lấy ID khớp, tham số có tên: `:tenant` (kernel bind), `:locale`, `:q`, `:cap`, `:kind`, `:org0…`.
pub(crate) fn sql(index: &dyn WordIndex, filter: &Matching<'_>) -> String {
    let mut sql = String::from(
        "SELECT entry_id FROM content_published WHERE tenant_id = :tenant AND locale = :locale \
         AND restricted_at IS NULL",
    );
    if filter.kind.is_some() {
        sql.push_str(" AND kind = :kind");
    }
    if filter.indexable_only {
        sql.push_str(" AND noindex = :noindex");
    }
    if let Some(lineage) = filter.site {
        sql.push_str(" AND ");
        sql.push_str(&on_site_sql(lineage.len()));
    }
    sql.push_str(" AND ");
    sql.push_str(index.match_clause());
    sql.push_str(" ORDER BY published_at DESC, entry_id DESC LIMIT :cap");
    sql
}

/// Điều kiện "hiện trên website" ([`on_site`](super::reading::on_site)) viết tay cho `nodes` nút: bài ở nút
/// `:org{i}` mà không có bài cùng ngôn ngữ, cùng đường dẫn ở nút gần website hơn (`:org0…:org{i-1}`). Không có nút nào:
/// không bài nào.
fn on_site_sql(nodes: usize) -> String {
    if nodes == 0 {
        return "1 = 0".to_owned();
    }
    let parts: Vec<String> = (0..nodes)
        .map(|at| {
            if at == 0 {
                return "org_id = :org0".to_owned();
            }
            let nearer: Vec<String> = (0..at).map(|i| format!(":org{i}")).collect();
            format!(
                "(org_id = :org{at} AND NOT EXISTS (SELECT 1 FROM content_published shadow \
                 WHERE shadow.tenant_id = content_published.tenant_id \
                 AND shadow.locale = content_published.locale AND shadow.path = content_published.path \
                 AND shadow.org_id IN ({})))",
                nearer.join(", ")
            )
        })
        .collect();
    format!("({})", parts.join(" OR "))
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
    let index = index_for(db.backend());
    let mut select = read
        .sql(&sql(index, &filter))
        .bind("locale", filter.locale.to_owned())
        .bind("q", index.expression(query, tenant.id().as_uuid()))
        .bind("cap", i64::from(MATCH_CAP))
        .within(DEADLINE);
    if let Some(kind) = filter.kind {
        select = select.bind("kind", kind.to_owned());
    }
    if filter.indexable_only {
        select = select.bind("noindex", false);
    }
    for (at, org) in filter.site.unwrap_or_default().iter().enumerate() {
        select = select.bind(&format!("org{at}"), *org);
    }
    match select.all::<(Uuid,)>().await {
        Ok(rows) => Ok(rows.into_iter().map(|(id,)| id).collect()),
        Err(error) if is_query_timeout(&error) => {
            tracing::warn!("public word search stopped at its deadline");
            Err(crate::ai::public_limits::timed_out())
        }
        Err(error) => Err(error),
    }
}

/// Postgres: GIN trên `to_tsvector('simple', search_words)` (migration `m0036_word_index`). Biểu thức trong câu phải
/// giống hệt biểu thức của index thì planner mới dùng index.
struct PostgresWords;

impl WordIndex for PostgresWords {
    fn match_clause(&self) -> &'static str {
        "to_tsvector('simple'::regconfig, search_words) @@ to_tsquery('simple'::regconfig, :q)"
    }

    fn expression(&self, query: &WordQuery, tenant: Uuid) -> String {
        query.tsquery(tenant)
    }
}

/// SQLite: bảng ảo FTS5 `content_published_fts`, `rowid` = `content_published.search_rowid` (trigger giữ đồng bộ,
/// migration `m0036_word_index`).
struct SqliteWords;

impl WordIndex for SqliteWords {
    fn match_clause(&self) -> &'static str {
        "search_rowid IN (SELECT rowid FROM content_published_fts WHERE content_published_fts MATCH :q)"
    }

    fn expression(&self, query: &WordQuery, tenant: Uuid) -> String {
        query.fts5(tenant)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TENANT: Uuid = Uuid::from_u128(0xAB);

    #[test]
    fn tach_tu_bo_dau_va_dau_cau() {
        let token = tenant_token(TENANT);
        assert_eq!(
            words(TENANT, "Đường đến Đà-Lạt! (2026)"),
            format!("{token} duong den da lat 2026")
        );
        assert_eq!(words(TENANT, "  ...  "), token);
        assert_eq!(words(TENANT, &"a".repeat(65)), token, "a word over 64 characters is dropped");
        assert_eq!(
            words(TENANT, "東京 tower 東京"),
            format!("{token} 東京 tower"),
            "each word once"
        );
    }

    #[test]
    fn bai_rat_dai_co_tran_so_tu_va_so_byte() {
        // Nhiều từ khác nhau: dừng ở trần số từ.
        let many: Vec<String> = (0..10_000).map(|n| format!("w{n}")).collect();
        let indexed = words(TENANT, &many.join(" "));
        assert_eq!(indexed.split(' ').count(), 1 + DISTINCT_WORDS_MAX);
        // Từ dài: dừng ở trần số byte.
        let long: Vec<String> = (0..10_000).map(|n| format!("{n:0>60}")).collect();
        let indexed = words(TENANT, &long.join(" "));
        assert!(indexed.len() <= WORDS_BYTES_MAX, "{}", indexed.len());
        assert!(indexed.len() > WORDS_BYTES_MAX - 70);
    }

    #[test]
    fn cau_tim_tien_to_tu_ba_ky_tu_va_luon_doi_tu_cua_khach() {
        let token = tenant_token(TENANT);
        let query = WordQuery::parse("Đà Lạt, đà");
        assert_eq!(
            query.terms,
            vec![
                Term { word: "da".into(), prefix: false },
                Term { word: "lat".into(), prefix: true },
            ]
        );
        assert_eq!(query.tsquery(TENANT), format!("{token} & da & lat:*"));
        assert_eq!(query.fts5(TENANT), format!("\"{token}\" AND \"da\" AND \"lat\"*"));
        assert!(WordQuery::parse("%_\"*'").is_empty());
        assert_eq!(WordQuery::parse("a b c d e f g h i j").terms.len(), TERMS_MAX);
    }

    #[test]
    fn dieu_kien_website_viet_tay() {
        assert_eq!(on_site_sql(0), "1 = 0");
        assert_eq!(on_site_sql(1), "(org_id = :org0)");
        let two = on_site_sql(2);
        assert!(two.starts_with("(org_id = :org0 OR (org_id = :org1 AND NOT EXISTS"), "{two}");
        assert!(two.contains("shadow.org_id IN (:org0)"), "{two}");
    }
}
