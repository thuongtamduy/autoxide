//! Test mức service của module `media`. Gate chạy trên cả SQLite và Postgres.

use autoxide_kernel::database::Database;
use autoxide_kernel::error::ErrorKind;
use autoxide_kernel::event::{Actor, Delivered, EventHandler};
use autoxide_kernel::pagination::{Order, PageQuery, PageRequest};
use autoxide_kernel::privacy::PersonalData;
use autoxide_kernel::storage::Storage;
use autoxide_testkit::TestDatabase;
use chrono::Utc;
use futures_util::TryStreamExt as _;
use sea_orm::{ConnectionTrait, Statement};
use serde_json::json;
use uuid::Uuid;

use crate::api::{Media, MediaVisibility};
use crate::folders::{FolderChanges, FolderSort};
use crate::service::{FolderFilter, Item, ItemChanges, ItemFilter, ItemSort, Visibility};
use crate::sniff::samples;
use crate::testing::{body, editor, limits, service, user};
use crate::upload::NewUpload;

fn upload(name: &str, content_type: &str) -> NewUpload {
    NewUpload {
        name: name.to_owned(),
        folder_id: None,
        content_type: Some(content_type.to_owned()),
        tags: Vec::new(),
    }
}

async fn events(database: &Database) -> Vec<String> {
    let backend = database.read().get_database_backend();
    let sql = "SELECT name FROM media_events ORDER BY id";
    let rows = database.read().query_all_raw(Statement::from_string(backend, sql)).await;
    rows.unwrap().iter().map(|row| row.try_get("", "name").unwrap()).collect()
}

async fn content(service: &crate::service::MediaService, item: &Item) -> Vec<u8> {
    let stored = service.read(item, None).await.unwrap();
    let chunks: Vec<bytes::Bytes> = stored.content.try_collect().await.unwrap();
    chunks.concat()
}

fn page(limit: u32) -> PageRequest {
    PageRequest::first(limit, Order::Asc)
}

/// Mỗi định dạng ảnh: nội dung, SHA-256, loại và kích thước đúng; loại không khai thì máy
/// chủ tự nhận.
#[tokio::test]
async fn tai_len_moi_dinh_dang_anh() {
    TestDatabase::run(async |test| {
        let (media, _, database) = service(test, limits()).await;
        let ada = editor();
        let cases = [
            ("a.png", "image/png", samples::png(640, 480), (640, 480)),
            ("a.jpg", "image/jpg", samples::jpeg(1920, 1080), (1920, 1080)),
            ("a.gif", "image/gif", samples::gif(32, 16), (32, 16)),
            ("a.webp", "image/webp", samples::webp(300, 200), (300, 200)),
            ("a.avif", "image/avif", samples::avif(4000, 3000), (4000, 3000)),
        ];
        for (name, declared, bytes, (width, height)) in cases {
            let uploaded = media.upload(&ada, upload(name, declared), body(&bytes)).await.unwrap();
            let item = uploaded.item;
            assert_eq!((item.width, item.height), (Some(width), Some(height)), "{name}");
            assert_eq!(item.size, bytes.len() as i64);
            assert_eq!(content(&media, &item).await, bytes);
            assert_eq!(item.sha256.len(), 64);
            assert_eq!(item.visibility, "private");
            assert_eq!(item.uploaded_by, Some(ada.user_id()));
        }
        let jpeg = media.list(&ada, &ItemFilter::default(), &page(10)).await.unwrap();
        assert_eq!(
            jpeg.data[1].content_type, "image/jpeg",
            "the alias is stored by its standard name"
        );

        // Không khai loại: tự nhận.
        let auto = NewUpload {
            name: "doc.pdf".into(),
            folder_id: None,
            content_type: None,
            tags: Vec::new(),
        };
        let pdf = media.upload(&ada, auto, body(samples::PDF)).await.unwrap().item;
        assert_eq!((pdf.content_type.as_str(), pdf.width), ("application/pdf", None));
        let octet = upload("clip.mp4", "application/octet-stream");
        assert_eq!(
            media.upload(&ada, octet, body(samples::MP4)).await.unwrap().item.content_type,
            "video/mp4"
        );
        assert_eq!(events(&database).await.len(), 7);
        assert!(events(&database).await.iter().all(|name| name == "media.item_uploaded"));
    })
    .await
    .unwrap();
}

/// Khai sai loại, loại không được phép, quá cỡ, ảnh hỏng, rỗng: từ chối, không để lại gì.
#[tokio::test]
async fn noi_dung_sai_bi_tu_choi_khong_de_lai_gi() {
    TestDatabase::run(async |test| {
        let mut small = limits();
        small.max_bytes = 2000;
        let (media, storage, database) = service(test, small).await;
        let ada = editor();
        let refuse = async |name: &str, declared: Option<&str>, bytes: &[u8]| {
            let input = NewUpload {
                name: name.into(),
                folder_id: None,
                content_type: declared.map(str::to_owned),
                tags: Vec::new(),
            };
            let error = media.upload(&ada, input, body(bytes)).await.unwrap_err();
            (error.kind(), error.code().to_owned())
        };
        let mismatch = (ErrorKind::UnsupportedMediaType, "media.content_mismatch".to_owned());
        assert_eq!(refuse("x.png", Some("image/png"), samples::HTML).await, mismatch);
        assert_eq!(refuse("x.png", Some("image/png"), &samples::jpeg(1, 1)).await, mismatch);
        assert_eq!(refuse("x.png", Some("image/png"), b"just text").await, mismatch);
        assert_eq!(refuse("x.svg", Some("image/svg+xml"), samples::HTML).await, mismatch);
        // Nội dung dài (quá `SNIFF_BYTES`): bị dừng giữa đường.
        let mut long_html = samples::HTML.to_vec();
        long_html.extend(vec![b' '; 1500]);
        assert_eq!(refuse("x.png", Some("image/png"), &long_html).await, mismatch);
        assert_eq!(
            refuse("x.html", Some("text/html"), samples::HTML).await,
            (ErrorKind::UnsupportedMediaType, "media.type_not_allowed".to_owned())
        );
        assert_eq!(
            refuse("x", None, samples::HTML).await,
            (ErrorKind::UnsupportedMediaType, "media.type_not_allowed".to_owned())
        );
        assert_eq!(
            refuse("x", None, b"plain words").await,
            (ErrorKind::UnsupportedMediaType, "media.unknown_type".to_owned())
        );
        assert_eq!(
            refuse("x.png", Some("image/png"), &[samples::png(1, 1), vec![0; 2000]].concat()).await,
            (ErrorKind::PayloadTooLarge, "media.too_large".to_owned())
        );
        assert_eq!(
            refuse("x.png", Some("image/png"), b"\x89PNG\r\n\x1a\n").await,
            (ErrorKind::UnsupportedMediaType, "media.invalid_image".to_owned())
        );
        assert_eq!(refuse("x.png", Some("image/png"), b"").await.1, "validation_failed");
        assert_eq!(refuse("x.png", Some("png"), b"x").await.1, "media.invalid_content_type");
        // Tên sai và thư mục không có: báo cùng lúc, trước khi đọc nội dung.
        let input = NewUpload {
            name: " ".into(),
            folder_id: Some(Uuid::now_v7()),
            content_type: Some("image/png".into()),
            tags: Vec::new(),
        };
        let error = media.upload(&ada, input, body(&samples::png(1, 1))).await.unwrap_err();
        let fields: Vec<&str> = error.fields().iter().map(|field| field.field.as_str()).collect();
        assert_eq!(fields, ["name", "folder_id"]);
        // Không có quyền.
        let reader = user(&["read"]);
        let denied = media.upload(&reader, upload("a.png", "image/png"), body(&samples::png(1, 1)));
        assert_eq!(denied.await.unwrap_err().code(), "permission_denied");

        assert!(storage.keys().await.unwrap().is_empty());
        assert!(events(&database).await.is_empty());
    })
    .await
    .unwrap();
}

