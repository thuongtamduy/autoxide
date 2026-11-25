//! Sửa sau rà soát (D-111): yêu cầu dịch kẹt ở `running` bị yêu cầu mới thay chỗ; yêu cầu đã
//! huỷ không bị job đang chạy ghi đè thành "thành công".

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_kernel::ai::{Part, Prompt};
use autoxide_testkit::ai::{FakeEmbedder, FakeTextModel};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use super::*;
use crate::entity::translation_request;
use crate::testing::service_with;
use autoxide_kernel::tenancy::TenantContext;

fn translator() -> FakeTextModel {
    FakeTextModel::new(|prompt: &Prompt| {
        let Part::Text(input) = &prompt.input[0] else { panic!() };
        let items: Vec<Value> = serde_json::from_str(input).unwrap();
        let segments: Vec<Value> =
            items.iter().map(|item| json!({"id": item["id"], "text": "dịch"})).collect();
        Ok(json!({"segments": segments}))
    })
}

#[tokio::test]
async fn yeu_cau_ket_bi_thay_va_huy_khong_bi_ghi_de() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service_with(
            test,
            config(&[]),
            Arc::new(translator()),
            Arc::new(FakeEmbedder::disabled()),
        )
        .await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();
        let source = content.create(&tenant, &ada, post("Hello")).await.unwrap();
        let stuck = content.request_translation(&tenant, &ada, source.id, "vi").await.unwrap();
        // Như một job bị worker cắt ngang ở lần cuối: `running`, đứng yên 20 phút.
        translation_request::Entity::update_many()
            .col_expr(translation_request::Column::Status, Expr::value("running"))
            .col_expr(
                translation_request::Column::UpdatedAt,
                Expr::value(Timestamp::now() - SignedDuration::from_mins(20)),
            )
            .filter(translation_request::Column::Id.eq(stuck.id))
            .exec(database.write())
            .await
            .unwrap();
        let fresh = content.request_translation(&tenant, &ada, source.id, "vi").await.unwrap();
        let old = translation_request::Entity::find_by_id(stuck.id)
            .one(database.read())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (old.status.as_str(), old.error_code.as_deref(), old.active_key),
            ("failed", Some("content.translation_stale"), None)
        );
        // Job cũ chạy tới: thấy đã xong, không làm gì.
        content.run_translation(&tenant, stuck.id, 8).await.unwrap();
        assert_eq!(
            translation_request::Entity::find_by_id(stuck.id)
                .one(database.read())
                .await
                .unwrap()
                .unwrap()
                .status,
            "failed"
        );

        // Huỷ (tài khoản bị xoá) rồi job chạy: vẫn là huỷ.
        assert_eq!(
            crate::ai::translate::cancel_translations_by(&tenant, ada.user_id()).await.unwrap(),
            1
        );
        content.run_translation(&tenant, fresh.id, 1).await.unwrap();
        let canceled = translation_request::Entity::find_by_id(fresh.id)
            .one(database.read())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (canceled.status.as_str(), canceled.error_code.as_deref()),
            ("failed", Some("content.translation_canceled"))
        );
    })
    .await
    .unwrap();
}
