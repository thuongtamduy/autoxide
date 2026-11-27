//! Lệnh `autoxide serve`: chạy máy chủ HTTP tới khi nhận tín hiệu dừng.
//!
//! Mặc định lệnh này KHÔNG tự áp dụng migration: còn migration chưa áp dụng thì nó từ chối
//! chạy và nói phải chạy `autoxide migrate up` — production chạy lệnh đó một lần mỗi lần deploy.
//! `AUTOXIDE_MIGRATE_ON_START=true` (cho một máy chủ, chạy thử) thì tự migrate trước, dưới khoá
//! migration để nhiều bản khởi động cùng lúc không chạy chồng (`startup.rs`,
//! D-20261121-migrate-on-start).

use autoxide_kernel::time::Timestamp;
use std::future::Future;
use std::process::ExitCode;

use autoxide_kernel::http::{self, Shutdown};
use autoxide_kernel::tenancy::TenancyMode;
use tokio::net::TcpListener;

use super::{report_failure, report_invalid_configuration, run_with_observability};
use crate::runtime::{self, Migrations, Runtime};
use crate::settings::{self, Settings};
use crate::startup;

pub fn run() -> ExitCode {
    let settings = match settings::load() {
        Ok(settings) => settings,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    let config = settings.config.clone();
    let task = async move {
        for (name, nearest) in settings::unknown_variables() {
            tracing::warn!(
                variable = %name,
                did_you_mean = nearest.as_deref().unwrap_or(""),
                "this AUTOXIDE_ variable is not a setting and is ignored (run `autoxide config` for the list)"
            );
        }
        serve(settings, http::shutdown_signal()).await
    };
    match run_with_observability(&config, task) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => report_failure(&error),
    }
}

/// Khởi động chương trình rồi phục vụ tới khi `shutdown` hoàn thành.
pub async fn serve<F>(settings: Settings, shutdown: F) -> Result<(), String>
where
    F: Future<Output = ()> + Send + 'static,
{
    let (runtime, listeners) = prepare(settings).await?;
    run_server(runtime, listeners, shutdown).await
}

/// Các port đã mở: port của API, và port riêng của `/metrics` khi có.
struct Listeners {
    api: TcpListener,
    metrics: Option<TcpListener>,
}

/// Khởi động mọi thứ và mở port, nhưng chưa nhận request. Tách riêng để test biết
/// port thật (khi cấu hình port `0`, hệ điều hành chọn một port trống).
async fn prepare(settings: Settings) -> Result<(Runtime, Listeners), String> {
    let startup = settings.startup.clone();
    let prepared = startup::before_start(&settings).await?;
    let runtime = match runtime::start(settings, Migrations::RequireApplied).await {
        Ok(runtime) => runtime,
        Err(error) => {
            startup::release(prepared).await;
            return Err(error);
        }
    };
    crate::modules::log_at_start(&runtime.modules, &runtime.database).await;
    if let Err(error) = startup::after_start(&runtime, &startup, prepared).await {
        let _ = runtime.database.close().await;
        return Err(error);
    }
    let address = runtime.config.http.address;
    let api = match TcpListener::bind(address).await {
        Ok(listener) => listener,
        Err(error) => {
            let _ = runtime.database.close().await;
            return Err(format!("http: cannot listen on `{address}`: {error}"));
        }
    };
    let metrics = match runtime.config.http.metrics_address {
        Some(address) if runtime.metrics.is_some() => match TcpListener::bind(address).await {
            Ok(listener) => Some(listener),
            Err(error) => {
                let _ = runtime.database.close().await;
                return Err(format!(
                    "http: cannot listen on `{address}` (AUTOXIDE_HTTP_METRICS_ADDRESS): {error}"
                ));
            }
        },
        _ => None,
    };
    Ok((runtime, Listeners { api, metrics }))
}

/// Khoảng giữa hai lần kiểm migration của các placement.
const PLACEMENT_CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

