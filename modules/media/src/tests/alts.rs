//! `alt` theo ngôn ngữ (D-202).

use super::*;

/// Số bản dịch `alt` đang lưu của item `id`.
async fn alt_rows(database: &Database, id: Uuid) -> u64 {
    use crate::entity::alt_translation::{Column, Entity};
    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
    Entity::find().filter(Column::MediaId.eq(id)).count(database.read()).await.unwrap()
}

/// `alt` theo ngôn ngữ (D-202): đọc theo ngôn ngữ có bản dịch thì ra bản dịch, ngôn ngữ khác thì
/// lùi về `alt` mặc định; xoá bản dịch thì lùi lại; `If-Match` cũ bị từ chối; mỗi thay đổi là một
/// event `media.item_updated`.
#[tokio::test]
async fn alt_theo_ngon_ngu_lui_ve_mac_dinh() {
    TestDatabase::run(async |test| {
        let (media, _, database) = service(test, limits()).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let ada = editor();
        let item =
            media.upload(&tenant, &ada, upload("bep.png", "image/png"), body(&samples::png(4, 4)));
        let item = item.await.unwrap().item;
        let changes =
            ItemChanges { alt: Some("Black induction hob".to_owned()), ..ItemChanges::default() };
        let item = media.update(&tenant, &ada, item.id, changes, None).await.unwrap();

        let set = media
            .set_alt(&tenant, &ada, item.id, "VI", Some("  Bếp từ màu đen "), Some(item.updated_at))
            .await
            .unwrap();
        assert_ne!(set.updated_at, item.updated_at, "the ETag changes");
        let details = media.describe_one(&tenant, set.clone()).await.unwrap();
        assert_eq!(details.alts, [("vi".to_owned(), "Bếp từ màu đen".to_owned())].into());
        assert_eq!(details.item.alt, "Black induction hob", "the default stays");

        let alt_in = async |locale: &str| {
            media.find_many_in(&tenant, &[item.id], locale).await.unwrap()[0].alt.clone()
        };
        assert_eq!(alt_in("vi").await, "Bếp từ màu đen");
        assert_eq!(alt_in("en").await, "Black induction hob", "no translation: the default");
        assert_eq!(alt_in("fr").await, "Black induction hob", "not offered: the default");
        assert_eq!(
            media.find_many(&tenant, &[item.id]).await.unwrap()[0].alt,
            "Black induction hob"
        );

        let stale =
            media.set_alt(&tenant, &ada, item.id, "vi", Some("Khác"), Some(item.updated_at)).await;
        assert_eq!(stale.unwrap_err().code(), "precondition_failed");
        let too_long = "x".repeat(1001);
        let refused =
            media.set_alt(&tenant, &ada, item.id, "vi", Some(&too_long), None).await.unwrap_err();
        assert_eq!(refused.fields()[0].field, "alt");
        let reader = user(&["read"]);
        let denied =
            media.set_alt(&tenant, &reader, item.id, "vi", Some("x"), None).await.unwrap_err();
        assert_eq!(denied.code(), "permission_denied");

        let removed =
            media.set_alt(&tenant, &ada, item.id, "vi", None, Some(set.updated_at)).await.unwrap();
        assert_eq!(alt_in("vi").await, "Black induction hob");
        let again = media.set_alt(&tenant, &ada, item.id, "vi", None, None).await.unwrap();
        assert_eq!(again.updated_at, removed.updated_at, "removing nothing changes nothing");
        assert_eq!(
            events(&database).await,
            [
                "media.item_uploaded",
                "media.item_updated",
                "media.item_updated",
                "media.item_updated"
            ]
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ma_ngon_ngu_sai_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let (media, _, database) = service(test, limits()).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let ada = editor();
        let item =
            media.upload(&tenant, &ada, upload("a.png", "image/png"), body(&samples::png(1, 1)));
        let item = item.await.unwrap().item;
        for locale in ["fr", "vietnamese", "", "vi'--"] {
            let refused =
                media.set_alt(&tenant, &ada, item.id, locale, Some("x"), None).await.unwrap_err();
            assert_eq!(refused.code(), "media.unsupported_locale", "{locale}");
            assert_eq!(refused.kind(), ErrorKind::Validation);
        }
        assert_eq!(alt_rows(&database, item.id).await, 0);
        let gone =
            media.set_alt(&tenant, &ada, Uuid::now_v7(), "vi", Some("x"), None).await.unwrap_err();
        assert_eq!(gone.code(), "media.item_not_found");
        media.trash(&tenant, &ada, item.id).await.unwrap();
        let trashed =
            media.set_alt(&tenant, &ada, item.id, "vi", Some("x"), None).await.unwrap_err();
        assert_eq!(trashed.code(), "media.item_in_trash");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn xoa_anh_thi_xoa_ban_dich() {
    TestDatabase::run(async |test| {
        let (media, _, database) = service(test, limits()).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let ada = editor();
        let item =
            media.upload(&tenant, &ada, upload("a.png", "image/png"), body(&samples::png(1, 1)));
        let item = item.await.unwrap().item;
        let other =
            media.upload(&tenant, &ada, upload("b.png", "image/png"), body(&samples::png(2, 2)));
        let other = other.await.unwrap().item;
        for id in [item.id, other.id] {
            media.set_alt(&tenant, &ada, id, "vi", Some("Ảnh"), None).await.unwrap();
            media.set_alt(&tenant, &ada, id, "en", Some("Picture"), None).await.unwrap();
        }
        assert_eq!(alt_rows(&database, item.id).await, 2);
        media.trash(&tenant, &ada, item.id).await.unwrap();
        assert_eq!(alt_rows(&database, item.id).await, 2, "the trash can still be restored");
        media.purge(&tenant, &ada, item.id).await.unwrap();
        assert_eq!(alt_rows(&database, item.id).await, 0);
        assert_eq!(alt_rows(&database, other.id).await, 2, "other items keep theirs");
    })
    .await
    .unwrap();
}
