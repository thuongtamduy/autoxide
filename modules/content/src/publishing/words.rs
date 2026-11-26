//! Chỉ mục từ của bản công khai (PT-15, D-20261126-content-word-index): tìm chữ công khai không quét mọi bài bằng
//! `LIKE '%q%'` mà đi qua chỉ mục tìm chữ của database — GIN trên `tsvector` ở Postgres, bảng ảo FTS5 ở SQLite.
//!
//! - Từ do Rust tách ([`words`]), cả khi ghi lẫn khi tìm: hai database chỉ thấy các từ chữ-số cách nhau bằng dấu cách,
//!   nên bộ tách từ của mỗi bên cho cùng một tập từ, và cùng một câu tìm cho cùng kết quả.
//! - Câu tìm ([`WordQuery`]): mọi từ phải có; từ từ [`PREFIX_MIN`] ký tự khớp tiền tố.
//! - [`WordIndex::matching`] chỉ trả ID (tối đa [`MATCH_CAP`], mới đăng trước) dưới trần thời gian câu lệnh; bên gọi
//!   lọc tiếp bằng câu SeaORM thường với `entry_id IN (…)`.
//!
//! Câu chạm chỉ mục viết bằng sqlx, không qua SeaORM: toán tử tìm chữ (`@@`, `MATCH`) không có trong sea-query, và
//! trần thời gian cần đặt trên CHÍNH kết nối chạy câu — SeaORM không cho lấy kết nối sqlx bên dưới.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use autoxide_kernel::error::Error;
use sea_orm::sqlx;
use sea_orm::{DatabaseBackend, DatabaseConnection};
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

/// Các từ của `text` để đưa vào chỉ mục: [`fold`](autoxide_kernel::search::fold) (bỏ dấu, chữ thường), cắt ở mọi ký
/// tự không phải chữ hay số, bỏ từ dài quá [`WORD_MAX`], nối bằng một dấu cách. `Đường đến Đà-Lạt!` → `duong den da
/// lat`.
pub(crate) fn words(text: &str) -> String {
    split(&autoxide_kernel::search::fold(text)).collect::<Vec<_>>().join(" ")
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

    /// Biểu thức `to_tsquery` của Postgres: `dala:* & lat`. Từ chỉ có chữ-số, nên không cần thoát ký tự.
    fn tsquery(&self) -> String {
        self.terms
            .iter()
            .map(|term| if term.prefix { format!("{}:*", term.word) } else { term.word.clone() })
            .collect::<Vec<_>>()
            .join(" & ")
    }

    /// Biểu thức `MATCH` của FTS5: `"dala"* AND "lat"`. Từ trong ngoặc kép là chuỗi, không phải toán tử.
    fn fts5(&self) -> String {
        self.terms
            .iter()
            .map(|term| {
                if term.prefix {
                    format!("\"{}\"*", term.word)
                } else {
                    format!("\"{}\"", term.word)
                }
            })
            .collect::<Vec<_>>()
            .join(" AND ")
    }
}

/// Lọc của một lần tìm trên `content_published`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Matching<'a> {
    pub tenant: Uuid,
    pub locale: &'a str,
    /// `post`, `page`…; `None`: mọi loại.
    pub kind: Option<&'a str>,
}

/// Giá trị gắn vào câu tìm, theo thứ tự `$1`, `$2`… (Postgres) hay `?1`, `?2`… (SQLite).
#[derive(Debug, Clone)]
pub(crate) enum Bind {
    Uuid(Uuid),
    Text(String),
    Integer(i64),
}

/// Chỉ mục từ của bản công khai — mỗi database một cài đặt (Luật số một: rẽ nhánh giấu sau trait).
#[async_trait::async_trait]
pub(crate) trait WordIndex: Send + Sync {
    /// Câu SQL lấy ID khớp: cột đầu là `entry_id`; `$1`… tenant, locale, biểu thức tìm, trần số dòng, kind.
    fn sql(&self, with_kind: bool) -> String;

    /// Biểu thức tìm của `query` theo cú pháp của database.
    fn expression(&self, query: &WordQuery) -> String;

