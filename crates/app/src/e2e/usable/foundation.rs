//! Nền dùng được như người dùng gọi: địa chỉ hành chính, trạng thái AI, thuế, báo cáo (cả SQLite và
//! Postgres), đơn vị tổ chức, file — mỗi endpoint và tool MCP được gọi thành công ít nhất một lần.

use autoxide_kernel::tenancy::{DEFAULT_TENANT, root_of};
use autoxide_testkit::TestDatabase;
use serde_json::json;

use super::kit::{ADMIN, app, app_with, call_tool, expect, get, import_reference_data};

#[tokio::test]
async fn dia_chi_hanh_chinh_tra_va_kiem_duoc() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        import_reference_data(&app).await;
        let communes = get(&app, "/api/v1/address/provinces/01/communes").await;
        assert!(communes["data"].as_array().unwrap().iter().any(|unit| unit["code"] == "00004"));
        let found = get(&app, "/api/v1/address/units?q=ba%20dinh").await;
        assert!(found["data"].as_array().unwrap().iter().any(|unit| unit["code"] == "00004"), "{found}");
        let unit = get(&app, "/api/v1/address/units/00004").await;
        assert_eq!(unit["data"]["name"], "Phường Ba Đình");
        let body = json!({"country": "VN", "province_code": "01", "commune_code": "00004", "line": "1 Phố Hoè"});
        let checked = expect(&app, "POST", "/api/v1/address/validate", ADMIN, Some(body.clone()), 200).await;
        assert_eq!(checked["data"]["commune_name"], "Phường Ba Đình", "{checked}");

        call_tool(&app, ADMIN, "address.list_provinces", json!({})).await;
        call_tool(&app, ADMIN, "address.list_communes", json!({"province": "01"})).await;
        call_tool(&app, ADMIN, "address.search_units", json!({"q": "ha noi"})).await;
        call_tool(&app, ADMIN, "address.validate_address", body).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn trang_thai_ai_thue_va_bao_cao_xem_duoc() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        import_reference_data(&app).await;
        get(&app, "/api/v1/ai/status").await;
        call_tool(&app, ADMIN, "ai.status", json!({})).await;

        let categories = get(&app, "/api/v1/tax/categories").await;
        assert!(!categories["data"].as_array().unwrap().is_empty(), "{categories}");
        get(&app, "/api/v1/tax/jurisdictions").await;
        let rates = get(&app, "/api/v1/tax/rates").await;
        assert!(!rates["data"].as_array().unwrap().is_empty(), "{rates}");
        get(&app, "/api/v1/tax/rules").await;
        get(&app, "/api/v1/tax/withholding-rates").await;
        call_tool(&app, ADMIN, "tax.rates", json!({"jurisdiction": "VN", "on": "2026-06-01"})).await;
        let quote = json!({
            "jurisdiction": "VN", "tax_point": "2026-06-01", "price_mode": "exclusive", "currency": "VND",
            "lines": [{"line_id": "a", "category": "vat.standard_reducible", "amount": "1000000"}],
        });
        call_tool(&app, ADMIN, "tax.quote", quote).await;

        get(&app, "/api/v1/reporting/kinds").await;
        get(&app, "/api/v1/reporting/lag").await;
        expect(&app, "POST", "/api/v1/reporting/rebuild", ADMIN, Some(json!({})), 200).await;
        call_tool(&app, ADMIN, "reporting.kinds", json!({})).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn don_vi_to_chuc_tao_doi_ten_chuyen_va_tam_dung_bang_tool() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let root = json!(root_of(DEFAULT_TENANT));
        let kind = json!({"code": "kiosk", "name": "Kiosk"});
        expect(&app, "POST", "/api/v1/organizations/unit-types", ADMIN, Some(kind), 201).await;
        let first = call_tool(
            &app,
            ADMIN,
            "organizations.create_unit",
            json!({"name": "Kiosk 1", "parent_id": root, "unit_type": "kiosk"}),
        )
        .await;
        let first = first["id"].as_str().unwrap_or_else(|| panic!("{first}")).to_owned();
        let second = call_tool(
            &app,
            ADMIN,
            "organizations.create_unit",
            json!({"name": "Kiosk 2", "parent_id": root, "unit_type": "kiosk"}),
        )
        .await;
        let second = second["id"].as_str().unwrap().to_owned();
        call_tool(&app, ADMIN, "organizations.get_unit", json!({"id": first})).await;
        call_tool(
            &app,
            ADMIN,
            "organizations.update_unit",
            json!({"id": first, "name": "Kiosk One"}),
        )
        .await;
        call_tool(
            &app,
            ADMIN,
            "organizations.move_unit",
            json!({"id": second, "parent_id": first}),
        )
        .await;
        call_tool(
            &app,
            ADMIN,
            "organizations.change_unit_status",
            json!({"id": second, "status": "suspended", "reason": "Stock count"}),
        )
        .await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn file_nho_tai_len_doc_va_xoa_bang_tool() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let body = json!({"name": "hello.txt", "content_type": "text/plain", "content_base64": "aGVsbG8="});
        let file = call_tool(&app, ADMIN, "files.upload_file", body).await;
        let file_id = file["id"].as_str().unwrap_or_else(|| panic!("{file}")).to_owned();
        let got = get(&app, &format!("/api/v1/files/{file_id}")).await;
        assert_eq!(got["data"]["size"], 5);
        call_tool(&app, ADMIN, "files.get_file", json!({"id": file_id})).await;
        let read = call_tool(&app, ADMIN, "files.read_file", json!({"id": file_id})).await;
        assert!(read.to_string().contains("aGVsbG8="), "{read}");
        call_tool(&app, ADMIN, "files.delete_file", json!({"id": file_id})).await;
    })
    .await
    .unwrap();
}

