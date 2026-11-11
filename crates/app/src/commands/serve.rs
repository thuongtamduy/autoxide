//! Lệnh `autoxide serve`: chạy máy chủ HTTP tới khi nhận tín hiệu dừng.
//!
//! Lệnh này KHÔNG tự áp dụng migration: còn migration chưa áp dụng thì nó từ chối
//! chạy và nói phải chạy `autoxide migrate up`. Tách hai việc ra để lúc deploy
//! nhiều bản cùng lúc, không có hai tiến trình tranh nhau sửa schema.

use std::future::Future;
use std::process::ExitCode;

use autoxide_kernel::http::{self, Shutdown};
use tokio::net::TcpListener;

use super::{report_failure, report_invalid_configuration, run_with_observability};
use crate::runtime::{self, Migrations, Runtime};
use crate::settings::{self, Settings};

pub fn run() -> ExitCode {
    let settings = match settings::load() {
        Ok(settings) => settings,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    let config = settings.config.clone();
    match run_with_observability(&config, serve(settings, http::shutdown_signal())) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => report_failure(&error),
    }
}

/// Khởi động chương trình rồi phục vụ tới khi `shutdown` hoàn thành.
pub async fn serve<F>(settings: Settings, shutdown: F) -> Result<(), String>
where
    F: Future<Output = ()> + Send + 'static,
{
    let (runtime, listener) = prepare(settings).await?;
    run_server(runtime, listener, shutdown).await
}

/// Khởi động mọi thứ và mở port, nhưng chưa nhận request. Tách riêng để test biết
/// port thật (khi cấu hình port `0`, hệ điều hành chọn một port trống).
async fn prepare(settings: Settings) -> Result<(Runtime, TcpListener), String> {
    let runtime = runtime::start(settings, Migrations::RequireApplied).await?;
    let address = runtime.config.http.address;
    match TcpListener::bind(address).await {
        Ok(listener) => Ok((runtime, listener)),
        Err(error) => {
            let _ = runtime.database.close().await;
            Err(format!("http: cannot listen on `{address}`: {error}"))
        }
    }
}

/// Phục vụ tới khi `shutdown` hoàn thành, rồi đóng kết nối database.
async fn run_server<F>(runtime: Runtime, listener: TcpListener, shutdown: F) -> Result<(), String>
where
    F: Future<Output = ()> + Send + 'static,
{
    // Tách `runtime` thành từng phần: router được chuyển hẳn (move) cho máy chủ,
    // còn database phải giữ lại để đóng sau khi máy chủ dừng.
    let Runtime { config, database, modules, api, dispatcher, worker } = runtime;
    let address = listener.local_addr().map_err(|error| format!("http: {error}"))?;
    let role = config.process.role;
    tracing::info!(%address, modules = ?modules.names(), ?role, "listening");

    // Việc nền (giao event, chạy job) chạy cùng máy chủ; dừng SAU máy chủ, để việc của
    // các request cuối cùng vẫn được làm. Việc chưa làm nằm lại trong database, lần
    // chạy sau làm tiếp — không mất.
    let (stop_background, background_stopped) = tokio::sync::watch::channel(false);
    let mut background = tokio::task::JoinSet::new();
    if role.runs_background_work() {
        let mut stopped = background_stopped.clone();
        background.spawn(dispatcher.run(async move {
            let _ = stopped.wait_for(|stop| *stop).await;
        }));
        let mut stopped = background_stopped;
        background.spawn(worker.run(async move {
            let _ = stopped.wait_for(|stop| *stop).await;
        }));
    }

    let outcome = http::serve(listener, api.router, shutdown, config.http.shutdown_grace).await;
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

#[cfg(test)]
mod tests {
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

            assert_eq!(
                error,
                "1 migration has not been applied to the database: notes/m0001_create_note; run \
                 `autoxide migrate up` first"
            );
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn phuc_vu_request_roi_dung_em_khi_nhan_tin_hieu() {
        TestDatabase::run(async |database| {
            let settings = settings(database);
            let db = Database::connect(&settings.config.database).await.unwrap();
            settings.modules.migrator().unwrap().up(&db).await.unwrap();
            db.close().await.unwrap();

            let (runtime, listener) = prepare(settings).await.unwrap();
            let address = listener.local_addr().unwrap();
            // `oneshot`: kênh gửi đúng một lần, đóng vai tín hiệu dừng.
            let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
            let server = tokio::spawn(run_server(runtime, listener, async {
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
            let db = Database::connect(&settings.config.database).await.unwrap();
            settings.modules.migrator().unwrap().up(&db).await.unwrap();
            db.close().await.unwrap();

            let (runtime, listener) = prepare(settings).await.unwrap();
            let address = listener.local_addr().unwrap();
            let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
            let server = tokio::spawn(run_server(runtime, listener, async {
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

    #[tokio::test]
    async fn port_dang_bi_chiem_thi_bao_ro() {
        TestDatabase::run(async |database| {
            let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = taken.local_addr().unwrap().to_string();
            let source = ConfigSource::from_pairs([
                ("DATABASE_URL", database.url()),
                ("AUTOXIDE_HTTP_ADDRESS", address.as_str()),
            ]);
            // Không module nào: không có migration chờ, lỗi duy nhất là port.
            let settings = settings::read_with(&source, Vec::new()).unwrap();

            let error = serve(settings, std::future::ready(())).await.unwrap_err();

            assert!(error.starts_with(&format!("http: cannot listen on `{address}`: ")), "{error}");
        })
        .await
        .unwrap();
    }
}
