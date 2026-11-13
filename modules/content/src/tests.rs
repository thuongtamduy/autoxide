//! Test mức service của module `content`. Gate chạy trên cả SQLite và Postgres.

use autoxide_kernel::database::Database;
use autoxide_kernel::error::ErrorKind;
use autoxide_kernel::pagination::{Order, PageRequest};
use autoxide_testkit::TestDatabase;
use sea_orm::{ConnectionTrait, Statement};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entries::{EntryFields, EntryFilter, Kind, NewEntry, ParentFilter};
use crate::testing::{config, editor, service, user};

pub(crate) fn fields(title: &str) -> EntryFields {
    EntryFields { title: Some(title.to_owned()), ..Default::default() }
}

pub(crate) fn post(title: &str) -> NewEntry {
    NewEntry { kind: Kind::Post, locale: None, parent_id: None, fields: fields(title) }
}

pub(crate) fn page(title: &str, parent: Option<Uuid>) -> NewEntry {
    NewEntry { kind: Kind::Page, locale: None, parent_id: parent, fields: fields(title) }
}

pub(crate) fn paragraph(text: &str) -> Value {
    json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": text}]}]})
}

pub(crate) async fn count(database: &Database, sql: &str) -> i64 {
    let backend = database.read().get_database_backend();
    let row =
        database.read().query_one_raw(Statement::from_string(backend, sql)).await.unwrap().unwrap();
    row.try_get_by_index::<i64>(0).unwrap()
}

pub(crate) async fn events(database: &Database) -> Vec<String> {
    let backend = database.read().get_database_backend();
    let sql = "SELECT name FROM content_events ORDER BY id";
    let rows = database.read().query_all_raw(Statement::from_string(backend, sql)).await;
    rows.unwrap().iter().map(|row| row.try_get("", "name").unwrap()).collect()
}

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

mod publishing {
    use super::*;
    use crate::redirects::{RedirectInput, Resolved};

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
            let stale = content
                .publish(&ada, entry.id, Some(entry.updated_at), None, false)
                .await
                .unwrap_err();
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
            assert!(matches!(
                resolve(&content, "/blog/hello-world").await.unwrap(),
                Resolved::Gone
            ));
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
            assert!(matches!(
                resolve(&content, "/company/team/lead").await.unwrap(),
                Resolved::Gone
            ));
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
                let x = content
                    .create(&ada, page(&format!("X {}", Uuid::now_v7()), None))
                    .await
                    .unwrap();
                let y = content
                    .create(&ada, page(&format!("Y {}", Uuid::now_v7()), None))
                    .await
                    .unwrap();
                let into = |from: Uuid, to: Uuid| {
                    let content = content.clone();
                    tokio::spawn(
                        async move { content.move_page(&editor(), from, Some(to), None).await },
                    )
                };
                let (first, second) = (into(x.id, y.id), into(y.id, x.id));
                let results = [first.await.unwrap(), second.await.unwrap()];
                assert_eq!(
                    results.iter().filter(|result| result.is_ok()).count(),
                    1,
                    "{results:?}"
                );
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
                        content
                            .publish(&editor(), entry.id, Some(entry.updated_at), None, false)
                            .await
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
            let input = |from: &str, to_entry: Option<Uuid>, to_url: Option<&str>, status: i32| {
                RedirectInput {
                    locale: None,
                    from_path: from.to_owned(),
                    to_entry_id: to_entry,
                    to_url: to_url.map(str::to_owned),
                    status_code: status,
                }
            };
            content
                .create_redirect(&ada, input("/lien-he", Some(target.id), None, 301))
                .await
                .unwrap();
            match resolve(&content, "/lien-he").await.unwrap() {
                Resolved::Redirect { status, location } => {
                    assert_eq!((status, location.as_str()), (301, "/contact"))
                }
                other => panic!("{other:?}"),
            }
            content.create_redirect(&ada, input("/a", None, Some("/b"), 302)).await.unwrap();
            let looped = content
                .create_redirect(&ada, input("/b", None, Some("/a"), 302))
                .await
                .unwrap_err();
            assert_eq!(looped.code(), "content.redirect_loop");
            let self_loop = content
                .create_redirect(&ada, input("/contact-old", Some(target.id), None, 301))
                .await;
            assert!(self_loop.is_ok());
            let on_live = content
                .create_redirect(&ada, input("/contact", None, Some("https://example.com"), 301))
                .await;
            assert_eq!(on_live.unwrap_err().code(), "content.path_taken");
            let duplicate = content
                .create_redirect(&ada, input("/a", None, Some("/c"), 301))
                .await
                .unwrap_err();
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
}

mod history {
    use chrono::{Duration, Utc};

    use super::*;
    use crate::redirects::Resolved;
    use crate::schedules::Action;

    fn title(resolved: Resolved) -> String {
        match resolved {
            Resolved::Entry(row) => row.title,
            other => panic!("{other:?}"),
        }
    }

    /// Revision: xem, khôi phục vào nháp (không tự đăng); dọn bớt giữ revision của bản công khai.
    #[tokio::test]
    async fn revision_xem_khoi_phuc_don_bot() {
        TestDatabase::run(async |test| {
            let (content, _, database) =
                service(test, config(&[("AUTOXIDE_CONTENT_MAX_REVISIONS", "10")])).await;
            let ada = editor();
            let entry = content.create(&ada, post("Version 0")).await.unwrap();
            content.publish(&ada, entry.id, None, None, false).await.unwrap();
            for n in 1..=14 {
                let changes =
                    EntryFields { title: Some(format!("Version {n}")), ..Default::default() };
                content.update(&ada, entry.id, changes, true, None).await.unwrap();
            }
            // initial + published + 14 lần lưu.
            assert_eq!(content.revision_count(entry.id).await, 16);
            let page = content
                .list_revisions(&ada, entry.id, &PageRequest::first(50, Order::Asc))
                .await
                .unwrap();
            let published = page.data.iter().find(|row| row.reason == "published").unwrap().id;
            let first_save = page.data.iter().find(|row| row.reason == "explicit_save").unwrap().id;

            let restored = content.restore(&ada, entry.id, first_save, None).await.unwrap();
            assert_eq!(
                (restored.title.as_str(), restored.has_unpublished_changes),
                ("Version 1", true)
            );
            assert_eq!(title(content.resolve("en", "/blog/version-0").await.unwrap()), "Version 0");
            assert!(events(&database).await.contains(&"content.entry_restored".to_owned()));

            let removed = content.prune_revisions().await.unwrap();
            assert_eq!(content.revision_count(entry.id).await, 11, "10 newest + the published one");
            assert_eq!(removed, 6);
            assert!(content.get_revision(&ada, entry.id, published).await.is_ok());
            let wrong = content.get_revision(&ada, Uuid::now_v7(), published).await.unwrap_err();
            assert_eq!(wrong.code(), "content.revision_not_found");
        })
        .await
        .unwrap();
    }

    /// Hẹn giờ đăng: đăng đúng bản nháp lúc đặt lịch; chạy lại không đăng hai lần; mỗi mục một
    /// lịch đăng đang chờ; huỷ; lịch gỡ.
    #[tokio::test]
    async fn hen_gio_dang_va_go() {
        TestDatabase::run(async |test| {
            let (content, _, database) = service(test, config(&[])).await;
            let ada = editor();
            let entry = content.create(&ada, post("Planned")).await.unwrap();
            let past = content
                .create_schedule(
                    &ada,
                    entry.id,
                    Action::Publish,
                    Utc::now() - Duration::minutes(1),
                    None,
                )
                .await;
            assert_eq!(past.unwrap_err().fields()[0].code, "schedule_in_past");
            let at = Utc::now() + Duration::hours(1);
            let schedule =
                content.create_schedule(&ada, entry.id, Action::Publish, at, None).await.unwrap();
            let conflict = content
                .create_schedule(&ada, entry.id, Action::Publish, at, None)
                .await
                .unwrap_err();
            assert_eq!(conflict.code(), "content.schedule_conflict");
            assert_eq!(
                count(
                    &database,
                    "SELECT COUNT(*) FROM content_jobs WHERE kind = 'content.run_schedule'"
                )
                .await,
                1
            );
            // Sửa nháp sau khi đặt lịch: lịch vẫn đăng bản lúc đặt.
            let later = EntryFields { title: Some("Edited later".into()), ..Default::default() };
            content.update(&ada, entry.id, later, false, None).await.unwrap();

            content.run_schedule(schedule.id, Utc::now()).await.unwrap();
            content.run_schedule(schedule.id, Utc::now()).await.unwrap();
            assert_eq!(title(content.resolve("en", "/blog/planned").await.unwrap()), "Planned");
            let after = content.get(&ada, entry.id).await.unwrap();
            assert_eq!((after.status.as_str(), after.has_unpublished_changes), ("published", true));
            let published = events(&database)
                .await
                .iter()
                .filter(|name| *name == "content.entry_published")
                .count();
            assert_eq!(published, 1, "running twice publishes once");
            let rows = content.list_schedules(&ada, entry.id).await.unwrap();
            assert_eq!(rows[0].status, "succeeded");

            let take_down =
                content.create_schedule(&ada, entry.id, Action::Unpublish, at, None).await.unwrap();
            let canceled = content.cancel_schedule(&ada, take_down.id).await.unwrap();
            assert_eq!(canceled.status, "canceled");
            assert_eq!(
                content.cancel_schedule(&ada, take_down.id).await.unwrap_err().code(),
                "content.schedule_not_pending"
            );
            let take_down =
                content.create_schedule(&ada, entry.id, Action::Unpublish, at, None).await.unwrap();
            content.run_schedule(take_down.id, Utc::now()).await.unwrap();
            assert_eq!(
                content.resolve("en", "/blog/planned").await.unwrap_err().code(),
                "content.not_found"
            );
        })
        .await
        .unwrap();
    }

    /// Bản công khai được đăng sau khi đặt lịch: lịch thất bại `content.schedule_outdated`, không
    /// ghi đè. Job chạy cùng lúc với đăng tay: không hỏng dữ liệu — trên cả hai database.
    #[tokio::test]
    async fn lich_loi_thoi_va_chay_dong_thoi() {
        TestDatabase::run(async |test| {
            let (content, _, _) = service(test, config(&[])).await;
            let ada = editor();
            let entry = content.create(&ada, post("Original")).await.unwrap();
            let at = Utc::now() + Duration::hours(1);
            let schedule =
                content.create_schedule(&ada, entry.id, Action::Publish, at, None).await.unwrap();
            let manual = EntryFields { title: Some("Manual".into()), ..Default::default() };
            content.update(&ada, entry.id, manual, false, None).await.unwrap();
            content.publish(&ada, entry.id, None, None, false).await.unwrap();
            content.run_schedule(schedule.id, Utc::now()).await.unwrap();
            let rows = content.list_schedules(&ada, entry.id).await.unwrap();
            assert_eq!(
                (rows[0].status.as_str(), rows[0].error_code.as_deref()),
                ("failed", Some("content.schedule_outdated"))
            );
            assert_eq!(title(content.resolve("en", "/blog/original").await.unwrap()), "Manual");

            for _ in 0..3 {
                let entry =
                    content.create(&ada, post(&format!("Race {}", Uuid::now_v7()))).await.unwrap();
                let schedule = content
                    .create_schedule(&ada, entry.id, Action::Publish, at, None)
                    .await
                    .unwrap();
                let entry = content.get(&ada, entry.id).await.unwrap();
                let (job, manual) = {
                    let (a, b) = (content.clone(), content.clone());
                    (
                        tokio::spawn(async move { a.run_schedule(schedule.id, Utc::now()).await }),
                        tokio::spawn(async move {
                            b.publish(&editor(), entry.id, Some(entry.updated_at), None, false)
                                .await
                        }),
                    )
                };
                job.await.unwrap().unwrap();
                let manual = manual.await.unwrap();
                let rows = content.list_schedules(&ada, entry.id).await.unwrap();
                let status = rows[0].status.clone();
                match manual {
                    // Đăng tay trước: lịch lỗi thời (hoặc đã chạy trước nhưng lệnh tay thấy 412).
                    Ok(_) => assert_eq!(status, "failed"),
                    Err(error) => {
                        assert_eq!(error.kind(), ErrorKind::PreconditionFailed);
                        assert_eq!(status, "succeeded");
                    }
                }
                assert_eq!(content.get(&ada, entry.id).await.unwrap().status, "published");
            }
        })
        .await
        .unwrap();
    }
}