/// Tải lên nội dung đã có: item mới vẫn tạo, kèm danh sách item trùng (ngoài thùng rác).
#[tokio::test]
async fn bao_item_trung_noi_dung() {
    TestDatabase::run(async |test| {
        let (media, _, _) = service(test, limits()).await;
        let ada = editor();
        let png = samples::png(2, 2);
        let first = media.upload(&ada, upload("a.png", "image/png"), body(&png)).await.unwrap();
        assert!(first.duplicates.is_empty());
        let second = media.upload(&ada, upload("b.png", "image/png"), body(&png)).await.unwrap();
        assert_eq!(
            second.duplicates.iter().map(|item| item.id).collect::<Vec<_>>(),
            [first.item.id]
        );
        media.trash(&ada, first.item.id).await.unwrap();
        let third = media.upload(&ada, upload("c.png", "image/png"), body(&png)).await.unwrap();
        assert_eq!(
            third.duplicates.iter().map(|item| item.id).collect::<Vec<_>>(),
            [second.item.id]
        );
    })
    .await
    .unwrap();
}

/// Thay nội dung: giữ ID, đổi phiên bản, nội dung cũ bị xoá; `If-Match` cũ → 412.
#[tokio::test]
async fn thay_noi_dung_giu_id() {
    TestDatabase::run(async |test| {
        let (media, storage, database) = service(test, limits()).await;
        let ada = editor();
        let old = media.upload(&ada, upload("a.png", "image/png"), body(&samples::png(1, 1))).await;
        let old = old.unwrap().item;

        let new = media
            .replace(
                &ada,
                old.id,
                Some("image/webp"),
                body(&samples::webp(8, 6)),
                Some(old.updated_at),
            )
            .await
            .unwrap();
        assert_eq!(new.id, old.id);
        assert_ne!(new.version_id, old.version_id);
        assert_ne!(new.sha256, old.sha256);
        assert_eq!(
            (new.content_type.as_str(), new.width, new.height),
            ("image/webp", Some(8), Some(6))
        );
        assert_eq!(storage.keys().await.unwrap(), [new.storage_key()]);
        assert_eq!(content(&media, &new).await, samples::webp(8, 6));

        // Phiên bản cũ: 412, không để lại nội dung mới nào.
        let stale = media
            .replace(
                &ada,
                old.id,
                Some("image/png"),
                body(&samples::png(2, 2)),
                Some(old.updated_at),
            )
            .await
            .unwrap_err();
        assert_eq!(stale.kind(), ErrorKind::PreconditionFailed);
        // Nội dung sai: không đổi gì.
        let wrong = media.replace(&ada, old.id, Some("image/png"), body(samples::HTML), None).await;
        assert_eq!(wrong.unwrap_err().code(), "media.content_mismatch");
        assert_eq!(storage.keys().await.unwrap(), [new.storage_key()]);
        assert_eq!(events(&database).await, ["media.item_uploaded", "media.item_content_replaced"]);
    })
    .await
    .unwrap();
}

/// Hai lệnh thay nội dung cùng lúc với cùng `If-Match`: đúng một thành công, bên kia 412;
/// chỉ còn đúng nội dung của bên thắng — trên cả hai database.
#[tokio::test]
async fn thay_noi_dung_dong_thoi_chi_mot_ben_thang() {
    TestDatabase::run(async |test| {
        let (media, storage, _) = service(test, limits()).await;
        let ada = editor();
        let item =
            media.upload(&ada, upload("a.png", "image/png"), body(&samples::png(1, 1))).await;
        let item = item.unwrap().item;

        let attempts: Vec<_> = (1..=4u32)
            .map(|size| {
                let (media, ada) = (media.clone(), ada.clone());
                tokio::spawn(async move {
                    let png = samples::png(size, size);
                    media
                        .replace(
                            &ada,
                            item.id,
                            Some("image/png"),
                            body(&png),
                            Some(item.updated_at),
                        )
                        .await
                })
            })
            .collect();
        let mut winners = Vec::new();
        for attempt in attempts {
            match attempt.await.unwrap() {
                Ok(item) => winners.push(item),
                Err(error) => assert_eq!(error.kind(), ErrorKind::PreconditionFailed),
            }
        }
        assert_eq!(winners.len(), 1);
        assert_eq!(storage.keys().await.unwrap(), [winners[0].storage_key()]);
    })
    .await
    .unwrap();
}