    /// Chạy `sql` (trả một cột UUID) với `binds`, bị database dừng nếu quá `limit`: lỗi
    /// `503 content.public_search_timeout`. Kết nối trở về pool với cấu hình cũ dù câu bị dừng.
    async fn ids_within(
        &self,
        read: &DatabaseConnection,
        sql: String,
        binds: Vec<Bind>,
        limit: Duration,
    ) -> Result<Vec<Uuid>, Error>;

    /// ID của tối đa [`MATCH_CAP`] bài công khai (không bị ẩn) khớp `query`, mới đăng trước, trong [`DEADLINE`].
    async fn matching(
        &self,
        read: &DatabaseConnection,
        filter: Matching<'_>,
        query: &WordQuery,
    ) -> Result<Vec<Uuid>, Error> {
        let mut binds = vec![
            Bind::Uuid(filter.tenant),
            Bind::Text(filter.locale.to_owned()),
            Bind::Text(self.expression(query)),
            Bind::Integer(i64::from(MATCH_CAP)),
        ];
        if let Some(kind) = filter.kind {
            binds.push(Bind::Text(kind.to_owned()));
        }
        self.ids_within(read, self.sql(filter.kind.is_some()), binds, DEADLINE).await
    }
}

/// Cài đặt cho database của `read`.
pub(crate) fn index_for(read: &DatabaseConnection) -> &'static dyn WordIndex {
    match read.get_database_backend() {
        DatabaseBackend::Postgres => &PostgresWords,
        _ => &SqliteWords,
    }
}

/// ID khớp `query` (hàm tiện cho bên gọi): câu rỗng thì không chạm database.
pub(crate) async fn matching_ids(
    read: &DatabaseConnection,
    filter: Matching<'_>,
    query: &WordQuery,
) -> Result<Vec<Uuid>, Error> {
    if query.is_empty() {
        return Ok(Vec::new());
    }
    index_for(read).matching(read, filter, query).await
}

/// Lỗi của câu tìm: quá hạn thì báo quá giờ, còn lại là lỗi máy chủ.
fn failed(error: sqlx::Error, deadline: Instant) -> Error {
    if Instant::now() >= deadline {
        tracing::warn!("public word search stopped at its deadline");
        return crate::ai::public_limits::timed_out();
    }
    Error::internal(error)
}

/// Gắn `binds` theo thứ tự. Macro thay cho hàm vì kiểu `Query` của sqlx khác nhau theo database.
macro_rules! bind_all {
    ($query:expr, $binds:expr) => {{
        let mut query = $query;
        for bind in $binds {
            query = match bind {
                Bind::Uuid(value) => query.bind(value),
                Bind::Text(value) => query.bind(value),
                Bind::Integer(value) => query.bind(value),
            };
        }
        query
    }};
}

/// Postgres: GIN trên `to_tsvector('simple', search_words)` (migration `m0036_word_index`). Biểu thức trong câu phải
/// giống hệt biểu thức của index thì planner mới dùng index.
struct PostgresWords;

#[async_trait::async_trait]
impl WordIndex for PostgresWords {
    fn sql(&self, with_kind: bool) -> String {
        let kind = if with_kind { " AND kind = $5" } else { "" };
        format!(
            "SELECT entry_id FROM content_published WHERE tenant_id = $1 AND locale = $2 \
             AND restricted_at IS NULL{kind} \
             AND to_tsvector('simple'::regconfig, search_words) @@ to_tsquery('simple'::regconfig, $3) \
             ORDER BY published_at DESC, entry_id DESC LIMIT $4"
        )
    }

    fn expression(&self, query: &WordQuery) -> String {
        query.tsquery()
    }

