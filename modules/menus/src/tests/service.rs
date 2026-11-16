//! Test service: giải đối tượng, bỏ đối tượng không công khai, nhóm con có giới hạn, nhãn có
//! hạn, `If-Match` đồng thời, nhập menu cũ hai lần, ngôn ngữ mặc định, cache; hai khách cùng
//! database không thấy menu hay cache của nhau (bước 33).

use autoxide_content::api::{LegacyMenu, LegacyMenuItem, LegacyTarget};
use autoxide_kernel::error::ErrorKind;
use autoxide_testkit::TestDatabase;
use chrono::{Duration, Utc};
use uuid::Uuid;

use super::{PUBLIC_IMAGE, admin, harness};
use crate::items::{BadgeInput, ItemInput, RefInput};
use crate::public::PublicSiteMenuItem;
use crate::service::NewMenu;

fn link(label: &str, url: &str) -> ItemInput {
    ItemInput { label: Some(label.into()), url: Some(url.into()), ..ItemInput::default() }
}

fn to(kind: &str, id: &str) -> Option<RefInput> {
    Some(RefInput { kind: kind.into(), id: id.into() })
}

fn object(kind: &str, id: &str) -> ItemInput {
    ItemInput { target: to(kind, id), ..ItemInput::default() }
}

fn menu(key: &str, locale: &str, items: Vec<ItemInput>) -> NewMenu {
    NewMenu { key: key.into(), locale: Some(locale.into()), org_id: None, items }
}

fn labels(items: &[PublicSiteMenuItem]) -> Vec<&str> {
    items.iter().map(|item| item.label.as_str()).collect()
}

/// Mục trỏ đối tượng lấy tiêu đề, đường dẫn theo ngôn ngữ của menu; đối tượng không công khai
/// bị bỏ cùng mục con; nhóm không còn con bị bỏ; ô gợi ý có ảnh chỉ khi ảnh công khai.
#[tokio::test]
async fn giai_doi_tuong_va_bo_doi_tuong_khong_cong_khai() {
    TestDatabase::run(async |test| {
        let harness = harness(test).await;
        let (service, t) = (&harness.service, &harness.tenant);
        let items = vec![
            object("shop.item", "p1"),
            ItemInput { label: Some("Named".into()), ..object("shop.item", "p2") },
            ItemInput { children: vec![link("Under hidden", "/x")], ..object("shop.item", "h1") },
            ItemInput {
                label: Some("Group".into()),
                children: vec![object("shop.item", "h2")],
                ..ItemInput::default()
            },
            ItemInput {
                label: Some("Column".into()),
                children: vec![link("Kept", "/kept")],
                feature: to("shop.item", "p-img"),
                ..ItemInput::default()
            },
            ItemInput {
                feature: to("shop.item", "p-private"),
                icon: Some("tag".into()),
                highlight: Some("sale".into()),
                ..link("Sale", "/sale")
            },
        ];
        service.create(t, &admin(), menu("main", "vi", items)).await.unwrap();
        let ready = service
            .public_menu(
                t,
                &autoxide_kernel::tenancy::Site::whole(t),
                "main",
                Some("vi"),
                Utc::now(),
            )
            .await
            .unwrap();
        let shown = &ready.menu.items;
        assert_eq!(labels(shown), ["p1@vi", "Named", "Column", "Sale"]);
        assert_eq!(shown[0].href.as_deref(), Some("/items/p1"));
        let target = shown[0].target.as_ref().unwrap();
        assert_eq!((target.kind.as_str(), target.id.as_str()), ("shop.item", "p1"));
        assert_eq!(shown[2].href, None, "a heading has no link");
        let feature = shown[2].feature.as_ref().unwrap();
        assert_eq!(
            (feature.title.as_str(), feature.href.as_deref()),
            ("p-img@vi", Some("/items/p-img"))
        );
        let image = feature.image.as_ref().unwrap();
        assert_eq!((image.alt.as_str(), image.width), ("[vi]", Some(640)));
        assert!(image.url.contains(&PUBLIC_IMAGE.to_string()));
        let sale = &shown[3];
        assert_eq!((sale.icon.as_deref(), sale.highlight.as_deref()), (Some("tag"), Some("sale")));
        assert!(sale.feature.as_ref().unwrap().image.is_none(), "private pictures are not shown");
    })
    .await
    .unwrap();
}

