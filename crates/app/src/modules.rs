//! Nơi DUY NHẤT đăng ký module vào chương trình.
//!
//! Thêm một module = thêm một dòng vào [`all`] (và một dòng phụ thuộc trong
//! `Cargo.toml` của crate này). Không chỗ nào khác trong core phải sửa.

//!
//! Module nào CHẠY ở một bản cài là việc của `AUTOXIDE_MODULES` (bước 70 P1): `settings.rs` đưa
//! danh sách này qua `Modules::select`. Nửa sau file là các câu báo cho người cài về lựa chọn đó
//! (log khởi động, `autoxide doctor`, lệnh chép / backup).

use autoxide_kernel::database::Database;
use autoxide_kernel::module::{DormantModule, Module, Modules};

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
        // Chat chăm sóc khách (bước 61): chuyển phiếu và trả lời tự động qua `dyn SupportIntake`,
        // `dyn AutoResponder` của `support` (đứng trước); chính sách theo nút qua `dyn OrgDirectory`,
        // thông báo qua `dyn Notifier`, ảnh qua `dyn Media` — đều đứng trước.
        Box::new(autoxide_conversations::module()),
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
        // Bộ nối hãng Giao Hàng Nhanh (bước 70 P7): ghi tên vào sổ hãng của `shipping`, nên đứng SAU nó.
        Box::new(autoxide_ghn::module()),
        // Khuyến mại (bước 43): cần `dyn Catalog` (biến thể, giá ngay trước khuyến mại, sàn giá),
        // nên đứng sau `catalog`; đọc cài đặt kế thừa qua `dyn OrgDirectory` nếu có. `orders`
        // (bước 45) đăng ký SAU nó để gọi `dyn Promotions`.
        Box::new(autoxide_promotions::module()),
        // Giỏ hàng, đặt hàng, đơn hàng (bước 45): gọi `catalog`, `inventory`, `shipping`, `promotions`,
        // `tax`, `ledger` qua `api` của chúng nên đứng sau cả sáu. Module sau (`marketplace`,
        // `customers`, `channels`, `affiliate`) cắm vào `dyn OrderExtensions` của nó.
        Box::new(autoxide_orders::module()),
        // Khách hàng và giữ chân khách (bước 47): nghe event của `orders` (hồ sơ, lịch sử mua) và cắm
        // nhóm khách vào `dyn OrderExtensions`, nên đứng SAU `orders`.
        Box::new(autoxide_customers::module()),
        // Báo cáo gộp của cây khách hàng (bước 38): nghe event mà mọi module khai số liệu
        // (`Module::facts`), cần `dyn Ledger` (tỉ giá) và `dyn OrgDirectory` (múi giờ), nên đứng
        // CUỐI CÙNG.
        Box::new(autoxide_reporting::module()),
        // Tự động hoá quy trình (bước 58): nghe event của mọi module và gọi tool MCP của mọi module
        // qua sổ tool của kernel, nên đứng CUỐI CÙNG (sau cả `reporting`).
        Box::new(autoxide_automations::module()),
    ]
}

/// Hai module nội bộ của kernel luôn chạy; không liệt kê cho người cài.
const BUILT_IN: [&str; 2] = ["kernel", "antispam"];

/// Một câu nói module nào đang chạy, module nào tắt — cho log khởi động và `autoxide doctor`.
pub fn summary(modules: &Modules) -> String {
    let enabled: Vec<&str> =
        modules.names().into_iter().filter(|name| !BUILT_IN.contains(name)).collect();
    let mut text = format!("{} modules run: {}.", enabled.len(), enabled.join(", "));
    let inactive = modules.inactive();
    if inactive.is_empty() {
        text.push_str(" No module is turned off.");
    } else {
        let listed: Vec<String> = inactive
            .iter()
            .map(|module| format!("{} ({})", module.name, module.stability))
            .collect();
        text.push_str(&format!(" Turned off: {}.", listed.join(", ")));
    }
    for (module, needed_by) in modules.implied() {
        text.push_str(&format!(" `{module}` runs because `{needed_by}` depends on it."));
    }
    text
}

/// Module đang tắt mà `database` còn DỮ LIỆU của nó (bảng có dòng). Bảng trống không tính: bản
/// cài trước 0.18 tạo bảng cho mọi module, kể cả module chưa ai dùng.
pub async fn dormant_with_data(
    modules: &Modules,
    database: &Database,
) -> Result<Vec<DormantModule>, String> {
    // Đã khởi động thì xem cả database của từng placement (khách có database riêng); chưa thì chỉ
    // có database điều khiển để xem.
    let dormant = if modules.tenancy().is_some() {
        let (dormant, unreachable) =
            modules.dormant_everywhere().await.map_err(|error| format!("database: {error}"))?;
        if !unreachable.is_empty() {
            tracing::warn!(
                placements = %unreachable.join(", "),
                "cannot reach these placements to check for data of modules that are turned off"
            );
        }
        dormant
    } else {
        modules.dormant(database).await.map_err(|error| format!("database: {error}"))?
    };
    Ok(dormant.into_iter().filter(|module| !module.tables_with_data.is_empty()).collect())
}