    async fn ids_within(
        &self,
        read: &DatabaseConnection,
        sql: String,
        binds: Vec<Bind>,
        limit: Duration,
    ) -> Result<Vec<Uuid>, Error> {
        let deadline = Instant::now() + limit;
        let pool = read.get_postgres_connection_pool();
        let mut transaction = pool.begin().await.map_err(|error| failed(error, deadline))?;
        // `SET LOCAL`: chỉ trong transaction này, kết nối về pool với cấu hình cũ. `SET` không nhận tham số `$1`;
        // giá trị là số mili giây do ta tính, không đến từ người gọi. Tối thiểu 1: `0` nghĩa là không giới hạn.
        let millis = limit.as_millis().max(1);
        sqlx::query(sqlx::AssertSqlSafe(format!("SET LOCAL statement_timeout = {millis}")))
            .execute(&mut *transaction)
            .await
            .map_err(|error| failed(error, deadline))?;
        let select = bind_all!(sqlx::query_scalar::<_, Uuid>(sqlx::AssertSqlSafe(sql)), binds);
        let ids =
            select.fetch_all(&mut *transaction).await.map_err(|error| failed(error, deadline))?;
        // Transaction chỉ đọc: kết thúc bằng rollback (không có gì để ghi).
        transaction.rollback().await.map_err(|error| failed(error, deadline))?;
        Ok(ids)
    }
}

/// SQLite: bảng ảo FTS5 `content_published_fts`, `rowid` = `content_published.search_rowid` (trigger giữ đồng bộ,
/// migration `m0036_word_index`).
struct SqliteWords;

/// Tắt progress handler khi bị bỏ: kết nối trở về pool dù future bị huỷ giữa chừng (client ngắt), handler còn gắn trên
/// kết nối nhưng không dừng câu nào nữa. Lần tìm sau trên kết nối đó thay handler mới.
struct Disarm(Arc<AtomicBool>);

impl Drop for Disarm {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

/// Số lệnh máy ảo SQLite giữa hai lần hỏi handler: đủ dày để dừng trong vài mili giây, đủ thưa để không tốn.
const PROGRESS_OPS: i32 = 10_000;

#[async_trait::async_trait]
impl WordIndex for SqliteWords {
    fn sql(&self, with_kind: bool) -> String {
        let kind = if with_kind { " AND kind = ?5" } else { "" };
        format!(
            "SELECT entry_id FROM content_published WHERE tenant_id = ?1 AND locale = ?2 \
             AND restricted_at IS NULL{kind} \
             AND search_rowid IN (SELECT rowid FROM content_published_fts \
             WHERE content_published_fts MATCH ?3) \
             ORDER BY published_at DESC, entry_id DESC LIMIT ?4"
        )
    }

    fn expression(&self, query: &WordQuery) -> String {
        query.fts5()
    }

    async fn ids_within(
        &self,
        read: &DatabaseConnection,
        sql: String,
        binds: Vec<Bind>,
        limit: Duration,
    ) -> Result<Vec<Uuid>, Error> {
        let deadline = Instant::now() + limit;
        let pool = read.get_sqlite_connection_pool();
        let mut connection = pool.acquire().await.map_err(|error| failed(error, deadline))?;
        let armed = Arc::new(AtomicBool::new(true));
        let _disarm = Disarm(armed.clone());
        {
            let mut handle =
                connection.lock_handle().await.map_err(|error| failed(error, deadline))?;
            // Trả `false` thì SQLite dừng câu đang chạy (`SQLITE_INTERRUPT`).
            handle.set_progress_handler(PROGRESS_OPS, move || {
                !armed.load(Ordering::Relaxed) || Instant::now() < deadline
            });
        }
        let select = bind_all!(sqlx::query_scalar::<_, Uuid>(sqlx::AssertSqlSafe(sql)), binds);
        let found = select.fetch_all(&mut *connection).await;
        if let Ok(mut handle) = connection.lock_handle().await {
            handle.remove_progress_handler();
        }
        found.map_err(|error| failed(error, deadline))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tach_tu_bo_dau_va_dau_cau() {
        assert_eq!(words("Đường đến Đà-Lạt! (2026)"), "duong den da lat 2026");
        assert_eq!(words("  ...  "), "");
        assert_eq!(words(&"a".repeat(65)), "", "a word over 64 characters is dropped");
        assert_eq!(words("東京 tower"), "東京 tower");
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
        assert_eq!(query.tsquery(), "da & lat:*");
        assert_eq!(query.fts5(), "\"da\" AND \"lat\"*");
        assert!(WordQuery::parse("%_\"*'").is_empty());
        assert_eq!(WordQuery::parse("a b c d e f g h i j").terms.len(), TERMS_MAX);
    }
}
