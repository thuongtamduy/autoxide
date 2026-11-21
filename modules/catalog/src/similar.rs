//! Sản phẩm tương tự theo ý nghĩa (B-c14) bằng embedding của module `ai` (`dyn Embedder`, D-109).
//!
//! - **Lập index**: một vector mỗi (sản phẩm đang bán, ngôn ngữ) từ tên + mô tả ngắn + mô tả, lưu ở
//!   `catalog_embeddings`; chữ không đổi (cùng SHA-256 của mô hình + chữ) thì không tính lại. Job
//!   [`INDEX`] được xếp trong transaction dựng lại storefront — mọi thay đổi làm sản phẩm hiện / ẩn
//!   hay đổi chữ đều đi qua đó. Sản phẩm thôi bán thì bỏ vector và danh sách của nó.
//! - **Tính sẵn danh sách**: lúc lập index cho P, duyệt vector cùng khách, cùng ngôn ngữ, cùng mô
//!   hình (việc của job, không ở đường đọc), giữ [`TOP`] sản phẩm gần nhất của P trong
//!   `catalog_similar_products`; đồng thời thêm P vào danh sách của các sản phẩm đó (cắt về
//!   [`TOP`]) — sản phẩm mới hiện ngay ở trang của hàng giống nó.
//! - **Đọc công khai**: một câu theo index (`tenant_id`, `product_id`, `locale`, `score`), lọc theo
//!   storefront của website. Chưa có danh sách (embedding tắt, chưa lập) thì lấy hàng cùng danh
//!   mục, bán chạy trước — vẫn có kết quả.
//!
//! Vector giữ theo khách (bước 33): không câu nào đọc vector của khách khác.

use autoxide_kernel::time::Timestamp;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use autoxide_kernel::ai::{EmbedKind, vector};
use autoxide_kernel::error::Error;
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::entity::{embedding, product, similar_product, translation};
use crate::service::{CatalogService, now};

/// Loại job lập index embedding của sản phẩm.
pub(crate) const INDEX: &str = "catalog.index_embeddings";
/// Số sản phẩm tương tự giữ cho mỗi sản phẩm.
pub(crate) const TOP: usize = 12;
/// Số ký tự tối đa của chữ gửi đi tính embedding (tên và mô tả ngắn đứng đầu nên không mất).
const TEXT_MAX: usize = 4_000;
/// Số đoạn chữ mỗi lần gọi mô hình.
const EMBED_BATCH: usize = 32;
/// Điểm thấp hơn ngưỡng (cosine × 10 000) thì không coi là tương tự.
const MIN_SCORE: i32 = 1_000;

/// Job: lập lại index của các sản phẩm.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct IndexJob {
    pub products: Vec<Uuid>,
}

impl Job for IndexJob {
    const KIND: &'static str = INDEX;
}

/// Chữ của một bản dịch dùng tính embedding.
fn text_of(row: &translation::Model) -> String {
    let mut text = row.name.clone();
    for part in [&row.short_description, &row.description] {
        if !part.trim().is_empty() {
            text.push('\n');
            text.push_str(part.trim());
        }
    }
    text.chars().take(TEXT_MAX).collect()
}

