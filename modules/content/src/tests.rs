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

    const SECRET: &str = "a-test-secret-that-is-long-enough-0123456789";

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
            let query = autoxide_kernel::pagination::PageQuery {
                limit: Some("2".into()),
                cursor: Some(cursor),
                order: Some("desc".into()),
            };
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
    use autoxide_kernel::event::{Actor, Delivered, EventHandler};
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

            let deleted = Delivered {
                id: Uuid::now_v7(),
                module: "auth",
                name: autoxide_kernel::privacy::USER_DELETED.to_owned(),
                actor: Actor::system(),
                subject: None,
                payload: json!({"before": {"id": ada.user_id()}}),
                occurred_at: Utc::now(),
            };
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
