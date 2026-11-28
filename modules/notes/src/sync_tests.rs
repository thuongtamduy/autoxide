//! Đồng bộ khi mất mạng (bước 69 P2) trên module mẫu: luồng thay đổi đúng thứ tự và đủ dấu xoá,
//! con trỏ cũ trả `410`, `If-Match` cũ trả `412` kèm bản hiện tại, lô ghi gửi lại không ghi trùng,
//! không thấy ghi chú ngoài phạm vi, hai máy cùng sửa một ghi chú — một bên thắng. Chạy trên cả
//! SQLite và Postgres.

use autoxide_kernel::time::SignedDuration;
use autoxide_kernel::time::Timestamp;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use autoxide_kernel::access::Scope;
use autoxide_kernel::error::Error;
use autoxide_kernel::idempotency::{Begin, IdempotencyStore, StoredResponse};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, root_of};
use autoxide_testkit::http::StatusCode;
use autoxide_testkit::{TestApp, TestDatabase};
use serde_json::{Value, json};
use uuid::Uuid;

/// `ada` và `cid` có ghi chú riêng (`own`); `mia` quản lý thấy mọi ghi chú của khách.
async fn app(test: &TestDatabase) -> TestApp {
    let root = root_of(DEFAULT_TENANT);
    let own = [("notes.read", Scope::Own), ("notes.write", Scope::Own)];
    TestApp::builder(test)
        .member("ada", 0xada, root, &own)
        .member("cid", 0xc1d, root, &own)
        .member("mia", 0x313, root, &[("notes.read", Scope::Tenant)])
        // Nơi lưu idempotency key (module `idempotency` cung cấp trong chương trình thật): lô gửi
        // lại không làm lại lệnh đã làm.
        .provide::<dyn IdempotencyStore>(Arc::new(MemoryStore::default()))
        .module(crate::module())
        .build()
        .await
        .unwrap()
}

async fn create(app: &TestApp, who: &str, title: &str) -> (String, String) {
    let answer = app
        .post("/api/v1/notes")
        .token(who)
        .json(json!({"title": title, "body": "first"}))
        .send()
        .await
        .unwrap();
    assert_eq!(answer.status_code(), StatusCode::CREATED, "{}", answer.text());
    let id = answer.json()["data"]["id"].as_str().unwrap().to_owned();
    let version = app.get(&format!("/api/v1/notes/{id}")).token(who).send().await.unwrap();
    (id, version.header("etag").to_owned())
}

async fn changes(app: &TestApp, who: &str, query: &str) -> (StatusCode, Value) {
    let answer = app.get(&format!("/api/v1/notes/changes{query}")).token(who).send().await.unwrap();
    (answer.status_code(), answer.json())
}

/// Đọc hết luồng như app: theo `next_cursor` tới trang cuối, áp từng mục vào bản sao trên máy;
/// trả bản sao, con trỏ của mục cuối và mọi mục đã nhận.
async fn pull(
    app: &TestApp,
    who: &str,
    mut cursor: Option<String>,
    limit: u32,
    device: &mut BTreeMap<String, Value>,
) -> (Option<String>, Vec<Value>) {
    let mut seen = Vec::new();
    loop {
        let query = match &cursor {
            Some(cursor) => format!("?limit={limit}&cursor={cursor}"),
            None => format!("?limit={limit}"),
        };
        let (status, page) = changes(app, who, &query).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        let items = page["data"].as_array().unwrap().clone();
        for item in &items {
            let id = item["id"].as_str().unwrap().to_owned();
            if item["deleted"] == true {
                assert!(item["data"].is_null() && item["version"].is_null());
                device.remove(&id);
            } else {
                device.insert(id, item["data"].clone());
            }
        }
        seen.extend(items.iter().cloned());
        match page["pagination"]["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => {
                return (
                    items.last().map(|item| item["cursor"].as_str().unwrap().to_owned()).or(cursor),
                    seen,
                );
            }
        }
    }
}

