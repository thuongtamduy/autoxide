//! Test lẻ của mục: tạo, slug, sửa nháp, trang có cha, sổ đối tượng.

use super::*;

/// Tạo bài: slug tiếng Việt từ tiêu đề, trùng thì thêm hậu tố; thân bài tính số chữ, thời
/// gian đọc; revision `initial`; event.
#[tokio::test]
async fn tao_bai_slug_va_than_bai() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let ada = editor();
        let mut first = post("Đường đến Đà Lạt");
        first.fields.body = Some(paragraph(&"chữ ".repeat(500)));
        let first = content.create(&ada, first).await.unwrap();
        assert_eq!(
            (first.slug.as_str(), first.locale.as_str(), first.status.as_str()),
            ("duong-den-da-lat", "en", "draft")
        );
        assert_eq!((first.word_count, first.reading_time_minutes), (500, 3));
        let second = content.create(&ada, post("Đường đến Đà Lạt!")).await.unwrap();
        assert_eq!(second.slug, "duong-den-da-lat-2");
        // Trang dùng không gian slug riêng với bài.
        let page = content.create(&ada, page("Đường đến Đà Lạt", None)).await.unwrap();
        assert_eq!(page.slug, "duong-den-da-lat");
        // Slug tự đặt mà trùng: 409.
        let mut taken = post("Khác");
        taken.fields.slug = Some("duong-den-da-lat".into());
        assert_eq!(content.create(&ada, taken).await.unwrap_err().code(), "content.slug_taken");
        assert_eq!(count(&database, "SELECT COUNT(*) FROM content_revisions").await, 3);
        assert_eq!(events(&database).await, ["content.entry_created"; 3]);
        // Tìm không dấu trong thân bài.
        let filter = EntryFilter { search: Some("chu".into()), ..Default::default() };
        let found = content.list(&ada, &filter, &PageRequest::first(10, Order::Asc)).await.unwrap();
        assert_eq!(found.data.len(), 1);
        let reader = user(&["read"]);
        assert_eq!(
            content.create(&reader, post("x")).await.unwrap_err().code(),
            "permission_denied"
        );
    })
    .await
    .unwrap();
}

/// Hai trang gốc cùng slug bị chặn — bẫy `NULL` trong unique index — kể cả khi nhiều request
/// cùng lúc tự sinh slug: mỗi mục một slug khác nhau, trên cả hai database.
#[tokio::test]
async fn slug_trung_dong_thoi_va_trang_goc() {
    TestDatabase::run(async |test| {
        let (content, _, _) = service(test, config(&[])).await;
        let ada = editor();
        let mut about = page("About", None);
        about.fields.slug = Some("about".into());
        content.create(&ada, about.clone()).await.unwrap();
        assert_eq!(content.create(&ada, about).await.unwrap_err().code(), "content.slug_taken");

        let attempts: Vec<_> = (0..6)
            .map(|_| {
                let content = content.clone();
                tokio::spawn(
                    async move { content.create(&editor(), page("Same title", None)).await },
                )
            })
            .collect();
        let mut slugs = Vec::new();
        for attempt in attempts {
            slugs.push(attempt.await.unwrap().unwrap().slug);
        }
        slugs.sort();
        slugs.dedup();
        assert_eq!(slugs.len(), 6, "{slugs:?}");
    })
    .await
    .unwrap();
}

/// Sửa nháp: `If-Match` cũ → 412; hai lệnh sửa cùng phiên bản → đúng một thắng; lưu revision
/// khi được yêu cầu; mọi lỗi báo một lần.
#[tokio::test]
async fn sua_nhap() {
    TestDatabase::run(async |test| {
        let (content, media, database) = service(test, config(&[])).await;
        let ada = editor();
        let entry = content.create(&ada, post("Draft")).await.unwrap();

        let changes = EntryFields { excerpt: Some("Short".into()), ..Default::default() };
        let changed = content
            .update(&ada, entry.id, changes.clone(), true, Some(entry.updated_at))
            .await
            .unwrap();
        assert_eq!(changed.excerpt, "Short");
        assert_eq!(count(&database, "SELECT COUNT(*) FROM content_revisions").await, 2);
        let stale = content
            .update(&ada, entry.id, changes, false, Some(entry.updated_at))
            .await
            .unwrap_err();
        assert_eq!(stale.kind(), ErrorKind::PreconditionFailed);

        let attempts: Vec<_> = (0..4)
            .map(|n| {
                let (content, ada) = (content.clone(), ada.clone());
                let version = changed.updated_at;
                tokio::spawn(async move {
                    let changes =
                        EntryFields { title: Some(format!("Title {n}")), ..Default::default() };
                    content.update(&ada, entry.id, changes, false, Some(version)).await
                })
            })
            .collect();
        let mut won = 0;
        for attempt in attempts {
            match attempt.await.unwrap() {
                Ok(_) => won += 1,
                Err(error) => assert_eq!(error.kind(), ErrorKind::PreconditionFailed),
            }
        }
        assert_eq!(won, 1);

        let private = media.add(false);
        let bad = EntryFields {
            title: Some(String::new()),
            slug: Some("Not A Slug".into()),
            canonical_url: Some("javascript:alert(1)".into()),
            featured_media_id: Some(Some(Uuid::now_v7())),
            og_media_id: Some(Some(private)),
            body: Some(json!({"type": "doc", "content": [{"type": "script"}]})),
            ..Default::default()
        };
        let error = content.update(&ada, entry.id, bad, false, None).await.unwrap_err();
        assert_eq!(error.code(), "validation_failed");
        let fields: Vec<&str> = error.fields().iter().map(|field| field.field.as_str()).collect();
        assert_eq!(
            fields,
            ["title", "slug", "canonical_url", "body.content[0].type", "featured_media_id"]
        );
        // Chỉ thân bài sai: mã riêng.
        let only_body = EntryFields {
            body: Some(json!({"type": "doc", "content": [{"type": "x"}]})),
            ..Default::default()
        };
        let error = content.update(&ada, entry.id, only_body, false, None).await.unwrap_err();
        assert_eq!(error.code(), "content.invalid_body");
    })
    .await
    .unwrap();
}

