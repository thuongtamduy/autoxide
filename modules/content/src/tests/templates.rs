//! Mẫu trang bắt buộc (bước 37, D-359): điền từ hồ sơ pháp lý (hợp đồng `LegalProfiles` của
//! `compliance`, ở đây là bản giả) và biến người gọi; thiếu biến bắt buộc báo một lần; giá trị chỉ
//! vào phần chữ; ảnh chụp bản đã đăng qua `PublishedEntries`.

use std::sync::Arc;

use super::*;
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{OrgId, TenantContext};

use crate::api::PublishedEntries;
use crate::templates::TemplateDraft;

struct FakeLegal;

#[async_trait::async_trait]
impl autoxide_compliance::api::LegalProfiles for FakeLegal {
    async fn template_variables(
        &self,
        _: &TenantContext,
        _: Option<OrgId>,
    ) -> Result<Option<Value>, Error> {
        Ok(Some(json!({
            "owner": {"legal_name": "Cong ty <b>Vi Du</b>", "address": "12 Trang Tien, Ha Noi"},
            "contact": {"email": "lienhe@shop.example", "phone": "024 1234 5678"},
        })))
    }
}

fn draft(locale: &str, variables: Value) -> TemplateDraft {
    TemplateDraft { locale: locale.to_owned(), variables, parent_id: None, org_id: None }
}

#[tokio::test]
async fn mau_trang_dien_tu_ho_so_phap_ly() {
    TestDatabase::run(async |test| {
        let (content, _, database) =
            service(test, config(&[("AUTOXIDE_CONTENT_LOCALES", "en,vi")])).await;
        let content = Arc::new(
            Arc::into_inner(content)
                .unwrap()
                .with_templates(Default::default(), Some(Arc::new(FakeLegal))),
        );
        let tenant = TenantContext::default_on(database.clone());
        let ada = editor();

        // Không có permission: 403 trước mọi kiểm khác (kể cả mẫu không có).
        let denied = content
            .draft_from_template(&tenant, &user(&[]), "nope", draft("vi", json!({})))
            .await
            .unwrap_err();
        assert_eq!(denied.code(), "permission_denied");
        let missing = content
            .draft_from_template(&tenant, &ada, "nope", draft("vi", json!({})))
            .await
            .unwrap_err();
        assert_eq!(missing.code(), "content.template_not_found");
        let bad = content
            .draft_from_template(&tenant, &ada, "return_policy", draft("fr", json!({})))
            .await
            .unwrap_err();
        assert_eq!(bad.kind(), ErrorKind::Validation);

        // Thiếu số ngày đổi trả (hồ sơ không có): báo đủ một lần.
        let error = content
            .draft_from_template(&tenant, &ada, "return_policy", draft("vi", json!({})))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "content.template_variables_missing");
        let fields: Vec<&str> = error.fields().iter().map(|field| field.field.as_str()).collect();
        assert_eq!(fields, ["variables.policy.return_days"]);

        let entry = content
            .draft_from_template(
                &tenant,
                &ada,
                "return_policy",
                draft(
                    "vi",
                    json!({"policy": {"return_days": 7}, "contact": {"phone": "0912 345 678"}}),
                ),
            )
            .await
            .unwrap();
        assert_eq!(entry.status, "draft");
        assert_eq!(entry.locale, "vi");
        assert_eq!(entry.title, "Chính sách đổi trả và hoàn tiền");
        let body: Value = serde_json::from_str(&entry.body).unwrap();
        let text = body.to_string();
        assert!(text.contains("trong 7 ngày"), "{text}");
        // Biến người gọi thắng hồ sơ; giá trị có thẻ chỉ là chữ (không thành node mới).
        assert!(text.contains("0912 345 678") && !text.contains("024 1234 5678"), "{text}");
        assert!(text.contains("Cong ty <b>Vi Du</b>"), "{text}");
        assert!(!text.contains("{{"), "{text}");
        assert!(text.contains("chuyên môn pháp lý"));
        assert!(!text.contains("\"type\":\"html\""));

        // Ảnh chụp bản đã đăng: chưa đăng → không có; đăng rồi → có revision và chữ.
        assert!(content.published_snapshot(&tenant, entry.id).await.unwrap().is_none());
        content.publish(&tenant, &ada, entry.id, None, None, false).await.unwrap();
        let snapshot = content.published_snapshot(&tenant, entry.id).await.unwrap().unwrap();
        assert_eq!(snapshot.title, "Chính sách đổi trả và hoàn tiền");
        assert!(snapshot.plain_text.contains("7 ngày"));

        // Không có hồ sơ pháp lý (app không có `compliance`): người gọi gửi đủ biến.
        let (plain, _, database) =
            service(test, config(&[("AUTOXIDE_CONTENT_LOCALES", "en,vi")])).await;
        let tenant = TenantContext::default_on(database);
        let error = plain
            .draft_from_template(&tenant, &ada, "owner_info", draft("en", json!({})))
            .await
            .unwrap_err();
        assert_eq!(error.fields().len(), 4);
    })
    .await
    .unwrap();
}