#[tokio::test]
async fn luong_thay_doi_dung_thu_tu_du_dau_xoa() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (a, _) = create(&app, "ada", "A").await;
        let (b, _) = create(&app, "ada", "B").await;
        let patched = app
            .patch(&format!("/api/v1/notes/{a}"))
            .token("ada")
            .json(json!({"body": "second"}))
            .send()
            .await
            .unwrap();
        assert_eq!(patched.status_code(), StatusCode::OK);
        let removed = app.delete(&format!("/api/v1/notes/{b}")).token("ada").send().await.unwrap();
        assert_eq!(removed.status_code(), StatusCode::NO_CONTENT);
        let (c, _) = create(&app, "ada", "C").await;

        // Một trang đủ lớn: mỗi ghi chú một mục, ở vị trí lần đổi cuối của nó.
        let (status, page) = changes(&app, "ada", "").await;
        assert_eq!(status, StatusCode::OK, "{page}");
        let order: Vec<(&str, bool)> = page["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| (item["id"].as_str().unwrap(), item["deleted"].as_bool().unwrap()))
            .collect();
        assert_eq!(order, vec![(a.as_str(), false), (b.as_str(), true), (c.as_str(), false)]);
        assert_eq!(page["data"][0]["data"]["body"], "second");
        assert!(page["data"][0]["version"].as_str().unwrap().starts_with('"'));
        assert!(page["pagination"]["next_cursor"].is_null());

        // Trang nhỏ, theo `next_cursor`: bản sao trên máy cuối cùng như máy chủ.
        let mut device = BTreeMap::new();
        let (cursor, _) = pull(&app, "ada", None, 2, &mut device).await;
        assert_eq!(device.keys().cloned().collect::<Vec<_>>(), vec![a.clone(), c.clone()]);

        // Lần sau, từ con trỏ đã giữ: thấy dấu xoá của C; mục gần đây có thể đến lại (con trỏ lùi
        // vài giây để không bỏ sót transaction commit muộn) — áp lại không đổi kết quả.
        let removed = app.delete(&format!("/api/v1/notes/{c}")).token("ada").send().await.unwrap();
        assert_eq!(removed.status_code(), StatusCode::NO_CONTENT);
        let (_, seen) = pull(&app, "ada", cursor, 100, &mut device).await;
        assert!(seen.iter().any(|item| item["id"] == c.as_str() && item["deleted"] == true));
        assert_eq!(device.keys().cloned().collect::<Vec<_>>(), vec![a.clone()]);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn con_tro_qua_cu_hay_sai() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        // UUIDv7 của 40 ngày trước: cũ hơn 30 ngày giữ mặc định.
        let old = Uuid::new_v7(uuid::Timestamp::from_unix(
            uuid::NoContext,
            u64::try_from(
                (Timestamp::now() - SignedDuration::from_hours(24 * 40)).as_unix_seconds(),
            )
            .unwrap(),
            0,
        ));
        let (status, problem) = changes(&app, "ada", &format!("?cursor={old}")).await;
        assert_eq!(status, StatusCode::GONE, "{problem}");
        assert_eq!(problem["code"], "sync_cursor_expired");
        let (status, problem) = changes(&app, "ada", "?cursor=abc&limit=0").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["errors"].as_array().unwrap().len(), 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khong_thay_ghi_chu_ngoai_pham_vi() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (mine, _) = create(&app, "ada", "Ada's").await;
        let (theirs, _) = create(&app, "cid", "Cid's").await;
        let ids = |page: &Value| -> Vec<String> {
            page["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| i["id"].as_str().unwrap().to_owned())
                .collect()
        };
        let (_, page) = changes(&app, "ada", "").await;
        assert_eq!(ids(&page), vec![mine.clone()]);
        let (_, page) = changes(&app, "cid", "").await;
        assert_eq!(ids(&page), vec![theirs.clone()]);
        // Dấu xoá cũng theo phạm vi: Ada xoá ghi chú của mình, Cid không biết gì.
        app.delete(&format!("/api/v1/notes/{mine}")).token("ada").send().await.unwrap();
        let (_, page) = changes(&app, "cid", "").await;
        assert_eq!(ids(&page), vec![theirs.clone()]);
        // Người quản lý thấy cả hai — ghi chú của Ada ở vị trí lần đổi cuối (dấu xoá).
        let (_, page) = changes(&app, "mia", "").await;
        assert_eq!(ids(&page), vec![theirs, mine]);
        assert_eq!(page["data"][1]["deleted"], true);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn if_match_cu_tra_412_kem_ban_hien_tai() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (id, old) = create(&app, "ada", "Draft").await;
        let uri = format!("/api/v1/notes/{id}");
        let first = app
            .patch(&uri)
            .token("ada")
            .header("if-match", &old)
            .json(json!({"body": "from phone"}))
            .send()
            .await
            .unwrap();
        assert_eq!(first.status_code(), StatusCode::OK);
        let now = first.header("etag").to_owned();

        let stale = app
            .patch(&uri)
            .token("ada")
            .header("if-match", &old)
            .json(json!({"body": "from laptop"}))
            .send()
            .await
            .unwrap();
        assert_eq!(stale.status_code(), StatusCode::PRECONDITION_FAILED);
        let problem = stale.json();
        assert_eq!(problem["code"], "precondition_failed");
        assert_eq!(problem["current"]["body"], "from phone");
        assert_eq!(problem["current"]["id"], id.as_str());
        assert_eq!(stale.header("etag"), now);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hai_may_cung_sua_mot_ben_thang() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (id, version) = create(&app, "ada", "Shared").await;
        let uri = format!("/api/v1/notes/{id}");
        let send = |body: &'static str| {
            app.patch(&uri)
                .token("ada")
                .header("if-match", &version)
                .json(json!({"body": body}))
                .send()
        };
        let (phone, laptop) = tokio::join!(send("phone"), send("laptop"));
        let mut statuses = [phone.unwrap().status_code(), laptop.unwrap().status_code()];
        statuses.sort();
        assert_eq!(statuses, [StatusCode::OK, StatusCode::PRECONDITION_FAILED]);
    })
    .await
    .unwrap();
}

