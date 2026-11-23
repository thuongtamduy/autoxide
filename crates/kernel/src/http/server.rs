//! Runs the server and shuts down gracefully.

use std::convert::Infallible;
use std::future::Future;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::Request;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto::Builder;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tower::ServiceExt as _;

/// How the server stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shutdown {
    /// Every in-flight request finished before stopping.
    Graceful,
    /// The wait timed out: the unfinished requests were cut off.
    TimedOut,
}

// Client phải gửi xong header của request trong khoảng này, không thì kết nối bị đóng. `axum::serve`
// không gắn timer cho hyper nên hyper bỏ qua timeout này: một client mở hàng nghìn kết nối, mỗi
// kết nối nhỏ giọt một byte header, giữ hết file descriptor mà rate limiting không thấy request nào
// (slowloris; rà soát bảo mật 2026-11). 30 giây là mặc định của hyper.
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(30);
// HTTP/2: ping kết nối im lặng, không trả lời trong `KEEP_ALIVE_TIMEOUT` thì đóng.
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(60);
const KEEP_ALIVE_TIMEOUT: Duration = Duration::from_secs(20);

/// Serves `router` on `listener` until `shutdown` completes.
///
/// When `shutdown` completes: stops accepting new connections and waits for in-flight requests
/// to finish - but no longer than `grace`. This way, when deploying a new version, in-flight requests
/// are not cut off midway, and the process also does not hang forever because of one stuck request.
///
/// A client must send a request's headers within 30 seconds, and idle HTTP/2 connections are
/// pinged, so slow or silent clients cannot hold connections open forever.
pub async fn serve<F>(
    listener: TcpListener,
    router: Router,
    shutdown: F,
    grace: Duration,
) -> std::io::Result<Shutdown>
where
    F: Future<Output = ()> + Send + 'static,
{
    serve_with(listener, router, shutdown, grace, HEADER_READ_TIMEOUT).await
}