/// Phục vụ tới khi `shutdown` hoàn thành, rồi đóng kết nối database.
async fn run_server<F>(runtime: Runtime, listeners: Listeners, shutdown: F) -> Result<(), String>
where
    F: Future<Output = ()> + Send + 'static,
{
    let Listeners { api: listener, metrics: metrics_listener } = listeners;
    // Tách `runtime` thành từng phần: router được chuyển hẳn (move) cho máy chủ,
    // còn database phải giữ lại để đóng sau khi máy chủ dừng.
    let Runtime { config, database, modules, api, dispatcher, worker, draining, metrics } = runtime;
    let address = listener.local_addr().map_err(|error| format!("http: {error}"))?;
    let role = config.process.role;
    tracing::info!(%address, modules = ?modules.names(), ?role, "listening");

    // Việc nền (giao event, chạy job) chạy cùng máy chủ; dừng SAU máy chủ, để việc của
    // các request cuối cùng vẫn được làm. Việc chưa làm nằm lại trong database, lần
    // chạy sau làm tiếp — không mất.
    let (stop_background, background_stopped) = tokio::sync::watch::channel(false);
    let mut background = tokio::task::JoinSet::new();
    // `/metrics` trên port riêng: phục vụ tới khi việc nền dừng (sau máy chủ API), để máy thu số
    // đo vẫn đọc được trong lúc rút.
    if let (Some(listener), Some(router)) = (metrics_listener, metrics) {
        if let Ok(address) = listener.local_addr() {
            tracing::info!(%address, "serving /metrics");
        }
        let mut stopped = background_stopped.clone();
        let grace = config.http.shutdown_grace;
        background.spawn(async move {
            let stop = async move {
                let _ = stopped.wait_for(|stop| *stop).await;
            };
            if let Err(error) = http::serve(listener, router, stop, grace).await {
                tracing::error!(error = %error, "the /metrics listener stopped");
            }
        });
    }
    if role.runs_background_work() {
        let mut stopped = background_stopped.clone();
        background.spawn(dispatcher.run(async move {
            let _ = stopped.wait_for(|stop| *stop).await;
        }));
        let mut stopped = background_stopped.clone();
        background.spawn(worker.run(async move {
            let _ = stopped.wait_for(|stop| *stop).await;
        }));
        // Gauge tồn đọng (bước 39): chỉ khi có `/metrics`, và chỉ ở tiến trình chạy việc nền —
        // tiến trình chỉ phục vụ API không đặt cùng gauge lần nữa (Prometheus sẽ thấy chuỗi trùng).
        if config.http.metrics {
            match modules.backlog_monitor() {
                Ok(monitor) => {
                    let monitor = monitor
                        .every(std::time::Duration::from_secs(
                            config.process.metrics_backlog_seconds,
                        ))
                        .max_database_labels(
                            usize::try_from(config.process.metrics_max_database_labels)
                                .unwrap_or(usize::MAX),
                        );
                    let mut stopped = background_stopped.clone();
                    background.spawn(monitor.run(async move {
                        let _ = stopped.wait_for(|stop| *stop).await;
                    }));
                }
                Err(error) => tracing::error!(error = %error, "cannot watch the queue backlog"),
            }
        }
        // Chế độ nhiều khách: kiểm migration của các placement mỗi phút (đặc tả 33).
        if let Some(registry) = modules.tenancy()
            && registry.mode() == TenancyMode::Multi
        {
            // Lỗi khai báo migration đã bị chặn lúc khởi động (`RequireApplied`); ở đây chỉ ghi log.
            match modules.placements(&registry) {
                Ok(placements) => {
                    let mut stopped = background_stopped.clone();
                    background.spawn(placements.watch(PLACEMENT_CHECK_EVERY, async move {
                        let _ = stopped.wait_for(|stop| *stop).await;
                    }));
                }
                Err(error) => tracing::error!(error = %error, "cannot watch the placements"),
            }
        }
    }

    // Số request API theo khách (bước 35, hạn mức `kernel.api_requests`): tiến trình phục vụ API
    // ghi gộp vào sổ lượng dùng mỗi phút, và một lần nữa khi dừng.
    if role.serves_api() {
        let entitlements = modules.entitlements();
        let mut stopped = background_stopped.clone();
        background.spawn(async move {
            let node = uuid::Uuid::now_v7().to_string();
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                // Chỉ chép giá trị `bool` ra: guard của kênh `watch` không được giữ qua `.await`.
                let stop = tokio::select! {
                    _ = tick.tick() => false,
                    changed = stopped.changed() => changed.is_err() || *stopped.borrow(),
                };
                entitlements.flush_usage(&node, Timestamp::now().as_unix_seconds() / 60).await;
                if stop {
                    break;
                }
            }
        });
    }

    // Tín hiệu dừng tới thì chưa ngừng nhận kết nối ngay: bật cờ để `/health/ready` trả 503, vẫn
    // phục vụ thêm `drain` để bộ cân bằng tải kịp chuyển request sang tiến trình khác. Hết khoảng
    // đó future này mới xong, và `http::serve` mới bắt đầu tắt (đếm `shutdown_grace` từ lúc ấy).
    warn_if_over_open_files(config.http.max_connections);
    let drain = config.http.drain;
    // Stream event (SSE) không tự kết thúc: đóng chúng ngay khi bắt đầu tắt, client nối lại sang
    // máy chủ khác bằng `Last-Event-ID` (bước 40 P5).
    let streams = modules.api::<autoxide_kernel::event::StreamHub>();
    let shutdown = async move {
        shutdown.await;
        draining.start();
        if let Some(streams) = &streams {
            streams.close();
        }
        if !drain.is_zero() {
            tracing::info!(drain_seconds = drain.as_secs(), "shutting down: draining");
            tokio::time::sleep(drain).await;
        }
    };
    let outcome = http::serve_with_limit(
        listener,
        api.router,
        shutdown,
        config.http.shutdown_grace,
        config.http.max_connections,
    )
    .await;
    match outcome {
        Ok(Shutdown::Graceful) => tracing::info!("stopped"),
        Ok(Shutdown::TimedOut) => tracing::warn!(
            grace_seconds = config.http.shutdown_grace.as_secs(),
            "stopped before every request finished; the remaining requests were cut off"
        ),
        Err(_) => {}
    }
    let _ = stop_background.send(true);
    while let Some(result) = background.join_next().await {
        if let Err(error) = result {
            tracing::error!(error = %error, "background work stopped abnormally");
        }
    }
    // Module sống tới khi máy chủ dừng hẳn: service của chúng nằm trong module.
    drop(modules);
    let closed = database.close().await.map_err(|error| format!("database: {error}"));
    outcome.map_err(|error| format!("http: {error}"))?;
    closed
}

