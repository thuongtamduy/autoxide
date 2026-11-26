//! Hạn mức trường tự định nghĩa theo gói (bước 51 P19): tạo trường giữ một suất `kernel.custom_fields`,
//! hết suất thì từ chối với lỗi của gói, xoá (tắt) trường thì trả suất.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use autoxide_kernel::access::Permission;
use autoxide_kernel::custom_fields::CustomFieldTarget;
use autoxide_kernel::entitlement::{Entitlements, Limit, TenantEntitlements};
use autoxide_kernel::error::Error;
use autoxide_kernel::module::Module;
use autoxide_kernel::tenancy::TenantId;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::json;

const SHOP_READ: Permission = Permission::new("shop.read", "See the shop.");
const SHOP_WRITE: Permission = Permission::new("shop.write", "Change the shop.");
const VAULT_READ: Permission = Permission::new("vault.read", "See the vault.");
const VAULT_WRITE: Permission = Permission::new("vault.write", "Change the vault.");

/// Module thứ hai có loại bản ghi nhận trường, đọc bằng permission khác.
struct Vault;

impl Module for Vault {
    fn name(&self) -> &'static str {
        "vault"
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![VAULT_READ, VAULT_WRITE]
    }

    fn custom_field_targets(&self) -> Vec<CustomFieldTarget> {
        vec![CustomFieldTarget::new("vault.box", VAULT_READ, VAULT_WRITE)]
    }
}

struct Shop;

impl Module for Shop {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![SHOP_READ, SHOP_WRITE]
    }

    fn custom_field_targets(&self) -> Vec<CustomFieldTarget> {
        vec![
            CustomFieldTarget::new("shop.item", SHOP_READ, SHOP_WRITE),
            // Loại có loại con `shop.kind.<khoá>` do khách tạo.
            CustomFieldTarget::new("shop.kind", SHOP_READ, SHOP_WRITE).with_subtypes(),
        ]
    }
}

/// Gói cho đúng một trường: giữ danh sách suất đang dùng.
#[derive(Default)]
struct OneField(Mutex<Vec<String>>);

#[async_trait::async_trait]
impl Entitlements for OneField {
    async fn snapshot(&self, _: TenantId) -> Result<Arc<TenantEntitlements>, Error> {
        Ok(Arc::new(TenantEntitlements {
            modules: ["shop".to_owned()].into(),
            ..TenantEntitlements::default()
        }))
    }

    async fn reserve(
        &self,
        _: TenantId,
        limit: &Limit,
        _: i64,
        reference: &str,
    ) -> Result<(), Error> {
        assert_eq!(limit.key(), "kernel.custom_fields");
        let mut used = self.0.lock().unwrap();
        if !used.is_empty() {
            return Err(Error::conflict("plans.limit_reached", "The plan allows one field."));
        }
        used.push(reference.to_owned());
        Ok(())
    }

    async fn confirm(&self, _: TenantId, _: &Limit, _: &str) -> Result<(), Error> {
        Ok(())
    }

    async fn release(&self, _: TenantId, _: &Limit, reference: &str) -> Result<(), Error> {
        self.0.lock().unwrap().retain(|known| known != reference);
        Ok(())
    }

    async fn record_usage(&self, _: TenantId, _: &Limit, _: i64, _: &str) -> Result<(), Error> {
        Ok(())
    }
}

#[tokio::test]
async fn han_muc_truong_tu_dinh_nghia_theo_goi() {
    TestDatabase::run(async |test| {
        let plan: Arc<dyn Entitlements> = Arc::new(OneField::default());
        let app = TestApp::builder(test)
            .tenant("acme")
            .user_in("acme", "admin", 1, &["shop.read", "kernel.custom_fields.manage"])
            .provide::<dyn Entitlements>(plan)
            .module(Shop)
            .build()
            .await
            .unwrap();
        let define = async |key: &str| {
            app.post("/api/v1/kernel/custom-fields")
                .token("admin")
                .header("autoxide-tenant", "acme")
                .json(json!({"target": "shop.item", "key": key, "data_type": "text", "labels": {"en": key}}))
                .send()
                .await
                .unwrap()
        };
        let first = define("color").await;
        assert_eq!(first.status(), StatusCode::CREATED, "{}", first.json());
        let second = define("size").await;
        assert_eq!(
            (second.status(), second.json()["code"].as_str()),
            (StatusCode::CONFLICT, Some("plans.limit_reached"))
        );
        let id = first.json()["data"]["id"].as_str().unwrap().to_owned();
        let removed = app
            .delete(&format!("/api/v1/kernel/custom-fields/{id}"))
            .token("admin")
            .header("autoxide-tenant", "acme")
            .send()
            .await
            .unwrap();
        assert_eq!(removed.status(), StatusCode::NO_CONTENT);
        assert_eq!(define("size").await.status(), StatusCode::CREATED, "xoá trả suất");
    })
    .await
    .unwrap();
}