async fn batch(app: &TestApp, who: &str, operations: Value) -> (StatusCode, Value) {
    let answer = app
        .post("/api/v1/kernel/sync/batch")
        .token(who)
        .json(json!({"operations": operations}))
        .send()
        .await
        .unwrap();
    (answer.status_code(), answer.json())
}

#[tokio::test]
async fn lo_ghi_gui_lai_khong_ghi_trung() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (id, old) = create(&app, "ada", "Existing").await;
        let changed = app
            .patch(&format!("/api/v1/notes/{id}"))
            .token("ada")
            .json(json!({"body": "changed online"}))
            .send()
            .await
            .unwrap();
        assert_eq!(changed.status_code(), StatusCode::OK);
        let missing = Uuid::now_v7();
        let operations = json!([
            {"id": "1", "method": "POST", "path": "/api/v1/notes", "idempotency_key": "k-1",
             "body": {"title": "Written offline"}},
            {"id": "2", "method": "PATCH", "path": format!("/api/v1/notes/{id}"),
             "idempotency_key": "k-2", "if_match": old, "body": {"body": "changed offline"}},
            {"id": "3", "method": "DELETE", "path": format!("/api/v1/notes/{missing}"),
             "idempotency_key": "k-3"},
            {"id": "4", "method": "POST", "path": "/api/v1/notes", "idempotency_key": "k-4",
             "body": {"title": "Also offline", "body": "x"}},
        ]);
        let (status, first) = batch(&app, "ada", operations.clone()).await;
        assert_eq!(status, StatusCode::OK, "{first}");
        let statuses: Vec<u64> = first["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["status"].as_u64().unwrap())
            .collect();
        // Một lệnh hỏng không chặn lệnh khác.
        assert_eq!(statuses, vec![201, 412, 404, 201]);
        assert_eq!(first["data"][1]["body"]["current"]["body"], "changed online");
        assert_eq!(first["data"][2]["body"]["code"], "notes.note_not_found");
        let created = first["data"][0]["body"]["data"]["id"].clone();
        assert!(
            first["data"][0]["location"].as_str().unwrap().ends_with(created.as_str().unwrap())
        );

        // Gửi lại cả lô (mất mạng giữa chừng, app không biết đã tới chưa): cùng kết quả, không ghi trùng.
        let (_, again) = batch(&app, "ada", operations).await;
        assert_eq!(again["data"][0]["body"]["data"]["id"], created);
        let list = app.get("/api/v1/notes?limit=100").token("ada").send().await.unwrap().json();
        let titles: Vec<&str> =
            list["data"].as_array().unwrap().iter().map(|n| n["title"].as_str().unwrap()).collect();
        assert_eq!(titles.iter().filter(|t| **t == "Written offline").count(), 1);
        assert_eq!(titles.len(), 3);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn lo_ghi_chi_nhan_duong_dan_dong_bo_va_quyen_nhu_rest() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (status, problem) = batch(
            &app,
            "ada",
            json!([{"id": "x", "method": "POST", "path": "/api/v1/kernel/sync/batch",
                    "idempotency_key": "k", "body": {}}]),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["errors"][0]["field"], "operations[0].path");
        let (status, _) = batch(&app, "ada", json!([])).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

        // Quyền như REST: `mia` chỉ đọc, ghi qua lô bị từ chối như gọi thẳng.
        let (status, answer) = batch(
            &app,
            "mia",
            json!([{"id": "1", "method": "POST", "path": "/api/v1/notes", "idempotency_key": "m",
                    "body": {"title": "Nope"}}]),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(answer["data"][0]["status"], 403);
        assert_eq!(answer["data"][0]["body"]["code"], "permission_denied");

        let resources = app.get("/api/v1/kernel/sync/resources").token("ada").send().await.unwrap();
        assert_eq!(
            resources.json()["data"],
            json!([{"name": "notes.notes", "path": "/api/v1/notes",
                    "changes": "/api/v1/notes/changes", "read_permission": "notes.read",
                    "description": "Your notes, in your scope of `notes.read`."}])
        );
    })
    .await
    .unwrap();
}

