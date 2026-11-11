//! Dựng chương trình đang chạy từ cấu hình đã kiểm: kết nối database, khởi động
//! các module, ghép router. Mọi lệnh cần "cả hệ thống" (`serve`, `openapi`) đi qua
//! [`start`], nên chúng thấy đúng cùng một hệ thống.

use autoxide_kernel::config::Config;
use autoxide_kernel::database::Database;
use autoxide_kernel::http::{self, Api};
use autoxide_kernel::metrics::Metrics;
use autoxide_kernel::module::Modules;

use crate::settings::Settings;

/// Chương trình đã khởi động xong, sẵn sàng phục vụ.
pub struct Runtime {
    #[expect(dead_code, reason = "used by `serve`, the last item of roadmap step 1")]
    pub config: Config,
    pub database: Database,
    /// Giữ các module sống cùng chương trình: chúng sở hữu service của mình.
    #[expect(dead_code, reason = "modules will own background work (events, jobs) in later steps")]
    pub modules: Modules,
    pub api: Api,
}

/// Kết nối database, khởi động từng module, rồi ghép router và tài liệu OpenAPI.
pub async fn start(settings: Settings) -> Result<Runtime, String> {
    let Settings { config, mut modules, .. } = settings;
    let database = Database::connect(&config.database).await.map_err(|error| error.to_string())?;
    if let Err(error) = modules.start(&database) {
        // Đóng kết nối trước khi báo lỗi; lỗi khởi động quan trọng hơn lỗi lúc đóng.
        let _ = database.close().await;
        return Err(error.to_string());
    }
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
    let api = http::build(&config.http, root, modules.routes());
    Ok(Runtime { config, database, modules, api })
}