/// Sửa thông tin: kiểm mọi trường một lần; item trong thùng rác phải khôi phục trước.
#[tokio::test]
async fn sua_thong_tin_item() {
    TestDatabase::run(async |test| {
        let (media, _, _) = service(test, limits()).await;
        let ada = editor();
        let item =
            media.upload(&ada, upload("a.png", "image/png"), body(&samples::png(1, 1))).await;
        let item = item.unwrap().item;
        let folder = media.create_folder(&ada, "Logos", None).await.unwrap();

        let changes = ItemChanges {
            name: Some(" Logo.png ".into()),
            alt: Some("Company logo".into()),
            caption: Some("Blue".into()),
            folder_id: Some(Some(folder.id)),
            visibility: Some(Visibility::Public),
            ..Default::default()
        };
        let changed = media.update(&ada, item.id, changes, Some(item.updated_at)).await.unwrap();
        assert_eq!((changed.name.as_str(), changed.name_key.as_str()), ("Logo.png", "logo.png"));
        assert_eq!((changed.alt.as_str(), changed.visibility.as_str()), ("Company logo", "public"));
        assert_eq!(changed.folder_id, folder.id);
        // Về gốc.
        let root = ItemChanges { folder_id: Some(None), ..Default::default() };
        let moved = media.update(&ada, item.id, root, None).await.unwrap();
        assert_eq!(moved.folder_id, crate::entity::ROOT);
        // Phiên bản cũ.
        let stale =
            media.update(&ada, item.id, ItemChanges::default(), Some(item.updated_at)).await;
        assert_eq!(stale.unwrap_err().kind(), ErrorKind::PreconditionFailed);

        let invalid = ItemChanges {
            name: Some("a/b".into()),
            alt: Some("x".repeat(1001)),
            folder_id: Some(Some(Uuid::now_v7())),
            ..Default::default()
        };
        let error = media.update(&ada, item.id, invalid, None).await.unwrap_err();
        let fields: Vec<&str> = error.fields().iter().map(|field| field.field.as_str()).collect();
        assert_eq!(fields, ["name", "alt", "folder_id"]);

        media.trash(&ada, item.id).await.unwrap();
        let trashed = media.update(&ada, item.id, ItemChanges::default(), None).await;
        assert_eq!(trashed.unwrap_err().code(), "media.item_in_trash");
        let missing = media.update(&ada, Uuid::now_v7(), ItemChanges::default(), None).await;
        assert_eq!(missing.unwrap_err().code(), "media.item_not_found");
    })
    .await
    .unwrap();
}

