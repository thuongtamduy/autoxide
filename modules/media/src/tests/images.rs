//! Ảnh thu nhỏ theo phiên bản nội dung.

use super::*;

/// Ảnh thu nhỏ: tải lên đưa job vào hàng đợi; job làm đúng các cỡ nhỏ hơn ảnh; thay nội dung
/// thì bản cũ bị xoá và job cũ không làm gì; xoá hẳn thì hết; job dọn mồ côi giữ bản hiện tại.
#[tokio::test]
async fn anh_thu_nho_theo_phien_ban() {
    use image::ImageFormat;

    use crate::images::variants::samples::encoded;

    TestDatabase::run(async |test| {
        let (media, storage, database) = service(test, limits()).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let ada = editor();
        let original = encoded(150, 90, ImageFormat::Png, false);
        let item =
            media.upload(&tenant, &ada, upload("wide.png", "image/png"), body(&original)).await;
        let item = item.unwrap().item;
        let backend = database.read().get_database_backend();
        let jobs = database
            .read()
            .query_all_raw(Statement::from_string(backend, "SELECT kind FROM media_jobs"))
            .await
            .unwrap();
        assert_eq!(jobs.len(), 1, "one job queued with the upload");

        // Cấu hình test: rộng 100 và 200; ảnh rộng 150 nên chỉ có bản 100.
        assert_eq!(media.make_variants(&tenant, item.id, item.version_id).await.unwrap(), 1);
        let copies = media.describe_one(&tenant, item.clone()).await.unwrap().variants;
        let sizes: Vec<_> = copies.iter().map(|c| (c.main.width, c.main.height)).collect();
        assert_eq!(sizes, [(100, 60)]);
        assert_eq!(copies[0].main.content_type, "image/jpeg");
        let copy_key = copies[0].main.storage_key();
        let webp = copies[0].webp.clone().expect("a WebP copy next to the JPEG one");
        assert_eq!((webp.width, webp.height, webp.content_type.as_str()), (100, 60, "image/webp"));
        let webp_key = webp.storage_key();
        assert_eq!(webp_key, format!("{copy_key}.webp"));
        for (key, format) in [(&copy_key, ImageFormat::Jpeg), (&webp_key, ImageFormat::WebP)] {
            let stored = media.read_key(&tenant, key, None).await.unwrap();
            let bytes: Vec<bytes::Bytes> = stored.content.try_collect().await.unwrap();
            let bytes = bytes.concat();
            assert_eq!(image::guess_format(&bytes).unwrap(), format);
            let decoded = image::load_from_memory(&bytes).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (100, 60));
        }
        // Chạy lại job (vd thử lại): không nhân đôi.
        media.make_variants(&tenant, item.id, item.version_id).await.unwrap();
        assert_eq!(media.describe_one(&tenant, item.clone()).await.unwrap().variants.len(), 1);

        // Ảnh SVG, PDF, ảnh nhỏ hơn mọi cỡ: không có job.
        media
            .upload(&tenant, &ada, upload("tiny.png", "image/png"), body(&samples::png(50, 50)))
            .await
            .unwrap();
        let jobs = database
            .read()
            .query_all_raw(Statement::from_string(backend, "SELECT kind FROM media_jobs"))
            .await;
        assert_eq!(jobs.unwrap().len(), 1);

        // Thay nội dung: bản thu nhỏ cũ bị xoá, job của phiên bản cũ không làm gì.
        let bigger = encoded(400, 200, ImageFormat::Png, true);
        let new = media
            .replace(&tenant, &ada, item.id, Some("image/png"), body(&bigger), None)
            .await
            .unwrap();
        assert!(media.describe_one(&tenant, new.clone()).await.unwrap().variants.is_empty());
        assert!(storage.get(&copy_key).await.unwrap().is_none());
        assert!(storage.get(&webp_key).await.unwrap().is_none());
        assert_eq!(media.make_variants(&tenant, item.id, item.version_id).await.unwrap(), 0);
        assert_eq!(media.make_variants(&tenant, new.id, new.version_id).await.unwrap(), 2);
        let copies = media.describe_one(&tenant, new.clone()).await.unwrap().variants;
        assert_eq!(
            copies.iter().map(|c| c.main.content_type.as_str()).collect::<Vec<_>>(),
            ["image/png", "image/png"]
        );
        assert!(copies.iter().all(|c| c.webp.as_ref().is_some_and(|w| w.width == c.main.width)));

        // Dọn mồ côi giữ bản thu nhỏ của phiên bản hiện tại.
        assert_eq!(
            media
                .clean_orphans(
                    &autoxide_kernel::tenancy::DatabaseRegistry::single(database.clone()),
                    Utc::now() + chrono::Duration::days(2),
                )
                .await
                .unwrap(),
            0
        );
        // Bản gốc, và mỗi cỡ hai bản (PNG, WebP).
        assert_eq!(storage.keys_with_prefix(&format!("media/{}/", new.id)).await.unwrap().len(), 5);

        media.trash(&tenant, &ada, new.id).await.unwrap();
        media.purge(&tenant, &ada, new.id).await.unwrap();
        assert!(storage.keys_with_prefix(&format!("media/{}/", new.id)).await.unwrap().is_empty());
    })
    .await
    .unwrap();
}
