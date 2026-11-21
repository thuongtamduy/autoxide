//! Item: tải lên mọi định dạng, nội dung sai, trùng, thay nội dung, sửa, danh sách, thùng rác,
//! quyền tải, job dọn, API cho module khác, dữ liệu cá nhân, tìm không dấu.

use super::*;
use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;

/// Mỗi định dạng ảnh: nội dung, SHA-256, loại và kích thước đúng; loại không khai thì máy
/// chủ tự nhận.
#[tokio::test]
async fn tai_len_moi_dinh_dang_anh() {
    TestDatabase::run(async |test| {
        let (media, _, database) = service(test, limits()).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let ada = editor();
        let cases = [
            ("a.png", "image/png", samples::png(640, 480), (640, 480)),
            ("a.jpg", "image/jpg", samples::jpeg(1920, 1080), (1920, 1080)),
            ("a.gif", "image/gif", samples::gif(32, 16), (32, 16)),
            ("a.webp", "image/webp", samples::webp(300, 200), (300, 200)),
            ("a.avif", "image/avif", samples::avif(4000, 3000), (4000, 3000)),
        ];
        for (name, declared, bytes, (width, height)) in cases {
            let uploaded =
                media.upload(&tenant, &ada, upload(name, declared), body(&bytes)).await.unwrap();
            let item = uploaded.item;
            assert_eq!((item.width, item.height), (Some(width), Some(height)), "{name}");
            assert_eq!(item.size, bytes.len() as i64);
            assert_eq!(content(&tenant, &media, &item).await, bytes);
            assert_eq!(item.sha256.len(), 64);
            assert_eq!(item.visibility, "private");
            assert_eq!(item.uploaded_by, Some(ada.user_id()));
        }
        let jpeg = media.list(&tenant, &ada, &ItemFilter::default(), &page(10)).await.unwrap();
        assert_eq!(
            jpeg.data[1].content_type, "image/jpeg",
            "the alias is stored by its standard name"
        );

        // Không khai loại: tự nhận.
        let auto = NewUpload {
            org_id: None,
            name: "doc.pdf".into(),
            folder_id: None,
            content_type: None,
            tags: Vec::new(),
        };
        let pdf = media.upload(&tenant, &ada, auto, body(samples::PDF)).await.unwrap().item;
        assert_eq!((pdf.content_type.as_str(), pdf.width), ("application/pdf", None));
        let octet = upload("clip.mp4", "application/octet-stream");
        assert_eq!(
            media.upload(&tenant, &ada, octet, body(samples::MP4)).await.unwrap().item.content_type,
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
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let ada = editor();
        let refuse = async |name: &str, declared: Option<&str>, bytes: &[u8]| {
            let input = NewUpload {
                org_id: None,
                name: name.into(),
                folder_id: None,
                content_type: declared.map(str::to_owned),
                tags: Vec::new(),
            };
            let error = media.upload(&tenant, &ada, input, body(bytes)).await.unwrap_err();
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
        // SVG có nội dung chạy được ở bất cứ đâu, kể cả sau phần đầu (D-149).
        let active = (ErrorKind::UnsupportedMediaType, "media.svg_active_content".to_owned());
        let mut late_script = b"<svg xmlns=\"http://www.w3.org/2000/svg\">".to_vec();
        late_script.extend(vec![b' '; 1200]);
        late_script.extend(b"<script>alert(1)</script></svg>");
        assert_eq!(refuse("x.svg", Some("image/svg+xml"), &late_script).await, active);
        assert_eq!(refuse("x.svg", None, &late_script).await, active);
        let short = b"<svg xmlns=\"http://www.w3.org/2000/svg\" onload=\"x()\"/>";
        assert_eq!(refuse("x.svg", Some("image/svg+xml"), short).await, active);
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
            org_id: None,
            name: " ".into(),
            folder_id: Some(Uuid::now_v7()),
            content_type: Some("image/png".into()),
            tags: Vec::new(),
        };
        let error =
            media.upload(&tenant, &ada, input, body(&samples::png(1, 1))).await.unwrap_err();
        let fields: Vec<&str> = error.fields().iter().map(|field| field.field.as_str()).collect();
        assert_eq!(fields, ["name", "folder_id"]);
        // Không có quyền.
        let reader = user(&["read"]);
        let denied =
            media.upload(&tenant, &reader, upload("a.png", "image/png"), body(&samples::png(1, 1)));
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
        let (media, _, database) = service(test, limits()).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let ada = editor();
        let png = samples::png(2, 2);
        let first =
            media.upload(&tenant, &ada, upload("a.png", "image/png"), body(&png)).await.unwrap();
        assert!(first.duplicates.is_empty());
        let second =
            media.upload(&tenant, &ada, upload("b.png", "image/png"), body(&png)).await.unwrap();
        assert_eq!(
            second.duplicates.iter().map(|item| item.id).collect::<Vec<_>>(),
            [first.item.id]
        );
        media.trash(&tenant, &ada, first.item.id).await.unwrap();
        let third =
            media.upload(&tenant, &ada, upload("c.png", "image/png"), body(&png)).await.unwrap();
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
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let ada = editor();
        let old = media
            .upload(&tenant, &ada, upload("a.png", "image/png"), body(&samples::png(1, 1)))
            .await;
        let old = old.unwrap().item;

        let new = media
            .replace(
                &tenant,
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
        assert_eq!(content(&tenant, &media, &new).await, samples::webp(8, 6));

        // Phiên bản cũ: 412, không để lại nội dung mới nào.
        let stale = media
            .replace(
                &tenant,
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
        let wrong = media
            .replace(&tenant, &ada, old.id, Some("image/png"), body(samples::HTML), None)
            .await;
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
        let (media, storage, database) = service(test, limits()).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let ada = editor();
        let item = media
            .upload(&tenant, &ada, upload("a.png", "image/png"), body(&samples::png(1, 1)))
            .await;
        let item = item.unwrap().item;

        let attempts: Vec<_> = (1..=4u32)
            .map(|size| {
                let (media, ada, tenant) = (media.clone(), ada.clone(), tenant.clone());
                tokio::spawn(async move {
                    let png = samples::png(size, size);
                    media
                        .replace(
                            &tenant,
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
        let (media, _, database) = service(test, limits()).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let ada = editor();
        let item = media
            .upload(&tenant, &ada, upload("a.png", "image/png"), body(&samples::png(1, 1)))
            .await;
        let item = item.unwrap().item;
        let folder = media.create_folder(&tenant, &ada, "Logos", None).await.unwrap();

        let changes = ItemChanges {
            name: Some(" Logo.png ".into()),
            alt: Some("Company logo".into()),
            caption: Some("Blue".into()),
            folder_id: Some(Some(folder.id)),
            visibility: Some(Visibility::Public),
            ..Default::default()
        };
        let changed =
            media.update(&tenant, &ada, item.id, changes, Some(item.updated_at)).await.unwrap();
        assert_eq!((changed.name.as_str(), changed.name_key.as_str()), ("Logo.png", "logo.png"));
        assert_eq!((changed.alt.as_str(), changed.visibility.as_str()), ("Company logo", "public"));
        assert_eq!(changed.folder_id, folder.id);
        // Về gốc.
        let root = ItemChanges { folder_id: Some(None), ..Default::default() };
        let moved = media.update(&tenant, &ada, item.id, root, None).await.unwrap();
        assert_eq!(moved.folder_id, crate::entity::ROOT);
        // Phiên bản cũ.
        let stale = media
            .update(&tenant, &ada, item.id, ItemChanges::default(), Some(item.updated_at))
            .await;
        assert_eq!(stale.unwrap_err().kind(), ErrorKind::PreconditionFailed);

        let invalid = ItemChanges {
            name: Some("a/b".into()),
            alt: Some("x".repeat(1001)),
            folder_id: Some(Some(Uuid::now_v7())),
            ..Default::default()
        };
        let error = media.update(&tenant, &ada, item.id, invalid, None).await.unwrap_err();
        let fields: Vec<&str> = error.fields().iter().map(|field| field.field.as_str()).collect();
        assert_eq!(fields, ["name", "alt", "folder_id"]);

        media.trash(&tenant, &ada, item.id).await.unwrap();
        let trashed = media.update(&tenant, &ada, item.id, ItemChanges::default(), None).await;
        assert_eq!(trashed.unwrap_err().code(), "media.item_in_trash");
        let missing =
            media.update(&tenant, &ada, Uuid::now_v7(), ItemChanges::default(), None).await;
        assert_eq!(missing.unwrap_err().code(), "media.item_not_found");
    })
    .await
    .unwrap();
}

/// Danh sách: từng bộ lọc, từng cách sắp, lật trang, tìm theo tên.
#[tokio::test]
async fn danh_sach_loc_sap_lat_trang() {
    TestDatabase::run(async |test| {
        let (media, _, database) = service(test, limits()).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let ada = editor();
        let folder = media.create_folder(&tenant, &ada, "Docs", None).await.unwrap();
        let mut ids = Vec::new();
        for (name, declared, bytes, folder_id) in [
            ("Banner.png", "image/png", samples::png(5, 5), None),
            ("apple.jpg", "image/jpeg", [samples::jpeg(2, 2), vec![0; 50]].concat(), None),
            ("Report 100%.pdf", "application/pdf", samples::PDF.to_vec(), Some(folder.id)),
            ("cat.gif", "image/gif", samples::gif(1, 1), Some(folder.id)),
        ] {
            let input = NewUpload {
                org_id: None,
                name: name.into(),
                folder_id,
                content_type: Some(declared.into()),
                tags: Vec::new(),
            };
            ids.push(media.upload(&tenant, &ada, input, body(&bytes)).await.unwrap().item.id);
        }
        let public = ItemChanges { visibility: Some(Visibility::Public), ..Default::default() };
        media.update(&tenant, &ada, ids[1], public, None).await.unwrap();
        media.trash(&tenant, &ada, ids[3]).await.unwrap();

        let names = async |filter: ItemFilter| -> Vec<String> {
            let page = media.list(&tenant, &ada, &filter, &page(50)).await.unwrap();
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
            let query = PageQuery::new(Some("1".into()), cursor, Some("desc".into()));
            let request = PageRequest::try_from(query).unwrap();
            let page = media.list(&tenant, &ada, &filter, &request).await.unwrap();
            seen.extend(page.data.into_iter().map(|item| item.name));
            match page.pagination.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        assert_eq!(seen, ["Report 100%.pdf", "Banner.png", "apple.jpg"]);
        let denied = media.list(&tenant, &user(&["write"]), &ItemFilter::default(), &page(5)).await;
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
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let writer = user(&["read", "write"]);
        let item = media
            .upload(&tenant, &writer, upload("a.png", "image/png"), body(&samples::png(1, 1)))
            .await;
        let item = item.unwrap().item;

        let purge = media.purge(&tenant, &editor(), item.id).await.unwrap_err();
        assert_eq!(purge.code(), "media.item_not_in_trash");
        let trashed = media.trash(&tenant, &writer, item.id).await.unwrap();
        assert!(trashed.trashed_at.is_some());
        assert_eq!(
            media.trash(&tenant, &writer, item.id).await.unwrap_err().code(),
            "media.item_in_trash"
        );
        let restored = media.restore(&tenant, &writer, item.id).await.unwrap();
        assert!(restored.trashed_at.is_none());
        assert_eq!(
            media.restore(&tenant, &writer, item.id).await.unwrap_err().code(),
            "media.item_not_in_trash"
        );
        media.trash(&tenant, &writer, item.id).await.unwrap();
        assert_eq!(
            media.purge(&tenant, &writer, item.id).await.unwrap_err().code(),
            "permission_denied"
        );
        media.purge(&tenant, &editor(), item.id).await.unwrap();

        assert_eq!(
            media.get(&tenant, &writer, item.id).await.unwrap_err().code(),
            "media.item_not_found"
        );
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
        let (media, _, database) = service(test, limits()).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let ada = editor();
        let item = media
            .upload(&tenant, &ada, upload("a.png", "image/png"), body(&samples::png(1, 1)))
            .await;
        let item = item.unwrap().item;
        let reader = user(&["read"]);
        let stranger = user(&[]);

        assert!(media.viewable(&tenant, None, item.id).await.is_err());
        assert!(media.viewable(&tenant, Some(&stranger), item.id).await.is_err());
        assert!(media.viewable(&tenant, Some(&reader), item.id).await.is_ok());
        let public = ItemChanges { visibility: Some(Visibility::Public), ..Default::default() };
        media.update(&tenant, &ada, item.id, public, None).await.unwrap();
        assert!(media.viewable(&tenant, None, item.id).await.is_ok());
        media.trash(&tenant, &ada, item.id).await.unwrap();
        let hidden = media.viewable(&tenant, None, item.id).await.unwrap_err();
        assert_eq!(hidden.code(), "media.item_not_found");
        assert!(media.viewable(&tenant, Some(&reader), item.id).await.is_ok());
    })
    .await
    .unwrap();
}

/// Job xoá item quá hạn trong thùng rác; job dọn nội dung mồ côi chỉ xoá khoá `media/`
/// đủ cũ mà không item nào trỏ tới.
#[tokio::test]
async fn job_don_thung_rac_va_mo_coi() {
    TestDatabase::run(async |test| {
        let (media, storage, database) = service(test, limits()).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let ada = editor();
        let old = media
            .upload(&tenant, &ada, upload("a.png", "image/png"), body(&samples::png(1, 1)))
            .await;
        let old = old.unwrap().item;
        let kept = media
            .upload(&tenant, &ada, upload("b.png", "image/png"), body(&samples::png(2, 2)))
            .await;
        let kept = kept.unwrap().item;
        media.trash(&tenant, &ada, old.id).await.unwrap();
        media.trash(&tenant, &ada, kept.id).await.unwrap();

        // Hôm nay: chưa quá hạn.
        let registry = autoxide_kernel::tenancy::DatabaseRegistry::single(database.clone());
        assert_eq!(media.purge_expired(&registry, &database, Timestamp::now()).await.unwrap(), 0);
        // 31 ngày sau: cả hai quá hạn.
        let later = Timestamp::now() + SignedDuration::from_hours(24 * 31);
        assert_eq!(media.purge_expired(&registry, &database, later).await.unwrap(), 2);
        assert!(storage.keys().await.unwrap().is_empty());

        let live = media
            .upload(&tenant, &ada, upload("c.png", "image/png"), body(&samples::png(3, 3)))
            .await;
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

        assert_eq!(media.clean_orphans(&registry, Timestamp::now()).await.unwrap(), 2);
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
        let (media, _, database) = service(test, limits()).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let ada = editor();
        let mut ids = Vec::new();
        for size in 1..=3 {
            let png = samples::png(size, size);
            ids.push(
                media
                    .upload(&tenant, &ada, upload("a.png", "image/png"), body(&png))
                    .await
                    .unwrap()
                    .item
                    .id,
            );
        }
        let public = ItemChanges {
            visibility: Some(Visibility::Public),
            alt: Some("Alt".into()),
            ..Default::default()
        };
        media.update(&tenant, &ada, ids[2], public, None).await.unwrap();
        media.trash(&tenant, &ada, ids[1]).await.unwrap();

        let asked = [ids[2], Uuid::now_v7(), ids[1], ids[0]];
        let found = media.find_many(&tenant, &asked).await.unwrap();
        assert_eq!(found.iter().map(|item| item.id).collect::<Vec<_>>(), [ids[2], ids[0]]);
        assert_eq!(found[0].visibility, MediaVisibility::Public);
        assert_eq!((found[0].width, found[0].alt.as_str()), (Some(3), "Alt"));
        assert!(found[0].url.starts_with(&format!("/api/v1/media/{}/content?v=", ids[2])));
        assert_eq!(found[1].visibility, MediaVisibility::Private);
        assert!(media.find(&tenant, ids[1]).await.unwrap().is_none());
        assert!(media.find_many(&tenant, &[]).await.unwrap().is_empty());
        assert!(media.find_many(&tenant, &vec![Uuid::nil(); 501]).await.is_err());
    })
    .await
    .unwrap();
}

/// Xuất dữ liệu cá nhân; tài khoản bị xoá thì item còn, chỉ mất liên kết tới người đó.
#[tokio::test]
async fn du_lieu_ca_nhan() {
    TestDatabase::run(async |test| {
        let (media, _, database) = service(test, limits()).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let ada = editor();
        let item = media
            .upload(&tenant, &ada, upload("a.png", "image/png"), body(&samples::png(1, 1)))
            .await;
        let item = item.unwrap().item;
        let exported = media.export(&tenant, ada.user_id()).await.unwrap();
        assert_eq!(exported[0]["name"], "a.png");

        let deleted = Delivered::new(
            tenant.clone(),
            "auth",
            autoxide_kernel::privacy::USER_DELETED.to_owned(),
            json!({"before": {"id": ada.user_id()}}),
        );
        media.handle(&deleted).await.unwrap();
        media.handle(&deleted).await.unwrap();
        assert_eq!(media.get(&tenant, &ada, item.id).await.unwrap().uploaded_by, None);
        assert_eq!(media.export(&tenant, ada.user_id()).await.unwrap(), json!([]));
    })
    .await
    .unwrap();
}

/// Tìm tiếng Việt không dấu: "da lat" ra "Đà Lạt"; nhãn "Ảnh" và "Anh" là hai nhãn khác nhau
/// nhưng tìm "anh" ra cả hai — trên cả hai database.
#[tokio::test]
async fn tim_khong_dau() {
    use crate::library::tags::TagSort;

    TestDatabase::run(async |test| {
        let (media, _, database) = service(test, limits()).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let ada = editor();
        let input = NewUpload {
            org_id: None,
            name: "Đường đến Đà Lạt.png".into(),
            folder_id: None,
            content_type: Some("image/png".into()),
            tags: vec!["Ảnh".into(), "Anh".into()],
        };
        media.upload(&tenant, &ada, input, body(&samples::png(1, 1))).await.unwrap();
        for search in ["da lat", "ĐÀ LẠT", "duong den"] {
            let filter = ItemFilter { search: Some(search.into()), ..Default::default() };
            let found = media.list(&tenant, &ada, &filter, &page(5)).await.unwrap();
            assert_eq!(found.data.len(), 1, "{search}");
        }
        let tags =
            media.list_tags(&tenant, &ada, Some("anh"), TagSort::Name, &page(5)).await.unwrap();
        let names: Vec<String> = tags.data.into_iter().map(|tag| tag.tag.name).collect();
        assert_eq!(names, ["Anh", "Ảnh"]);
    })
    .await
    .unwrap();
}

/// Khách khác khách mặc định: nội dung nằm dưới tiền tố của khách đó (`tenants/<id>/media/…`),
/// khách mặc định không thấy item, thư mục hay nhãn của nó; tên thư mục chỉ cần khác nhau trong một
/// khách; chuyển thư mục được dù khách chưa có dòng khoá cây thư mục. Dọn mồ côi để nguyên khoá
/// của khách không có trong sổ — trên cả hai database.
#[tokio::test]
async fn khoa_file_co_tien_to_khach() {
    TestDatabase::run(async |test| {
        let (media, storage, database) = service(test, limits()).await;
        let default = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let other = autoxide_kernel::tenancy::TenantContext::new(
            autoxide_kernel::tenancy::TenantId::from_uuid(Uuid::now_v7()),
            "other",
            "Other",
            autoxide_kernel::tenancy::TenantStatus::Active,
            autoxide_kernel::tenancy::MAIN_PLACEMENT,
            database.clone(),
        );
        let ada = editor();
        let mut input = upload("a.png", "image/png");
        input.tags = vec!["Shared".into()];
        let item = media.upload(&other, &ada, input, body(&samples::png(2, 2))).await.unwrap().item;

        let prefixed = format!("tenants/{}/{}", other.id(), item.storage_key());
        assert!(storage.get(&prefixed).await.unwrap().is_some(), "under the tenant prefix");
        assert!(storage.get(&item.storage_key()).await.unwrap().is_none());
        assert_eq!(content(&other, &media, &item).await, samples::png(2, 2));

        // Khách mặc định không thấy gì của khách kia.
        let unseen = media.get(&default, &ada, item.id).await.unwrap_err();
        assert_eq!(unseen.code(), "media.item_not_found");
        assert!(media.viewable(&default, Some(&ada), item.id).await.is_err());
        let listed = media.list(&default, &ada, &ItemFilter::default(), &page(10)).await.unwrap();
        assert!(listed.data.is_empty());
        let tags = media
            .list_tags(&default, &ada, None, crate::library::tags::TagSort::Name, &page(10))
            .await
            .unwrap();
        assert!(tags.data.is_empty());
        assert!(media.find_many(&default, &[item.id]).await.unwrap().is_empty());
        assert_eq!(media.find_many(&other, &[item.id]).await.unwrap().len(), 1);

        // Cùng tên thư mục ở hai khách; chuyển thư mục ở khách chưa có dòng khoá.
        let mine = media.create_folder(&default, &ada, "Photos", None).await.unwrap();
        let theirs = media.create_folder(&other, &ada, "Photos", None).await.unwrap();
        let inner = media.create_folder(&other, &ada, "Inner", None).await.unwrap();
        let moved = FolderChanges { parent_id: Some(Some(theirs.id)), ..Default::default() };
        media.update_folder(&other, &ada, inner.id, moved, None).await.unwrap();
        let across = FolderChanges { parent_id: Some(Some(mine.id)), ..Default::default() };
        let refused = media.update_folder(&other, &ada, inner.id, across, None).await.unwrap_err();
        assert_eq!(refused.code(), "validation_failed");

        // Sổ chỉ có khách mặc định: khoá của khách kia để nguyên, kể cả khoá mồ côi.
        let old = Uuid::new_v7(uuid::Timestamp::from_unix(uuid::NoContext, 1_700_000_000, 0));
        let orphan = format!("tenants/{}/media/{}/{old}", other.id(), Uuid::now_v7());
        storage.put(&orphan, body(b"x")).await.unwrap();
        let registry = autoxide_kernel::tenancy::DatabaseRegistry::single(database.clone());
        assert_eq!(media.clean_orphans(&registry, Timestamp::now()).await.unwrap(), 0);
        assert!(storage.get(&orphan).await.unwrap().is_some());
        assert!(storage.get(&prefixed).await.unwrap().is_some());
    })
    .await
    .unwrap();
}

/// Media theo nút (bước 34, D-208): người bán trên sàn chỉ thấy, sửa, tải được media của gian hàng
/// mình; item của người khác báo như không có (cả qua link tải riêng tư).
#[tokio::test]
async fn nguoi_ban_chi_thay_media_cua_minh() {
    use autoxide_kernel::access::{Grant, Principal, Scope};
    use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree};

    TestDatabase::run(async |test| {
        let (media, _, database) = service(test, limits()).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let org = || OrgId::from_uuid(Uuid::now_v7());
        let (seller_a, seller_b) = (org(), org());
        let write = database.begin_write().await.unwrap();
        let root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, seller_a, root, 8).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, seller_b, root, 8).await.unwrap();
        write.commit().await.unwrap();
        let seller = |unit: OrgId| {
            Principal::with_grants(
                Uuid::now_v7(),
                ["media.read", "media.write", "media.purge"]
                    .map(|name| (name.to_owned(), Grant::new(unit, Scope::Subtree))),
            )
        };
        let (a, b) = (seller(seller_a), seller(seller_b));
        let png = samples::png(8, 8);

        // Không nêu nút: nút duy nhất của người bán.
        let of_a =
            media.upload(&tenant, &a, upload("a.png", "image/png"), body(&png)).await.unwrap().item;
        assert_eq!(of_a.org_id, Some(seller_a.as_uuid()));
        let of_b =
            media.upload(&tenant, &b, upload("b.png", "image/png"), body(&png)).await.unwrap().item;
        // Tải vào gian hàng người khác: 403.
        let mut elsewhere = upload("x.png", "image/png");
        elsewhere.org_id = Some(seller_a.as_uuid());
        let error = media.upload(&tenant, &b, elsewhere, body(&png)).await.unwrap_err();
        assert_eq!(error.code(), "org_out_of_scope");

        let names = |page: autoxide_kernel::pagination::Page<Item>| -> Vec<String> {
            page.data.into_iter().map(|item| item.name).collect()
        };
        let listed = media.list(&tenant, &a, &ItemFilter::default(), &page(10)).await.unwrap();
        assert_eq!(names(listed), ["a.png"]);
        let listed =
            media.list(&tenant, &editor(), &ItemFilter::default(), &page(10)).await.unwrap();
        assert_eq!(listed.data.len(), 2, "người có quyền cả khách thấy hết");
        let only_b = ItemFilter { owner_org: Some(seller_b.as_uuid()), ..Default::default() };
        assert_eq!(
            names(media.list(&tenant, &editor(), &only_b, &page(10)).await.unwrap()),
            ["b.png"]
        );

        // Item của B với A: như không có — đọc, sửa, bỏ vào thùng rác, tải nội dung riêng tư.
        let missing =
            |error: autoxide_kernel::error::Error| assert_eq!(error.code(), "media.item_not_found");
        missing(media.get(&tenant, &a, of_b.id).await.unwrap_err());
        let rename = ItemChanges { name: Some("mine.png".to_owned()), ..Default::default() };
        missing(media.update(&tenant, &a, of_b.id, rename, None).await.unwrap_err());
        missing(media.trash(&tenant, &a, of_b.id).await.unwrap_err());
        missing(media.viewable(&tenant, Some(&a), of_b.id).await.unwrap_err());
        assert!(media.viewable(&tenant, Some(&a), of_a.id).await.is_ok());
    })
    .await
    .unwrap();
}