/// Lỗi đích gom một lần, có mã riêng khi mọi lỗi cùng loại (D-083).
#[tokio::test]
async fn loi_dich_gom_mot_lan() {
    TestDatabase::run(async |test| {
        let harness = harness(test).await;
        let (service, t) = (&harness.service, &harness.tenant);
        let error = service
            .create(
                t,
                &admin(),
                menu("main", "en", vec![object("shop.item", "x1"), object("shop.item", "x2")]),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code(), "menus.ref_not_found");
        let fields: Vec<&str> = error.fields().iter().map(|field| field.field.as_str()).collect();
        assert_eq!(fields, ["items[0].ref.id", "items[1].ref.id"]);

        let error = service
            .create(t, &admin(), menu("main", "en", vec![object("forms.form", "1")]))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "menus.ref_unknown_kind");

        let unsupported = ItemInput { auto_children: true, ..object("shop.item", "p1") };
        let error =
            service.create(t, &admin(), menu("main", "en", vec![unsupported])).await.unwrap_err();
        assert_eq!(error.code(), "menus.children_unsupported");
        assert_eq!(error.fields()[0].field, "items[0].auto_children");

        let mixed = vec![object("shop.item", "x1"), link("", "/x")];
        let error = service.create(t, &admin(), menu("main", "en", mixed)).await.unwrap_err();
        assert_eq!(error.code(), "validation_failed");
        assert_eq!(error.fields().len(), 2);

        let feature = ItemInput { feature: to("shop.item", "x9"), ..link("A", "/a") };
        let error =
            service.create(t, &admin(), menu("main", "en", vec![feature])).await.unwrap_err();
        assert_eq!(error.fields()[0].field, "items[0].feature.id");

        let reader =
            autoxide_kernel::access::Principal::new(Uuid::now_v7(), ["menus.read".to_owned()]);
        let error = service.create(t, &reader, menu("main", "en", vec![])).await.unwrap_err();
        assert_eq!(error.code(), "permission_denied");

        service.create(t, &admin(), menu("main", "en", vec![])).await.unwrap();
        let error = service.create(t, &admin(), menu("main", "en", vec![])).await.unwrap_err();
        assert_eq!(error.code(), "menus.menu_exists");
    })
    .await
    .unwrap();
}

/// `auto_children` mở nhóm con qua sổ đối tượng, sau mục con tự thêm, có giới hạn; con không
/// công khai bị bỏ (giới hạn tính trước khi lọc).
#[tokio::test]
async fn nhom_con_tu_lay_co_gioi_han() {
    TestDatabase::run(async |test| {
        let harness = harness(test).await;
        let (service, t) = (&harness.service, &harness.tenant);
        let items = vec![ItemInput {
            label: Some("Shop".into()),
            auto_children: true,
            auto_children_limit: Some(3),
            children: vec![link("All", "/all")],
            ..object("shop.category", "c")
        }];
        service.create(t, &admin(), menu("main", "en", items)).await.unwrap();
        let ready = service
            .public_menu(t, &autoxide_kernel::tenancy::Site::whole(t), "main", None, Utc::now())
            .await
            .unwrap();
        let shop = &ready.menu.items[0];
        assert_eq!(labels(&shop.children), ["All", "Category c1", "Category c2"]);
        assert_eq!(shop.children[1].href.as_deref(), Some("/c/c1"));
        assert_eq!(shop.children[1].target.as_ref().unwrap().kind, "shop.category");
    })
    .await
    .unwrap();
}