// `serve` với timeout đọc header `header_timeout` (test dùng số nhỏ).
async fn serve_with<F>(
    listener: TcpListener,
    router: Router,
    shutdown: F,
    grace: Duration,
    header_timeout: Duration,
) -> std::io::Result<Shutdown>
where
    F: Future<Output = ()> + Send + 'static,
{
    // Hai kênh `watch` dùng như chuông (cách `axum::serve` làm): `stop` — bên nhận bị bỏ khi bắt đầu
    // tắt, mỗi kết nối thấy `closed()` thì đóng êm; `alive` — mỗi kết nối giữ một bên nhận, bỏ khi
    // xong, nên `closed()` của bên gửi là "mọi kết nối đã xong".
    let (stop_tx, stop_rx) = watch::channel(());
    let (alive_tx, alive_rx) = watch::channel(());
    let mut builder = Builder::new(TokioExecutor::new());
    builder.http1().timer(TokioTimer::new()).header_read_timeout(header_timeout);
    builder
        .http2()
        .timer(TokioTimer::new())
        .keep_alive_interval(KEEP_ALIVE_INTERVAL)
        .keep_alive_timeout(KEEP_ALIVE_TIMEOUT)
        // WebSocket qua HTTP/2 (RFC 8441), như `axum::serve`.
        .enable_connect_protocol();

    let mut shutdown = std::pin::pin!(shutdown);
    loop {
        tokio::select! {
            () = &mut shutdown => break,
            accepted = listener.accept() => {
                let (stream, remote) = match accepted {
                    Ok(accepted) => accepted,
                    Err(error) => {
                        // Lỗi của một kết nối (client bỏ giữa chừng) thì nhận tiếp ngay; lỗi khác (hết
                        // file descriptor…) thì chờ một chút để không quay vòng ăn CPU.
                        if !is_connection_error(&error) {
                            tracing::error!(error = %error, "cannot accept a connection");
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                        continue;
                    }
                };
                // `TCP_NODELAY`: gửi ngay từng mảnh của câu trả lời. Không bật thì header và thân
                // (vd file tải xuống) gửi thành hai mảnh vướng thuật toán Nagle + delayed ACK của
                // TCP, mỗi request chờ thêm ~40 ms dù máy chủ rảnh (đo được khi chạy tải, D-076).
                if let Err(error) = stream.set_nodelay(true) {
                    tracing::warn!(error = %error, "cannot set TCP_NODELAY on a connection");
                }
                let router = router.clone();
                // Mỗi request mang địa chỉ của kết nối (`ConnectInfo`), để rate limiting biết
                // client là ai — như `into_make_service_with_connect_info`.
                let service = hyper::service::service_fn(move |request: Request<hyper::body::Incoming>| {
                    let mut request = request.map(Body::new);
                    request.extensions_mut().insert(ConnectInfo(remote));
                    let router = router.clone();
                    async move { router.oneshot(request).await.map_err(|never: Infallible| never) }
                });
                let builder = builder.clone();
                let stop_tx = stop_tx.clone();
                let alive_rx = alive_rx.clone();
                tokio::spawn(async move {
                    let connection =
                        builder.serve_connection_with_upgrades(TokioIo::new(stream), service);
                    let mut connection = std::pin::pin!(connection);
                    let mut stopping = std::pin::pin!(stop_tx.closed());
                    let mut stopped = false;
                    loop {
                        tokio::select! {
                            result = connection.as_mut() => {
                                if let Err(error) = result {
                                    tracing::trace!(error = %error, "connection ended with an error");
                                }
                                break;
                            }
                            () = &mut stopping, if !stopped => {
                                stopped = true;
                                connection.as_mut().graceful_shutdown();
                            }
                        }
                    }
                    drop(alive_rx);
                });
            }
        }
    }

    // Ngừng nhận kết nối mới, báo các kết nối đang mở đóng êm, rồi chờ chúng xong — tối đa `grace`.
    drop(listener);
    drop(stop_rx);
    drop(alive_rx);
    tokio::select! {
        () = alive_tx.closed() => Ok(Shutdown::Graceful),
        () = tokio::time::sleep(grace) => Ok(Shutdown::TimedOut),
    }
}

// Lỗi chỉ của một kết nối (không phải của listener).
fn is_connection_error(error: &std::io::Error) -> bool {
    use std::io::ErrorKind;
    matches!(
        error.kind(),
        ErrorKind::ConnectionRefused | ErrorKind::ConnectionAborted | ErrorKind::ConnectionReset
    )
}

/// Completes when the process receives a stop signal: Ctrl-C, or SIGTERM (the signal
/// Docker and systemd send when stopping a service).
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // Rà soát bảo mật 2026-11: client gửi header nhỏ giọt bị đóng kết nối sau timeout; request
    // bình thường vẫn được trả lời.
    #[tokio::test]
    async fn client_gui_header_qua_cham_bi_dong_ket_noi() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = Router::new().route("/", axum::routing::get(|| async { "ok" }));
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(serve_with(
            listener,
            router,
            async move {
                let _ = stopped.await;
            },
            Duration::from_secs(5),
            Duration::from_millis(300),
        ));

        let mut normal = tokio::net::TcpStream::connect(address).await.unwrap();
        normal.write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await.unwrap();
        let mut answer = String::new();
        normal.read_to_string(&mut answer).await.unwrap();
        assert!(answer.starts_with("HTTP/1.1 200") && answer.ends_with("ok"), "{answer}");

        let mut slow = tokio::net::TcpStream::connect(address).await.unwrap();
        slow.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n").await.unwrap();
        let mut buffer = Vec::new();
        // Máy chủ đóng kết nối (có thể kèm 408) thay vì treo: đọc kết thúc trước 5 giây.
        let closed =
            tokio::time::timeout(Duration::from_secs(5), slow.read_to_end(&mut buffer)).await;
        assert!(closed.is_ok(), "the server closes a connection whose headers never finish");

        stop.send(()).unwrap();
        assert_eq!(server.await.unwrap().unwrap(), Shutdown::Graceful);
    }
}
