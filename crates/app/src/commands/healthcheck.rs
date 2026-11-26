//! Lệnh `autoxide healthcheck`: hỏi chính máy chủ đang chạy trên máy này có khoẻ không, để dùng làm `HEALTHCHECK` của image
//! (image không có shell, `curl` hay `wget`) và làm healthcheck của compose (G7).
//!
//! Chỉ dùng thư viện chuẩn: mở một kết nối TCP tới địa chỉ `AUTOXIDE_HTTP_ADDRESS` (địa chỉ "nghe mọi nơi" `0.0.0.0` /
//! `[::]` đổi thành loopback cùng họ địa chỉ), gửi một request HTTP/1.1 tới `/health/ready` (hay `/health/live`) rồi đọc dòng
//! trạng thái. Không cần runtime async, không chạm database, không in secret. Mã thoát: 0 khoẻ, 1 không khoẻ hay không kết nối
//! được, 2 cấu hình sai.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::process::ExitCode;
use std::time::Duration;

use super::{EXIT_FAILURE, report_invalid_configuration};
use crate::settings;

#[derive(Debug, clap::Args)]
pub struct Arguments {
    /// Check `/health/live` (the process is alive) instead of `/health/ready` (it can serve requests: the database answers).
    #[arg(long)]
    live: bool,

    /// Seconds to wait for the answer, 1 to 60.
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u64).range(1..=60))]
    timeout: u64,
}

pub fn run(arguments: &Arguments) -> ExitCode {
    let settings = match settings::load() {
        Ok(settings) => settings,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    let path = if arguments.live { "/health/live" } else { "/health/ready" };
    let target = loopback_of(settings.config.http.address);
    match probe(target, path, Duration::from_secs(arguments.timeout)) {
        Ok(200) => ExitCode::SUCCESS,
        Ok(status) => {
            eprintln!("autoxide: {path} on {target} answered {status}, not 200");
            ExitCode::from(EXIT_FAILURE)
        }
        Err(error) => {
            eprintln!("autoxide: {path} on {target}: {error}");
            ExitCode::from(EXIT_FAILURE)
        }
    }
}

/// Địa chỉ để hỏi: nghe mọi nơi (`0.0.0.0`, `[::]`) thì hỏi loopback cùng họ địa chỉ, còn lại giữ nguyên.
fn loopback_of(address: SocketAddr) -> SocketAddr {
    let ip = match address.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        other => other,
    };
    SocketAddr::new(ip, address.port())
}

/// Gửi `GET path` tới `target` và trả mã trạng thái HTTP. `timeout` áp cho kết nối, ghi và đọc.
fn probe(target: SocketAddr, path: &str, timeout: Duration) -> Result<u16, String> {
    let mut stream = TcpStream::connect_timeout(&target, timeout).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(timeout)).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(timeout)).map_err(|e| e.to_string())?;
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nUser-Agent: autoxide-healthcheck\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).map_err(|e| e.to_string())?;
    // Chỉ cần dòng trạng thái: đọc tới khi có `\r\n` hay tối đa 256 byte.
    let mut head = Vec::with_capacity(64);
    let mut chunk = [0u8; 64];
    while head.len() < 256 && !head.windows(2).any(|pair| pair == b"\r\n") {
        let read = stream.read(&mut chunk).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        head.extend_from_slice(&chunk[..read]);
    }
    status_of(&head).ok_or_else(|| "the answer is not HTTP".to_owned())
}

/// Mã trạng thái trong dòng đầu `HTTP/1.1 200 OK`.
fn status_of(head: &[u8]) -> Option<u16> {
    let line = head.split(|byte| *byte == b'\r').next()?;
    let line = std::str::from_utf8(line).ok()?;
    let mut parts = line.split(' ');
    if !parts.next()?.starts_with("HTTP/") {
        return None;
    }
    parts.next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// Máy chủ giả: trả đúng `answer` cho kết nối đầu tiên rồi thoát.
    fn serve_once(answer: &'static [u8]) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 512];
            let _ = stream.read(&mut request);
            let _ = stream.write_all(answer);
        });
        address
    }

    #[test]
    fn doc_ma_trang_thai() {
        assert_eq!(status_of(b"HTTP/1.1 200 OK\r\nx: y\r\n\r\n"), Some(200));
        assert_eq!(status_of(b"HTTP/1.1 503 Service Unavailable\r\n"), Some(503));
        assert_eq!(status_of(b"hello\r\n"), None);
        assert_eq!(status_of(b""), None);
    }

    #[test]
    fn dia_chi_nghe_moi_noi_doi_thanh_loopback() {
        let v4: SocketAddr = "0.0.0.0:8080".parse().unwrap();
        let v6: SocketAddr = "[::]:8080".parse().unwrap();
        let other: SocketAddr = "10.1.2.3:9000".parse().unwrap();
        assert_eq!(loopback_of(v4), "127.0.0.1:8080".parse().unwrap());
        assert_eq!(loopback_of(v6), "[::1]:8080".parse().unwrap());
        assert_eq!(loopback_of(other), other);
    }

    #[test]
    fn khoe_va_khong_khoe() {
        let ok = serve_once(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n");
        assert_eq!(probe(ok, "/health/ready", Duration::from_secs(2)), Ok(200));
        let draining = serve_once(b"HTTP/1.1 503 Service Unavailable\r\n\r\n");
        assert_eq!(probe(draining, "/health/ready", Duration::from_secs(2)), Ok(503));
    }

    #[test]
    fn khong_phai_http_va_khong_ket_noi_duoc() {
        let junk = serve_once(b"SSH-2.0-nope\r\n");
        assert!(probe(junk, "/health/ready", Duration::from_secs(2)).is_err());
        // Cổng đã đóng: kết nối bị từ chối.
        let closed = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        assert!(probe(closed, "/health/ready", Duration::from_secs(1)).is_err());
    }
}
