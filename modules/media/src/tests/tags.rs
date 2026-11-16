//! Nhãn: gắn, lọc, đổi tên, xoá; gắn nhãn mới đồng thời.

use super::*;

/// Nhãn: gắn khi tải lên và khi sửa (thay cả bộ, bỏ trùng không phân biệt hoa thường), lọc
/// theo nhãn, đếm item (không tính thùng rác), đổi tên, xoá — trên cả hai database.
#[tokio::test]
async fn nhan_gan_loc_doi_ten_xoa() {
    use crate::library::tags::TagSort;

    TestDatabase::run(async |test| {
        let (media, _, database) = service(test, limits()).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let ada = editor();
        let tagged = |name: &str, tags: &[&str]| NewUpload {
            org_id: None,
            name: name.into(),
            folder_id: None,
            content_type: Some("image/png".into()),
            tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        };
        let a = media
            .upload(
                &tenant,
                &ada,
                tagged("a.png", &["Brand", " logo ", "brand"]),
                body(&samples::png(1, 1)),
            )
            .await;
        let a = a.unwrap().item;
        let b = media
            .upload(&tenant, &ada, tagged("b.png", &["logo"]), body(&samples::png(2, 2)))
            .await
            .unwrap()
            .item;
        let described = media.describe(&tenant, vec![a.clone(), b.clone()]).await.unwrap();
        assert_eq!(described[0].tags, ["Brand", "logo"]);
        assert_eq!(described[1].tags, ["logo"]);

        let only = |tag: &str| ItemFilter { tag: Some(tag.into()), ..Default::default() };
        let names = async |filter: ItemFilter| -> Vec<String> {
            media
                .list(&tenant, &ada, &filter, &page(10))
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
        media.update(&tenant, &ada, a.id, changes, None).await.unwrap();
        assert_eq!(names(only("brand")).await, Vec::<String>::new());
        let bad = ItemChanges {
            tags: Some(vec!["a,b".into(), "x".repeat(51)]),
            alt: Some("x".repeat(1001)),
            ..Default::default()
        };
        let error = media.update(&tenant, &ada, a.id, bad, None).await.unwrap_err();
        let fields: Vec<&str> = error.fields().iter().map(|field| field.field.as_str()).collect();
        assert_eq!(fields, ["alt", "tags"]);
        let many = ItemChanges {
            tags: Some((0..21).map(|n| format!("t{n}")).collect()),
            ..Default::default()
        };
        assert_eq!(
            media.update(&tenant, &ada, a.id, many, None).await.unwrap_err().fields()[0].code,
            "length"
        );

        media.trash(&tenant, &ada, b.id).await.unwrap();
        let tags = media.list_tags(&tenant, &ada, None, TagSort::Name, &page(10)).await.unwrap();
        let counted: Vec<(String, u64)> =
            tags.data.iter().map(|t| (t.tag.name.clone(), t.items)).collect();
        assert_eq!(counted, [("Brand".into(), 0), ("logo".into(), 0), ("Summer".into(), 1)]);
        let summer = tags.data[2].tag.clone();
        let searched =
            media.list_tags(&tenant, &ada, Some("SUM"), TagSort::Name, &page(10)).await.unwrap();
        assert_eq!(searched.data.len(), 1);

        let renamed = media
            .rename_tag(&tenant, &ada, summer.id, "Summer 2026", Some(summer.updated_at))
            .await
            .unwrap();
        assert_eq!(renamed.tag.name, "Summer 2026");
        let stale = media
            .rename_tag(&tenant, &ada, summer.id, "x", Some(summer.updated_at))
            .await
            .unwrap_err();
        assert_eq!(stale.kind(), ErrorKind::PreconditionFailed);
        let taken = media.rename_tag(&tenant, &ada, summer.id, "LOGO", None).await.unwrap_err();
        assert_eq!(taken.code(), "media.tag_name_taken");
        assert_eq!(
            media.describe(&tenant, vec![a.clone()]).await.unwrap()[0].tags,
            ["Summer 2026"]
        );

        media.delete_tag(&tenant, &ada, summer.id).await.unwrap();
        assert!(media.describe(&tenant, vec![a.clone()]).await.unwrap()[0].tags.is_empty());
        assert_eq!(
            media.delete_tag(&tenant, &ada, summer.id).await.unwrap_err().code(),
            "media.tag_not_found"
        );
        let reader = user(&["read"]);
        assert_eq!(
            media.delete_tag(&tenant, &reader, tags.data[0].tag.id).await.unwrap_err().code(),
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
        let (media, _, database) = service(test, limits()).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let attempts: Vec<_> = (1..=5u32)
            .map(|size| {
                let (media, tenant) = (media.clone(), tenant.clone());
                tokio::spawn(async move {
                    let input = NewUpload {
                        org_id: None,
                        name: format!("{size}.png"),
                        folder_id: None,
                        content_type: Some("image/png".into()),
                        tags: vec!["Fresh".into()],
                    };
                    media.upload(&tenant, &editor(), input, body(&samples::png(size, size))).await
                })
            })
            .collect();
        for attempt in attempts {
            attempt.await.unwrap().unwrap();
        }
        let tags = media
            .list_tags(&tenant, &editor(), None, crate::library::tags::TagSort::Name, &page(10))
            .await
            .unwrap();
        assert_eq!(tags.data.len(), 1);
        assert_eq!(tags.data[0].items, 5);
    })
    .await
    .unwrap();
}