// Chỗ dành cho file descriptor ngoài các kết nối HTTP (database, file log, cổng `/metrics`, file tải lên…).
const SPARE_FILES: usize = 64;

// Cảnh báo khi `AUTOXIDE_HTTP_MAX_CONNECTIONS` không chặn được trước giới hạn file mở của process (`ulimit -n`): khi
// đó hết file descriptor vẫn xảy ra trước, giới hạn của Autoxide không có tác dụng (L-10). Chỉ Linux có
// `/proc/self/limits`; nơi khác bỏ qua.
fn warn_if_over_open_files(max_connections: usize) {
    // `0`: người vận hành tắt giới hạn (để reverse proxy lo) — không có gì để so.
    if max_connections == 0 {
        return;
    }
    let Ok(limits) = std::fs::read_to_string("/proc/self/limits") else { return };
    if let Some(open_files) = soft_open_files(&limits)
        && over_open_files(max_connections, open_files)
    {
        tracing::warn!(
            max_connections,
            open_files,
            "AUTOXIDE_HTTP_MAX_CONNECTIONS is not below the open file limit (ulimit -n); raise the limit \
             (systemd LimitNOFILE, docker --ulimit nofile) or lower the setting so new connections wait \
             instead of failing"
        );
    }
}

// Giới hạn kết nối cộng chỗ dự phòng không thấp hơn giới hạn file mở.
fn over_open_files(max_connections: usize, open_files: usize) -> bool {
    max_connections.saturating_add(SPARE_FILES) > open_files
}

