//! Test tải thẳng (bước 40 P4): trên nơi lưu giả cấp link (cả hai database), trên ổ đĩa (không
//! cấp link), và trên S3 thật khi `TEST_S3_ENDPOINT` được đặt (gate luôn đặt, dùng moto).

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::database::Database;
use autoxide_kernel::error::ErrorKind;
use autoxide_kernel::storage::{Storage, TenantStorage};
use autoxide_kernel::tenancy::{DatabaseRegistry, TenantContext};
use autoxide_testkit::{MemoryStorage, TestDatabase};
use sea_orm::{ConnectionTrait, Statement};
use sha2::{Digest, Sha256};

use super::*;
use crate::testing::{no_limits, service, service_on};

fn uploader() -> Principal {
    Principal::new(Uuid::now_v7(), ["files.upload".to_owned()])
}

fn sha(content: &[u8]) -> String {
    autoxide_kernel::upload::hex(&Sha256::digest(content))
}

fn request(name: &str, content: &[u8]) -> StartUpload {
    StartUpload {
        name: name.to_owned(),
        content_type: "text/plain".to_owned(),
        size: content.len() as u64,
        sha256: sha(content),
    }
}

/// Service trên nơi lưu giả cấp link.
async fn presigning(test: &TestDatabase) -> (Arc<FileService>, Arc<MemoryStorage>, Database) {
    let storage = Arc::new(MemoryStorage::new());
    storage.presign(true);
    let (files, database) = service_on(test, no_limits(), storage.clone()).await;
    (files, storage, database)
}

async fn count(database: &Database, sql: &str) -> i64 {
    let backend = database.read().get_database_backend();
    let row = database.read().query_one_raw(Statement::from_string(backend, sql)).await;
    row.unwrap().unwrap().try_get_by_index::<i64>(0).unwrap()
}

