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
//! - Theo khách (bước 33): vector trong bộ nhớ giữ theo (khách, `scope`, `locale`), câu tìm theo
//!   (khách, mô hình, câu), ngân sách tìm công khai đếm riêng mỗi khách — khách này không bao giờ
//!   thấy vector, câu tìm hay hạn mức của khách kia. Job lập index bù chạy một lần mỗi database,
//!   xếp job lập index cho từng khách có mục trên database đó.

use autoxide_kernel::time::Timestamp;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use autoxide_kernel::access::Principal;
use autoxide_kernel::ai::{EmbedKind, vector};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext, TenantId};
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
use crate::publishing::words;
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

/// Khoá của vector trong bộ nhớ: (khách, bản nào, ngôn ngữ).
type IndexKey = (TenantId, Scope, String);
/// Khoá của vector câu tìm: (khách, "mô hình + câu đã bỏ dấu").
type QueryKey = (TenantId, String);

/// Bộ nhớ của tìm theo ý nghĩa trong một tiến trình. Mọi khoá bắt đầu bằng khách (bước 33).
#[derive(Default)]
pub(crate) struct SearchCache {
    // `Mutex` của thư viện chuẩn: giữ khoá rất ngắn, không có `await` bên trong.
    indexes: Mutex<HashMap<IndexKey, Arc<Loaded>>>,
    /// Khoá nạp của từng (khách, phạm vi, ngôn ngữ): nhiều lượt tìm cùng thấy cache cũ thì chỉ một lượt đọc database,
    /// các lượt kia chờ rồi đọc cache (single-flight). Giữ `Weak`: khoá tự mất khi không còn ai nạp.
    loading: Mutex<HashMap<IndexKey, std::sync::Weak<tokio::sync::Mutex<()>>>>,
    /// Vector câu tìm theo (khách, "mô hình + câu"): khách có thể dùng mô hình riêng (khoá AI
    /// theo khách), và câu tìm của khách này không được lộ qua thời gian trả lời ở khách kia.
    queries: Mutex<HashMap<QueryKey, Arc<Vec<f32>>>>,
    /// Ngân sách tìm công khai theo ý nghĩa của từng khách: (đầu phút, số lần đã dùng trong
    /// phút đó). Khách này tiêu hết không làm khách kia mất tìm theo ý nghĩa.
    public_budget: Mutex<HashMap<TenantId, (Instant, u32)>>,
    /// Chốt của tìm công khai: số lượt cùng lúc, số lượt mỗi phút theo địa chỉ (PT-15).
    pub(crate) public_limits: super::public_limits::PublicLimits,
}

impl SearchCache {
    /// Khoá nạp (single-flight) của `key`: cùng một khoá cho mọi lượt đang nạp cùng `key`.
    fn loading_gate(&self, key: &IndexKey) -> Arc<tokio::sync::Mutex<()>> {
        let Ok(mut loading) = self.loading.lock() else {
            // Mutex hỏng (một thread panic khi giữ): không chặn tìm, mỗi lượt tự nạp như trước.
            return Arc::new(tokio::sync::Mutex::new(()));
        };
        if let Some(gate) = loading.get(key).and_then(std::sync::Weak::upgrade) {
            return gate;
        }
        // Dọn khoá không còn ai giữ: bảng không lớn dần theo số (khách, ngôn ngữ) từng tìm.
        loading.retain(|_, gate| gate.strong_count() > 0);
        let gate = Arc::new(tokio::sync::Mutex::new(()));
        loading.insert(key.clone(), Arc::downgrade(&gate));
        gate
    }