/// Nhãn nổi bật chỉ hiện trong hạn; cache hết hạn đúng lúc nhãn bật hay tắt.
#[tokio::test]
async fn nhan_het_han_tu_an() {
    TestDatabase::run(async |test| {
        let harness = harness(test).await;
        let (service, t) = (&harness.service, &harness.tenant);
        let now = crate::items::micros(Utc::now());
        let badge = |from: Option<i64>, until: Option<i64>| {
            Some(BadgeInput {
                text: "-50%".into(),
                tone: "sale".into(),
                from: from.map(|hours| now + Duration::hours(hours)),
                until: until.map(|hours| now + Duration::hours(hours)),
            })
        };
        let items = vec![
            ItemInput { badge: badge(None, None), ..link("Always", "/a") },
            ItemInput { badge: badge(Some(-2), Some(-1)), ..link("Over", "/b") },
            ItemInput { badge: badge(Some(2), None), ..link("Later", "/c") },
            ItemInput { badge: badge(Some(-1), Some(1)), ..link("Now", "/d") },
        ];
        service.create(t, &admin(), menu("main", "en", items)).await.unwrap();
        let ready = service
            .public_menu(t, &autoxide_kernel::tenancy::Site::whole(t), "main", None, now)
            .await
            .unwrap();
        let shown: Vec<bool> = ready.menu.items.iter().map(|item| item.badge.is_some()).collect();
        assert_eq!(shown, [true, false, false, true]);
        assert_eq!(ready.expires_at, now + Duration::seconds(60), "the next change is 1 hour away");

        // Một giờ sau: "Now" tắt, nên bản đã giải chỉ dùng được tới đúng lúc đó.
        service.cache.clear(t.id());
        let later = now + Duration::minutes(59) + Duration::seconds(30);
        let ready = service
            .public_menu(t, &autoxide_kernel::tenancy::Site::whole(t), "main", None, later)
            .await
            .unwrap();
        assert_eq!(ready.expires_at, now + Duration::hours(1));
        let after = service
            .public_menu(
                t,
                &autoxide_kernel::tenancy::Site::whole(t),
                "main",
                None,
                now + Duration::hours(1),
            )
            .await
            .unwrap();
        let shown: Vec<bool> = after.menu.items.iter().map(|item| item.badge.is_some()).collect();
        assert_eq!(shown, [true, false, false, false], "expired badges hide by themselves");
        assert_ne!(after.etag, ready.etag);
    })
    .await
    .unwrap();
}

/// Hai người cùng thay mục từ cùng một phiên bản: đúng một người được, người kia nhận 412.
#[tokio::test]
async fn if_match_dong_thoi() {
    TestDatabase::run(async |test| {
        let harness = harness(test).await;
        let (service, t) = (&harness.service, &harness.tenant);
        let (saved, _) = service.create(t, &admin(), menu("main", "en", vec![])).await.unwrap();
        let (ada, bob) = (admin(), admin());
        let (first, second) = tokio::join!(
            service.replace(t, &ada, saved.id, vec![link("Ada", "/ada")], Some(saved.updated_at)),
            service.replace(t, &bob, saved.id, vec![link("Bob", "/bob")], Some(saved.updated_at)),
        );
        let results = [first, second];
        let ok = results.iter().filter(|result| result.is_ok()).count();
        let stale = results
            .iter()
            .filter(|result| {
                result.as_ref().is_err_and(|error| error.kind() == ErrorKind::PreconditionFailed)
            })
            .count();
        assert_eq!((ok, stale), (1, 1));
        let (_, nodes) = service.get(t, &admin(), saved.id).await.unwrap();
        assert_eq!(nodes.len(), 1, "only the winner's items are saved");

        // `*` (không kiểm phiên bản) vẫn ghi; menu không còn thì 404.
        service.replace(t, &ada, saved.id, vec![], None).await.unwrap();
        service.delete(t, &ada, saved.id).await.unwrap();
        let gone = service.replace(t, &ada, saved.id, vec![], None).await.unwrap_err();
        assert_eq!(gone.code(), "menus.menu_not_found");
        assert_eq!(
            service.delete(t, &ada, saved.id).await.unwrap_err().code(),
            "menus.menu_not_found"
        );
    })
    .await
    .unwrap();
}

