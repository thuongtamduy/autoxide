//! Cấu hình và giờ trống: dịch vụ, nhân viên, lịch tuần, ngày nghỉ, múi giờ của nút.

use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use chrono::{Duration, Utc};
use serde_json::json;

use super::{admin, app, call, next_monday, openings, service, set_setting, staff, utc};

#[tokio::test]
async fn gio_trong_tru_nghi_va_dem() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        set_setting(&app, "organizations.timezone", json!("Asia/Ho_Chi_Minh"));
        let lan = staff(&app, "Lan").await;
        let haircut = service(&app, 60, 15, json!({}), &[&lan]).await;
        let monday = next_monday();

        // Lịch 9:00–12:00, 60 phút + 15 phút dọn: giờ cuối là 10:45.
        let (body, starts) = openings(&app, &haircut, monday).await;
        assert_eq!(body["data"]["timezone"], "Asia/Ho_Chi_Minh");
        assert_eq!(body["data"]["terms"]["deposit"]["amount"], "0");
        let day = monday.format("%Y-%m-%d");
        let expect = |times: &[&str]| -> Vec<String> {
            times.iter().map(|time| format!("{day}T{time}:00+07:00")).collect()
        };
        assert_eq!(starts, expect(&["09:00", "09:15", "09:30", "09:45", "10:00", "10:15", "10:30", "10:45"]));

        // Nghỉ 10:00–10:30 giờ VN (03:00–03:30 UTC).
        let off = json!({
            "resource_id": lan, "kind": "closed", "reason": "Dentist",
            "starts_at": utc(monday, 3, 0), "ends_at": utc(monday, 3, 30),
        });
        let created = admin(&app, "POST", "/bookings/exceptions", Some(off), StatusCode::CREATED).await;
        let (_, starts) = openings(&app, &haircut, monday).await;
        // 9:00 + 75 phút chạm giờ nghỉ: không còn giờ nào trước 10:30.
        assert_eq!(starts, expect(&["10:30", "10:45"]));

        // Bỏ ngày nghỉ: giờ trống trở lại.
        let id = created["data"]["id"].as_str().unwrap();
        admin(&app, "DELETE", &format!("/bookings/exceptions/{id}"), None, StatusCode::NO_CONTENT).await;
        let (_, starts) = openings(&app, &haircut, monday).await;
        assert_eq!(starts.len(), 8);

        // Ca thêm thứ Ba 14:00–15:00 giờ VN (07:00–08:00 UTC).
        let tuesday = monday + Duration::days(1);
        let extra = json!({
            "resource_id": lan, "kind": "open", "starts_at": utc(tuesday, 7, 0), "ends_at": utc(tuesday, 8, 0),
        });
        admin(&app, "POST", "/bookings/exceptions", Some(extra), StatusCode::CREATED).await;
        let short = service(&app, 30, 0, json!({}), &[&lan]).await;
        let (_, starts) = openings(&app, &short, tuesday).await;
        let tuesday = tuesday.format("%Y-%m-%d");
        assert_eq!(
            starts.iter().filter(|start| start.starts_with(&tuesday.to_string())).count(),
            3,
            "14:00, 14:15, 14:30: {starts:?}"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doi_mui_gio_hien_dung_gio_dia_phuong() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let lan = staff(&app, "Lan").await;
        let haircut = service(&app, 60, 0, json!({}), &[&lan]).await;
        let monday = next_monday();

        // Chưa đặt múi giờ: UTC — 9:00 địa phương là 09:00Z.
        let (body, _) = openings(&app, &haircut, monday).await;
        assert_eq!(body["data"]["timezone"], "UTC");
        let first = &body["data"]["openings"][0];
        assert_eq!(first["starts_at"], format!("{}T09:00:00Z", monday.format("%Y-%m-%d")));

        // Cửa hàng ở Việt Nam: vẫn 9:00 địa phương, tức 02:00 UTC.
        set_setting(&app, "organizations.timezone", json!("Asia/Ho_Chi_Minh"));
        let (body, starts) = openings(&app, &haircut, monday).await;
        let first = &body["data"]["openings"][0];
        assert_eq!(first["starts_at"], format!("{}T02:00:00Z", monday.format("%Y-%m-%d")));
        assert_eq!(starts[0], format!("{}T09:00:00+07:00", monday.format("%Y-%m-%d")));

        // Chuyển sang Tokyo: 9:00 địa phương = 00:00 UTC.
        set_setting(&app, "organizations.timezone", json!("Asia/Tokyo"));
        let (_, starts) = openings(&app, &haircut, monday).await;
        assert_eq!(starts[0], format!("{}T09:00:00+09:00", monday.format("%Y-%m-%d")));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cau_hinh_can_quyen_va_bao_moi_loi_mot_lan() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let body = json!({"kind": "staff", "name": "Lan"});
        let (status, problem) =
            call(&app, "POST", "/bookings/resources", Some("reader"), Some(body)).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("permission_denied"))
        );

        let bad = json!({
            "name": " ", "duration_minutes": 7, "buffer_after_minutes": -5,
            "price": {"amount": "100000", "currency": "VND"},
            "deposit": {"kind": "percent", "value": 150}, "capacity": 0,
            "performers": [uuid_of(9)],
        });
        let (status, problem) =
            call(&app, "POST", "/bookings/services", Some("admin"), Some(bad)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        let fields: Vec<&str> = problem["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["field"].as_str().unwrap())
            .collect();
        for field in [
            "name",
            "duration_minutes",
            "buffer_after_minutes",
            "deposit.value",
            "capacity",
            "performers[0]",
        ] {
            assert!(fields.contains(&field), "{field} missing in {fields:?}");
        }

        let lan = staff(&app, "Lan").await;
        let bad = json!({"ranges": [
            {"weekday": 1, "start_minute": 540, "end_minute": 720},
            {"weekday": 1, "start_minute": 700, "end_minute": 800},
            {"weekday": 8, "start_minute": 0, "end_minute": 10},
        ]});
        let (status, problem) = call(
            &app,
            "PUT",
            &format!("/bookings/resources/{lan}/schedule"),
            Some("admin"),
            Some(bad),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");

        // Dịch vụ ngừng nhận đặt: không còn giờ trống công khai.
        let haircut = service(&app, 60, 0, json!({}), &[&lan]).await;
        admin(
            &app,
            "PATCH",
            &format!("/bookings/services/{haircut}"),
            Some(json!({"active": false})),
            StatusCode::OK,
        )
        .await;
        let from = Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
        let to = (Utc::now() + Duration::days(3)).format("%Y-%m-%dT%H:%M:%SZ");
        let uri = format!("/bookings/public/availability?service_id={haircut}&from={from}&to={to}");
        let (status, problem) = call(&app, "GET", &uri, None, None).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("bookings.service_not_found"))
        );
        let (_, listed) = call(&app, "GET", "/bookings/public/services", None, None).await;
        assert_eq!(listed["data"].as_array().unwrap().len(), 0);
    })
    .await
    .unwrap();
}

fn uuid_of(n: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(n)
}