mod library {
    use super::*;
    use crate::authors::AuthorInput;
    use crate::menus::ItemInput;
    use crate::redirects::Resolved;
    use crate::terms::{Taxonomy, TermInput};

    fn name(locale: &str, name: &str) -> TermInput {
        TermInput { locale: Some(locale.into()), name: Some(name.into()), ..Default::default() }
    }

    /// Bản dịch: một mục mới cùng nhóm, slug riêng; trang dịch đi theo bản dịch của cha; thiếu
    /// bản dịch thì 404, không lấy ngôn ngữ khác; bản công khai liệt kê bản dịch (hreflang).
    #[tokio::test]
    async fn ban_dich() {
        TestDatabase::run(async |test| {
            let (content, _, _) = service(test, config(&[])).await;
            let ada = editor();
            let about = content.create(&ada, page("About", None)).await.unwrap();
            let team = content.create(&ada, page("Team", Some(about.id))).await.unwrap();
            let early = content
                .create_translation(&ada, team.id, "vi", EntryFields::default())
                .await
                .unwrap_err();
            assert_eq!(early.code(), "content.parent_not_translated");
            let mut vi_fields = fields("Giới thiệu");
            vi_fields.slug = Some(String::new());
            let gioi_thieu =
                content.create_translation(&ada, about.id, "vi", vi_fields).await.unwrap();
            assert_eq!(
                (gioi_thieu.translation_group_id, gioi_thieu.slug.as_str()),
                (about.translation_group_id, "gioi-thieu")
            );
            let doi_ngu =
                content.create_translation(&ada, team.id, "vi", fields("Đội ngũ")).await.unwrap();
            assert_eq!(
                (doi_ngu.parent_id, doi_ngu.slug.as_str()),
                (Some(gioi_thieu.id), "team"),
                "slug copied unless given"
            );
            let again = content
                .create_translation(&ada, about.id, "vi", EntryFields::default())
                .await
                .unwrap_err();
            assert_eq!(again.code(), "content.translation_exists");
            let same = content
                .create_translation(&ada, about.id, "en", EntryFields::default())
                .await
                .unwrap_err();
            assert_eq!(same.fields()[0].field, "locale");

            content.publish(&ada, about.id, None, None, false).await.unwrap();
            assert_eq!(
                content.resolve("vi", "/gioi-thieu").await.unwrap_err().code(),
                "content.not_found"
            );
            assert_eq!(
                content.resolve("vi", "/about").await.unwrap_err().code(),
                "content.not_found",
                "no fallback"
            );
            content.publish(&ada, gioi_thieu.id, None, None, false).await.unwrap();
            let Resolved::Entry(row) = content.resolve("en", "/about").await.unwrap() else {
                panic!()
            };
            let view = content.public_views(vec![*row]).await.unwrap().remove(0);
            assert_eq!(view.translations, [("vi".to_owned(), "/gioi-thieu".to_owned())]);
        })
        .await
        .unwrap();
    }

    /// Danh mục (cây) và thẻ: tên theo ngôn ngữ, gắn vào mục (có hiệu lực ngay), lọc; tên hiển
    /// thị lấy ngôn ngữ mặc định khi thiếu; chặn vòng lặp; không xoá danh mục còn con.
    #[tokio::test]
    async fn danh_muc_va_the() {
        TestDatabase::run(async |test| {
            let (content, _, _) = service(test, config(&[])).await;
            let ada = editor();
            let travel =
                content.create_term(&ada, Taxonomy::Category, name("en", "Travel")).await.unwrap();
            let mut child = name("en", "Asia");
            child.parent_id = Some(Some(travel.term.id));
            let asia = content.create_term(&ada, Taxonomy::Category, child).await.unwrap();
            let tag = content.create_term(&ada, Taxonomy::Tag, name("en", "Food")).await.unwrap();
            assert_eq!(asia.texts[0].slug, "asia");
            let mut tag_parent = name("en", "Bad");
            tag_parent.parent_id = Some(Some(travel.term.id));
            assert_eq!(
                content.create_term(&ada, Taxonomy::Tag, tag_parent).await.unwrap_err().fields()[0]
                    .code,
                "not_allowed"
            );
            let loop_ = TermInput { parent_id: Some(Some(asia.term.id)), ..Default::default() };
            assert_eq!(
                content.update_term(&ada, travel.term.id, loop_, None).await.unwrap_err().fields()
                    [0]
                .code,
                "term_cycle"
            );
            assert_eq!(
                content
                    .create_term(&ada, Taxonomy::Tag, name("en", "Food"))
                    .await
                    .unwrap_err()
                    .code(),
                "content.slug_taken"
            );
            let with_vi = content
                .update_term(
                    &ada,
                    travel.term.id,
                    name("vi", "Du lịch"),
                    Some(travel.term.updated_at),
                )
                .await
                .unwrap();
            assert_eq!(with_vi.texts.len(), 2);

            let mut vi_post = post("Phở Hà Nội");
            vi_post.locale = Some("vi".into());
            let entry = content.create(&ada, vi_post).await.unwrap();
            content
                .set_entry_terms(&ada, entry.id, vec![travel.term.id, tag.term.id, travel.term.id])
                .await
                .unwrap();
            let filter = EntryFilter { term_id: Some(tag.term.id), ..Default::default() };
            assert_eq!(
                content
                    .list(&ada, &filter, &PageRequest::first(10, Order::Asc))
                    .await
                    .unwrap()
                    .data
                    .len(),
                1
            );
            content.publish(&ada, entry.id, None, None, false).await.unwrap();
            let Resolved::Entry(row) = content.resolve("vi", "/blog/pho-ha-noi").await.unwrap()
            else {
                panic!()
            };
            let view = content.public_views(vec![*row]).await.unwrap().remove(0);
            let shown: Vec<(String, String)> =
                view.terms.iter().map(|term| (term.name.clone(), term.locale.clone())).collect();
            assert_eq!(
                shown,
                [("Du lịch".to_owned(), "vi".to_owned()), ("Food".to_owned(), "en".to_owned())]
            );
            let (_, by_slug) =
                content.term_by_slug(Taxonomy::Category, "vi", "du-lich").await.unwrap();
            assert_eq!(by_slug.name, "Du lịch");

            assert_eq!(
                content.delete_term(&ada, travel.term.id).await.unwrap_err().code(),
                "content.term_has_children"
            );
            content.delete_term(&ada, tag.term.id).await.unwrap();
            assert_eq!(
                crate::terms::term_ids_of(content.database.read(), &[entry.id]).await.unwrap()
                    [&entry.id],
                [travel.term.id]
            );
        })
        .await
        .unwrap();
    }

    /// Tác giả: hồ sơ của `content`, nhiều tác giả theo thứ tự, lọc theo tác giả, xoá thì gỡ
    /// khỏi mục.
    #[tokio::test]
    async fn tac_gia() {
        TestDatabase::run(async |test| {
            let (content, media, _) = service(test, config(&[])).await;
            let ada = editor();
            let profile =
                |name: &str| AuthorInput { display_name: Some(name.into()), ..Default::default() };
            let a = content.create_author(&ada, profile("Ada Lovelace")).await.unwrap();
            let b = content.create_author(&ada, profile("Trần Đạt")).await.unwrap();
            assert_eq!((a.slug.as_str(), b.slug.as_str()), ("ada-lovelace", "tran-dat"));
            let missing =
                AuthorInput { avatar_media_id: Some(Some(Uuid::now_v7())), ..profile("X") };
            assert_eq!(
                content.create_author(&ada, missing).await.unwrap_err().code(),
                "content.media_not_found"
            );
            let avatar = media.add(true);
            let with_avatar =
                AuthorInput { avatar_media_id: Some(Some(avatar)), ..Default::default() };
            content.update_author(&ada, a.id, with_avatar, Some(a.updated_at)).await.unwrap();

            let entry = content.create(&ada, post("Co-written")).await.unwrap();
            content.set_entry_authors(&ada, entry.id, vec![b.id, a.id]).await.unwrap();
            let filter = EntryFilter { author_id: Some(a.id), ..Default::default() };
            assert_eq!(
                content
                    .list(&ada, &filter, &PageRequest::first(10, Order::Asc))
                    .await
                    .unwrap()
                    .data
                    .len(),
                1
            );
            let refs =
                crate::authors::authors_of(content.database.read(), &[entry.id]).await.unwrap();
            let names: Vec<&str> =
                refs[&entry.id].iter().map(|author| author.display_name.as_str()).collect();
            assert_eq!(names, ["Trần Đạt", "Ada Lovelace"]);
            assert_eq!(content.author_by_slug("tran-dat").await.unwrap().id, b.id);
            content.delete_author(&ada, b.id).await.unwrap();
            let left =
                crate::authors::author_ids_of(content.database.read(), &[entry.id]).await.unwrap();
            assert_eq!(left[&entry.id], [a.id]);
        })
        .await
        .unwrap();
    }

    /// Menu: thay cả cây, `If-Match`; mọi lỗi kèm vị trí; bản công khai tính đường dẫn, ẩn mục
    /// chưa công khai (cả nhánh con), dùng ngôn ngữ mặc định khi thiếu.
    #[tokio::test]
    async fn menu() {
        TestDatabase::run(async |test| {
            let (content, _, _) = service(test, config(&[])).await;
            let ada = editor();
            let about = content.create(&ada, page("About", None)).await.unwrap();
            let hidden = content.create(&ada, page("Hidden", None)).await.unwrap();
            content.publish(&ada, about.id, None, None, false).await.unwrap();
            let item = |label: &str,
                        entry: Option<Uuid>,
                        url: Option<&str>,
                        children: Vec<ItemInput>| ItemInput {
                label: label.into(),
                entry_id: entry,
                term_id: None,
                url: url.map(str::to_owned),
                children,
            };
            let tree = vec![
                item(
                    "About",
                    Some(about.id),
                    None,
                    vec![item("Docs", None, Some("https://example.com/docs"), vec![])],
                ),
                item(
                    "Hidden",
                    Some(hidden.id),
                    None,
                    vec![item("Under hidden", None, Some("/x"), vec![])],
                ),
            ];
            let (menu, nodes) =
                content.put_menu(&ada, "main", None, tree.clone(), None).await.unwrap();
            assert_eq!((menu.locale.as_str(), nodes.len(), nodes[0].children.len()), ("en", 2, 1));
            let stale = content
                .put_menu(
                    &ada,
                    "main",
                    None,
                    tree.clone(),
                    Some(menu.created_at - chrono::Duration::seconds(1)),
                )
                .await;
            assert_eq!(stale.unwrap_err().kind(), ErrorKind::PreconditionFailed);
            content.put_menu(&ada, "main", None, tree, Some(menu.updated_at)).await.unwrap();

            let bad =
                vec![item("", None, None, vec![item("x", None, Some("javascript:x"), vec![])])];
            let error = content.put_menu(&ada, "Main Menu", None, bad, None).await.unwrap_err();
            let fields: Vec<&str> =
                error.fields().iter().map(|field| field.field.as_str()).collect();
            assert_eq!(
                fields,
                ["key", "items[0].label", "items[0].entry_id", "items[0].children[0].url"]
            );

            let (shown, nodes) = content.public_menu("main", "vi").await.unwrap();
            assert_eq!(shown.locale, "en", "falls back to the default language");
            assert_eq!(nodes.len(), 1, "unpublished target hidden with its sub-items");
            assert_eq!(
                (nodes[0].href.as_str(), nodes[0].children[0].href.as_str()),
                ("/about", "https://example.com/docs")
            );
            assert_eq!(
                content.public_menu("footer", "en").await.unwrap_err().code(),
                "content.menu_not_found"
            );
        })
        .await
        .unwrap();
    }
}

mod public {
    use super::*;
    use crate::reading::PostFilter;

    pub(super) const SECRET: &str = "a-test-secret-that-is-long-enough-0123456789";