/// Nơi lưu idempotency key trong bộ nhớ, cùng giao kèo với module `idempotency`.
#[derive(Default)]
struct MemoryStore(Mutex<HashMap<String, (Uuid, Option<StoredResponse>)>>);

#[autoxide_kernel::async_trait]
impl IdempotencyStore for MemoryStore {
    async fn begin(
        &self,
        _: &TenantContext,
        scope: &str,
        key: &str,
        _: &str,
        _: Timestamp,
    ) -> Result<Begin, Error> {
        let mut keys = self.0.lock().unwrap();
        match keys.get(&format!("{scope}|{key}")) {
            Some((_, Some(stored))) => Ok(Begin::Replay(stored.clone())),
            Some((_, None)) => Ok(Begin::InProgress),
            None => {
                let id = Uuid::now_v7();
                keys.insert(format!("{scope}|{key}"), (id, None));
                Ok(Begin::Started { id })
            }
        }
    }

    async fn complete(
        &self,
        _: &TenantContext,
        id: Uuid,
        response: StoredResponse,
    ) -> Result<(), Error> {
        for value in self.0.lock().unwrap().values_mut() {
            if value.0 == id {
                value.1 = Some(response.clone());
            }
        }
        Ok(())
    }

    async fn abandon(&self, _: &TenantContext, id: Uuid) -> Result<(), Error> {
        self.0.lock().unwrap().retain(|_, value| value.0 != id);
        Ok(())
    }
}

/// Giới hạn request theo khách (D-20261126-tenant-rate-limit): một lô ghi tính MỘT lượt, không một lượt cho mỗi lệnh,
/// nên lô không bị 429 giữa chừng.
#[tokio::test]
async fn lo_ghi_tinh_mot_luot_vao_khach() {
    TestDatabase::run(async |test| {
        let own = [("notes.read", Scope::Own), ("notes.write", Scope::Own)];
        let app = TestApp::builder(test)
            .setting("AUTOXIDE_HTTP_RATE_LIMIT_TENANT_PER_MINUTE", "2")
            .setting("AUTOXIDE_HTTP_RATE_LIMIT_TENANT_BURST", "2")
            .setting("AUTOXIDE_HTTP_RATE_LIMIT_PER_MINUTE", "0")
            .member("ada", 0xada, root_of(DEFAULT_TENANT), &own)
            .provide::<dyn IdempotencyStore>(Arc::new(MemoryStore::default()))
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let operations: Vec<Value> = (1..=3)
            .map(|n| {
                json!({"id": n.to_string(), "method": "POST", "path": "/api/v1/notes",
                       "idempotency_key": format!("k-{n}"), "body": {"title": format!("Note {n}")}})
            })
            .collect();
        let (status, answer) = batch(&app, "ada", json!(operations)).await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        for result in answer["data"].as_array().unwrap() {
            assert_eq!(result["status"], 201, "{answer}");
        }
        let list = || app.get("/api/v1/notes").token("ada").send();
        assert_eq!(list().await.unwrap().status_code(), StatusCode::OK);
        assert_eq!(list().await.unwrap().status_code(), StatusCode::TOO_MANY_REQUESTS);
    })
    .await
    .unwrap();
}
