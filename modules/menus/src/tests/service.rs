//! Test service: giải đối tượng, bỏ đối tượng không công khai, nhóm con có giới hạn, nhãn có
//! hạn, `If-Match` đồng thời, nhập menu cũ hai lần, ngôn ngữ mặc định, cache.

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
    NewMenu { key: key.into(), locale: Some(locale.into()), items }
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
        let service = &harness.service;
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
        service.create(&admin(), menu("main", "vi", items)).await.unwrap();
        let ready = service.public_menu("main", Some("vi"), Utc::now()).await.unwrap();
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
        let service = &harness.service;
        let error = service
            .create(
                &admin(),
                menu("main", "en", vec![object("shop.item", "x1"), object("shop.item", "x2")]),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code(), "menus.ref_not_found");
        let fields: Vec<&str> = error.fields().iter().map(|field| field.field.as_str()).collect();
        assert_eq!(fields, ["items[0].ref.id", "items[1].ref.id"]);

        let error = service
            .create(&admin(), menu("main", "en", vec![object("forms.form", "1")]))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "menus.ref_unknown_kind");

        let unsupported = ItemInput { auto_children: true, ..object("shop.item", "p1") };
        let error =
            service.create(&admin(), menu("main", "en", vec![unsupported])).await.unwrap_err();
        assert_eq!(error.code(), "menus.children_unsupported");
        assert_eq!(error.fields()[0].field, "items[0].auto_children");

        let mixed = vec![object("shop.item", "x1"), link("", "/x")];
        let error = service.create(&admin(), menu("main", "en", mixed)).await.unwrap_err();
        assert_eq!(error.code(), "validation_failed");
        assert_eq!(error.fields().len(), 2);

        let feature = ItemInput { feature: to("shop.item", "x9"), ..link("A", "/a") };
        let error = service.create(&admin(), menu("main", "en", vec![feature])).await.unwrap_err();
        assert_eq!(error.fields()[0].field, "items[0].feature.id");

        let reader =
            autoxide_kernel::access::Principal::new(Uuid::now_v7(), ["menus.read".to_owned()]);
        let error = service.create(&reader, menu("main", "en", vec![])).await.unwrap_err();
        assert_eq!(error.code(), "permission_denied");

        service.create(&admin(), menu("main", "en", vec![])).await.unwrap();
        let error = service.create(&admin(), menu("main", "en", vec![])).await.unwrap_err();
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
        let service = &harness.service;
        let items = vec![ItemInput {
            label: Some("Shop".into()),
            auto_children: true,
            auto_children_limit: Some(3),
            children: vec![link("All", "/all")],
            ..object("shop.category", "c")
        }];
        service.create(&admin(), menu("main", "en", items)).await.unwrap();
        let ready = service.public_menu("main", None, Utc::now()).await.unwrap();
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
        let service = &harness.service;
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
        service.create(&admin(), menu("main", "en", items)).await.unwrap();
        let ready = service.public_menu("main", None, now).await.unwrap();
        let shown: Vec<bool> = ready.menu.items.iter().map(|item| item.badge.is_some()).collect();
        assert_eq!(shown, [true, false, false, true]);
        assert_eq!(ready.expires_at, now + Duration::seconds(60), "the next change is 1 hour away");

        // Một giờ sau: "Now" tắt, nên bản đã giải chỉ dùng được tới đúng lúc đó.
        service.cache.clear();
        let later = now + Duration::minutes(59) + Duration::seconds(30);
        let ready = service.public_menu("main", None, later).await.unwrap();
        assert_eq!(ready.expires_at, now + Duration::hours(1));
        let after = service.public_menu("main", None, now + Duration::hours(1)).await.unwrap();
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
        let service = &harness.service;
        let (saved, _) = service.create(&admin(), menu("main", "en", vec![])).await.unwrap();
        let (ada, bob) = (admin(), admin());
        let (first, second) = tokio::join!(
            service.replace(&ada, saved.id, vec![link("Ada", "/ada")], Some(saved.updated_at)),
            service.replace(&bob, saved.id, vec![link("Bob", "/bob")], Some(saved.updated_at)),
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
        let (_, nodes) = service.get(&admin(), saved.id).await.unwrap();
        assert_eq!(nodes.len(), 1, "only the winner's items are saved");

        // `*` (không kiểm phiên bản) vẫn ghi; menu không còn thì 404.
        service.replace(&ada, saved.id, vec![], None).await.unwrap();
        service.delete(&ada, saved.id).await.unwrap();
        let gone = service.replace(&ada, saved.id, vec![], None).await.unwrap_err();
        assert_eq!(gone.code(), "menus.menu_not_found");
        assert_eq!(
            service.delete(&ada, saved.id).await.unwrap_err().code(),
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
        let service = &harness.service;
        let (saved, _) =
            service.create(&admin(), menu("main", "en", vec![link("Home", "/")])).await.unwrap();
        let now = Utc::now();
        let ready = service.public_menu("main", Some("VI"), now).await.unwrap();
        assert_eq!(ready.menu.locale, "en", "falls back to the default language");
        let error = service.public_menu("main", Some("fr"), now).await.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Validation);
        let error = service.public_menu("nope", None, now).await.unwrap_err();
        assert_eq!(error.code(), "menus.public_menu_not_found");

        let again = service.public_menu("main", None, now).await.unwrap();
        service
            .replace(&admin(), saved.id, vec![link("Start", "/")], Some(saved.updated_at))
            .await
            .unwrap();
        let changed = service.public_menu("main", None, now).await.unwrap();
        assert_eq!(labels(&changed.menu.items), ["Start"]);
        assert_ne!(changed.etag, again.etag);

        service.create(&admin(), menu("main", "vi", vec![link("Trang chủ", "/")])).await.unwrap();
        let vietnamese = service.public_menu("main", Some("vi"), now).await.unwrap();
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
        let service = &harness.service;
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

        let report = service.import_content_menus().await.unwrap();
        assert_eq!(report.imported.len(), 2);
        assert_eq!(report.failed.len(), 1, "`fr` is not a menus language");
        let first = service.find_by_key("main", "en").await.unwrap().unwrap();
        let (_, nodes) = service.get(&admin(), first.id).await.unwrap();
        let about = &nodes[0].item;
        assert_eq!(
            (about.label.as_str(), about.ref_kind.as_deref(), about.ref_id.clone()),
            ("About", Some("content.entry"), Some(entry.to_string()))
        );
        assert_eq!(nodes[0].children[0].item.url.as_deref(), Some("/docs"));
        assert_eq!(nodes[1].item.ref_kind.as_deref(), Some("content.term"));

        // Sửa menu đã nhập rồi nhập lại: không thêm, không đè.
        service.replace(&admin(), first.id, vec![link("Mine", "/")], None).await.unwrap();
        let report = service.import_content_menus().await.unwrap();
        assert_eq!((report.imported.len(), report.skipped.len()), (0, 2));
        let page = autoxide_kernel::pagination::PageRequest::try_from(
            autoxide_kernel::pagination::PageQuery::default(),
        )
        .unwrap();
        assert_eq!(service.list(&admin(), &page).await.unwrap().data.len(), 2);
        let (_, nodes) = service.get(&admin(), first.id).await.unwrap();
        assert_eq!(nodes[0].item.label, "Mine");
    })
    .await
    .unwrap();
}
