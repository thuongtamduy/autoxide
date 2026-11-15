//! Thư mục lồng nhau: tạo trùng tên đồng thời, chuyển chéo nhau không tạo vòng.

use super::*;

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
