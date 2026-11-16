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
        let filter = PostFilter {
            locale: "en".into(),
            site: autoxide_kernel::tenancy::Site::whole(&tenant).lineage_ids(),
            ..Default::default()
        };
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
        let vi_posts = PostFilter { locale: "vi".into(), ..filter.clone() };
        assert_eq!(content.public_posts(&tenant, &vi_posts, &first).await.unwrap().data.len(), 1);

        let all = content
            .sitemap(
                &tenant,
                &autoxide_kernel::tenancy::Site::whole(&tenant),
                None,
                &PageRequest::first(50, Order::Asc),
            )
            .await
            .unwrap()
            .data;
        assert_eq!(all.len(), 5, "3 posts, 1 page, 1 translation");
        assert!(all.iter().all(|row| row.entry_id != draft.id));
        let translations = content.translations_of(&tenant, &all).await.unwrap();
        assert_eq!(
            translations[&published[0].id],
            [("vi".to_owned(), "/blog/pho-ha-noi".to_owned())]
        );
        assert_eq!(
            content
                .sitemap(
                    &tenant,
                    &autoxide_kernel::tenancy::Site::whole(&tenant),
                    Some(&["vi".to_owned()]),
                    &PageRequest::first(50, Order::Asc)
                )
                .await
                .unwrap()
                .data
                .len(),
            1
        );

        let feed = content
            .feed(&tenant, &autoxide_kernel::tenancy::Site::whole(&tenant), "en", 2)
            .await
            .unwrap();
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

/// Nội dung theo website và nút (bước 34, D-208): website của cửa hàng A hiện bài của A và bài
/// của cả khách (nút cấp trên), không bao giờ hiện bài của cửa hàng B; người viết của B không
/// thấy, không sửa được bài của A.
#[tokio::test]
async fn bai_cua_cua_hang_a_khong_hien_tren_website_cua_cua_hang_b() {
    use autoxide_kernel::access::{Grant, Principal, Scope};
    use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, Site};

    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let org = || OrgId::from_uuid(Uuid::now_v7());
        let (shop_a, shop_b) = (org(), org());
        let write = database.begin_write().await.unwrap();
        let root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, shop_a, root, 8).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, shop_b, root, 8).await.unwrap();
        write.commit().await.unwrap();
        let ada = editor();
        let publish = async |title: &str, org: Option<OrgId>| {
            let mut new = post(title);
            new.org_id = org.map(|org| org.as_uuid());
            let entry = content.create(&tenant, &ada, new).await.unwrap();
            content.publish(&tenant, &ada, entry.id, None, None, false).await.unwrap();
            entry
        };
        let whole = publish("Whole chain", None).await;
        let of_a = publish("Shop A news", Some(shop_a)).await;
        let of_b = publish("Shop B news", Some(shop_b)).await;
        assert_eq!(of_a.org_id, Some(shop_a.as_uuid()));

        let titles = async |site: &Site| -> Vec<String> {
            let filter =
                PostFilter { locale: "en".into(), site: site.lineage_ids(), ..Default::default() };
            let page = PageRequest::first(10, Order::Asc);
            let rows = content.public_posts(&tenant, &filter, &page).await.unwrap().data;
            rows.into_iter().map(|row| row.title).collect()
        };
        let site_a = Site::of(&tenant, shop_a).await.unwrap();
        let site_b = Site::of(&tenant, shop_b).await.unwrap();
        assert_eq!(titles(&site_a).await, ["Whole chain", "Shop A news"]);
        assert_eq!(titles(&site_b).await, ["Whole chain", "Shop B news"]);
        assert_eq!(titles(&Site::whole(&tenant)).await, ["Whole chain"]);

        // Địa chỉ bài của A trên website của B: không có.
        let path_of = |entry: &crate::entries::Entry| format!("/blog/{}", entry.slug);
        let on_b = content.resolve(&tenant, &site_b, "en", &path_of(&of_a)).await;
        assert_eq!(on_b.unwrap_err().code(), "content.not_found");
        assert!(content.resolve(&tenant, &site_a, "en", &path_of(&of_a)).await.is_ok());
        assert!(content.resolve(&tenant, &site_b, "en", &path_of(&whole)).await.is_ok());
        let sitemap =
            content.sitemap(&tenant, &site_b, None, &PageRequest::first(10, Order::Asc)).await;
        assert_eq!(sitemap.unwrap().data.len(), 2);

        // Người viết của cửa hàng B: chỉ thấy bài của B; bài của A như không có.
        let writer_b = Principal::with_grants(
            Uuid::now_v7(),
            ["content.read", "content.write", "content.publish"]
                .map(|name| (name.to_owned(), Grant::new(shop_b, Scope::Subtree))),
        );
        let page = PageRequest::first(10, Order::Asc);
        let listed = content.list(&tenant, &writer_b, &Default::default(), &page).await.unwrap();
        let ids: Vec<Uuid> = listed.data.iter().map(|entry| entry.id).collect();
        assert_eq!(ids, [of_b.id]);
        assert_eq!(
            content.get(&tenant, &writer_b, of_a.id).await.unwrap_err().code(),
            "content.not_found"
        );
        let unpublish = content.unpublish(&tenant, &writer_b, of_a.id, None).await;
        assert_eq!(unpublish.unwrap_err().code(), "content.not_found");
        // Viết vào nút A: 403.
        let mut into_a = post("Not mine");
        into_a.org_id = Some(shop_a.as_uuid());
        assert_eq!(
            content.create(&tenant, &writer_b, into_a).await.unwrap_err().code(),
            "org_out_of_scope"
        );
        // Không nêu nút: nút duy nhất của người viết.
        let own = content.create(&tenant, &writer_b, post("Mine")).await.unwrap();
        assert_eq!(own.org_id, Some(shop_b.as_uuid()));
    })
    .await
    .unwrap();
}