// Danh sách trường phân trang theo `position` (keyset trong database), chỉ trường của loại bản ghi người gọi
// đọc được: trường của loại khác bị bỏ khi đọc từng lô, trang vẫn đủ `limit` và con trỏ đi tiếp đúng chỗ.
#[tokio::test]
async fn danh_sach_truong_phan_trang_chi_loai_doc_duoc() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .user("admin", 1, &["kernel.custom_fields.manage"])
            .user("reader", 2, &["shop.read"])
            .module(Shop)
            .module(Vault)
            .build()
            .await
            .unwrap();
        // Xen kẽ hai loại: vị trí 1..=6, loại `vault.box` ở vị trí chẵn.
        for (position, target) in
            [(1, "shop.item"), (2, "vault.box"), (3, "shop.item"), (4, "vault.box"), (5, "shop.item"), (6, "shop.item")]
        {
            let key = format!("f{position}");
            let made = app
                .post("/api/v1/kernel/custom-fields")
                .token("admin")
                .json(json!({"target": target, "key": key, "data_type": "text", "labels": {"en": key}, "position": position}))
                .send()
                .await
                .unwrap();
            assert_eq!(made.status(), StatusCode::CREATED, "{}", made.json());
        }
        let walk = async |token: &str, order: &str| -> Vec<String> {
            let mut keys = Vec::new();
            let mut cursor: Option<String> = None;
            loop {
                let mut path = format!("/api/v1/kernel/custom-fields?limit=2&order={order}");
                if let Some(cursor) = &cursor {
                    path.push_str(&format!("&cursor={cursor}"));
                }
                let page = app.get(&path).token(token).send().await.unwrap().json();
                let data = page["data"].as_array().unwrap();
                if page["pagination"]["next_cursor"].is_string() {
                    assert_eq!(data.len(), 2, "trang chưa cuối luôn đủ `limit`: {page}");
                }
                keys.extend(data.iter().map(|field| field["key"].as_str().unwrap().to_owned()));
                match page["pagination"]["next_cursor"].as_str() {
                    Some(next) => cursor = Some(next.to_owned()),
                    None => return keys,
                }
            }
        };
        assert_eq!(walk("reader", "asc").await, ["f1", "f3", "f5", "f6"]);
        assert_eq!(walk("reader", "desc").await, ["f6", "f5", "f3", "f1"]);
        assert_eq!(walk("admin", "asc").await, ["f1", "f2", "f3", "f4", "f5", "f6"]);
        let counted =
            app.get("/api/v1/kernel/custom-fields?limit=1&count=true").token("reader").send().await.unwrap().json();
        assert_eq!(counted["pagination"]["total"], 4);
        // Quyền đọc loại bản ghi kiểm trước tham số trang: 403 chứ không phải 422.
        let refused =
            app.get("/api/v1/kernel/custom-fields?target=vault.box&limit=0").token("reader").send().await.unwrap();
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    })
    .await
    .unwrap();
}

/// Thêm thẳng vào bảng các trường `(khoá, loại bản ghi, vị trí)` (bỏ qua trần 100 trường mỗi loại của API: cần
/// hàng nghìn dòng ẩn để vượt nhiều lô đọc).
async fn insert_fields(app: &TestApp, rows: &[(String, &str, i32)]) {
    use sea_orm::ConnectionTrait;
    use sea_orm::sea_query::{Alias, Query};
    let tenant =
        autoxide_kernel::tenancy::DatabaseRegistry::single(app.database().clone()).default_tenant();
    let write = tenant.database().write();
    for chunk in rows.chunks(200) {
        let mut insert = Query::insert();
        insert.into_table(Alias::new("kernel_custom_fields")).columns(
            [
                "id",
                "tenant_id",
                "target",
                "key",
                "data_type",
                "labels",
                "help",
                "required",
                "options",
                "visibility",
                "filterable",
                "personal_data",
                "position",
                "active",
                "created_at",
                "updated_at",
            ]
            .map(Alias::new),
        );
        for (key, target, position) in chunk {
            let now = autoxide_kernel::time::Timestamp::now();
            insert.values_panic([
                uuid::Uuid::now_v7().into(),
                tenant.id().as_uuid().into(),
                (*target).into(),
                key.clone().into(),
                "text".into(),
                "{}".into(),
                "{}".into(),
                false.into(),
                "[]".into(),
                "internal".into(),
                false.into(),
                false.into(),
                (*position).into(),
                true.into(),
                now.into(),
                now.into(),
            ]);
        }
        write.execute(&insert).await.unwrap();
    }
}