/// Danh sách: từng bộ lọc, từng cách sắp, lật trang, tìm theo tên.
#[tokio::test]
async fn danh_sach_loc_sap_lat_trang() {
    TestDatabase::run(async |test| {
        let (media, _, _) = service(test, limits()).await;
        let ada = editor();
        let folder = media.create_folder(&ada, "Docs", None).await.unwrap();
        let mut ids = Vec::new();
        for (name, declared, bytes, folder_id) in [
            ("Banner.png", "image/png", samples::png(5, 5), None),
            ("apple.jpg", "image/jpeg", [samples::jpeg(2, 2), vec![0; 50]].concat(), None),
            ("Report 100%.pdf", "application/pdf", samples::PDF.to_vec(), Some(folder.id)),
            ("cat.gif", "image/gif", samples::gif(1, 1), Some(folder.id)),
        ] {
            let input = NewUpload {
                name: name.into(),
                folder_id,
                content_type: Some(declared.into()),
                tags: Vec::new(),
            };
            ids.push(media.upload(&ada, input, body(&bytes)).await.unwrap().item.id);
        }
        let public = ItemChanges { visibility: Some(Visibility::Public), ..Default::default() };
        media.update(&ada, ids[1], public, None).await.unwrap();
        media.trash(&ada, ids[3]).await.unwrap();

        let names = async |filter: ItemFilter| -> Vec<String> {
            let page = media.list(&ada, &filter, &page(50)).await.unwrap();
            page.data.into_iter().map(|item| item.name).collect()
        };
        assert_eq!(
            names(ItemFilter::default()).await,
            ["Banner.png", "apple.jpg", "Report 100%.pdf"]
        );
        let root = ItemFilter { folder: FolderFilter::Root, ..Default::default() };
        assert_eq!(names(root).await, ["Banner.png", "apple.jpg"]);
        let in_folder = ItemFilter { folder: FolderFilter::Id(folder.id), ..Default::default() };
        assert_eq!(names(in_folder).await, ["Report 100%.pdf"]);
        let images = ItemFilter { content_type: Some("image/*".into()), ..Default::default() };
        assert_eq!(names(images).await, ["Banner.png", "apple.jpg"]);
        let pdf = ItemFilter { content_type: Some("application/pdf".into()), ..Default::default() };
        assert_eq!(names(pdf).await, ["Report 100%.pdf"]);
        let shared = ItemFilter { visibility: Some(Visibility::Public), ..Default::default() };
        assert_eq!(names(shared).await, ["apple.jpg"]);
        let trash = ItemFilter { trashed: true, ..Default::default() };
        assert_eq!(names(trash).await, ["cat.gif"]);
        let search = ItemFilter { search: Some("REPORT 100%".into()), ..Default::default() };
        assert_eq!(names(search).await, ["Report 100%.pdf"]);
        // Không phân biệt dấu (tìm "report" ra "Report", và ngược lại).
        let plain = ItemFilter { search: Some("rëport".into()), ..Default::default() };
        assert_eq!(names(plain).await, ["Report 100%.pdf"]);
        // `%` gõ vào là chữ, không phải ký tự đại diện.
        let literal = ItemFilter { search: Some("%".into()), ..Default::default() };
        assert_eq!(names(literal).await, ["Report 100%.pdf"]);
        let by_name = ItemFilter { sort: ItemSort::Name, ..Default::default() };
        assert_eq!(names(by_name).await, ["apple.jpg", "Banner.png", "Report 100%.pdf"]);
        let by_size = ItemFilter { sort: ItemSort::Size, ..Default::default() };
        // PDF mẫu 41 byte, PNG mẫu 45 byte, JPEG mẫu + 50 byte đệm.
        assert_eq!(names(by_size).await, ["Report 100%.pdf", "Banner.png", "apple.jpg"]);

        // Lật trang theo tên, giảm dần, mỗi trang một item.
        let filter = ItemFilter { sort: ItemSort::Name, ..Default::default() };
        let mut cursor = None;
        let mut seen = Vec::new();
        loop {
            let query = PageQuery { limit: Some("1".into()), cursor, order: Some("desc".into()) };
            let request = PageRequest::try_from(query).unwrap();
            let page = media.list(&ada, &filter, &request).await.unwrap();
            seen.extend(page.data.into_iter().map(|item| item.name));
            match page.pagination.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        assert_eq!(seen, ["Report 100%.pdf", "Banner.png", "apple.jpg"]);
        let denied = media.list(&user(&["write"]), &ItemFilter::default(), &page(5)).await;
        assert_eq!(denied.unwrap_err().code(), "permission_denied");
    })
    .await
    .unwrap();
}

/// Thùng rác: vào, khôi phục, xoá hẳn (permission riêng, phải ở thùng rác trước).
#[tokio::test]
async fn thung_rac_vao_ra_va_xoa_han() {
    TestDatabase::run(async |test| {
        let (media, storage, database) = service(test, limits()).await;
        let writer = user(&["read", "write"]);
        let item =
            media.upload(&writer, upload("a.png", "image/png"), body(&samples::png(1, 1))).await;
        let item = item.unwrap().item;

        let purge = media.purge(&editor(), item.id).await.unwrap_err();
        assert_eq!(purge.code(), "media.item_not_in_trash");
        let trashed = media.trash(&writer, item.id).await.unwrap();
        assert!(trashed.trashed_at.is_some());
        assert_eq!(media.trash(&writer, item.id).await.unwrap_err().code(), "media.item_in_trash");
        let restored = media.restore(&writer, item.id).await.unwrap();
        assert!(restored.trashed_at.is_none());
        assert_eq!(
            media.restore(&writer, item.id).await.unwrap_err().code(),
            "media.item_not_in_trash"
        );
        media.trash(&writer, item.id).await.unwrap();
        assert_eq!(media.purge(&writer, item.id).await.unwrap_err().code(), "permission_denied");
        media.purge(&editor(), item.id).await.unwrap();

        assert_eq!(media.get(&writer, item.id).await.unwrap_err().code(), "media.item_not_found");
        assert!(storage.keys().await.unwrap().is_empty());
        assert_eq!(
            events(&database).await,
            [
                "media.item_uploaded",
                "media.item_trashed",
                "media.item_restored",
                "media.item_trashed",
                "media.item_purged"
            ]
        );
    })
    .await
    .unwrap();
}

/// Ai tải được nội dung: item công khai ngoài thùng rác thì ai cũng được; còn lại cần
/// `media.read`; không được thì như không có.
#[tokio::test]
async fn quyen_tai_noi_dung() {
    TestDatabase::run(async |test| {
        let (media, _, _) = service(test, limits()).await;
        let ada = editor();
        let item =
            media.upload(&ada, upload("a.png", "image/png"), body(&samples::png(1, 1))).await;
        let item = item.unwrap().item;
        let reader = user(&["read"]);
        let stranger = user(&[]);

        assert!(media.viewable(None, item.id).await.is_err());
        assert!(media.viewable(Some(&stranger), item.id).await.is_err());
        assert!(media.viewable(Some(&reader), item.id).await.is_ok());
        let public = ItemChanges { visibility: Some(Visibility::Public), ..Default::default() };
        media.update(&ada, item.id, public, None).await.unwrap();
        assert!(media.viewable(None, item.id).await.is_ok());
        media.trash(&ada, item.id).await.unwrap();
        let hidden = media.viewable(None, item.id).await.unwrap_err();
        assert_eq!(hidden.code(), "media.item_not_found");
        assert!(media.viewable(Some(&reader), item.id).await.is_ok());
    })
    .await
    .unwrap();
}

/// Thư mục: lồng nhau, tên trùng trong cùng cha bị chặn (kể cả ở gốc, kể cả khác hoa
/// thường), xoá thư mục còn đồ → 409, không chuyển vào chính nó hay con của nó.
#[tokio::test]
async fn thu_muc_long_nhau() {
    TestDatabase::run(async |test| {
        let (media, _, database) = service(test, limits()).await;
        let ada = editor();
        let photos = media.create_folder(&ada, "Photos", None).await.unwrap();
        let taken = media.create_folder(&ada, "photos", None).await.unwrap_err();
        assert_eq!(taken.code(), "media.folder_name_taken");
        let summer = media.create_folder(&ada, "Summer", Some(photos.id)).await.unwrap();
        let deep = media.create_folder(&ada, "Beach", Some(summer.id)).await.unwrap();
        // Cùng tên, khác cha: được.
        media.create_folder(&ada, "Summer", None).await.unwrap();
        let ghost = media.create_folder(&ada, "x", Some(Uuid::now_v7())).await.unwrap_err();
        assert_eq!(ghost.fields()[0].field, "parent_id");

        let children = media
            .list_folders(&ada, FolderFilter::Id(photos.id), FolderSort::Name, &page(10))
            .await
            .unwrap();
        assert_eq!(children.data.iter().map(|f| f.id).collect::<Vec<_>>(), [summer.id]);
        let roots = media.list_folders(&ada, FolderFilter::Root, FolderSort::Name, &page(10)).await;
        let roots: Vec<String> = roots.unwrap().data.into_iter().map(|f| f.name).collect();
        assert_eq!(roots, ["Photos", "Summer"]);

        // Vòng lặp: vào chính nó, vào cháu của nó.
        for target in [photos.id, deep.id] {
            let changes = FolderChanges { parent_id: Some(Some(target)), ..Default::default() };
            let cycle = media.update_folder(&ada, photos.id, changes, None).await.unwrap_err();
            assert_eq!(cycle.code(), "media.folder_cycle");
        }
        // Đổi tên trùng anh em.
        let rename = FolderChanges { name: Some("SUMMER".into()), ..Default::default() };
        let clash = media.update_folder(&ada, photos.id, rename, None).await.unwrap_err();
        assert_eq!(clash.code(), "media.folder_name_taken");
        // Chuyển "Beach" lên gốc, phiên bản đúng.
        let up = FolderChanges { parent_id: Some(None), ..Default::default() };
        let moved = media.update_folder(&ada, deep.id, up, Some(deep.updated_at)).await.unwrap();
        assert_eq!(moved.parent_id, crate::entity::ROOT);
        let stale = FolderChanges { name: Some("B".into()), ..Default::default() };
        let stale = media.update_folder(&ada, deep.id, stale, Some(deep.updated_at)).await;
        assert_eq!(stale.unwrap_err().kind(), ErrorKind::PreconditionFailed);

        // Còn thư mục con, rồi còn item (kể cả trong thùng rác): không xoá được.
        let busy = media.delete_folder(&ada, photos.id).await.unwrap_err();
        assert_eq!((busy.kind(), busy.code()), (ErrorKind::Conflict, "media.folder_not_empty"));
        let input = NewUpload {
            name: "a.png".into(),
            folder_id: Some(summer.id),
            content_type: Some("image/png".into()),
            tags: Vec::new(),
        };
        let item = media.upload(&ada, input, body(&samples::png(1, 1))).await.unwrap().item;
        media.trash(&ada, item.id).await.unwrap();
        assert_eq!(
            media.delete_folder(&ada, summer.id).await.unwrap_err().code(),
            "media.folder_not_empty"
        );
        media.purge(&ada, item.id).await.unwrap();
        media.delete_folder(&ada, summer.id).await.unwrap();
        media.delete_folder(&ada, photos.id).await.unwrap();
        let gone = media.get_folder(&ada, photos.id).await.unwrap_err();
        assert_eq!(gone.code(), "media.folder_not_found");
        let names = events(&database).await;
        assert!(names.contains(&"media.folder_created".to_owned()));
        assert!(names.contains(&"media.folder_updated".to_owned()));
        assert!(names.contains(&"media.folder_deleted".to_owned()));
    })
    .await
    .unwrap();
}

/// Nhiều request cùng tạo thư mục cùng tên ở gốc: đúng một được — trường hợp `NULL` mà
/// unique index thường bỏ lọt, trên cả hai database.
#[tokio::test]
async fn tao_thu_muc_trung_ten_dong_thoi_o_goc() {
    TestDatabase::run(async |test| {
        let (media, _, _) = service(test, limits()).await;
        let attempts: Vec<_> = (0..5)
            .map(|_| {
                let media = media.clone();
                tokio::spawn(async move { media.create_folder(&editor(), "Same", None).await })
            })
            .collect();
        let mut created = 0;
        for attempt in attempts {
            match attempt.await.unwrap() {
                Ok(_) => created += 1,
                Err(error) => assert_eq!(error.code(), "media.folder_name_taken"),
            }
        }
        assert_eq!(created, 1);
    })
    .await
    .unwrap();
}

/// Hai lệnh chuyển chéo nhau cùng lúc (A vào B, B vào A): không bao giờ thành vòng lặp.
#[tokio::test]
async fn chuyen_thu_muc_cheo_nhau_khong_tao_vong() {
    TestDatabase::run(async |test| {
        let (media, _, _) = service(test, limits()).await;
        let ada = editor();
        for _ in 0..3 {
            let a = media.create_folder(&ada, &format!("A{}", Uuid::now_v7()), None).await.unwrap();
            let b = media.create_folder(&ada, &format!("B{}", Uuid::now_v7()), None).await.unwrap();
            let into = |from: Uuid, to: Uuid| {
                let media = media.clone();
                tokio::spawn(async move {
                    let changes = FolderChanges { parent_id: Some(Some(to)), ..Default::default() };
                    media.update_folder(&editor(), from, changes, None).await
                })
            };
            let (first, second) = (into(a.id, b.id), into(b.id, a.id));
            let results = [first.await.unwrap(), second.await.unwrap()];
            assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1, "{results:?}");
            let refused = results.iter().find_map(|result| result.as_ref().err()).unwrap();
            assert_eq!(refused.code(), "media.folder_cycle");
        }
    })
    .await
    .unwrap();
}