/// Thiếu menu ở ngôn ngữ hỏi thì dùng ngôn ngữ mặc định; ngôn ngữ không hỗ trợ là 422; khoá
/// không có là 404; sửa menu thì bản công khai đổi ngay (xoá cache).
#[tokio::test]
async fn ngon_ngu_mac_dinh_va_cache() {
    TestDatabase::run(async |test| {
        let harness = harness(test).await;
        let (service, t) = (&harness.service, &harness.tenant);
        let (saved, _) =
            service.create(t, &admin(), menu("main", "en", vec![link("Home", "/")])).await.unwrap();
        let now = Utc::now();
        let ready = service
            .public_menu(t, &autoxide_kernel::tenancy::Site::whole(t), "main", Some("VI"), now)
            .await
            .unwrap();
        assert_eq!(ready.menu.locale, "en", "falls back to the default language");
        let error = service
            .public_menu(t, &autoxide_kernel::tenancy::Site::whole(t), "main", Some("fr"), now)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Validation);
        let error = service
            .public_menu(t, &autoxide_kernel::tenancy::Site::whole(t), "nope", None, now)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "menus.public_menu_not_found");

        let again = service
            .public_menu(t, &autoxide_kernel::tenancy::Site::whole(t), "main", None, now)
            .await
            .unwrap();
        service
            .replace(t, &admin(), saved.id, vec![link("Start", "/")], Some(saved.updated_at))
            .await
            .unwrap();
        let changed = service
            .public_menu(t, &autoxide_kernel::tenancy::Site::whole(t), "main", None, now)
            .await
            .unwrap();
        assert_eq!(labels(&changed.menu.items), ["Start"]);
        assert_ne!(changed.etag, again.etag);

        service
            .create(t, &admin(), menu("main", "vi", vec![link("Trang chủ", "/")]))
            .await
            .unwrap();
        let vietnamese = service
            .public_menu(t, &autoxide_kernel::tenancy::Site::whole(t), "main", Some("vi"), now)
            .await
            .unwrap();
        assert_eq!(vietnamese.menu.locale, "vi");
    })
    .await
    .unwrap();
}

/// Nhập menu cũ của `content`: đích đổi sang sổ đối tượng; chạy lại không nhân đôi và không đè
/// menu đã sửa; ngôn ngữ không hỗ trợ thì báo lỗi, không dừng cả lần nhập.
#[tokio::test]
async fn nhap_menu_cu_hai_lan_khong_nhan_doi() {
    TestDatabase::run(async |test| {
        let harness = harness(test).await;
        let (service, t) = (&harness.service, &harness.tenant);
        let entry = Uuid::now_v7();
        let term = Uuid::now_v7();
        let old = |key: &str, locale: &str| LegacyMenu {
            key: key.into(),
            locale: locale.into(),
            items: vec![
                LegacyMenuItem {
                    label: "About".into(),
                    target: LegacyTarget::Entry(entry),
                    children: vec![LegacyMenuItem {
                        label: "Docs".into(),
                        target: LegacyTarget::Url("/docs".into()),
                        children: vec![],
                    }],
                },
                LegacyMenuItem {
                    label: "Travel".into(),
                    target: LegacyTarget::Term(term),
                    children: vec![],
                },
            ],
        };
        *harness.legacy.0.lock().unwrap() =
            vec![old("main", "en"), old("footer", "vi"), old("main", "fr")];

        let report = service.import_content_menus(t).await.unwrap();
        assert_eq!(report.imported.len(), 2);
        assert_eq!(report.failed.len(), 1, "`fr` is not a menus language");
        let first = service
            .find_by_key(t, &autoxide_kernel::tenancy::Site::whole(t), "main", "en")
            .await
            .unwrap()
            .unwrap();
        let (_, nodes) = service.get(t, &admin(), first.id).await.unwrap();
        let about = &nodes[0].item;
        assert_eq!(
            (about.label.as_str(), about.ref_kind.as_deref(), about.ref_id.clone()),
            ("About", Some("content.entry"), Some(entry.to_string()))
        );
        assert_eq!(nodes[0].children[0].item.url.as_deref(), Some("/docs"));
        assert_eq!(nodes[1].item.ref_kind.as_deref(), Some("content.term"));

        // Sửa menu đã nhập rồi nhập lại: không thêm, không đè.
        service.replace(t, &admin(), first.id, vec![link("Mine", "/")], None).await.unwrap();
        let report = service.import_content_menus(t).await.unwrap();
        assert_eq!((report.imported.len(), report.skipped.len()), (0, 2));
        let page = autoxide_kernel::pagination::PageRequest::try_from(
            autoxide_kernel::pagination::PageQuery::default(),
        )
        .unwrap();
        assert_eq!(service.list(t, &admin(), None, &page).await.unwrap().data.len(), 2);
        let (_, nodes) = service.get(t, &admin(), first.id).await.unwrap();
        assert_eq!(nodes[0].item.label, "Mine");
    })
    .await
    .unwrap();
}

