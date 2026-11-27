//! Tìm theo ý nghĩa (D-109): index theo đoạn, không tính lại đoạn không đổi, gộp với tìm theo
//! chữ, nhiều máy chủ cùng database, đổi mô hình, embedding tắt.

use std::sync::Arc;

use autoxide_testkit::ai::{FakeEmbedder, FakeTextModel};

use super::*;
use crate::ai::semantic::SearchQuery;
use crate::testing::service_with;
use autoxide_kernel::tenancy::{Site, TenantContext};

async fn rows_of(database: &Database, entry_id: Uuid) -> u64 {
    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
    crate::entity::embedding::Entity::find()
        .filter(crate::entity::embedding::Column::EntryId.eq(entry_id))
        .count(database.read())
        .await
        .unwrap()
}

fn query(q: &str) -> SearchQuery {
    SearchQuery { q: q.into(), locale: Some("vi".into()), ..Default::default() }
}

fn vi_post(title: &str, text: &str) -> NewEntry {
    let mut input = post(title);
    input.locale = Some("vi".into());
    input.fields.body = Some(paragraph(text));
    input
}

#[tokio::test]
async fn tim_theo_y_nghia_va_theo_chu() {
    TestDatabase::run(async |test| {
        let embedder = Arc::new(FakeEmbedder::new(256));
        let (content, _, database) = service_with(
            test,
            config(&[("AUTOXIDE_CONTENT_PUBLIC_SEMANTIC_SEARCH", "true")]),
            Arc::new(FakeTextModel::disabled()),
            embedder.clone(),
        )
        .await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let dalat = content
            .create(&tenant, &ada, vi_post("Đà Lạt mùa hoa", "Hoa dã quỳ nở vàng đồi thông."))
            .await
            .unwrap();
        let rust = content
            .create(&tenant, &ada, vi_post("Học Rust", "Ownership và lifetime."))
            .await
            .unwrap();
        for id in [dalat.id, rust.id] {
            content.index_entry(&tenant, id).await.unwrap();
        }
        let calls = embedder.calls();
        // Không đổi gì: không gọi lại embedding.
        content.index_entry(&tenant, dalat.id).await.unwrap();
        assert_eq!(embedder.calls(), calls);

        // Khớp cả ý nghĩa lẫn chữ.
        let hits = content.search(&tenant, Some(&ada), &query("đà lạt"), None).await.unwrap();
        assert_eq!(hits[0].entry_id, dalat.id);
        assert_eq!(hits[0].matched_by, ["meaning", "text"]);
        assert!(hits[0].similarity.unwrap() > 0.0);
        assert_eq!(hits[0].status.as_deref(), Some("draft"));
        // Không chứa nguyên cụm chữ, nhưng gần về ý nghĩa.
        let hits =
            content.search(&tenant, Some(&ada), &query("hoa nở ở đà lạt"), None).await.unwrap();
        assert_eq!((hits[0].entry_id, hits[0].matched_by.clone()), (dalat.id, vec!["meaning"]));
        assert!(hits[0].snippet.contains("dã quỳ"));

        // Công khai: chỉ bản đã đăng, kèm đường dẫn.
        assert!(
            content
                .search(&tenant, None, &query("đà lạt"), Some(&Site::whole(&tenant)))
                .await
                .unwrap()
                .is_empty()
        );
        content.publish(&tenant, &ada, dalat.id, None, None, false).await.unwrap();
        content.index_entry(&tenant, dalat.id).await.unwrap();
        let public = content
            .search(&tenant, None, &query("đà lạt"), Some(&Site::whole(&tenant)))
            .await
            .unwrap();
        assert_eq!(public.len(), 1);
        assert_eq!(
            (public[0].path.as_deref(), public[0].status.clone()),
            (Some("/blog/da-lat-mua-hoa"), None)
        );

        // Kiểm đầu vào; cần quyền đọc.
        let bad = SearchQuery { q: " ".into(), limit: Some(51), ..Default::default() };
        let error = content.search(&tenant, Some(&ada), &bad, None).await.unwrap_err();
        assert_eq!(
            error.fields().iter().map(|f| f.field.as_str()).collect::<Vec<_>>(),
            ["q", "limit"]
        );
        assert_eq!(
            content
                .search(&tenant, Some(&user(&["write"])), &query("x"), None)
                .await
                .unwrap_err()
                .code(),
            "permission_denied"
        );

        // Xoá mục: index của nó biến mất.
        content.delete(&tenant, &ada, rust.id, None).await.unwrap();
        assert_eq!(rows_of(&database, rust.id).await, 1);
        content.index_entry(&tenant, rust.id).await.unwrap();
        assert_eq!(rows_of(&database, rust.id).await, 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nhieu_may_chu_va_doi_mo_hinh() {
    TestDatabase::run(async |test| {
        let embedder = Arc::new(FakeEmbedder::new(128));
        let ai = Arc::new(FakeTextModel::disabled());
        let (first, _, database) =
            service_with(test, config(&[]), ai.clone(), embedder.clone()).await;
        let tenant = TenantContext::default_on(database.clone());
        let (second, _, _) = service_with(test, config(&[]), ai.clone(), embedder.clone()).await;
        let ada = editor();
        let a =
            first.create(&tenant, &ada, vi_post("Phở Hà Nội", "Nước dùng trong.")).await.unwrap();
        first.index_entry(&tenant, a.id).await.unwrap();
        assert_eq!(second.search(&tenant, Some(&ada), &query("phở"), None).await.unwrap().len(), 1);
        // Máy chủ thứ nhất thêm mục: máy thứ hai (đã nạp vào bộ nhớ) thấy nhờ `version`.
        let b = first
            .create(&tenant, &ada, vi_post("Bún chả", "Phở và bún chả là món Hà Nội."))
            .await
            .unwrap();
        first.index_entry(&tenant, b.id).await.unwrap();
        let hits = second.search(&tenant, Some(&ada), &query("phở hà nội"), None).await.unwrap();
        assert_eq!(hits.iter().filter(|hit| hit.matched_by.contains(&"meaning")).count(), 2);

        // Hai job lập index cùng mục chạy cùng lúc: cuối cùng mỗi đoạn một dòng.
        let (x, y) =
            tokio::join!(first.index_entry(&tenant, b.id), second.index_entry(&tenant, b.id));
        if x.is_err() || y.is_err() {
            first.index_entry(&tenant, b.id).await.unwrap();
        }
        assert_eq!(rows_of(&database, b.id).await, 1, "one row per chunk (draft only)");

        // Đổi mô hình: job định kỳ xếp lập lại mọi mục.
        let other = Arc::new(FakeEmbedder::new(128).with_model("another-model"));
        let (third, _, database) = service_with(test, config(&[]), ai, other).await;
        let tenant = TenantContext::default_on(database.clone());
        assert_eq!(third.catch_up_embeddings(&database).await.unwrap(), 2);
        third.index_entry(&tenant, a.id).await.unwrap();
        assert_eq!(third.catch_up_embeddings(&database).await.unwrap(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn embedding_tat_chi_tim_theo_chu() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let dalat = content.create(&tenant, &ada, vi_post("Đà Lạt mùa hoa", "Hoa.")).await.unwrap();
        content.index_entry(&tenant, dalat.id).await.unwrap();
        assert_eq!(content.catch_up_embeddings(&database).await.unwrap(), 0);
        let hits = content.search(&tenant, Some(&ada), &query("da lat"), None).await.unwrap();
        assert_eq!(
            (hits.len(), hits[0].matched_by.clone(), hits[0].similarity),
            (1, vec!["text"], None)
        );
        assert_eq!(
            content
                .search(&tenant, None, &query("da lat"), Some(&Site::whole(&tenant)))
                .await
                .unwrap_err()
                .code(),
            "content.semantic_search_disabled"
        );
    })
    .await
    .unwrap();
}

/// PT-15: tìm công khai qua các chốt — hết chỗ chạy thì 429 `content.public_search_busy` ngay (không chờ, không làm chậm
/// request khác), một địa chỉ tìm quá số lượt mỗi phút thì 429 `content.public_search_rate_limited`; địa chỉ khác vẫn tìm
/// được. Tìm công khai tắt thì vẫn 404 như trước, không bị tính lượt.
#[tokio::test]
async fn tim_cong_khai_co_cho_va_gioi_han_theo_dia_chi() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service_with(
            test,
            config(&[
                ("AUTOXIDE_CONTENT_PUBLIC_SEMANTIC_SEARCH", "true"),
                ("AUTOXIDE_CONTENT_PUBLIC_SEARCH_CONCURRENCY", "2"),
                ("AUTOXIDE_CONTENT_PUBLIC_SEARCH_PER_ADDRESS_PER_MINUTE", "3"),
            ]),
            Arc::new(FakeTextModel::disabled()),
            Arc::new(FakeEmbedder::disabled()),
        )
        .await;
        let tenant = TenantContext::default_on(database.clone());
        let site = Site::whole(&tenant);
        let reader: Option<std::net::IpAddr> = "203.0.113.7".parse().ok();
        let other: Option<std::net::IpAddr> = "198.51.100.9".parse().ok();
        let q = query("đà lạt");
        let search = |ip| content.search_public(&tenant, None, &q, &site, ip);
        // Hai lượt đang chạy giữ hết chỗ: lượt mới bị từ chối ngay.
        let limits = &content.search_cache.public_limits;
        let held = (limits.admit(None, 100, 2).unwrap(), limits.admit(None, 100, 2).unwrap());
        assert_eq!(search(other).await.unwrap_err().code(), "content.public_search_busy");
        drop(held);
        // Theo địa chỉ: 3 lượt mỗi phút (lượt bị từ chối vì hết chỗ ở trên là của địa chỉ khác).
        for _ in 0..3 {
            search(reader).await.unwrap();
        }
        let limited = search(reader).await.unwrap_err();
        assert_eq!(limited.code(), "content.public_search_rate_limited");
        assert!(limited.retry_after().is_some());
        search(other).await.unwrap();
    })
    .await
    .unwrap();
}

/// PT-15: danh sách bài công khai có `q` và gợi ý tìm dùng chung chốt của tìm công khai — hết chỗ chạy thì 429
/// `content.public_search_busy` (gợi ý: nguồn `content` bị bỏ), một địa chỉ quá số lượt mỗi phút thì 429
/// `content.public_search_rate_limited`. Danh sách không có `q` không bị tính.
#[tokio::test]
async fn danh_sach_co_q_va_goi_y_qua_cung_chot() {
    TestDatabase::run(async |test| {
        use autoxide_kernel::suggest::SuggestSource as _;
        let (content, _, database) = service_with(
            test,
            config(&[
                ("AUTOXIDE_CONTENT_PUBLIC_SEARCH_CONCURRENCY", "2"),
                ("AUTOXIDE_CONTENT_PUBLIC_SEARCH_PER_ADDRESS_PER_MINUTE", "2"),
            ]),
            Arc::new(FakeTextModel::disabled()),
            Arc::new(FakeEmbedder::disabled()),
        )
        .await;
        let tenant = TenantContext::default_on(database.clone());
        let site = Site::whole(&tenant);
        let ada = editor();
        let entry = content.create(&tenant, &ada, post("Đà Lạt mùa hoa")).await.unwrap();
        content.publish(&tenant, &ada, entry.id, None, None, false).await.unwrap();
        let filter = crate::publishing::reading::PostFilter {
            locale: "en".into(),
            site: site.lineage_ids(),
            ..Default::default()
        };
        let searching = crate::publishing::reading::PostFilter {
            search: Some("da lat".into()),
            ..filter.clone()
        };
        let first = PageRequest::first(10, Order::Desc);
        let reader: Option<std::net::IpAddr> = "203.0.113.7".parse().ok();
        let suggest = crate::suggest::Entries(content.clone());

        // Hết chỗ chạy: danh sách có `q` và gợi ý bị từ chối ngay; danh sách không có `q` vẫn chạy.
        let limits = &content.search_cache.public_limits;
        let held = (limits.admit(None, 100, 2).unwrap(), limits.admit(None, 100, 2).unwrap());
        let busy = content.public_posts_for(&tenant, &searching, &first, reader).await;
        assert_eq!(busy.unwrap_err().code(), "content.public_search_busy");
        let busy = suggest.suggest(&tenant, &site, "en", "da lat", 5).await;
        assert_eq!(busy.unwrap_err().code(), "content.public_search_busy");
        let plain = content.public_posts_for(&tenant, &filter, &first, reader).await.unwrap();
        assert_eq!(plain.data.len(), 1);
        drop(held);

        // Có chỗ: tìm được; theo địa chỉ, 2 lượt mỗi phút (lượt bị từ chối vì hết chỗ không tính).
        for _ in 0..2 {
            let page = content.public_posts_for(&tenant, &searching, &first, reader).await.unwrap();
            assert_eq!(page.data.len(), 1);
        }
        let limited = content.public_posts_for(&tenant, &searching, &first, reader).await;
        assert_eq!(limited.unwrap_err().code(), "content.public_search_rate_limited");
        let found = suggest.suggest(&tenant, &site, "en", "da lat", 5).await.unwrap();
        assert_eq!(found.len(), 1, "suggestions do not count per address");
    })
    .await
    .unwrap();
}

/// Mô hình vector chậm khi tính vector của câu tìm (nhà cung cấp quá tải); vector của bài thì tính ngay.
struct SlowQueries(FakeEmbedder);

#[async_trait::async_trait]
impl autoxide_kernel::ai::Embedder for SlowQueries {
    fn enabled(&self) -> bool {
        self.0.enabled()
    }
    fn ready(&self) -> Result<(), autoxide_kernel::error::Error> {
        self.0.ready()
    }
    fn model_id(&self) -> &str {
        self.0.model_id()
    }
    fn dimensions(&self) -> usize {
        self.0.dimensions()
    }
    async fn embed(
        &self,
        tenant: &TenantContext,
        purpose: &'static str,
        texts: &[String],
        kind: autoxide_kernel::ai::EmbedKind,
    ) -> Result<Vec<Vec<f32>>, autoxide_kernel::error::Error> {
        if kind == autoxide_kernel::ai::EmbedKind::Query {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
        self.0.embed(tenant, purpose, texts, kind).await
    }
}

/// PT-15 (rà soát #548): mô hình vector chậm không làm tìm công khai hỏng — phần tìm theo ý nghĩa có trần riêng, quá thì
/// trả kết quả theo chữ trong thời gian trả lời, không phải 503.
#[tokio::test]
async fn mo_hinh_cham_thi_tim_cong_khai_lui_ve_theo_chu() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service_with(
            test,
            config(&[("AUTOXIDE_CONTENT_PUBLIC_SEMANTIC_SEARCH", "true")]),
            Arc::new(FakeTextModel::disabled()),
            Arc::new(SlowQueries(FakeEmbedder::new(64))),
        )
        .await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let dalat = content
            .create(&tenant, &ada, vi_post("Đà Lạt mùa hoa", "Hoa dã quỳ nở vàng đồi thông."))
            .await
            .unwrap();
        content.publish(&tenant, &ada, dalat.id, None, None, false).await.unwrap();
        content.index_entry(&tenant, dalat.id).await.unwrap();
        let started = std::time::Instant::now();
        let hits = content
            .search_public(&tenant, None, &query("đà lạt"), &Site::whole(&tenant), None)
            .await
            .unwrap();
        assert!(started.elapsed() < crate::ai::public_limits::TIMEOUT, "{:?}", started.elapsed());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].matched_by, ["text"]);
    })
    .await
    .unwrap();
}
