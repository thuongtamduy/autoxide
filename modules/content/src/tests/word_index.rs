//! Chỉ mục từ của bản công khai (PT-15, D-20261126-content-word-index): khớp theo từ và tiền tố, chỉ mục theo kịp khi
//! đăng lại / gỡ / xoá, chỉ trong khách của người gọi, chỉ bài người đọc thấy được trên website của họ — lọc ngay
//! trong câu chạm chỉ mục, nên trần số bài khớp không bị bài của website khác lấp. Trần thời gian của câu do kernel
//! lo và có test riêng (`crates/kernel/tests/integration/db_within.rs`).

use super::*;
use crate::entity::published;
use crate::publishing::reading::PostFilter;
use crate::publishing::words::{self, MATCH_CAP, Matching, WordQuery};
use autoxide_kernel::tenancy::{ScopeExt as _, Site, TenantContext};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait as _, EntityTrait as _, QueryFilter as _};

fn titles(page: &autoxide_kernel::pagination::Page<published::Model>) -> Vec<&str> {
    page.data.iter().map(|row| row.title.as_str()).collect()
}

/// Mọi nút của khách, mọi loại, kể cả `noindex`.
fn anywhere(locale: &str) -> Matching<'_> {
    Matching { locale, kind: None, site: None, indexable_only: false }
}

#[tokio::test]
async fn tim_cong_khai_theo_tu_qua_chi_muc() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let dalat = content.create(&tenant, &ada, post("Đà Lạt mùa hoa")).await.unwrap();
        let hue = content.create(&tenant, &ada, post("Huế mộng mơ")).await.unwrap();
        for id in [dalat.id, hue.id] {
            content.publish(&tenant, &ada, id, None, None, false).await.unwrap();
        }
        let first = PageRequest::first(10, Order::Desc);
        let filter = PostFilter {
            locale: "en".into(),
            site: Site::whole(&tenant).lineage_ids(),
            ..Default::default()
        };
        let find = |q: &str| PostFilter { search: Some(q.into()), ..filter.clone() };

        // Theo từ, không dấu, tiền tố từ 3 ký tự; mọi từ phải có.
        for q in ["đà lạt", "DA LAT", "lat", "mùa hoa", "lạt, đà!"] {
            let page = content.public_posts(&tenant, &find(q), &first).await.unwrap();
            assert_eq!(titles(&page), ["Đà Lạt mùa hoa"], "{q}");
        }
        for q in ["alat", "la", "mua ho", "lat hue", "%", "\"*"] {
            let page = content.public_posts(&tenant, &find(q), &first).await.unwrap();
            assert!(page.data.is_empty(), "{q}");
        }

        // Đăng lại với tiêu đề mới: từ cũ hết khớp, từ mới khớp.
        content
            .update(&tenant, &ada, dalat.id, fields("Sa Pa mây trắng"), false, None)
            .await
            .unwrap();
        content.publish(&tenant, &ada, dalat.id, None, None, false).await.unwrap();
        assert!(content.public_posts(&tenant, &find("lat"), &first).await.unwrap().data.is_empty());
        let page = content.public_posts(&tenant, &find("sapa"), &first).await.unwrap();
        assert!(page.data.is_empty(), "two words are not one");
        let page = content.public_posts(&tenant, &find("sa pa"), &first).await.unwrap();
        assert_eq!(titles(&page), ["Sa Pa mây trắng"]);

        // Gỡ: không còn trong chỉ mục.
        content.unpublish(&tenant, &ada, hue.id, None).await.unwrap();
        assert!(content.public_posts(&tenant, &find("hue"), &first).await.unwrap().data.is_empty());

        // Theo loại: không có trang nào mang các từ này.
        let query = WordQuery::parse("sa pa");
        let pages = Matching { kind: Some("page"), ..anywhere("en") };
        assert!(words::matching_ids(&tenant, pages, &query).await.unwrap().is_empty());
    })
    .await
    .unwrap();
}

/// Cùng chữ ở hai khách trên cùng database: mỗi khách chỉ thấy bài của mình (chỉ mục đảo có `tenant_id`, handle của
/// kernel lọc cả câu ngoài lẫn subquery của từng từ).
#[tokio::test]
async fn moi_khach_chi_thay_bai_cua_minh() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let a = TenantContext::default_on(database.clone());
        let b = TenantContext::new(
            autoxide_kernel::tenancy::TenantId::from_uuid(Uuid::now_v7()),
            "b".to_owned(),
            "B".to_owned(),
            autoxide_kernel::tenancy::TenantStatus::Active,
            autoxide_kernel::tenancy::MAIN_PLACEMENT,
            database.clone(),
        );
        let ada = editor();
        let mut ids = Vec::new();
        for tenant in [&a, &b] {
            let who = ada.clone().in_tenant(tenant.id());
            let entry = content.create(tenant, &who, post("Phở bò Hà Nội")).await.unwrap();
            content.publish(tenant, &who, entry.id, None, None, false).await.unwrap();
            ids.push(entry.id);
        }
        let query = WordQuery::parse("pho ha noi");
        assert_eq!(words::matching_ids(&a, anywhere("en"), &query).await.unwrap(), [ids[0]]);
        assert_eq!(words::matching_ids(&b, anywhere("en"), &query).await.unwrap(), [ids[1]]);
        // Từ chỉ khách B có: khách A không thấy gì.
        let who = ada.clone().in_tenant(b.id());
        let only_b = content.create(&b, &who, post("Bánh xèo miền Tây")).await.unwrap();
        content.publish(&b, &who, only_b.id, None, None, false).await.unwrap();
        let query = WordQuery::parse("banh xeo");
        assert!(words::matching_ids(&a, anywhere("en"), &query).await.unwrap().is_empty());
        assert_eq!(words::matching_ids(&b, anywhere("en"), &query).await.unwrap(), [only_b.id]);
    })
    .await
    .unwrap();
}

