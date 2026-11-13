use super::*;
use crate::publishing::redirects::{RedirectInput, Resolved};

async fn resolve(
    content: &crate::service::ContentService,
    path: &str,
) -> Result<Resolved, autoxide_kernel::error::Error> {
    content.resolve("en", path).await
}

fn entry_path(resolved: Resolved) -> String {
    match resolved {
        Resolved::Entry(row) => row.path,
        other => panic!("expected an entry, got {other:?}"),
    }
}

/// Đăng bài: đường dẫn `/blog/<slug>`; sửa nháp không lộ ra ngoài tới khi đăng lại; gỡ thì
/// 404; lưu trữ thì 410.
#[tokio::test]
async fn dang_sua_go_luu_tru() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let ada = editor();
        let entry = content.create(&ada, post("Hello world")).await.unwrap();
        assert_eq!(
            resolve(&content, "/blog/hello-world").await.unwrap_err().code(),
            "content.not_found"
        );
        let published =
            content.publish(&ada, entry.id, Some(entry.updated_at), None, false).await.unwrap();
        assert_eq!(
            (published.status.as_str(), published.has_unpublished_changes),
            ("published", false)
        );
        assert_eq!(
            entry_path(resolve(&content, "/blog/hello-world/").await.unwrap()),
            "/blog/hello-world"
        );

        let changes = EntryFields { title: Some("Draft title".into()), ..Default::default() };
        let edited = content.update(&ada, entry.id, changes, false, None).await.unwrap();
        assert!(edited.has_unpublished_changes);
        match resolve(&content, "/blog/hello-world").await.unwrap() {
            Resolved::Entry(row) => {
                assert_eq!(row.title, "Hello world", "readers never see the draft")
            }
            other => panic!("{other:?}"),
        }
        // Phiên bản cũ: 412.
        let stale =
            content.publish(&ada, entry.id, Some(entry.updated_at), None, false).await.unwrap_err();
        assert_eq!(stale.kind(), ErrorKind::PreconditionFailed);

        let down = content.unpublish(&ada, entry.id, None).await.unwrap();
        assert_eq!(down.status, "draft");
        assert_eq!(
            resolve(&content, "/blog/hello-world").await.unwrap_err().code(),
            "content.not_found"
        );
        assert_eq!(
            content.unpublish(&ada, entry.id, None).await.unwrap_err().code(),
            "content.not_published"
        );
        content.publish(&ada, entry.id, None, None, false).await.unwrap();
        content.archive(&ada, entry.id, None).await.unwrap();
        assert!(matches!(resolve(&content, "/blog/hello-world").await.unwrap(), Resolved::Gone));
        // Đăng lại sau khi lưu trữ: đường dẫn sống lại.
        content.publish(&ada, entry.id, None, None, false).await.unwrap();
        assert_eq!(
            entry_path(resolve(&content, "/blog/hello-world").await.unwrap()),
            "/blog/hello-world"
        );
        let names = events(&database).await;
        for name in
            ["content.entry_published", "content.entry_unpublished", "content.entry_archived"]
        {
            assert!(names.contains(&name.to_owned()), "{name}");
        }
        let writer = user(&["read", "write"]);
        assert_eq!(
            content.publish(&writer, entry.id, None, None, false).await.unwrap_err().code(),
            "permission_denied"
        );
    })
    .await
    .unwrap();
}