fn hash(model: &str, text: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(model.as_bytes());
    digest.update(b"\n");
    digest.update(text.as_bytes());
    digest.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Cosine × 10 000 làm tròn, để lưu và sắp bằng số nguyên trên cả hai database.
fn score(a: &[f32], b: &[f32]) -> i32 {
    // `as`: cosine nằm trong [-1, 1], nhân 10 000 vẫn trong khoảng của `i32`.
    let value = (vector::dot(a, b) * 10_000.0).round() as i32;
    value.clamp(-10_000, 10_000)
}

impl CatalogService {
    /// Embedding đang bật (module `ai` đã cài và cấu hình).
    pub(crate) fn embeddings_on(&self) -> bool {
        self.embedder.as_ref().is_some_and(|embedder| embedder.enabled())
    }

    /// Xếp job lập index cho `products` trong transaction của bên gọi (khi embedding bật).
    pub(crate) async fn queue_embeddings<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        products: &[Uuid],
    ) -> Result<(), Error> {
        if !self.embeddings_on() || products.is_empty() {
            return Ok(());
        }
        let job = IndexJob { products: products.to_vec() };
        self.jobs.enqueue(connection, tenant, &job, EnqueueOptions::default()).await?;
        Ok(())
    }

    /// Lập lại index của `products`: tính vector đổi, bỏ vector của sản phẩm thôi bán, tính lại
    /// danh sách tương tự.
    pub(crate) async fn index_embeddings(
        &self,
        tenant: &TenantContext,
        products: &[Uuid],
    ) -> Result<(), Error> {
        let Some(embedder) = self.embedder.clone().filter(|embedder| embedder.enabled()) else {
            return Ok(());
        };
        let model = embedder.model_id().to_owned();
        let read = tenant.database().read();
        let active: BTreeSet<Uuid> = product::Entity::find()
            .select_only()
            .column(product::Column::Id)
            .filter(product::Column::Id.is_in(products.iter().copied()))
            .filter(product::Column::Status.eq("active"))
            .scoped(tenant)
            .into_tuple::<Uuid>()
            .all(read)
            .await?
            .into_iter()
            .collect();
        let gone: Vec<Uuid> = products.iter().copied().filter(|id| !active.contains(id)).collect();
        if !gone.is_empty() {
            let write = tenant.database().begin_write().await?;
            embedding::Entity::delete_many()
                .filter(embedding::Column::ProductId.is_in(gone.clone()))
                .scoped(tenant)
                .exec(&write)
                .await?;
            similar_product::Entity::delete_many()
                .filter(similar_product::Column::ProductId.is_in(gone))
                .scoped(tenant)
                .exec(&write)
                .await?;
            write.commit().await?;
        }
        if active.is_empty() {
            return Ok(());
        }
        let texts = translation::Entity::find()
            .filter(translation::Column::ProductId.is_in(active.iter().copied()))
            .scoped(tenant)
            .all(read)
            .await?;
        let stored: HashMap<(Uuid, String), embedding::Model> = embedding::Entity::find()
            .filter(embedding::Column::ProductId.is_in(active.iter().copied()))
            .scoped(tenant)
            .all(read)
            .await?
            .into_iter()
            .map(|row| ((row.product_id, row.locale.clone()), row))
            .collect();
        // Bản dịch có chữ đổi (hay đổi mô hình) cần tính lại.
        let mut todo: Vec<(Uuid, String, String, String)> = Vec::new();
        for row in &texts {
            let text = text_of(row);
            let digest = hash(&model, &text);
            let same = stored
                .get(&(row.product_id, row.locale.clone()))
                .is_some_and(|old| old.content_hash == digest && old.model == model);
            if !same {
                todo.push((row.product_id, row.locale.clone(), text, digest));
            }
        }
        let mut changed: Vec<(Uuid, String)> = Vec::new();
        for batch in todo.chunks(EMBED_BATCH) {
            let inputs: Vec<String> = batch.iter().map(|(_, _, text, _)| text.clone()).collect();
            let vectors = embedder
                .embed(tenant, "catalog.similar_products", &inputs, EmbedKind::Document)
                .await?;
            let write = tenant.database().begin_write().await?;
            for ((product_id, locale, _, digest), mut values) in batch.iter().zip(vectors) {
                vector::normalize(&mut values);
                embedding::Entity::delete_many()
                    .filter(embedding::Column::ProductId.eq(*product_id))
                    .filter(embedding::Column::Locale.eq(locale.as_str()))
                    .scoped(tenant)
                    .exec(&write)
                    .await?;
                embedding::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    product_id: Set(*product_id),
                    locale: Set(locale.clone()),
                    content_hash: Set(digest.clone()),
                    model: Set(model.clone()),
                    dimensions: Set(i32::try_from(values.len()).unwrap_or(i32::MAX)),
                    vector: Set(vector::encode(&values)),
                    updated_at: Set(now()),
                }
                .insert(&write)
                .await?;
                changed.push((*product_id, locale.clone()));
            }
            write.commit().await?;
        }
        let mut locales: BTreeSet<&str> = BTreeSet::new();
        for (_, locale) in &changed {
            locales.insert(locale);
        }
        for locale in locales {
            let mine: Vec<Uuid> =
                changed.iter().filter(|(_, l)| l == locale).map(|(id, _)| *id).collect();
            self.recompute_similar(tenant, &model, locale, &mine).await?;
        }
        Ok(())
    }

    /// Tính lại danh sách tương tự của `products` ở `locale`, và thêm mỗi sản phẩm vào danh sách
    /// của hàng giống nó.
    async fn recompute_similar(
        &self,
        tenant: &TenantContext,
        model: &str,
        locale: &str,
        products: &[Uuid],
    ) -> Result<(), Error> {
        let read = tenant.database().read();
        let all: Vec<(Uuid, Vec<f32>)> = embedding::Entity::find()
            .filter(embedding::Column::Locale.eq(locale))
            .filter(embedding::Column::Model.eq(model))
            .scoped(tenant)
            .all(read)
            .await?
            .into_iter()
            .filter_map(|row| Some((row.product_id, vector::decode(&row.vector)?)))
            .collect();
        let by_id: HashMap<Uuid, &Vec<f32>> = all.iter().map(|(id, v)| (*id, v)).collect();
        let at = now();
        for product in products {
            let Some(mine) = by_id.get(product) else { continue };
            let mut ranked: Vec<(Uuid, i32)> = all
                .iter()
                .filter(|(other, _)| other != product)
                .map(|(other, values)| (*other, score(mine, values)))
                .filter(|(_, value)| *value >= MIN_SCORE)
                .collect();
            ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            ranked.truncate(TOP);
            let write = tenant.database().begin_write().await?;
            similar_product::Entity::delete_many()
                .filter(similar_product::Column::ProductId.eq(*product))
                .filter(similar_product::Column::Locale.eq(locale))
                .scoped(tenant)
                .exec(&write)
                .await?;
            for (other, value) in &ranked {
                self.put_similar(&write, tenant, locale, (*product, *other, *value), at).await?;
                // Đối xứng: P cũng là hàng tương tự của Q; giữ Q tối đa `TOP` dòng tốt nhất.
                self.put_similar(&write, tenant, locale, (*other, *product, *value), at).await?;
                self.trim_similar(&write, tenant, *other, locale).await?;
            }
            write.commit().await?;
        }
        Ok(())
    }

    /// Ghi (thay) dòng "`other` giống `product` với điểm `value`".
    async fn put_similar<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        locale: &str,
        (product, other, value): (Uuid, Uuid, i32),
        at: Timestamp,
    ) -> Result<(), Error> {
        similar_product::Entity::delete_many()
            .filter(similar_product::Column::ProductId.eq(product))
            .filter(similar_product::Column::Locale.eq(locale))
            .filter(similar_product::Column::OtherProductId.eq(other))
            .scoped(tenant)
            .exec(connection)
            .await?;
        similar_product::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            product_id: Set(product),
            locale: Set(locale.to_owned()),
            other_product_id: Set(other),
            score: Set(value),
            updated_at: Set(at),
        }
        .insert(connection)
        .await?;
        Ok(())
    }

    /// Giữ `TOP` dòng điểm cao nhất của `product`.
    async fn trim_similar<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        product: Uuid,
        locale: &str,
    ) -> Result<(), Error> {
        let extra: Vec<Uuid> = similar_product::Entity::find()
            .select_only()
            .column(similar_product::Column::Id)
            .filter(similar_product::Column::ProductId.eq(product))
            .filter(similar_product::Column::Locale.eq(locale))
            .scoped(tenant)
            .order_by_desc(similar_product::Column::Score)
            .order_by_asc(similar_product::Column::OtherProductId)
            .into_tuple()
            .all(connection)
            .await?
            // Danh sách tối đa `TOP + 1` dòng (vừa thêm một): cắt trong Rust thay vì `OFFSET` không
            // kèm `LIMIT` (SQLite không nhận).
            .into_iter()
            .skip(TOP)
            .collect();
        if !extra.is_empty() {
            similar_product::Entity::delete_many()
                .filter(similar_product::Column::Id.is_in(extra))
                .scoped(tenant)
                .exec(connection)
                .await?;
        }
        Ok(())
    }

    /// Danh sách tương tự tính sẵn của `product` ở `locale`, giống nhất trước.
    pub(crate) async fn similar_products(
        &self,
        tenant: &TenantContext,
        product: Uuid,
        locale: &str,
    ) -> Result<Vec<Uuid>, Error> {
        Ok(similar_product::Entity::find()
            .select_only()
            .column(similar_product::Column::OtherProductId)
            .filter(similar_product::Column::ProductId.eq(product))
            .filter(similar_product::Column::Locale.eq(locale))
            .scoped(tenant)
            .order_by_desc(similar_product::Column::Score)
            .order_by_asc(similar_product::Column::OtherProductId)
            .limit(TOP as u64)
            .into_tuple()
            .all(tenant.database().read())
            .await?)
    }
}

/// Bộ chạy job [`IndexJob`].
pub(crate) struct Indexer(pub(crate) Arc<CatalogService>);

#[async_trait::async_trait]
impl JobHandler for Indexer {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let Some(tenant) = &job.tenant else { return Ok(()) };
        let input: IndexJob = serde_json::from_value(job.payload.clone()).unwrap_or_default();
        for chunk in input.products.chunks(crate::api::MAX_IDS) {
            self.0.index_embeddings(tenant, chunk).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diem_la_cosine_nhan_muoi_nghin() {
        let mut a = vec![1.0, 0.0];
        let mut b = vec![1.0, 1.0];
        vector::normalize(&mut a);
        vector::normalize(&mut b);
        assert_eq!(score(&a, &a), 10_000);
        assert_eq!(score(&a, &b), 7_071);
        assert_eq!(hash("m", "x").len(), 64);
        assert_ne!(hash("m", "x"), hash("n", "x"));
    }
}
