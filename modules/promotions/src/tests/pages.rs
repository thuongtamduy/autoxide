//! Danh sách có phân trang (bước 70 P9 mục 3, D-20261125-list-pages): số liệu theo ngày và lịch sử phiên bản
//! (keyset trong database), hạn mức luật (`page_of`); theo `next_cursor` tới hết không trùng, không sót, cả hai
//! chiều; lỗi khoảng ngày và lỗi tham số trang báo một lần.

use autoxide_kernel::time::{Date, Timestamp};
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use sea_orm::{ActiveModelTrait, Set};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Setup, body, call, load_legal, running, setup};
use crate::entity::{stats, version};

/// Theo `next_cursor` của `uri` (đã có `limit`) tới hết; trả giá trị trường `field` của mọi dòng theo thứ tự.
async fn walk(setup: &Setup, uri: &str, field: &str) -> Vec<Value> {
    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page_uri = match &cursor {
            Some(cursor) => format!("{uri}&cursor={cursor}"),
            None => uri.to_owned(),
        };
        let (status, page) = call(&setup.app, "GET", &page_uri, "boss", None).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        seen.extend(page["data"].as_array().unwrap().iter().map(|row| row[field].clone()));
        match page["pagination"]["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => return seen,
        }
    }
}

#[tokio::test]
async fn so_lieu_va_phien_ban_theo_trang_ca_hai_chieu() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let id = running(&setup, body(setup.tree.a, json!({}))).await;
        let tenant = setup.tenant();
        let write = tenant.database().write();
        let first = Date::new(2026, 3, 1).unwrap();
        let days: Vec<Date> =
            (0..5).map(|offset| first.checked_add_days(offset).unwrap()).collect();
        for day in &days {
            stats::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                promotion_id: Set(id),
                day: Set(day.to_string()),
                redemptions: Set(1),
                discount_minor: Set(1_000),
                order_value_minor: Set(10_000),
                platform_funded_minor: Set(0),
                seller_funded_minor: Set(0),
            }
            .insert(write)
            .await
            .unwrap();
        }
        // Đã có phiên bản 1 (`activated`); thêm 2…5.
        for number in 2..=5 {
            version::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                promotion_id: Set(id),
                version: Set(number),
                action: Set("updated".to_owned()),
                snapshot: Set("{}".to_owned()),
                changed_by: Set(None),
                changed_at: Set(Timestamp::now()),
            }
            .insert(write)
            .await
            .unwrap();
        }
        let range = format!("/promotions/{id}/stats?from={}&to={}", days[0], days[4]);
        let expected: Vec<Value> = days.iter().map(|day| json!(day.to_string())).collect();
        assert_eq!(walk(&setup, &format!("{range}&limit=2"), "day").await, expected);
        let mut reversed = expected.clone();
        reversed.reverse();
        assert_eq!(walk(&setup, &format!("{range}&limit=2&order=desc"), "day").await, reversed);

        let versions = format!("/promotions/{id}/versions?limit=2");
        assert_eq!(
            walk(&setup, &versions, "version").await,
            (1..=5).map(|n| json!(n)).collect::<Vec<_>>()
        );
        let (_, counted) =
            call(&setup.app, "GET", &format!("{versions}&count=true"), "boss", None).await;
        assert_eq!(counted["pagination"]["total"], 5);

        // Khoảng ngày ngược và `limit` sai: một lỗi 422 nêu cả hai ô.
        let (status, error) = call(
            &setup.app,
            "GET",
            &format!("/promotions/{id}/stats?from={}&to={}&limit=0", days[4], days[0]),
            "boss",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
        let fields: Vec<&str> = error["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["field"].as_str().unwrap())
            .collect();
        assert!(fields.contains(&"from") && fields.contains(&"limit"), "{error}");
    })
    .await
    .unwrap();
}

/// Hạn mức luật (dữ liệu dùng chung, `page_of`): từng trang một dòng ra đúng danh sách của một trang lớn.
#[tokio::test]
async fn han_muc_luat_theo_trang() {
    TestDatabase::run(async |test| {
        let setup = setup(test, false).await;
        load_legal(&setup.app).await;
        let key = |row: &Value| {
            format!("{}|{}|{}", row["promotion_kind"], row["program_kind"], row["jurisdiction"])
        };
        let (status, all) = call(&setup.app, "GET", "/legal-limits?limit=200", "boss", None).await;
        assert_eq!(status, StatusCode::OK, "{all}");
        let all: Vec<String> = all["data"].as_array().unwrap().iter().map(key).collect();
        assert!(all.len() > 1, "{all:?}");
        let mut walked = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let uri = match &cursor {
                Some(cursor) => format!("/legal-limits?limit=1&cursor={cursor}"),
                None => "/legal-limits?limit=1".to_owned(),
            };
            let (_, page) = call(&setup.app, "GET", &uri, "boss", None).await;
            walked.extend(page["data"].as_array().unwrap().iter().map(key));
            match page["pagination"]["next_cursor"].as_str() {
                Some(next) => cursor = Some(next.to_owned()),
                None => break,
            }
        }
        assert_eq!(walked, all);
        let (_, rules) =
            call(&setup.app, "GET", "/notice-rules?limit=1&count=true", "boss", None).await;
        assert_eq!(rules["data"].as_array().unwrap().len(), 1, "{rules}");
        assert!(rules["pagination"]["total"].as_u64().unwrap() >= 1);
    })
    .await
    .unwrap();
}