// Loại bản ghi đọc được lọc trong SQL; phần kiểm từng dòng còn lại (loại con sai dạng `shop.kind.a.b` của một loại
// đọc được) làm trang đi qua nhiều lô 200 dòng bị bỏ; mỗi request quét tối đa 10 lô, hết ngân sách thì trả trang ngắn
// kèm `next_cursor` ở dòng cuối đã quét, request sau đi tiếp từ đó; danh sách dài hơn ngân sách thì `count=true`
// không có `total`. Trường của loại người gọi không đọc được (`vault.box`) không bao giờ được quét, nên không con
// trỏ nào nêu nó.
#[tokio::test]
async fn danh_sach_truong_vuot_lo_va_ngan_sach_quet() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .user("reader", 2, &["shop.read"])
            .module(Shop)
            .module(Vault)
            .build()
            .await
            .unwrap();
        let mut rows: Vec<(String, &str, i32)> = vec![("first".to_owned(), "shop.item", 0)];
        rows.extend((1..=450).map(|position| (format!("h{position}"), "shop.kind.a.b", position)));
        rows.push(("middle".to_owned(), "shop.item", 451));
        rows.extend(
            (452..=2551).map(|position| (format!("h{position}"), "shop.kind.a.b", position)),
        );
        // Trường của loại người đọc không được đọc, xen giữa: SQL bỏ, không tính vào ngân sách quét.
        rows.extend((2552..=2851).map(|position| (format!("v{position}"), "vault.box", position)));
        rows.push(("last".to_owned(), "shop.item", 3000));
        insert_fields(&app, &rows).await;
        // Con trỏ là base64 của JSON: vị trí trong đó không bao giờ là của một trường `vault.box`.
        let cursor_position = |cursor: &str| -> i64 {
            use base64::Engine as _;
            let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(cursor).unwrap();
            let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            value["v"]["i"].as_i64().unwrap()
        };
        let get = async |query: &str| {
            let answer = app
                .get(&format!("/api/v1/kernel/custom-fields?{query}"))
                .token("reader")
                .send()
                .await
                .unwrap();
            assert_eq!(answer.status(), StatusCode::OK, "{}", answer.json());
            answer.json()
        };
        let keys = |page: &serde_json::Value| -> Vec<String> {
            page["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f["key"].as_str().unwrap().to_owned())
                .collect()
        };
        // `limit=1`: theo con trỏ tới hết ra đúng ba trường được xem, qua nhiều lô ẩn.
        let mut seen = Vec::new();
        let mut cursor: Option<String> = None;
        let mut requests = 0;
        loop {
            let mut query = "limit=1".to_owned();
            if let Some(cursor) = &cursor {
                query.push_str(&format!("&cursor={cursor}"));
            }
            let page = get(&query).await;
            seen.extend(keys(&page));
            requests += 1;
            assert!(requests < 10, "không đi vòng");
            match page["pagination"]["next_cursor"].as_str() {
                Some(next) => {
                    assert!(!(2552..=2851).contains(&cursor_position(next)), "{next}");
                    cursor = Some(next.to_owned());
                }
                None => break,
            }
        }
        assert_eq!(seen, ["first", "middle", "last"]);
        // Trang lớn: 10 lô (2000 dòng) hết trước khi gặp `last` — trang ngắn hơn `limit` nhưng vẫn có con trỏ.
        let page = get("limit=50&count=true").await;
        assert_eq!(keys(&page), ["first", "middle"]);
        let next = page["pagination"]["next_cursor"].as_str().unwrap().to_owned();
        assert_eq!(cursor_position(&next), 1999, "dòng cuối đã quét, một trường loại con sai dạng");
        assert!(page["pagination"].get("total").is_none(), "quá ngân sách thì không đếm: {page}");
        let rest = get(&format!("limit=50&cursor={next}")).await;
        assert_eq!(keys(&rest), ["last"]);
        assert!(rest["pagination"]["next_cursor"].is_null());
    })
    .await
    .unwrap();
}