/// Cây trang: con chỉ đăng khi cha đã công khai; đổi slug của cha rồi đăng lại → mọi trang
/// con có đường dẫn mới và chuyển hướng từ đường dẫn cũ; `resolve` đường dẫn cũ ra chuyển
/// hướng tới đúng mục.
#[tokio::test]
async fn cay_trang_doi_slug_cha_va_chuyen_huong() {
    TestDatabase::run(async |test| {
        let (content, _, _) = service(test, config(&[])).await;
        let ada = editor();
        let about = content.create(&ada, page("About", None)).await.unwrap();
        let team = content.create(&ada, page("Team", Some(about.id))).await.unwrap();
        let lead = content.create(&ada, page("Lead", Some(team.id))).await.unwrap();
        assert_eq!(
            content.publish(&ada, team.id, None, None, false).await.unwrap_err().code(),
            "content.parent_not_published"
        );
        for id in [about.id, team.id, lead.id] {
            content.publish(&ada, id, None, None, false).await.unwrap();
        }
        assert_eq!(
            entry_path(resolve(&content, "/about/team/lead").await.unwrap()),
            "/about/team/lead"
        );
        // Gỡ cha khi con còn công khai: không được.
        let blocked = content.unpublish(&ada, about.id, None).await.unwrap_err();
        assert_eq!(blocked.code(), "content.page_has_published_children");

        let rename = EntryFields { slug: Some("company".into()), ..Default::default() };
        content.update(&ada, about.id, rename, false, None).await.unwrap();
        content.publish(&ada, about.id, None, None, false).await.unwrap();
        assert_eq!(
            entry_path(resolve(&content, "/company/team/lead").await.unwrap()),
            "/company/team/lead"
        );
        for (old, new) in [
            ("/about", "/company"),
            ("/about/team", "/company/team"),
            ("/about/team/lead", "/company/team/lead"),
        ] {
            match resolve(&content, old).await.unwrap() {
                Resolved::Redirect { status, location } => {
                    assert_eq!((status, location.as_str()), (301, new), "{old}")
                }
                other => panic!("{old}: {other:?}"),
            }
        }
        // Đổi lại tên cũ: chuyển hướng tự động ở đường dẫn đó bị bỏ, trang sống lại ở đó.
        let back = EntryFields { slug: Some("about".into()), ..Default::default() };
        content.update(&ada, about.id, back, false, None).await.unwrap();
        content.publish(&ada, about.id, None, None, false).await.unwrap();
        assert_eq!(entry_path(resolve(&content, "/about/team").await.unwrap()), "/about/team");
        assert!(matches!(
            resolve(&content, "/company/team").await.unwrap(),
            Resolved::Redirect { .. }
        ));

        // Xoá trang còn trang con: không được; xoá lá đang công khai: 410.
        assert_eq!(
            content.delete(&ada, team.id, None).await.unwrap_err().code(),
            "content.page_has_children"
        );
        content.delete(&ada, lead.id, None).await.unwrap();
        assert!(matches!(resolve(&content, "/about/team/lead").await.unwrap(), Resolved::Gone));
        // Chuyển hướng cũ trỏ tới trang đã xoá thành "đã gỡ".
        assert!(matches!(resolve(&content, "/company/team/lead").await.unwrap(), Resolved::Gone));
    })
    .await
    .unwrap();
}

