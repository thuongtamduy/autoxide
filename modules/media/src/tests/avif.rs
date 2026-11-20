//! Bản AVIF của ảnh thu nhỏ ở mức service (bước 39 B8, D-20261119-step39-avif): chạy lại job, ảnh
//! hỏng, hai worker cùng lúc. Đường thật qua HTTP và worker: `avif_http.rs`.

use image::ImageFormat;

use super::*;
use crate::images::variants::samples::encoded;

async fn avif_rows(database: &Database) -> usize {
    let backend = database.read().get_database_backend();
    let sql = "SELECT id FROM media_variant WHERE content_type = 'image/avif'";
    database.read().query_all_raw(Statement::from_string(backend, sql)).await.unwrap().len()
}

/// Ảnh thu nhỏ xong thì job AVIF được xếp (khoá chống trùng); chạy lại job: vẫn mỗi cỡ một bản;
/// thay nội dung thì job của phiên bản cũ không làm gì.
#[tokio::test]
async fn chay_lai_job_avif_khong_nhan_doi() {
    TestDatabase::run(async |test| {
        let (media, _, database) = crate::testing::service_avif(test).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let png = encoded(260, 130, ImageFormat::Png, false);
        let ada = editor();
        let item = media.upload(&tenant, &ada, upload("a.png", "image/png"), body(&png));
        let item = item.await.unwrap().item;
        // Cấu hình test: rộng 100 và 200 → hai cỡ.
        assert_eq!(media.make_variants(&tenant, item.id, item.version_id).await.unwrap(), 2);
        let backend = database.read().get_database_backend();
        let sql = "SELECT kind FROM media_jobs WHERE kind = 'media.make_avif'";
        let queued = database.read().query_all_raw(Statement::from_string(backend, sql));
        assert_eq!(queued.await.unwrap().len(), 1, "the AVIF job follows the smaller copies");
        // Xếp lại (vd lệnh refresh chạy hai lần): khoá chống trùng.
        media.enqueue_avif(database.write(), &tenant, item.id, item.version_id).await.unwrap();
        let queued = database.read().query_all_raw(Statement::from_string(backend, sql));
        assert_eq!(queued.await.unwrap().len(), 1);

        assert_eq!(media.make_avif(&tenant, item.id, item.version_id).await.unwrap(), 2);
        assert_eq!(media.make_avif(&tenant, item.id, item.version_id).await.unwrap(), 2);
        assert_eq!(avif_rows(&database).await, 2);
        let copies = media.describe_one(&tenant, item.clone()).await.unwrap().variants;
        let avif = copies[0].avif.clone().expect("an AVIF copy next to the PNG one");
        assert_eq!(avif.storage_key(), format!("{}.avif", copies[0].main.storage_key()));

        // Phiên bản cũ: không làm gì.
        assert_eq!(media.make_avif(&tenant, item.id, uuid::Uuid::now_v7()).await.unwrap(), 0);
    })
    .await
    .unwrap();
}

/// Ảnh gốc hỏng: không có bản AVIF, kết thúc `Ok` (không thử lại); ảnh khác vẫn làm được.
#[tokio::test]
async fn avif_anh_hong_khong_lam_hong_job_khac() {
    TestDatabase::run(async |test| {
        let (media, _, database) = crate::testing::service_avif(test).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let png = encoded(260, 130, ImageFormat::Png, false);
        let mut items = Vec::new();
        for name in ["bad.png", "good.png"] {
            let ada = editor();
            let item = media.upload(&tenant, &ada, upload(name, "image/png"), body(&png));
            let item = item.await.unwrap().item;
            media.make_variants(&tenant, item.id, item.version_id).await.unwrap();
            items.push(item);
        }
        // Nội dung gốc của ảnh đầu bị hỏng ở nơi lưu.
        let junk = futures_util::stream::iter([Ok(bytes::Bytes::from_static(b"not an image"))]);
        media.store(&tenant).put(&items[0].storage_key(), Box::pin(junk)).await.unwrap();
        assert_eq!(media.make_avif(&tenant, items[0].id, items[0].version_id).await.unwrap(), 0);
        assert_eq!(media.make_avif(&tenant, items[1].id, items[1].version_id).await.unwrap(), 2);
        assert_eq!(avif_rows(&database).await, 2);
    })
    .await
    .unwrap();
}

/// Hai worker chạy cùng một job AVIF cùng lúc: vẫn mỗi cỡ một bản (khoá dòng item, thay bản cũ).
#[tokio::test]
async fn hai_worker_cung_job_avif_mot_ban() {
    TestDatabase::run(async |test| {
        let (media, _, database) = crate::testing::service_avif(test).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let png = encoded(260, 130, ImageFormat::Png, false);
        let ada = editor();
        let item = media.upload(&tenant, &ada, upload("a.png", "image/png"), body(&png));
        let item = item.await.unwrap().item;
        media.make_variants(&tenant, item.id, item.version_id).await.unwrap();
        let (a, b) = tokio::join!(
            media.make_avif(&tenant, item.id, item.version_id),
            media.make_avif(&tenant, item.id, item.version_id)
        );
        assert_eq!((a.unwrap(), b.unwrap()), (2, 2));
        assert_eq!(avif_rows(&database).await, 2);
    })
    .await
    .unwrap();
}
