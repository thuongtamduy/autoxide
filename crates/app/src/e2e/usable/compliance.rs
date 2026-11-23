//! Tuân thủ pháp luật dùng được như người phụ trách pháp lý của cửa hàng thật (cả SQLite và Postgres):
//! văn bản pháp lý và phiên bản, website khai với Bộ Công Thương và trang bắt buộc, thẻ theo dõi, vụ việc
//! (khiếu nại, vi phạm) và nhật ký, sự cố dữ liệu cá nhân, yêu cầu của chủ thể dữ liệu, hồ sơ định danh
//! người bán (KYC) xét tay bằng giấy tờ tải lên; các tool tra cứu và ghi của module.

use autoxide_kernel::tenancy::{DEFAULT_TENANT, root_of};
use autoxide_testkit::{TestApp, TestDatabase};
use serde_json::{Value, json};

use super::kit::{ADMIN, app, call_tool, expect, get, id};

async fn write(
    app: &TestApp,
    method: &str,
    path: &str,
    body: Option<Value>,
    expected: u16,
) -> Value {
    let method = axum::http::Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method.clone(), path).token(ADMIN).header("if-match", "*");
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let json: Value = serde_json::from_slice(answer.body()).unwrap_or(Value::Null);
    assert_eq!(answer.status().as_u16(), expected, "{method} {path}: {json}");
    json
}