/// Bài nháp (chưa đăng) và bài bị ẩn (D-363) không khớp; `noindex` chỉ bị bỏ khi bên gọi xin (gợi ý tìm kiếm).
#[tokio::test]
async fn nhap_va_bai_bi_an_khong_khop() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        content.create(&tenant, &ada, post("Bún chả nháp")).await.unwrap();
        let hidden = content.create(&tenant, &ada, post("Bún chả bị ẩn")).await.unwrap();
        let quiet = content.create(&tenant, &ada, post("Bún chả noindex")).await.unwrap();
        for id in [hidden.id, quiet.id] {
            content.publish(&tenant, &ada, id, None, None, false).await.unwrap();
        }
        let set = |id: Uuid, column: published::Column, value: sea_orm::Value| {
            published::Entity::update_many()
                .col_expr(column, Expr::value(value))
                .filter(published::Column::EntryId.eq(id))
                .scoped(&tenant)
                .exec(database.write())
        };
        let now = autoxide_kernel::time::Timestamp::now();
        set(hidden.id, published::Column::RestrictedAt, Some(now).into()).await.unwrap();
        set(quiet.id, published::Column::Noindex, true.into()).await.unwrap();

        let query = WordQuery::parse("bun cha");
        assert_eq!(words::matching_ids(&tenant, anywhere("en"), &query).await.unwrap(), [quiet.id]);
        let indexable = Matching { indexable_only: true, ..anywhere("en") };
        assert!(words::matching_ids(&tenant, indexable, &query).await.unwrap().is_empty());
    })
    .await
    .unwrap();
}

/// Website lọc ngay trong câu chạm chỉ mục: website A có một bài khớp cũ, website B (nhánh khác) có nhiều hơn
/// [`MATCH_CAP`] bài khớp mới hơn — A vẫn thấy bài của mình (lọc sau khi cắt trần thì A không thấy gì).
#[tokio::test]
async fn website_khac_nhieu_bai_khop_khong_lap_ket_qua() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let root = Site::whole(&tenant).lineage_ids();
        let (shop_a, shop_b) = (Uuid::now_v7(), Uuid::now_v7());
        let mine = content.create(&tenant, &ada, post("Phở gà cũ")).await.unwrap();
        content.publish(&tenant, &ada, mine.id, None, None, false).await.unwrap();
        // Bài của A ở nút A.
        published::Entity::update_many()
            .col_expr(published::Column::OrgId, Expr::value(shop_a))
            .filter(published::Column::EntryId.eq(mine.id))
            .scoped(&tenant)
            .exec(database.write())
            .await
            .unwrap();
        let template = published::Entity::find()
            .filter(published::Column::EntryId.eq(mine.id))
            .scoped(&tenant)
            .one(database.read())
            .await
            .unwrap()
            .unwrap();
        // Hơn `MATCH_CAP` bài khớp mới hơn ở nút B (chép thẳng vào bảng: nhanh, như đã đăng).
        let newer = template.published_at + jiff::SignedDuration::from_hours(1);
        let rows: Vec<published::ActiveModel> = (0..MATCH_CAP + 50)
            .map(|n| {
                sea_orm::IntoActiveModel::into_active_model(published::Model {
                    entry_id: Uuid::now_v7(),
                    org_id: Some(shop_b),
                    path: format!("/b/{n}"),
                    slug: format!("b-{n}"),
                    title: format!("Phở gà {n}"),
                    published_at: newer,
                    ..template.clone()
                })
            })
            .collect();
        for chunk in rows.chunks(200) {
            published::Entity::insert_many(chunk.to_vec()).exec(database.write()).await.unwrap();
        }
        for row in &rows {
            let entry = row.entry_id.clone().unwrap();
            words::write(database.write(), &tenant, entry, "en", "pho ga").await.unwrap();
        }

        let query = WordQuery::parse("pho ga");
        let site_a = [shop_a, root[0]];
        let on_a = Matching { site: Some(&site_a), ..anywhere("en") };
        assert_eq!(words::matching_ids(&tenant, on_a, &query).await.unwrap(), [mine.id]);
        let site_b = [shop_b, root[0]];
        let on_b = Matching { site: Some(&site_b), ..anywhere("en") };
        let found = words::matching_ids(&tenant, on_b, &query).await.unwrap();
        assert_eq!(found.len(), MATCH_CAP as usize);
        assert!(!found.contains(&mine.id));
    })
    .await
    .unwrap();
}