/// Job xoá item quá hạn trong thùng rác; job dọn nội dung mồ côi chỉ xoá khoá `media/`
/// đủ cũ mà không item nào trỏ tới.
#[tokio::test]
async fn job_don_thung_rac_va_mo_coi() {
    TestDatabase::run(async |test| {
        let (media, storage, _) = service(test, limits()).await;
        let ada = editor();
        let old = media.upload(&ada, upload("a.png", "image/png"), body(&samples::png(1, 1))).await;
        let old = old.unwrap().item;
        let kept =
            media.upload(&ada, upload("b.png", "image/png"), body(&samples::png(2, 2))).await;
        let kept = kept.unwrap().item;
        media.trash(&ada, old.id).await.unwrap();
        media.trash(&ada, kept.id).await.unwrap();

        // Hôm nay: chưa quá hạn.
        assert_eq!(media.purge_expired(Utc::now()).await.unwrap(), 0);
        // 31 ngày sau: cả hai quá hạn.
        let later = Utc::now() + chrono::Duration::days(31);
        assert_eq!(media.purge_expired(later).await.unwrap(), 2);
        assert!(storage.keys().await.unwrap().is_empty());

        let live =
            media.upload(&ada, upload("c.png", "image/png"), body(&samples::png(3, 3))).await;
        let live = live.unwrap().item;
        let old_version =
            Uuid::new_v7(uuid::Timestamp::from_unix(uuid::NoContext, 1_700_000_000, 0));
        let put = async |key: String| {
            storage.put(&key, body(b"x")).await.unwrap();
            key
        };
        let orphan = put(format!("media/{}/{old_version}", Uuid::now_v7())).await;
        let stale_version = put(format!("media/{}/{old_version}", live.id)).await;
        let fresh = put(format!("media/{}/{}", Uuid::now_v7(), Uuid::now_v7())).await;
        let foreign = put(format!("files/{}", Uuid::now_v7())).await;
        let odd = put("media/readme.txt".to_owned()).await;

        assert_eq!(media.clean_orphans(Utc::now()).await.unwrap(), 2);
        let mut expected = vec![live.storage_key(), fresh, foreign, odd];
        expected.sort();
        assert_eq!(storage.keys().await.unwrap(), expected);
        assert!(!expected.contains(&orphan) && !expected.contains(&stale_version));
    })
    .await
    .unwrap();
}

