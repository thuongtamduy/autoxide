//! Các lỗi tìm thấy ở lượt rà soát độc lập sau khi gộp module (ngày 12/11/2026). Mỗi test giữ
//! một lỗi không quay lại.

use super::*;
use crate::publishing::reading::PostFilter;
use crate::publishing::redirects::{RedirectInput, Resolved};
use crate::publishing::schedules::Action;
use crate::taxonomy::authors::AuthorInput;
use crate::taxonomy::terms::{Taxonomy, TermInput};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;

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
        let (content, _, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let parent = content.create(&tenant, &ada, page("Parent", None)).await.unwrap();
        let about = content.create(&tenant, &ada, page("About", None)).await.unwrap();
        content.publish(&tenant, &ada, parent.id, None, None, false).await.unwrap();
        content.publish(&tenant, &ada, about.id, None, None, false).await.unwrap();
        let draft = EntryFields { slug: Some("secret-draft".into()), ..Default::default() };
        content.update(&tenant, &ada, about.id, draft, false, None).await.unwrap();
        content.move_page(&tenant, &ada, about.id, Some(parent.id), None).await.unwrap();
        assert!(
            content
                .resolve(
                    &tenant,
                    &autoxide_kernel::tenancy::Site::whole(&tenant),
                    "en",
                    "/parent/secret-draft"
                )
                .await
                .is_err()
        );
        assert!(matches!(
            content
                .resolve(
                    &tenant,
                    &autoxide_kernel::tenancy::Site::whole(&tenant),
                    "en",
                    "/parent/about"
                )
                .await
                .unwrap(),
            Resolved::Entry(_)
        ));
        assert!(matches!(
            content
                .resolve(&tenant, &autoxide_kernel::tenancy::Site::whole(&tenant), "en", "/about")
                .await
                .unwrap(),
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
        let (content, _, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let hello = content.create(&tenant, &ada, post("Hello")).await.unwrap();
        content.publish(&tenant, &ada, hello.id, None, None, false).await.unwrap();
        let news = content.create(&tenant, &ada, page("News", None)).await.unwrap();
        let child = content.create(&tenant, &ada, page("Hello", Some(news.id))).await.unwrap();
        content.publish(&tenant, &ada, news.id, None, None, false).await.unwrap();
        content.publish(&tenant, &ada, child.id, None, None, false).await.unwrap();
        let rename = EntryFields { slug: Some("blog".into()), ..Default::default() };
        content.update(&tenant, &ada, news.id, rename, false, None).await.unwrap();
        let error = content.publish(&tenant, &ada, news.id, None, None, false).await.unwrap_err();
        assert_eq!(error.code(), "content.path_taken", "{error:?}");
        assert!(matches!(
            content
                .resolve(
                    &tenant,
                    &autoxide_kernel::tenancy::Site::whole(&tenant),
                    "en",
                    "/news/hello"
                )
                .await
                .unwrap(),
            Resolved::Entry(_)
        ));

        // Chuyển trang cũng vậy: trang `blog` gốc có con `hello` → `/blog/hello` trùng bài.
        // (Trả slug nháp của `News` về như cũ để `blog` còn trống.)
        let back = EntryFields { slug: Some("news".into()), ..Default::default() };
        content.update(&tenant, &ada, news.id, back, false, None).await.unwrap();
        let blog = content.create(&tenant, &ada, page("Blog", None)).await.unwrap();
        assert_eq!(blog.slug, "blog");
        content.publish(&tenant, &ada, blog.id, None, None, false).await.unwrap();
        let error =
            content.move_page(&tenant, &ada, child.id, Some(blog.id), None).await.unwrap_err();
        assert_eq!(error.code(), "content.path_taken", "{error:?}");
    })
    .await
    .unwrap();
}

/// Bản dịch của trang không được vượt độ sâu tối đa (cha trong ngôn ngữ kia đã sâu hơn).
#[tokio::test]
async fn ban_dich_khong_vuot_do_sau() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let root = content.create(&tenant, &ada, page("Root", None)).await.unwrap();
        let child = content.create(&tenant, &ada, page("Child", Some(root.id))).await.unwrap();
        let root_vi =
            content.create_translation(&tenant, &ada, root.id, "vi", fields("Gốc")).await.unwrap();
        // Cây tiếng Việt: bốn cấp, rồi đưa bản dịch của `Root` xuống cấp 5.
        let mut parent = None;
        for level in 1..=4 {
            let mut input = page(&format!("Cấp {level}"), parent);
            input.locale = Some("vi".into());
            parent = Some(content.create(&tenant, &ada, input).await.unwrap().id);
        }
        content.move_page(&tenant, &ada, root_vi.id, parent, None).await.unwrap();
        let error = content
            .create_translation(&tenant, &ada, child.id, "vi", fields("Con"))
            .await
            .unwrap_err();
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
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        for _ in 0..5 {
            let parent = content.create(&tenant, &ada, page("Parent", None)).await.unwrap();
            let (deleted, created) = tokio::join!(
                content.delete(&tenant, &ada, parent.id, None),
                content.create(&tenant, &ada, page("Child", Some(parent.id)))
            );
            assert!(deleted.is_ok() != created.is_ok(), "exactly one wins");
            if let Ok(child) = created {
                content.delete(&tenant, &ada, child.id, None).await.unwrap();
                content.delete(&tenant, &ada, parent.id, None).await.unwrap();
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
        let (content, _, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let english = content.create(&tenant, &ada, post("English")).await.unwrap();
        content.publish(&tenant, &ada, english.id, None, None, false).await.unwrap();
        let error = content
            .create_redirect(&tenant, &ada, redirect("vi", "/cu", english.id))
            .await
            .unwrap_err();
        assert_eq!(error.fields()[0].code, "other_locale");

        content.create_redirect(&tenant, &ada, redirect("en", "/old", english.id)).await.unwrap();
        let rename = EntryFields { slug: Some("renamed".into()), ..Default::default() };
        content.update(&tenant, &ada, english.id, rename, false, None).await.unwrap();
        content.publish(&tenant, &ada, english.id, None, None, false).await.unwrap();
        content.archive(&tenant, &ada, english.id, None).await.unwrap();
        for old in ["/old", "/blog/english", "/blog/renamed"] {
            assert!(
                matches!(
                    content
                        .resolve(
                            &tenant,
                            &autoxide_kernel::tenancy::Site::whole(&tenant),
                            "en",
                            old
                        )
                        .await
                        .unwrap(),
                    Resolved::Gone
                ),
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
        let (content, _, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let writer = user(&["read", "write"]);
        let tag = TermInput {
            locale: Some("en".into()),
            name: Some("Food".into()),
            ..Default::default()
        };
        let tag = content.create_term(&tenant, &ada, Taxonomy::Tag, tag).await.unwrap();
        let author = AuthorInput { display_name: Some("Ada".into()), ..Default::default() };
        let author = content.create_author(&tenant, &ada, author).await.unwrap();
        let entry = content.create(&tenant, &ada, post("Live")).await.unwrap();
        content.set_entry_terms(&tenant, &writer, entry.id, vec![tag.term.id]).await.unwrap();
        content.publish(&tenant, &ada, entry.id, None, None, false).await.unwrap();
        let denied = content.set_entry_terms(&tenant, &writer, entry.id, vec![]).await.unwrap_err();
        assert_eq!(denied.kind(), ErrorKind::Forbidden);
        let denied = content
            .set_entry_authors(&tenant, &writer, entry.id, vec![author.id])
            .await
            .unwrap_err();
        assert_eq!(denied.kind(), ErrorKind::Forbidden);
        content.set_entry_authors(&tenant, &ada, entry.id, vec![author.id]).await.unwrap();
    })
    .await
    .unwrap();
}

/// Hai lệnh đổi cha chéo nhau của danh mục chạy cùng lúc: không tạo vòng.
#[tokio::test]
async fn doi_cha_danh_muc_cheo_dong_thoi() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        for round in 0..5 {
            let name = |n: &str| TermInput {
                locale: Some("en".into()),
                name: Some(format!("{n} {round}")),
                ..Default::default()
            };
            let a =
                content.create_term(&tenant, &ada, Taxonomy::Category, name("A")).await.unwrap();
            let b =
                content.create_term(&tenant, &ada, Taxonomy::Category, name("B")).await.unwrap();
            let under =
                |parent: Uuid| TermInput { parent_id: Some(Some(parent)), ..Default::default() };
            let (first, second) = tokio::join!(
                content.update_term(&tenant, &ada, a.term.id, under(b.term.id), None),
                content.update_term(&tenant, &ada, b.term.id, under(a.term.id), None)
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
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let entry = content.create(&tenant, &ada, post("Planned")).await.unwrap();
        content.publish(&tenant, &ada, entry.id, None, None, false).await.unwrap();
        let at = Timestamp::now() + SignedDuration::from_hours(1);
        let down = content
            .create_schedule(&tenant, &ada, entry.id, Action::Unpublish, at, None)
            .await
            .unwrap();
        let before = content.revision_count(&tenant, entry.id).await;
        content.run_schedule(&tenant, down.id, Timestamp::now()).await.unwrap();
        assert_eq!(content.revision_count(&tenant, entry.id).await, before + 1);

        content.publish(&tenant, &ada, entry.id, None, None, false).await.unwrap();
        content
            .create_schedule(&tenant, &ada, entry.id, Action::Unpublish, at, None)
            .await
            .unwrap();
        assert_eq!(
            crate::publishing::schedules::cancel_schedules_by(
                &tenant,
                Some(&content.outbox),
                ada.user_id()
            )
            .await
            .unwrap(),
            1
        );
        assert_eq!(
            crate::publishing::schedules::cancel_schedules_by(
                &tenant,
                Some(&content.outbox),
                ada.user_id()
            )
            .await
            .unwrap(),
            0
        );
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
        let (content, media, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let private = media.add(false);
        let input = AuthorInput {
            display_name: Some("Ada".into()),
            avatar_media_id: Some(Some(private)),
            ..Default::default()
        };
        let author = content.create_author(&tenant, &ada, input).await.unwrap();
        let entry = content.create(&tenant, &ada, post("By Ada")).await.unwrap();
        content.set_entry_authors(&tenant, &ada, entry.id, vec![author.id]).await.unwrap();
        content.publish(&tenant, &ada, entry.id, None, None, false).await.unwrap();
        let filter = PostFilter {
            locale: "en".into(),
            site: autoxide_kernel::tenancy::Site::whole(&tenant).lineage_ids(),
            ..Default::default()
        };
        let page =
            content.public_posts(&tenant, &filter, &PageRequest::first(10, Order::Desc)).await;
        let mut views = content.public_views(&tenant, page.unwrap().data).await.unwrap();
        let view = views.remove(0);
        assert_eq!(view.authors[0].avatar_media_id, None);
    })
    .await
    .unwrap();
}

/// PT-16: tám bài cùng tiêu đề tạo cùng lúc đều được tạo, mỗi bài một slug (`load-test-draft`, `-2` … `-8`); slug đã
/// có ở nhóm ứng viên đầu thì lấy ở nhóm sau — đụng unique index thì thử hậu tố kế, không trả 409.
#[tokio::test]
async fn tao_cung_tieu_de_dong_thoi_moi_bai_mot_slug() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let create = || content.create(&tenant, &ada, post("Load test draft"));
        let (a, b, c, d, e, f, g, h) = tokio::join!(
            create(),
            create(),
            create(),
            create(),
            create(),
            create(),
            create(),
            create()
        );
        let mut slugs: Vec<String> =
            [a, b, c, d, e, f, g, h].into_iter().map(|result| result.unwrap().slug).collect();
        slugs.sort();
        slugs.dedup();
        assert_eq!(slugs.len(), 8, "{slugs:?}");
        assert!(slugs.contains(&"load-test-draft".to_owned()), "{slugs:?}");
        // Hết nhóm đầu (`base`, `-2` … `-20`): bài thứ 21 lấy `-21`.
        for _ in 8..20 {
            content.create(&tenant, &ada, post("Load test draft")).await.unwrap();
        }
        let next = content.create(&tenant, &ada, post("Load test draft")).await.unwrap();
        assert_eq!(next.slug, "load-test-draft-21");
    })
    .await
    .unwrap();
}