    /// Số `version` của vector (khách, `scope`, `locale`) đang giữ trong bộ nhớ (cho test).
    #[cfg(test)]
    pub(crate) fn cached_version(
        &self,
        tenant: TenantId,
        scope: Scope,
        locale: &str,
    ) -> Option<i64> {
        let cache = self.indexes.lock().ok()?;
        cache.get(&(tenant, scope, locale.to_owned())).map(|loaded| loaded.version)
    }
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
        tenant: &TenantContext,
        connection: &C,
        entry_id: Uuid,
    ) -> Result<(), Error> {
        if self.embedder.enabled() {
            self.jobs
                .enqueue(connection, tenant, &IndexJob { entry_id }, EnqueueOptions::default())
                .await?;
        }
        Ok(())
    }

    /// Lập lại index (bản nháp và bản công khai) của mục `entry_id`.
    pub(crate) async fn index_entry(
        &self,
        tenant: &TenantContext,
        entry_id: Uuid,
    ) -> Result<(), Error> {
        if !self.embedder.enabled() {
            return Ok(());
        }
        let read = tenant.database().read();
        let draft =
            entry::Entity::find_by_id(entry_id).scoped(tenant).one(read).await?.map(|row| {
                (
                    row.locale.clone(),
                    chunks(&row.title, &row.excerpt, &row.plain_text),
                    row.updated_at,
                )
            });
        let public = published::Entity::find()
            .scoped(tenant)
            .filter(published::Column::EntryId.eq(entry_id))
            .one(read)
            .await?
            .map(|row| {
                (
                    row.locale.clone(),
                    chunks(&row.title, &row.excerpt, &row.plain_text),
                    row.updated_at,
                )
            });
        self.index_scope(tenant, entry_id, Scope::Draft, draft).await?;
        self.index_scope(tenant, entry_id, Scope::Published, public).await
    }

    async fn index_scope(
        &self,
        tenant: &TenantContext,
        entry_id: Uuid,
        scope: Scope,
        source: Option<(String, Vec<String>, Timestamp)>,
    ) -> Result<(), Error> {
        let model = self.embedder.model_id().to_owned();
        let read_stamp = source.as_ref().map(|(_, _, stamp)| *stamp);
        let dimensions = i32::try_from(self.embedder.dimensions()).unwrap_or(0);
        let existing = embedding::Entity::find()
            .scoped(tenant)
            .filter(embedding::Column::EntryId.eq(entry_id))
            .filter(embedding::Column::Scope.eq(scope.as_str()))
            .order_by_asc(embedding::Column::ChunkIndex)
            .all(tenant.database().read())
            .await?;
        let Some((locale, texts, _)) = source else {
            if let Some(first) = existing.first() {
                ensure_state(tenant.database().write(), tenant, scope, &first.locale).await?;
                let transaction = tenant.database().begin_write().await?;
                if stamp(&transaction, tenant, entry_id, scope).await?.is_some() {
                    // Mục vừa có lại (vd đăng lại) sau lúc đọc: để job mới hơn lo.
                    return Ok(());
                }
                delete_rows(&transaction, tenant, entry_id, scope).await?;
                bump(&transaction, tenant, scope, &first.locale).await?;
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
        let mut fresh =
            self.embedder.embed(tenant, "content.index", &missing, EmbedKind::Document).await?;
        for vector in &mut fresh {
            vector::normalize(vector);
        }
        let mut fresh = fresh.into_iter();
        let now = now();
        ensure_state(tenant.database().write(), tenant, scope, &locale).await?;
        let transaction = tenant.database().begin_write().await?;
        // Mục đổi trong lúc tính embedding (job khác sẽ chạy với chữ mới): không ghi đè bằng
        // vector của chữ cũ.
        if stamp(&transaction, tenant, entry_id, scope).await? != read_stamp {
            return Ok(());
        }
        delete_rows(&transaction, tenant, entry_id, scope).await?;
        for (index, hash) in hashes.iter().enumerate() {
            let bytes = match known.get(hash.as_str()) {
                Some(bytes) => bytes.clone(),
                None => fresh.next().map(|vector| vector::encode(&vector)).unwrap_or_default(),
            };
            embedding::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
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
        bump(&transaction, tenant, scope, &locale).await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Xếp job lập index cho các mục trên `database` chưa có index với mô hình hiện tại, theo
    /// từng khách có mục ở đó (tối đa [`CATCH_UP_BATCH`] mỗi khách mỗi lần). Trả số mục đã xếp.
    pub(crate) async fn catch_up_embeddings(&self, database: &Database) -> Result<usize, Error> {
        if !self.embedder.enabled() {
            return Ok(0);
        }
        // Khách có mục trên database này (quét cả database qua index có `tenant_id` đứng đầu).
        let tenants: Vec<Uuid> = entry::Entity::find()
            .select_only()
            .column(entry::Column::TenantId)
            .distinct()
            .into_tuple::<Uuid>()
            .all(database.read())
            .await?;
        let mut queued = 0;
        for id in tenants {
            match self.databases.tenant(TenantId::from_uuid(id)).await {
                Ok(tenant) => queued += self.catch_up_tenant(&tenant).await?,
                // Khách đã bị xoá khỏi sổ: dữ liệu còn lại do việc xoá khách lo, không đoán ở đây.
                Err(error) => {
                    tracing::warn!(tenant_id = %id, error = %error, "content search catch-up skipped a tenant");
                }
            }
        }
        Ok(queued)
    }

    /// Phần của khách `tenant` trong [`catch_up_embeddings`](Self::catch_up_embeddings).
    async fn catch_up_tenant(&self, tenant: &TenantContext) -> Result<usize, Error> {
        let read = tenant.database().read();
        let model = self.embedder.model_id().to_owned();
        let dimensions = i32::try_from(self.embedder.dimensions()).unwrap_or(0);
        let indexed: BTreeSet<Uuid> = embedding::Entity::find()
            .scoped(tenant)
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
            .scoped(tenant)
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
        let transaction = tenant.database().begin_write().await?;
        // Mỗi mục tối đa một job lập bù mỗi ngày: nhà cung cấp hỏng hay hết hạn mức thì không
        // xếp chồng job mỗi giờ.
        let day = Timestamp::now().format("%Y-%m-%d");
        for entry_id in &missing {
            let options =
                EnqueueOptions::default().unique_key(format!("content.index:{entry_id}:{day}"));
            self.jobs
                .enqueue(&transaction, tenant, &IndexJob { entry_id: *entry_id }, options)
                .await?;
        }
        transaction.commit().await?;
        tracing::info!(entries = missing.len(), "content entries queued for the search index");
        Ok(missing.len())
    }

    /// Tìm công khai trên website `site` của người đọc ở địa chỉ `ip`, qua các chốt của
    /// [`public_limits`](super::public_limits): tìm công khai tắt thì 404 như [`search`](Self::search); bật thì giới hạn
    /// theo địa chỉ, số lượt cùng lúc, trần thời gian trả lời, và trần riêng cho phần tìm theo ý nghĩa.
    pub(crate) async fn search_public(
        self: &Arc<Self>,
        tenant: &TenantContext,
        principal: Option<&Principal>,
        query: &SearchQuery,
        site: &autoxide_kernel::tenancy::Site,
        ip: Option<std::net::IpAddr>,
    ) -> Result<Vec<Hit>, Error> {
        if !self.config.public_semantic_search {
            return self.search(tenant, principal, query, Some(site)).await;
        }
        let slot = self.search_cache.public_limits.admit(
            ip,
            self.config.public_search_per_address,
            self.config.public_search_concurrency,
        )?;
        // Lượt tìm chạy trong task riêng giữ `slot` tới khi việc xong thật: hết giờ trả lời hay client ngắt chỉ bỏ phần
        // chờ ở đây; câu truy vấn và việc tính vector đang chạy vẫn chiếm chỗ cho tới khi xong (`tokio::spawn` cần dữ
        // liệu sở hữu: chép `Arc` của service và các tham số).
        let (answer, answered) = tokio::sync::oneshot::channel();
        let service = Arc::clone(self);
        let (tenant, principal, query, site) =
            (tenant.clone(), principal.cloned(), query.clone(), site.clone());
        tokio::spawn(async move {
            let _slot = slot;
            let found = service
                .search_with(
                    &tenant,
                    principal.as_ref(),
                    &query,
                    Some(&site),
                    Some(super::public_limits::MEANING_BUDGET),
                )
                .await;
            let warm = found.as_ref().ok().and_then(|(_, warm)| warm.clone());
            let _ = answer.send(found.map(|(hits, _)| hits));
            // Phần ý nghĩa quá giờ: nạp xong index của bản đã đăng ở đây (đã trả lời rồi), để lần sau có sẵn trong bộ nhớ
            // — không thì mọi lần nạp đều bị cắt giữa chừng và index không bao giờ vào bộ nhớ.
            if let Some(locale) = warm
                && let Err(error) = service.loaded(&tenant, Scope::Published, &locale).await
            {
                tracing::warn!(code = error.code(), "could not load the published search index");
            }
        });
        match tokio::time::timeout(super::public_limits::TIMEOUT, answered).await {
            Ok(Ok(found)) => found,
            Ok(Err(_)) => Err(Error::internal("the public search task ended without an answer")),
            Err(_) => Err(super::public_limits::timed_out()),
        }
    }

    /// Tìm bản nháp hay bản công khai — xem [`search_with`](Self::search_with).
    pub(crate) async fn search(
        &self,
        tenant: &TenantContext,
        principal: Option<&Principal>,
        query: &SearchQuery,
        site: Option<&autoxide_kernel::tenancy::Site>,
    ) -> Result<Vec<Hit>, Error> {
        Ok(self.search_with(tenant, principal, query, site, None).await?.0)
    }

    /// Tìm bản nháp (người biên tập; cần `content.read`) hay bản công khai (`public`, không cần
    /// đăng nhập, tắt mặc định).
    /// `meaning_budget`: trần của phần tìm theo ý nghĩa; quá thì chỉ trả kết quả theo chữ, và trả kèm ngôn ngữ có index
    /// chưa nạp xong (`Some(locale)`) để bên gọi nạp sau.
    async fn search_with(
        &self,
        tenant: &TenantContext,
        principal: Option<&Principal>,
        query: &SearchQuery,
        site: Option<&autoxide_kernel::tenancy::Site>,
        meaning_budget: Option<Duration>,
    ) -> Result<(Vec<Hit>, Option<String>), Error> {
        // `site`: tìm công khai trên website đó (chỉ bài hiện trên website, D-307); `None`: bản nháp.
        let public = site.is_some();
        if public {
            if !self.config.public_semantic_search {
                return Err(Error::not_found(
                    "content.semantic_search_disabled",
                    "Search by meaning is not offered to the public on this server (AUTOXIDE_CONTENT_PUBLIC_SEMANTIC_SEARCH); search by words instead.",
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
            problems.push(
                FieldError::new("q", "length", "must be 1 to {query_max} characters")
                    .with_param("query_max", QUERY_MAX),
            );
        }
        let limit = query.limit.unwrap_or(LIMIT_DEFAULT);
        if !(1..=LIMIT_MAX).contains(&limit) {
            problems.push(
                FieldError::new("limit", "range", "must be 1 to {limit_max}")
                    .with_param("limit_max", LIMIT_MAX),
            );
        }
        let languages = match site {
            Some(site) => self.site_languages(tenant, site).await?,
            None => self.languages(tenant).await?,
        };
        let locale =
            self.check_locale(&languages, query.locale.as_deref(), "locale", &mut problems);
        if !problems.is_empty() {
            return Err(invalid(problems));
        }
        let scope = if public { Scope::Published } else { Scope::Draft };
        let by_text = self.text_candidates(tenant, scope, q, &locale, query.kind).await?;
        let mut unloaded = None;
        let by_meaning = if self.embedder.enabled() && (!public || self.spend_public_budget(tenant))
        {
            let meaning = self.meaning_candidates(tenant, scope, q, &locale);
            let outcome = match meaning_budget {
                None => Some(meaning.await),
                Some(budget) => tokio::time::timeout(budget, meaning).await.ok(),
            };
            match outcome {
                Some(Ok(found)) => found,
                None => {
                    tracing::warn!("search by meaning too slow; matching words only");
                    // Phần ý nghĩa không cho kết quả: không tính vào ngân sách mỗi phút của khách.
                    if public {
                        self.refund_public_budget(tenant);
                    }
                    unloaded = Some(locale.clone());
                    Vec::new()
                }
                Some(Err(error)) => {
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
        let mut hits = self.describe_hits(tenant, scope, &ids, query.kind, &fused).await?;
        // Tìm bản nháp: chỉ mục trong phạm vi `content.read` của người gọi (bước 34).
        if let (false, Some(principal)) = (public, principal) {
            let scope = principal.scope_condition(
                &CONTENT_READ,
                entry::Column::OrgId,
                Some(entry::Column::CreatedBy),
            )?;
            let permitted: std::collections::HashSet<Uuid> = entry::Entity::find()
                .filter(scope)
                .filter(entry::Column::Id.is_in(hits.iter().map(|hit| hit.entry_id)))
                .scoped(tenant)
                .all(tenant.database().read())
                .await?
                .into_iter()
                .map(|row| row.id)
                .collect();
            hits.retain(|hit| permitted.contains(&hit.entry_id));
        }
        if let Some(site) = site {
            let shown: std::collections::HashSet<Uuid> = published::Entity::find()
                .filter(crate::publishing::reading::on_site(&site.lineage_ids()))
                .filter(published::Column::EntryId.is_in(hits.iter().map(|hit| hit.entry_id)))
                .scoped(tenant)
                .all(tenant.database().read())
                .await?
                .into_iter()
                .map(|row| row.entry_id)
                .collect();
            hits.retain(|hit| shown.contains(&hit.entry_id));
        }
        hits.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.entry_id.cmp(&b.entry_id)));
        hits.truncate(limit as usize);
        Ok((hits, unloaded))
    }

    /// Mục khớp chữ (không dấu, D-104), mới nhất trước.
    async fn text_candidates(
        &self,
        tenant: &TenantContext,
        scope: Scope,
        q: &str,
        locale: &str,
        kind: Option<Kind>,
    ) -> Result<Vec<Uuid>, Error> {
        let read = tenant.database().read();
        let limit = CANDIDATES as u64;
        Ok(match scope {
            Scope::Draft => {
                let mut select = entry::Entity::find()
                    .scoped(tenant)
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
            // Bản công khai: chỉ mục từ (PT-15), đã mới đăng trước.
            Scope::Published => {
                let matching = words::Matching {
                    tenant: tenant.id().as_uuid(),
                    locale,
                    kind: kind.map(Kind::as_str),
                };
                let mut ids =
                    words::matching_ids(read, matching, &words::WordQuery::parse(q)).await?;
                ids.truncate(CANDIDATES);
                ids
            }
        })
    }

    /// Mục gần câu tìm nhất theo ý nghĩa: (ID, độ giống, đoạn khớp nhất), giống nhất trước.
    async fn meaning_candidates(
        &self,
        tenant: &TenantContext,
        scope: Scope,
        q: &str,
        locale: &str,
    ) -> Result<Vec<(Uuid, f32, i32)>, Error> {
        let query = self.query_vector(tenant, q).await?;
        let loaded = self.loaded(tenant, scope, locale).await?;
        // Duyệt mọi vector là việc CPU dài (100 000 đoạn × 1 024 chiều ~50 ms một nhân): chạy trên
        // thread cho việc chặn của tokio để không giữ thread async, và chia ra các nhân (bước 39 B6,
        // `docs/BENCHMARKS.md` "Tìm theo ý nghĩa").
        // Future bị bỏ (quá giờ, client ngắt) thì `_cancel` bật cờ và vòng duyệt dừng sớm: việc CPU không chạy tiếp
        // ngoài trần số lượt cùng lúc của tìm công khai.
        let cancelled = Arc::new(AtomicBool::new(false));
        let _cancel = CancelOnDrop(cancelled.clone());
        let best =
            tokio::task::spawn_blocking(move || best_chunks(&query, &loaded.chunks, &cancelled))
                .await
                .map_err(Error::internal)??;
        let mut ranked: Vec<(Uuid, f32, i32)> =
            best.into_iter().map(|(id, (similarity, chunk))| (id, similarity, chunk)).collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        ranked.truncate(CANDIDATES);
        Ok(ranked)
    }

    /// Vector (đã chuẩn hoá) của câu tìm, giữ lại để lần sau khỏi gọi nhà cung cấp.
    async fn query_vector(&self, tenant: &TenantContext, q: &str) -> Result<Arc<Vec<f32>>, Error> {
        let key = (
            tenant.id(),
            format!("{}\n{}", self.embedder.model_id(), autoxide_kernel::search::fold(q)),
        );
        if let Some(found) =
            self.search_cache.queries.lock().ok().and_then(|cache| cache.get(&key).cloned())
        {
            return Ok(found);
        }
        let mut vectors = self
            .embedder
            .embed(tenant, "content.search", &[q.to_owned()], EmbedKind::Query)
            .await?;
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
    async fn loaded(
        &self,
        tenant: &TenantContext,
        scope: Scope,
        locale: &str,
    ) -> Result<Arc<Loaded>, Error> {
        let read = tenant.database().read();
        let version = embedding_state::Entity::find()
            .scoped(tenant)
            .filter(embedding_state::Column::Scope.eq(scope.as_str()))
            .filter(embedding_state::Column::Locale.eq(locale))
            .one(read)
            .await?
            .map_or(0, |row| row.version);
        let model = self.embedder.model_id().to_owned();
        let key = (tenant.id(), scope, locale.to_owned());
        let fresh = |cache: &SearchCache| {
            cache
                .indexes
                .lock()
                .ok()
                .and_then(|cache| cache.get(&key).cloned())
                .filter(|cached| cached.version == version && cached.model == model)
        };
        if let Some(cached) = fresh(&self.search_cache) {
            return Ok(cached);
        }
        // Một lượt nạp mỗi khoá: lượt tới sau chờ khoá, rồi thấy cache đã mới thì dùng luôn.
        let gate = self.search_cache.loading_gate(&key);
        let _loading = gate.lock().await;
        if let Some(cached) = fresh(&self.search_cache) {
            return Ok(cached);
        }
        let dimensions = i32::try_from(self.embedder.dimensions()).unwrap_or(0);
        let rows = embedding::Entity::find()
            .scoped(tenant)
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

    /// Một lần tìm công khai theo ý nghĩa trong ngân sách mỗi phút của khách `tenant` không
    /// (đếm trong tiến trình).
    fn spend_public_budget(&self, tenant: &TenantContext) -> bool {
        let Ok(mut budgets) = self.search_cache.public_budget.lock() else { return false };
        let now = Instant::now();
        // Bỏ phút đã qua của mọi khách: bảng không lớn dần theo số khách từng tìm.
        budgets.retain(|_, (start, _)| now.duration_since(*start) < Duration::from_secs(60));
        let (start, used) = budgets.get(&tenant.id()).copied().unwrap_or((now, 0));
        if used >= self.config.public_search_per_minute {
            return false;
        }
        budgets.insert(tenant.id(), (start, used + 1));
        true
    }

    /// Trả lại một lần đã trừ bởi [`spend_public_budget`](Self::spend_public_budget) trong phút hiện tại.
    fn refund_public_budget(&self, tenant: &TenantContext) {
        let Ok(mut budgets) = self.search_cache.public_budget.lock() else { return };
        if let Some((_, used)) = budgets.get_mut(&tenant.id()) {
            *used = used.saturating_sub(1);
        }
    }

    /// Chi tiết của các mục tìm được (bỏ mục không còn, hay khác `kind`).
    async fn describe_hits(
        &self,
        tenant: &TenantContext,
        scope: Scope,
        ids: &[Uuid],
        kind: Option<Kind>,
        fused: &BTreeMap<Uuid, Fused>,
    ) -> Result<Vec<Hit>, Error> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let read = tenant.database().read();
        // (ID, loại, ngôn ngữ, tiêu đề, tóm tắt, chữ thuần, trạng thái, đường dẫn)
        type Row = (Uuid, String, String, String, String, String, Option<String>, Option<String>);
        let rows: Vec<Row> = match scope {
            Scope::Draft => entry::Entity::find()
                .scoped(tenant)
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
                .scoped(tenant)
                .filter(published::Column::EntryId.is_in(ids.to_vec()))
                // Mục bị ẩn vì vụ việc (D-363) không hiện trong tìm kiếm công khai.
                .filter(published::Column::RestrictedAt.is_null())
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
/// Số đoạn tối thiểu mỗi khúc khi chia ra nhiều nhân: ít hơn thì mở thread tốn hơn tính.
const CHUNKS_PER_THREAD: usize = 10_000;

/// Đoạn giống `query` nhất của mỗi mục: mục → (độ giống, số thứ tự đoạn). Chia `chunks` thành các
/// khúc liền nhau, mỗi nhân một khúc (`std::thread::scope`: thread mượn được dữ liệu, xong hết mới
/// trả về), rồi gộp theo thứ tự khúc — bằng nhau thì giữ đoạn đứng trước, như duyệt một lượt.
fn best_chunks(
    query: &[f32],
    chunks: &[(Uuid, i32, Vec<f32>)],
    cancelled: &AtomicBool,
) -> Result<HashMap<Uuid, (f32, i32)>, Error> {
    let threads = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(chunks.len() / CHUNKS_PER_THREAD)
        .max(1);
    if threads == 1 {
        return best_in(query, chunks, cancelled);
    }
    let size = chunks.len().div_ceil(threads);
    let parts: Vec<HashMap<Uuid, (f32, i32)>> = std::thread::scope(|scope| {
        let handles: Vec<_> = chunks
            .chunks(size)
            .map(|part| scope.spawn(move || best_in(query, part, cancelled)))
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle.join().map_err(|_| Error::internal("a semantic search thread failed"))?
            })
            .collect::<Result<_, _>>()
    })?;
    let mut best: HashMap<Uuid, (f32, i32)> = HashMap::new();
    for part in parts {
        for (entry_id, (similarity, chunk)) in part {
            let slot = best.entry(entry_id).or_insert((f32::MIN, chunk));
            if similarity > slot.0 {
                *slot = (similarity, chunk);
            }
        }
    }
    Ok(best)
}

fn best_in(
    query: &[f32],
    chunks: &[(Uuid, i32, Vec<f32>)],
    cancelled: &AtomicBool,
) -> Result<HashMap<Uuid, (f32, i32)>, Error> {
    let mut best: HashMap<Uuid, (f32, i32)> = HashMap::new();
    for (at, (entry_id, chunk, vector)) in chunks.iter().enumerate() {
        if at % CANCEL_CHECK_EVERY == 0 && cancelled.load(Ordering::Relaxed) {
            return Err(Error::internal("search by meaning was cancelled"));
        }
        let similarity = vector::dot(query, vector);
        let slot = best.entry(*entry_id).or_insert((f32::MIN, *chunk));
        if similarity > slot.0 {
            *slot = (similarity, *chunk);
        }
    }
    Ok(best)
}

/// Bật cờ huỷ khi bị bỏ (xem [`best_in`]).
struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Số đoạn giữa hai lần xem cờ huỷ: đủ thưa để không tốn, đủ dày để dừng trong vài mili giây.
const CANCEL_CHECK_EVERY: usize = 1_024;

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
    tenant: &TenantContext,
    entry_id: Uuid,
    scope: Scope,
) -> Result<(), Error> {
    embedding::Entity::delete_many()
        .scoped(tenant)
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
    tenant: &TenantContext,
    scope: Scope,
    locale: &str,
) -> Result<(), Error> {
    let exists = embedding_state::Entity::find()
        .scoped(tenant)
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
        tenant_id: Set(tenant.id().as_uuid()),
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
async fn bump<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    scope: Scope,
    locale: &str,
) -> Result<(), Error> {
    embedding_state::Entity::update_many()
        .scoped(tenant)
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
    tenant: &TenantContext,
    entry_id: Uuid,
    scope: Scope,
) -> Result<Option<Timestamp>, Error> {
    Ok(match scope {
        Scope::Draft => entry::Entity::find_by_id(entry_id)
            .scoped(tenant)
            .one(connection)
            .await?
            .map(|row| row.updated_at),
        Scope::Published => published::Entity::find()
            .scoped(tenant)
            .filter(published::Column::EntryId.eq(entry_id))
            .one(connection)
            .await?
            .map(|row| row.updated_at),
    })
}

/// Bộ chạy job lập index của một mục.
pub(crate) struct Indexer(pub Arc<ContentService>);

#[async_trait::async_trait]
impl JobHandler for Indexer {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let tenant = job
            .tenant
            .as_ref()
            .ok_or_else(|| Error::internal("a content index job must belong to a tenant"))?;
        let input: IndexJob = serde_json::from_value(job.payload.clone())
            .map_err(|error| Error::internal(format!("content index job payload: {error}")))?;
        self.0.index_entry(tenant, input.entry_id).await
    }
}

/// Bộ chạy job lập index còn thiếu (theo lịch: một lần cho mỗi database).
pub(crate) struct CatchUp(pub Arc<ContentService>);

#[async_trait::async_trait]
impl JobHandler for CatchUp {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.catch_up_embeddings(&job.database).await.map(|_| ())
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

    #[test]
    fn chia_nhieu_nhan_cho_ket_qua_nhu_duyet_mot_luot() {
        // Đủ đoạn để chia ra nhiều khúc; mỗi mục 3 đoạn, có đoạn trùng điểm để thử luật "bằng nhau
        // thì giữ đoạn đứng trước" qua ranh giới khúc.
        let entries: Vec<Uuid> = (0..12_000).map(|_| Uuid::now_v7()).collect();
        let mut data = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            for chunk in 0..3 {
                let value = ((index * 7 + chunk * 3) % 11) as f32;
                let tie = if chunk == 2 { 0.0 } else { value };
                data.push((*entry, i32::try_from(chunk).unwrap(), vec![tie, 1.0, 0.5]));
            }
        }
        let query = [0.3, 0.2, 0.1];
        let parallel = best_chunks(&query, &data, &AtomicBool::new(false)).unwrap();
        assert_eq!(parallel, best_in(&query, &data, &AtomicBool::new(false)).unwrap());
        assert_eq!(parallel.len(), entries.len());
    }

    #[test]
    fn co_huy_dung_vong_duyet() {
        let data = vec![(Uuid::now_v7(), 0, vec![1.0_f32])];
        assert!(best_in(&[1.0], &data, &AtomicBool::new(true)).is_err());
        assert!(best_chunks(&[1.0], &data, &AtomicBool::new(true)).is_err());
        let cancel = Arc::new(AtomicBool::new(false));
        drop(CancelOnDrop(cancel.clone()));
        assert!(cancel.load(Ordering::Relaxed), "dropping the guard sets the flag");
    }
}