/// Cây trang khi tạo: cha phải là trang cùng ngôn ngữ; không quá 5 cấp; lọc theo cha.
#[tokio::test]
async fn trang_co_cha() {
    TestDatabase::run(async |test| {
        let (content, _, _) = service(test, config(&[])).await;
        let ada = editor();
        let mut parent = content.create(&ada, page("Level 1", None)).await.unwrap();
        for level in 2..=5 {
            parent = content
                .create(&ada, page(&format!("Level {level}"), Some(parent.id)))
                .await
                .unwrap();
        }
        let too_deep = content.create(&ada, page("Level 6", Some(parent.id))).await.unwrap_err();
        assert_eq!(too_deep.fields()[0].code, "tree_too_deep");
        let a_post = content.create(&ada, post("Post")).await.unwrap();
        let under_post = content.create(&ada, page("Child", Some(a_post.id))).await.unwrap_err();
        assert_eq!(under_post.fields()[0].field, "parent_id");
        let mut post_with_parent = post("Post 2");
        post_with_parent.parent_id = Some(parent.id);
        assert_eq!(
            content.create(&ada, post_with_parent).await.unwrap_err().fields()[0].code,
            "not_allowed"
        );
        let mut vi = page("Trang", Some(parent.id));
        vi.locale = Some("vi".into());
        assert_eq!(content.create(&ada, vi).await.unwrap_err().fields()[0].code, "invalid");
        let mut fr = post("Bonjour");
        fr.locale = Some("fr".into());
        assert_eq!(
            content.create(&ada, fr).await.unwrap_err().fields()[0].code,
            "locale_not_supported"
        );

        let roots = EntryFilter {
            parent: ParentFilter::Root,
            kind: Some(Kind::Page),
            ..Default::default()
        };
        let found = content.list(&ada, &roots, &PageRequest::first(10, Order::Asc)).await.unwrap();
        assert_eq!(found.data.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(), ["Level 1"]);
    })
    .await
    .unwrap();
}

/// Sổ đối tượng: bài nháp có nhưng không công khai và không lộ tiêu đề (D-150); bài đã đăng có
/// tiêu đề, tóm tắt, đường dẫn, ngôn ngữ của bản công khai; ID lạ thì không có.
#[tokio::test]
async fn so_doi_tuong_content_entry() {
    use autoxide_kernel::resource::ResourceResolver;
    TestDatabase::run(async |test| {
        let (content, _, _) = service(test, config(&[])).await;
        let resolver = crate::resource::EntryResolver(content.clone());
        let ada = editor();
        let mut new = post("Hello");
        new.fields.excerpt = Some("Short".into());
        let entry = content.create(&ada, new).await.unwrap();
        let draft = resolver.resolve(&entry.id.to_string()).await.unwrap().unwrap();
        assert_eq!(
            (draft.public, draft.title.as_str(), draft.summary.as_str(), draft.url.as_deref()),
            (false, "", "", None)
        );
        assert_eq!(draft.owner_user_ids, [ada.user_id()]);
        content.publish(&ada, entry.id, None, None, false).await.unwrap();
        let live = resolver.resolve(&entry.id.to_string()).await.unwrap().unwrap();
        assert_eq!(
            (live.public, live.summary.as_str(), live.url.as_deref(), live.locale.as_deref()),
            (true, "Short", Some("/blog/hello"), Some("en"))
        );
        assert!(resolver.resolve(&Uuid::now_v7().to_string()).await.unwrap().is_none());
        assert!(resolver.resolve("not-a-uuid").await.unwrap().is_none());
    })
    .await
    .unwrap();
}
