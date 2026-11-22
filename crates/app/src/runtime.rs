//! Dựng chương trình đang chạy từ cấu hình đã kiểm: kết nối database, khởi động
//! các module, ghép router. Mọi lệnh cần "cả hệ thống" (`serve`, `openapi`) đi qua
//! [`start`], nên chúng thấy đúng cùng một hệ thống.

use autoxide_kernel::access::Authenticator;
use autoxide_kernel::config::Config;
use autoxide_kernel::database::Database;
use autoxide_kernel::event::Dispatcher;
use autoxide_kernel::http::{self, Api};
use autoxide_kernel::idempotency::IdempotencyStore;
use autoxide_kernel::job::Worker;
use autoxide_kernel::metrics::Metrics;
use autoxide_kernel::module::Modules;
use autoxide_kernel::secret::SecretKey;

use crate::settings::Settings;

/// Chương trình đã khởi động xong, sẵn sàng phục vụ.
pub struct Runtime {
    pub config: Config,
    pub database: Database,
    /// Giữ các module sống cùng chương trình: chúng sở hữu service của mình.
    pub modules: Modules,
    pub api: Api,
    /// Bộ phát event; `serve` chạy nó nền, lệnh khác không cần.
    pub dispatcher: Dispatcher,
    /// Worker chạy job; `serve` chạy nó nền; `autoxide jobs` dùng nó để xem, chạy lại.
    pub worker: Worker,
    /// Cờ "đang tắt" mà `/health/ready` đọc; `serve` bật nó khi nhận tín hiệu dừng.
    pub draining: http::Draining,
    /// Router chỉ có `/metrics`, khi số đo có port riêng (`AUTOXIDE_HTTP_METRICS_ADDRESS`):
    /// `serve` mở port đó; `/metrics` khi ấy không có trên port của API.
    pub metrics: Option<axum::Router>,
}

/// Có đòi database đã áp dụng đủ migration trước khi khởi động không.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Migrations {
    /// Không kiểm: lệnh không chạm tới bảng của module (vd `openapi`).
    Ignore,
    /// Còn migration chưa áp dụng thì từ chối khởi động (vd `serve`): module chạy
    /// trên bảng chưa có hay bảng cũ sẽ hỏng giữa chừng, khó hiểu hơn nhiều.
    RequireApplied,
}

/// Kết nối database, khởi động từng module, rồi ghép router và tài liệu OpenAPI.
pub async fn start(settings: Settings, migrations: Migrations) -> Result<Runtime, String> {
    let Settings { config, mut modules, .. } = settings;
    let database = Database::connect(&config.database).await.map_err(|error| error.to_string())?;
    if migrations == Migrations::RequireApplied
        && let Err(error) = require_applied(&modules, &database).await
    {
        let _ = database.close().await;
        return Err(error);
    }
    // Sổ database: chế độ (`AUTOXIDE_TENANCY_MODE`) và các placement (bước 33).
    let registry = autoxide_kernel::tenancy::DatabaseRegistry::new(
        database.clone(),
        config.database.clone(),
        config.tenancy.clone(),
    );
    if let Err(error) = modules.start_in(&registry) {
        // Đóng kết nối trước khi báo lỗi; lỗi khởi động quan trọng hơn lỗi lúc đóng.
        let _ = database.close().await;
        return Err(error.to_string());
    }
    // Khoá ký nằm trong database: ai có bản sao database cũng tính ngược được dấu IP của khách
    // (D-153). Chỉ cảnh báo ở tiến trình phục vụ API — lệnh CLI không cần nghe điều này.
    if config.process.role.serves_api()
        && modules.api::<SecretKey>().is_some_and(|key| !key.is_configured())
    {
        tracing::warn!(
            "AUTOXIDE_SECRET_KEY is not set: the root key is kept in the database, so anyone with \
             a copy of the database (a backup) can open the secrets stored encrypted and work \
             visitors' IP blocks back out of the stored hashes. Set it (outside the database, \
             the same on every server) in production."
        );
    }
    let background = modules.dispatcher().and_then(|dispatcher| {
        let worker = modules
            .worker(config.process.job_concurrency)?
            .blocked_delay(std::time::Duration::from_secs(config.process.job_blocked_delay));
        Ok((dispatcher, worker))
    });
    let (dispatcher, worker) = match background {
        Ok(background) => background,
        Err(error) => {
            let _ = database.close().await;
            return Err(error.to_string());
        }
    };
    let draining = http::Draining::default();
    let mut root = http::health_routes(database.clone(), draining.clone());
    let mut metrics_router = None;
    if config.http.metrics {
        // Bộ thu số đo là toàn tiến trình: bật một lần, ở đây.
        match Metrics::install() {
            Ok(metrics) => {
                metrics.spawn_upkeep();
                if config.http.metrics_address.is_some() {
                    // Port riêng: số đo không có xác thực, nên không để chung port công khai.
                    let (router, _) = http::metrics_routes(metrics).split_for_parts();
                    metrics_router = Some(router);
                } else {
                    root = root.merge(http::metrics_routes(metrics));
                }
            }
            Err(error) => {
                let _ = database.close().await;
                return Err(error.to_string());
            }
        }
    }
    // Tiến trình chỉ làm việc nền (`worker`) không phục vụ API của module, chỉ health
    // check (và số đo) để hệ thống vận hành theo dõi.
    let (routes, authenticator, mcp) = if config.process.role.serves_api() {
        // Bộ xác thực (nếu một module công bố) được gắn vào mọi request.
        let mcp = modules
            .tools()
            .and_then(|tools| Ok((tools, modules.mcp_resources()?, modules.prompts()?)));
        let mcp = match mcp {
            Ok(mcp) => mcp,
            Err(error) => {
                let _ = database.close().await;
                return Err(error.to_string());
            }
        };
        (modules.routes(), modules.api::<dyn Authenticator>(), mcp)
    } else {
        (Vec::new(), None, (Vec::new(), Vec::new(), Vec::new()))
    };
    let (tools, resources, prompts) = mcp;
    let public_url = modules
        .api::<autoxide_kernel::public_url::PublicUrl>()
        .and_then(|url| url.base().ok().map(str::to_owned));
    let mut extras = http::Extras::default()
        .authenticator(authenticator)
        .idempotency(modules.api::<dyn IdempotencyStore>())
        .translations(modules.translations().clone())
        .deprecations(modules.deprecations())
        .experimental(modules.experimental())
        .body_limits(modules.body_limits())
        .entitlements(modules.entitlements())
        .sync(modules.sync_resources())
        .tools(tools)
        .resources(resources)
        .prompts(prompts)
        .modules(modules.descriptions())
        .public_url(public_url);
    if let Some(registry) = modules.tenancy() {
        extras = extras.tenancy(registry);
    }
    let api = http::build(&config.http, root, routes, extras);
    Ok(Runtime {
        config,
        database,
        modules,
        api,
        dispatcher,
        worker,
        draining,
        metrics: metrics_router,
    })
}

/// Lỗi nếu database còn migration chưa áp dụng; thông báo nêu số lượng, vài tên đầu và
/// lệnh cần chạy (`kernel.migrations_pending`).
pub(crate) async fn require_applied(modules: &Modules, database: &Database) -> Result<(), String> {
    let migrator = modules.migrator().map_err(|error| error.to_string())?;
    let pending = migrator.pending(database).await.map_err(|error| error.to_string())?;
    if pending.is_empty() {
        return Ok(());
    }
    Err(autoxide_kernel::migration::pending_error(&pending).message())
}