/// Tiền tố trên từ ngoài ASCII: khoảng `[w, w_next)` đúng với thứ tự byte trên cả hai database; tiền tố không khớp
/// từ khác chỉ chung vài ký tự đầu.
#[tokio::test]
async fn tien_to_voi_chu_ngoai_ascii() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let tokyo = content.create(&tenant, &ada, post("東京タワー の 夜景")).await.unwrap();
        let other = content.create(&tenant, &ada, post("東西 の 道")).await.unwrap();
        let late = content.create(&tenant, &ada, post("latte lau")).await.unwrap();
        for id in [tokyo.id, other.id, late.id] {
            content.publish(&tenant, &ada, id, None, None, false).await.unwrap();
        }
        let find = async |q: &str| {
            words::matching_ids(&tenant, anywhere("en"), &WordQuery::parse(q)).await.unwrap()
        };
        assert_eq!(find("東京タ").await, [tokyo.id]);
        assert_eq!(find("lat").await, [late.id], "`lau` is not under the prefix `lat`");
        assert_eq!(find("lau").await, [late.id]);
        assert!(find("東京タワー夜").await.is_empty());
    })
    .await
    .unwrap();
}

/// Gấp dấu tiếng Việt (D-104): "ca phe" tìm thấy "Cà phê", và ngược lại; "đ" là "d".
#[tokio::test]
async fn tim_khong_dau_tieng_viet() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let coffee = content.create(&tenant, &ada, post("Cà phê trứng Hà Nội")).await.unwrap();
        let road = content.create(&tenant, &ada, post("Đường lên Đà Lạt")).await.unwrap();
        for id in [coffee.id, road.id] {
            content.publish(&tenant, &ada, id, None, None, false).await.unwrap();
        }
        for (q, expected) in [
            ("ca phe", coffee.id),
            ("CÀ PHÊ", coffee.id),
            ("cà phe trung", coffee.id),
            ("duong len da lat", road.id),
            ("đường", road.id),
        ] {
            let found =
                words::matching_ids(&tenant, anywhere("en"), &WordQuery::parse(q)).await.unwrap();
            assert_eq!(found, [expected], "{q}");
        }
    })
    .await
    .unwrap();
}

/// Bài rất dài vẫn đăng được (nội dung chỉ mục có trần số từ và số byte, dưới trần `tsvector` của Postgres) và vẫn
/// tìm được theo từ ở đầu bài.
#[tokio::test]
async fn bai_rat_dai_van_dang_va_tim_duoc() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        // Sát trần thân bài (512 KB JSON): chừng 10 000 từ khác nhau, dài — quá trần số từ và số byte của chỉ mục.
        let body: Vec<String> =
            (0..10_000).map(|n| format!("tu{n:0>5}{}", "y".repeat(38))).collect();
        let mut input = post("Bách khoa toàn thư");
        input.fields.body = Some(paragraph(&body.join(" ")));
        let entry = content.create(&tenant, &ada, input).await.unwrap();
        content.publish(&tenant, &ada, entry.id, None, None, false).await.unwrap();
        let found = words::matching_ids(&tenant, anywhere("en"), &WordQuery::parse("bach khoa"))
            .await
            .unwrap();
        assert_eq!(found, [entry.id]);
    })
    .await
    .unwrap();
}

/// Migration `m0036_word_index` trên database đã có bài đăng: `down` rồi `up` (lần đầu chạy trên dữ liệu cũ) nạp lại
/// từ của mọi bài theo lô; chạy lại vẫn đúng.
#[tokio::test]
async fn migration_nap_lai_tu_cho_bai_cu() {
    TestDatabase::run(async |test| {
        use autoxide_kernel::migration::prelude::{MigrationTrait as _, SchemaManager};
        let (content, _, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let mut ids = Vec::new();
        for title in ["Phở bò tái", "Bún chả Hà Nội", "Bánh mì Hội An"] {
            let entry = content.create(&tenant, &ada, post(title)).await.unwrap();
            content.publish(&tenant, &ada, entry.id, None, None, false).await.unwrap();
            ids.push(entry.id);
        }
        let migration = crate::migration::m0036_word_index::Migration;
        let find = async |q: &str| {
            words::matching_ids(&tenant, anywhere("en"), &WordQuery::parse(q)).await.unwrap()
        };
        for _ in 0..2 {
            migration.down(&SchemaManager::new(database.write())).await.unwrap();
            migration.up(&SchemaManager::new(database.write())).await.unwrap();
            assert_eq!(find("ha noi").await, [ids[1]]);
            assert_eq!(find("hoi an").await, [ids[2]]);
        }
        // Bài đăng sau migration vẫn vào chỉ mục (trigger SQLite, cột của Postgres).
        let late = content.create(&tenant, &ada, post("Cơm tấm Sài Gòn")).await.unwrap();
        content.publish(&tenant, &ada, late.id, None, None, false).await.unwrap();
        assert_eq!(find("com tam").await, [late.id]);
    })
    .await
    .unwrap();
}
