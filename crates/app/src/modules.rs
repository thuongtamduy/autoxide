//! Nơi DUY NHẤT đăng ký module vào chương trình.
//!
//! Thêm một module = thêm một dòng vào [`all`] (và một dòng phụ thuộc trong
//! `Cargo.toml` của crate này). Không chỗ nào khác trong core phải sửa.

use autoxide_kernel::module::Module;

/// Mọi module của chương trình, theo thứ tự đăng ký.
pub fn all() -> Vec<Box<dyn Module>> {
    // Mỗi module một dòng. Module cần trait của module khác đăng ký SAU module đó.
    vec![
        // Gửi mail: các module gửi mail cần nó, nên đứng trước chúng.
        Box::new(autoxide_mail::module()),
        // AI (D-109): công bố `dyn TextModel` và `dyn Embedder` (bản "đã tắt" khi chưa cấu
        // hình); `media` và `content` cần, nên đứng trước chúng.
        Box::new(autoxide_ai::module()),
        // Lưu file: các module lưu file cần nó, nên đứng trước chúng.
        Box::new(autoxide_files::module()),
        // Đơn vị hành chính dùng chung (bước 34, D-314): `organizations` cần
        // `dyn AdministrativeUnits` để kiểm địa chỉ của nút, nên đứng trước nó.
        Box::new(autoxide_address::module()),
        // Cây tổ chức của mỗi khách (bước 34): `auth` cần `dyn OrgDirectory` để kiểm nút khi cấp
        // membership, nên đứng trước `auth`; không cần gì của `auth`.
        Box::new(autoxide_organizations::module()),
        // Người dùng, mật khẩu (và sau này đăng nhập, role, permission): mọi module
        // khác có thể cần, nên đứng đầu.
        Box::new(autoxide_auth::module()),
        // Audit log: nhận event của mọi module; không module nào cần trait của nó.
        Box::new(autoxide_audit::module()),
        // Chỗ lưu idempotency key cho lớp HTTP của kernel.
        Box::new(autoxide_idempotency::module()),
        // Webhook: gửi event của mọi module ra ngoài; không module nào cần trait của nó.
        Box::new(autoxide_webhooks::module()),
        // Thuế (bước 36): sổ cái và chứng từ (sau) cần `dyn TaxEngine`, nên đứng trước chúng.
        Box::new(autoxide_tax::module()),
        // Sổ cái kép (bước 36): cần `dyn OrgDirectory` (múi giờ, tiền tệ của nút); chứng từ, thanh
        // toán và module thương mại (sau) ghi sổ qua `dyn Ledger`.
        Box::new(autoxide_ledger::module()),
        // Chứng từ (bước 36): cần `dyn TaxEngine` và `dyn Ledger`; `plans` (tính tiền thuê bao) và module
        // thương mại lập chứng từ qua `dyn Invoices`.
        Box::new(autoxide_invoices::module()),
        // Tuân thủ (bước 37): định danh (KYC), quy tắc pháp lý, văn bản pháp lý, sổ đồng ý. Dùng
        // `dyn IdentityVerifier` nếu có module cung cấp (đứng trước); `plans` (khách tự đăng ký) và module
        // thương mại chặn qua `dyn KycGate` / `dyn AcceptanceGate`.
        Box::new(autoxide_compliance::module()),
        // Thông báo (bước 37, D-348): email, SMS, ZNS, đẩy, hộp thư trong ứng dụng. Cần `dyn Mailer`,
        // `dyn Users` / `dyn TenantAdmins` của `auth`, sổ đồng ý và quy tắc của `compliance` (đều
        // đứng trước); `plans` (thông báo thuê bao tới quản trị khách) và module sau khai loại thông
        // báo và gửi qua `dyn Notifier`.
        Box::new(autoxide_notifications::module()),
        // Đặt lịch dịch vụ (bước 60): múi giờ và chính sách theo nút qua `dyn OrgDirectory`, cọc qua
        // `dyn Ledger`, nhắc lịch qua `dyn Notifier` — đều đứng trước; không module nào cần nó.
        Box::new(autoxide_bookings::module()),
        // Gói, thuê bao, quyền dùng (bước 35): sau mọi module nền nó cần — `invoices` (hoá đơn thuê
        // bao), `ledger` (thu tiền) — và (bước 37) `compliance`. Module đăng ký trước vẫn hỏi quyền
        // dùng được qua tay cầm của kernel, gắn sau khi mọi module khởi động (D-323).
        Box::new(autoxide_plans::module()),
        // Thư viện media: cần nơi lưu của `files` (đứng sau nó); module nội dung sau này
        // cần trait `Media` của nó.
        Box::new(autoxide_media::module()),
        // Sau `media`: `content` cần `dyn Media` mà `media` công bố.
        Box::new(autoxide_content::module()),
        // Biểu mẫu: cần `dyn Mailer` (mail báo quản trị viên) và `dyn Storage` (file xuất CSV lớn),
        // nên đứng sau `mail` và `files`.
        Box::new(autoxide_forms::module()),
        // Bản tin: cần `dyn Mailer` (sau `mail`), sổ đồng ý chung `dyn ConsentLedger` của
        // `compliance` (sau `compliance`, D-367), nghe bài mới của `content` qua sổ đối tượng của
        // kernel (sau `content`).
        Box::new(autoxide_newsletter::module()),
        // Bình luận: cần `dyn Mailer` (sau `mail`), kiểm đích qua sổ đối tượng của kernel và nghe
        // event lưu trữ / xoá của `content` (sau `content`).
        Box::new(autoxide_comments::module()),
        Box::new(autoxide_support::module()),
        // Phân tích truy cập (bước 57): đọc sổ đồng ý `dyn ConsentLedger` của `compliance` (sau
        // `compliance`); loại sự kiện và phễu của mọi module đọc từ kernel, nên module đăng ký sau
        // vẫn khai được.
        Box::new(autoxide_analytics::module()),
        // Menu của website (D-238): cần `dyn Media` (ảnh của ô gợi ý) và đọc menu cũ của
        // `content` để nhập sang, nên đứng sau cả hai. Đối tượng đích tra qua sổ đối tượng của
        // kernel, nên module đăng ký sau vẫn được menu trỏ tới.
        Box::new(autoxide_menus::module()),
        // Sản phẩm (bước 41): cần `dyn Media` (ảnh sản phẩm) và `dyn TaxEngine` (luật giá bán cho
        // người tiêu dùng), nên đứng sau `media` và `tax`; đứng sau `comments` để hỏi đáp sản phẩm
        // (D-202). Module thương mại sau (`inventory`, `promotions`, `shipping`…) đăng ký SAU nó.
        Box::new(autoxide_catalog::module()),
        // Kho (bước 42): cần `dyn Catalog` (biến thể, combo, chế độ bán) nên đứng sau `catalog`;
        // dùng `dyn OrgDirectory` (loại nút) và `dyn Ledger` (công nợ nhà cung cấp) nếu có. `orders`
        // (bước 45) giữ chỗ qua `dyn Inventory`, nên đăng ký SAU nó.
        Box::new(autoxide_inventory::module()),
        // Vận chuyển (bước 44): cần `dyn AdministrativeUnits` (kiểm địa chỉ), đứng sau `catalog`
        // (cạnh `inventory`, `promotions`) và trước `orders`. Bộ nối hãng đăng ký SAU nó.
        Box::new(autoxide_shipping::module()),
        // Khuyến mại (bước 43): cần `dyn Catalog` (biến thể, giá ngay trước khuyến mại, sàn giá),
        // nên đứng sau `catalog`; đọc cài đặt kế thừa qua `dyn OrgDirectory` nếu có. `orders`
        // (bước 45) đăng ký SAU nó để gọi `dyn Promotions`.
        Box::new(autoxide_promotions::module()),
        // Giỏ hàng, đặt hàng, đơn hàng (bước 45): gọi `catalog`, `inventory`, `shipping`, `promotions`,
        // `tax`, `ledger` qua `api` của chúng nên đứng sau cả sáu. Module sau (`marketplace`,
        // `customers`, `channels`, `affiliate`) cắm vào `dyn OrderExtensions` của nó.
        Box::new(autoxide_orders::module()),
        // Báo cáo gộp của cây khách hàng (bước 38): nghe event mà mọi module khai số liệu
        // (`Module::facts`), cần `dyn Ledger` (tỉ giá) và `dyn OrgDirectory` (múi giờ), nên đứng
        // CUỐI CÙNG.
        Box::new(autoxide_reporting::module()),
        // Tự động hoá quy trình (bước 58): nghe event của mọi module và gọi tool MCP của mọi module
        // qua sổ tool của kernel, nên đứng CUỐI CÙNG (sau cả `reporting`).
        Box::new(autoxide_automations::module()),
    ]
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::config::ConfigSource;
    use autoxide_kernel::database::Database;
    use autoxide_kernel::tenancy::TenantContext;
    use autoxide_testkit::{
        TestDatabase, schema_ownership_violations, schema_snapshot, schema_tenancy_violations,
    };

    /// Mô tả do module khai (cài đặt theo khách, cài đặt theo nút, permission) hiện nguyên văn ở
    /// API: không được có chuỗi dấu cách liền nhau giữa câu — dấu vết của chuỗi xuống dòng mà thiếu
    /// `\` trong mã nguồn.
    #[test]
    fn mo_ta_cua_module_khong_co_dau_cach_thua() {
        let mut bad = Vec::new();
        for module in super::all() {
            let texts = module
                .tenant_settings()
                .iter()
                .map(|setting| (setting.name().to_owned(), setting.description()))
                .chain(
                    module
                        .org_settings()
                        .iter()
                        .map(|setting| (setting.key().to_owned(), setting.description())),
                )
                .chain(
                    module
                        .permissions()
                        .iter()
                        .map(|permission| (permission.name().to_owned(), permission.description())),
                )
                .collect::<Vec<_>>();
            for (name, text) in texts {
                if text.contains("  ") || text.contains('\n') {
                    bad.push(format!("{}: {name}", module.name()));
                }
            }
        }
        assert!(bad.is_empty(), "descriptions with repeated spaces: {bad:?}");
    }

    /// Chạy mọi migration của chương trình trên một database trống rồi hỏi chính
    /// database: mỗi bảng có thuộc đúng một module không, có foreign key nào xuyên
    /// module không, bảng cấp khách hàng có `tenant_id` đúng luật không. Gate chạy bài này
    /// trên cả SQLite và Postgres.
    #[tokio::test]
    async fn moi_bang_thuoc_mot_module_va_khong_foreign_key_xuyen_module() {
        TestDatabase::run(async |database| {
            let source = ConfigSource::from_pairs([("DATABASE_URL", database.url())]);
            let settings = crate::settings::read(&source).unwrap();
            let db = Database::connect(&settings.config.database).await.unwrap();
            let names = settings.modules.names();
            let migrator = settings.modules.migrator().unwrap();

            migrator.up(&db).await.unwrap();

            let violations = schema_ownership_violations(db.write(), &names).await.unwrap();
            assert_eq!(violations, Vec::<String>::new());
            // Bảng cấp khách hàng có `tenant_id`, khoá và index bắt đầu bằng nó (bước 33).
            let snapshot = schema_snapshot(db.write()).await.unwrap();
            let control_plane = settings.modules.control_plane_tables();
            let violations = schema_tenancy_violations(&snapshot, &control_plane);
            assert_eq!(violations, Vec::<String>::new());

            // Migration nào cũng phải gỡ được: xuống hết rồi lên lại.
            for name in names.iter().rev() {
                migrator.down(&db, name, u32::MAX).await.unwrap();
            }
            migrator.up(&db).await.unwrap();
            assert!(migrator.pending(&db).await.unwrap().is_empty());
            db.close().await.unwrap();
        })
        .await
        .unwrap();
    }

    /// Xuyên module: `auth` ghi event cùng transaction, bộ phát giao, `audit` ghi
    /// đúng một dòng — trên database gate đang chọn.
    #[tokio::test]
    async fn thay_doi_cua_auth_vao_audit_log() {
        use autoxide_auth::api::{Accounts, NewAccount};
        use sea_orm::{ConnectionTrait, Statement};

        TestDatabase::run(async |database| {
            let source = ConfigSource::from_pairs([("DATABASE_URL", database.url())]);
            let mut settings = crate::settings::read(&source).unwrap();
            let db = Database::connect(&settings.config.database).await.unwrap();
            settings.modules.migrator().unwrap().up(&db).await.unwrap();
            settings.modules.start(&db).unwrap();
            let accounts = settings.modules.api::<dyn Accounts>().unwrap();
            let dispatcher = settings.modules.dispatcher().unwrap();

            let account = NewAccount {
                email: "ada@example.com".to_owned(),
                name: "Ada".to_owned(),
                password: None,
                admin: true,
            };
            let ada =
                accounts.create(&TenantContext::default_on(db.clone()), account).await.unwrap();
            assert_eq!(dispatcher.run_once().await.unwrap(), 1);
            assert_eq!(dispatcher.run_once().await.unwrap(), 0);

            let backend = db.read().get_database_backend();
            let sql = "SELECT action, subject, actor_user_id, after FROM audit_entry";
            let rows = db.read().query_all_raw(Statement::from_string(backend, sql)).await.unwrap();
            assert_eq!(rows.len(), 1);
            let action: String = rows[0].try_get("", "action").unwrap();
            let subject: Option<String> = rows[0].try_get("", "subject").unwrap();
            let actor: Option<uuid::Uuid> = rows[0].try_get("", "actor_user_id").unwrap();
            let after: Option<String> = rows[0].try_get("", "after").unwrap();
            assert_eq!(action, "auth.user_created");
            assert_eq!(subject, Some(format!("auth.user:{}", ada.id)));
            assert_eq!(actor, None, "created from the command line, by the system");
            assert!(after.unwrap().contains(r#""admin":true"#));
            db.close().await.unwrap();
        })
        .await
        .unwrap();
    }

    /// Lịch định kỳ của `auth` được worker đưa vào hàng đợi và chạy — đúng một lần dù
    /// hai worker cùng chạy.
    #[tokio::test]
    async fn worker_chay_lich_dinh_ky_cua_module() {
        TestDatabase::run(async |database| {
            let source = ConfigSource::from_pairs([("DATABASE_URL", database.url())]);
            let mut settings = crate::settings::read(&source).unwrap();
            let db = Database::connect(&settings.config.database).await.unwrap();
            settings.modules.migrator().unwrap().up(&db).await.unwrap();
            settings.modules.start(&db).unwrap();
            let first = settings.modules.worker(4).unwrap();
            let second = settings.modules.worker(4).unwrap();

            let (a, b) = tokio::join!(first.run_once(), second.run_once());

            // Mỗi lịch (của `kernel` — dọn bản xuất, dọn file của job, backup theo lịch (bước 39) —, `ai`, `auth`, `idempotency`, `webhooks`, hai của `files` (dọn mồ côi, dọn lượt tải thẳng bỏ dở), sáu của `plans`, một
            // của `tax`, sáu của `ledger` (niêm phong, hết hạn thanh toán, đối soát, nhả tạm giữ, kiểm chi trả, đọc biến động ngân hàng), hai của `invoices` (hỏi trạng thái hoá đơn điện tử, dọn chứng từ hết hạn lưu), năm của
            // `compliance` (hết hạn định danh, nhắc hạn sự cố 72 giờ, dọn bằng chứng đồng ý cookie, nhắc vụ việc quá
            // hạn — D-360, D-361, D-363;
            // nhập quy tắc chạy ở mặt điều khiển — cũng là database này), một
            // của `notifications` (dọn nhật ký gửi), hai của `bookings` (trả chỗ giữ tạm quá hạn, nhắc lịch), ba của `media` (thêm dọn lượt tải thẳng bỏ dở), hai của `content`, một của `forms`, hai
            // của `newsletter` (dọn dẹp, ghi sổ đồng ý còn sót — D-367), một của `comments`, hai của `support` (đánh dấu quá hạn, tự đóng — bước 53) và hai của
            // `reporting` (đẩy số liệu, dọn số liệu cục bộ — bước 38), hai của `analytics` (gộp mỗi giờ,
            // dọn theo hạn lưu — bước 57), hai của `automations` (luật tới hạn, dọn nhật ký — bước 58),
            // một của `catalog` (tính lại readiness theo giấy tờ — bước 41), một của `inventory` (trả chỗ
            // giữ hàng quá hạn — bước 42), ba của `shipping` (hỏi lại vận đơn kẹt khi đặt hãng, hỏi hành
            // trình hãng không có webhook, COD quá hạn — bước 44), ba của `promotions` (chuyển trạng thái
            // theo giờ, hết hạn lượt giữ, hết hạn voucher — bước 43), hai của `orders` (giỏ bỏ dở, dọn giỏ
            // hết hạn — bước 45)) đúng một job.
            assert_eq!(a.unwrap() + b.unwrap(), 59);
            let done =
                first.list("auth", autoxide_kernel::job::JobStatus::Completed, 10).await.unwrap();
            assert_eq!(done.len(), 1);
            assert_eq!(done[0].kind, "auth.cleanup");
            db.close().await.unwrap();
        })
        .await
        .unwrap();
    }

    /// Đầu-cuối: chương trình thật, database thật (gate chạy cả hai), client MCP thật.
    /// Quản trị viên đăng nhập, gọi tool ghi qua MCP; thay đổi vào audit log, ghi đúng
    /// người làm.
    #[tokio::test]
    async fn client_mcp_goi_tool_ghi_va_vao_audit_log() {
        use autoxide_auth::api::{Accounts, NewAccount};
        use rmcp::ServiceExt as _;
        use rmcp::model::CallToolRequestParams;
        use rmcp::transport::StreamableHttpClientTransport;
        use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
        use sea_orm::{ConnectionTrait, Statement};
        use serde_json::json;
        use tower::ServiceExt as _;

        TestDatabase::run(async |database| {
            let source = ConfigSource::from_pairs([("DATABASE_URL", database.url())]);
            let settings = crate::settings::read(&source).unwrap();
            let db = Database::connect(&settings.config.database).await.unwrap();
            settings.modules.migrator().unwrap().up(&db).await.unwrap();
            db.close().await.unwrap();
            let runtime =
                crate::runtime::start(settings, crate::runtime::Migrations::RequireApplied)
                    .await
                    .unwrap();
            let accounts = runtime.modules.api::<dyn Accounts>().unwrap();
            let password = "correct horse battery staple";
            let account = |email: &str, admin: bool| NewAccount {
                email: email.to_owned(),
                name: "Someone".to_owned(),
                password: Some(password.to_owned()),
                admin,
            };
            let root = accounts
                .create(
                    &TenantContext::default_on(runtime.database.clone()),
                    account("root@example.com", true),
                )
                .await
                .unwrap();
            let bob = accounts
                .create(
                    &TenantContext::default_on(runtime.database.clone()),
                    account("bob@example.com", false),
                )
                .await
                .unwrap();
            runtime.dispatcher.run_once().await.unwrap();

            // Đăng nhập qua REST để lấy access token.
            let login = json!({"email": "root@example.com", "password": password});
            let request = axum::http::Request::builder()
                .method("POST")
                .uri("/api/v1/auth/login")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(login.to_string()))
                .unwrap();
            let response = runtime.api.router.clone().oneshot(request).await.unwrap();
            let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let token = body["data"]["access_token"].as_str().unwrap().to_owned();

            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/mcp", listener.local_addr().unwrap());
            let router = runtime.api.router.clone();
            let server = tokio::spawn(async move { axum::serve(listener, router).await });
            autoxide_kernel::tls::install_crypto_provider();
            let config = StreamableHttpClientTransportConfig::with_uri(url).auth_header(token);
            let client =
                ().serve(StreamableHttpClientTransport::from_config(config)).await.unwrap();

            let tools = client.list_all_tools().await.unwrap();
            let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
            for expected in ["auth.whoami", "auth.list_permissions", "auth.update_user"] {
                assert!(names.contains(&expected), "{names:?}");
            }
            let whoami = client.call_tool(CallToolRequestParams::new("auth.whoami")).await;
            let whoami = whoami.unwrap().structured_content.unwrap();
            assert_eq!(whoami["data"]["email"], "root@example.com");
            assert_eq!(whoami["data"]["all_permissions"], true);

            let arguments = json!({"id": bob.id, "name": "Robert"});
            let call = CallToolRequestParams::new("auth.update_user")
                .with_arguments(arguments.as_object().unwrap().clone());
            let updated = client.call_tool(call).await.unwrap();
            assert_eq!(updated.is_error, Some(false), "{updated:?}");
            assert_eq!(updated.structured_content.unwrap()["data"]["name"], "Robert");
            client.cancel().await.unwrap();
            server.abort();

            assert!(runtime.dispatcher.run_once().await.unwrap() >= 1);
            let backend = runtime.database.read().get_database_backend();
            let sql = "SELECT actor_user_id FROM audit_entry WHERE action = 'auth.user_updated'";
            let rows = runtime
                .database
                .read()
                .query_all_raw(Statement::from_string(backend, sql))
                .await
                .unwrap();
            assert_eq!(rows.len(), 1);
            let actor: Option<uuid::Uuid> = rows[0].try_get("", "actor_user_id").unwrap();
            assert_eq!(actor, Some(root.id));
            runtime.database.close().await.unwrap();
        })
        .await
        .unwrap();
    }
}
