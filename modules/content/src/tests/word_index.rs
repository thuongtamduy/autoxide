//! Chỉ mục từ của bản công khai (PT-15, D-20261126-content-word-index): khớp theo từ và tiền tố, chỉ mục theo kịp khi
//! đăng lại / gỡ / xoá, chỉ trong khách của người gọi, và câu chạy quá trần thời gian bị database dừng — kết nối trở về
//! pool vẫn dùng được.

use std::time::Duration;

use super::*;
use crate::publishing::reading::PostFilter;
use crate::publishing::words::{self, Bind, Matching, WordQuery};
use autoxide_kernel::tenancy::{Site, TenantContext};

fn titles(page: &autoxide_kernel::pagination::Page<crate::entity::published::Model>) -> Vec<&str> {
    page.data.iter().map(|row| row.title.as_str()).collect()
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

        // Chỉ khách của người gọi: cùng chữ ở khách khác không lộ.
        let read = database.read();
        let query = WordQuery::parse("sa pa");
        let mine = Matching { tenant: tenant.id().as_uuid(), locale: "en", kind: Some("post") };
        assert_eq!(words::matching_ids(read, mine, &query).await.unwrap(), [dalat.id]);
        let other = Matching { tenant: Uuid::now_v7(), ..mine };
        assert!(words::matching_ids(read, other, &query).await.unwrap().is_empty());
        let pages = Matching { kind: Some("page"), ..mine };
        assert!(words::matching_ids(read, pages, &query).await.unwrap().is_empty());
    })
    .await
    .unwrap();
}

/// Câu chạy quá trần bị database dừng (Postgres `statement_timeout`, SQLite progress handler) và trả
/// `content.public_search_timeout`; mọi kết nối của pool dùng lại được ngay sau đó (cấu hình không rò ra ngoài).
#[tokio::test]
async fn cau_tim_qua_tran_thoi_gian_bi_dung() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let entry = content.create(&tenant, &ada, post("Phở Hà Nội")).await.unwrap();
        content.publish(&tenant, &ada, entry.id, None, None, false).await.unwrap();
        let read = database.read();
        let index = words::index_for(read);
        // Câu chậm của từng database: ngủ (Postgres), đệ quy rất dài (SQLite).
        let slow = match read.get_database_backend() {
            sea_orm::DatabaseBackend::Postgres => {
                "SELECT gen_random_uuid() FROM pg_sleep(30)".to_owned()
            }
            _ => "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c \
                  WHERE x < 1000000000) \
                  SELECT entry_id FROM content_published WHERE (SELECT MAX(x) FROM c) > 0"
                .to_owned(),
        };
        let started = std::time::Instant::now();
        let error = index
            .ids_within(read, slow, Vec::<Bind>::new(), Duration::from_millis(300))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "content.public_search_timeout");
        assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());

        // Sau khi bị dừng: tìm bình thường nhiều lần (đi qua mọi kết nối của pool) vẫn đúng.
        let mine = Matching { tenant: tenant.id().as_uuid(), locale: "en", kind: None };
        for _ in 0..8 {
            let found =
                words::matching_ids(read, mine, &WordQuery::parse("pho ha noi")).await.unwrap();
            assert_eq!(found, [entry.id]);
        }
    })
    .await
    .unwrap();
}
