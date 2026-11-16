use super::*;
use crate::publishing::reading::PostFilter;
use autoxide_kernel::tenancy::TenantContext;

pub(super) const SECRET: &str = "a-test-secret-that-is-long-enough-0123456789";

/// Danh sách bài công khai: chỉ bài đã đăng của đúng ngôn ngữ, mới nhất trước, lọc theo
/// danh mục và tìm không dấu; bản nháp và trang không bao giờ lộ. Sitemap đủ mọi mục kèm
/// bản dịch; feed giới hạn số bài.
#[tokio::test]
async fn bai_cong_khai_sitemap_feed() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let mut published = Vec::new();
        for (index, title) in ["Phở Hà Nội", "Bún chả", "Cà phê trứng"].iter().enumerate()
        {
            let entry = content.create(&tenant, &ada, post(title)).await.unwrap();
            let at = chrono::Utc::now() - chrono::Duration::days(10 - index as i64);
            content.publish(&tenant, &ada, entry.id, None, Some(at), false).await.unwrap();
            published.push(entry);
        }
        let draft = content.create(&tenant, &ada, post("Bản nháp bí mật")).await.unwrap();
        let about = content.create(&tenant, &ada, page("About", None)).await.unwrap();
        content.publish(&tenant, &ada, about.id, None, None, false).await.unwrap();
        let vi = content
            .create_translation(&tenant, &ada, published[0].id, "vi", fields("Phở"))
            .await
            .unwrap();
        content.publish(&tenant, &ada, vi.id, None, None, false).await.unwrap();
        let travel = content
            .create_term(
                &tenant,
                &ada,
                crate::taxonomy::terms::Taxonomy::Tag,
                crate::taxonomy::terms::TermInput {
                    locale: Some("en".into()),
                    name: Some("Noodles".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        content
            .set_entry_terms(&tenant, &ada, published[1].id, vec![travel.term.id])
            .await
            .unwrap();

        let first = PageRequest::first(2, Order::Desc);
        let filter = PostFilter { locale: "en".into(), ..Default::default() };
        let page1 = content.public_posts(&tenant, &filter, &first).await.unwrap();
        let titles: Vec<&str> = page1.data.iter().map(|row| row.title.as_str()).collect();
        assert_eq!(titles, ["Cà phê trứng", "Bún chả"]);
        let cursor = page1.pagination.next_cursor.clone().unwrap();
        let query = autoxide_kernel::pagination::PageQuery::new(
            Some("2".into()),
            Some(cursor),
            Some("desc".into()),
        );
        let next = PageRequest::try_from(query).unwrap();
        let page2 = content.public_posts(&tenant, &filter, &next).await.unwrap();
        assert_eq!(
            page2.data.iter().map(|row| row.title.as_str()).collect::<Vec<_>>(),
            ["Phở Hà Nội"]
        );
        assert!(page2.pagination.next_cursor.is_none());

        let tagged = PostFilter { term_id: Some(travel.term.id), ..filter.clone() };
        assert_eq!(
            content.public_posts(&tenant, &tagged, &first).await.unwrap().data[0].entry_id,
            published[1].id
        );
        let search = PostFilter { search: Some("ca phe".into()), ..filter.clone() };
        assert_eq!(
            content.public_posts(&tenant, &search, &first).await.unwrap().data[0].entry_id,
            published[2].id
        );
        let secret = PostFilter { search: Some("bí mật".into()), ..filter.clone() };
        assert!(
            content.public_posts(&tenant, &secret, &first).await.unwrap().data.is_empty(),
            "drafts never listed"
        );
        let vi_posts = PostFilter { locale: "vi".into(), ..Default::default() };
        assert_eq!(content.public_posts(&tenant, &vi_posts, &first).await.unwrap().data.len(), 1);

        let all =
            content.sitemap(&tenant, None, &PageRequest::first(50, Order::Asc)).await.unwrap().data;
        assert_eq!(all.len(), 5, "3 posts, 1 page, 1 translation");
        assert!(all.iter().all(|row| row.entry_id != draft.id));
        let translations = content.translations_of(&tenant, &all).await.unwrap();
        assert_eq!(
            translations[&published[0].id],
            [("vi".to_owned(), "/blog/pho-ha-noi".to_owned())]
        );
        assert_eq!(
            content
                .sitemap(&tenant, Some("vi"), &PageRequest::first(50, Order::Asc))
                .await
                .unwrap()
                .data
                .len(),
            1
        );

        let feed = content.feed(&tenant, "en", 2).await.unwrap();
        assert_eq!(
            feed.iter().map(|row| row.entry_id).collect::<Vec<_>>(),
            [published[2].id, published[1].id]
        );
    })
    .await
    .unwrap();
}

