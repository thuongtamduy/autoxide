//! Test sổ thư bị dội (D-20261125-mail-bounces) trên database gate đang chọn: dội cứng lúc gửi được ghi và chặn,
//! thư tới địa chỉ đang chặn không xếp hàng mà để dấu vết, chặn theo từng khách, gỡ chặn và xoá qua REST.

use autoxide_kernel::event::Outbox;
use autoxide_kernel::mail::{Mail, Mailer};
use autoxide_kernel::tenancy::{DatabaseRegistry, TenantContext};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::Value;

use crate::bounces::{Bounces, address_of};
use crate::tests::{fake_smtp_answering, setup_with_database};

fn bounces(limit: u32) -> Bounces {
    Bounces { hard_limit: limit, retention_days: 365, outbox: Outbox::standalone("mail") }
}

/// Ghi một lần dội cứng của `address` cho `tenant`, như job gửi làm.
async fn bounce(service: &Bounces, tenant: &TenantContext, address: &str) -> bool {
    let transaction = tenant.database().begin_write().await.unwrap();
    let suppressed = service.record_hard(&transaction, tenant, address, "550").await.unwrap();
    transaction.commit().await.unwrap();
    suppressed
}

#[tokio::test]
async fn doi_cung_luc_gui_duoc_ghi_va_chan_thu_sau() {
    TestDatabase::run(async |test| {
        let (port, _) = fake_smtp_answering(b"550 5.1.1 no such user\r\n").await;
        let url = format!("smtp://127.0.0.1:{port}");
        let (mailer, worker, database) = setup_with_database(
            test,
            &[("AUTOXIDE_MAIL_SMTP_URL", url.as_str()), ("AUTOXIDE_MAIL_HARD_BOUNCE_LIMIT", "1")],
        )
        .await;
        let tenant = DatabaseRegistry::single(database.clone()).default_tenant();
        mailer.send(&tenant, Mail::new("Ghost <Ghost@Example.com>", "Hi", "Hello")).await.unwrap();
        assert_eq!(worker.run_once().await.unwrap(), 1);
        let service = bounces(1);
        // Đã chặn sau một lần (ngưỡng 1): thư sau không xếp hàng, để dấu vết với nguồn của thư.
        let mut reset = Mail::new("ghost@example.com", "Reset", "Your code");
        reset.dedupe_key = Some("auth.password_reset:42".to_owned());
        mailer.send(&tenant, reset).await.unwrap();
        assert_eq!(worker.run_once().await.unwrap(), 0, "nothing queued for a suppressed address");
        let filter = crate::bounces::BounceFilter {
            email: Some("GHOST@example.com".to_owned()),
            suppressed: Some(true),
        };
        let owner =
            autoxide_kernel::access::Principal::with_all_permissions(uuid::Uuid::from_u128(1))
                .in_tenant(tenant.id());
        let page = autoxide_kernel::pagination::PageRequest::default();
        let rows = service.list(&tenant, &owner, &filter, &page).await.unwrap().data;
        assert_eq!(rows.len(), 1);
        let row = &rows[0].row;
        assert_eq!(
            (row.address.as_str(), row.hard_count, row.last_code.as_str(), row.skipped_count),
            ("ghost@example.com", 1, "550", 1)
        );
        assert!(row.suppressed_at.is_some());
        assert_eq!(row.last_skipped_source.as_deref(), Some("auth.password_reset"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chan_theo_tung_khach_go_chan_va_xoa() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .tenant("a")
            .tenant("b")
            .user_in("a", "owner-a", 1, &["mail.bounces.read", "mail.bounces.manage"])
            .user_in("b", "owner-b", 2, &["mail.bounces.read", "mail.bounces.manage"])
            .user_in("a", "reader-a", 3, &[])
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let (a, b) = (app.tenant("a").await.unwrap(), app.tenant("b").await.unwrap());
        let service = bounces(2);
        let address = address_of("ada@example.com").unwrap();
        assert!(!bounce(&service, &a, &address).await, "one bounce: not yet");
        assert!(bounce(&service, &a, &address).await, "second bounce: suppressed");
        assert!(!bounce(&service, &a, &address).await, "already suppressed: no second event");

        // Chặn chỉ ở khách A: thư của khách B tới cùng địa chỉ vẫn đi.
        assert!(service.skip_if_suppressed(&a, &address, None).await.unwrap());
        assert!(!service.skip_if_suppressed(&b, &address, None).await.unwrap());
        let mailer = app.api::<dyn Mailer>().unwrap();
        mailer.send(&b, Mail::new("ada@example.com", "Hi", "Hello")).await.unwrap();

        let list = |token: &'static str, query: &'static str| {
            app.get(&format!("/api/v1/mail/bounces{query}")).token(token).send()
        };
        let listed = list("owner-a", "?email=Ada@Example.com").await.unwrap();
        assert_eq!(listed.status(), StatusCode::OK, "{}", listed.text());
        let listed = listed.json();
        let rows = listed["data"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{listed}");
        assert_eq!(
            (&rows[0]["hard_bounces"], &rows[0]["suppressed"]),
            (&Value::from(3), &Value::Bool(true))
        );
        let id = rows[0]["id"].as_str().unwrap().to_owned();
        // Khách B không thấy, không gỡ được bản ghi của A.
        assert_eq!(list("owner-b", "").await.unwrap().json()["data"], serde_json::json!([]));
        let foreign = app.get(&format!("/api/v1/mail/bounces/{id}")).token("owner-b").send();
        assert_eq!(foreign.await.unwrap().status(), StatusCode::NOT_FOUND);
        let released = app.post(&format!("/api/v1/mail/bounces/{id}/release")).token("owner-b");
        assert_eq!(released.send().await.unwrap().status(), StatusCode::NOT_FOUND);
        // Thiếu permission.
        assert_eq!(list("reader-a", "").await.unwrap().status(), StatusCode::FORBIDDEN);
        let bad = list("owner-a", "?email=not-an-address").await.unwrap();
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let stats = app.get("/api/v1/mail/bounces/stats").token("owner-a").send().await.unwrap();
        let stats = stats.json();
        assert_eq!(
            (&stats["data"]["addresses"], &stats["data"]["suppressed"]),
            (&Value::from(1), &Value::from(1))
        );

        // Gỡ chặn: thư đi lại; số lần tính chặn về 0, tổng giữ nguyên.
        let released = app.post(&format!("/api/v1/mail/bounces/{id}/release")).token("owner-a");
        let released = released.send().await.unwrap();
        assert_eq!(released.status(), StatusCode::OK, "{}", released.text());
        let released = released.json();
        assert_eq!(
            (
                &released["data"]["suppressed"],
                &released["data"]["strikes"],
                &released["data"]["hard_bounces"]
            ),
            (&Value::Bool(false), &Value::from(0), &Value::from(3))
        );
        assert!(!service.skip_if_suppressed(&a, &address, None).await.unwrap());
        // Xoá dấu vết (yêu cầu của chủ dữ liệu).
        let deleted = app.delete(&format!("/api/v1/mail/bounces/{id}")).token("owner-a");
        assert_eq!(deleted.send().await.unwrap().status(), StatusCode::NO_CONTENT);
        assert_eq!(list("owner-a", "").await.unwrap().json()["data"], serde_json::json!([]));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hai_lan_doi_cung_luc_khong_tao_hai_dong() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).tenant("a").module(crate::module()).build().await.unwrap();
        let a = app.tenant("a").await.unwrap();
        let service = std::sync::Arc::new(bounces(3));
        let address = "race@example.com";
        let tasks: Vec<_> = (0..4)
            .map(|_| {
                let (service, a) = (service.clone(), a.clone());
                tokio::spawn(async move { bounce(&service, &a, address).await })
            })
            .collect();
        let mut suppressed = 0;
        for task in tasks {
            if task.await.unwrap() {
                suppressed += 1;
            }
        }
        assert_eq!(suppressed, 1, "exactly one bounce crosses the limit");
        let owner =
            autoxide_kernel::access::Principal::with_all_permissions(uuid::Uuid::from_u128(1))
                .in_tenant(a.id());
        let page = autoxide_kernel::pagination::PageRequest::default();
        let rows = service
            .list(&a, &owner, &crate::bounces::BounceFilter::default(), &page)
            .await
            .unwrap()
            .data;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].row.hard_count, 4);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn don_theo_thoi_han_giu() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).tenant("a").module(crate::module()).build().await.unwrap();
        let a = app.tenant("a").await.unwrap();
        let service = bounces(3);
        bounce(&service, &a, "old@example.com").await;
        let soon = autoxide_kernel::time::Timestamp::now();
        assert_eq!(service.cleanup(a.database(), soon).await.unwrap(), 0);
        let later = soon + jiff::SignedDuration::from_hours(24 * 366);
        assert_eq!(service.cleanup(a.database(), later).await.unwrap(), 1);
    })
    .await
    .unwrap();
}
