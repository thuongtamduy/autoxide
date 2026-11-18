//! Tải thẳng (bước 40 P4) trên nơi lưu giả cấp link: xác nhận đọc lại qua cùng bộ kiểm như tải qua
//! máy chủ (loại thật, SVG, kích thước ảnh, cỡ, hash), gọi lại trả cùng item, đồng thời chỉ một
//! item, dọn lượt bỏ dở. Chạy trên cả hai database.

use super::*;
use crate::library::direct::StartMediaUpload;
use autoxide_kernel::tenancy::{DatabaseRegistry, TenantContext};
use sha2::{Digest, Sha256};

fn sha(content: &[u8]) -> String {
    autoxide_kernel::upload::hex(&Sha256::digest(content))
}

fn start(name: &str, content_type: &str, content: &[u8]) -> StartMediaUpload {
    StartMediaUpload {
        name: name.to_owned(),
        content_type: content_type.to_owned(),
        size: content.len() as u64,
        sha256: sha(content),
        ..StartMediaUpload::default()
    }
}

#[tokio::test]
async fn tai_thang_roi_xac_nhan_tao_item() {
    TestDatabase::run(async |test| {
        let (media, storage, database) = service(test, limits()).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let png = samples::png(640, 480);

        // Nơi lưu như ổ đĩa: không cấp link.
        let refused = media.start_upload(&tenant, &ada, start("a.png", "image/png", &png)).await;
        assert_eq!(refused.unwrap_err().code(), "media.direct_upload_unavailable");

        storage.presign(true);
        let mut input = start("a.png", "image/png", &png);
        input.tags = vec!["brand".to_owned()];
        let (upload, link) = media.start_upload(&tenant, &ada, input).await.unwrap();
        assert!(link.url.starts_with(&format!("memory://media/{}/", upload.id)), "{}", link.url);
        let early = media.complete_upload(&tenant, &ada, upload.id).await.unwrap_err();
        assert_eq!(early.code(), "media.upload_incomplete");

        storage.put_presigned(&link.url, png.clone());
        let confirmed = media.complete_upload(&tenant, &ada, upload.id).await.unwrap();
        assert!(confirmed.created);
        let item = confirmed.uploaded.item;
        assert_eq!((item.id, item.width, item.height), (upload.id, Some(640), Some(480)));
        assert_eq!(item.content_type, "image/png");
        assert_eq!(content(&tenant, &media, &item).await, png);
        assert_eq!(events(&database).await, ["media.item_uploaded"]);

        let again = media.complete_upload(&tenant, &ada, upload.id).await.unwrap();
        assert!(!again.created);
        assert_eq!(again.uploaded.item, item);
        // Người khác không thấy lượt.
        let theirs = media.complete_upload(&tenant, &editor(), upload.id).await.unwrap_err();
        assert_eq!(theirs.code(), "media.upload_not_found");
    })
    .await
    .unwrap();
}

