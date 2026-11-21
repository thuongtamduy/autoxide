//! Chương trình: tạo, sửa, khoá sau khi chạy, lưu vết, kiểm tuân thủ khi bật, rò rỉ giữa cửa hàng.

use autoxide_kernel::time::Timestamp;
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use jiff::SignedDuration;
use serde_json::json;

use super::{activate, body, call, create, load_legal, running, setup};

#[tokio::test]
async fn tao_sua_xoa_ban_nhap() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let id = create(&setup, body(setup.tree.a, json!({"priority": 3}))).await;
        let (status, found) =
            call(&setup.app, "GET", &format!("/promotions/{id}"), "boss", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(found["data"]["status"], "draft");
        assert_eq!(found["data"]["rule"]["priority"], 3);
        let (status, changed) = call(
            &setup.app,
            "PATCH",
            &format!("/promotions/{id}"),
            "boss",
            Some(json!({"value_bp": 1500, "title": "Mới"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{changed}");
        assert_eq!(changed["data"]["rule"]["value_bp"], 1500);
        assert_eq!(changed["data"]["texts"][0]["title"], "Mới");
        let (status, list) = call(&setup.app, "GET", "/promotions?q=Mới", "boss", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(list["data"].as_array().unwrap().len(), 1);
        let (status, _) =
            call(&setup.app, "DELETE", &format!("/promotions/{id}"), "boss", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = call(&setup.app, "GET", &format!("/promotions/{id}"), "boss", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khong_tao_duoc_khi_ends_at_truoc_starts_at_va_bao_moi_loi_mot_lan() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let bad = body(
            setup.tree.a,
            json!({"ends_at": Timestamp::now() - SignedDuration::from_hours(24), "value_bp": 20000, "title": ""}),
        );
        let (status, error) = call(&setup.app, "POST", "/promotions", "boss", Some(bad)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(error["errors"].as_array().unwrap().len() >= 3, "{error}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn sua_sau_khi_chay_bi_khoa_va_moi_lan_doi_co_phien_ban() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let id = running(&setup, body(setup.tree.a, json!({"usage_limit": 10}))).await;
        let uri = format!("/promotions/{id}");
        let (status, error) =
            call(&setup.app, "PATCH", &uri, "boss", Some(json!({"value_bp": 2000}))).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(error["code"], "promotions.locked_after_start");
        let (status, error) =
            call(&setup.app, "PATCH", &uri, "boss", Some(json!({"usage_limit": 5}))).await;
        assert_eq!(status, StatusCode::CONFLICT, "{error}");
        let later = Timestamp::now() + SignedDuration::from_hours(24 * 9);
        let (status, changed) = call(
            &setup.app,
            "PATCH",
            &uri,
            "boss",
            Some(json!({"usage_limit": 20, "ends_at": later})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{changed}");
        let (status, _) = call(
            &setup.app,
            "PUT",
            &format!("/promotions/{id}/translations/en"),
            "boss",
            Some(json!({"title": "Sale", "terms": "Whole store"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = call(
            &setup.app,
            "PUT",
            &format!("/promotions/{id}/targets"),
            "boss",
            Some(json!({"targets": []})),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        let (status, _) =
            call(&setup.app, "POST", &format!("/promotions/{id}/pause"), "boss", None).await;
        assert_eq!(status, StatusCode::OK);
        let (_, versions) =
            call(&setup.app, "GET", &format!("/promotions/{id}/versions"), "boss", None).await;
        let actions: Vec<&str> = versions["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["action"].as_str().unwrap())
            .collect();
        assert_eq!(actions, vec!["activated", "updated", "translation", "paused"]);
        let (status, _) = call(&setup.app, "DELETE", &uri, "boss", None).await;
        assert_eq!(status, StatusCode::CONFLICT);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chan_kich_hoat_vuot_tran_50_phan_tram() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let id = create(&setup, body(setup.tree.a, json!({"value_bp": 6000}))).await;
        let (status, error) = activate(&setup, &id).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
        assert_eq!(error["code"], "promotions.exceeds_legal_limit");
        // Giảm theo số tiền: so với giá ngay trước khuyến mại của biến thể.
        let variant = setup.catalog.add(setup.tree.a, 100_000, &[]);
        let amount = create(
            &setup,
            body(setup.tree.a, json!({"kind": "amount_off", "value_bp": null, "value": {"amount": "60000", "currency": "VND"}})),
        )
        .await;
        call(
            &setup.app,
            "PUT",
            &format!("/promotions/{amount}/targets"),
            "boss",
            Some(json!({"targets": [{"target_type": "variant", "target_id": variant}]})),
        )
        .await;
        let (status, error) = activate(&setup, &amount).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
        assert!(error["detail"].as_str().unwrap().contains("60%"), "{error}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chuong_trinh_tap_trung_quoc_gia_theo_du_lieu() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let id = create(
            &setup,
            body(setup.tree.a, json!({"value_bp": 7000, "program_kind": "national_focus"})),
        )
        .await;
        let (status, activated) = activate(&setup, &id).await;
        assert_eq!(status, StatusCode::OK, "{activated}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khong_co_du_lieu_luat_thi_khong_tu_cho_qua() {
    TestDatabase::run(async |test| {
        let setup = setup(test, false).await;
        let id = create(&setup, body(setup.tree.a, json!({}))).await;
        let (status, error) = activate(&setup, &id).await;
        assert_eq!(status, StatusCode::CONFLICT, "{error}");
        assert_eq!(error["code"], "promotions.no_legal_data");
        // Nạp lại cùng gói không nhân đôi.
        assert!(load_legal(&setup.app).await.contains("Loaded"));
        assert!(load_legal(&setup.app).await.contains("already loaded"));
        let (_, limits) =
            call(&setup.app, "GET", "/legal-limits?at=2024-11-30", "boss", None).await;
        let regular = limits["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|limit| {
                limit["promotion_kind"] == "price_reduction" && limit["program_kind"] == "regular"
            })
            .unwrap()
            .clone();
        assert_eq!(regular["max_days_per_year"], 90);
        let (_, limits) =
            call(&setup.app, "GET", "/legal-limits?at=2024-12-01", "boss", None).await;
        assert!(
            limits["data"]
                .as_array()
                .unwrap()
                .iter()
                .any(|limit| limit["max_days_per_year"] == 120)
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn can_thong_bao_truoc_khi_chay_va_kiem_khong_ghi() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let id = create(&setup, body(setup.tree.a, json!({}))).await;
        let (status, checks) =
            call(&setup.app, "POST", &format!("/promotions/{id}/activation-check"), "boss", None)
                .await;
        assert_eq!(status, StatusCode::OK);
        let notice = checks["data"]["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|check| check["code"] == "promotions.notice_required")
            .unwrap()
            .clone();
        assert_eq!(notice["ok"], false);
        // Kiểm không ghi: chương trình vẫn là bản nháp.
        let (_, found) = call(&setup.app, "GET", &format!("/promotions/{id}"), "boss", None).await;
        assert_eq!(found["data"]["status"], "draft");
        let (status, error) =
            call(&setup.app, "POST", &format!("/promotions/{id}/activate"), "boss", None).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(error["code"], "promotions.notice_required");
        let (status, activated) = activate(&setup, &id).await;
        assert_eq!(status, StatusCode::OK, "{activated}");
        assert_eq!(activated["data"]["promotion"]["status"], "active");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn vuot_so_ngay_giam_gia_trong_nam() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        // Năm sau, để chương trình nằm gọn trong một năm: 100 ngày đã chạy + 30 ngày > 120.
        let year = Timestamp::now().format("%Y").to_string().parse::<i32>().unwrap() + 1;
        let at = |month: u32, day: u32| format!("{year}-{month:02}-{day:02}T00:00:00Z");
        let first = create(
            &setup,
            body(setup.tree.a, json!({"starts_at": at(1, 10), "ends_at": at(4, 20)})),
        )
        .await;
        let (status, activated) = activate(&setup, &first).await;
        assert_eq!(status, StatusCode::OK, "{activated}");
        assert_eq!(activated["data"]["promotion"]["status"], "scheduled");
        let second =
            create(&setup, body(setup.tree.a, json!({"starts_at": at(6, 1), "ends_at": at(7, 1)})))
                .await;
        let (status, error) = activate(&setup, &second).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
        assert_eq!(error["code"], "promotions.exceeds_duration_limit");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ro_ri_giua_hai_cua_hang_anh_em() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let (status, created) =
            call(&setup.app, "POST", "/promotions", "store_a", Some(body(setup.tree.a, json!({}))))
                .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let id = created["data"]["id"].as_str().unwrap().to_owned();
        let (status, _) =
            call(&setup.app, "GET", &format!("/promotions/{id}"), "store_b", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = call(
            &setup.app,
            "PATCH",
            &format!("/promotions/{id}"),
            "store_b",
            Some(json!({"priority": 1})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) =
            call(&setup.app, "POST", "/promotions", "store_b", Some(body(setup.tree.a, json!({}))))
                .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (_, list) = call(&setup.app, "GET", "/promotions", "store_b", None).await;
        assert!(list["data"].as_array().unwrap().is_empty());
        // Chương trình của cửa hàng A không áp cho giỏ ở cửa hàng B.
        let variant = setup.catalog.add(setup.tree.chain, 100_000, &[]);
        activate(&setup, &id).await;
        let mut cart = setup.cart(&[(variant, 1)]);
        let tenant = setup.tenant();
        let at_a = setup.promotions().evaluate(&tenant, &cart).await.unwrap();
        assert_eq!(at_a.total_discount.minor(), 10_000);
        cart.org_id = setup.tree.b;
        let at_b = setup.promotions().evaluate(&tenant, &cart).await.unwrap();
        assert!(at_b.applied.is_empty());
    })
    .await
    .unwrap();
}

/// Payload của mọi event `promotions.automatic_prices_changed` đã ghi, cũ trước.
async fn price_events(setup: &super::Setup) -> Vec<serde_json::Value> {
    use sea_orm::{ConnectionTrait, Statement};
    let read = setup.app.database().read();
    let sql = "SELECT payload FROM promotions_events \
               WHERE name = 'promotions.automatic_prices_changed' ORDER BY id";
    let rows =
        read.query_all_raw(Statement::from_string(read.get_database_backend(), sql)).await.unwrap();
    rows.iter()
        .map(|row| {
            let payload: String = row.try_get("", "payload").unwrap();
            serde_json::from_str(&payload).unwrap()
        })
        .collect()
}

#[tokio::test]
async fn gia_tu_dong_doi_khi_bat_dau_va_ket_thuc() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let variant = setup.catalog.add(setup.tree.a, 100_000, &[]);
        let id = create(&setup, body(setup.tree.a, json!({}))).await;
        let (status, put) = call(
            &setup.app,
            "PUT",
            &format!("/promotions/{id}/targets"),
            "boss",
            Some(json!({"targets": [{"target_type": "variant", "target_id": variant}]})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{put}");
        assert!(price_events(&setup).await.is_empty(), "bản nháp chưa có giá");
        let (status, activated) = activate(&setup, &id).await;
        assert_eq!(status, StatusCode::OK, "{activated}");
        let (status, ended) =
            call(&setup.app, "POST", &format!("/promotions/{id}/end"), "boss", None).await;
        assert_eq!(status, StatusCode::OK, "{ended}");
        let found = price_events(&setup).await;
        assert_eq!(found.len(), 2, "{found:?}");
        for event in &found {
            assert_eq!(event["after"]["promotion_id"], id.as_str());
            assert_eq!(event["after"]["variant_ids"], json!([variant]));
            assert_eq!(event["after"]["whole_store"], false);
        }

        // Có điều kiện theo giỏ (đơn tối thiểu): giá chỉ biết lúc tính giỏ, không phát event.
        running(
            &setup,
            body(setup.tree.a, json!({"min_order": {"amount": "500000", "currency": "VND"}})),
        )
        .await;
        assert_eq!(price_events(&setup).await.len(), 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn xuat_thong_bao_theo_noi_nhan_json_va_csv() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let id = create(&setup, body(setup.tree.a, json!({"terms": "=HYPERLINK(\"x\")"}))).await;
        let uri = format!("/promotions/{id}/notice-export");
        let (status, export) = call(&setup.app, "GET", &uri, "boss", None).await;
        assert_eq!(status, StatusCode::OK, "{export}");
        let data = &export["data"];
        assert_eq!(data["rules_found"], true);
        assert_eq!(data["promotion_kind"], "price_reduction");
        let entry = &data["authorities"][0];
        assert_eq!(entry["authority_code"], "province:unknown");
        assert_eq!(
            (entry["required"].clone(), entry["status"].clone()),
            (json!(true), json!("pending"))
        );
        assert!(entry["due_on"].is_string() && entry["legal_basis"].is_string(), "{entry}");

        let (status, _) = call(
            &setup.app,
            "PATCH",
            &format!("/promotions/{id}/notices/province:unknown"),
            "boss",
            Some(json!({"status": "submitted", "reference": "TB-42"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (_, export) = call(&setup.app, "GET", &uri, "boss", None).await;
        assert_eq!(export["data"]["authorities"][0]["status"], "submitted");
        assert_eq!(export["data"]["authorities"][0]["reference"], "TB-42");

        // CSV: một dòng mỗi nơi nhận; ô bắt đầu bằng `=` bị vô hiệu (chống CSV injection).
        let (status, csv) =
            call(&setup.app, "GET", &format!("{uri}?format=csv"), "boss", None).await;
        assert_eq!(status, StatusCode::OK);
        let csv = csv.as_str().unwrap();
        assert!(csv.contains("\"authority_code\""), "{csv}");
        assert!(csv.contains("\"TB-42\""));
        assert!(csv.contains("\"'=HYPERLINK(\"\"x\"\")\""), "{csv}");
        assert_eq!(csv.lines().count(), 2);

        // Cửa hàng B không thấy chương trình của A; thiếu `promotions.compliance` thì 403.
        let (status, _) = call(&setup.app, "GET", &uri, "store_b", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = call(&setup.app, "GET", &uri, "buyer0", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn danh_sach_cong_khai_kem_thong_bao_va_tran_theo_luat() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        // Chuỗi (nút cha của A): hiện ở website của A. Cửa hàng B: không. Không `public`: không.
        let chain = running(&setup, body(setup.tree.chain, json!({"public": true}))).await;
        running(&setup, body(setup.tree.b, json!({"public": true}))).await;
        running(&setup, body(setup.tree.a, json!({"public": false}))).await;
        let uri = format!(
            "/api/v1/promotions/public/promotions?site={}&locale=vi",
            setup.tree.a.as_uuid()
        );
        let answer = setup.app.request(axum::http::Method::GET, &uri).send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::OK, "{}", answer.text());
        let etag = answer.header("etag").to_owned();
        let found: serde_json::Value = serde_json::from_slice(answer.body()).unwrap();
        let rows = found["data"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{found}");
        let row = &rows[0];
        assert_eq!(row["id"], chain.to_string());
        assert_eq!((row["title"].clone(), row["locale"].clone()), (json!("Giảm giá"), json!("vi")));
        assert_eq!(row["notices"][0]["reference"], "TB-1");
        assert_eq!(row["legal_limit"]["max_bp"], 5000);
        assert_eq!(row["legal_limit"]["promotion_kind"], "price_reduction");

        // Không đổi gì: cùng ETag → 304.
        let again = setup
            .app
            .request(axum::http::Method::GET, &uri)
            .header("if-none-match", &etag)
            .send()
            .await
            .unwrap();
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn flash_sale_cong_khai_voi_suat_con_that() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let variant = setup.catalog.add(setup.tree.a, 200_000, &[]);
        let id = create(
            &setup,
            body(setup.tree.a, json!({"kind": "flash_sale", "value_bp": null, "public": true})),
        )
        .await;
        call(
            &setup.app,
            "PUT",
            &format!("/promotions/{id}/flash-items"),
            "boss",
            Some(json!({"items": [{"variant_id": variant, "sale_price_minor": 120000, "quantity_limit": 5}]})),
        )
        .await;
        let (status, activated) = activate(&setup, &id).await;
        assert_eq!(status, StatusCode::OK, "{activated}");
        // Một người giữ 2 suất: còn 3.
        let request = crate::api::ReserveRequest::new(
            uuid::Uuid::now_v7(),
            setup.cart(&[(variant, 2)]),
            Timestamp::now() + SignedDuration::from_hours(1),
        );
        setup.promotions().reserve(&setup.tenant(), &request).await.unwrap();
        let uri = format!(
            "/api/v1/promotions/public/promotions/flash-sales?site={}",
            setup.tree.a.as_uuid()
        );
        let answer = setup.app.request(axum::http::Method::GET, &uri).send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::OK, "{}", answer.text());
        let found: serde_json::Value = serde_json::from_slice(answer.body()).unwrap();
        let sale = &found["data"][0];
        assert_eq!(sale["state"], "running");
        assert_eq!(sale["items"][0]["sale_price_minor"], 120_000);
        assert_eq!(sale["items"][0]["quantity_left"], 3);
    })
    .await
    .unwrap();
}

/// Job theo lịch `promotions.send_reminders` chạy ngay.
#[derive(Debug, serde::Serialize)]
struct SendRemindersNow {}

impl autoxide_kernel::job::Job for SendRemindersNow {
    const KIND: &'static str = crate::jobs::SEND_REMINDERS;
}

#[tokio::test]
async fn nhac_khi_flash_sale_bat_dau_gui_mot_lan_kem_dong_y_rieng() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let variant = setup.catalog.add(setup.tree.a, 200_000, &[]);
        let starts = Timestamp::now() + SignedDuration::from_hours(1);
        let id = create(
            &setup,
            body(
                setup.tree.a,
                json!({"kind": "flash_sale", "value_bp": null, "public": true, "starts_at": starts}),
            ),
        )
        .await;
        call(
            &setup.app,
            "PUT",
            &format!("/promotions/{id}/flash-items"),
            "boss",
            Some(json!({"items": [{"variant_id": variant, "sale_price_minor": 120000}]})),
        )
        .await;
        let (status, activated) = activate(&setup, &id).await;
        assert_eq!(status, StatusCode::OK, "{activated}");
        let uri = format!("/public/promotions/{id}/reminders");
        let (status, made) = call(&setup.app, "POST", &uri, "buyer0", None).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        let (status, _) = call(&setup.app, "POST", &uri, "buyer0", None).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = call(&setup.app, "POST", &uri, "buyer1", None).await;
        assert_eq!(status, StatusCode::CREATED);
        // Đồng ý riêng cho lời nhắc, không phải quảng cáo.
        let consents = setup.consents.0.lock().unwrap().clone();
        assert!(consents.iter().all(|row| row.0.purpose == "promotions.flash_sale_reminder"));
        // buyer1 đổi ý: bỏ lời nhắc, rút đồng ý.
        let (status, _) = call(&setup.app, "DELETE", &uri, "buyer1", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(setup.consents.0.lock().unwrap().iter().any(|row| !row.2));

        // Tới hạn: job gửi đúng một lần, chạy lại không gửi thêm.
        {
            use sea_orm::sea_query::Expr;
            use sea_orm::EntityTrait;
            crate::entity::reminder::Entity::update_many()
                .col_expr(
                    crate::entity::reminder::Column::RemindAt,
                    Expr::value(Timestamp::now() - SignedDuration::from_secs(1)),
                )
                .exec(setup.app.database().write())
                .await
                .unwrap();
        }
        for _ in 0..2 {
            autoxide_kernel::job::JobQueue::new("promotions")
                .enqueue_database(
                    setup.app.database().write(),
                    &SendRemindersNow {},
                    autoxide_kernel::job::EnqueueOptions::default(),
                )
                .await
                .unwrap();
            super::drain(&setup.app).await;
        }
        let sent = setup.notifier.0.lock().unwrap().clone();
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(sent[0].kind, "promotions.flash_sale_starting");
        assert_eq!(sent[0].params["title"], "Giảm giá");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bam_nhac_toi_dong_thoi_chi_mot_loi_nhac() {
    TestDatabase::run(async |test| {
        let setup = std::sync::Arc::new(setup(test, true).await);
        let variant = setup.catalog.add(setup.tree.a, 200_000, &[]);
        let starts = Timestamp::now() + SignedDuration::from_hours(2);
        let id = create(
            &setup,
            body(
                setup.tree.a,
                json!({"kind": "flash_sale", "value_bp": null, "public": true, "starts_at": starts}),
            ),
        )
        .await;
        call(
            &setup.app,
            "PUT",
            &format!("/promotions/{id}/flash-items"),
            "boss",
            Some(json!({"items": [{"variant_id": variant, "sale_price_minor": 120000}]})),
        )
        .await;
        activate(&setup, &id).await;
        let uri = format!("/public/promotions/{id}/reminders");
        let tasks: Vec<_> = (0..8)
            .map(|_| {
                let setup = setup.clone();
                let uri = uri.clone();
                tokio::spawn(async move { call(&setup.app, "POST", &uri, "buyer0", None).await.0 })
            })
            .collect();
        let mut statuses = Vec::new();
        for task in tasks {
            statuses.push(task.await.unwrap());
        }
        assert!(statuses.iter().all(|status| status.is_success()), "{statuses:?}");
        use sea_orm::{EntityTrait, PaginatorTrait};
        let rows = crate::entity::reminder::Entity::find()
            .count(setup.app.database().read())
            .await
            .unwrap();
        assert_eq!(rows, 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn so_doi_tuong_chi_cong_khai_khi_dang_chay() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let live = running(&setup, body(setup.tree.a, json!({"public": true}))).await;
        let hidden = running(&setup, body(setup.tree.a, json!({"public": false}))).await;
        let draft = create(&setup, body(setup.tree.a, json!({"public": true}))).await;
        let resources = setup.app.modules().resources();
        assert!(resources.knows("promotions.promotion"));
        let found = resources
            .resolve_many(
                &setup.tenant(),
                "promotions.promotion",
                &[live.to_string(), hidden.to_string(), draft.clone()],
                Some("vi"),
            )
            .await
            .unwrap();
        let shown = &found[&live.to_string()];
        assert!(shown.public);
        assert_eq!(shown.title, "Giảm giá");
        assert_eq!(shown.url.as_deref(), Some(format!("/promotions/{live}").as_str()));
        for id in [hidden.to_string(), draft] {
            assert!(!found[&id].public);
            assert_eq!(found[&id].title, "");
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tam_dung_va_ket_thuc_cung_luc_khong_phat_event_sai() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        for _ in 0..4 {
            let id = running(&setup, body(setup.tree.a, json!({}))).await;
            // Tạm dừng và kết thúc cùng lúc. Postgres từng cho cả hai cùng xuất phát từ `active`:
            // bên thua không đổi được dòng nhưng vẫn trả 200, ghi phiên bản và phát event (vd
            // `promotion_ended` cho chương trình đang tạm dừng, nhả suất hạn mức gói).
            let (pause, end) = (format!("/promotions/{id}/pause"), format!("/promotions/{id}/end"));
            let (paused, ended) = tokio::join!(
                call(&setup.app, "POST", &pause, "boss", None),
                call(&setup.app, "POST", &end, "boss", None),
            );
            assert_eq!(ended.0, StatusCode::OK, "{}", ended.1);
            assert_eq!(ended.1["data"]["status"], "ended", "{}", ended.1);
            match paused.0 {
                StatusCode::OK => assert_eq!(paused.1["data"]["status"], "paused", "{}", paused.1),
                StatusCode::CONFLICT => {}
                other => panic!("pause: {other} {}", paused.1),
            }
            let (_, versions) =
                call(&setup.app, "GET", &format!("/promotions/{id}/versions"), "boss", None).await;
            let actions: Vec<&str> = versions["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["action"].as_str().unwrap())
                .collect();
            let expected: &[&str] = if paused.0 == StatusCode::OK {
                &["activated", "paused", "ended"]
            } else {
                &["activated", "ended"]
            };
            assert_eq!(actions, expected);
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bat_hai_chuong_trinh_cung_luc_khong_vuot_so_ngay_trong_nam() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        // Năm sau: mỗi chương trình 70 ngày (một mình không vượt 120), cả hai thì 140 > 120.
        let year = Timestamp::now().format("%Y").to_string().parse::<i32>().unwrap() + 1;
        let at = |month: u32, day: u32| format!("{year}-{month:02}-{day:02}T00:00:00Z");
        for (start, end) in [(at(1, 1), at(3, 11)), (at(5, 1), at(7, 10)), (at(9, 1), at(11, 10))] {
            let window = json!({"starts_at": start, "ends_at": end});
            let first = create(&setup, body(setup.tree.a, window.clone())).await;
            let second = create(&setup, body(setup.tree.a, window)).await;
            // Bật cùng lúc. Bộ kiểm chạy ngoài transaction: cả hai từng cùng thấy chương trình
            // kia còn nháp và cùng bật, tổng số ngày giảm giá vượt trần (cả hai database).
            let (one, two) = tokio::join!(activate(&setup, &first), activate(&setup, &second));
            let ok = [&one, &two].iter().filter(|(status, _)| *status == StatusCode::OK).count();
            assert_eq!(ok, 1, "{} / {}", one.1, two.1);
            let failed = if one.0 == StatusCode::OK { &two } else { &one };
            assert_eq!(failed.0, StatusCode::UNPROCESSABLE_ENTITY, "{}", failed.1);
            assert_eq!(failed.1["code"], "promotions.exceeds_duration_limit");
            // Lượt sau trên cùng mục tiêu: dọn để mỗi lượt bắt đầu từ 0 ngày đã dùng.
            let winner = if one.0 == StatusCode::OK { &first } else { &second };
            let (status, _) =
                call(&setup.app, "POST", &format!("/promotions/{winner}/cancel"), "boss", None)
                    .await;
            assert_eq!(status, StatusCode::OK);
        }
    })
    .await
    .unwrap();
}