// Giới hạn mềm "Max open files" trong nội dung của `/proc/self/limits` (`unlimited` hay không đọc được: `None`).
fn soft_open_files(limits: &str) -> Option<usize> {
    let line = limits.lines().find(|line| line.starts_with("Max open files"))?;
    line.trim_start_matches("Max open files").split_whitespace().next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn doc_gioi_han_file_mo() {
        let limits = "Limit                     Soft Limit           Hard Limit           Units     \n\
                      Max processes             63000                63000                processes \n\
                      Max open files            1024                 524288               files     \n";
        assert_eq!(super::soft_open_files(limits), Some(1024));
        let unlimited =
            "Max open files            unlimited            unlimited            files\n";
        assert_eq!(super::soft_open_files(unlimited), None);
        assert_eq!(super::soft_open_files(""), None);
        assert!(super::over_open_files(4096, 1024));
        assert!(!super::over_open_files(4096, 8192));
        assert!(super::over_open_files(usize::MAX, 8192), "không tràn số");
    }

    use super::*;
    use std::io::{Read, Write};

    use autoxide_kernel::config::ConfigSource;
    use autoxide_kernel::database::Database;
    use autoxide_kernel::migration::prelude::*;
    use autoxide_kernel::module::Module;
    use autoxide_testkit::TestDatabase;

    struct CreateNote;

    impl MigrationName for CreateNote {
        fn name(&self) -> &str {
            "m0001_create_note"
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for CreateNote {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .create_table(
                    Table::create().table("notes_note").col(schema::pk_uuid("id")).to_owned(),
                )
                .await
        }

        async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager.drop_table(Table::drop().table("notes_note").to_owned()).await
        }
    }

    struct Notes;

    impl Module for Notes {
        fn name(&self) -> &'static str {
            "notes"
        }

        fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
            vec![Box::new(CreateNote)]
        }
    }

    /// Cấu hình trỏ tới database của test, máy chủ nghe ở port do hệ điều hành chọn.
    fn settings(database: &TestDatabase) -> Settings {
        let source = ConfigSource::from_pairs([
            ("DATABASE_URL", database.url()),
            ("AUTOXIDE_HTTP_ADDRESS", "127.0.0.1:0"),
        ]);
        settings::read_with(&source, vec![Box::new(Notes)]).unwrap()
    }

    /// Gửi `GET path` và trả về toàn bộ response dạng chữ. Dùng thư viện chuẩn
    /// trong một thread riêng: đủ cho một request, không cần thêm HTTP client.
    async fn get(address: std::net::SocketAddr, path: &str) -> String {
        let request = format!("GET {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n");
        tokio::task::spawn_blocking(move || {
            let mut stream = std::net::TcpStream::connect(address).unwrap();
            stream.write_all(request.as_bytes()).unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn con_migration_chua_ap_dung_thi_tu_choi_chay() {
        TestDatabase::run(async |database| {
            let error = serve(settings(database), std::future::ready(())).await.unwrap_err();

            assert!(error.contains("migration(s) have not been applied"), "{error}");
            assert!(error.contains("first: kernel/m0001_create_secrets"), "{error}");
            assert!(error.contains("and ") && error.contains(" more"), "{error}");
            assert!(error.contains("`autoxide migrate up`"), "{error}");
            assert!(error.len() < 400, "thông báo phải ngắn: {} byte", error.len());
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn phuc_vu_request_roi_dung_em_khi_nhan_tin_hieu() {
        TestDatabase::run(async |database| {
            let settings = settings(database);
            autoxide_testkit::prepare_from_template(
                &settings.config.database,
                &settings.modules.migrator().unwrap(),
            )
            .await;
            let db = Database::connect(&settings.config.database).await.unwrap();
            settings.modules.migrator().unwrap().up(&db).await.unwrap();
            db.close().await.unwrap();

            let (runtime, listeners) = prepare(settings).await.unwrap();
            let address = listeners.api.local_addr().unwrap();
            // `oneshot`: kênh gửi đúng một lần, đóng vai tín hiệu dừng.
            let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
            let server = tokio::spawn(run_server(runtime, listeners, async {
                let _ = stopped.await;
            }));

            let live = get(address, "/health/live").await;
            let ready = get(address, "/health/ready").await;
            stop.send(()).unwrap();

            assert!(live.starts_with("HTTP/1.1 200 OK\r\n"), "{live}");
            assert!(ready.starts_with("HTTP/1.1 200 OK\r\n"), "{ready}");
            assert_eq!(server.await.unwrap(), Ok(()));
        })
        .await
        .unwrap();
    }

    /// Tiến trình chỉ làm việc nền: không phục vụ API của module, vẫn trả health check.
    #[tokio::test]
    async fn vai_tro_worker_chi_co_health_check() {
        TestDatabase::run(async |database| {
            let source = ConfigSource::from_pairs([
                ("DATABASE_URL", database.url()),
                ("AUTOXIDE_HTTP_ADDRESS", "127.0.0.1:0"),
                ("AUTOXIDE_PROCESS_ROLE", "worker"),
            ]);
            let settings = settings::read_with(&source, vec![Box::new(Notes)]).unwrap();
            autoxide_testkit::prepare_from_template(
                &settings.config.database,
                &settings.modules.migrator().unwrap(),
            )
            .await;
            let db = Database::connect(&settings.config.database).await.unwrap();
            settings.modules.migrator().unwrap().up(&db).await.unwrap();
            db.close().await.unwrap();

            let (runtime, listeners) = prepare(settings).await.unwrap();
            let address = listeners.api.local_addr().unwrap();
            let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
            let server = tokio::spawn(run_server(runtime, listeners, async {
                let _ = stopped.await;
            }));

            let live = get(address, "/health/live").await;
            let docs = get(address, "/api/v1/openapi.json").await;
            stop.send(()).unwrap();

            assert!(live.starts_with("HTTP/1.1 200 OK\r\n"), "{live}");
            assert!(!docs.contains("/api/v1/notes"), "{docs}");
            assert_eq!(server.await.unwrap(), Ok(()));
        })
        .await
        .unwrap();
    }

    /// `AUTOXIDE_HTTP_METRICS_ADDRESS`: `/metrics` chỉ có ở port riêng, không có ở port của API
    /// (số đo không có xác thực). Test duy nhất của chương trình bật bộ thu số đo toàn tiến trình.
    #[tokio::test]
    async fn so_do_o_port_rieng_khong_o_port_api() {
        TestDatabase::run(async |database| {
            let source = ConfigSource::from_pairs([
                ("DATABASE_URL", database.url()),
                ("AUTOXIDE_HTTP_ADDRESS", "127.0.0.1:0"),
                ("AUTOXIDE_HTTP_METRICS", "true"),
                ("AUTOXIDE_HTTP_METRICS_ADDRESS", "127.0.0.1:0"),
            ]);
            let settings = settings::read_with(&source, vec![Box::new(Notes)]).unwrap();
            autoxide_testkit::prepare_from_template(
                &settings.config.database,
                &settings.modules.migrator().unwrap(),
            )
            .await;
            let db = Database::connect(&settings.config.database).await.unwrap();
            settings.modules.migrator().unwrap().up(&db).await.unwrap();
            db.close().await.unwrap();

            let (runtime, listeners) = prepare(settings).await.unwrap();
            let address = listeners.api.local_addr().unwrap();
            let metrics_address = listeners.metrics.as_ref().unwrap().local_addr().unwrap();
            let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
            let server = tokio::spawn(run_server(runtime, listeners, async {
                let _ = stopped.await;
            }));

            let on_api = get(address, "/metrics").await;
            let docs = get(address, "/api/v1/openapi.json").await;
            let scraped = get(metrics_address, "/metrics").await;
            let other = get(metrics_address, "/health/live").await;
            stop.send(()).unwrap();

            assert!(on_api.starts_with("HTTP/1.1 404"), "{on_api}");
            assert!(!docs.contains("\"/metrics\""), "{docs}");
            assert!(scraped.starts_with("HTTP/1.1 200 OK\r\n"), "{scraped}");
            assert!(scraped.contains("text/plain"), "{scraped}");
            assert!(other.starts_with("HTTP/1.1 404"), "{other}");
            assert_eq!(server.await.unwrap(), Ok(()));
        })
        .await
        .unwrap();
    }

    /// Trong khoảng rút: `/health/ready` trả 503 `shutting_down`, mà máy chủ vẫn nhận kết nối mới
    /// và vẫn sống; hết khoảng rút thì dừng êm.
    #[tokio::test]
    async fn khi_tat_rut_truoc_roi_moi_ngung_nhan_ket_noi() {
        TestDatabase::run(async |database| {
            let source = ConfigSource::from_pairs([
                ("DATABASE_URL", database.url()),
                ("AUTOXIDE_HTTP_ADDRESS", "127.0.0.1:0"),
                ("AUTOXIDE_HTTP_DRAIN_SECONDS", "2"),
            ]);
            let settings = settings::read_with(&source, vec![Box::new(Notes)]).unwrap();
            autoxide_testkit::prepare_from_template(
                &settings.config.database,
                &settings.modules.migrator().unwrap(),
            )
            .await;
            let db = Database::connect(&settings.config.database).await.unwrap();
            settings.modules.migrator().unwrap().up(&db).await.unwrap();
            db.close().await.unwrap();

            let (runtime, listeners) = prepare(settings).await.unwrap();
            let address = listeners.api.local_addr().unwrap();
            let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
            let server = tokio::spawn(run_server(runtime, listeners, async {
                let _ = stopped.await;
            }));
            assert!(get(address, "/health/ready").await.starts_with("HTTP/1.1 200 OK\r\n"));

            let started = std::time::Instant::now();
            stop.send(()).unwrap();
            // Cờ được bật trong task của máy chủ: chờ tới khi nó thấy tín hiệu.
            let mut ready = get(address, "/health/ready").await;
            while ready.starts_with("HTTP/1.1 200") && started.elapsed().as_secs() < 1 {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                ready = get(address, "/health/ready").await;
            }
            let live = get(address, "/health/live").await;

            assert!(ready.starts_with("HTTP/1.1 503 Service Unavailable\r\n"), "{ready}");
            assert!(ready.contains(r#""code":"shutting_down""#), "{ready}");
            assert!(live.starts_with("HTTP/1.1 200 OK\r\n"), "{live}");
            assert!(started.elapsed().as_secs() < 2, "kiểm xong trước khi hết khoảng rút");
            assert_eq!(server.await.unwrap(), Ok(()));
            assert!(
                started.elapsed() >= std::time::Duration::from_secs(2),
                "dừng trước khi rút xong"
            );
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn port_dang_bi_chiem_thi_bao_ro() {
        TestDatabase::run(async |database| {
            let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = taken.local_addr().unwrap().to_string();
            let source = ConfigSource::from_pairs([
                ("DATABASE_URL", database.url()),
                ("AUTOXIDE_HTTP_ADDRESS", address.as_str()),
            ]);
            // Không module nghiệp vụ nào, migration của module nội bộ `kernel` đã chạy: không có
            // migration chờ, lỗi duy nhất là port.
            let settings = settings::read_with(&source, Vec::new()).unwrap();
            let db = autoxide_kernel::database::Database::connect(&settings.config.database)
                .await
                .unwrap();
            settings.modules.migrator().unwrap().up(&db).await.unwrap();

            let error = serve(settings, std::future::ready(())).await.unwrap_err();

            assert!(error.starts_with(&format!("http: cannot listen on `{address}`: ")), "{error}");
        })
        .await
        .unwrap();
    }
}
