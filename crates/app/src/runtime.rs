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
    if let Err(error) = modules.start(&database) {
        // Đóng kết nối trước khi báo lỗi; lỗi khởi động quan trọng hơn lỗi lúc đóng.
        let _ = database.close().await;
        return Err(error.to_string());
    }
    let background = modules.dispatcher(&database).and_then(|dispatcher| {
        let worker = modules.worker(&database, config.process.job_concurrency)?;
        Ok((dispatcher, worker))
    });
    let (dispatcher, worker) = match background {
        Ok(background) => background,
        Err(error) => {
            let _ = database.close().await;
            return Err(error.to_string());
        }
    };
    let mut root = http::health_routes(database.clone());
    if config.http.metrics {
        // Bộ thu số đo là toàn tiến trình: bật một lần, ở đây.
        match Metrics::install() {
            Ok(metrics) => {
                metrics.spawn_upkeep();
                root = root.merge(http::metrics_routes(metrics));
            }
            Err(error) => {
                let _ = database.close().await;
                return Err(error.to_string());
            }
        }
    }
    // Tiến trình chỉ làm việc nền (`worker`) không phục vụ API của module, chỉ health
    // check (và số đo) để hệ thống vận hành theo dõi.
    let (routes, authenticator) = if config.process.role.serves_api() {
        // Bộ xác thực (nếu một module công bố) được gắn vào mọi request.
        (modules.routes(), modules.api::<dyn Authenticator>())
    } else {
        (Vec::new(), None)
    };
    let extras = http::Extras {
        authenticator,
        idempotency: modules.api::<dyn IdempotencyStore>(),
        translations: modules.translations().clone(),
    };
    let api = http::build(&config.http, root, routes, extras);
    Ok(Runtime { config, database, modules, api, dispatcher, worker })
}

/// Lỗi nếu database còn migration chưa áp dụng; thông báo nêu từng migration và
/// lệnh cần chạy.
async fn require_applied(modules: &Modules, database: &Database) -> Result<(), String> {
    let migrator = modules.migrator().map_err(|error| error.to_string())?;
    let pending = migrator.pending(database).await.map_err(|error| error.to_string())?;
    if pending.is_empty() {
        return Ok(());
    }
    let noun = if pending.len() == 1 { "migration has" } else { "migrations have" };
    let list: Vec<String> =
        pending.iter().map(|state| format!("{}/{}", state.module, state.name)).collect();
    Err(format!(
        "{} {noun} not been applied to the database: {}; run `autoxide migrate up` first",
        pending.len(),
        list.join(", ")
    ))
}
