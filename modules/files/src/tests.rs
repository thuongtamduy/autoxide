//! Test của nơi lưu: chạy trên ổ đĩa, và trên S3 khi `TEST_S3_ENDPOINT` được đặt (gate
//! luôn đặt).

mod dormant;

use autoxide_kernel::config::{ConfigReader, ConfigSource};
use autoxide_kernel::error::Error;
use autoxide_kernel::storage::{ByteStream, Storage};
use bytes::Bytes;
use futures_util::{StreamExt as _, TryStreamExt as _};

use crate::config::FilesConfig;
use crate::store::ObjectStorage;

/// Các nơi lưu cần thử: một thư mục tạm, và S3 nếu có.
fn backends() -> Vec<(&'static str, ObjectStorage)> {
    let dir = std::env::temp_dir().join(format!("autoxide-files-{}", uuid_like()));
    let mut settings = vec![vec![("AUTOXIDE_FILES_DIR", dir.display().to_string())]];
    if let Some(s3) = autoxide_testkit::s3_settings() {
        settings.push(s3);
    }
    settings
        .into_iter()
        .map(|pairs| {
            let source = ConfigSource::from_pairs(pairs.iter().map(|(k, v)| (*k, v.as_str())));
            let mut reader = ConfigReader::new(&source);
            let config = FilesConfig::read(&mut reader);
            let (config, _) = reader.finish(config).unwrap();
            let name = if pairs.len() == 1 { "local" } else { "s3" };
            (name, ObjectStorage::open(&config.backend).unwrap())
        })
        .collect()
}

fn uuid_like() -> String {
    format!("{:x}", std::time::SystemTime::UNIX_EPOCH.elapsed().unwrap().as_nanos())
}

fn stream(chunks: Vec<Result<Bytes, Error>>) -> ByteStream {
    futures_util::stream::iter(chunks).boxed()
}

async fn read_all(storage: &ObjectStorage, key: &str) -> Option<Vec<u8>> {
    let file = storage.get(key).await.unwrap()?;
    let chunks: Vec<Bytes> = file.content.try_collect().await.unwrap();
    let content = chunks.concat();
    assert_eq!(file.size, content.len() as u64);
    Some(content)
}

#[tokio::test]
async fn ghi_doc_ghi_de_va_xoa() {
    for (name, storage) in backends() {
        let chunks = vec![Ok(Bytes::from_static(b"hello ")), Ok(Bytes::from_static(b"world"))];
        assert_eq!(storage.put("notes/a.txt", stream(chunks)).await.unwrap(), 11, "{name}");
        assert_eq!(read_all(&storage, "notes/a.txt").await.unwrap(), b"hello world", "{name}");

        storage.put("notes/a.txt", stream(vec![Ok(Bytes::from_static(b"new"))])).await.unwrap();
        assert_eq!(read_all(&storage, "notes/a.txt").await.unwrap(), b"new", "{name}");

        storage.put("notes/b/c.txt", stream(vec![Ok(Bytes::from_static(b"c"))])).await.unwrap();
        assert_eq!(storage.keys().await.unwrap(), ["notes/a.txt", "notes/b/c.txt"], "{name}");
        storage.delete("notes/b/c.txt").await.unwrap();
        storage.delete("notes/a.txt").await.unwrap();
        assert!(storage.get("notes/a.txt").await.unwrap().is_none(), "{name}");
        assert!(storage.keys().await.unwrap().is_empty(), "{name}");
        // Xoá thứ không có: coi như xong.
        storage.delete("notes/a.txt").await.unwrap();
        storage.put("empty", stream(Vec::new())).await.unwrap();
        assert_eq!(read_all(&storage, "empty").await.unwrap(), b"", "{name}");
    }
}