/// Hai khách trên cùng database (bước 33), cùng menu `main` ở `en`: menu công khai của mỗi khách
/// chỉ có mục của khách đó — kể cả khi menu của khách kia đã nằm trong cache, và sau khi một khách
/// sửa menu (chỉ bản cache của khách đó đổi). Đọc, sửa, xoá theo ID không vượt khách; nhập menu cũ
/// theo từng khách.
#[tokio::test]
async fn hai_khach_cung_khoa_khong_lo_menu_va_cache() {
    TestDatabase::run(async |test| {
        use autoxide_kernel::tenancy::{MAIN_PLACEMENT, TenantContext, TenantId, TenantStatus};

        let harness = harness(test).await;
        let (service, a) = (&harness.service, &harness.tenant);
        let b = &TenantContext::new(
            TenantId::from_uuid(Uuid::now_v7()),
            "other",
            "Other",
            TenantStatus::Active,
            MAIN_PLACEMENT,
            harness.database.clone(),
        );
        let (mine, _) = service
            .create(a, &admin(), menu("main", "en", vec![link("Home A", "/")]))
            .await
            .unwrap();
        let (theirs, _) = service
            .create(b, &admin(), menu("main", "en", vec![link("Home B", "/")]))
            .await
            .unwrap();
        let now = Utc::now();

        // Khách A đọc trước (vào cache), rồi khách B: mỗi khách menu của mình.
        let cached_a = service
            .public_menu(a, &autoxide_kernel::tenancy::Site::whole(a), "main", None, now)
            .await
            .unwrap();
        assert_eq!(labels(&cached_a.menu.items), ["Home A"]);
        let cached_b = service
            .public_menu(b, &autoxide_kernel::tenancy::Site::whole(b), "main", None, now)
            .await
            .unwrap();
        assert_eq!(labels(&cached_b.menu.items), ["Home B"]);
        assert_ne!(cached_a.etag, cached_b.etag);
        let again_b = service
            .public_menu(b, &autoxide_kernel::tenancy::Site::whole(b), "main", None, now)
            .await
            .unwrap();
        assert!(std::sync::Arc::ptr_eq(&cached_b, &again_b), "served from the cache of B");

        // Khách B sửa menu: bản của B đổi, bản đã cache của A giữ nguyên (không bị xoá).
        let items = vec![link("New B", "/new")];
        service.replace(b, &admin(), theirs.id, items, Some(theirs.updated_at)).await.unwrap();
        let changed_b = service
            .public_menu(b, &autoxide_kernel::tenancy::Site::whole(b), "main", None, now)
            .await
            .unwrap();
        assert_eq!(labels(&changed_b.menu.items), ["New B"]);
        let still_a = service
            .public_menu(a, &autoxide_kernel::tenancy::Site::whole(a), "main", None, now)
            .await
            .unwrap();
        assert!(std::sync::Arc::ptr_eq(&cached_a, &still_a), "A's cached copy is kept");
        assert_eq!(labels(&still_a.menu.items), ["Home A"]);

        // Hai khách có menu giống hệt nhau vẫn nhận hai `ETag` khác nhau.
        for tenant in [a, b] {
            service
                .create(tenant, &admin(), menu("footer", "en", vec![link("Hi", "/hi")]))
                .await
                .unwrap();
        }
        let footer_a = service
            .public_menu(a, &autoxide_kernel::tenancy::Site::whole(a), "footer", None, now)
            .await
            .unwrap();
        let footer_b = service
            .public_menu(b, &autoxide_kernel::tenancy::Site::whole(b), "footer", None, now)
            .await
            .unwrap();
        assert_eq!(footer_a.menu, footer_b.menu);
        assert_ne!(footer_a.etag, footer_b.etag);

        // Theo ID: khách B không đọc, sửa hay xoá được menu của khách A.
        let page = autoxide_kernel::pagination::PageRequest::try_from(
            autoxide_kernel::pagination::PageQuery::default(),
        )
        .unwrap();
        let listed = service.list(b, &admin(), None, &page).await.unwrap();
        assert!(listed.data.iter().all(|(menu, _)| menu.tenant_id == b.id().as_uuid()));
        assert_eq!(listed.data.len(), 2);
        let missing = "menus.menu_not_found";
        assert_eq!(service.get(b, &admin(), mine.id).await.unwrap_err().code(), missing);
        let replaced = service.replace(b, &admin(), mine.id, vec![], None).await;
        assert_eq!(replaced.unwrap_err().code(), missing);
        assert_eq!(service.delete(b, &admin(), mine.id).await.unwrap_err().code(), missing);
        let (_, nodes) = service.get(a, &admin(), mine.id).await.unwrap();
        assert_eq!(nodes[0].item.label, "Home A");

        // Nhập menu cũ: mỗi khách một bản, chạy lại ở một khách không nhân đôi.
        *harness.legacy.0.lock().unwrap() = vec![LegacyMenu {
            key: "old".into(),
            locale: "en".into(),
            items: vec![LegacyMenuItem {
                label: "Docs".into(),
                target: LegacyTarget::Url("/docs".into()),
                children: vec![],
            }],
        }];
        for tenant in [a, b] {
            assert_eq!(service.import_content_menus(tenant).await.unwrap().imported.len(), 1);
        }
        let report = service.import_content_menus(a).await.unwrap();
        assert_eq!((report.imported.len(), report.skipped.len()), (0, 1));
        let old_b = service
            .find_by_key(b, &autoxide_kernel::tenancy::Site::whole(b), "old", "en")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(old_b.tenant_id, b.id().as_uuid());
    })
    .await
    .unwrap();
}