/// `api::Media`: nhiều ID một lần, theo thứ tự hỏi, bỏ ID không có và item trong thùng rác.
#[tokio::test]
async fn api_lay_nhieu_item_mot_lan() {
    TestDatabase::run(async |test| {
        let (media, _, _) = service(test, limits()).await;
        let ada = editor();
        let mut ids = Vec::new();
        for size in 1..=3 {
            let png = samples::png(size, size);
            ids.push(
                media.upload(&ada, upload("a.png", "image/png"), body(&png)).await.unwrap().item.id,
            );
        }
        let public = ItemChanges {
            visibility: Some(Visibility::Public),
            alt: Some("Alt".into()),
            ..Default::default()
        };
        media.update(&ada, ids[2], public, None).await.unwrap();
        media.trash(&ada, ids[1]).await.unwrap();

        let asked = [ids[2], Uuid::now_v7(), ids[1], ids[0]];
        let found = media.find_many(&asked).await.unwrap();
        assert_eq!(found.iter().map(|item| item.id).collect::<Vec<_>>(), [ids[2], ids[0]]);
        assert_eq!(found[0].visibility, MediaVisibility::Public);
        assert_eq!((found[0].width, found[0].alt.as_str()), (Some(3), "Alt"));
        assert!(found[0].url.starts_with(&format!("/api/v1/media/{}/content?v=", ids[2])));
        assert_eq!(found[1].visibility, MediaVisibility::Private);
        assert!(media.find(ids[1]).await.unwrap().is_none());
        assert!(media.find_many(&[]).await.unwrap().is_empty());
        assert!(media.find_many(&vec![Uuid::nil(); 501]).await.is_err());
    })
    .await
    .unwrap();
}

/// Xuất dữ liệu cá nhân; tài khoản bị xoá thì item còn, chỉ mất liên kết tới người đó.
#[tokio::test]
async fn du_lieu_ca_nhan() {
    TestDatabase::run(async |test| {
        let (media, _, _) = service(test, limits()).await;
        let ada = editor();
        let item =
            media.upload(&ada, upload("a.png", "image/png"), body(&samples::png(1, 1))).await;
        let item = item.unwrap().item;
        let exported = media.export(ada.user_id()).await.unwrap();
        assert_eq!(exported[0]["name"], "a.png");

        let deleted = Delivered {
            id: Uuid::now_v7(),
            module: "auth",
            name: autoxide_kernel::privacy::USER_DELETED.to_owned(),
            actor: Actor::system(),
            subject: None,
            payload: json!({"before": {"id": ada.user_id()}}),
            occurred_at: Utc::now(),
        };
        media.handle(&deleted).await.unwrap();
        media.handle(&deleted).await.unwrap();
        assert_eq!(media.get(&ada, item.id).await.unwrap().uploaded_by, None);
        assert_eq!(media.export(ada.user_id()).await.unwrap(), json!([]));
    })
    .await
    .unwrap();
}

/// Nhãn: gắn khi tải lên và khi sửa (thay cả bộ, bỏ trùng không phân biệt hoa thường), lọc
/// theo nhãn, đếm item (không tính thùng rác), đổi tên, xoá — trên cả hai database.
#[tokio::test]
async fn nhan_gan_loc_doi_ten_xoa() {
    use crate::tags::TagSort;

    TestDatabase::run(async |test| {
        let (media, _, database) = service(test, limits()).await;
        let ada = editor();
        let tagged = |name: &str, tags: &[&str]| NewUpload {
            name: name.into(),
            folder_id: None,
            content_type: Some("image/png".into()),
            tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        };
        let a = media
            .upload(&ada, tagged("a.png", &["Brand", " logo ", "brand"]), body(&samples::png(1, 1)))
            .await;
        let a = a.unwrap().item;
        let b = media
            .upload(&ada, tagged("b.png", &["logo"]), body(&samples::png(2, 2)))
            .await
            .unwrap()
            .item;
        let described = media.describe(vec![a.clone(), b.clone()]).await.unwrap();
        assert_eq!(described[0].tags, ["Brand", "logo"]);
        assert_eq!(described[1].tags, ["logo"]);

        let only = |tag: &str| ItemFilter { tag: Some(tag.into()), ..Default::default() };
        let names = async |filter: ItemFilter| -> Vec<String> {
            media
                .list(&ada, &filter, &page(10))
                .await
                .unwrap()
                .data
                .into_iter()
                .map(|item| item.name)
                .collect()
        };
        assert_eq!(names(only("LOGO")).await, ["a.png", "b.png"]);
        assert_eq!(names(only("brand")).await, ["a.png"]);
        assert!(names(only("missing")).await.is_empty());

        // Thay cả bộ nhãn; lỗi của mọi trường báo một lần.
        let changes = ItemChanges { tags: Some(vec!["Summer".into()]), ..Default::default() };
        media.update(&ada, a.id, changes, None).await.unwrap();
        assert_eq!(names(only("brand")).await, Vec::<String>::new());
        let bad = ItemChanges {
            tags: Some(vec!["a,b".into(), "x".repeat(51)]),
            alt: Some("x".repeat(1001)),
            ..Default::default()
        };
        let error = media.update(&ada, a.id, bad, None).await.unwrap_err();
        let fields: Vec<&str> = error.fields().iter().map(|field| field.field.as_str()).collect();
        assert_eq!(fields, ["alt", "tags"]);
        let many = ItemChanges {
            tags: Some((0..21).map(|n| format!("t{n}")).collect()),
            ..Default::default()
        };
        assert_eq!(
            media.update(&ada, a.id, many, None).await.unwrap_err().fields()[0].code,
            "length"
        );

        media.trash(&ada, b.id).await.unwrap();
        let tags = media.list_tags(&ada, None, TagSort::Name, &page(10)).await.unwrap();
        let counted: Vec<(String, u64)> =
            tags.data.iter().map(|t| (t.tag.name.clone(), t.items)).collect();
        assert_eq!(counted, [("Brand".into(), 0), ("logo".into(), 0), ("Summer".into(), 1)]);
        let summer = tags.data[2].tag.clone();
        let searched = media.list_tags(&ada, Some("SUM"), TagSort::Name, &page(10)).await.unwrap();
        assert_eq!(searched.data.len(), 1);

        let renamed = media
            .rename_tag(&ada, summer.id, "Summer 2026", Some(summer.updated_at))
            .await
            .unwrap();
        assert_eq!(renamed.tag.name, "Summer 2026");
        let stale =
            media.rename_tag(&ada, summer.id, "x", Some(summer.updated_at)).await.unwrap_err();
        assert_eq!(stale.kind(), ErrorKind::PreconditionFailed);
        let taken = media.rename_tag(&ada, summer.id, "LOGO", None).await.unwrap_err();
        assert_eq!(taken.code(), "media.tag_name_taken");
        assert_eq!(media.describe(vec![a.clone()]).await.unwrap()[0].tags, ["Summer 2026"]);

        media.delete_tag(&ada, summer.id).await.unwrap();
        assert!(media.describe(vec![a.clone()]).await.unwrap()[0].tags.is_empty());
        assert_eq!(
            media.delete_tag(&ada, summer.id).await.unwrap_err().code(),
            "media.tag_not_found"
        );
        let reader = user(&["read"]);
        assert_eq!(
            media.delete_tag(&reader, tags.data[0].tag.id).await.unwrap_err().code(),
            "permission_denied"
        );
        let names = events(&database).await;
        assert!(names.contains(&"media.tag_updated".to_owned()));
        assert!(names.contains(&"media.tag_deleted".to_owned()));
    })
    .await
    .unwrap();
}

