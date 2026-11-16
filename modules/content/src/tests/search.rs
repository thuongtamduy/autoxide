//! Tìm theo ý nghĩa (D-109): index theo đoạn, không tính lại đoạn không đổi, gộp với tìm theo
//! chữ, nhiều máy chủ cùng database, đổi mô hình, embedding tắt.

use std::sync::Arc;

use autoxide_testkit::ai::{FakeEmbedder, FakeTextModel};

use super::*;
use crate::ai::semantic::SearchQuery;
use crate::testing::service_with;
use autoxide_kernel::tenancy::TenantContext;

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
        let hits = content.search(&tenant, Some(&ada), &query("đà lạt"), false).await.unwrap();
        assert_eq!(hits[0].entry_id, dalat.id);
        assert_eq!(hits[0].matched_by, ["meaning", "text"]);
        assert!(hits[0].similarity.unwrap() > 0.0);
        assert_eq!(hits[0].status.as_deref(), Some("draft"));
        // Không chứa nguyên cụm chữ, nhưng gần về ý nghĩa.
        let hits =
            content.search(&tenant, Some(&ada), &query("hoa nở ở đà lạt"), false).await.unwrap();
        assert_eq!((hits[0].entry_id, hits[0].matched_by.clone()), (dalat.id, vec!["meaning"]));
        assert!(hits[0].snippet.contains("dã quỳ"));

        // Công khai: chỉ bản đã đăng, kèm đường dẫn.
        assert!(content.search(&tenant, None, &query("đà lạt"), true).await.unwrap().is_empty());
        content.publish(&tenant, &ada, dalat.id, None, None, false).await.unwrap();
        content.index_entry(&tenant, dalat.id).await.unwrap();
        let public = content.search(&tenant, None, &query("đà lạt"), true).await.unwrap();
        assert_eq!(public.len(), 1);
        assert_eq!(
            (public[0].path.as_deref(), public[0].status.clone()),
            (Some("/blog/da-lat-mua-hoa"), None)
        );

        // Kiểm đầu vào; cần quyền đọc.
        let bad = SearchQuery { q: " ".into(), limit: Some(51), ..Default::default() };
        let error = content.search(&tenant, Some(&ada), &bad, false).await.unwrap_err();
        assert_eq!(
            error.fields().iter().map(|f| f.field.as_str()).collect::<Vec<_>>(),
            ["q", "limit"]
        );
        assert_eq!(
            content
                .search(&tenant, Some(&user(&["write"])), &query("x"), false)
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
        assert_eq!(
            second.search(&tenant, Some(&ada), &query("phở"), false).await.unwrap().len(),
            1
        );
        // Máy chủ thứ nhất thêm mục: máy thứ hai (đã nạp vào bộ nhớ) thấy nhờ `version`.
        let b = first
            .create(&tenant, &ada, vi_post("Bún chả", "Phở và bún chả là món Hà Nội."))
            .await
            .unwrap();
        first.index_entry(&tenant, b.id).await.unwrap();
        let hits = second.search(&tenant, Some(&ada), &query("phở hà nội"), false).await.unwrap();
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
        let hits = content.search(&tenant, Some(&ada), &query("da lat"), false).await.unwrap();
        assert_eq!(
            (hits.len(), hits[0].matched_by.clone(), hits[0].similarity),
            (1, vec!["text"], None)
        );
        assert_eq!(
            content.search(&tenant, None, &query("da lat"), true).await.unwrap_err().code(),
            "content.semantic_search_disabled"
        );
    })
    .await
    .unwrap();
}