/// Tải thẳng lên S3 bằng REST và bằng tool (không qua máy chủ). Chỉ chạy khi có S3 (gate dựng một máy
/// chủ S3 giả lập), như `e2e::media::tai_thang_media_len_s3`.
#[tokio::test]
async fn file_lon_tai_thang_len_s3_bang_rest_va_tool() {
    use sha2::{Digest as _, Sha256};
    let Some(s3) = autoxide_testkit::s3_settings() else {
        return;
    };
    TestDatabase::run(async |test| {
        let app = app_with(test, |mut builder| {
            for (name, value) in &s3 {
                builder = builder.setting(name, value);
            }
            builder
        })
        .await;
        autoxide_kernel::tls::install_crypto_provider();
        for (n, via_tool) in [(1, false), (2, true)] {
            let content = format!("report number {n}").into_bytes();
            let sha256: String =
                Sha256::digest(&content).iter().map(|b| format!("{b:02x}")).collect();
            let start = json!({"name": format!("report-{n}.txt"), "content_type": "text/plain",
                               "size": content.len(), "sha256": sha256});
            let link = if via_tool {
                call_tool(&app, ADMIN, "files.create_upload", start).await
            } else {
                expect(&app, "POST", "/api/v1/files/uploads", ADMIN, Some(start), 201).await["data"]
                    .clone()
            };
            let mut put = reqwest::Client::new().put(link["url"].as_str().unwrap()).body(content);
            for (name, value) in link["headers"].as_object().unwrap() {
                put = put.header(name.as_str(), value.as_str().unwrap());
            }
            assert!(put.send().await.unwrap().status().is_success());
            let upload = link["id"].as_str().unwrap();
            if via_tool {
                call_tool(&app, ADMIN, "files.confirm_upload", json!({"id": upload})).await;
            } else {
                expect(
                    &app,
                    "POST",
                    &format!("/api/v1/files/uploads/{upload}/complete"),
                    ADMIN,
                    None,
                    201,
                )
                .await;
            }
        }
    })
    .await
    .unwrap();
}