/// Nhiều lần tải lên cùng lúc, cùng gắn một nhãn mới: mọi lần thành công, chỉ có một nhãn —
/// trên cả hai database.
#[tokio::test]
async fn gan_nhan_moi_dong_thoi() {
    TestDatabase::run(async |test| {
        let (media, _, _) = service(test, limits()).await;
        let attempts: Vec<_> = (1..=5u32)
            .map(|size| {
                let media = media.clone();
                tokio::spawn(async move {
                    let input = NewUpload {
                        name: format!("{size}.png"),
                        folder_id: None,
                        content_type: Some("image/png".into()),
                        tags: vec!["Fresh".into()],
                    };
                    media.upload(&editor(), input, body(&samples::png(size, size))).await
                })
            })
            .collect();
        for attempt in attempts {
            attempt.await.unwrap().unwrap();
        }
        let tags =
            media.list_tags(&editor(), None, crate::tags::TagSort::Name, &page(10)).await.unwrap();
        assert_eq!(tags.data.len(), 1);
        assert_eq!(tags.data[0].items, 5);
    })
    .await
    .unwrap();
}

/// Ảnh thu nhỏ: tải lên đưa job vào hàng đợi; job làm đúng các cỡ nhỏ hơn ảnh; thay nội dung
/// thì bản cũ bị xoá và job cũ không làm gì; xoá hẳn thì hết; job dọn mồ côi giữ bản hiện tại.
#[tokio::test]
async fn anh_thu_nho_theo_phien_ban() {
    use image::ImageFormat;

    use crate::variants::samples::encoded;

    TestDatabase::run(async |test| {
        let (media, storage, database) = service(test, limits()).await;
        let ada = editor();
        let original = encoded(150, 90, ImageFormat::Png, false);
        let item = media.upload(&ada, upload("wide.png", "image/png"), body(&original)).await;
        let item = item.unwrap().item;
        let backend = database.read().get_database_backend();
        let jobs = database
            .read()
            .query_all_raw(Statement::from_string(backend, "SELECT kind FROM media_jobs"))
            .await
            .unwrap();
        assert_eq!(jobs.len(), 1, "one job queued with the upload");

        // Cấu hình test: rộng 100 và 200; ảnh rộng 150 nên chỉ có bản 100.
        assert_eq!(media.make_variants(item.id, item.version_id).await.unwrap(), 1);
        let copies = media.describe_one(item.clone()).await.unwrap().variants;
        assert_eq!(copies.iter().map(|c| (c.width, c.height)).collect::<Vec<_>>(), [(100, 60)]);
        assert_eq!(copies[0].content_type, "image/jpeg");
        let copy_key = copies[0].storage_key();
        let stored = media.read_key(&copy_key, None).await.unwrap();
        let bytes: Vec<bytes::Bytes> = stored.content.try_collect().await.unwrap();
        let decoded = image::load_from_memory(&bytes.concat()).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (100, 60));
        // Chạy lại job (vd thử lại): không nhân đôi.
        media.make_variants(item.id, item.version_id).await.unwrap();
        assert_eq!(media.describe_one(item.clone()).await.unwrap().variants.len(), 1);

        // Ảnh SVG, PDF, ảnh nhỏ hơn mọi cỡ: không có job.
        media
            .upload(&ada, upload("tiny.png", "image/png"), body(&samples::png(50, 50)))
            .await
            .unwrap();
        let jobs = database
            .read()
            .query_all_raw(Statement::from_string(backend, "SELECT kind FROM media_jobs"))
            .await;
        assert_eq!(jobs.unwrap().len(), 1);

        // Thay nội dung: bản thu nhỏ cũ bị xoá, job của phiên bản cũ không làm gì.
        let bigger = encoded(400, 200, ImageFormat::Png, true);
        let new =
            media.replace(&ada, item.id, Some("image/png"), body(&bigger), None).await.unwrap();
        assert!(media.describe_one(new.clone()).await.unwrap().variants.is_empty());
        assert!(storage.get(&copy_key).await.unwrap().is_none());
        assert_eq!(media.make_variants(item.id, item.version_id).await.unwrap(), 0);
        assert_eq!(media.make_variants(new.id, new.version_id).await.unwrap(), 2);
        let copies = media.describe_one(new.clone()).await.unwrap().variants;
        assert_eq!(
            copies.iter().map(|c| c.content_type.as_str()).collect::<Vec<_>>(),
            ["image/png", "image/png"]
        );

        // Dọn mồ côi giữ bản thu nhỏ của phiên bản hiện tại.
        assert_eq!(media.clean_orphans(Utc::now() + chrono::Duration::days(2)).await.unwrap(), 0);
        assert_eq!(storage.keys_with_prefix(&format!("media/{}/", new.id)).await.unwrap().len(), 3);

        media.trash(&ada, new.id).await.unwrap();
        media.purge(&ada, new.id).await.unwrap();
        assert!(storage.keys_with_prefix(&format!("media/{}/", new.id)).await.unwrap().is_empty());
    })
    .await
    .unwrap();
}

/// Tìm tiếng Việt không dấu: "da lat" ra "Đà Lạt"; nhãn "Ảnh" và "Anh" là hai nhãn khác nhau
/// nhưng tìm "anh" ra cả hai — trên cả hai database.
#[tokio::test]
async fn tim_khong_dau() {
    use crate::tags::TagSort;

    TestDatabase::run(async |test| {
        let (media, _, _) = service(test, limits()).await;
        let ada = editor();
        let input = NewUpload {
            name: "Đường đến Đà Lạt.png".into(),
            folder_id: None,
            content_type: Some("image/png".into()),
            tags: vec!["Ảnh".into(), "Anh".into()],
        };
        media.upload(&ada, input, body(&samples::png(1, 1))).await.unwrap();
        for search in ["da lat", "ĐÀ LẠT", "duong den"] {
            let filter = ItemFilter { search: Some(search.into()), ..Default::default() };
            let found = media.list(&ada, &filter, &page(5)).await.unwrap();
            assert_eq!(found.data.len(), 1, "{search}");
        }
        let tags = media.list_tags(&ada, Some("anh"), TagSort::Name, &page(5)).await.unwrap();
        let names: Vec<String> = tags.data.into_iter().map(|tag| tag.tag.name).collect();
        assert_eq!(names, ["Anh", "Ảnh"]);
    })
    .await
    .unwrap();
}