/// Link xem trước: cần `content.write`; đọc được bản nháp; sửa token, hết hạn, sai secret
/// hay chưa cấu hình secret đều bị từ chối.
#[tokio::test]
async fn xem_truoc() {
    TestDatabase::run(async |test| {
        let (off, _, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let entry = off.create(&tenant, &ada, post("Draft")).await.unwrap();
        assert_eq!(
            off.preview_token(&tenant, &ada, entry.id).await.unwrap_err().code(),
            "content.preview_disabled"
        );
        assert_eq!(off.preview(None, "x.y").await.unwrap_err().code(), "content.preview_disabled");

        let (content, _, database) =
            service(test, config(&[("AUTOXIDE_CONTENT_PREVIEW_SECRET", SECRET)])).await;

        let tenant = TenantContext::default_on(database.clone());
        let reader = user(&["read"]);
        assert_eq!(
            content.preview_token(&tenant, &reader, entry.id).await.unwrap_err().code(),
            "permission_denied"
        );
        assert_eq!(
            content.preview_token(&tenant, &ada, Uuid::now_v7()).await.unwrap_err().code(),
            "content.not_found"
        );
        let (token, expires) = content.preview_token(&tenant, &ada, entry.id).await.unwrap();
        assert!(expires > chrono::Utc::now() + chrono::Duration::seconds(590));
        content.update(&tenant, &ada, entry.id, fields("Draft v2"), false, None).await.unwrap();
        assert_eq!(
            content.preview(None, &token).await.unwrap().1.title,
            "Draft v2",
            "always the latest draft"
        );

        let mut tampered = token.clone();
        tampered.insert(3, if token.as_bytes()[3] == b'A' { 'B' } else { 'A' });
        tampered.remove(4);
        for bad in [tampered.as_str(), "nonsense", "abc.def", ""] {
            assert_eq!(
                content.preview(Some(&tenant), bad).await.unwrap_err().code(),
                "content.preview_token_invalid",
                "{bad}"
            );
        }
        let past = chrono::Utc::now() - chrono::Duration::seconds(1);
        let expired =
            crate::publishing::preview::sign(SECRET, tenant.id(), entry.id, past).unwrap();
        assert_eq!(
            content.preview(None, &expired).await.unwrap_err().code(),
            "content.preview_token_invalid"
        );
        let other_secret = crate::publishing::preview::sign(
            "another-secret-that-is-long-enough-012345",
            tenant.id(),
            entry.id,
            expires,
        )
        .unwrap();
        assert_eq!(
            content.preview(None, &other_secret).await.unwrap_err().code(),
            "content.preview_token_invalid"
        );
        // Token ký cho một khách khác (không có trong sổ): như token sai, kể cả khi request
        // không nêu khách.
        let foreign = crate::publishing::preview::sign(
            SECRET,
            autoxide_kernel::tenancy::TenantId::from_uuid(Uuid::now_v7()),
            entry.id,
            expires,
        )
        .unwrap();
        assert_eq!(
            content.preview(None, &foreign).await.unwrap_err().code(),
            "content.preview_token_invalid"
        );
        content.delete(&tenant, &ada, entry.id, None).await.unwrap();
        assert_eq!(
            content.preview(Some(&tenant), &token).await.unwrap_err().code(),
            "content.preview_token_invalid"
        );
    })
    .await
    .unwrap();
}