async fn upload(app: &TestApp, name: &str) -> String {
    let answer = app
        .post(&format!("/api/v1/files?name={name}"))
        .token(ADMIN)
        .header("content-type", "application/pdf")
        .bytes(b"%PDF-1.4 giay to".to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(answer.status(), 201, "{}", answer.text());
    answer.json()["data"]["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn van_ban_phap_ly_website_trang_bat_buoc_va_the_theo_doi() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let root = json!(root_of(DEFAULT_TENANT));
        let document = json!({"kind": "terms_of_use", "title": "Điều khoản sử dụng"});
        let document = id(&expect(&app, "POST", "/api/v1/compliance/documents", ADMIN, Some(document), 201).await);
        get(&app, "/api/v1/compliance/documents").await;
        get(&app, &format!("/api/v1/compliance/documents/{document}")).await;
        call_tool(&app, ADMIN, "compliance.documents", json!({})).await;
        let version = json!({"document_id": document, "locale": "vi", "title": "Điều khoản sử dụng",
                             "body": "Nội dung điều khoản.", "requires_reacceptance": false});
        call_tool(&app, ADMIN, "compliance.publish_document_version", version).await;
        call_tool(&app, ADMIN, "compliance.pending_acceptances", json!({})).await;
        call_tool(&app, ADMIN, "compliance.account_verification", json!({})).await;
        call_tool(&app, ADMIN, "compliance.verification_status", json!({})).await;

        let legal = json!({"entity_type": "organization", "legal_name": "Công ty TNHH An", "tax_code": "0101234567"});
        expect(&app, "POST", "/api/v1/compliance/legal-profiles", ADMIN, Some(legal), 201).await;
        call_tool(&app, ADMIN, "compliance.legal_profiles", json!({})).await;

        let website = json!({"domain": "shop.example", "kind": "sales", "org_id": root});
        let website = id(&expect(&app, "POST", "/api/v1/compliance/websites", ADMIN, Some(website), 201).await);
        let path = format!("/api/v1/compliance/websites/{website}");
        get(&app, "/api/v1/compliance/websites").await;
        write(&app, "PATCH", &path, Some(json!({"domain": "www.shop.example"})), 200).await;
        let page = json!({"kind": "page", "title": "Chính sách bảo mật"});
        let page = id(&expect(&app, "POST", "/api/v1/content/entries", ADMIN, Some(page), 201).await);
        write(&app, "PUT", &format!("{path}/pages/privacy_policy"), Some(json!({"entry_id": page})), 200).await;
        get(&app, &path).await;
        write(&app, "DELETE", &format!("{path}/pages/privacy_policy"), None, 204).await;
        call_tool(&app, ADMIN, "compliance.websites", json!({})).await;
        call_tool(&app, ADMIN, "compliance.website_dossier", json!({"website_id": website})).await;
        call_tool(&app, ADMIN, "compliance.set_website_status", json!({"website_id": website, "status": "submitted", "reference": "HS-1"})).await;

        let tag = json!({"provider": "ga4", "tracking_id": "G-ABC123", "org_id": root});
        let tag = id(&expect(&app, "POST", "/api/v1/compliance/tracking-tags", ADMIN, Some(tag), 201).await);
        get(&app, &format!("/api/v1/compliance/tracking-tags/{tag}")).await;
        call_tool(&app, ADMIN, "compliance.tracking_tags", json!({})).await;
        write(&app, "DELETE", &format!("/api/v1/compliance/tracking-tags/{tag}"), None, 204).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn vu_viec_su_co_du_lieu_va_yeu_cau_cua_chu_the() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let page = json!({"kind": "page", "title": "Hàng nhái"});
        let page = id(&expect(&app, "POST", "/api/v1/content/entries", ADMIN, Some(page), 201).await);
        let case = json!({"kind": "ip_infringement", "reporter_kind": "rights_holder", "claim": "Trang dùng ảnh của chúng tôi.",
                          "name": "Công ty B", "email": "phaply@b.example", "target": format!("content.entry:{page}")});
        let case = id(&expect(&app, "POST", "/api/v1/compliance/cases", ADMIN, Some(case), 201).await);
        let entry = json!({"body": "Đã liên hệ người đăng.", "kind": "note"});
        expect(&app, "POST", &format!("/api/v1/compliance/cases/{case}/entries"), ADMIN, Some(entry), 201).await;
        call_tool(&app, ADMIN, "compliance.list_cases", json!({})).await;
        call_tool(&app, ADMIN, "compliance.get_case", json!({"case_id": case})).await;
        call_tool(&app, ADMIN, "compliance.add_case_note", json!({"case_id": case, "body": "Chờ phản hồi."})).await;
        call_tool(&app, ADMIN, "compliance.restrict_case_target", json!({"case_id": case, "reason": "Đang xác minh"})).await;

        let incident = json!({"title": "Lộ bản sao database", "severity": "high", "data_categories": ["contact"]});
        let incident = call_tool(&app, ADMIN, "compliance.open_incident", incident).await;
        let incident = incident["id"].as_str().unwrap_or_else(|| panic!("{incident}")).to_owned();
        get(&app, "/api/v1/compliance/incidents").await;
        call_tool(&app, ADMIN, "compliance.incidents", json!({})).await;
        call_tool(&app, ADMIN, "compliance.add_incident_entry", json!({"incident_id": incident, "body": "Đã đổi mật khẩu."})).await;
        call_tool(&app, ADMIN, "compliance.incident_draft", json!({"incident_id": incident, "audience": "authority"})).await;

        let request = json!({"channel": "email", "kind": "access", "subject": "khach@example.com"});
        let request = call_tool(&app, ADMIN, "compliance.record_subject_request", request).await;
        let request = request["id"].as_str().unwrap_or_else(|| panic!("{request}")).to_owned();
        let path = format!("/api/v1/compliance/subject-requests/{request}");
        get(&app, &path).await;
        write(&app, "PATCH", &path, Some(json!({"notes": "Đã gửi bản sao dữ liệu."})), 200).await;
        call_tool(&app, ADMIN, "compliance.subject_requests", json!({})).await;
        call_tool(&app, ADMIN, "compliance.purposes", json!({})).await;
        call_tool(&app, ADMIN, "compliance.processing_register", json!({})).await;
        call_tool(&app, ADMIN, "compliance.retention", json!({})).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ho_so_dinh_danh_nguoi_ban_xet_tay_bang_giay_to() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let root = json!(root_of(DEFAULT_TENANT));
        let profile = json!({"org_id": root, "party_type": "organization"});
        let profile =
            id(&expect(&app, "POST", "/api/v1/compliance/kyc/profiles", ADMIN, Some(profile), 201)
                .await);
        let path = format!("/api/v1/compliance/kyc/profiles/{profile}");
        let file = upload(&app, "giay-phep.pdf").await;
        expect(
            &app,
            "POST",
            &format!("{path}/documents"),
            ADMIN,
            Some(json!({"file_ids": [file]})),
            200,
        )
        .await;
        let ask =
            json!({"fields": ["registration_number"], "message": "Bổ sung số đăng ký kinh doanh."});
        expect(&app, "POST", &format!("{path}/request-info"), ADMIN, Some(ask), 200).await;
        call_tool(&app, ADMIN, "compliance.kyc_profiles", json!({})).await;
        // Hồ sơ của chính mình (người bán cá nhân): mở rồi gửi giấy tờ.
        expect(&app, "POST", "/api/v1/compliance/me/kyc", ADMIN, Some(json!({})), 200).await;
        let mine = upload(&app, "cccd.pdf").await;
        expect(
            &app,
            "POST",
            "/api/v1/compliance/me/kyc/documents",
            ADMIN,
            Some(json!({"file_ids": [mine]})),
            200,
        )
        .await;
    })
    .await
    .unwrap();
}
