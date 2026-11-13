//! Tìm theo ý nghĩa (D-109), như nhau trên SQLite và Postgres.
//!
//! - **Lập index**: chữ của mục (tiêu đề, tóm tắt, chữ thuần) cắt thành đoạn ~300 từ chồng nhau,
//!   mỗi đoạn một vector lưu trong `content_embeddings` (cột nhị phân). Đoạn không đổi (cùng
//!   SHA-256, cùng mô hình) thì không tính lại. Job `content.index_embeddings` chạy sau mỗi thay
//!   đổi của mục (xếp trong cùng transaction); job định kỳ `content.catch_up_embeddings` lập cho
//!   mục còn thiếu — khi vừa bật embedding hay đổi mô hình.
//! - **Tìm**: mỗi tiến trình giữ vector của từng (`scope`, `locale`) trong bộ nhớ, đọc một dòng
//!   `version` mỗi lần tìm để biết có phải nạp lại không (đúng khi nhiều máy chủ). Duyệt hết
//!   (brute force) bằng tích vô hướng; điểm của mục = đoạn khớp nhất. Gộp với tìm theo chữ không
//!   dấu sẵn có (D-104) bằng Reciprocal Rank Fusion (`k = 60`).
//! - Embedding tắt (hay lỗi, hay hết ngân sách tìm công khai): chỉ tìm theo chữ — vẫn có kết quả.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use autoxide_kernel::access::Principal;
use autoxide_kernel::ai::{EmbedKind, vector};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use sea_orm::sea_query::{Expr, ExprTrait as _};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::entity::{embedding, embedding_state, entry, published};
use crate::entries::{Kind, is_unique_violation};
use crate::permissions::CONTENT_READ;
use crate::service::{ContentService, invalid, now};

pub(crate) const INDEX: &str = "content.index_embeddings";
pub(crate) const CATCH_UP: &str = "content.catch_up_embeddings";
/// Số từ mỗi đoạn, và số từ chồng lên đoạn trước (để ý ở ranh giới không bị cắt mất).
const CHUNK_WORDS: usize = 300;
const OVERLAP_WORDS: usize = 50;
/// Mỗi cách tìm đưa tối đa chừng này ứng viên vào bước gộp.
const CANDIDATES: usize = 50;
const RRF_K: f32 = 60.0;
/// Độ dài tối đa của câu tìm (ký tự); dùng chung cho tìm theo chữ công khai (`/public/posts?q=`).
pub(crate) const QUERY_MAX: usize = 200;
const LIMIT_MAX: u32 = 50;
const LIMIT_DEFAULT: u32 = 10;
const SNIPPET_MAX: usize = 300;
/// Câu tìm đã tính embedding, giữ trong bộ nhớ (bỏ hết khi đầy).
const QUERY_CACHE_MAX: usize = 256;
/// Job lập index còn thiếu: mỗi lần chạy xếp tối đa chừng này mục.
const CATCH_UP_BATCH: usize = 500;

/// Bản nào của mục.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Scope {
    Draft,
    Published,
}

impl Scope {
    fn as_str(self) -> &'static str {
        match self {
            Scope::Draft => "draft",
            Scope::Published => "published",
        }
    }
}

/// Job: lập lại index của mục `entry_id`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct IndexJob {
    pub entry_id: Uuid,
}

impl Job for IndexJob {
    const KIND: &'static str = INDEX;
}

/// Vector của một (`scope`, `locale`) đã nạp vào bộ nhớ.
struct Loaded {
    version: i64,
    model: String,
    chunks: Vec<(Uuid, i32, Vec<f32>)>,
}

/// Bộ nhớ của tìm theo ý nghĩa trong một tiến trình.
#[derive(Default)]
pub(crate) struct SearchCache {
    // `Mutex` của thư viện chuẩn: giữ khoá rất ngắn, không có `await` bên trong.
    indexes: Mutex<HashMap<(Scope, String), Arc<Loaded>>>,
    queries: Mutex<HashMap<String, Arc<Vec<f32>>>>,
    /// Ngân sách tìm công khai theo ý nghĩa: (đầu phút, số lần đã dùng trong phút đó).
    public_budget: Mutex<Option<(Instant, u32)>>,
}

