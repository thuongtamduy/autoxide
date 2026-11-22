//! Sổ đồng ý chung (bước 37 C3, D-367), cả SQLite và Postgres: xác nhận ghi sổ, huỷ rút đồng ý, rút
//! ở `compliance` huỷ đăng ký, chiến dịch bỏ qua người đã rút, đồng bộ phần còn sót chạy lại không
//! trùng, sổ lỗi thì không gửi; chiến dịch `template` cá nhân hoá từng người nhận.

use autoxide_compliance::api::testing::FakeConsentLedger;
use std::sync::Arc;

use autoxide_compliance::api::{ConsentKey, ConsentSubject};
use autoxide_testkit::TestDatabase;
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query};

use super::*;
use crate::campaigns::CampaignFormat;
use crate::testing::{config, service_with_ledger};

async fn harness(test: &TestDatabase) -> (Harness, Arc<FakeConsentLedger>) {
    let ledger = Arc::new(FakeConsentLedger::new());
    let h = service_with_ledger(test, config(), ledger.clone()).await;
    (h, ledger)
}

fn email(address: &str) -> ConsentSubject {
    ConsentSubject::Email(address.to_owned())
}

fn scope(list: &List) -> String {
    format!("newsletter.list:{}", list.row.id)
}

#[tokio::test]
async fn xac_nhan_ghi_so_huy_rut_dong_y() {
    TestDatabase::run(async |test| {
        let (h, ledger) = harness(test).await;
        let weekly = list(&h, "weekly", false).await;
        subscribe(&h, "an@example.com", &["weekly"], "203.0.113.1").await.unwrap();
        assert_eq!(
            ledger
                .state_of(&email("an@example.com"), &crate::consents::consent_key(weekly.row.id))
                .as_deref(),
            Some("granted")
        );
        assert_eq!(subscriptions(&h.database).await[0].ledger_state, "granted");

        // Huỷ một chạm từ thư chiến dịch: rút đồng ý trong sổ.
        let sent = campaign(&h, weekly.row.id, false).await;
        send(&h, sent.row.id).await;
        drain(&h).await;
        let token = unsubscribe_token(&h.mailer.to("an@example.com")[0]);
        h.service.unsubscribe(Some(&h.tenant), &token).await.unwrap();
        assert_eq!(
            ledger
                .state_of(&email("an@example.com"), &crate::consents::consent_key(weekly.row.id))
                .as_deref(),
            Some("withdrawn")
        );
        assert_eq!(subscriptions(&h.database).await[0].ledger_state, "withdrawn");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn rut_o_compliance_huy_dang_ky_va_chien_dich_bo_qua_nguoi_da_rut() {
    TestDatabase::run(async |test| {
        let (h, ledger) = harness(test).await;
        let weekly = list(&h, "weekly", false).await;
        for email in ["an@example.com", "binh@example.com", "chi@example.com"] {
            subscribe(&h, email, &["weekly"], "203.0.113.1").await.unwrap();
        }
        // Rút ở `compliance` cho danh sách này: event → huỷ đăng ký ở đây, không gọi lại sổ.
        let consent_id =
            ledger.deny(email("an@example.com"), crate::consents::consent_key(weekly.row.id)).id;
        let changed =
            h.service.consent_withdrawn(&h.tenant, consent_id, &scope(&weekly)).await.unwrap();
        assert_eq!(changed, 1);
        // Nhận lại event không đổi gì.
        assert_eq!(
            h.service.consent_withdrawn(&h.tenant, consent_id, &scope(&weekly)).await.unwrap(),
            0
        );
        // Rút mọi quảng cáo ở khoá rộng hơn: người đó vẫn `confirmed` ở đây nhưng lô bỏ qua.
        ledger.deny(email("binh@example.com"), ConsentKey::new("marketing", "", ""));
        let sent = campaign(&h, weekly.row.id, false).await;
        send(&h, sent.row.id).await;
        drain(&h).await;
        assert!(h.mailer.to("an@example.com").is_empty());
        assert!(h.mailer.to("binh@example.com").is_empty());
        assert_eq!(h.mailer.to("chi@example.com").len(), 1);
        let reasons: Vec<String> = h
            .database
            .read()
            .query_all(
                &Query::select()
                    .column(Alias::new("error"))
                    .from(Alias::new("newsletter_deliveries"))
                    .and_where(Expr::col(Alias::new("status")).eq("skipped"))
                    .to_owned(),
            )
            .await
            .unwrap()
            .iter()
            .map(|row| row.try_get::<String>("", "error").unwrap())
            .collect();
        assert_eq!(reasons, ["consent_withdrawn"], "{reasons:?}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn backfill_chay_lai_khong_trung_va_so_loi_thi_khong_gui() {
    TestDatabase::run(async |test| {
        let (h, ledger) = harness(test).await;
        let weekly = list(&h, "weekly", false).await;
        // Đăng ký có sẵn từ trước khi có sổ: sổ "lỗi" lúc nhập, `ledger_state` còn rỗng.
        ledger.set_failing(true);
        import(&h, weekly.row.id, &emails("old", 3)).await;
        subscribe(&h, "new@example.com", &["weekly"], "203.0.113.9").await.unwrap();
        assert!(subscriptions(&h.database).await.iter().all(|row| row.ledger_state.is_empty()));
        // Sổ lỗi: lô gửi lỗi (job thử lại), không mail nào đi.
        let sent = campaign(&h, weekly.row.id, false).await;
        send(&h, sent.row.id).await;
        let (_, _, payload) = next_job(&h.database, true).await.unwrap();
        let prepare: crate::campaigns::sending::PrepareJob =
            serde_json::from_value(payload).unwrap();
        h.service.prepare(&h.tenant, &prepare).await.unwrap();
        let send_job = crate::campaigns::sending::SendJob {
            campaign_id: sent.row.id,
            run: prepare.run,
            seq: 0,
        };
        assert!(h.service.send_batch(&h.tenant, &send_job).await.is_err());
        assert!(h.mailer.sent().iter().all(|mail| mail.dedupe_key.is_none()), "no campaign mail");

        ledger.set_failing(false);
        assert_eq!(h.service.backfill_consents(&h.database).await.unwrap(), 4);
        assert_eq!(h.service.backfill_consents(&h.database).await.unwrap(), 0);
        assert_eq!(ledger.source_refs().len(), 4);
        assert_eq!(
            ledger
                .state_of(
                    &email("old000@example.com"),
                    &crate::consents::consent_key(weekly.row.id)
                )
                .as_deref(),
            Some("granted")
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ban_tin_ca_nhan_hoa_moi_nguoi_nhan() {
    TestDatabase::run(async |test| {
        let (h, _ledger) = harness(test).await;
        let weekly = list(&h, "weekly", false).await;
        subscribe(&h, "an@example.com", &["weekly"], "203.0.113.1").await.unwrap();
        subscribe(&h, "binh@example.com", &["weekly"], "203.0.113.2").await.unwrap();
        // Biến ngoài danh sách được phép: lỗi khi lưu, gom một lần.
        let bad = CampaignInput {
            list_id: Some(weekly.row.id),
            subject: Some("{{ secret }}".to_owned()),
            body_text: Some("{% if %}".to_owned()),
            format: Some(CampaignFormat::Template),
            ..CampaignInput::default()
        };
        let error = h.service.create_campaign(&h.tenant, &admin(), bad).await.unwrap_err();
        assert_eq!(error.fields().len(), 2, "{error:?}");
        let input = CampaignInput {
            list_id: Some(weekly.row.id),
            subject: Some("Hi {{ subscriber.email }}".to_owned()),
            body_text: Some("News of {{ list.name }} for {{ subscriber.email }}.".to_owned()),
            body_html: Some(Some("<p>{{ subscriber.email }}</p>".to_owned())),
            format: Some(CampaignFormat::Template),
            ..CampaignInput::default()
        };
        let created = h.service.create_campaign(&h.tenant, &admin(), input).await.unwrap();
        send(&h, created.row.id).await;
        drain(&h).await;
        let an = h.mailer.to("an@example.com").pop().unwrap();
        assert_eq!(an.subject, "Hi a***@example.com");
        assert!(an.text.starts_with("News of weekly news for a***@example.com."), "{}", an.text);
        assert!(an.html.unwrap().contains("<p>a***@example.com</p>"));
        assert_eq!(h.mailer.to("binh@example.com").pop().unwrap().subject, "Hi b***@example.com");
    })
    .await
    .unwrap();
}
