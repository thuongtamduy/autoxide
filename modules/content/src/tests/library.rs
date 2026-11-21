use super::*;
use crate::publishing::redirects::Resolved;
use crate::taxonomy::authors::AuthorInput;
use crate::taxonomy::menus::ItemInput;
use crate::taxonomy::terms::{Taxonomy, TermInput};
use autoxide_kernel::tenancy::TenantContext;
use jiff::SignedDuration;

fn name(locale: &str, name: &str) -> TermInput {
    TermInput { locale: Some(locale.into()), name: Some(name.into()), ..Default::default() }
}

/// Bản dịch: một mục mới cùng nhóm, slug riêng; trang dịch đi theo bản dịch của cha; thiếu
/// bản dịch thì 404, không lấy ngôn ngữ khác; bản công khai liệt kê bản dịch (hreflang).
#[tokio::test]
async fn ban_dich() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let about = content.create(&tenant, &ada, page("About", None)).await.unwrap();
        let team = content.create(&tenant, &ada, page("Team", Some(about.id))).await.unwrap();
        let early = content
            .create_translation(&tenant, &ada, team.id, "vi", EntryFields::default())
            .await
            .unwrap_err();
        assert_eq!(early.code(), "content.parent_not_translated");
        let mut vi_fields = fields("Giới thiệu");
        vi_fields.slug = Some(String::new());
        let gioi_thieu =
            content.create_translation(&tenant, &ada, about.id, "vi", vi_fields).await.unwrap();
        assert_eq!(
            (gioi_thieu.translation_group_id, gioi_thieu.slug.as_str()),
            (about.translation_group_id, "gioi-thieu")
        );
        let doi_ngu = content
            .create_translation(&tenant, &ada, team.id, "vi", fields("Đội ngũ"))
            .await
            .unwrap();
        assert_eq!(
            (doi_ngu.parent_id, doi_ngu.slug.as_str()),
            (Some(gioi_thieu.id), "team"),
            "slug copied unless given"
        );
        let again = content
            .create_translation(&tenant, &ada, about.id, "vi", EntryFields::default())
            .await
            .unwrap_err();
        assert_eq!(again.code(), "content.translation_exists");
        let same = content
            .create_translation(&tenant, &ada, about.id, "en", EntryFields::default())
            .await
            .unwrap_err();
        assert_eq!(same.fields()[0].field, "locale");

        content.publish(&tenant, &ada, about.id, None, None, false).await.unwrap();
        assert_eq!(
            content
                .resolve(
                    &tenant,
                    &autoxide_kernel::tenancy::Site::whole(&tenant),
                    "vi",
                    "/gioi-thieu"
                )
                .await
                .unwrap_err()
                .code(),
            "content.not_found"
        );
        assert_eq!(
            content
                .resolve(&tenant, &autoxide_kernel::tenancy::Site::whole(&tenant), "vi", "/about")
                .await
                .unwrap_err()
                .code(),
            "content.not_found",
            "no fallback"
        );
        content.publish(&tenant, &ada, gioi_thieu.id, None, None, false).await.unwrap();
        let Resolved::Entry(row) = content
            .resolve(&tenant, &autoxide_kernel::tenancy::Site::whole(&tenant), "en", "/about")
            .await
            .unwrap()
        else {
            panic!()
        };
        let view = content.public_views(&tenant, vec![*row]).await.unwrap().remove(0);
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
        let (content, _, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let travel = content
            .create_term(&tenant, &ada, Taxonomy::Category, name("en", "Travel"))
            .await
            .unwrap();
        let mut child = name("en", "Asia");
        child.parent_id = Some(Some(travel.term.id));
        let asia = content.create_term(&tenant, &ada, Taxonomy::Category, child).await.unwrap();
        let tag =
            content.create_term(&tenant, &ada, Taxonomy::Tag, name("en", "Food")).await.unwrap();
        assert_eq!(asia.texts[0].slug, "asia");
        let mut tag_parent = name("en", "Bad");
        tag_parent.parent_id = Some(Some(travel.term.id));
        assert_eq!(
            content
                .create_term(&tenant, &ada, Taxonomy::Tag, tag_parent)
                .await
                .unwrap_err()
                .fields()[0]
                .code,
            "not_allowed"
        );
        let loop_ = TermInput { parent_id: Some(Some(asia.term.id)), ..Default::default() };
        assert_eq!(
            content
                .update_term(&tenant, &ada, travel.term.id, loop_, None)
                .await
                .unwrap_err()
                .fields()[0]
                .code,
            "term_cycle"
        );
        assert_eq!(
            content
                .create_term(&tenant, &ada, Taxonomy::Tag, name("en", "Food"))
                .await
                .unwrap_err()
                .code(),
            "content.slug_taken"
        );
        let with_vi = content
            .update_term(
                &tenant,
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
        let entry = content.create(&tenant, &ada, vi_post).await.unwrap();
        content
            .set_entry_terms(
                &tenant,
                &ada,
                entry.id,
                vec![travel.term.id, tag.term.id, travel.term.id],
            )
            .await
            .unwrap();
        let filter = EntryFilter { term_id: Some(tag.term.id), ..Default::default() };
        assert_eq!(
            content
                .list(&tenant, &ada, &filter, &PageRequest::first(10, Order::Asc))
                .await
                .unwrap()
                .data
                .len(),
            1
        );
        content.publish(&tenant, &ada, entry.id, None, None, false).await.unwrap();
        let Resolved::Entry(row) = content
            .resolve(
                &tenant,
                &autoxide_kernel::tenancy::Site::whole(&tenant),
                "vi",
                "/blog/pho-ha-noi",
            )
            .await
            .unwrap()
        else {
            panic!()
        };
        let view = content.public_views(&tenant, vec![*row]).await.unwrap().remove(0);
        let shown: Vec<(String, String)> =
            view.terms.iter().map(|term| (term.name.clone(), term.locale.clone())).collect();
        assert_eq!(
            shown,
            [("Du lịch".to_owned(), "vi".to_owned()), ("Food".to_owned(), "en".to_owned())]
        );
        let (_, by_slug) =
            content.term_by_slug(&tenant, Taxonomy::Category, "vi", "du-lich").await.unwrap();
        assert_eq!(by_slug.name, "Du lịch");

        assert_eq!(
            content.delete_term(&tenant, &ada, travel.term.id).await.unwrap_err().code(),
            "content.term_has_children"
        );
        content.delete_term(&tenant, &ada, tag.term.id).await.unwrap();
        assert_eq!(
            crate::taxonomy::terms::term_ids_of(tenant.database().read(), &tenant, &[entry.id])
                .await
                .unwrap()[&entry.id],
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
        let (content, media, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let profile =
            |name: &str| AuthorInput { display_name: Some(name.into()), ..Default::default() };
        let a = content.create_author(&tenant, &ada, profile("Ada Lovelace")).await.unwrap();
        let b = content.create_author(&tenant, &ada, profile("Trần Đạt")).await.unwrap();
        assert_eq!((a.slug.as_str(), b.slug.as_str()), ("ada-lovelace", "tran-dat"));
        let missing = AuthorInput { avatar_media_id: Some(Some(Uuid::now_v7())), ..profile("X") };
        assert_eq!(
            content.create_author(&tenant, &ada, missing).await.unwrap_err().code(),
            "content.media_not_found"
        );
        let avatar = media.add(true);
        let with_avatar = AuthorInput { avatar_media_id: Some(Some(avatar)), ..Default::default() };
        content.update_author(&tenant, &ada, a.id, with_avatar, Some(a.updated_at)).await.unwrap();

        let entry = content.create(&tenant, &ada, post("Co-written")).await.unwrap();
        content.set_entry_authors(&tenant, &ada, entry.id, vec![b.id, a.id]).await.unwrap();
        let filter = EntryFilter { author_id: Some(a.id), ..Default::default() };
        assert_eq!(
            content
                .list(&tenant, &ada, &filter, &PageRequest::first(10, Order::Asc))
                .await
                .unwrap()
                .data
                .len(),
            1
        );
        let refs =
            crate::taxonomy::authors::authors_of(tenant.database().read(), &tenant, &[entry.id])
                .await
                .unwrap();
        let names: Vec<&str> =
            refs[&entry.id].iter().map(|author| author.display_name.as_str()).collect();
        assert_eq!(names, ["Trần Đạt", "Ada Lovelace"]);
        assert_eq!(content.author_by_slug(&tenant, "tran-dat").await.unwrap().id, b.id);
        content.delete_author(&tenant, &ada, b.id).await.unwrap();
        let left =
            crate::taxonomy::authors::author_ids_of(tenant.database().read(), &tenant, &[entry.id])
                .await
                .unwrap();
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
        let (content, _, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let about = content.create(&tenant, &ada, page("About", None)).await.unwrap();
        let hidden = content.create(&tenant, &ada, page("Hidden", None)).await.unwrap();
        content.publish(&tenant, &ada, about.id, None, None, false).await.unwrap();
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
            content.put_menu(&tenant, &ada, "main", None, tree.clone(), None).await.unwrap();
        assert_eq!((menu.locale.as_str(), nodes.len(), nodes[0].children.len()), ("en", 2, 1));
        let stale = content
            .put_menu(
                &tenant,
                &ada,
                "main",
                None,
                tree.clone(),
                Some(menu.created_at - SignedDuration::from_secs(1)),
            )
            .await;
        assert_eq!(stale.unwrap_err().kind(), ErrorKind::PreconditionFailed);
        content.put_menu(&tenant, &ada, "main", None, tree, Some(menu.updated_at)).await.unwrap();

        let bad = vec![item("", None, None, vec![item("x", None, Some("javascript:x"), vec![])])];
        let error =
            content.put_menu(&tenant, &ada, "Main Menu", None, bad, None).await.unwrap_err();
        let fields: Vec<&str> = error.fields().iter().map(|field| field.field.as_str()).collect();
        assert_eq!(
            fields,
            ["key", "items[0].label", "items[0].entry_id", "items[0].children[0].url"]
        );

        let (shown, nodes) = content.public_menu(&tenant, "main", "vi").await.unwrap();
        assert_eq!(shown.locale, "en", "falls back to the default language");
        assert_eq!(nodes.len(), 1, "unpublished target hidden with its sub-items");
        assert_eq!(
            (nodes[0].href.as_str(), nodes[0].children[0].href.as_str()),
            ("/about", "https://example.com/docs")
        );
        assert_eq!(
            content.public_menu(&tenant, "footer", "en").await.unwrap_err().code(),
            "content.menu_not_found"
        );
    })
    .await
    .unwrap();
}

/// Sổ đối tượng `content.term` (D-238): tên theo ngôn ngữ (thiếu thì mặc định), đường dẫn công
/// khai, tra theo lô, danh mục con có giới hạn; `content.entry` trả ảnh đại diện.
#[tokio::test]
async fn so_doi_tuong_content_term() {
    use autoxide_kernel::resource::{ResourceRef, ResourceResolver};
    TestDatabase::run(async |test| {
        let (content, media, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let resolver = crate::resource::TermResolver(content.clone());
        let ada = editor();
        let travel = content
            .create_term(&tenant, &ada, Taxonomy::Category, name("en", "Travel"))
            .await
            .unwrap();
        content
            .update_term(
                &tenant,
                &ada,
                travel.term.id,
                name("vi", "Du lịch"),
                Some(travel.term.updated_at),
            )
            .await
            .unwrap();
        let mut ids = Vec::new();
        for label in ["Asia", "Europe", "Africa"] {
            let mut child = name("en", label);
            child.parent_id = Some(Some(travel.term.id));
            ids.push(
                content
                    .create_term(&tenant, &ada, Taxonomy::Category, child)
                    .await
                    .unwrap()
                    .term
                    .id,
            );
        }
        let food =
            content.create_term(&tenant, &ada, Taxonomy::Tag, name("en", "Food")).await.unwrap();

        let wanted = vec![travel.term.id.to_string(), food.term.id.to_string(), "x".to_owned()];
        let found = resolver.resolve_many(&tenant, &wanted, Some("vi")).await.unwrap();
        assert_eq!(found.len(), 2, "malformed IDs are absent");
        let travel_vi = &found[&travel.term.id.to_string()];
        assert_eq!(
            (travel_vi.public, travel_vi.title.as_str(), travel_vi.url.as_deref()),
            (true, "Du lịch", Some("/blog/category/du-lich"))
        );
        let food_vi = &found[&food.term.id.to_string()];
        assert_eq!(
            (food_vi.title.as_str(), food_vi.locale.as_deref(), food_vi.url.as_deref()),
            ("Food", Some("en"), Some("/blog/tag/food")),
            "falls back to the default language"
        );
        assert_eq!(
            resolver.resolve(&tenant, &food.term.id.to_string()).await.unwrap().unwrap().title,
            "Food"
        );
        assert!(resolver.resolve(&tenant, &Uuid::now_v7().to_string()).await.unwrap().is_none());

        assert!(resolver.has_children());
        let children =
            resolver.children(&tenant, &travel.term.id.to_string(), None, 2).await.unwrap();
        let expected: Vec<ResourceRef> =
            ids.iter().take(2).map(|id| ResourceRef::new("content.term", id.to_string())).collect();
        assert_eq!(children, expected, "creation order, cut at the limit");
        assert!(
            resolver
                .children(&tenant, &food.term.id.to_string(), None, 5)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(resolver.children(&tenant, "x", None, 5).await.unwrap().is_empty());

        let cover = media.add(true);
        let mut new = post("Cover");
        new.fields.featured_media_id = Some(Some(cover));
        let entry = content.create(&tenant, &ada, new).await.unwrap();
        content.publish(&tenant, &ada, entry.id, None, None, false).await.unwrap();
        let entries = crate::resource::EntryResolver;
        let found = entries.resolve_many(&tenant, &[entry.id.to_string()], None).await.unwrap();
        assert_eq!(found[&entry.id.to_string()].image, Some(cover));
    })
    .await
    .unwrap();
}

/// Menu cũ đọc được qua `LegacyMenus` cho module `menus` nhập sang (D-238): cây và đích giữ
/// nguyên.
#[tokio::test]
async fn doc_menu_cu() {
    use crate::api::{LegacyMenuItem, LegacyMenus, LegacyTarget};
    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let about = content.create(&tenant, &ada, page("About", None)).await.unwrap();
        let travel = content
            .create_term(&tenant, &ada, Taxonomy::Category, name("en", "Travel"))
            .await
            .unwrap();
        let tree = vec![
            ItemInput {
                label: "About".into(),
                entry_id: Some(about.id),
                children: vec![ItemInput {
                    label: "Docs".into(),
                    url: Some("/docs".into()),
                    ..ItemInput::default()
                }],
                ..ItemInput::default()
            },
            ItemInput {
                label: "Travel".into(),
                term_id: Some(travel.term.id),
                ..ItemInput::default()
            },
        ];
        content.put_menu(&tenant, &ada, "main", None, tree, None).await.unwrap();
        let menus = content.legacy_menus(&tenant).await.unwrap();
        assert_eq!(menus.len(), 1);
        assert_eq!((menus[0].key.as_str(), menus[0].locale.as_str()), ("main", "en"));
        assert_eq!(
            menus[0].items,
            [
                LegacyMenuItem {
                    label: "About".into(),
                    target: LegacyTarget::Entry(about.id),
                    children: vec![LegacyMenuItem {
                        label: "Docs".into(),
                        target: LegacyTarget::Url("/docs".into()),
                        children: vec![],
                    }],
                },
                LegacyMenuItem {
                    label: "Travel".into(),
                    target: LegacyTarget::Term(travel.term.id),
                    children: vec![],
                },
            ]
        );
    })
    .await
    .unwrap();
}