/// Chuyển trang: đổi đường dẫn của nó và cây con, chuyển hướng; không vào chính nó hay con
/// của nó; hai lệnh chuyển chéo nhau cùng lúc không tạo vòng — trên cả hai database.
#[tokio::test]
async fn chuyen_trang_va_chuyen_cheo_dong_thoi() {
    TestDatabase::run(async |test| {
        let (content, _, _) = service(test, config(&[])).await;
        let ada = editor();
        let a = content.create(&ada, page("A", None)).await.unwrap();
        let b = content.create(&ada, page("B", None)).await.unwrap();
        let child = content.create(&ada, page("Child", Some(a.id))).await.unwrap();
        for id in [a.id, b.id, child.id] {
            content.publish(&ada, id, None, None, false).await.unwrap();
        }
        let moved = content.move_page(&ada, a.id, Some(b.id), None).await.unwrap();
        assert_eq!(moved.parent_id, Some(b.id));
        assert_eq!(entry_path(resolve(&content, "/b/a/child").await.unwrap()), "/b/a/child");
        assert!(matches!(
            resolve(&content, "/a/child").await.unwrap(),
            Resolved::Redirect { status: 301, .. }
        ));
        let cycle = content.move_page(&ada, b.id, Some(child.id), None).await.unwrap_err();
        assert_eq!(cycle.code(), "content.tree_cycle");
        content.move_page(&ada, a.id, None, None).await.unwrap();

        for _ in 0..3 {
            let x =
                content.create(&ada, page(&format!("X {}", Uuid::now_v7()), None)).await.unwrap();
            let y =
                content.create(&ada, page(&format!("Y {}", Uuid::now_v7()), None)).await.unwrap();
            let into = |from: Uuid, to: Uuid| {
                let content = content.clone();
                tokio::spawn(
                    async move { content.move_page(&editor(), from, Some(to), None).await },
                )
            };
            let (first, second) = (into(x.id, y.id), into(y.id, x.id));
            let results = [first.await.unwrap(), second.await.unwrap()];
            assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1, "{results:?}");
            let refused = results.iter().find_map(|result| result.as_ref().err()).unwrap();
            assert_eq!(refused.code(), "content.tree_cycle");
        }
    })
    .await
    .unwrap();
}

