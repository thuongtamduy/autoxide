//! Chăm sóc khách, đặt lịch, tự động hoá và phân tích dùng được như người vận hành cửa hàng thật (cả
//! SQLite và Postgres): đội hỗ trợ, câu trả lời mẫu, quy tắc chia phiếu, chính sách của đơn vị, phiếu của
//! khách vãng lai (chuyển người, chấm điểm), tool của phiếu; hội thoại (gõ, thu hồi, báo xấu, gọi lại,
//! đóng); dịch vụ và người làm, ngày nghỉ, lịch tải về; luật tự động có bước rủi ro chờ duyệt.

use autoxide_kernel::tenancy::{DEFAULT_TENANT, root_of};
use autoxide_testkit::{TestApp, TestDatabase};
use serde_json::{Value, json};
use uuid::Uuid;

use super::kit::{ANONYMOUS, RealApp, call_tool, drain, expect, id, real_app};

/// Cửa hàng với đăng nhập thật: quản trị viên, và một người mua tự đăng ký tài khoản.
struct Desk {
    real: RealApp,
    buyer: String,
}

async fn desk(test: &TestDatabase) -> Desk {
    let real = real_app(test, &[]).await;
    let body = json!({"email": "buyer@example.com", "name": "An", "password": "a long buyer password", "realm": "customer"});
    expect(&real.app, "POST", "/api/v1/auth/registrations", ANONYMOUS, Some(body), 202).await;
    let slug = real.app.default_tenant().slug().to_owned();
    let answer = real
        .app
        .post("/api/v1/auth/login")
        .header("autoxide-tenant", &slug)
        .json(json!({"email": "buyer@example.com", "password": "a long buyer password", "realm": "customer"}))
        .send()
        .await
        .unwrap();
    let signed_in = answer.json();
    assert_eq!(answer.status(), 200, "{signed_in}");
    let buyer = signed_in["data"]["access_token"].as_str().unwrap().to_owned();
    Desk { real, buyer }
}

async fn get(app: &TestApp, who: &str, path: &str) -> Value {
    expect(app, "GET", path, who, None, 200).await
}

async fn write(
    app: &TestApp,
    who: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
    expected: u16,
) -> Value {
    let method = axum::http::Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method.clone(), path).token(who).header("if-match", "*");
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let json: Value = serde_json::from_slice(answer.body()).unwrap_or(Value::Null);
    assert_eq!(answer.status().as_u16(), expected, "{method} {path}: {json}");
    json
}