/// File lớn hơn một phần (multipart trên S3) đi qua từng đoạn và đọc lại đúng.
#[tokio::test]
async fn file_lon_ghi_tung_phan() {
    for (name, storage) in backends() {
        let chunk: Vec<u8> = (0..=255u8).cycle().take(1024 * 1024).collect();
        let chunks = (0..10).map(|_| Ok(Bytes::from(chunk.clone()))).collect();

        let written = storage.put("big.bin", stream(chunks)).await.unwrap();

        assert_eq!(written, 10 * 1024 * 1024, "{name}");
        let content = read_all(&storage, "big.bin").await.unwrap();
        assert_eq!(content.len(), 10 * 1024 * 1024, "{name}");
        assert!(content.chunks(chunk.len()).all(|part| part == chunk.as_slice()), "{name}");
    }
}

/// Luồng báo lỗi giữa chừng (vd quá giới hạn): lỗi đó được trả lại, không có file nào.
#[tokio::test]
async fn luong_loi_thi_khong_ghi_gi() {
    for (name, storage) in backends() {
        let big = Bytes::from(vec![7u8; 9 * 1024 * 1024]);
        let failing = vec![Ok(big), Err(Error::internal("the client went away"))];

        let error = storage.put("broken.bin", stream(failing)).await.unwrap_err();

        assert!(format!("{error:?}").contains("the client went away"), "{name}: {error:?}");
        assert!(storage.get("broken.bin").await.unwrap().is_none(), "{name}");
    }
}

#[tokio::test]
async fn khoa_sai_dang_bi_tu_choi() {
    for (name, storage) in backends() {
        for key in ["", "../etc/passwd", "a//b", "a/./b", "a b", "/abs"] {
            assert!(storage.get(key).await.is_err(), "{name}: `{key}` accepted");
        }
    }
}

/// Đọc theo khoảng byte và liệt kê theo tiền tố — trên ổ đĩa và S3.
#[tokio::test]
async fn doc_theo_khoang_va_liet_ke_theo_tien_to() {
    for (name, storage) in backends() {
        let chunks = vec![Ok(Bytes::from_static(b"0123456789"))];
        storage.put("media/a/1", stream(chunks)).await.unwrap();
        storage.put("media/b/2", stream(vec![Ok(Bytes::from_static(b"x"))])).await.unwrap();
        storage.put("files/c", stream(vec![Ok(Bytes::from_static(b"y"))])).await.unwrap();
        storage.put("mediax", stream(vec![Ok(Bytes::from_static(b"z"))])).await.unwrap();

        let part = storage.get_range("media/a/1", 2..6).await.unwrap().unwrap();
        assert_eq!(part.size, 4, "{name}");
        let chunks: Vec<Bytes> = part.content.try_collect().await.unwrap();
        assert_eq!(chunks.concat(), b"2345", "{name}");
        assert!(storage.get_range("media/none", 0..1).await.unwrap().is_none(), "{name}");

        assert_eq!(
            storage.keys_with_prefix("media/").await.unwrap(),
            ["media/a/1", "media/b/2"],
            "{name}"
        );
        assert_eq!(storage.keys_with_prefix("media/a/").await.unwrap(), ["media/a/1"], "{name}");
        for key in ["media/a/1", "media/b/2", "files/c", "mediax"] {
            storage.delete(key).await.unwrap();
        }
    }
}

/// Request bị huỷ giữa lúc ghi (hết giờ, client ngắt): không để lại file dở. Trên S3, phần đã
/// gửi của multipart upload được huỷ (D-158).
#[tokio::test]
async fn ghi_bi_huy_giua_chung_khong_de_lai_gi() {
    for (name, storage) in backends() {
        let part: Vec<u8> = vec![7u8; 9 * 1024 * 1024];
        // Một phần lớn (đủ để S3 bắt đầu multipart), rồi luồng đứng im mãi.
        let content = futures_util::stream::iter(vec![Ok(Bytes::from(part))])
            .chain(futures_util::stream::pending())
            .boxed();
        let put = storage.put("cut.bin", content);
        let cut = tokio::time::timeout(std::time::Duration::from_millis(500), put).await;
        assert!(cut.is_err(), "{name}");
        // Cho việc huỷ chạy nền xong.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(storage.get("cut.bin").await.unwrap().is_none(), "{name}");
    }
}
