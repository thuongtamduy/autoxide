//! Sàn chặn mở bán hàng của người bán chưa định danh (LG-3, D-20261127-seller-kyc-listing): `catalog` hỏi
//! `KycGate` khi khách là sàn (`kernel.business_model = marketplace`). Ở đây `KycGate` giả ghi lại ai bị hỏi;
//! đường chặn thật với `compliance` (REST và MCP) ở test đầu-cuối `crates/app/src/e2e/seller_kyc.rs`.

use std::sync::Arc;

use autoxide_compliance::api::{Activity, KycGate, KycSession, KycStatus, Subject};
use autoxide_kernel::error::Error;
use autoxide_kernel::identity::PartyType;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, TenantId};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::{Value, json};

use super::{app_with, call, create_product};

/// Đặt `kernel.business_model` của khách mặc định (trước lần đọc cài đặt đầu tiên: có bộ nhớ đệm).
async fn business_model(app: &TestApp, value: &str) {
    use sea_orm::ConnectionTrait as _;
    let write = app.database().write();
    let backend = write.get_database_backend();
    let sql = "INSERT INTO kernel_tenant_settings (tenant_id, name, value, secret, updated_by, updated_at) \
               VALUES ($1, 'kernel.business_model', $2, FALSE, NULL, $3)";
    let values: Vec<sea_orm::Value> = vec![
        DEFAULT_TENANT.as_uuid().into(),
        value.into(),
        autoxide_kernel::time::Timestamp::now().into(),
    ];
    write.execute_raw(autoxide_kernel::database::statement(backend, sql, values)).await.unwrap();
}

async fn publish(app: &TestApp, id: &str) -> (StatusCode, Value) {
    let body = Some(json!({"status": "active"}));
    call(app, "POST", &format!("/products/{id}/status"), "boss", body).await
}

/// `KycGate` giả ghi lại mọi lần bị hỏi.
#[derive(Default)]
struct Recorder(std::sync::Mutex<Vec<String>>);

#[async_trait::async_trait]
impl KycGate for Recorder {
    async fn require_verified(
        &self,
        _tenant: TenantId,
        subject: Subject,
        activity: Activity,
    ) -> Result<(), Error> {
        let (_, id) = subject.columns();
        self.0.lock().unwrap().push(format!("{id}:{}", activity.as_str()));
        Ok(())
    }

    async fn status(&self, _tenant: TenantId, _subject: Subject) -> Result<KycStatus, Error> {
        Err(Error::internal("not used in this test"))
    }

    async fn start(
        &self,
        _tenant: TenantId,
        _subject: Subject,
        _party: PartyType,
        _return_url: Option<String>,
    ) -> Result<KycSession, Error> {
        Err(Error::internal("not used in this test"))
    }
}

/// Chỉ sàn mới hỏi, và hỏi đúng người bán (nút sở hữu sản phẩm) cho việc `sell`; khách chưa đặt mô hình
/// kinh doanh, hay đặt mô hình khác, không bị hỏi.
#[tokio::test]
async fn chi_san_hoi_dinh_danh_dung_nguoi_ban() {
    for (model, asked) in [(None, false), (Some("independent"), false), (Some("marketplace"), true)]
    {
        TestDatabase::run(async |test| {
            let recorder = Arc::new(Recorder::default());
            let gate: Arc<dyn KycGate> = recorder.clone();
            let (app, tree) = app_with(test, |builder| builder.provide(gate)).await;
            if let Some(model) = model {
                business_model(&app, model).await;
            }
            let (product, _) = create_product(&app, "boss", tree.a, "K-1", "Hàng").await;
            let (status, sold) = publish(&app, &product).await;
            assert_eq!(status, StatusCode::OK, "{sold}");
            let expected =
                if asked { vec![format!("{}:sell", tree.a.as_uuid())] } else { Vec::new() };
            assert_eq!(*recorder.0.lock().unwrap(), expected, "{model:?}");
        })
        .await
        .unwrap();
    }
}

/// Sàn không cài `compliance` (tự ghép module): không có gì để hỏi, mở bán được (máy chủ ghi log cảnh báo).
#[tokio::test]
async fn san_khong_co_compliance_khong_chan() {
    TestDatabase::run(async |test| {
        let (app, tree) = super::app(test).await;
        business_model(&app, "marketplace").await;
        let (product, _) = create_product(&app, "boss", tree.a, "K-1", "Hàng").await;
        let (status, sold) = publish(&app, &product).await;
        assert_eq!(status, StatusCode::OK, "{sold}");
    })
    .await
    .unwrap();
}