/// Câu nói module nào tắt mà còn dữ liệu, và dòng cấu hình bật lại chúng.
pub fn dormant_notice(modules: &Modules, dormant: &[DormantModule]) -> (String, String) {
    let names: Vec<&str> = dormant.iter().map(|module| module.name).collect();
    let listed: Vec<String> = dormant
        .iter()
        .map(|module| {
            let mut text =
                format!("{} ({} table(s) with rows", module.name, module.tables_with_data.len());
            if !module.placements.is_empty() {
                text.push_str(&format!("; also in placement {}", module.placements.join(", ")));
            }
            text.push(')');
            text
        })
        .collect();
    let message = format!(
        "The database holds data of modules that are turned off: {}. Their endpoints, MCP tools, \
         jobs and events do not run; the data is kept untouched.",
        listed.join(", ")
    );
    let hint = format!(
        "To keep using them, set `{}` and restart (experimental modules are no longer on by \
         default; see docs/MODULES.md).",
        modules.setting_enabling(&names)
    );
    (message, hint)
}

/// Ghi vào log lúc `serve` khởi động: module nào chạy, module nào tắt, và — điều người nâng cấp
/// không được bỏ lỡ — module tắt mà database còn dữ liệu của nó, kèm đúng dòng cấu hình cần thêm.
pub async fn log_at_start(modules: &Modules, database: &Database) {
    tracing::info!(summary = %summary(modules), "modules");
    match dormant_with_data(modules, database).await {
        Ok(dormant) if dormant.is_empty() => {}
        Ok(dormant) => {
            let (message, hint) = dormant_notice(modules, &dormant);
            // Người cài tự viết `AUTOXIDE_MODULES` thì đã chọn; mặc định thì có thể họ chưa biết.
            if modules.selection_is_explicit() {
                tracing::info!(hint, "{message}");
            } else {
                tracing::warn!(hint, "{message}");
            }
        }
        // Không đọc được catalog (vd quyền): không chặn khởi động vì một câu nhắc.
        Err(error) => tracing::warn!(error, "cannot check for data of modules that are turned off"),
    }
}

/// Lỗi nếu `database` còn dữ liệu của module đang tắt: lệnh chép dữ liệu (`copy`, `backup`) chỉ
/// thấy bảng của module đang bật, nên chạy tiếp sẽ để lại dữ liệu đó trong im lặng.
pub async fn refuse_dormant_data(
    modules: &Modules,
    database: &Database,
    command: &str,
) -> Result<(), String> {
    let dormant = dormant_with_data(modules, database).await?;
    if dormant.is_empty() {
        return Ok(());
    }
    let names: Vec<&str> = dormant.iter().map(|module| module.name).collect();
    Err(format!(
        "the database holds data of modules that are turned off ({}), and `autoxide {command}` \
         only handles the modules that are on, so that data would be left out. Run the command \
         with `{}` (or `AUTOXIDE_MODULES=all`).",
        names.join(", "),
        modules.setting_enabling(&names)
    ))
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
            autoxide_testkit::prepare_from_template(
                &settings.config.database,
                &settings.modules.migrator().unwrap(),
            )
            .await;
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
            autoxide_testkit::prepare_from_template(
                &settings.config.database,
                &settings.modules.migrator().unwrap(),
            )
            .await;
            let db = Database::connect(&settings.config.database).await.unwrap();
            settings.modules.migrator().unwrap().up(&db).await.unwrap();
            settings.modules.start(&db).unwrap();
            // Số lịch đếm từ chính các module đã đăng ký (không viết tay: hai PR cùng thêm lịch sẽ
            // cùng sửa một con số và git gộp im lặng sai). Mỗi worker nhận tối đa `concurrency` job
            // của một module mỗi lượt: lấy bằng số lịch của module nhiều lịch nhất để một lượt đủ chỗ.
            let counts = settings.modules.schedule_counts();
            let total: usize = counts.iter().map(|(_, count)| count).sum();
            let most = counts.iter().map(|(_, count)| *count).max().unwrap_or(1);
            assert!(total > 0, "{counts:?}");
            let concurrency = u64::try_from(most).unwrap();
            let first = settings.modules.worker(concurrency).unwrap();
            let second = settings.modules.worker(concurrency).unwrap();

            let (a, b) = tokio::join!(first.run_once(), second.run_once());

            // Mỗi lịch của mọi module tới hạn ở lượt đầu: hai worker cùng chạy mà mỗi lịch đúng một
            // job.
            assert_eq!(a.unwrap() + b.unwrap(), total, "{counts:?}");
            let done =
                first.list("auth", autoxide_kernel::job::JobStatus::Completed, 10).await.unwrap();
            assert_eq!(done.len(), 1);
            assert_eq!(done[0].kind, "auth.cleanup");
            db.close().await.unwrap();
        })
        .await
        .unwrap();
    }

    /// Đầu-cuối: chương trình thật, database thật (gate chạy cả hai), client MCP (đặc tả 2026-07-28).
    /// Quản trị viên đăng nhập, gọi tool ghi qua MCP; thay đổi vào audit log, ghi đúng
    /// người làm.
    #[tokio::test]
    async fn client_mcp_goi_tool_ghi_va_vao_audit_log() {
        use autoxide_auth::api::{Accounts, NewAccount};
        use autoxide_testkit::mcp::{CallToolRequestParams, McpClient};
        use sea_orm::{ConnectionTrait, Statement};
        use serde_json::json;
        use tower::ServiceExt as _;

        TestDatabase::run(async |database| {
            let source = ConfigSource::from_pairs([("DATABASE_URL", database.url())]);
            let settings = crate::settings::read(&source).unwrap();
            autoxide_testkit::prepare_from_template(
                &settings.config.database,
                &settings.modules.migrator().unwrap(),
            )
            .await;
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

            // Client MCP của testkit gửi thẳng vào router của chương trình (đủ mọi lớp HTTP: xác thực,
            // Problem, nhật ký…), không cần mở cổng TCP.
            let client = McpClient::new(runtime.api.router.clone(), "/mcp", Some(&token));

            let tools = client.list_all_tools().await.unwrap();
            let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
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