/// Gợi ý alt text (D-109): gửi ảnh thu nhỏ (không bao giờ ảnh gốc lớn), ảnh riêng tư chỉ khi
/// được phép, ảnh trang trí thì alt rỗng; không ghi gì.
mod alt {
    use std::sync::Arc;

    use autoxide_kernel::ai::Part;
    use autoxide_testkit::ai::FakeTextModel;
    use image::ImageFormat;

    use super::*;
    use crate::testing::service_with_ai;
    use crate::variants::samples::encoded;

    fn image_of(ai: &FakeTextModel, call: usize) -> (String, image::DynamicImage) {
        let prompt = ai.prompts.lock().unwrap()[call].clone();
        let Part::Image { media_type, bytes } = &prompt.input[0] else { panic!("no image") };
        (media_type.clone(), image::load_from_memory(bytes).unwrap())
    }

    #[tokio::test]
    async fn goi_y_alt_tu_anh_thu_nho() {
        TestDatabase::run(async |test| {
            let ai = Arc::new(FakeTextModel::new(|prompt| {
                let Part::Text(context) = &prompt.input[1] else { panic!() };
                let decorative = context.contains("divider");
                Ok(json!({"alt": format!("  {} ", "x".repeat(300)), "decorative": decorative}))
            }));
            let (media, _, _) = service_with_ai(test, limits(), ai.clone(), false).await;
            let ada = editor();
            // Ảnh lớn chưa có ảnh thu nhỏ: thu nhỏ tạm còn 1280 px.
            let big = encoded(2000, 1000, ImageFormat::Png, false);
            let item =
                media.upload(&ada, upload("big.png", "image/png"), body(&big)).await.unwrap().item;
            let changes =
                ItemChanges { visibility: Some(Visibility::Public), ..Default::default() };
            let item = media.update(&ada, item.id, changes, None).await.unwrap();
            let suggestion =
                media.suggest_alt(&ada, item.id, "vi", Some("Bài về Đà Lạt")).await.unwrap();
            assert_eq!(suggestion.alt.chars().count(), 250);
            assert!(!suggestion.decorative);
            let (media_type, sent) = image_of(&ai, 0);
            assert_eq!(
                (media_type.as_str(), sent.width(), sent.height()),
                ("image/jpeg", 1280, 640)
            );
            let prompt = ai.prompts.lock().unwrap()[0].clone();
            assert_eq!(prompt.purpose, "media.suggest_alt");
            assert!(prompt.instructions.contains("`vi`"));
            // Đã có ảnh thu nhỏ (cấu hình test: 100 và 200): gửi bản lớn nhất.
            media.make_variants(item.id, item.version_id).await.unwrap();
            let decorative =
                media.suggest_alt(&ada, item.id, "en", Some("a divider")).await.unwrap();
            assert_eq!((decorative.alt.as_str(), decorative.decorative), ("", true));
            assert_eq!(image_of(&ai, 1).1.width(), 200);
            // Không ghi gì.
            assert_eq!(media.get(&ada, item.id).await.unwrap().alt, "");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn anh_rieng_tu_loai_khong_ho_tro_va_quyen() {
        TestDatabase::run(async |test| {
            let ai =
                Arc::new(FakeTextModel::new(|_| Ok(json!({"alt": "A cat", "decorative": false}))));
            let (media, _, _) = service_with_ai(test, limits(), ai.clone(), false).await;
            let ada = editor();
            let small = encoded(40, 30, ImageFormat::Png, false);
            let private = media
                .upload(&ada, upload("cat.png", "image/png"), body(&small))
                .await
                .unwrap()
                .item;
            let error = media.suggest_alt(&ada, private.id, "en", None).await.unwrap_err();
            assert_eq!(
                (error.kind(), error.code()),
                (ErrorKind::Forbidden, "media.ai_private_not_allowed")
            );
            let pdf = media
                .upload(&ada, upload("a.pdf", "application/pdf"), body(samples::PDF))
                .await
                .unwrap()
                .item;
            assert_eq!(
                media.suggest_alt(&ada, pdf.id, "en", None).await.unwrap_err().code(),
                "media.ai_unsupported_image"
            );
            let bad = media
                .suggest_alt(&ada, private.id, "not a code", Some(&"x".repeat(501)))
                .await
                .unwrap_err();
            assert_eq!(
                bad.fields().iter().map(|f| f.field.as_str()).collect::<Vec<_>>(),
                ["locale", "context"]
            );
            assert_eq!(
                media
                    .suggest_alt(&user(&["read"]), private.id, "en", None)
                    .await
                    .unwrap_err()
                    .code(),
                "permission_denied"
            );
            assert_eq!(ai.calls(), 0);

            // Cho gửi ảnh riêng tư: ảnh gốc đã nhỏ thì gửi nguyên.
            let (allowed, _, _) = service_with_ai(test, limits(), ai.clone(), true).await;
            let item = allowed
                .upload(&ada, upload("cat.png", "image/png"), body(&small))
                .await
                .unwrap()
                .item;
            assert_eq!(allowed.suggest_alt(&ada, item.id, "en", None).await.unwrap().alt, "A cat");
            let (media_type, sent) = image_of(&ai, 0);
            assert_eq!((media_type.as_str(), sent.width()), ("image/png", 40));
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn ai_tat_hay_khong_doc_anh() {
        TestDatabase::run(async |test| {
            let (media, _, _) = service(test, limits()).await;
            let ada = editor();
            let small = encoded(40, 30, ImageFormat::Png, false);
            let item = media
                .upload(&ada, upload("cat.png", "image/png"), body(&small))
                .await
                .unwrap()
                .item;
            let changes =
                ItemChanges { visibility: Some(Visibility::Public), ..Default::default() };
            media.update(&ada, item.id, changes, None).await.unwrap();
            assert_eq!(
                media.suggest_alt(&ada, item.id, "en", None).await.unwrap_err().code(),
                "ai.disabled"
            );
        })
        .await
        .unwrap();
    }
}
