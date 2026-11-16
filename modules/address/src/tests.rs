//! Test của `address` trên database gate đang chọn (cả SQLite và Postgres), với một bộ dữ liệu
//! NHỎ viết tay chỉ để test (không phải danh mục thật).

use std::sync::Arc;

use autoxide_kernel::pagination::{Order, PageRequest};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;

use crate::api::AddressInput;
use crate::import::{Manifest, parse};
use crate::service::AddressService;

const FIRST: &str = "level,code,name,kind,parent_code
1,01,Thành phố Hà Nội,city,
1,79,Thành phố Hồ Chí Minh,city,
2,00004,Phường Ba Đình,ward,01
2,00008,Xã Đông Anh,commune,01
2,26734,Phường Sài Gòn,ward,79
";

/// Bộ sau: Đông Anh đổi tên, Sài Gòn bỏ, thêm Côn Đảo.
const SECOND: &str = "level,code,name,kind,parent_code
1,01,Thành phố Hà Nội,city,
1,79,Thành phố Hồ Chí Minh,city,
2,00004,Phường Ba Đình,ward,01
2,00008,Xã Đông Anh Mới,commune,01
2,26999,Đặc khu Côn Đảo,special_zone,79
";

fn manifest(from: &str) -> Manifest {
    Manifest {
        country: "VN".to_owned(),
        title: "Test list".to_owned(),
        source_url: "https://example.com/list".to_owned(),
        retrieved_on: "2025-06-30".to_owned(),
        effective_from: from.to_owned(),
    }
}

async fn app(test: &TestDatabase) -> (TestApp, Arc<AddressService>) {
    let app = TestApp::builder(test).module(crate::module()).build().await.unwrap();
    let service = Arc::new(AddressService {
        databases: app.modules().tenancy().unwrap(),
        outbox: autoxide_kernel::event::Outbox::standalone("address"),
    });
    (app, service)
}

async fn import(service: &AddressService, csv: &str, from: &str) -> crate::import::Imported {
    let rows = parse(csv).unwrap();
    service.import(&manifest(from), &rows, &autoxide_kernel::secret::sha256(csv)).await.unwrap()
}