#[tokio::test]
async fn doi_ho_tro_cau_tra_loi_mau_chia_phieu_va_phieu_khach_vang_lai() {
    TestDatabase::run(async |test| {
        let desk = desk(test).await;
        let (app, admin) = (&desk.real.app, desk.real.admin.as_str());
        let root = json!(root_of(DEFAULT_TENANT));

        let team = id(&expect(app, "POST", "/api/v1/support/teams", admin, Some(json!({"name": "CSKH", "org_id": root})), 201).await);
        let team_path = format!("/api/v1/support/teams/{team}");
        let me = get(app, admin, "/api/v1/auth/me").await;
        let member = format!("{team_path}/members/{}", me["data"]["id"].as_str().unwrap());
        expect(app, "PUT", &member, admin, Some(json!({"accepting": true})), 200).await;
        get(app, admin, "/api/v1/support/teams").await;
        get(app, admin, &team_path).await;
        let rule = json!({"org_id": root, "team_id": team, "category": "product"});
        let rule = id(&expect(app, "POST", "/api/v1/support/routing-rules", admin, Some(rule), 201).await);
        get(app, admin, &format!("/api/v1/support/routing-rules?org_id={}", root.as_str().unwrap())).await;

        let macro_body = json!({"name": "Xin lỗi", "org_id": root, "bodies": {"vi": "Shop xin lỗi bạn."}});
        let saved = id(&expect(app, "POST", "/api/v1/support/macros", admin, Some(macro_body), 201).await);
        let replaced = json!({"name": "Xin lỗi", "org_id": root, "bodies": {"vi": "Shop thành thật xin lỗi bạn."}});
        write(app, admin, "PUT", &format!("/api/v1/support/macros/{saved}"), Some(replaced), 200).await;

        // Chính sách riêng của đơn vị: lấy chính sách đang áp dụng, bật phiếu vãng lai, rồi gỡ.
        let policy_path = format!("/api/v1/support/policies/{}", root.as_str().unwrap());
        let mut policy = get(app, admin, &policy_path).await["data"].clone();
        let object = policy.as_object_mut().unwrap();
        for key in ["org_id", "inherited", "source", "updated_at", "version", "defined_at"] {
            object.remove(key);
        }
        policy["guest_tickets"] = json!(true);
        write(app, admin, "PUT", &policy_path, Some(policy), 200).await;

        // Khách vãng lai mở phiếu, xin gặp người, chấm điểm.
// Biểu mẫu công khai cần form token chống spam lấy lúc hiện biểu mẫu.
        let form = expect(app, "GET", "/api/v1/antispam/token?scope=support:tickets", ANONYMOUS, None, 200).await;
        let opened = json!({"category": "product", "subject": "Hàng lỗi", "body": "Áo bị rách.", "name": "An",
                            "email": "an@example.com", "form_token": form["data"]["token"]});
        let opened = expect(app, "POST", "/api/v1/support/public/tickets", ANONYMOUS, Some(opened), 201).await;
        let ticket = id(&opened);
        let token = opened["data"]["access_token"].clone();
        let public = format!("/api/v1/support/public/tickets/{ticket}");
        expect(app, "POST", &format!("{public}/handoff"), ANONYMOUS, Some(json!({"token": token})), 200).await;

        // Nhân viên: ghi chú nội bộ, liên kết, đổi trạng thái bằng tool; thử câu trả lời tự động.
        call_tool(app, admin, "support.add_internal_note", json!({"id": ticket, "body": "Kiểm tra lô hàng"})).await;
        let page = json!({"kind": "page", "title": "Chính sách đổi trả"});
        let page = id(&expect(app, "POST", "/api/v1/content/entries", admin, Some(page), 201).await);
        let link = json!({"id": ticket, "target": format!("content.entry:{page}")});
        call_tool(app, admin, "support.link_ticket", link).await;
        call_tool(app, admin, "support.update_ticket", json!({"id": ticket, "status": "solved", "resolution": "answered"})).await;
        call_tool(app, admin, "support.preview_auto_reply", json!({"org_id": root, "text": "Bao lâu thì giao hàng?"})).await;
        let rating = json!({"rating": 5, "note": "Nhanh", "token": token});
        expect(app, "POST", &format!("{public}/satisfaction"), ANONYMOUS, Some(rating), 200).await;

        write(app, admin, "DELETE", &format!("/api/v1/support/routing-rules/{rule}"), None, 204).await;
        write(app, admin, "DELETE", &format!("/api/v1/support/macros/{saved}"), None, 204).await;
        write(app, admin, "DELETE", &member, None, 204).await;
        write(app, admin, "DELETE", &team_path, None, 204).await;
        write(app, admin, "DELETE", &policy_path, None, 200).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hoi_thoai_go_thu_hoi_bao_xau_va_dong() {
    TestDatabase::run(async |test| {
        let desk = desk(test).await;
        let (app, admin, buyer) = (&desk.real.app, desk.real.admin.as_str(), desk.buyer.as_str());
        let body = json!({"body": "Áo này còn size M không?", "client_id": "c-1"});
        let started =
            expect(app, "POST", "/api/v1/conversations/me/threads", buyer, Some(body), 201).await;
        let thread = started["data"]["thread"]["id"].as_str().unwrap().to_owned();
        let mine = format!("/api/v1/conversations/me/threads/{thread}");
        expect(app, "POST", &format!("{mine}/typing"), buyer, None, 204).await;
        let staff = format!("/api/v1/conversations/threads/{thread}");
        expect(app, "POST", &format!("{staff}/typing"), admin, None, 204).await;
        let reply = json!({"body": "Còn bạn nhé", "client_id": "r-1"});
        let sent = expect(app, "POST", &format!("{staff}/messages"), admin, Some(reply), 201).await;
        expect(app, "POST", &format!("{staff}/messages/{}/recall", id(&sent)), admin, None, 200)
            .await;
        call_tool(app, admin, "conversations.get_thread", json!({"id": thread})).await;
        call_tool(app, admin, "conversations.list_calls", json!({"id": thread})).await;
        expect(
            app,
            "POST",
            &format!("{staff}/reports"),
            admin,
            Some(json!({"reason": "spam"})),
            201,
        )
        .await;

        expect(app, "POST", &format!("{mine}/close"), buyer, None, 200).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dich_vu_nguoi_lam_ngay_nghi_lich_va_dat_lich_bang_tool() {
    TestDatabase::run(async |test| {
        let desk = desk(test).await;
        let (app, admin) = (&desk.real.app, desk.real.admin.as_str());
        let staff = id(&expect(app, "POST", "/api/v1/bookings/resources", admin, Some(json!({"kind": "staff", "name": "Lan"})), 201).await);
        let week = json!({"ranges": (1..=7).map(|day| json!({"weekday": day, "start_minute": 0, "end_minute": 1440})).collect::<Vec<_>>()});
        expect(app, "PUT", &format!("/api/v1/bookings/resources/{staff}/schedule"), admin, Some(week), 200).await;
        get(app, admin, "/api/v1/bookings/resources").await;
        get(app, admin, &format!("/api/v1/bookings/resources/{staff}")).await;
        get(app, admin, &format!("/api/v1/bookings/resources/{staff}/schedule")).await;
        let service = json!({"name": "Tư vấn", "duration_minutes": 30, "price": {"amount": "0", "currency": "VND"}, "performers": [staff]});
        let service = id(&expect(app, "POST", "/api/v1/bookings/services", admin, Some(service), 201).await);
        get(app, admin, "/api/v1/bookings/services").await;
        get(app, admin, &format!("/api/v1/bookings/services/{service}")).await;

        let day = (autoxide_kernel::time::Timestamp::now() + jiff::SignedDuration::from_hours(24 * 3)).date_utc();
        let off_start = day.start_of_day_utc();
        let off = json!({"resource_id": staff, "kind": "closed", "reason": "Nghỉ phép",
                         "starts_at": off_start, "ends_at": off_start + jiff::SignedDuration::from_hours(4)});
        expect(app, "POST", "/api/v1/bookings/exceptions", admin, Some(off), 201).await;
        get(app, admin, &format!("/api/v1/bookings/exceptions?resource_id={staff}")).await;

        let starts = off_start + jiff::SignedDuration::from_hours(6);
        let booking = json!({"service_id": service, "starts_at": starts, "customer_name": "Bình", "customer_phone": "0912345678"});
        let booked = call_tool(app, admin, "bookings.create_booking", booking).await;
        let booking = booked["booking"]["id"].as_str().unwrap_or_else(|| panic!("{booked}")).to_owned();
        let ics = app.get(&format!("/api/v1/bookings/bookings/{booking}/calendar.ics")).token(admin).send().await.unwrap();
        assert_eq!(ics.status(), 200, "{}", ics.text());
        assert!(ics.text().contains("BEGIN:VCALENDAR"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn luat_tu_dong_buoc_rui_ro_cho_duyet_tu_choi_va_chay_lai() {
    TestDatabase::run(async |test| {
        let desk = desk(test).await;
        let (app, admin) = (&desk.real.app, desk.real.admin.as_str());
        // Bước rủi ro: xoá menu (tool phá huỷ) chờ một người duyệt.
        let rule = json!({"name": "Dọn menu", "trigger": {"type": "manual"}, "enabled": true,
            "actions": [{"tool": "menus.delete_menu", "arguments": {"id": "{{ event.menu_id }}"}}]});
        let rule = call_tool(app, admin, "automations.create_rule", rule).await;
        let rule = rule["id"].as_str().unwrap_or_else(|| panic!("{rule}")).to_owned();
        call_tool(app, admin, "automations.get_rule", json!({"id": rule})).await;
        call_tool(app, admin, "automations.list_rules", json!({})).await;
        call_tool(app, admin, "automations.update_rule", json!({"id": rule, "description": "Xoá menu cũ"})).await;
        call_tool(app, admin, "automations.test_rule", json!({"id": rule, "event_sample": {"menu_id": Uuid::now_v7()}})).await;
        let path = format!("/api/v1/automations/rules/{rule}");
        get(app, admin, "/api/v1/automations/rules").await;
        get(app, admin, &path).await;
        write(app, admin, "PATCH", &path, Some(json!({"name": "Dọn menu cũ"})), 200).await;
        write(app, admin, "POST", &format!("{path}/disable"), None, 200).await;
        write(app, admin, "POST", &format!("{path}/enable"), None, 200).await;

        let menu = |key: &str| json!({"key": key, "locale": "vi", "items": [{"label": "Trang chủ", "url": "/"}]});
        let mut runs = Vec::new();
        for key in ["cu-1", "cu-2"] {
            let menu = id(&expect(app, "POST", "/api/v1/menus", admin, Some(menu(key)), 201).await);
            let run = expect(app, "POST", &format!("{path}/run"), admin, Some(json!({"input": {"menu_id": menu}})), 202).await;
            runs.push(id(&run));
        }
        drain(app).await;
        expect(app, "POST", &format!("/api/v1/automations/runs/{}/approve", runs[0]), admin, None, 202).await;
        expect(app, "POST", &format!("/api/v1/automations/runs/{}/reject", runs[1]), admin, Some(json!({"reason": "Còn dùng"})), 200).await;

        // Lần chạy hỏng (menu không có): chạy lại từ bước hỏng.
        let run = expect(app, "POST", &format!("{path}/run"), admin, Some(json!({"input": {"menu_id": Uuid::now_v7()}})), 202).await;
        drain(app).await;
        let run = id(&run);
        expect(app, "POST", &format!("/api/v1/automations/runs/{run}/approve"), admin, None, 202).await;
        drain(app).await;
        expect(app, "POST", &format!("/api/v1/automations/runs/{run}/retry"), admin, None, 202).await;
        write(app, admin, "DELETE", &path, None, 204).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn phan_tich_nguon_pheu_va_tool() {
    TestDatabase::run(async |test| {
        let desk = desk(test).await;
        let (app, admin) = (&desk.real.app, desk.real.admin.as_str());
        expect(app, "GET", "/api/v1/analytics/event-types", ANONYMOUS, None, 200).await;
        get(app, admin, "/api/v1/analytics/sources").await;
        let funnels = get(app, admin, "/api/v1/analytics/funnels").await;
        call_tool(app, admin, "analytics.overview", json!({})).await;
        call_tool(app, admin, "analytics.top_objects", json!({})).await;
        if let Some(key) = funnels["data"][0]["key"].as_str() {
            call_tool(app, admin, "analytics.funnel", json!({"key": key})).await;
        }
    })
    .await
    .unwrap();
}