#[tokio::test]
async fn o_dia_khong_cap_link() {
    TestDatabase::run(async |test| {
        let (files, database) = service(test, no_limits()).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = uploader();

        let error = files.start_upload(&tenant, &ada, request("a.txt", b"hello")).await;
        let error = error.unwrap_err();
        assert_eq!(
            (error.kind(), error.code()),
            (ErrorKind::Conflict, "files.direct_upload_unavailable")
        );
        assert!(error.message().contains("POST /api/v1/files"), "{}", error.message());

        // Mọi lỗi của đầu vào báo một lần (D-083); thiếu permission báo trước tiên.
        let bad = StartUpload {
            name: "a/b".to_owned(),
            content_type: "pdf".to_owned(),
            size: 0,
            sha256: "abc".to_owned(),
        };
        let error = files.start_upload(&tenant, &ada, bad.clone()).await.unwrap_err();
        let fields: Vec<&str> = error.fields().iter().map(|field| field.field.as_str()).collect();
        assert_eq!(fields, ["name", "content_type", "size", "sha256"]);
        let nobody = Principal::new(Uuid::now_v7(), []);
        let denied = files.start_upload(&tenant, &nobody, bad).await.unwrap_err();
        assert_eq!(denied.code(), "permission_denied");
        assert_eq!(count(&database, "SELECT COUNT(*) FROM files_uploads").await, 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tai_thang_roi_xac_nhan_tao_ban_ghi() {
    TestDatabase::run(async |test| {
        let (files, storage, database) = presigning(test).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = uploader();

        let started =
            files.start_upload(&tenant, &ada, request("big.txt", b"hello")).await.unwrap();
        assert_eq!(started.link.method, "PUT");
        assert_eq!(started.link.url, format!("memory://files/{}", started.upload.id));
        assert!(started.link.headers.contains(&("content-type".into(), "text/plain".into())));
        let id = started.upload.id;

        // Chưa PUT: chưa xong.
        let early = files.complete_upload(&tenant, &ada, id).await.unwrap_err();
        assert_eq!(early.code(), "files.upload_incomplete");

        storage.put_presigned(&started.link.url, &b"hello"[..]);
        let Completed::Created(file) = files.complete_upload(&tenant, &ada, id).await.unwrap()
        else {
            panic!("expected a new file")
        };
        assert_eq!((file.id, file.size, file.name.as_str()), (id, 5, "big.txt"));
        assert_eq!(file.sha256, sha(b"hello"));
        // Bản ghi đọc và tải xuống được như file tải qua máy chủ.
        let download = files.download(&tenant, &ada, id, |_| false).await.unwrap().unwrap();
        assert_eq!(download.file, file);

        // Gọi lại: cùng bản ghi (agent hay retry).
        let Completed::Existing(again) = files.complete_upload(&tenant, &ada, id).await.unwrap()
        else {
            panic!("expected the same file")
        };
        assert_eq!(again, file);
        // Người khác không thấy lượt của ada.
        let theirs = files.complete_upload(&tenant, &uploader(), id).await.unwrap_err();
        assert_eq!(theirs.code(), "files.upload_not_found");
        let events = "SELECT COUNT(*) FROM files_events WHERE name = 'files.file_uploaded'";
        assert_eq!(count(&database, events).await, 1);
    })
    .await
    .unwrap();
}

/// File sai cỡ hay sai hash: bị xoá, lượt vẫn còn — PUT lại đúng file rồi xác nhận được.
#[tokio::test]
async fn sai_co_hay_hash_bi_xoa() {
    TestDatabase::run(async |test| {
        let (files, storage, database) = presigning(test).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = uploader();
        let started = files.start_upload(&tenant, &ada, request("a.txt", b"hello")).await.unwrap();
        let (id, url) = (started.upload.id, started.link.url.clone());

        for wrong in [&b"hello!"[..], &b"jello"[..]] {
            storage.put_presigned(&url, wrong);
            let error = files.complete_upload(&tenant, &ada, id).await.unwrap_err();
            assert_eq!(
                (error.kind(), error.code()),
                (ErrorKind::Validation, "files.upload_mismatch")
            );
            assert!(storage.bytes(&format!("files/{id}")).is_none());
        }
        let error = {
            storage.put_presigned(&url, &b"hello!"[..]);
            files.complete_upload(&tenant, &ada, id).await.unwrap_err()
        };
        assert!(error.message().contains("size is 6 bytes, not 5"), "{}", error.message());

        storage.put_presigned(&url, &b"hello"[..]);
        assert!(matches!(
            files.complete_upload(&tenant, &ada, id).await.unwrap(),
            Completed::Created(_)
        ));
    })
    .await
    .unwrap();
}

/// Nhiều lệnh xác nhận cùng lúc: đúng một bản ghi, mọi lệnh trả cùng file — trên cả hai database.
#[tokio::test]
async fn xac_nhan_dong_thoi_chi_mot_ban_ghi() {
    TestDatabase::run(async |test| {
        let (files, storage, database) = presigning(test).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = uploader();
        let started = files.start_upload(&tenant, &ada, request("a.txt", b"hello")).await.unwrap();
        storage.put_presigned(&started.link.url, &b"hello"[..]);
        let id = started.upload.id;

        let attempts: Vec<_> = (0..5)
            .map(|_| {
                let (files, ada, tenant) = (files.clone(), ada.clone(), tenant.clone());
                tokio::spawn(async move { files.complete_upload(&tenant, &ada, id).await })
            })
            .collect();
        let mut created = 0;
        for attempt in attempts {
            match attempt.await.unwrap().unwrap() {
                Completed::Created(file) => {
                    created += 1;
                    assert_eq!(file.id, id);
                }
                Completed::Existing(file) => assert_eq!(file.id, id),
            }
        }
        assert_eq!(created, 1);
        assert_eq!(count(&database, "SELECT COUNT(*) FROM files_file").await, 1);
        let events = "SELECT COUNT(*) FROM files_events WHERE name = 'files.file_uploaded'";
        assert_eq!(count(&database, events).await, 1);
    })
    .await
    .unwrap();
}

/// Lượt không xác nhận: hết hạn thì không xác nhận được nữa; job dọn xoá object và dòng, chạy
/// lại không sao; lượt đã xác nhận giữ nguyên file.
#[tokio::test]
async fn luot_het_han_duoc_don() {
    TestDatabase::run(async |test| {
        let (files, storage, database) = presigning(test).await;
        let tenant = TenantContext::default_on(database.clone());
        let registry = DatabaseRegistry::single(database.clone());
        let ada = uploader();
        let abandoned = files.start_upload(&tenant, &ada, request("a.txt", b"a")).await.unwrap();
        storage.put_presigned(&abandoned.link.url, &b"a"[..]);
        let kept = files.start_upload(&tenant, &ada, request("b.txt", b"b")).await.unwrap();
        storage.put_presigned(&kept.link.url, &b"b"[..]);
        files.complete_upload(&tenant, &ada, kept.upload.id).await.unwrap();

        // Chưa quá hạn: không đụng tới.
        assert_eq!(files.purge_uploads(&registry, &database, Timestamp::now()).await.unwrap(), 0);
        let later = Timestamp::now() + SignedDuration::from_hours(3);
        assert_eq!(files.purge_uploads(&registry, &database, later).await.unwrap(), 1);
        assert_eq!(files.purge_uploads(&registry, &database, later).await.unwrap(), 0);
        assert!(storage.bytes(&format!("files/{}", abandoned.upload.id)).is_none());
        assert!(storage.bytes(&format!("files/{}", kept.upload.id)).is_some());
        let gone = files.complete_upload(&tenant, &ada, abandoned.upload.id).await.unwrap_err();
        assert_eq!(gone.code(), "files.upload_not_found");
        // Một tuần sau: dòng của lượt đã xác nhận cũng được xoá, file vẫn còn.
        let week = Timestamp::now() + SignedDuration::from_hours(24 * 8);
        files.purge_uploads(&registry, &database, week).await.unwrap();
        assert_eq!(count(&database, "SELECT COUNT(*) FROM files_uploads").await, 0);
        assert!(files.get(&tenant, &ada, kept.upload.id).await.is_ok());
    })
    .await
    .unwrap();
}

/// Khoá lưu có tiền tố khách (D-259); khách B không xác nhận được lượt của khách A.
#[tokio::test]
async fn khoa_luu_co_tien_to_khach() {
    TestDatabase::run(async |test| {
        let (files, storage, database) = presigning(test).await;
        let other = TenantContext::new(
            autoxide_kernel::tenancy::TenantId::from_uuid(Uuid::now_v7()),
            "other",
            "Other",
            autoxide_kernel::tenancy::TenantStatus::Active,
            autoxide_kernel::tenancy::MAIN_PLACEMENT,
            database.clone(),
        );
        let ada = uploader();
        let started = files.start_upload(&other, &ada, request("a.txt", b"x")).await.unwrap();
        let prefix = format!("memory://tenants/{}/files/", other.id().as_uuid());
        assert!(started.link.url.starts_with(&prefix), "{}", started.link.url);

        let ours = TenantContext::default_on(database.clone());
        let error = files.complete_upload(&ours, &ada, started.upload.id).await.unwrap_err();
        assert_eq!(error.code(), "files.upload_not_found");
        storage.put_presigned(&started.link.url, &b"x"[..]);
        let store = TenantStorage::new(storage.clone(), &other);
        assert!(store.head(&format!("files/{}", started.upload.id)).await.unwrap().is_some());
        assert!(files.complete_upload(&other, &ada, started.upload.id).await.is_ok());
    })
    .await
    .unwrap();
}

/// Trên S3 thật (moto của gate): link ký kèm cỡ, loại và checksum; PUT bằng HTTP rồi xác nhận.
#[tokio::test]
async fn tai_thang_len_s3_that() {
    let Some(settings) = autoxide_testkit::s3_settings() else {
        return;
    };
    TestDatabase::run(async |test| {
        let source = autoxide_kernel::config::ConfigSource::from_pairs(
            settings.iter().map(|(name, value)| (*name, value.as_str())),
        );
        let mut reader = autoxide_kernel::config::ConfigReader::new(&source);
        let config = crate::config::FilesConfig::read(&mut reader);
        let (config, _) = reader.finish(config).unwrap();
        let storage: Arc<dyn Storage> =
            Arc::new(crate::store::ObjectStorage::open(&config.backend).unwrap());
        let (files, database) = service_on(test, no_limits(), storage.clone()).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = uploader();
        let content: Vec<u8> = (0..=255u8).cycle().take(300_000).collect();

        let started =
            files.start_upload(&tenant, &ada, request("data.bin", &content)).await.unwrap();
        let url = &started.link.url;
        assert!(url.contains("X-Amz-Signature="), "{url}");
        assert!(url.contains("x-amz-checksum-sha256"), "{url}");
        let headers: Vec<&str> =
            started.link.headers.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(headers, ["content-length", "content-type", "x-amz-checksum-sha256"]);

        autoxide_kernel::tls::install_crypto_provider();
        let client = reqwest::Client::new();
        let mut put = client.put(url).body(content.clone());
        for (name, value) in &started.link.headers {
            put = put.header(name, value);
        }
        let response = put.send().await.unwrap();
        assert!(response.status().is_success(), "{}", response.status());

        let key = format!("files/{}", started.upload.id);
        assert_eq!(storage.head(&key).await.unwrap().unwrap().size, content.len() as u64);
        let Completed::Created(file) =
            files.complete_upload(&tenant, &ada, started.upload.id).await.unwrap()
        else {
            panic!("expected a new file")
        };
        assert_eq!(file.sha256, sha(&content));
        assert!(storage.head("files/none").await.unwrap().is_none());
        storage.delete(&key).await.unwrap();
    })
    .await
    .unwrap();
}