    /// Danh sách bài công khai: chỉ bài đã đăng của đúng ngôn ngữ, mới nhất trước, lọc theo
    /// danh mục và tìm không dấu; bản nháp và trang không bao giờ lộ. Sitemap đủ mọi mục kèm
    /// bản dịch; feed giới hạn số bài.
    #[tokio::test]
    async fn bai_cong_khai_sitemap_feed() {
        TestDatabase::run(async |test| {
            let (content, _, _) = service(test, config(&[])).await;
            let ada = editor();
            let mut published = Vec::new();
            for (index, title) in ["Phở Hà Nội", "Bún chả", "Cà phê trứng"].iter().enumerate()
            {
                let entry = content.create(&ada, post(title)).await.unwrap();
                let at = chrono::Utc::now() - chrono::Duration::days(10 - index as i64);
                content.publish(&ada, entry.id, None, Some(at), false).await.unwrap();
                published.push(entry);
            }
            let draft = content.create(&ada, post("Bản nháp bí mật")).await.unwrap();
            let about = content.create(&ada, page("About", None)).await.unwrap();
            content.publish(&ada, about.id, None, None, false).await.unwrap();
            let vi = content
                .create_translation(&ada, published[0].id, "vi", fields("Phở"))
                .await
                .unwrap();
            content.publish(&ada, vi.id, None, None, false).await.unwrap();
            let travel = content
                .create_term(
                    &ada,
                    crate::terms::Taxonomy::Tag,
                    crate::terms::TermInput {
                        locale: Some("en".into()),
                        name: Some("Noodles".into()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            content.set_entry_terms(&ada, published[1].id, vec![travel.term.id]).await.unwrap();

            let first = PageRequest::first(2, Order::Desc);
            let filter = PostFilter { locale: "en".into(), ..Default::default() };
            let page1 = content.public_posts(&filter, &first).await.unwrap();
            let titles: Vec<&str> = page1.data.iter().map(|row| row.title.as_str()).collect();
            assert_eq!(titles, ["Cà phê trứng", "Bún chả"]);
            let cursor = page1.pagination.next_cursor.clone().unwrap();
            let query = autoxide_kernel::pagination::PageQuery::new(
                Some("2".into()),
                Some(cursor),
                Some("desc".into()),
            );
            let next = PageRequest::try_from(query).unwrap();
            let page2 = content.public_posts(&filter, &next).await.unwrap();
            assert_eq!(
                page2.data.iter().map(|row| row.title.as_str()).collect::<Vec<_>>(),
                ["Phở Hà Nội"]
            );
            assert!(page2.pagination.next_cursor.is_none());

            let tagged = PostFilter { term_id: Some(travel.term.id), ..filter.clone() };
            assert_eq!(
                content.public_posts(&tagged, &first).await.unwrap().data[0].entry_id,
                published[1].id
            );
            let search = PostFilter { search: Some("ca phe".into()), ..filter.clone() };
            assert_eq!(
                content.public_posts(&search, &first).await.unwrap().data[0].entry_id,
                published[2].id
            );
            let secret = PostFilter { search: Some("bí mật".into()), ..filter.clone() };
            assert!(
                content.public_posts(&secret, &first).await.unwrap().data.is_empty(),
                "drafts never listed"
            );
            let vi_posts = PostFilter { locale: "vi".into(), ..Default::default() };
            assert_eq!(content.public_posts(&vi_posts, &first).await.unwrap().data.len(), 1);

            let all =
                content.sitemap(None, &PageRequest::first(50, Order::Asc)).await.unwrap().data;
            assert_eq!(all.len(), 5, "3 posts, 1 page, 1 translation");
            assert!(all.iter().all(|row| row.entry_id != draft.id));
            let translations = content.translations_of(&all).await.unwrap();
            assert_eq!(
                translations[&published[0].id],
                [("vi".to_owned(), "/blog/pho-ha-noi".to_owned())]
            );
            assert_eq!(
                content
                    .sitemap(Some("vi"), &PageRequest::first(50, Order::Asc))
                    .await
                    .unwrap()
                    .data
                    .len(),
                1
            );

            let feed = content.feed("en", 2).await.unwrap();
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
            let (off, _, _) = service(test, config(&[])).await;
            let ada = editor();
            let entry = off.create(&ada, post("Draft")).await.unwrap();
            assert_eq!(
                off.preview_token(&ada, entry.id).await.unwrap_err().code(),
                "content.preview_disabled"
            );
            assert_eq!(off.preview("x.y").await.unwrap_err().code(), "content.preview_disabled");

            let (content, _, _) =
                service(test, config(&[("AUTOXIDE_CONTENT_PREVIEW_SECRET", SECRET)])).await;
            let reader = user(&["read"]);
            assert_eq!(
                content.preview_token(&reader, entry.id).await.unwrap_err().code(),
                "permission_denied"
            );
            assert_eq!(
                content.preview_token(&ada, Uuid::now_v7()).await.unwrap_err().code(),
                "content.not_found"
            );
            let (token, expires) = content.preview_token(&ada, entry.id).await.unwrap();
            assert!(expires > chrono::Utc::now() + chrono::Duration::seconds(590));
            content.update(&ada, entry.id, fields("Draft v2"), false, None).await.unwrap();
            assert_eq!(
                content.preview(&token).await.unwrap().title,
                "Draft v2",
                "always the latest draft"
            );

            let mut tampered = token.clone();
            tampered.insert(3, if token.as_bytes()[3] == b'A' { 'B' } else { 'A' });
            tampered.remove(4);
            for bad in [tampered.as_str(), "nonsense", "abc.def", ""] {
                assert_eq!(
                    content.preview(bad).await.unwrap_err().code(),
                    "content.preview_token_invalid",
                    "{bad}"
                );
            }
            let past = chrono::Utc::now() - chrono::Duration::seconds(1);
            let expired = crate::preview::sign(SECRET, entry.id, past).unwrap();
            assert_eq!(
                content.preview(&expired).await.unwrap_err().code(),
                "content.preview_token_invalid"
            );
            let other_secret = crate::preview::sign(
                "another-secret-that-is-long-enough-012345",
                entry.id,
                expires,
            )
            .unwrap();
            assert_eq!(
                content.preview(&other_secret).await.unwrap_err().code(),
                "content.preview_token_invalid"
            );
            content.delete(&ada, entry.id, None).await.unwrap();
            assert_eq!(
                content.preview(&token).await.unwrap_err().code(),
                "content.preview_token_invalid"
            );
        })
        .await
        .unwrap();
    }
}

mod privacy {
    use autoxide_kernel::event::{Delivered, EventHandler};
    use autoxide_kernel::privacy::PersonalData;
    use chrono::{Duration, Utc};

    use super::*;
    use crate::authors::AuthorInput;
    use crate::schedules::Action;

    /// Xoá tài khoản: hồ sơ tác giả gắn với người đó bị xoá (cùng liên kết với bài), lịch họ
    /// đặt bị huỷ, `created_by` bỏ trống; bài vẫn giữ. Nhận lại event không đổi kết quả.
    /// Xuất dữ liệu: hồ sơ tác giả và các mục họ tạo.
    #[tokio::test]
    async fn xoa_tai_khoan_va_xuat_du_lieu() {
        TestDatabase::run(async |test| {
            let (content, _, database) = service(test, config(&[])).await;
            let ada = editor();
            let other = editor();
            let mut profile =
                AuthorInput { display_name: Some("Ada".into()), ..Default::default() };
            profile.user_id = Some(Some(ada.user_id()));
            let author = content.create_author(&ada, profile).await.unwrap();
            let kept = content
                .create_author(
                    &other,
                    AuthorInput { display_name: Some("Bob".into()), ..Default::default() },
                )
                .await
                .unwrap();
            let entry = content.create(&ada, post("Ada's post")).await.unwrap();
            content.set_entry_authors(&ada, entry.id, vec![author.id, kept.id]).await.unwrap();
            content
                .create_schedule(
                    &ada,
                    entry.id,
                    Action::Publish,
                    Utc::now() + Duration::days(1),
                    None,
                )
                .await
                .unwrap();

            let exported = content.export(ada.user_id()).await.unwrap();
            assert_eq!(exported["authors"][0]["display_name"], "Ada");
            assert_eq!(exported["entries"][0]["title"], "Ada's post");
            assert_eq!(
                content.export(Uuid::now_v7()).await.unwrap(),
                json!({"authors": [], "entries": []})
            );

            let deleted = Delivered::new(
                "auth",
                autoxide_kernel::privacy::USER_DELETED.to_owned(),
                json!({"before": {"id": ada.user_id()}}),
            );
            content.handle(&deleted).await.unwrap();
            content.handle(&deleted).await.unwrap();

            let left =
                crate::authors::author_ids_of(content.database.read(), &[entry.id]).await.unwrap();
            assert_eq!(left[&entry.id], [kept.id], "profile and its link removed, others kept");
            assert_eq!(
                content.get(&other, entry.id).await.unwrap().created_by,
                None,
                "entry kept, unlinked"
            );
            assert_eq!(
                count(
                    &database,
                    "SELECT COUNT(*) FROM content_schedules WHERE status = 'scheduled'"
                )
                .await,
                0
            );
            assert_eq!(
                count(
                    &database,
                    "SELECT COUNT(*) FROM content_revisions WHERE created_by IS NOT NULL"
                )
                .await,
                0
            );
            assert_eq!(
                content.export(ada.user_id()).await.unwrap(),
                json!({"authors": [], "entries": []})
            );
        })
        .await
        .unwrap();
    }
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

/// Các lỗi tìm thấy ở lượt rà soát độc lập sau khi gộp module (ngày 12/11/2026). Mỗi test giữ
/// một lỗi không quay lại.
mod ra_soat {
    use chrono::{Duration, Utc};

    use super::*;
    use crate::authors::AuthorInput;
    use crate::reading::PostFilter;
    use crate::redirects::{RedirectInput, Resolved};
    use crate::schedules::Action;
    use crate::terms::{Taxonomy, TermInput};

    fn redirect(locale: &str, from: &str, to: Uuid) -> RedirectInput {
        RedirectInput {
            locale: Some(locale.into()),
            from_path: from.into(),
            to_entry_id: Some(to),
            to_url: None,
            status_code: 301,
        }
    }

    /// Chuyển trang tính đường dẫn công khai từ slug đã đăng, không lộ slug của nháp.
    #[tokio::test]
    async fn chuyen_trang_dung_slug_da_dang() {
        TestDatabase::run(async |test| {
            let (content, _, _) = service(test, config(&[])).await;
            let ada = editor();
            let parent = content.create(&ada, page("Parent", None)).await.unwrap();
            let about = content.create(&ada, page("About", None)).await.unwrap();
            content.publish(&ada, parent.id, None, None, false).await.unwrap();
            content.publish(&ada, about.id, None, None, false).await.unwrap();
            let draft = EntryFields { slug: Some("secret-draft".into()), ..Default::default() };
            content.update(&ada, about.id, draft, false, None).await.unwrap();
            content.move_page(&ada, about.id, Some(parent.id), None).await.unwrap();
            assert!(content.resolve("en", "/parent/secret-draft").await.is_err());
            assert!(matches!(
                content.resolve("en", "/parent/about").await.unwrap(),
                Resolved::Entry(_)
            ));
            assert!(matches!(
                content.resolve("en", "/about").await.unwrap(),
                Resolved::Redirect { .. }
            ));
        })
        .await
        .unwrap();
    }

    /// Đường dẫn mới của trang con trùng một bài: `content.path_taken`, không phải lỗi 500, và
    /// không có gì đổi.
    #[tokio::test]
    async fn trang_con_trung_duong_dan() {
        TestDatabase::run(async |test| {
            let (content, _, _) = service(test, config(&[])).await;
            let ada = editor();
            let hello = content.create(&ada, post("Hello")).await.unwrap();
            content.publish(&ada, hello.id, None, None, false).await.unwrap();
            let news = content.create(&ada, page("News", None)).await.unwrap();
            let child = content.create(&ada, page("Hello", Some(news.id))).await.unwrap();
            content.publish(&ada, news.id, None, None, false).await.unwrap();
            content.publish(&ada, child.id, None, None, false).await.unwrap();
            let rename = EntryFields { slug: Some("blog".into()), ..Default::default() };
            content.update(&ada, news.id, rename, false, None).await.unwrap();
            let error = content.publish(&ada, news.id, None, None, false).await.unwrap_err();
            assert_eq!(error.code(), "content.path_taken", "{error:?}");
            assert!(matches!(
                content.resolve("en", "/news/hello").await.unwrap(),
                Resolved::Entry(_)
            ));

            // Chuyển trang cũng vậy: trang `blog` gốc có con `hello` → `/blog/hello` trùng bài.
            // (Trả slug nháp của `News` về như cũ để `blog` còn trống.)
            let back = EntryFields { slug: Some("news".into()), ..Default::default() };
            content.update(&ada, news.id, back, false, None).await.unwrap();
            let blog = content.create(&ada, page("Blog", None)).await.unwrap();
            assert_eq!(blog.slug, "blog");
            content.publish(&ada, blog.id, None, None, false).await.unwrap();
            let error = content.move_page(&ada, child.id, Some(blog.id), None).await.unwrap_err();
            assert_eq!(error.code(), "content.path_taken", "{error:?}");
        })
        .await
        .unwrap();
    }

    /// Bản dịch của trang không được vượt độ sâu tối đa (cha trong ngôn ngữ kia đã sâu hơn).
    #[tokio::test]
    async fn ban_dich_khong_vuot_do_sau() {
        TestDatabase::run(async |test| {
            let (content, _, _) = service(test, config(&[])).await;
            let ada = editor();
            let root = content.create(&ada, page("Root", None)).await.unwrap();
            let child = content.create(&ada, page("Child", Some(root.id))).await.unwrap();
            let root_vi =
                content.create_translation(&ada, root.id, "vi", fields("Gốc")).await.unwrap();
            // Cây tiếng Việt: bốn cấp, rồi đưa bản dịch của `Root` xuống cấp 5.
            let mut parent = None;
            for level in 1..=4 {
                let mut input = page(&format!("Cấp {level}"), parent);
                input.locale = Some("vi".into());
                parent = Some(content.create(&ada, input).await.unwrap().id);
            }
            content.move_page(&ada, root_vi.id, parent, None).await.unwrap();
            let error =
                content.create_translation(&ada, child.id, "vi", fields("Con")).await.unwrap_err();
            assert_eq!(error.fields()[0].code, "tree_too_deep", "{error:?}");
        })
        .await
        .unwrap();
    }

    /// Xoá trang cha và tạo trang con dưới nó cùng lúc: không bao giờ còn trang con mồ côi.
    #[tokio::test]
    async fn xoa_cha_va_tao_con_dong_thoi() {
        TestDatabase::run(async |test| {
            let (content, _, database) = service(test, config(&[])).await;
            let ada = editor();
            for _ in 0..5 {
                let parent = content.create(&ada, page("Parent", None)).await.unwrap();
                let (deleted, created) = tokio::join!(
                    content.delete(&ada, parent.id, None),
                    content.create(&ada, page("Child", Some(parent.id)))
                );
                assert!(deleted.is_ok() != created.is_ok(), "exactly one wins");
                if let Ok(child) = created {
                    content.delete(&ada, child.id, None).await.unwrap();
                    content.delete(&ada, parent.id, None).await.unwrap();
                }
            }
            let orphans = count(
                &database,
                "SELECT COUNT(*) FROM content_entries c WHERE c.parent_id IS NOT NULL \
                 AND NOT EXISTS (SELECT 1 FROM content_entries p WHERE p.id = c.parent_id)",
            )
            .await;
            assert_eq!(orphans, 0);
        })
        .await
        .unwrap();
    }

    /// Chuyển hướng làm tay chỉ trỏ tới mục cùng ngôn ngữ; đích đã lưu trữ thì đường dẫn cũ 410.
    #[tokio::test]
    async fn chuyen_huong_cung_ngon_ngu_va_luu_tru() {
        TestDatabase::run(async |test| {
            let (content, _, _) = service(test, config(&[])).await;
            let ada = editor();
            let english = content.create(&ada, post("English")).await.unwrap();
            content.publish(&ada, english.id, None, None, false).await.unwrap();
            let error =
                content.create_redirect(&ada, redirect("vi", "/cu", english.id)).await.unwrap_err();
            assert_eq!(error.fields()[0].code, "other_locale");

            content.create_redirect(&ada, redirect("en", "/old", english.id)).await.unwrap();
            let rename = EntryFields { slug: Some("renamed".into()), ..Default::default() };
            content.update(&ada, english.id, rename, false, None).await.unwrap();
            content.publish(&ada, english.id, None, None, false).await.unwrap();
            content.archive(&ada, english.id, None).await.unwrap();
            for old in ["/old", "/blog/english", "/blog/renamed"] {
                assert!(
                    matches!(content.resolve("en", old).await.unwrap(), Resolved::Gone),
                    "{old}"
                );
            }
        })
        .await
        .unwrap();
    }

    /// Đổi danh mục / tác giả của mục đang công khai cần quyền đăng; nháp thì quyền viết đủ.
    #[tokio::test]
    async fn gan_the_bai_cong_khai_can_quyen_dang() {
        TestDatabase::run(async |test| {
            let (content, _, _) = service(test, config(&[])).await;
            let ada = editor();
            let writer = user(&["read", "write"]);
            let tag = TermInput {
                locale: Some("en".into()),
                name: Some("Food".into()),
                ..Default::default()
            };
            let tag = content.create_term(&ada, Taxonomy::Tag, tag).await.unwrap();
            let author = AuthorInput { display_name: Some("Ada".into()), ..Default::default() };
            let author = content.create_author(&ada, author).await.unwrap();
            let entry = content.create(&ada, post("Live")).await.unwrap();
            content.set_entry_terms(&writer, entry.id, vec![tag.term.id]).await.unwrap();
            content.publish(&ada, entry.id, None, None, false).await.unwrap();
            let denied = content.set_entry_terms(&writer, entry.id, vec![]).await.unwrap_err();
            assert_eq!(denied.kind(), ErrorKind::Forbidden);
            let denied =
                content.set_entry_authors(&writer, entry.id, vec![author.id]).await.unwrap_err();
            assert_eq!(denied.kind(), ErrorKind::Forbidden);
            content.set_entry_authors(&ada, entry.id, vec![author.id]).await.unwrap();
        })
        .await
        .unwrap();
    }

    /// Hai lệnh đổi cha chéo nhau của danh mục chạy cùng lúc: không tạo vòng.
    #[tokio::test]
    async fn doi_cha_danh_muc_cheo_dong_thoi() {
        TestDatabase::run(async |test| {
            let (content, _, _) = service(test, config(&[])).await;
            let ada = editor();
            for round in 0..5 {
                let name = |n: &str| TermInput {
                    locale: Some("en".into()),
                    name: Some(format!("{n} {round}")),
                    ..Default::default()
                };
                let a = content.create_term(&ada, Taxonomy::Category, name("A")).await.unwrap();
                let b = content.create_term(&ada, Taxonomy::Category, name("B")).await.unwrap();
                let under = |parent: Uuid| TermInput {
                    parent_id: Some(Some(parent)),
                    ..Default::default()
                };
                let (first, second) = tokio::join!(
                    content.update_term(&ada, a.term.id, under(b.term.id), None),
                    content.update_term(&ada, b.term.id, under(a.term.id), None)
                );
                assert!(first.is_err() || second.is_err(), "round {round}: both moved");
            }
        })
        .await
        .unwrap();
    }

    /// Gỡ theo lịch ghi revision như gỡ bằng tay; xoá tài khoản huỷ lịch kèm event.
    #[tokio::test]
    async fn lich_go_ghi_revision_va_huy_lich_co_event() {
        TestDatabase::run(async |test| {
            let (content, _, database) = service(test, config(&[])).await;
            let ada = editor();
            let entry = content.create(&ada, post("Planned")).await.unwrap();
            content.publish(&ada, entry.id, None, None, false).await.unwrap();
            let at = Utc::now() + Duration::hours(1);
            let down =
                content.create_schedule(&ada, entry.id, Action::Unpublish, at, None).await.unwrap();
            let before = content.revision_count(entry.id).await;
            content.run_schedule(down.id, Utc::now()).await.unwrap();
            assert_eq!(content.revision_count(entry.id).await, before + 1);

            content.publish(&ada, entry.id, None, None, false).await.unwrap();
            content.create_schedule(&ada, entry.id, Action::Unpublish, at, None).await.unwrap();
            assert_eq!(content.cancel_schedules_of(ada.user_id()).await.unwrap(), 1);
            assert_eq!(content.cancel_schedules_of(ada.user_id()).await.unwrap(), 0);
            let canceled = events(&database)
                .await
                .iter()
                .filter(|name| *name == "content.schedule_canceled")
                .count();
            assert_eq!(canceled, 1);
        })
        .await
        .unwrap();
    }

    /// Bài công khai không lộ ID ảnh đại diện riêng tư của tác giả.
    #[tokio::test]
    async fn anh_tac_gia_rieng_tu_khong_lo() {
        TestDatabase::run(async |test| {
            let (content, media, _) = service(test, config(&[])).await;
            let ada = editor();
            let private = media.add(false);
            let input = AuthorInput {
                display_name: Some("Ada".into()),
                avatar_media_id: Some(Some(private)),
                ..Default::default()
            };
            let author = content.create_author(&ada, input).await.unwrap();
            let entry = content.create(&ada, post("By Ada")).await.unwrap();
            content.set_entry_authors(&ada, entry.id, vec![author.id]).await.unwrap();
            content.publish(&ada, entry.id, None, None, false).await.unwrap();
            let filter = PostFilter { locale: "en".into(), ..Default::default() };
            let page = content.public_posts(&filter, &PageRequest::first(10, Order::Desc)).await;
            let mut views = content.public_views(page.unwrap().data).await.unwrap();
            let view = views.remove(0);
            assert_eq!(view.authors[0].avatar_media_id, None);
        })
        .await
        .unwrap();
    }
}

/// Quyền cho AI agent (D-109): `write_own` / `publish_own`, quyền xoá riêng, xin duyệt,
/// `ai_generated` / `ai_reviewed`.
mod agent {
    use autoxide_kernel::access::Principal;
    use sea_orm::EntityTrait;

    use super::*;
    use crate::revisions::Snapshot;

    /// Agent dùng API key `key` của người dùng `user_id`.
    fn agent(user_id: Uuid, key: Uuid) -> Principal {
        Principal::new(
            user_id,
            ["read", "write_own", "publish_own"].iter().map(|p| format!("content.{p}")),
        )
        .via_api_key(key)
    }

    /// Chỉ sửa, đăng, gỡ mục do chính mình (cùng người, cùng key) tạo; không lưu trữ, không
    /// xoá; xoá cần `content.delete` (kể cả người có `content.publish`).
    #[tokio::test]
    async fn chi_lam_tren_muc_cua_minh() {
        TestDatabase::run(async |test| {
            let (content, _, _) = service(test, config(&[])).await;
            let owner = Uuid::now_v7();
            let bot = agent(owner, Uuid::now_v7());
            let other_key = agent(owner, Uuid::now_v7());
            let ada = editor();

            let mine = content.create(&bot, post("Mine")).await.unwrap();
            assert_eq!(mine.created_by_api_key_id, bot.api_key_id());
            let theirs = content.create(&ada, post("Theirs")).await.unwrap();

            let mine = content
                .update(&bot, mine.id, fields("Mine, edited"), false, Some(mine.updated_at))
                .await
                .unwrap();
            for (who, id) in [(&other_key, mine.id), (&bot, theirs.id)] {
                let error = content.update(who, id, fields("x"), false, None).await.unwrap_err();
                assert_eq!(
                    (error.kind(), error.code()),
                    (ErrorKind::Forbidden, "content.not_owner")
                );
                assert_eq!(
                    content.publish(who, id, None, None, false).await.unwrap_err().code(),
                    "content.not_owner"
                );
            }
            // Bản dịch: chỉ từ bản gốc của mình; bản dịch cũng là của mình.
            let vi = content.create_translation(&bot, mine.id, "vi", fields("Của tôi")).await;
            let vi = vi.unwrap();
            assert_eq!(vi.created_by_api_key_id, bot.api_key_id());
            assert_eq!(
                content
                    .create_translation(&bot, theirs.id, "vi", fields("x"))
                    .await
                    .unwrap_err()
                    .code(),
                "content.not_owner"
            );

            let mine = content.publish(&bot, mine.id, None, None, false).await.unwrap();
            assert_eq!(mine.status, "published");
            let mine = content.unpublish(&bot, mine.id, None).await.unwrap();
            for error in [
                content.archive(&bot, mine.id, None).await.unwrap_err(),
                content.delete(&bot, mine.id, None).await.unwrap_err(),
            ] {
                assert_eq!(error.code(), "permission_denied");
            }
            // Người có `content.publish` mà không có `content.delete` không xoá được.
            let publisher = user(&["read", "write", "publish"]);
            assert_eq!(
                content.delete(&publisher, theirs.id, None).await.unwrap_err().code(),
                "permission_denied"
            );
            content.delete(&ada, theirs.id, None).await.unwrap();
            // Không có quyền viết nào: 403 trước khi biết mục có tồn tại không.
            let reader = user(&["read"]);
            assert_eq!(
                content
                    .update(&reader, Uuid::now_v7(), fields("x"), false, None)
                    .await
                    .unwrap_err()
                    .code(),
                "permission_denied"
            );
        })
        .await
        .unwrap();
    }

    /// Xin duyệt rồi rút lại; người đăng xác nhận đã duyệt nội dung do AI tạo; API key không
    /// xác nhận được; `ai_generated` đi qua revision và khôi phục.
    #[tokio::test]
    async fn xin_duyet_va_xac_nhan_da_duyet() {
        TestDatabase::run(async |test| {
            let (content, _, database) = service(test, config(&[])).await;
            let bot = agent(Uuid::now_v7(), Uuid::now_v7());
            let ada = editor();
            let mut input = post("Written by a bot");
            input.fields.ai_generated = Some(true);
            let entry = content.create(&bot, input).await.unwrap();
            assert!(entry.ai_generated);

            let entry =
                content.set_review_request(&bot, entry.id, Some(entry.updated_at), false).await;
            let entry = entry.unwrap();
            assert!(entry.review_requested_at.is_some());
            assert_eq!(entry.review_requested_by, Some(bot.user_id()));
            let entry = content.set_review_request(&bot, entry.id, None, true).await.unwrap();
            assert_eq!(entry.review_requested_at, None);
            let entry = content.set_review_request(&bot, entry.id, None, false).await.unwrap();
            let names = events(&database).await;
            assert!(names.contains(&"content.review_requested".to_owned()));
            assert!(names.contains(&"content.review_withdrawn".to_owned()));

            assert_eq!(
                content.publish(&bot, entry.id, None, None, true).await.unwrap_err().code(),
                "content.ai_review_requires_human"
            );
            let entry = content.publish(&ada, entry.id, None, None, true).await.unwrap();
            assert_eq!(entry.review_requested_at, None);
            let published = crate::entity::published::Entity::find_by_id(entry.id)
                .one(database.read())
                .await
                .unwrap()
                .unwrap();
            assert!(published.ai_generated);
            assert_eq!(published.ai_reviewed_by, Some(ada.user_id()));
            assert!(published.ai_reviewed_at.is_some());
            // Đăng lại mà không xác nhận: không còn ghi là đã duyệt.
            content.publish(&ada, entry.id, None, None, false).await.unwrap();
            let published = crate::entity::published::Entity::find_by_id(entry.id)
                .one(database.read())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(published.ai_reviewed_by, None);

            // Revision giữ cờ; khôi phục đưa cờ về nháp.
            let revisions = content
                .list_revisions(&ada, entry.id, &PageRequest::first(50, Order::Asc))
                .await
                .unwrap();
            let first = &revisions.data[0];
            let snapshot: Snapshot = serde_json::from_str(&first.snapshot).unwrap();
            assert!(snapshot.ai_generated);
            let entry = content
                .update(
                    &ada,
                    entry.id,
                    EntryFields { ai_generated: Some(false), ..Default::default() },
                    false,
                    None,
                )
                .await
                .unwrap();
            assert!(!entry.ai_generated);
            let entry = content.restore(&ada, entry.id, first.id, None).await.unwrap();
            assert!(entry.ai_generated);
            // Revision cũ (trước D-109) không có trường: đọc được, coi là `false`.
            let old: Snapshot = serde_json::from_value(json!({
                "title": "t", "slug": "t", "excerpt": "", "body": paragraph("x"),
                "featured_media_id": null, "featured_media_alt": "", "meta_title": "",
                "meta_description": "", "og_media_id": null, "canonical_url": "", "noindex": false
            }))
            .unwrap();
            assert!(!old.ai_generated);
        })
        .await
        .unwrap();
    }
}

/// SEO (D-109): kiểm không dùng AI; gợi ý bằng AI chỉ trả gợi ý, chỉ giữ term có thật.
mod seo {
    use std::sync::Arc;

    use autoxide_kernel::access::Principal;
    use autoxide_testkit::ai::{FakeEmbedder, FakeTextModel};

    use super::*;
    use crate::terms::{Taxonomy, TermInput};
    use crate::testing::service_with;

    #[tokio::test]
    async fn kiem_seo_khong_dung_ai() {
        TestDatabase::run(async |test| {
            let (content, media, _) = service(test, config(&[])).await;
            let ada = editor();
            let image = media.add(true);
            let mut input = post("Hello");
            input.fields.featured_media_id = Some(Some(image));
            input.fields.meta_title = Some("x".repeat(70));
            input.fields.body = Some(json!({"type": "doc", "content": [
                {"type": "heading", "attrs": {"level": 2}, "content": [{"type": "text", "text": "A"}]},
                {"type": "heading", "attrs": {"level": 4}, "content": [{"type": "text", "text": "B"}]},
                {"type": "image", "attrs": {"mediaId": image.to_string(), "alt": "", "caption": ""}}
            ]}));
            let entry = content.create(&ada, input).await.unwrap();
            content.create(&ada, post("Hello")).await.unwrap();
            let findings = content.seo_check(&ada, entry.id).await.unwrap();
            let found: Vec<(&str, &str)> =
                findings.iter().map(|finding| (finding.code, finding.field.as_str())).collect();
            for expected in [
                ("seo.meta_title_too_long", "meta_title"),
                ("seo.meta_description_missing", "meta_description"),
                ("seo.excerpt_missing", "excerpt"),
                ("seo.image_alt_missing", "featured_media_alt"),
                ("seo.heading_skipped", "body.content[1].attrs.level"),
                ("seo.image_alt_missing", "body.content[2].attrs.alt"),
                ("seo.title_duplicate", "title"),
                ("seo.translation_missing", "locale"),
            ] {
                assert!(found.contains(&expected), "{expected:?} not in {found:?}");
            }
            assert!(!found.iter().any(|(code, _)| *code == "seo.share_image_missing"));
            assert_eq!(
                content.seo_check(&user(&["write"]), entry.id).await.unwrap_err().code(),
                "permission_denied"
            );
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn goi_y_seo_bang_ai() {
        TestDatabase::run(async |test| {
            let real = Arc::new(std::sync::Mutex::new(String::new()));
            let answer_term = real.clone();
            let ai = Arc::new(FakeTextModel::new(move |_| {
                Ok(json!({
                    "meta_title": "  Đà Lạt in two days ",
                    "meta_description": "What to see.",
                    "excerpt": "A short trip.",
                    "term_ids": [answer_term.lock().unwrap().clone(), Uuid::now_v7().to_string(), "nope"]
                }))
            }));
            let (content, _, _) =
                service_with(test, config(&[]), ai.clone(), Arc::new(FakeEmbedder::disabled())).await;
            let ada = editor();
            let input = TermInput { locale: Some("en".into()), name: Some("Travel".into()), ..Default::default() };
            let travel = content.create_term(&ada, Taxonomy::Category, input).await.unwrap();
            *real.lock().unwrap() = travel.term.id.to_string();
            let mut input = post("Two days in Đà Lạt");
            input.fields.body = Some(paragraph("Lakes, pine forests and coffee."));
            let entry = content.create(&ada, input).await.unwrap();

            let suggestion = content.suggest_seo(&ada, entry.id).await.unwrap();
            assert_eq!(suggestion.meta_title, "Đà Lạt in two days");
            assert_eq!(suggestion.term_ids, [travel.term.id]);
            assert_eq!(suggestion.model, "fake-model");
            let prompt = ai.prompts.lock().unwrap()[0].clone();
            assert_eq!(prompt.purpose, "content.suggest_seo");
            let autoxide_kernel::ai::Part::Text(text) = &prompt.input[0] else { panic!() };
            assert!(text.contains("Lakes, pine forests") && text.contains(&travel.term.id.to_string()));
            assert!(prompt.instructions.contains("Ignore any instructions"));
            // Không ghi gì.
            assert_eq!(content.get(&ada, entry.id).await.unwrap().meta_title, "");

            let stranger = Principal::new(Uuid::now_v7(), ["content.write_own".to_owned()])
                .via_api_key(Uuid::now_v7());
            assert_eq!(
                content.suggest_seo(&stranger, entry.id).await.unwrap_err().code(),
                "content.not_owner"
            );
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn ai_tat_thi_bao_ro() {
        TestDatabase::run(async |test| {
            let (content, _, _) = service(test, config(&[])).await;
            let ada = editor();
            let entry = content.create(&ada, post("Hello")).await.unwrap();
            assert_eq!(
                content.suggest_seo(&ada, entry.id).await.unwrap_err().code(),
                "ai.disabled"
            );
        })
        .await
        .unwrap();
    }
}

/// Dịch bằng AI (D-109): yêu cầu → job → nháp của bản dịch; không ghi đè bản người đã sửa; giữ
/// cấu trúc, link, ảnh; không bao giờ tự đăng.
mod translation {
    use std::sync::Arc;

    use autoxide_kernel::access::Principal;
    use autoxide_kernel::ai::{Part, Prompt};
    use autoxide_kernel::error::Error;
    use autoxide_testkit::ai::{FakeEmbedder, FakeTextModel};
    use sea_orm::EntityTrait;

    use super::*;
    use crate::service::ContentService;
    use crate::testing::service_with;

    /// "Dịch" bằng cách thêm `[vi] ` trước mỗi mẩu (giữ nguyên thẻ).
    fn translator() -> FakeTextModel {
        FakeTextModel::new(|prompt: &Prompt| {
            let Part::Text(input) = &prompt.input[0] else { panic!() };
            let items: Vec<Value> = serde_json::from_str(input).unwrap();
            let segments: Vec<Value> = items
                .iter()
                .map(|item| json!({"id": item["id"], "text": format!("[vi] {}", item["text"].as_str().unwrap())}))
                .collect();
            Ok(json!({"segments": segments}))
        })
    }

    fn rich_body(image: Uuid) -> Value {
        json!({"type": "doc", "content": [
            {"type": "paragraph", "content": [
                {"type": "text", "text": "See the "},
                {"type": "text", "text": "map", "marks": [{"type": "link", "attrs": {"href": "https://example.com/map"}}]}
            ]},
            {"type": "codeBlock", "content": [{"type": "text", "text": "let x = 1;"}]},
            {"type": "image", "attrs": {"mediaId": image.to_string(), "alt": "A lake", "caption": ""}}
        ]})
    }

    async fn setup(
        test: &TestDatabase,
        ai: FakeTextModel,
        pairs: &[(&str, &str)],
    ) -> (Arc<ContentService>, Arc<crate::testing::FakeMedia>, Database, Arc<FakeTextModel>) {
        let ai = Arc::new(ai);
        let (content, media, database) =
            service_with(test, config(pairs), ai.clone(), Arc::new(FakeEmbedder::disabled())).await;
        (content, media, database, ai)
    }

    async fn request(database: &Database, id: Uuid) -> crate::translate::TranslationRequest {
        crate::entity::translation_request::Entity::find_by_id(id)
            .one(database.read())
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn dich_moi_giu_cau_truc_va_khong_tu_dang() {
        TestDatabase::run(async |test| {
            let (content, media, database, ai) = setup(test, translator(), &[]).await;
            let ada = editor();
            let image = media.add(true);
            let mut input = post("Two days in Da Lat");
            input.fields.excerpt = Some("A short trip.".into());
            input.fields.body = Some(rich_body(image));
            let source = content.create(&ada, input).await.unwrap();
            content.publish(&ada, source.id, None, None, false).await.unwrap();

            let queued = content.request_translation(&ada, source.id, "vi").await.unwrap();
            assert_eq!((queued.status.as_str(), queued.target_entry_id), ("queued", None));
            content.run_translation(queued.id, 1).await.unwrap();
            let done = request(&database, queued.id).await;
            assert_eq!(done.status, "succeeded");
            assert_eq!(done.active_key, None);
            let vi = content.get(&ada, done.result_entry_id.unwrap()).await.unwrap();
            assert_eq!((vi.locale.as_str(), vi.status.as_str()), ("vi", "draft"));
            assert_eq!(
                (vi.title.as_str(), vi.excerpt.as_str()),
                ("[vi] Two days in Da Lat", "[vi] A short trip.")
            );
            assert_eq!(vi.slug, "vi-two-days-in-da-lat");
            assert!(vi.ai_generated);
            assert_eq!(vi.translated_from_revision_id, Some(done.source_revision_id));
            assert_eq!(vi.translation_group_id, source.translation_group_id);
            let body: Value = serde_json::from_str(&vi.body).unwrap();
            assert_eq!(body["content"][0]["content"][0]["text"], "[vi] See the ");
            assert_eq!(
                body["content"][0]["content"][1]["marks"][0]["attrs"]["href"],
                "https://example.com/map"
            );
            assert_eq!(body["content"][1]["content"][0]["text"], "let x = 1;");
            assert_eq!(body["content"][2]["attrs"]["alt"], "[vi] A lake");
            assert_eq!(body["content"][2]["attrs"]["mediaId"], image.to_string());
            let prompt = ai.prompts.lock().unwrap()[0].clone();
            assert!(prompt.instructions.contains("`en`") && prompt.instructions.contains("`vi`"));
            let Part::Text(sent) = &prompt.input[0] else { panic!() };
            assert!(!sent.contains("example.com"), "the model never sees links");
            let names = events(&database).await;
            for name in ["content.translation_requested", "content.translation_succeeded"] {
                assert!(names.contains(&name.to_owned()), "{name}");
            }
            // Chạy lại job (thử lại): không làm gì thêm.
            content.run_translation(queued.id, 2).await.unwrap();
            assert_eq!(ai.calls(), 1);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn cap_nhat_ban_dich_co_san_va_bao_da_cu() {
        TestDatabase::run(async |test| {
            let (content, _, database, _) = setup(test, translator(), &[]).await;
            let ada = editor();
            let source = content.create(&ada, post("Hello")).await.unwrap();
            let vi = content
                .create_translation(&ada, source.id, "vi", fields("Xin chào"))
                .await
                .unwrap();
            let queued = content.request_translation(&ada, source.id, "vi").await.unwrap();
            assert_eq!(queued.target_entry_id, Some(vi.id));
            content.run_translation(queued.id, 1).await.unwrap();
            let updated = content.get(&ada, vi.id).await.unwrap();
            assert_eq!(
                (updated.title.as_str(), updated.slug.as_str()),
                ("[vi] Hello", vi.slug.as_str())
            );
            let view = |entry| {
                let content = content.clone();
                async move {
                    let described = crate::http::describe(&content, vec![entry]).await.unwrap();
                    serde_json::to_value(&described[0]).unwrap()
                }
            };
            assert_eq!(view(updated.clone()).await["translation_outdated"], false);
            // Đăng bản gốc đổi `updated_at` nhưng không đổi chữ: chưa cũ.
            content.publish(&ada, source.id, None, None, false).await.unwrap();
            assert_eq!(view(updated.clone()).await["translation_outdated"], false);
            content.update(&ada, source.id, fields("Hello again"), false, None).await.unwrap();
            assert_eq!(view(updated).await["translation_outdated"], true);

            // Có người sửa bản dịch trong lúc chờ: không ghi đè.
            let queued = content.request_translation(&ada, source.id, "vi").await.unwrap();
            content.update(&ada, vi.id, fields("Sửa tay"), false, None).await.unwrap();
            content.run_translation(queued.id, 1).await.unwrap();
            let failed = request(&database, queued.id).await;
            assert_eq!(
                (failed.status.as_str(), failed.error_code.as_deref()),
                ("failed", Some("content.translation_conflict"))
            );
            assert_eq!(content.get(&ada, vi.id).await.unwrap().title, "Sửa tay");
            assert!(events(&database).await.contains(&"content.translation_failed".to_owned()));

            // Bản gốc không đổi (cùng revision với lần dịch trước), người sửa bản dịch sau khi
            // yêu cầu: vẫn là xung đột, không báo nhầm "thành công".
            let queued = content.request_translation(&ada, source.id, "vi").await.unwrap();
            content.run_translation(queued.id, 1).await.unwrap();
            assert_eq!(request(&database, queued.id).await.status, "succeeded");
            let again = content.request_translation(&ada, source.id, "vi").await.unwrap();
            assert_eq!(again.source_revision_id, queued.source_revision_id);
            content.update(&ada, vi.id, fields("Sửa tay lần hai"), false, None).await.unwrap();
            content.run_translation(again.id, 1).await.unwrap();
            assert_eq!(
                request(&database, again.id).await.error_code.as_deref(),
                Some("content.translation_conflict")
            );
            assert_eq!(content.get(&ada, vi.id).await.unwrap().title, "Sửa tay lần hai");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn sai_cau_truc_loi_tam_thoi_va_chia_lo() {
        TestDatabase::run(async |test| {
            // Bỏ mất thẻ: bị từ chối hai lần → thất bại, không ghi gì.
            let broken = FakeTextModel::new(|prompt: &Prompt| {
                let Part::Text(input) = &prompt.input[0] else { panic!() };
                let items: Vec<Value> = serde_json::from_str(input).unwrap();
                let segments: Vec<Value> =
                    items.iter().map(|item| json!({"id": item["id"], "text": "no tags"})).collect();
                Ok(json!({"segments": segments}))
            });
            let (content, media, database, ai) = setup(test, broken, &[]).await;
            let ada = editor();
            let mut input = post("Hello");
            input.fields.body = Some(rich_body(media.add(true)));
            let source = content.create(&ada, input).await.unwrap();
            let queued = content.request_translation(&ada, source.id, "vi").await.unwrap();
            content.run_translation(queued.id, 1).await.unwrap();
            let failed = request(&database, queued.id).await;
            assert_eq!(failed.error_code.as_deref(), Some("content.translation_invalid"));
            assert_eq!(ai.calls(), 2);

            // Nhà cung cấp bận: thử lại (lỗi trả về cho job queue), lần cuối thì thất bại.
            // Mã của module `ai` (không phải của `content`): để trong biến cho `cargo xtask check`.
            let code = "ai.provider_unavailable";
            let busy = FakeTextModel::new(move |_| Err(Error::unavailable(code, "busy")));
            let (content, _, database, _) = setup(test, busy, &[]).await;
            let source = content.create(&ada, post("Hello")).await.unwrap();
            let queued = content.request_translation(&ada, source.id, "vi").await.unwrap();
            assert_eq!(content.run_translation(queued.id, 1).await.unwrap_err().code(), "ai.provider_unavailable");
            assert_eq!(request(&database, queued.id).await.status, "running");
            content.run_translation(queued.id, 8).await.unwrap();
            let failed = request(&database, queued.id).await;
            assert_eq!((failed.status.as_str(), failed.error_code.as_deref()), ("failed", Some("ai.provider_unavailable")));

            // Bài dài: chia nhiều lô.
            let (content, _, _, ai) =
                setup(test, translator(), &[("AUTOXIDE_CONTENT_TRANSLATE_BATCH_TOKENS", "500")]).await;
            let paragraphs: Vec<Value> = (0..6)
                .map(|i| json!({"type": "paragraph", "content": [{"type": "text", "text": format!("{i} {}", "word ".repeat(80))}]}))
                .collect();
            let mut input = post("Long");
            input.fields.body = Some(json!({"type": "doc", "content": paragraphs}));
            let source = content.create(&ada, input).await.unwrap();
            let queued = content.request_translation(&ada, source.id, "vi").await.unwrap();
            content.run_translation(queued.id, 1).await.unwrap();
            assert!(ai.calls() > 1, "{} calls", ai.calls());
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn dong_thoi_quyen_va_ai_tat() {
        TestDatabase::run(async |test| {
            let (content, _, _, _) = setup(test, translator(), &[]).await;
            let ada = editor();
            let source = content.create(&ada, post("Hello")).await.unwrap();
            // Hai yêu cầu cùng lúc cho cùng một ngôn ngữ: đúng một được nhận.
            let attempts: Vec<_> = (0..4)
                .map(|_| {
                    let content = content.clone();
                    tokio::spawn(async move {
                        content.request_translation(&editor(), source.id, "vi").await
                    })
                })
                .collect();
            let mut accepted = Vec::new();
            for attempt in attempts {
                match attempt.await.unwrap() {
                    Ok(request) => accepted.push(request),
                    Err(error) => assert_eq!(error.code(), "content.translation_in_progress"),
                }
            }
            assert_eq!(accepted.len(), 1);
            content.run_translation(accepted[0].id, 1).await.unwrap();
            // Xong rồi thì xin dịch lại được.
            content.request_translation(&ada, source.id, "vi").await.unwrap();
            let bad = content.request_translation(&ada, source.id, "en").await.unwrap_err();
            assert_eq!(bad.fields()[0].field, "locale");

            let stranger = Principal::new(Uuid::now_v7(), ["content.write_own".to_owned()])
                .via_api_key(Uuid::now_v7());
            assert_eq!(
                content.request_translation(&stranger, source.id, "vi").await.unwrap_err().code(),
                "content.not_owner"
            );

            let (off, _, _) = service(test, config(&[])).await;
            let source = off.create(&ada, post("Hello")).await.unwrap();
            assert_eq!(
                off.request_translation(&ada, source.id, "vi").await.unwrap_err().code(),
                "ai.disabled"
            );
        })
        .await
        .unwrap();
    }
}

/// Tìm theo ý nghĩa (D-109): index theo đoạn, không tính lại đoạn không đổi, gộp với tìm theo
/// chữ, nhiều máy chủ cùng database, đổi mô hình, embedding tắt.
mod search {
    use std::sync::Arc;

    use autoxide_testkit::ai::{FakeEmbedder, FakeTextModel};

    use super::*;
    use crate::semantic::SearchQuery;
    use crate::testing::service_with;

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
            let ada = editor();
            let dalat = content
                .create(&ada, vi_post("Đà Lạt mùa hoa", "Hoa dã quỳ nở vàng đồi thông."))
                .await
                .unwrap();
            let rust =
                content.create(&ada, vi_post("Học Rust", "Ownership và lifetime.")).await.unwrap();
            for id in [dalat.id, rust.id] {
                content.index_entry(id).await.unwrap();
            }
            let calls = embedder.calls();
            // Không đổi gì: không gọi lại embedding.
            content.index_entry(dalat.id).await.unwrap();
            assert_eq!(embedder.calls(), calls);

            // Khớp cả ý nghĩa lẫn chữ.
            let hits = content.search(Some(&ada), &query("đà lạt"), false).await.unwrap();
            assert_eq!(hits[0].entry_id, dalat.id);
            assert_eq!(hits[0].matched_by, ["meaning", "text"]);
            assert!(hits[0].similarity.unwrap() > 0.0);
            assert_eq!(hits[0].status.as_deref(), Some("draft"));
            // Không chứa nguyên cụm chữ, nhưng gần về ý nghĩa.
            let hits = content.search(Some(&ada), &query("hoa nở ở đà lạt"), false).await.unwrap();
            assert_eq!((hits[0].entry_id, hits[0].matched_by.clone()), (dalat.id, vec!["meaning"]));
            assert!(hits[0].snippet.contains("dã quỳ"));

            // Công khai: chỉ bản đã đăng, kèm đường dẫn.
            assert!(content.search(None, &query("đà lạt"), true).await.unwrap().is_empty());
            content.publish(&ada, dalat.id, None, None, false).await.unwrap();
            content.index_entry(dalat.id).await.unwrap();
            let public = content.search(None, &query("đà lạt"), true).await.unwrap();
            assert_eq!(public.len(), 1);
            assert_eq!(
                (public[0].path.as_deref(), public[0].status.clone()),
                (Some("/blog/da-lat-mua-hoa"), None)
            );

            // Kiểm đầu vào; cần quyền đọc.
            let bad = SearchQuery { q: " ".into(), limit: Some(51), ..Default::default() };
            let error = content.search(Some(&ada), &bad, false).await.unwrap_err();
            assert_eq!(
                error.fields().iter().map(|f| f.field.as_str()).collect::<Vec<_>>(),
                ["q", "limit"]
            );
            assert_eq!(
                content
                    .search(Some(&user(&["write"])), &query("x"), false)
                    .await
                    .unwrap_err()
                    .code(),
                "permission_denied"
            );

            // Xoá mục: index của nó biến mất.
            content.delete(&ada, rust.id, None).await.unwrap();
            assert_eq!(rows_of(&database, rust.id).await, 1);
            content.index_entry(rust.id).await.unwrap();
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
            let (second, _, _) =
                service_with(test, config(&[]), ai.clone(), embedder.clone()).await;
            let ada = editor();
            let a = first.create(&ada, vi_post("Phở Hà Nội", "Nước dùng trong.")).await.unwrap();
            first.index_entry(a.id).await.unwrap();
            assert_eq!(second.search(Some(&ada), &query("phở"), false).await.unwrap().len(), 1);
            // Máy chủ thứ nhất thêm mục: máy thứ hai (đã nạp vào bộ nhớ) thấy nhờ `version`.
            let b = first
                .create(&ada, vi_post("Bún chả", "Phở và bún chả là món Hà Nội."))
                .await
                .unwrap();
            first.index_entry(b.id).await.unwrap();
            let hits = second.search(Some(&ada), &query("phở hà nội"), false).await.unwrap();
            assert_eq!(hits.iter().filter(|hit| hit.matched_by.contains(&"meaning")).count(), 2);

            // Hai job lập index cùng mục chạy cùng lúc: cuối cùng mỗi đoạn một dòng.
            let (x, y) = tokio::join!(first.index_entry(b.id), second.index_entry(b.id));
            if x.is_err() || y.is_err() {
                first.index_entry(b.id).await.unwrap();
            }
            assert_eq!(rows_of(&database, b.id).await, 1, "one row per chunk (draft only)");

            // Đổi mô hình: job định kỳ xếp lập lại mọi mục.
            let other = Arc::new(FakeEmbedder::new(128).with_model("another-model"));
            let (third, _, _) = service_with(test, config(&[]), ai, other).await;
            assert_eq!(third.catch_up_embeddings().await.unwrap(), 2);
            third.index_entry(a.id).await.unwrap();
            assert_eq!(third.catch_up_embeddings().await.unwrap(), 1);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn embedding_tat_chi_tim_theo_chu() {
        TestDatabase::run(async |test| {
            let (content, _, _) = service(test, config(&[])).await;
            let ada = editor();
            let dalat = content.create(&ada, vi_post("Đà Lạt mùa hoa", "Hoa.")).await.unwrap();
            content.index_entry(dalat.id).await.unwrap();
            assert_eq!(content.catch_up_embeddings().await.unwrap(), 0);
            let hits = content.search(Some(&ada), &query("da lat"), false).await.unwrap();
            assert_eq!(
                (hits.len(), hits[0].matched_by.clone(), hits[0].similarity),
                (1, vec!["text"], None)
            );
            assert_eq!(
                content.search(None, &query("da lat"), true).await.unwrap_err().code(),
                "content.semantic_search_disabled"
            );
        })
        .await
        .unwrap();
    }
}

/// Sửa sau rà soát (D-111): yêu cầu dịch kẹt ở `running` bị yêu cầu mới thay chỗ; yêu cầu đã
/// huỷ không bị job đang chạy ghi đè thành "thành công".
mod translation_recovery {
    use std::sync::Arc;

    use autoxide_kernel::ai::{Part, Prompt};
    use autoxide_testkit::ai::{FakeEmbedder, FakeTextModel};
    use sea_orm::sea_query::Expr;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    use super::*;
    use crate::entity::translation_request;
    use crate::testing::service_with;

    fn translator() -> FakeTextModel {
        FakeTextModel::new(|prompt: &Prompt| {
            let Part::Text(input) = &prompt.input[0] else { panic!() };
            let items: Vec<Value> = serde_json::from_str(input).unwrap();
            let segments: Vec<Value> =
                items.iter().map(|item| json!({"id": item["id"], "text": "dịch"})).collect();
            Ok(json!({"segments": segments}))
        })
    }

    #[tokio::test]
    async fn yeu_cau_ket_bi_thay_va_huy_khong_bi_ghi_de() {
        TestDatabase::run(async |test| {
            let (content, _, database) = service_with(
                test,
                config(&[]),
                Arc::new(translator()),
                Arc::new(FakeEmbedder::disabled()),
            )
            .await;
            let ada = editor();
            let source = content.create(&ada, post("Hello")).await.unwrap();
            let stuck = content.request_translation(&ada, source.id, "vi").await.unwrap();
            // Như một job bị worker cắt ngang ở lần cuối: `running`, đứng yên 20 phút.
            translation_request::Entity::update_many()
                .col_expr(translation_request::Column::Status, Expr::value("running"))
                .col_expr(
                    translation_request::Column::UpdatedAt,
                    Expr::value(chrono::Utc::now() - chrono::Duration::minutes(20)),
                )
                .filter(translation_request::Column::Id.eq(stuck.id))
                .exec(database.write())
                .await
                .unwrap();
            let fresh = content.request_translation(&ada, source.id, "vi").await.unwrap();
            let old = translation_request::Entity::find_by_id(stuck.id)
                .one(database.read())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                (old.status.as_str(), old.error_code.as_deref(), old.active_key),
                ("failed", Some("content.translation_stale"), None)
            );
            // Job cũ chạy tới: thấy đã xong, không làm gì.
            content.run_translation(stuck.id, 8).await.unwrap();
            assert_eq!(
                translation_request::Entity::find_by_id(stuck.id)
                    .one(database.read())
                    .await
                    .unwrap()
                    .unwrap()
                    .status,
                "failed"
            );

            // Huỷ (tài khoản bị xoá) rồi job chạy: vẫn là huỷ.
            assert_eq!(content.cancel_translations_of(ada.user_id()).await.unwrap(), 1);
            content.run_translation(fresh.id, 1).await.unwrap();
            let canceled = translation_request::Entity::find_by_id(fresh.id)
                .one(database.read())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                (canceled.status.as_str(), canceled.error_code.as_deref()),
                ("failed", Some("content.translation_canceled"))
            );
        })
        .await
        .unwrap();
    }
}

/// Lượt rà soát bảo mật module `content` (sau bước 18): mỗi test giữ một lỗi không quay lại,
/// chạy trên cả SQLite và Postgres.
mod bao_mat {
    use autoxide_kernel::access::Principal;
    use autoxide_kernel::event::{Delivered, EventHandler};
    use chrono::{Duration, Utc};
    use sea_orm::EntityTrait;

    use super::*;
    use crate::authors::AuthorInput;
    use crate::menus::ItemInput;
    use crate::redirects::{RedirectInput, Resolved};
    use crate::schedules::Action;
    use crate::terms::{Taxonomy, TermInput};

    /// Agent dùng API key `key` của người dùng `user_id`, chỉ có quyền `_own`.
    fn agent(user_id: Uuid, key: Uuid) -> Principal {
        Principal::new(
            user_id,
            ["read", "write_own", "publish_own"].iter().map(|p| format!("content.{p}")),
        )
        .via_api_key(key)
    }

    fn bot() -> Principal {
        agent(Uuid::now_v7(), Uuid::now_v7())
    }

    fn field_codes(error: &autoxide_kernel::error::Error) -> Vec<(String, String)> {
        error.fields().iter().map(|field| (field.field.clone(), field.code.clone())).collect()
    }

    /// Gỡ hay lưu trữ huỷ lịch đăng đang chờ (lưu trữ huỷ cả lịch gỡ): lịch đặt từ trước không
    /// đăng lại mục vừa bị gỡ.
    #[tokio::test]
    async fn go_va_luu_tru_huy_lich_dang() {
        TestDatabase::run(async |test| {
            let (content, _, database) = service(test, config(&[])).await;
            let ada = editor();
            let at = Utc::now() + Duration::hours(1);

            let entry = content.create(&ada, post("Taken down")).await.unwrap();
            content.publish(&ada, entry.id, None, None, false).await.unwrap();
            let planned =
                content.create_schedule(&ada, entry.id, Action::Publish, at, None).await.unwrap();
            content.unpublish(&ada, entry.id, None).await.unwrap();
            content.run_schedule(planned.id, Utc::now()).await.unwrap();
            assert_eq!(content.get(&ada, entry.id).await.unwrap().status, "draft");
            assert_eq!(
                content.resolve("en", "/blog/taken-down").await.unwrap_err().code(),
                "content.not_found"
            );
            let rows = content.list_schedules(&ada, entry.id).await.unwrap();
            assert_eq!(rows[0].status, "canceled");

            let archived = content.create(&ada, post("Archived")).await.unwrap();
            content.publish(&ada, archived.id, None, None, false).await.unwrap();
            let up = content
                .create_schedule(&ada, archived.id, Action::Publish, at, None)
                .await
                .unwrap();
            let down = content
                .create_schedule(&ada, archived.id, Action::Unpublish, at, None)
                .await
                .unwrap();
            content.archive(&ada, archived.id, None).await.unwrap();
            for id in [up.id, down.id] {
                content.run_schedule(id, Utc::now()).await.unwrap();
            }
            assert_eq!(content.get(&ada, archived.id).await.unwrap().status, "archived");
            assert!(matches!(
                content.resolve("en", "/blog/archived").await.unwrap(),
                Resolved::Gone
            ));
            let statuses: Vec<String> = content
                .list_schedules(&ada, archived.id)
                .await
                .unwrap()
                .into_iter()
                .map(|row| row.status)
                .collect();
            assert_eq!(statuses, ["canceled", "canceled"]);
            let canceled = events(&database)
                .await
                .iter()
                .filter(|name| *name == "content.schedule_canceled")
                .count();
            assert_eq!(canceled, 3, "one event per canceled schedule");
        })
        .await
        .unwrap();
    }

    /// Người chỉ có `content.publish_own` không đăng lại mục biên tập viên đã lưu trữ, không tự
    /// đặt `published_at`, không đăng đè chuyển hướng làm tay; người có `content.publish` thì
    /// được.
    #[tokio::test]
    async fn publish_own_bi_gioi_han() {
        TestDatabase::run(async |test| {
            let (content, _, _) = service(test, config(&[])).await;
            let ada = editor();
            let bot = bot();
            let mine = content.create(&bot, post("Mine")).await.unwrap();
            content.publish(&bot, mine.id, None, None, false).await.unwrap();
            content.archive(&ada, mine.id, None).await.unwrap();
            let error = content.publish(&bot, mine.id, None, None, false).await.unwrap_err();
            assert_eq!((error.kind(), error.code()), (ErrorKind::Forbidden, "content.archived"));
            content.publish(&ada, mine.id, None, None, false).await.unwrap();

            let dated = content.create(&bot, post("Dated")).await.unwrap();
            let when = Some(Utc::now() + Duration::days(30));
            let error = content.publish(&bot, dated.id, None, when, false).await.unwrap_err();
            assert_eq!(error.code(), "validation_failed");
            assert_eq!(field_codes(&error), [("published_at".into(), "not_allowed".into())]);
            content.publish(&bot, dated.id, None, None, false).await.unwrap();
            let theirs = content.create(&ada, post("Editor dated")).await.unwrap();
            content.publish(&ada, theirs.id, None, when, false).await.unwrap();

            let target = content.create(&ada, post("Target")).await.unwrap();
            let redirect = RedirectInput {
                locale: Some("en".into()),
                from_path: "/blog/campaign".into(),
                to_entry_id: Some(target.id),
                to_url: None,
                status_code: 302,
            };
            content.create_redirect(&ada, redirect).await.unwrap();
            let campaign = content.create(&bot, post("Campaign")).await.unwrap();
            let error = content.publish(&bot, campaign.id, None, None, false).await.unwrap_err();
            assert_eq!(
                (error.kind(), error.code()),
                (ErrorKind::Conflict, "content.path_has_redirect")
            );
            assert_eq!(content.get(&ada, campaign.id).await.unwrap().status, "draft");
            content.publish(&ada, campaign.id, None, None, false).await.unwrap();
        })
        .await
        .unwrap();
    }

    /// Người chỉ có `content.write_own`: trang cha (khi tạo, chuyển, dịch) phải là của mình;
    /// `ai_generated` luôn bật; chỉ thêm hồ sơ tác giả của chính mình.
    #[tokio::test]
    async fn write_own_bi_gioi_han() {
        TestDatabase::run(async |test| {
            let (content, _, _) = service(test, config(&[])).await;
            let ada = editor();
            let bot = bot();

            // Trang cha.
            let theirs = content.create(&ada, page("Theirs", None)).await.unwrap();
            let error = content.create(&bot, page("Child", Some(theirs.id))).await.unwrap_err();
            assert_eq!((error.kind(), error.code()), (ErrorKind::Forbidden, "content.not_owner"));
            let mine = content.create(&bot, page("Mine", None)).await.unwrap();
            let child = content.create(&bot, page("Child", Some(mine.id))).await.unwrap();
            let error = content.move_page(&bot, child.id, Some(theirs.id), None).await.unwrap_err();
            assert_eq!(error.code(), "content.not_owner");
            content.move_page(&bot, child.id, None, None).await.unwrap();
            content.move_page(&bot, child.id, Some(mine.id), None).await.unwrap();
            // Bản dịch của trang cha do người khác tạo: bản dịch trang con không nằm dưới nó.
            content.create_translation(&ada, mine.id, "vi", fields("Của tôi")).await.unwrap();
            let error =
                content.create_translation(&bot, child.id, "vi", fields("Con")).await.unwrap_err();
            assert_eq!(error.code(), "content.not_owner");
            // Người có `content.write` đặt được dưới trang của người khác.
            content.create(&ada, page("Editor child", Some(mine.id))).await.unwrap();

            // `ai_generated` không tự khai được.
            let mut input = post("Bot post");
            input.fields.ai_generated = Some(false);
            let entry = content.create(&bot, input).await.unwrap();
            assert!(entry.ai_generated);
            let no = EntryFields { ai_generated: Some(false), ..Default::default() };
            let entry = content.update(&bot, entry.id, no.clone(), false, None).await.unwrap();
            assert!(entry.ai_generated);
            let vi = content.create_translation(&bot, entry.id, "vi", fields("Bài")).await;
            assert!(vi.unwrap().ai_generated);
            let entry = content.update(&ada, entry.id, no, false, None).await.unwrap();
            assert!(!entry.ai_generated, "an editor may correct it");
            let human = content.create(&ada, post("Human post")).await.unwrap();
            assert!(!human.ai_generated);

            // Tác giả.
            let profile = |name: &str, user: Option<Uuid>| AuthorInput {
                display_name: Some(name.into()),
                user_id: Some(user),
                ..Default::default()
            };
            let own = content.create_author(&ada, profile("Bot", Some(bot.user_id()))).await;
            let own = own.unwrap();
            let other = content.create_author(&ada, profile("Ada", Some(ada.user_id()))).await;
            let other = other.unwrap();
            let error =
                content.set_entry_authors(&bot, entry.id, vec![other.id]).await.unwrap_err();
            assert_eq!(field_codes(&error), [("author_ids".into(), "not_allowed".into())]);
            content.set_entry_authors(&bot, entry.id, vec![own.id]).await.unwrap();
            // Hồ sơ biên tập viên đã gắn thì giữ được.
            content.set_entry_authors(&ada, entry.id, vec![own.id, other.id]).await.unwrap();
            let kept = content.set_entry_authors(&bot, entry.id, vec![other.id, own.id]).await;
            assert_eq!(kept.unwrap(), [other.id, own.id]);
        })
        .await
        .unwrap();
    }

    /// Đích chuyển hướng và link menu dạng `//host` (địa chỉ không scheme của site khác) bị từ
    /// chối, như link trong thân bài.
    #[tokio::test]
    async fn duong_dan_hai_gach_bi_tu_choi() {
        TestDatabase::run(async |test| {
            let (content, _, _) = service(test, config(&[])).await;
            let ada = editor();
            for to in ["//evil.com", "//evil.com/path", "/\\evil.com"] {
                let input = RedirectInput {
                    locale: Some("en".into()),
                    from_path: "/old".into(),
                    to_entry_id: None,
                    to_url: Some(to.into()),
                    status_code: 301,
                };
                let error = content.create_redirect(&ada, input).await.unwrap_err();
                assert_eq!(field_codes(&error), [("to_url".into(), "invalid".into())], "{to}");
            }
            let fine = RedirectInput {
                locale: Some("en".into()),
                from_path: "/old".into(),
                to_entry_id: None,
                to_url: Some("/new".into()),
                status_code: 301,
            };
            content.create_redirect(&ada, fine).await.unwrap();

            let item = |url: &str| ItemInput {
                label: "Link".into(),
                entry_id: None,
                term_id: None,
                url: Some(url.into()),
                children: vec![],
            };
            let error = content.put_menu(&ada, "main", None, vec![item("//evil.com")], None).await;
            let error = error.unwrap_err();
            assert_eq!(field_codes(&error), [("items[0].url".into(), "invalid".into())]);
            content.put_menu(&ada, "main", None, vec![item("/about")], None).await.unwrap();
        })
        .await
        .unwrap();
    }

    /// Gắn term / tác giả cùng lúc cho một mục (và cùng lúc với xoá mục): không lỗi 500 vì trùng
    /// khoá, không liên kết mồ côi — trên cả hai database.
    #[tokio::test]
    async fn gan_the_tac_gia_dong_thoi() {
        TestDatabase::run(async |test| {
            let (content, _, database) = service(test, config(&[])).await;
            let ada = editor();
            let mut tags = Vec::new();
            for name in ["A", "B", "C"] {
                let input = TermInput {
                    locale: Some("en".into()),
                    name: Some(name.into()),
                    ..Default::default()
                };
                tags.push(content.create_term(&ada, Taxonomy::Tag, input).await.unwrap().term.id);
            }
            let mut authors = Vec::new();
            for name in ["X", "Y"] {
                let input = AuthorInput { display_name: Some(name.into()), ..Default::default() };
                authors.push(content.create_author(&ada, input).await.unwrap().id);
            }
            let entry = content.create(&ada, post("Busy")).await.unwrap();
            let sets: Vec<Vec<Uuid>> = vec![
                tags.clone(),
                vec![tags[1], tags[0]],
                vec![tags[2]],
                tags.iter().rev().copied().collect(),
            ];
            // Nhiều vòng, mỗi vòng mọi bộ cùng lúc: đủ để các transaction chồng lên nhau.
            for _ in 0..5 {
                let tasks: Vec<_> = sets
                    .iter()
                    .chain(sets.iter())
                    .cloned()
                    .enumerate()
                    .map(|(n, set)| {
                        let content = content.clone();
                        let authors = if n % 2 == 0 { authors.clone() } else { vec![authors[1]] };
                        tokio::spawn(async move {
                            let ada = editor();
                            let (terms, names) = tokio::join!(
                                content.set_entry_terms(&ada, entry.id, set),
                                content.set_entry_authors(&ada, entry.id, authors),
                            );
                            (terms, names)
                        })
                    })
                    .collect();
                for task in tasks {
                    let (terms, names) = task.await.unwrap();
                    terms.unwrap();
                    names.unwrap();
                }
            }
            let now = crate::terms::term_ids_of(database.read(), &[entry.id]).await.unwrap();
            assert!(sets.contains(&now[&entry.id]), "one of the sets won whole: {now:?}");
            let now = crate::authors::author_ids_of(database.read(), &[entry.id]).await.unwrap();
            assert!(now[&entry.id] == authors || now[&entry.id] == [authors[1]]);

            for _ in 0..4 {
                let entry = content.create(&ada, post("Doomed")).await.unwrap();
                let (deleted, terms, names) = tokio::join!(
                    content.delete(&ada, entry.id, None),
                    content.set_entry_terms(&ada, entry.id, tags.clone()),
                    content.set_entry_authors(&ada, entry.id, authors.clone()),
                );
                deleted.unwrap();
                for result in [terms, names] {
                    if let Err(error) = result {
                        assert_eq!(error.code(), "content.not_found");
                    }
                }
            }
            for table in ["content_entry_terms", "content_entry_authors"] {
                let orphans = count(
                    &database,
                    &format!(
                        "SELECT COUNT(*) FROM {table} l WHERE NOT EXISTS \
                         (SELECT 1 FROM content_entries e WHERE e.id = l.entry_id)"
                    ),
                )
                .await;
                assert_eq!(orphans, 0, "{table}");
            }
        })
        .await
        .unwrap();
    }

    /// Gắn term / tác giả và tạo link xem trước chỉ cần quyền sửa mục, không cần `content.read`
    /// (như khôi phục revision, D-111).
    #[tokio::test]
    async fn sua_lien_ket_khong_can_quyen_doc() {
        TestDatabase::run(async |test| {
            let (content, _, _) =
                service(test, config(&[("AUTOXIDE_CONTENT_PREVIEW_SECRET", public::SECRET)])).await;
            let ada = editor();
            let writer = user(&["write"]);
            let tag = TermInput {
                locale: Some("en".into()),
                name: Some("Tag".into()),
                ..Default::default()
            };
            let tag = content.create_term(&ada, Taxonomy::Tag, tag).await.unwrap();
            let author = AuthorInput { display_name: Some("Ada".into()), ..Default::default() };
            let author = content.create_author(&ada, author).await.unwrap();
            let entry = content.create(&writer, post("Writer's")).await.unwrap();
            content.set_entry_terms(&writer, entry.id, vec![tag.term.id]).await.unwrap();
            content.set_entry_authors(&writer, entry.id, vec![author.id]).await.unwrap();
            content.preview_token(&writer, entry.id).await.unwrap();
            assert_eq!(
                content.get(&writer, entry.id).await.unwrap_err().code(),
                "permission_denied",
                "reading still needs content.read"
            );
        })
        .await
        .unwrap();
    }

    /// Xoá tài khoản bỏ trống cả `review_requested_by` và `ai_reviewed_by` (giữ thời điểm);
    /// chạy hai lần vẫn vậy.
    #[tokio::test]
    async fn xoa_tai_khoan_bo_nguoi_duyet() {
        TestDatabase::run(async |test| {
            let (content, _, database) = service(test, config(&[])).await;
            let ada = editor();
            let mut input = post("Reviewed");
            input.fields.ai_generated = Some(true);
            let entry = content.create(&ada, input).await.unwrap();
            content.publish(&ada, entry.id, None, None, true).await.unwrap();
            content.set_review_request(&ada, entry.id, None, false).await.unwrap();

            let deleted = Delivered::new(
                "auth",
                autoxide_kernel::privacy::USER_DELETED.to_owned(),
                json!({"before": {"id": ada.user_id()}}),
            );
            content.handle(&deleted).await.unwrap();
            content.handle(&deleted).await.unwrap();

            let after = content.get(&editor(), entry.id).await.unwrap();
            assert_eq!(after.review_requested_by, None);
            assert!(after.review_requested_at.is_some(), "the request itself stays");
            let published = crate::entity::published::Entity::find_by_id(entry.id)
                .one(database.read())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(published.ai_reviewed_by, None);
            assert!(published.ai_reviewed_at.is_some(), "readers still see ai_reviewed");
        })
        .await
        .unwrap();
    }
}