/// Lệnh `menus.import_content_menus`: không nêu khách thì nhập vào khách mặc định; khách không có
/// thì báo lỗi, không nhập gì.
#[tokio::test]
async fn lenh_nhap_theo_khach() {
    TestDatabase::run(async |test| {
        let harness = harness(test).await;
        *harness.legacy.0.lock().unwrap() =
            vec![LegacyMenu { key: "old".into(), locale: "en".into(), items: vec![] }];
        let registry = autoxide_kernel::tenancy::DatabaseRegistry::single(harness.database.clone());
        let task = crate::legacy::task(harness.service.clone(), registry);
        let unknown = task.parse(&["tenant=nope".to_owned()]).unwrap();
        assert_eq!(task.run(&unknown).await.unwrap_err().code(), "tenant_not_found");
        let report = task.run(&task.parse(&[]).unwrap()).await.unwrap();
        assert!(report.starts_with("Tenant `default`: imported 1 menu(s)"), "{report}");
        let found = harness
            .service
            .find_by_key(
                &harness.tenant,
                &autoxide_kernel::tenancy::Site::whole(&harness.tenant),
                "old",
                "en",
            )
            .await
            .unwrap();
        assert!(found.is_some());
    })
    .await
    .unwrap();
}

/// `OrgDirectory` giả: ngôn ngữ của website đã khai cho từng nút (D-307).
struct SiteLanguages(autoxide_kernel::tenancy::OrgId, Vec<&'static str>);

#[async_trait::async_trait]
impl autoxide_organizations::api::OrgDirectory for SiteLanguages {
    async fn unit(
        &self,
        _: &autoxide_kernel::tenancy::TenantContext,
        _: autoxide_kernel::tenancy::OrgId,
    ) -> Result<Option<autoxide_organizations::api::Unit>, autoxide_kernel::error::Error> {
        Ok(None)
    }
    async fn root(
        &self,
        _: &autoxide_kernel::tenancy::TenantContext,
    ) -> Result<autoxide_organizations::api::Unit, autoxide_kernel::error::Error> {
        Err(autoxide_kernel::error::Error::internal("not used"))
    }
    async fn ancestors(
        &self,
        _: &autoxide_kernel::tenancy::TenantContext,
        _: autoxide_kernel::tenancy::OrgId,
    ) -> Result<Vec<autoxide_organizations::api::Unit>, autoxide_kernel::error::Error> {
        Ok(Vec::new())
    }
    async fn is_within(
        &self,
        _: &autoxide_kernel::tenancy::TenantContext,
        _: autoxide_kernel::tenancy::OrgId,
        _: autoxide_kernel::tenancy::OrgId,
    ) -> Result<bool, autoxide_kernel::error::Error> {
        Ok(false)
    }
    async fn setting(
        &self,
        _: &autoxide_kernel::tenancy::TenantContext,
        id: autoxide_kernel::tenancy::OrgId,
        key: &str,
    ) -> Result<Option<autoxide_organizations::api::EffectiveSetting>, autoxide_kernel::error::Error>
    {
        let value = (id == self.0).then(|| serde_json::json!(self.1));
        Ok(Some(autoxide_organizations::api::EffectiveSetting::new(key, value, Some(id), false)))
    }
}

/// Website chỉ đưa tiếng Việt (D-307): menu công khai mặc định tiếng Việt, tiếng Anh bị từ chối
/// trên website đó; website của cả khách vẫn theo cấu hình máy chủ.
#[tokio::test]
async fn ngon_ngu_cua_menu_theo_website() {
    use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, Site};
    TestDatabase::run(async |test| {
        let harness = harness(test).await;
        let shop = OrgId::from_uuid(Uuid::now_v7());
        let mut service = std::sync::Arc::try_unwrap(harness.service).ok().unwrap();
        service.orgs = Some(std::sync::Arc::new(SiteLanguages(shop, vec!["vi"])));
        let t = &harness.tenant;
        let write = harness.database.begin_write().await.unwrap();
        let root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, shop, root, 8).await.unwrap();
        write.commit().await.unwrap();
        for locale in ["en", "vi"] {
            service
                .create(t, &admin(), menu("main", locale, vec![link(locale, "/")]))
                .await
                .unwrap();
        }
        let site = Site::of(t, shop).await.unwrap();
        let ready = service.public_menu(t, &site, "main", None, Utc::now()).await.unwrap();
        assert_eq!(ready.menu.locale, "vi", "mặc định của website");
        let refused = service.public_menu(t, &site, "main", Some("en"), Utc::now()).await;
        assert_eq!(refused.unwrap_err().kind(), ErrorKind::Validation);
        let whole = service.public_menu(t, &Site::whole(t), "main", None, Utc::now()).await;
        assert_eq!(whole.unwrap().menu.locale, "en");
    })
    .await
    .unwrap();
}