impl std::fmt::Debug for SearchCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchCache").finish_non_exhaustive()
    }
}

/// Điểm gộp của một mục: (điểm RRF, (độ giống, đoạn khớp nhất) nếu khớp ý nghĩa, có khớp chữ).
type Fused = (f32, Option<(f32, i32)>, bool);

/// Một câu tìm.
#[derive(Debug, Clone, Default)]
pub(crate) struct SearchQuery {
    pub q: String,
    pub locale: Option<String>,
    pub kind: Option<Kind>,
    pub limit: Option<u32>,
}

/// Một kết quả.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Hit {
    pub entry_id: Uuid,
    pub kind: String,
    pub locale: String,
    pub title: String,
    /// Bản nháp: trạng thái; bản công khai: `None`.
    pub status: Option<String>,
    /// Bản công khai: đường dẫn; bản nháp: `None`.
    pub path: Option<String>,
    /// Điểm gộp (RRF); chỉ để so trong cùng một lần tìm.
    pub score: f32,
    /// Độ giống theo ý nghĩa (cosine, −1 … 1) của đoạn khớp nhất; `None` khi chỉ khớp chữ.
    pub similarity: Option<f32>,
    pub snippet: String,
    /// `meaning`, `text`, hay cả hai.
    pub matched_by: Vec<&'static str>,
}

impl ContentService {
    /// Xếp job lập lại index của mục `entry_id`, trong `connection` (transaction của thay đổi).
    /// Embedding tắt thì không làm gì.
    pub(crate) async fn enqueue_index<C: ConnectionTrait>(
        &self,
        connection: &C,
        entry_id: Uuid,
    ) -> Result<(), Error> {
        if self.embedder.enabled() {
            self.jobs
                .enqueue(connection, &IndexJob { entry_id }, EnqueueOptions::default())
                .await?;
        }
        Ok(())
    }

    /// Lập lại index (bản nháp và bản công khai) của mục `entry_id`.
    pub(crate) async fn index_entry(&self, entry_id: Uuid) -> Result<(), Error> {
        if !self.embedder.enabled() {
            return Ok(());
        }
        let read = self.database.read();
        let draft = entry::Entity::find_by_id(entry_id).one(read).await?.map(|row| {
            (row.locale.clone(), chunks(&row.title, &row.excerpt, &row.plain_text), row.updated_at)
        });
        let public = published::Entity::find_by_id(entry_id).one(read).await?.map(|row| {
            (row.locale.clone(), chunks(&row.title, &row.excerpt, &row.plain_text), row.updated_at)
        });
        self.index_scope(entry_id, Scope::Draft, draft).await?;
        self.index_scope(entry_id, Scope::Published, public).await
    }

