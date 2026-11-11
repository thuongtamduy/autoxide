//! Chạy máy chủ và tắt cho êm (graceful shutdown).

use std::future::{Future, IntoFuture};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use tokio::net::TcpListener;
use tokio::sync::Notify;

/// Máy chủ đã dừng như thế nào.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shutdown {
    /// Mọi request đang chạy đã xong trước khi dừng.
    Graceful,
    /// Hết thời gian chờ: các request còn dở bị cắt.
    TimedOut,
}

/// Phục vụ `router` trên `listener` cho tới khi `shutdown` hoàn thành.
///
/// Khi `shutdown` hoàn thành: ngừng nhận kết nối mới, chờ các request đang chạy
/// xong — nhưng không quá `grace`. Nhờ vậy lúc deploy bản mới, request đang dở
/// không bị đứt giữa chừng, mà tiến trình cũng không treo mãi vì một request kẹt.
pub async fn serve<F>(
    listener: TcpListener,
    router: Router,
    shutdown: F,
    grace: Duration,
) -> std::io::Result<Shutdown>
where
    F: Future<Output = ()> + Send + 'static,
{
    // `Notify`: một chiếc chuông. Việc tắt bắt đầu thì rung chuông, để nhánh đếm
    // thời gian chờ bên dưới biết lúc bắt đầu đếm.
    let shutdown_started = Arc::new(Notify::new());
    let bell = Arc::clone(&shutdown_started);
    let server = axum::serve(listener, router).with_graceful_shutdown(async move {
        shutdown.await;
        bell.notify_one();
    });

    // `select!` chạy hai nhánh cùng lúc và lấy nhánh xong trước.
    tokio::select! {
        result = server.into_future() => result.map(|()| Shutdown::Graceful),
        () = async {
            shutdown_started.notified().await;
            tokio::time::sleep(grace).await;
        } => Ok(Shutdown::TimedOut),
    }
}

/// Hoàn thành khi tiến trình nhận tín hiệu dừng: Ctrl-C, hoặc SIGTERM (tín hiệu
/// Docker và systemd gửi khi dừng dịch vụ).
pub async fn shutdown_signal() {
    let interrupt = async {
        // Không đăng ký được tín hiệu thì coi như không bao giờ nhận: chờ mãi.
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = interrupt => {},
        () = terminate => {},
    }
}