/// Hai lệnh đăng cùng một mục, cùng phiên bản: đúng một thành công — trên cả hai database.
#[tokio::test]
async fn dang_dong_thoi_mot_ben_thang() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let entry = content.create(&editor(), post("Race")).await.unwrap();
        let attempts: Vec<_> = (0..4)
            .map(|_| {
                let content = content.clone();
                tokio::spawn(async move {
                    content.publish(&editor(), entry.id, Some(entry.updated_at), None, false).await
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
        assert_eq!(count(&database, "SELECT COUNT(*) FROM content_published").await, 1);
    })
    .await
    .unwrap();
}

/// Bài và trang chung không gian đường dẫn: trang `blog` có con trùng đường dẫn với bài bị
/// chặn. Ảnh phải công khai khi đăng.
#[tokio::test]
async fn duong_dan_trung_va_anh_rieng_tu() {
    TestDatabase::run(async |test| {
        let (content, media, _) = service(test, config(&[])).await;
        let ada = editor();
        let a_post = content.create(&ada, post("News")).await.unwrap();
        content.publish(&ada, a_post.id, None, None, false).await.unwrap();
        let blog = content.create(&ada, page("Blog", None)).await.unwrap();
        content.publish(&ada, blog.id, None, None, false).await.unwrap();
        let clash = content.create(&ada, page("News", Some(blog.id))).await.unwrap();
        assert_eq!(
            content.publish(&ada, clash.id, None, None, false).await.unwrap_err().code(),
            "content.path_taken"
        );

        let private = media.add(false);
        let mut with_image = post("Photo");
        with_image.fields.featured_media_id = Some(Some(private));
        let photo = content.create(&ada, with_image).await.unwrap();
        let error = content.publish(&ada, photo.id, None, None, false).await.unwrap_err();
        assert_eq!(
            (error.code(), error.fields()[0].field.as_str()),
            ("content.media_not_public", "featured_media_id")
        );
        media.0.lock().unwrap().insert(private, autoxide_media::api::MediaVisibility::Public);
        content.publish(&ada, photo.id, None, None, false).await.unwrap();
    })
    .await
    .unwrap();
}

/// Bản công khai kèm ảnh trong thân bài, theo thứ tự xuất hiện và không lặp (D-168); ảnh
/// thôi công khai sau khi đăng thì không còn trong danh sách.
#[tokio::test]
async fn anh_trong_than_bai_cua_ban_cong_khai() {
    TestDatabase::run(async |test| {
        let (content, media, _) = service(test, config(&[])).await;
        let ada = editor();
        let (first, second) = (media.add(true), media.add(true));
        let image = |id: Uuid| json!({"type": "image", "attrs": {"mediaId": id.to_string()}});
        let mut with_images = post("Gallery");
        with_images.fields.body = Some(json!({"type": "doc", "content": [
            image(second),
            {"type": "blockquote", "content": [image(first)]},
            image(second),
        ]}));
        let gallery = content.create(&ada, with_images).await.unwrap();
        content.publish(&ada, gallery.id, None, None, false).await.unwrap();
        let plain = content.create(&ada, post("Plain")).await.unwrap();
        content.publish(&ada, plain.id, None, None, false).await.unwrap();

        let body_media = async |path: &str| {
            let Resolved::Entry(row) = resolve(&content, path).await.unwrap() else { panic!() };
            let view = content.public_views(vec![*row]).await.unwrap().remove(0);
            view.body_media.into_iter().map(|item| item.id).collect::<Vec<_>>()
        };
        assert_eq!(body_media("/blog/gallery").await, [second, first]);
        assert!(body_media("/blog/plain").await.is_empty());

        media.0.lock().unwrap().insert(second, autoxide_media::api::MediaVisibility::Private);
        assert_eq!(body_media("/blog/gallery").await, [first]);
    })
    .await
    .unwrap();
}

/// Chuyển hướng làm tay: tới địa chỉ, tới mục; chặn vòng lặp và địa chỉ đang có nội dung.
#[tokio::test]
async fn chuyen_huong_lam_tay() {
    TestDatabase::run(async |test| {
        let (content, _, _) = service(test, config(&[])).await;
        let ada = editor();
        let target = content.create(&ada, page("Contact", None)).await.unwrap();
        content.publish(&ada, target.id, None, None, false).await.unwrap();
        let input =
            |from: &str, to_entry: Option<Uuid>, to_url: Option<&str>, status: i32| RedirectInput {
                locale: None,
                from_path: from.to_owned(),
                to_entry_id: to_entry,
                to_url: to_url.map(str::to_owned),
                status_code: status,
            };
        content.create_redirect(&ada, input("/lien-he", Some(target.id), None, 301)).await.unwrap();
        match resolve(&content, "/lien-he").await.unwrap() {
            Resolved::Redirect { status, location } => {
                assert_eq!((status, location.as_str()), (301, "/contact"))
            }
            other => panic!("{other:?}"),
        }
        content.create_redirect(&ada, input("/a", None, Some("/b"), 302)).await.unwrap();
        let looped =
            content.create_redirect(&ada, input("/b", None, Some("/a"), 302)).await.unwrap_err();
        assert_eq!(looped.code(), "content.redirect_loop");
        let self_loop =
            content.create_redirect(&ada, input("/contact-old", Some(target.id), None, 301)).await;
        assert!(self_loop.is_ok());
        let on_live = content
            .create_redirect(&ada, input("/contact", None, Some("https://example.com"), 301))
            .await;
        assert_eq!(on_live.unwrap_err().code(), "content.path_taken");
        let duplicate =
            content.create_redirect(&ada, input("/a", None, Some("/c"), 301)).await.unwrap_err();
        assert_eq!(duplicate.code(), "content.redirect_exists");
        let gone = content.create_redirect(&ada, input("/old", None, None, 410)).await.unwrap();
        assert!(matches!(resolve(&content, "/old").await.unwrap(), Resolved::Gone));
        let bad = content
            .create_redirect(&ada, input("old", None, Some("javascript:x"), 418))
            .await
            .unwrap_err();
        let fields: Vec<&str> = bad.fields().iter().map(|f| f.field.as_str()).collect();
        assert_eq!(fields, ["from_path", "status_code", "to_url"]);
        content.delete_redirect(&ada, gone.id).await.unwrap();
        assert_eq!(resolve(&content, "/old").await.unwrap_err().code(), "content.not_found");
        assert_eq!(
            content
                .create_redirect(&user(&["write"]), input("/z", None, None, 410))
                .await
                .unwrap_err()
                .code(),
            "permission_denied"
        );
    })
    .await
    .unwrap();
}