/// Đầu vào sai báo một lần; loại thật khác loại khai, SVG có script, sai hash: nội dung bị xoá.
#[tokio::test]
async fn loai_that_khac_khai_bao_bi_xoa() {
    TestDatabase::run(async |test| {
        let (media, storage, database) = service(test, limits()).await;
        storage.presign(true);
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();

        let bad = StartMediaUpload {
            name: String::new(),
            content_type: "application/octet-stream".to_owned(),
            size: 0,
            sha256: "x".to_owned(),
            ..StartMediaUpload::default()
        };
        let error = media.start_upload(&tenant, &ada, bad).await.unwrap_err();
        let fields: Vec<&str> = error.fields().iter().map(|field| field.field.as_str()).collect();
        assert_eq!(fields, ["name", "content_type", "size", "sha256"]);
        let zip = start("a.zip", "application/zip", b"PK\x03\x04");
        let error = media.start_upload(&tenant, &ada, zip).await.unwrap_err();
        assert_eq!(error.fields()[0].code, "not_allowed");

        // Khai PNG, gửi JPEG.
        let jpeg = samples::jpeg(32, 32);
        let (upload, link) =
            media.start_upload(&tenant, &ada, start("a.png", "image/png", &jpeg)).await.unwrap();
        storage.put_presigned(&link.url, jpeg.clone());
        let error = media.complete_upload(&tenant, &ada, upload.id).await.unwrap_err();
        assert_eq!(
            (error.kind(), error.code()),
            (ErrorKind::UnsupportedMediaType, "media.content_mismatch")
        );
        assert!(storage.bytes(link.url.strip_prefix("memory://").unwrap()).is_none());

        // SVG có nội dung chạy được.
        let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\" onload=\"x()\"/>";
        let (upload, link) =
            media.start_upload(&tenant, &ada, start("a.svg", "image/svg+xml", svg)).await.unwrap();
        storage.put_presigned(&link.url, &svg[..]);
        let error = media.complete_upload(&tenant, &ada, upload.id).await.unwrap_err();
        assert_eq!(error.code(), "media.svg_active_content");

        // Đúng loại, sai nội dung (hash): xoá; PUT lại đúng file thì xác nhận được.
        let png = samples::png(16, 16);
        let (upload, link) =
            media.start_upload(&tenant, &ada, start("b.png", "image/png", &png)).await.unwrap();
        storage.put_presigned(&link.url, samples::png(17, 16));
        let error = media.complete_upload(&tenant, &ada, upload.id).await.unwrap_err();
        assert_eq!((error.kind(), error.code()), (ErrorKind::Validation, "media.upload_mismatch"));
        storage.put_presigned(&link.url, png);
        assert!(media.complete_upload(&tenant, &ada, upload.id).await.unwrap().created);
        assert_eq!(events(&database).await, ["media.item_uploaded"]);
    })
    .await
    .unwrap();
}

/// Nhiều lệnh xác nhận cùng lúc: đúng một item — trên cả hai database.
#[tokio::test]
async fn xac_nhan_dong_thoi_chi_mot_item() {
    TestDatabase::run(async |test| {
        let (media, storage, database) = service(test, limits()).await;
        storage.presign(true);
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let png = samples::png(8, 8);
        let (upload, link) =
            media.start_upload(&tenant, &ada, start("a.png", "image/png", &png)).await.unwrap();
        storage.put_presigned(&link.url, png);

        let attempts: Vec<_> = (0..5)
            .map(|_| {
                let (media, ada, tenant) = (media.clone(), ada.clone(), tenant.clone());
                tokio::spawn(async move { media.complete_upload(&tenant, &ada, upload.id).await })
            })
            .collect();
        let mut created = 0;
        for attempt in attempts {
            let confirmed = attempt.await.unwrap().unwrap();
            assert_eq!(confirmed.uploaded.item.id, upload.id);
            created += usize::from(confirmed.created);
        }
        assert_eq!(created, 1);
        assert_eq!(events(&database).await, ["media.item_uploaded"]);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn luot_bo_do_duoc_don() {
    TestDatabase::run(async |test| {
        let (media, storage, database) = service(test, limits()).await;
        storage.presign(true);
        let tenant = TenantContext::default_on(database.clone());
        let registry = DatabaseRegistry::single(database.clone());
        let ada = editor();
        let png = samples::png(8, 8);
        let (upload, link) =
            media.start_upload(&tenant, &ada, start("a.png", "image/png", &png)).await.unwrap();
        storage.put_presigned(&link.url, png);

        assert_eq!(media.purge_uploads(&registry, &database, Utc::now()).await.unwrap(), 0);
        let later = Utc::now() + chrono::Duration::hours(3);
        assert_eq!(media.purge_uploads(&registry, &database, later).await.unwrap(), 1);
        assert_eq!(media.purge_uploads(&registry, &database, later).await.unwrap(), 0);
        assert!(storage.bytes(link.url.strip_prefix("memory://").unwrap()).is_none());
        let gone = media.complete_upload(&tenant, &ada, upload.id).await.unwrap_err();
        assert_eq!(gone.code(), "media.upload_not_found");
    })
    .await
    .unwrap();
}