/// `OrgDirectory` giả: chỉ trả ngôn ngữ của website đã khai cho từng nút (D-307).
struct SiteLanguages(std::collections::HashMap<autoxide_kernel::tenancy::OrgId, Vec<&'static str>>);

#[async_trait::async_trait]
impl autoxide_organizations::api::OrgDirectory for SiteLanguages {
    async fn unit(
        &self,
        _: &TenantContext,
        _: autoxide_kernel::tenancy::OrgId,
    ) -> Result<Option<autoxide_organizations::api::Unit>, autoxide_kernel::error::Error> {
        Ok(None)
    }
    async fn root(
        &self,
        _: &TenantContext,
    ) -> Result<autoxide_organizations::api::Unit, autoxide_kernel::error::Error> {
        Err(autoxide_kernel::error::Error::internal("not used"))
    }
    async fn ancestors(
        &self,
        _: &TenantContext,
        _: autoxide_kernel::tenancy::OrgId,
    ) -> Result<Vec<autoxide_organizations::api::Unit>, autoxide_kernel::error::Error> {
        Ok(Vec::new())
    }
    async fn is_within(
        &self,
        _: &TenantContext,
        _: autoxide_kernel::tenancy::OrgId,
        _: autoxide_kernel::tenancy::OrgId,
    ) -> Result<bool, autoxide_kernel::error::Error> {
        Ok(false)
    }
    async fn setting(
        &self,
        _: &TenantContext,
        id: autoxide_kernel::tenancy::OrgId,
        key: &str,
    ) -> Result<Option<autoxide_organizations::api::EffectiveSetting>, autoxide_kernel::error::Error>
    {
        assert_eq!(key, "organizations.languages");
        let value = self.0.get(&id).map(|locales| json!(locales));
        Ok(Some(autoxide_organizations::api::EffectiveSetting::new(key, value, Some(id), false)))
    }
}

/// Đường dẫn và ngôn ngữ theo website (D-307): cửa hàng A và cả chuỗi cùng có `/about` — website
/// A thấy trang của A, website B thấy trang của chuỗi, sitemap không lặp đường dẫn; website A chỉ
/// đưa tiếng Việt thì tiếng Anh bị từ chối trên A nhưng vẫn chạy trên B.
#[tokio::test]
async fn duong_dan_va_ngon_ngu_theo_website() {
    use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, Site};

    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let org = || OrgId::from_uuid(Uuid::now_v7());
        let (shop_a, shop_b) = (org(), org());
        let content = std::sync::Arc::new(std::sync::Arc::into_inner(content).unwrap().with_orgs(
            Some(std::sync::Arc::new(SiteLanguages(
                [(shop_a, vec!["vi", "fr"])].into_iter().collect(),
            ))),
        ));
        let tenant = TenantContext::default_on(database.clone());
        let write = database.begin_write().await.unwrap();
        let root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, shop_a, root, 8).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, shop_b, root, 8).await.unwrap();
        write.commit().await.unwrap();
        let ada = editor();
        let about = async |org: Option<OrgId>, locale: &str| {
            let mut new = page("About", None);
            new.org_id = org.map(|org| org.as_uuid());
            new.locale = Some(locale.to_owned());
            let entry = content.create(&tenant, &ada, new).await.unwrap();
            content.publish(&tenant, &ada, entry.id, None, None, false).await.unwrap();
            entry
        };
        let chain = about(None, "vi").await;
        let of_a = about(Some(shop_a), "vi").await;
        assert_eq!(of_a.slug, "about", "website khác: cùng slug được");
        // Cùng website (cả chuỗi) thì slug tự sinh tránh trùng.
        let mut again = page("About", None);
        again.locale = Some("vi".to_owned());
        assert_eq!(content.create(&tenant, &ada, again).await.unwrap().slug, "about-2");

        let site_a = Site::of(&tenant, shop_a).await.unwrap();
        let site_b = Site::of(&tenant, shop_b).await.unwrap();
        let shown = |resolved: crate::publishing::redirects::Resolved| match resolved {
            crate::publishing::redirects::Resolved::Entry(row) => row.entry_id,
            _ => panic!("không phải bài"),
        };
        let on_a = content.resolve(&tenant, &site_a, "vi", "/about").await.unwrap();
        assert_eq!(shown(on_a), of_a.id, "nút gần website nhất thắng");
        let on_b = content.resolve(&tenant, &site_b, "vi", "/about").await.unwrap();
        assert_eq!(shown(on_b), chain.id);
        let page_of = PageRequest::first(10, Order::Asc);
        let map = content.sitemap(&tenant, &site_a, None, &page_of).await.unwrap().data;
        let ids: Vec<Uuid> = map.iter().map(|row| row.entry_id).collect();
        assert_eq!(ids, [of_a.id], "trang của chuỗi bị che trên website A");

        // Ngôn ngữ: A đưa `vi`, `fr` (máy chủ chỉ bật `en`, `vi`) → còn `vi`, mặc định `vi`.
        let languages = content.site_languages(&tenant, &site_a).await.unwrap();
        assert_eq!(
            (languages.locales.clone(), languages.default.clone()),
            (vec!["vi".to_owned()], "vi".to_owned())
        );
        let refused = content.resolve(&tenant, &site_a, "en", "/about").await.unwrap_err();
        assert_eq!(refused.code(), "validation_failed");
        let b_languages = content.site_languages(&tenant, &site_b).await.unwrap();
        assert_eq!(b_languages.locales, ["en", "vi"], "B không đặt: ngôn ngữ của khách");
    })
    .await
    .unwrap();
}
