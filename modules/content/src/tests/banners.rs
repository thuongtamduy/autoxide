//! Vị trí hiển thị và banner (bước 55, D-236): kiểm khi lưu, đọc công khai theo khung giờ, theo
//! nhánh, theo trang, cache, thứ tự, chương trình khuyến mại kết thúc. Chạy trên cả hai database.

use autoxide_kernel::time::Timestamp;
use autoxide_media::api::testing::FakeMedia;
use jiff::SignedDuration;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use autoxide_kernel::access::{Grant, Principal, Scope};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::resource::{Resource, ResourceResolver};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, Site, TenantContext, TenantId};
use serde_json::json;

use super::*;
use crate::banners::items::{BannerFields, BannerPatch, Link, NewBanner, PageTarget, Via};
use crate::banners::placements::PlacementInput;
use crate::banners::public::PageContext;
use crate::service::ContentService;

/// Ảnh mới cỡ `width` × `height` (công khai hay riêng tư), `alt` "ảnh".
fn sized(media: &FakeMedia, public: bool, width: i32, height: i32) -> Uuid {
    media.add_with(public, |item| {
        item.width = Some(width);
        item.height = Some(height);
        item.alt = "ảnh".to_owned();
    })
}

/// Sổ đối tượng giả cho một loại: ID → công khai không.
struct Objects(&'static str, Mutex<HashMap<String, bool>>);

#[async_trait::async_trait]
impl ResourceResolver for Objects {
    async fn resolve(&self, _: &TenantContext, id: &str) -> Result<Option<Resource>, Error> {
        let known = self.1.lock().unwrap();
        Ok(known
            .get(id)
            .map(|public| Resource::new(id).public(*public).url(Some(format!("/{}/{id}", self.0)))))
    }
}

struct Harness {
    service: Arc<ContentService>,
    media: Arc<FakeMedia>,
    tenant: TenantContext,
    products: Arc<Objects>,
    categories: Arc<Objects>,
}

async fn harness(test: &TestDatabase) -> Harness {
    let (service, _, database) = service(test, config(&[])).await;
    let mut service = Arc::try_unwrap(service).ok().unwrap();
    let media = Arc::new(FakeMedia::new());
    service.media = media.clone();
    let products = Arc::new(Objects("p", Mutex::default()));
    let categories = Arc::new(Objects("c", Mutex::default()));
    service.resources.register("catalog", "catalog.product", products.clone()).unwrap();
    service.resources.register("catalog", "catalog.category", categories.clone()).unwrap();
    let tenant = TenantContext::default_on(database.clone());
    let write = database.begin_write().await.unwrap();
    OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
    write.commit().await.unwrap();
    Harness { service: Arc::new(service), media, tenant, products, categories }
}

fn manager() -> Principal {
    user(&["manage", "banners"])
}

fn placement(key: &str, display: &str, accepts: &str) -> PlacementInput {
    PlacementInput {
        key: key.to_owned(),
        name: key.to_owned(),
        display: display.to_owned(),
        accepts: Some(accepts.to_owned()),
        ..Default::default()
    }
}

fn image(placement: &str, media_id: Uuid) -> NewBanner {
    NewBanner {
        placement: placement.to_owned(),
        org_id: None,
        fields: BannerFields {
            title: "Sale".to_owned(),
            media_id: Some(media_id),
            status: Some("active".to_owned()),
            ..Default::default()
        },
    }
}

fn text(placement: &str, title: &str) -> NewBanner {
    NewBanner {
        placement: placement.to_owned(),
        org_id: None,
        fields: BannerFields {
            kind: Some("text".to_owned()),
            title: title.to_owned(),
            status: Some("active".to_owned()),
            ..Default::default()
        },
    }
}

async fn titles(h: &Harness, site: &Site, keys: &[&str], page: &PageContext) -> Vec<Vec<String>> {
    let keys: Vec<String> = keys.iter().map(|key| (*key).to_owned()).collect();
    let found = h
        .service
        .public_placements(&h.tenant, site, &keys, page, Timestamp::now(), false)
        .await
        .unwrap();
    found
        .placements
        .into_iter()
        .map(|ready| ready.banners.into_iter().map(|banner| banner.row.title).collect())
        .collect()
}

#[tokio::test]
async fn khach_moi_co_bo_vi_tri_chuan() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        let seed = crate::banners::presets::seed();
        seed.seed(&h.tenant).await.unwrap();
        let all = h
            .service
            .list_placements(&h.tenant, &manager(), &Default::default())
            .await
            .unwrap()
            .data;
        assert_eq!(all.len(), 9);
        let hero = all.iter().find(|row| row.key == "home_hero").unwrap();
        assert_eq!(
            (hero.display.as_str(), hero.ratio_w, hero.ratio_h),
            ("carousel", Some(3), Some(1))
        );
        assert_eq!(hero.preset_key.as_deref(), Some("home_hero"));
        // gieo_lai_khong_trung: chạy lại không thêm dòng nào.
        seed.seed(&h.tenant).await.unwrap();
        assert_eq!(
            h.service
                .list_placements(&h.tenant, &manager(), &Default::default())
                .await
                .unwrap()
                .data
                .len(),
            9
        );
        // xoa_vi_tri_chuan_khong_gieo_lai: khách xoá một vị trí, gieo lại không thêm về.
        h.service.delete_placement(&h.tenant, &manager(), "popup").await.unwrap();
        seed.seed(&h.tenant).await.unwrap();
        assert_eq!(
            h.service
                .list_placements(&h.tenant, &manager(), &Default::default())
                .await
                .unwrap()
                .data
                .len(),
            8
        );
        // Thêm tay phần còn thiếu thì có lại đúng một vị trí.
        let added = h.service.install_presets(&h.tenant, &manager()).await.unwrap();
        assert_eq!(added.iter().map(|row| row.key.as_str()).collect::<Vec<_>>(), ["popup"]);
        assert!(h.service.install_presets(&h.tenant, &manager()).await.unwrap().is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cai_dat_vi_tri_sai_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        let mut input = placement("hero", "carousel", "image");
        input.display_settings = json!({"autoplay_seconds": 2}).as_object().cloned().unwrap();
        let error = h.service.create_placement(&h.tenant, &manager(), input).await.unwrap_err();
        assert_eq!(error.code(), "content.placement_settings_invalid");
        let error = h
            .service
            .create_placement(&h.tenant, &manager(), placement("strip", "strip", "image"))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "validation_failed");
        let error = h
            .service
            .create_placement(
                &h.tenant,
                &manager(),
                placement("seller_store.top", "single", "image"),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code(), "validation_failed", "khoá có dấu chấm để dành");
        h.service
            .create_placement(&h.tenant, &manager(), placement("ok", "single", "image"))
            .await
            .unwrap();
        let again = h
            .service
            .create_placement(&h.tenant, &manager(), placement("ok", "single", "image"))
            .await
            .unwrap_err();
        assert_eq!(again.code(), "content.placement_exists");
        let current = h.service.get_placement(&h.tenant, &manager(), "ok").await.unwrap();
        let mut changed = placement("ignored", "grid", "image");
        changed.name = "Renamed".to_owned();
        let saved = h
            .service
            .replace_placement(
                &h.tenant,
                &manager(),
                "ok",
                changed.clone(),
                Some(current.updated_at),
            )
            .await
            .unwrap();
        assert_eq!(
            (saved.key.as_str(), saved.name.as_str(), saved.display.as_str()),
            ("ok", "Renamed", "grid")
        );
        let reread = h.service.get_placement(&h.tenant, &manager(), "ok").await.unwrap();
        assert_eq!(reread, saved);
        let stale = h
            .service
            .replace_placement(&h.tenant, &manager(), "ok", changed, Some(current.updated_at))
            .await
            .unwrap_err();
        assert_eq!(stale.code(), "precondition_failed");
        let banners_only = user(&["banners"]);
        let refused = h
            .service
            .create_placement(&h.tenant, &banners_only, placement("x", "single", "image"))
            .await;
        assert_eq!(refused.unwrap_err().code(), "permission_denied");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn anh_sai_ti_le_hay_qua_nho_bi_tu_choi_kem_co_can() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        let mut input = placement("hero", "single", "image");
        input.ratio = Some((3, 1));
        input.mobile_ratio = Some((4, 5));
        input.min_width = Some(1800);
        input.min_height = Some(600);
        h.service.create_placement(&h.tenant, &manager(), input).await.unwrap();
        let wrong_ratio = sized(&h.media, true, 1800, 900);
        let error = h
            .service
            .create_banner(&h.tenant, &manager(), image("hero", wrong_ratio), Via::Rest)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "content.banner_image_size_invalid");
        assert!(
            error.message().contains("3:1") && error.message().contains("1800×600"),
            "{}",
            error.message()
        );
        let small = sized(&h.media, true, 900, 300);
        let error = h
            .service
            .create_banner(&h.tenant, &manager(), image("hero", small), Via::Rest)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "content.banner_image_size_invalid");
        // anh_dien_thoai_kiem_ti_le_rieng: ảnh điện thoại theo 4:5, không theo 3:1.
        let good = sized(&h.media, true, 1800, 600);
        let mut banner = image("hero", good);
        banner.fields.mobile_media_id = Some(sized(&h.media, true, 1800, 600));
        let error = h
            .service
            .create_banner(&h.tenant, &manager(), banner.clone(), Via::Rest)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "content.banner_image_size_invalid");
        banner.fields.mobile_media_id = Some(sized(&h.media, true, 800, 1000));
        h.service.create_banner(&h.tenant, &manager(), banner, Via::Rest).await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn anh_banner_rieng_tu_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        h.service
            .create_placement(&h.tenant, &manager(), placement("hero", "single", "image"))
            .await
            .unwrap();
        let private = sized(&h.media, false, 100, 100);
        let error = h
            .service
            .create_banner(&h.tenant, &manager(), image("hero", private), Via::Rest)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "content.media_not_public");
        let error = h
            .service
            .create_banner(&h.tenant, &manager(), image("hero", Uuid::now_v7()), Via::Rest)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "content.media_not_found");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dai_chu_chi_nhan_banner_chu() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        h.service
            .create_placement(&h.tenant, &manager(), placement("top", "strip", "text"))
            .await
            .unwrap();
        let picture = sized(&h.media, true, 100, 100);
        let error = h
            .service
            .create_banner(&h.tenant, &manager(), image("top", picture), Via::Rest)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "content.banner_kind_not_allowed");
        let (saved, _) = h
            .service
            .create_banner(&h.tenant, &manager(), text("top", "Free shipping"), Via::Rest)
            .await
            .unwrap();
        assert_eq!(saved.tone.as_deref(), Some("neutral"));
        // banner_chu_khong_nhan_ma_mau_tu_do
        let mut colored = text("top", "Red");
        colored.fields.tone = Some("#ff0000".to_owned());
        let error =
            h.service.create_banner(&h.tenant, &manager(), colored, Via::Rest).await.unwrap_err();
        assert_eq!(error.code(), "validation_failed");
        assert_eq!(error.fields()[0].field, "tone");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn banner_ngoai_khung_gio_khong_hien() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        h.service
            .create_placement(&h.tenant, &manager(), placement("top", "strip", "text"))
            .await
            .unwrap();
        let now = Timestamp::now();
        let mut later = text("top", "later");
        later.fields.starts_at = Some(now + SignedDuration::from_secs(30));
        let mut ending = text("top", "ending");
        ending.fields.ends_at = Some(now + SignedDuration::from_secs(20));
        let mut past = text("top", "past");
        past.fields.starts_at = Some(now - SignedDuration::from_hours(2));
        past.fields.ends_at = Some(now - SignedDuration::from_hours(1));
        for banner in [later, ending, past] {
            h.service.create_banner(&h.tenant, &manager(), banner, Via::Rest).await.unwrap();
        }
        let site = Site::whole(&h.tenant);
        let keys = vec!["top".to_owned()];
        let read = |at| {
            let service = h.service.clone();
            let tenant = h.tenant.clone();
            let site = site.clone();
            let keys = keys.clone();
            async move {
                service
                    .public_placements(&tenant, &site, &keys, &PageContext::default(), at, false)
                    .await
                    .unwrap()
            }
        };
        let found = read(now).await;
        let shown: Vec<&str> =
            found.placements[0].banners.iter().map(|b| b.row.title.as_str()).collect();
        assert_eq!(shown, ["ending"]);
        assert!(found.max_age <= 20 && found.max_age >= 19, "max-age {}", found.max_age);
        let found = read(now + SignedDuration::from_secs(40)).await;
        let shown: Vec<&str> =
            found.placements[0].banners.iter().map(|b| b.row.title.as_str()).collect();
        assert_eq!(shown, ["later"]);
        assert_eq!(found.max_age, crate::banners::public::MAX_AGE_SECONDS);
        // Bản nháp không hiện.
        let mut draft = text("top", "draft");
        draft.fields.status = Some("draft".to_owned());
        h.service.create_banner(&h.tenant, &manager(), draft, Via::Rest).await.unwrap();
        assert_eq!(read(now + SignedDuration::from_secs(40)).await.placements[0].banners.len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn max_age_la_moc_gan_nhat_cua_moi_vi_tri() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        for key in ["a", "b"] {
            h.service
                .create_placement(&h.tenant, &manager(), placement(key, "strip", "text"))
                .await
                .unwrap();
        }
        let now = Timestamp::now();
        let mut soon = text("b", "soon");
        soon.fields.starts_at = Some(now + SignedDuration::from_secs(10));
        h.service.create_banner(&h.tenant, &manager(), text("a", "a"), Via::Rest).await.unwrap();
        h.service.create_banner(&h.tenant, &manager(), soon, Via::Rest).await.unwrap();
        // doc_nhieu_vi_tri_mot_lan_goi: một lời gọi trả cả hai vị trí, theo thứ tự hỏi.
        let keys = vec!["b".to_owned(), "a".to_owned(), "missing".to_owned()];
        let found = h
            .service
            .public_placements(
                &h.tenant,
                &Site::whole(&h.tenant),
                &keys,
                &PageContext::default(),
                now,
                false,
            )
            .await
            .unwrap();
        let order: Vec<&str> = found.placements.iter().map(|p| p.placement.key.as_str()).collect();
        assert_eq!(order, ["b", "a"]);
        assert!(found.max_age <= 10, "max-age {}", found.max_age);
        let many: Vec<String> = (0..11).map(|i| format!("k{i}")).collect();
        let error = h
            .service
            .public_placements(
                &h.tenant,
                &Site::whole(&h.tenant),
                &many,
                &PageContext::default(),
                now,
                false,
            )
            .await
            .unwrap_err();
        assert_eq!(error.code(), "content.placement_keys_too_many");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn banner_cua_cua_hang_va_cua_chuoi() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        h.service
            .create_placement(&h.tenant, &manager(), placement("top", "strip", "text"))
            .await
            .unwrap();
        let root = autoxide_kernel::tenancy::root_of(DEFAULT_TENANT);
        let chain = OrgId::from_uuid(Uuid::now_v7());
        let (shop_a, shop_b) = (OrgId::from_uuid(Uuid::now_v7()), OrgId::from_uuid(Uuid::now_v7()));
        let write = h.tenant.database().begin_write().await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, chain, root, 8).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, shop_a, chain, 8).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, shop_b, chain, 8).await.unwrap();
        write.commit().await.unwrap();
        for (title, org) in [("tenant", None), ("chain", Some(chain)), ("shop b", Some(shop_b))] {
            let mut banner = text("top", title);
            banner.org_id = org.map(|org| org.as_uuid());
            h.service.create_banner(&h.tenant, &manager(), banner, Via::Rest).await.unwrap();
        }
        let site_a = Site::of(&h.tenant, shop_a).await.unwrap();
        assert_eq!(
            titles(&h, &site_a, &["top"], &PageContext::default()).await,
            [["tenant", "chain"]]
        );
        let site_b = Site::of(&h.tenant, shop_b).await.unwrap();
        assert_eq!(
            titles(&h, &site_b, &["top"], &PageContext::default()).await,
            [["tenant", "chain", "shop b"]]
        );
        // Người của cửa hàng A (độ sâu `unit`) không thấy, không sửa được banner của cửa hàng B.
        let staff = Principal::with_grants(
            Uuid::now_v7(),
            [("content.banners".to_owned(), Grant::new(shop_a, Scope::Unit))],
        );
        let page = autoxide_kernel::pagination::PageRequest::default();
        let listed =
            h.service.list_banners(&h.tenant, &staff, Default::default(), &page).await.unwrap();
        assert!(listed.data.is_empty());
        let mut theirs = text("top", "theirs");
        theirs.org_id = Some(shop_b.as_uuid());
        let error =
            h.service.create_banner(&h.tenant, &staff, theirs, Via::Rest).await.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Forbidden);
        let mut mine = text("top", "mine");
        mine.org_id = Some(shop_a.as_uuid());
        let (saved, _) = h.service.create_banner(&h.tenant, &staff, mine, Via::Rest).await.unwrap();
        let listed =
            h.service.list_banners(&h.tenant, &staff, Default::default(), &page).await.unwrap();
        assert_eq!(listed.data.iter().map(|row| row.id).collect::<Vec<_>>(), [saved.id]);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn banner_khach_khac_khong_thay() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        h.service
            .create_placement(&h.tenant, &manager(), placement("top", "strip", "text"))
            .await
            .unwrap();
        let (saved, _) = h
            .service
            .create_banner(&h.tenant, &manager(), text("top", "mine"), Via::Rest)
            .await
            .unwrap();
        let other = TenantContext::new(
            TenantId::from_uuid(Uuid::now_v7()),
            "other",
            "Other",
            autoxide_kernel::tenancy::TenantStatus::Active,
            autoxide_kernel::tenancy::MAIN_PLACEMENT,
            h.tenant.database().clone(),
        );
        let write = other.database().begin_write().await.unwrap();
        OrgTree::insert_root(&write, other.id()).await.unwrap();
        write.commit().await.unwrap();
        let error = h.service.get_banner(&other, &manager(), saved.id).await.unwrap_err();
        assert_eq!(error.code(), "content.banner_not_found");
        let error = h
            .service
            .public_placements(
                &other,
                &Site::whole(&other),
                &["top".to_owned()],
                &PageContext::default(),
                Timestamp::now(),
                true,
            )
            .await
            .unwrap_err();
        assert_eq!(error.code(), "content.placement_not_found");
        // Khách kia tạo cùng khoá được.
        h.service
            .create_placement(&other, &manager(), placement("top", "strip", "text"))
            .await
            .unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn banner_chi_hien_o_danh_muc_da_chon() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        let mut side = placement("side", "stack", "text");
        side.page_kinds = vec!["category".to_owned(), "search".to_owned()];
        h.service.create_placement(&h.tenant, &manager(), side).await.unwrap();
        h.categories
            .1
            .lock()
            .unwrap()
            .extend([("phones".to_owned(), true), ("tv".to_owned(), true)]);
        let mut chosen = text("side", "phones only");
        chosen.fields.page_kinds = vec!["category".to_owned()];
        chosen.fields.page_targets =
            vec![PageTarget { kind: "catalog.category".into(), id: "phones".into() }];
        h.service.create_banner(&h.tenant, &manager(), chosen, Via::Rest).await.unwrap();
        h.service
            .create_banner(&h.tenant, &manager(), text("side", "everywhere"), Via::Rest)
            .await
            .unwrap();
        let site = Site::whole(&h.tenant);
        let at = |kind: &str, id: Option<&str>| PageContext {
            kind: Some(kind.to_owned()),
            reference: id.map(|id| ("catalog.category".to_owned(), id.to_owned())),
            locale: None,
        };
        assert_eq!(
            titles(&h, &site, &["side"], &at("category", Some("phones"))).await,
            [["phones only", "everywhere"]]
        );
        assert_eq!(
            titles(&h, &site, &["side"], &at("category", Some("tv"))).await,
            [["everywhere"]]
        );
        assert_eq!(titles(&h, &site, &["side"], &at("search", None)).await, [["everywhere"]]);
        // Trang chủ không thuộc loại trang của vị trí: không banner nào.
        assert_eq!(titles(&h, &site, &["side"], &at("home", None)).await, [Vec::<String>::new()]);
        // page_kind_ngoai_pham_vi_vi_tri_bi_tu_choi
        let mut outside = text("side", "home");
        outside.fields.page_kinds = vec!["home".to_owned()];
        let error =
            h.service.create_banner(&h.tenant, &manager(), outside, Via::Rest).await.unwrap_err();
        assert_eq!(error.code(), "content.page_kind_not_allowed");
        // page_target_khong_ton_tai_bi_tu_choi
        let mut missing = text("side", "missing");
        missing.fields.page_targets =
            vec![PageTarget { kind: "catalog.category".into(), id: "nope".into() }];
        let error =
            h.service.create_banner(&h.tenant, &manager(), missing, Via::Rest).await.unwrap_err();
        assert_eq!(error.code(), "content.page_target_not_found");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn link_toi_doi_tuong_khong_con_cong_khai_bi_bo() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        h.service
            .create_placement(&h.tenant, &manager(), placement("top", "strip", "text"))
            .await
            .unwrap();
        h.products.1.lock().unwrap().insert("p1".to_owned(), true);
        let mut linked = text("top", "product");
        linked.fields.link = Link::Ref { kind: "catalog.product".into(), id: "p1".into() };
        h.service.create_banner(&h.tenant, &manager(), linked, Via::Rest).await.unwrap();
        let site = Site::whole(&h.tenant);
        let keys = vec!["top".to_owned()];
        let found = h
            .service
            .public_placements(
                &h.tenant,
                &site,
                &keys,
                &PageContext::default(),
                Timestamp::now(),
                false,
            )
            .await
            .unwrap();
        assert_eq!(found.placements[0].banners[0].href.as_deref(), Some("/p/p1"));
        h.products.1.lock().unwrap().insert("p1".to_owned(), false);
        assert_eq!(
            titles(&h, &site, &["top"], &PageContext::default()).await,
            [Vec::<String>::new()]
        );
        let mut missing = text("top", "missing");
        missing.fields.link = Link::Ref { kind: "catalog.product".into(), id: "nope".into() };
        let error =
            h.service.create_banner(&h.tenant, &manager(), missing, Via::Rest).await.unwrap_err();
        assert_eq!(error.code(), "content.link_target_missing");
        let mut url = text("top", "bad url");
        url.fields.link = Link::Url("javascript:alert(1)".into());
        let error =
            h.service.create_banner(&h.tenant, &manager(), url, Via::Rest).await.unwrap_err();
        assert_eq!(error.code(), "validation_failed");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn banner_tro_toi_chuong_trinh_het_han_bi_an() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        h.service
            .create_placement(&h.tenant, &manager(), placement("top", "strip", "text"))
            .await
            .unwrap();
        let promotion = Uuid::now_v7().to_string();
        let mut linked = text("top", "deal");
        linked.fields.link =
            Link::Ref { kind: "promotions.promotion".into(), id: promotion.clone() };
        // Chưa cài `promotions`: không lưu được link loại chương trình.
        let error = h
            .service
            .create_banner(&h.tenant, &manager(), linked.clone(), Via::Rest)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "content.link_kind_unavailable");
        let promotions = Arc::new(Objects("deal", Mutex::default()));
        promotions.1.lock().unwrap().insert(promotion.clone(), true);
        h.service.resources.register("promotions", "promotions.promotion", promotions).unwrap();
        let (saved, _) =
            h.service.create_banner(&h.tenant, &manager(), linked, Via::Rest).await.unwrap();
        let handler = crate::banners::promotions::Ended(h.service.clone());
        let mut event =
            Delivered::new(h.tenant.clone(), "promotions", "promotions.promotion_ended", json!({}));
        event.subject = Some(format!("promotions.promotion:{promotion}"));
        handler.handle(&event).await.unwrap();
        handler.handle(&event).await.unwrap();
        let (row, _) = h.service.get_banner(&h.tenant, &manager(), saved.id).await.unwrap();
        assert_eq!(row.status, "archived");
        let changed = events(h.tenant.database())
            .await
            .into_iter()
            .filter(|name| name == "content.banner_changed")
            .count();
        assert_eq!(changed, 2, "tạo + lưu trữ một lần; nhận lại event không ghi thêm");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn thu_tu_co_dinh_khong_dao() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        h.service
            .create_placement(&h.tenant, &manager(), placement("top", "strip", "text"))
            .await
            .unwrap();
        let mut ids = Vec::new();
        for title in ["one", "two", "three"] {
            let (saved, _) = h
                .service
                .create_banner(&h.tenant, &manager(), text("top", title), Via::Rest)
                .await
                .unwrap();
            ids.push(saved.id);
        }
        let site = Site::whole(&h.tenant);
        for _ in 0..3 {
            assert_eq!(
                titles(&h, &site, &["top"], &PageContext::default()).await,
                [["one", "two", "three"]]
            );
        }
        let current = h.service.get_placement(&h.tenant, &manager(), "top").await.unwrap();
        // doi_thu_tu_dong_thoi_mot_ben_thua: cùng `If-Match`, bên sau nhận 412.
        let first = h
            .service
            .order_placement(&h.tenant, &manager(), "top", vec![ids[2]], Some(current.updated_at))
            .await;
        let second = h
            .service
            .order_placement(&h.tenant, &manager(), "top", vec![ids[1]], Some(current.updated_at))
            .await;
        assert!(first.is_ok());
        assert_eq!(second.unwrap_err().code(), "precondition_failed");
        assert_eq!(
            titles(&h, &site, &["top"], &PageContext::default()).await,
            [["three", "one", "two"]]
        );
        let error = h
            .service
            .order_placement(&h.tenant, &manager(), "top", vec![Uuid::now_v7()], None)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "validation_failed");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn vi_tri_day_va_cong_cu_ai_chi_tao_nhap() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        let mut small = placement("top", "strip", "text");
        small.max_items = Some(1);
        h.service.create_placement(&h.tenant, &manager(), small).await.unwrap();
        let (first, _) = h
            .service
            .create_banner(&h.tenant, &manager(), text("top", "one"), Via::Rest)
            .await
            .unwrap();
        let error = h
            .service
            .create_banner(&h.tenant, &manager(), text("top", "two"), Via::Rest)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "content.placement_full");
        // Lưu trữ một cái thì tạo được.
        let archive = BannerPatch { status: Some("archived".into()), ..Default::default() };
        h.service
            .update_banner(&h.tenant, &manager(), first.id, archive, None, Via::Rest)
            .await
            .unwrap();
        let (draft, _) = h
            .service
            .create_banner(&h.tenant, &manager(), text("top", "two"), Via::Tool)
            .await
            .unwrap();
        assert_eq!(draft.status, "draft", "tool chỉ tạo bản nháp");
        let activate = BannerPatch { status: Some("active".into()), ..Default::default() };
        let error = h
            .service
            .update_banner(&h.tenant, &manager(), draft.id, activate.clone(), None, Via::Tool)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "content.banner_activation_needs_person");
        h.service
            .update_banner(&h.tenant, &manager(), draft.id, activate, None, Via::Rest)
            .await
            .unwrap();
        let window = BannerPatch {
            starts_at: Some(Some(Timestamp::now())),
            ends_at: Some(Some(Timestamp::now() - SignedDuration::from_hours(1))),
            ..Default::default()
        };
        let error = h
            .service
            .update_banner(&h.tenant, &manager(), draft.id, window, None, Via::Rest)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "content.banner_window_invalid");
        let error = h.service.delete_placement(&h.tenant, &manager(), "top").await.unwrap_err();
        assert_eq!(error.code(), "content.placement_in_use");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tao_banner_cung_luc_khong_vuot_max_items_khong_trung_vi_tri() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        let mut small = placement("top", "strip", "text");
        small.max_items = Some(2);
        h.service.create_placement(&h.tenant, &manager(), small).await.unwrap();
        // Bốn lần tạo cùng lúc. Trước đây đếm và lấy `position` kế tiếp không khoá: Postgres cho
        // nhiều bên cùng đếm thiếu, vượt `max_items` và trùng `position`.
        let boss = manager();
        let create = |title: &'static str| {
            h.service.create_banner(&h.tenant, &boss, text("top", title), Via::Rest)
        };
        let (a, b, c, d) = tokio::join!(create("a"), create("b"), create("c"), create("d"));
        let outcomes = [a, b, c, d];
        let mut positions = Vec::new();
        for outcome in outcomes {
            match outcome {
                Ok((banner, _)) => positions.push(banner.position),
                Err(error) => assert_eq!(error.code(), "content.placement_full"),
            }
        }
        assert_eq!(positions.len(), 2, "{positions:?}");
        positions.sort_unstable();
        positions.dedup();
        assert_eq!(positions.len(), 2, "trùng vị trí");
    })
    .await
    .unwrap();
}