#[tokio::test]
async fn tim_khong_dau_ra_co_dau() {
    TestDatabase::run(async |test| {
        let (_, service) = app(test).await;
        import(&service, FIRST, "2025-07-01").await;
        let page = PageRequest::first(10, Order::Asc);
        let found = service.search("VN", Some("ha noi"), None, None, &page).await.unwrap();
        assert_eq!(found.data.iter().map(|unit| unit.code.as_str()).collect::<Vec<_>>(), ["01"]);
        let found =
            service.search("vn", Some("Đông Anh"), Some("01"), Some(2), &page).await.unwrap();
        assert_eq!(found.data[0].code, "00008");
        let provinces = service.provinces("VN", None).await.unwrap();
        assert_eq!(
            provinces.iter().map(|unit| unit.code.as_str()).collect::<Vec<_>>(),
            ["01", "79"]
        );
        let communes = service.communes("VN", "01", None).await.unwrap();
        assert_eq!(communes.len(), 2);
        assert_eq!(
            service.communes("VN", "99", None).await.unwrap_err().code(),
            "address.unit_not_found"
        );
        assert_eq!(
            service.provinces("US", None).await.unwrap_err().code(),
            "address.country_unsupported"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nhap_lai_cung_bo_khong_doi_gi() {
    TestDatabase::run(async |test| {
        let (_, service) = app(test).await;
        let first = import(&service, FIRST, "2025-07-01").await;
        assert!(first.changed);
        assert_eq!((first.opened, first.closed, first.provinces, first.communes), (5, 0, 2, 3));
        let again = import(&service, FIRST, "2025-07-01").await;
        assert!(!again.changed);
        assert_eq!(again.dataset_id, first.dataset_id);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bo_moi_dong_don_vi_cu_va_mo_don_vi_moi() {
    TestDatabase::run(async |test| {
        let (_, service) = app(test).await;
        import(&service, FIRST, "2025-07-01").await;
        let second = import(&service, SECOND, "2026-01-01").await;
        // Đông Anh đổi tên (đóng + mở), Sài Gòn bỏ (đóng), Côn Đảo mới (mở).
        assert_eq!((second.opened, second.closed), (2, 2));
        let before = service.communes("VN", "79", Some("2025-12-31")).await.unwrap();
        assert_eq!(before.iter().map(|unit| unit.code.as_str()).collect::<Vec<_>>(), ["26734"]);
        let after = service.communes("VN", "79", Some("2026-01-01")).await.unwrap();
        assert_eq!(after.iter().map(|unit| unit.code.as_str()).collect::<Vec<_>>(), ["26999"]);
        let renamed = service.communes("VN", "01", Some("2026-02-01")).await.unwrap();
        assert!(renamed.iter().any(|unit| unit.name == "Xã Đông Anh Mới"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn xa_nao_cung_thuoc_mot_tinh_co_that() {
    let orphan = "level,code,name,kind,parent_code\n1,01,A,city,\n2,00001,B,ward,02\n";
    let error = parse(orphan).unwrap_err();
    assert!(format!("{error:?}").contains("names province 02"), "{error:?}");
    let twice = "level,code,name,kind,parent_code\n1,01,A,city,\n1,01,A,city,\n";
    assert!(parse(twice).is_err());
    assert!(parse("code,name\n").is_err(), "sai dòng đầu");
}

#[tokio::test]
async fn validate_bao_xa_khong_thuoc_tinh() {
    TestDatabase::run(async |test| {
        let (app, service) = app(test).await;
        import(&service, FIRST, "2025-07-01").await;
        let good = service.check(AddressInput::new("VN", "01", "00004", " 12 Trang Tien "), "address.").await.unwrap();
        assert_eq!((good.province_name.as_str(), good.commune_name.as_str(), good.line.as_str()), ("Thành phố Hà Nội", "Phường Ba Đình", "12 Trang Tien"));
        let wrong = service.check(AddressInput::new("VN", "01", "26734", "x"), "address.").await.unwrap_err();
        assert_eq!((wrong[0].field.as_str(), wrong[0].code.as_str()), ("address.commune_code", "address.commune_not_in_province"));
        // Qua REST: 422 cùng lỗi.
        let body = serde_json::json!({"country": "VN", "province_code": "79", "commune_code": "00004", "line": "x"});
        let answer = app.post("/api/v1/address/validate").json(body).send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(answer.json()["errors"][0]["code"], "address.commune_not_in_province");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn etag_tra_304() {
    TestDatabase::run(async |test| {
        let (app, service) = app(test).await;
        import(&service, FIRST, "2025-07-01").await;
        let first = app.get("/api/v1/address/provinces").send().await.unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        let etag = first.headers().get("etag").unwrap().to_str().unwrap().to_owned();
        assert_eq!(first.headers().get("cache-control").unwrap(), "public, max-age=86400");
        let again = app
            .get("/api/v1/address/provinces")
            .header("if-none-match", &etag)
            .send()
            .await
            .unwrap();
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED);
        // Bộ mới: ETag đổi.
        import(&service, SECOND, "2026-01-01").await;
        let changed = app
            .get("/api/v1/address/provinces")
            .header("if-none-match", &etag)
            .send()
            .await
            .unwrap();
        assert_eq!(changed.status(), StatusCode::OK);
    })
    .await
    .unwrap();
}

/// Bộ đóng gói (D-318): đủ 34 tỉnh / thành (28 tỉnh, 6 thành phố) và 3.321 đơn vị cấp xã (2.621 xã,
/// 687 phường, 13 đặc khu) theo QĐ 19/2025/QĐ-TTg; nhập được, xã nào cũng thuộc một tỉnh có thật.
#[tokio::test]
async fn du_lieu_goc_du_34_tinh_3321_xa() {
    let rows = parse(crate::import::BUNDLED_UNITS).unwrap();
    let count = |kind: &str| rows.iter().filter(|row| row.kind == kind).count();
    assert_eq!((count("province"), count("city")), (28, 6));
    assert_eq!((count("commune"), count("ward"), count("special_zone")), (2621, 687, 13));
    TestDatabase::run(async |test| {
        let (_, service) = app(test).await;
        let (manifest, sha) = crate::import::bundled().unwrap();
        let imported = service.import(&manifest, &rows, &sha).await.unwrap();
        assert_eq!((imported.provinces, imported.communes), (34, 3321));
        assert_eq!(service.provinces("VN", None).await.unwrap().len(), 34);
        let hanoi = service.communes("VN", "01", None).await.unwrap();
        assert!(hanoi.iter().any(|unit| unit.name == "Phường Ba Đình"));
    })
    .await
    .unwrap();
}