    async fn index_scope(
        &self,
        entry_id: Uuid,
        scope: Scope,
        source: Option<(String, Vec<String>, chrono::DateTime<chrono::Utc>)>,
    ) -> Result<(), Error> {
        let model = self.embedder.model_id().to_owned();
        let read_stamp = source.as_ref().map(|(_, _, stamp)| *stamp);
        let dimensions = i32::try_from(self.embedder.dimensions()).unwrap_or(0);
        let existing = embedding::Entity::find()
            .filter(embedding::Column::EntryId.eq(entry_id))
            .filter(embedding::Column::Scope.eq(scope.as_str()))
            .order_by_asc(embedding::Column::ChunkIndex)
            .all(self.database.read())
            .await?;
        let Some((locale, texts, _)) = source else {
            if let Some(first) = existing.first() {
                ensure_state(self.database.write(), scope, &first.locale).await?;
                let transaction = self.database.begin_write().await?;
                if stamp(&transaction, entry_id, scope).await?.is_some() {
                    // Mục vừa có lại (vd đăng lại) sau lúc đọc: để job mới hơn lo.
                    return Ok(());
                }
                delete_rows(&transaction, entry_id, scope).await?;
                bump(&transaction, scope, &first.locale).await?;
                transaction.commit().await?;
            }
            return Ok(());
        };
        let hashes: Vec<String> = texts.iter().map(|text| hash(text)).collect();
        let current = |row: &embedding::Model| row.model == model && row.dimensions == dimensions;
        let unchanged = existing.len() == hashes.len()
            && existing
                .iter()
                .zip(&hashes)
                .all(|(row, hash)| current(row) && &row.content_hash == hash);
        if unchanged {
            return Ok(());
        }
        let mut known: HashMap<&str, Vec<u8>> = HashMap::new();
        for row in existing.iter().filter(|row| current(row)) {
            known.insert(row.content_hash.as_str(), row.vector.clone());
        }
        let missing: Vec<String> = texts
            .iter()
            .zip(&hashes)
            .filter(|(_, hash)| !known.contains_key(hash.as_str()))
            .map(|(text, _)| text.clone())
            .collect();
        // Gọi embedding NGOÀI transaction (luật: không gọi mạng trong transaction ghi).
        let mut fresh = self.embedder.embed("content.index", &missing, EmbedKind::Document).await?;
        for vector in &mut fresh {
            vector::normalize(vector);
        }
        let mut fresh = fresh.into_iter();
        let now = now();
        ensure_state(self.database.write(), scope, &locale).await?;
        let transaction = self.database.begin_write().await?;
        // Mục đổi trong lúc tính embedding (job khác sẽ chạy với chữ mới): không ghi đè bằng
        // vector của chữ cũ.
        if stamp(&transaction, entry_id, scope).await? != read_stamp {
            return Ok(());
        }
        delete_rows(&transaction, entry_id, scope).await?;
        for (index, hash) in hashes.iter().enumerate() {
            let bytes = match known.get(hash.as_str()) {
                Some(bytes) => bytes.clone(),
                None => fresh.next().map(|vector| vector::encode(&vector)).unwrap_or_default(),
            };
            embedding::ActiveModel {
                id: Set(Uuid::now_v7()),
                entry_id: Set(entry_id),
                scope: Set(scope.as_str().to_owned()),
                locale: Set(locale.clone()),
                chunk_index: Set(i32::try_from(index).unwrap_or(i32::MAX)),
                content_hash: Set(hash.clone()),
                model: Set(model.clone()),
                dimensions: Set(dimensions),
                vector: Set(bytes),
                updated_at: Set(now),
            }
            .insert(&transaction)
            .await?;
        }
        bump(&transaction, scope, &locale).await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Xếp job lập index cho các mục chưa có index với mô hình hiện tại (tối đa
    /// [`CATCH_UP_BATCH`] mỗi lần). Trả số mục đã xếp.
    pub(crate) async fn catch_up_embeddings(&self) -> Result<usize, Error> {
        if !self.embedder.enabled() {
            return Ok(0);
        }
        let read = self.database.read();
        let model = self.embedder.model_id().to_owned();
        let dimensions = i32::try_from(self.embedder.dimensions()).unwrap_or(0);
        let indexed: BTreeSet<Uuid> = embedding::Entity::find()
            .select_only()
            .column(embedding::Column::EntryId)
            .filter(embedding::Column::Scope.eq(Scope::Draft.as_str()))
            .filter(embedding::Column::Model.eq(model))
            .filter(embedding::Column::Dimensions.eq(dimensions))
            .distinct()
            .into_tuple::<Uuid>()
            .all(read)
            .await?
            .into_iter()
            .collect();
        let all: Vec<Uuid> = entry::Entity::find()
            .select_only()
            .column(entry::Column::Id)
            .into_tuple()
            .all(read)
            .await?;
        let missing: Vec<Uuid> =
            all.into_iter().filter(|id| !indexed.contains(id)).take(CATCH_UP_BATCH).collect();
        if missing.is_empty() {
            return Ok(0);
        }
        let transaction = self.database.begin_write().await?;
        // Mỗi mục tối đa một job lập bù mỗi ngày: nhà cung cấp hỏng hay hết hạn mức thì không
        // xếp chồng job mỗi giờ.
        let day = chrono::Utc::now().format("%Y-%m-%d");
        for entry_id in &missing {
            let options =
                EnqueueOptions::default().unique_key(format!("content.index:{entry_id}:{day}"));
            self.jobs.enqueue(&transaction, &IndexJob { entry_id: *entry_id }, options).await?;
        }
        transaction.commit().await?;
        tracing::info!(entries = missing.len(), "content entries queued for the search index");
        Ok(missing.len())
    }

    /// Tìm bản nháp (người biên tập; cần `content.read`) hay bản công khai (`public`, không cần
    /// đăng nhập, tắt mặc định).
    pub(crate) async fn search(
        &self,
        principal: Option<&Principal>,
        query: &SearchQuery,
        public: bool,
    ) -> Result<Vec<Hit>, Error> {
        if public {
            if !self.config.public_semantic_search {
                return Err(Error::not_found(
                    "content.semantic_search_disabled",
                    "Search by meaning is not offered to the public on this server \
                     (AUTOXIDE_CONTENT_PUBLIC_SEMANTIC_SEARCH). Use `GET /api/v1/content/public/posts?q=`.",
                ));
            }
        } else {
            let principal = principal.ok_or_else(|| {
                Error::unauthenticated("content.sign_in_required", "Sign in to search drafts.")
            })?;
            principal.require(&CONTENT_READ)?;
        }
        let q = query.q.trim();
        let mut problems = Vec::new();
        if q.is_empty() || q.chars().count() > QUERY_MAX {
            problems.push(FieldError::new(
                "q",
                "length",
                format!("must be 1 to {QUERY_MAX} characters"),
            ));
        }
        let limit = query.limit.unwrap_or(LIMIT_DEFAULT);
        if !(1..=LIMIT_MAX).contains(&limit) {
            problems.push(FieldError::new("limit", "range", format!("must be 1 to {LIMIT_MAX}")));
        }
        let locale = self.check_locale(query.locale.as_deref(), "locale", &mut problems);
        if !problems.is_empty() {
            return Err(invalid(problems));
        }
        let scope = if public { Scope::Published } else { Scope::Draft };
        let by_text = self.text_candidates(scope, q, &locale, query.kind).await?;
        let by_meaning = if self.embedder.enabled() && (!public || self.spend_public_budget()) {
            match self.meaning_candidates(scope, q, &locale).await {
                Ok(found) => found,
                Err(error) => {
                    // Lỗi embedding (hết hạn mức, nhà cung cấp bận): vẫn trả kết quả theo chữ.
                    tracing::warn!(
                        code = error.code(),
                        "search by meaning unavailable; matching words only"
                    );
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

        // Reciprocal Rank Fusion: mỗi danh sách góp 1 / (k + hạng), hạng tính từ 1.
        let mut fused: BTreeMap<Uuid, Fused> = BTreeMap::new();
        for (rank, (id, similarity, chunk)) in by_meaning.iter().enumerate() {
            let slot = fused.entry(*id).or_insert((0.0, None, false));
            slot.0 += 1.0 / (RRF_K + rank as f32 + 1.0);
            slot.1 = Some((*similarity, *chunk));
        }
        for (rank, id) in by_text.iter().enumerate() {
            let slot = fused.entry(*id).or_insert((0.0, None, false));
            slot.0 += 1.0 / (RRF_K + rank as f32 + 1.0);
            slot.2 = true;
        }
        let ids: Vec<Uuid> = fused.keys().copied().collect();
        let mut hits = self.describe_hits(scope, &ids, query.kind, &fused).await?;
        hits.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.entry_id.cmp(&b.entry_id)));
        hits.truncate(limit as usize);
        Ok(hits)
    }

    /// Mục khớp chữ (không dấu, D-104), mới nhất trước.
    async fn text_candidates(
        &self,
        scope: Scope,
        q: &str,
        locale: &str,
        kind: Option<Kind>,
    ) -> Result<Vec<Uuid>, Error> {
        let read = self.database.read();
        let limit = CANDIDATES as u64;
        Ok(match scope {
            Scope::Draft => {
                let mut select = entry::Entity::find()
                    .select_only()
                    .column(entry::Column::Id)
                    .filter(entry::Column::Locale.eq(locale))
                    .filter(autoxide_kernel::search::contains(entry::Column::SearchKey, q));
                if let Some(kind) = kind {
                    select = select.filter(entry::Column::Kind.eq(kind.as_str()));
                }
                select
                    .order_by_desc(entry::Column::UpdatedAt)
                    .limit(limit)
                    .into_tuple()
                    .all(read)
                    .await?
            }
            Scope::Published => {
                let mut select = published::Entity::find()
                    .select_only()
                    .column(published::Column::EntryId)
                    .filter(published::Column::Locale.eq(locale))
                    .filter(autoxide_kernel::search::contains(published::Column::SearchKey, q));
                if let Some(kind) = kind {
                    select = select.filter(published::Column::Kind.eq(kind.as_str()));
                }
                select
                    .order_by_desc(published::Column::PublishedAt)
                    .limit(limit)
                    .into_tuple()
                    .all(read)
                    .await?
            }
        })
    }

    /// Mục gần câu tìm nhất theo ý nghĩa: (ID, độ giống, đoạn khớp nhất), giống nhất trước.
    async fn meaning_candidates(
        &self,
        scope: Scope,
        q: &str,
        locale: &str,
    ) -> Result<Vec<(Uuid, f32, i32)>, Error> {
        let query = self.query_vector(q).await?;
        let loaded = self.loaded(scope, locale).await?;
        let mut best: HashMap<Uuid, (f32, i32)> = HashMap::new();
        for (entry_id, chunk, vector) in &loaded.chunks {
            let similarity = vector::dot(&query, vector);
            let slot = best.entry(*entry_id).or_insert((f32::MIN, *chunk));
            if similarity > slot.0 {
                *slot = (similarity, *chunk);
            }
        }
        let mut ranked: Vec<(Uuid, f32, i32)> =
            best.into_iter().map(|(id, (similarity, chunk))| (id, similarity, chunk)).collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        ranked.truncate(CANDIDATES);
        Ok(ranked)
    }

    /// Vector (đã chuẩn hoá) của câu tìm, giữ lại để lần sau khỏi gọi nhà cung cấp.
    async fn query_vector(&self, q: &str) -> Result<Arc<Vec<f32>>, Error> {
        let key = format!("{}\n{}", self.embedder.model_id(), autoxide_kernel::search::fold(q));
        if let Some(found) =
            self.search_cache.queries.lock().ok().and_then(|cache| cache.get(&key).cloned())
        {
            return Ok(found);
        }
        let mut vectors =
            self.embedder.embed("content.search", &[q.to_owned()], EmbedKind::Query).await?;
        let mut vector = vectors.pop().unwrap_or_default();
        vector::normalize(&mut vector);
        let vector = Arc::new(vector);
        if let Ok(mut cache) = self.search_cache.queries.lock() {
            if cache.len() >= QUERY_CACHE_MAX {
                cache.clear();
            }
            cache.insert(key, vector.clone());
        }
        Ok(vector)
    }

    /// Vector của (`scope`, `locale`) trong bộ nhớ; nạp lại khi `version` trong database đổi.
    async fn loaded(&self, scope: Scope, locale: &str) -> Result<Arc<Loaded>, Error> {
        let read = self.database.read();
        let version = embedding_state::Entity::find()
            .filter(embedding_state::Column::Scope.eq(scope.as_str()))
            .filter(embedding_state::Column::Locale.eq(locale))
            .one(read)
            .await?
            .map_or(0, |row| row.version);
        let model = self.embedder.model_id().to_owned();
        let key = (scope, locale.to_owned());
        let cached =
            self.search_cache.indexes.lock().ok().and_then(|cache| cache.get(&key).cloned());
        if let Some(cached) =
            cached.filter(|cached| cached.version == version && cached.model == model)
        {
            return Ok(cached);
        }
        let dimensions = i32::try_from(self.embedder.dimensions()).unwrap_or(0);
        let rows = embedding::Entity::find()
            .filter(embedding::Column::Scope.eq(scope.as_str()))
            .filter(embedding::Column::Locale.eq(locale))
            .filter(embedding::Column::Model.eq(model.as_str()))
            .filter(embedding::Column::Dimensions.eq(dimensions))
            .all(read)
            .await?;
        let chunks = rows
            .into_iter()
            .filter_map(|row| {
                vector::decode(&row.vector).map(|vector| (row.entry_id, row.chunk_index, vector))
            })
            .collect();
        let loaded = Arc::new(Loaded { version, model, chunks });
        if let Ok(mut cache) = self.search_cache.indexes.lock() {
            cache.insert(key, loaded.clone());
        }
        Ok(loaded)
    }

    /// Một lần tìm công khai theo ý nghĩa trong ngân sách mỗi phút không (đếm trong tiến trình).
    fn spend_public_budget(&self) -> bool {
        let Ok(mut budget) = self.search_cache.public_budget.lock() else { return false };
        let now = Instant::now();
        let (start, used) = match *budget {
            Some((start, used)) if now.duration_since(start) < Duration::from_secs(60) => {
                (start, used)
            }
            _ => (now, 0),
        };
        if used >= self.config.public_search_per_minute {
            *budget = Some((start, used));
            return false;
        }
        *budget = Some((start, used + 1));
        true
    }

    /// Chi tiết của các mục tìm được (bỏ mục không còn, hay khác `kind`).
    async fn describe_hits(
        &self,
        scope: Scope,
        ids: &[Uuid],
        kind: Option<Kind>,
        fused: &BTreeMap<Uuid, Fused>,
    ) -> Result<Vec<Hit>, Error> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let read = self.database.read();
        // (ID, loại, ngôn ngữ, tiêu đề, tóm tắt, chữ thuần, trạng thái, đường dẫn)
        type Row = (Uuid, String, String, String, String, String, Option<String>, Option<String>);
        let rows: Vec<Row> = match scope {
            Scope::Draft => entry::Entity::find()
                .filter(entry::Column::Id.is_in(ids.to_vec()))
                .all(read)
                .await?
                .into_iter()
                .map(|row| {
                    (
                        row.id,
                        row.kind,
                        row.locale,
                        row.title,
                        row.excerpt,
                        row.plain_text,
                        Some(row.status),
                        None,
                    )
                })
                .collect(),
            Scope::Published => published::Entity::find()
                .filter(published::Column::EntryId.is_in(ids.to_vec()))
                .all(read)
                .await?
                .into_iter()
                .map(|row| {
                    (
                        row.entry_id,
                        row.kind,
                        row.locale,
                        row.title,
                        row.excerpt,
                        row.plain_text,
                        None,
                        Some(row.path),
                    )
                })
                .collect(),
        };
        let mut hits = Vec::new();
        for (id, row_kind, locale, title, excerpt, plain_text, status, path) in rows {
            if kind.is_some_and(|kind| kind.as_str() != row_kind) {
                continue;
            }
            let Some((score, meaning, text)) = fused.get(&id) else { continue };
            let snippet = match meaning {
                Some((_, chunk)) => {
                    let pieces = chunks("", "", &plain_text);
                    pieces.get(usize::try_from(*chunk).unwrap_or(0)).cloned().unwrap_or_default()
                }
                None if !excerpt.is_empty() => excerpt.clone(),
                None => plain_text.clone(),
            };
            let mut matched_by = Vec::new();
            if meaning.is_some() {
                matched_by.push("meaning");
            }
            if *text {
                matched_by.push("text");
            }
            hits.push(Hit {
                entry_id: id,
                kind: row_kind,
                locale,
                title,
                status,
                path,
                score: *score,
                similarity: meaning.map(|(similarity, _)| similarity),
                snippet: snippet.trim().chars().take(SNIPPET_MAX).collect(),
                matched_by,
            });
        }
        Ok(hits)
    }
}

/// Chữ của mục cắt thành đoạn: tiêu đề và tóm tắt đầu mỗi đoạn (cho đoạn có ngữ cảnh), rồi
/// ~[`CHUNK_WORDS`] từ, chồng [`OVERLAP_WORDS`] từ lên đoạn trước. Ít nhất một đoạn.
pub(crate) fn chunks(title: &str, excerpt: &str, plain_text: &str) -> Vec<String> {
    let header: String = [title, excerpt]
        .iter()
        .filter(|part| !part.trim().is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    let words: Vec<&str> = plain_text.split_whitespace().collect();
    let join = |body: &[&str]| {
        let body = body.join(" ");
        match (header.is_empty(), body.is_empty()) {
            (true, _) => body,
            (false, true) => header.clone(),
            (false, false) => format!("{header}\n{body}"),
        }
    };
    if words.len() <= CHUNK_WORDS {
        return vec![join(&words)];
    }
    let step = CHUNK_WORDS - OVERLAP_WORDS;
    let mut out = Vec::new();
    let mut start = 0;
    while start < words.len() {
        let end = (start + CHUNK_WORDS).min(words.len());
        out.push(join(&words[start..end]));
        if end == words.len() {
            break;
        }
        start += step;
    }
    out
}

fn hash(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}

async fn delete_rows<C: ConnectionTrait>(
    connection: &C,
    entry_id: Uuid,
    scope: Scope,
) -> Result<(), Error> {
    embedding::Entity::delete_many()
        .filter(embedding::Column::EntryId.eq(entry_id))
        .filter(embedding::Column::Scope.eq(scope.as_str()))
        .exec(connection)
        .await?;
    Ok(())
}

/// Bảo đảm có dòng `version` của (`scope`, `locale`) — gọi NGOÀI transaction: trên Postgres,
/// một lỗi trùng khoá trong transaction làm hỏng cả transaction đó.
async fn ensure_state<C: ConnectionTrait>(
    connection: &C,
    scope: Scope,
    locale: &str,
) -> Result<(), Error> {
    let exists = embedding_state::Entity::find()
        .filter(embedding_state::Column::Scope.eq(scope.as_str()))
        .filter(embedding_state::Column::Locale.eq(locale))
        .one(connection)
        .await?
        .is_some();
    if exists {
        return Ok(());
    }
    let row = embedding_state::ActiveModel {
        id: Set(Uuid::now_v7()),
        scope: Set(scope.as_str().to_owned()),
        locale: Set(locale.to_owned()),
        version: Set(0),
    };
    match row.insert(connection).await {
        Ok(_) => Ok(()),
        // Một tiến trình khác vừa thêm: đã có dòng.
        Err(error) if is_unique_violation(&error) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Tăng `version` của (`scope`, `locale`) để mọi tiến trình nạp lại vector. Một câu `UPDATE`
/// (`version = version + 1`); dòng đã có nhờ [`ensure_state`].
async fn bump<C: ConnectionTrait>(connection: &C, scope: Scope, locale: &str) -> Result<(), Error> {
    embedding_state::Entity::update_many()
        .col_expr(
            embedding_state::Column::Version,
            Expr::col(embedding_state::Column::Version).add(1),
        )
        .filter(embedding_state::Column::Scope.eq(scope.as_str()))
        .filter(embedding_state::Column::Locale.eq(locale))
        .exec(connection)
        .await?;
    Ok(())
}

/// Dấu thời gian hiện tại của bản (`scope`) của mục, đọc trong `connection`; `None`: không có.
async fn stamp<C: ConnectionTrait>(
    connection: &C,
    entry_id: Uuid,
    scope: Scope,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, Error> {
    Ok(match scope {
        Scope::Draft => {
            entry::Entity::find_by_id(entry_id).one(connection).await?.map(|row| row.updated_at)
        }
        Scope::Published => {
            published::Entity::find_by_id(entry_id).one(connection).await?.map(|row| row.updated_at)
        }
    })
}

/// Bộ chạy job lập index của một mục.
pub(crate) struct Indexer(pub Arc<ContentService>);

#[async_trait::async_trait]
impl JobHandler for Indexer {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let input: IndexJob = serde_json::from_value(job.payload.clone())
            .map_err(|error| Error::internal(format!("content index job payload: {error}")))?;
        self.0.index_entry(input.entry_id).await
    }
}

/// Bộ chạy job lập index còn thiếu.
pub(crate) struct CatchUp(pub Arc<ContentService>);

#[async_trait::async_trait]
impl JobHandler for CatchUp {
    async fn run(&self, _: &JobContext) -> Result<(), Error> {
        self.0.catch_up_embeddings().await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cat_doan_chong_nhau() {
        assert_eq!(chunks("T", "", ""), ["T"]);
        assert_eq!(chunks("T", "E", "a b"), ["T\nE\na b"]);
        let text: Vec<String> = (0..700).map(|i| i.to_string()).collect();
        let pieces = chunks("", "", &text.join(" "));
        assert_eq!(pieces.len(), 3);
        assert!(pieces[0].starts_with("0 ") && pieces[0].ends_with(" 299"));
        assert!(pieces[1].starts_with("250 "), "overlaps the previous chunk");
        assert!(pieces[2].ends_with(" 699"));
    }
}
