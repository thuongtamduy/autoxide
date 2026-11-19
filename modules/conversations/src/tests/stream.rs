//! Thời gian thực qua stream SSE của kernel (D-20261119-step61-stream-audience): người mua nhận event
//! của cuộc trò chuyện của mình, người khác thì không; nhân viên nhánh khác không nhận.

use std::time::Duration;

use autoxide_testkit::{TestApp, TestDatabase};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use tower::ServiceExt as _;

use super::{harness_with, reply, start};

struct Stream {
    body: Body,
    buffer: String,
}

impl Stream {
    async fn open(app: &TestApp, token: &str) -> Stream {
        let request = Request::get("/api/v1/kernel/events/stream?names=conversations.*")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let response = app.router().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "stream of {token}");
        Stream { body: response.into_body(), buffer: String::new() }
    }

    /// Tên các event nhận được trong `wait`.
    async fn names(&mut self, wait: Duration) -> Vec<String> {
        let deadline = tokio::time::Instant::now() + wait;
        let mut names = Vec::new();
        loop {
            while let Some(end) = self.buffer.find("\n\n") {
                let block: String = self.buffer.drain(..end + 2).collect();
                for line in block.lines() {
                    if let Some(name) = line.strip_prefix("event: ") {
                        names.push(name.to_owned());
                    }
                    // Không bao giờ lộ nội dung tin hay danh sách người nhận.
                    assert!(!line.contains("Bí mật") && !line.contains("audience"), "{line}");
                }
            }
            let Ok(Some(Ok(frame))) = tokio::time::timeout_at(deadline, self.body.frame()).await
            else {
                return names;
            };
            if let Ok(data) = frame.into_data() {
                self.buffer.push_str(&String::from_utf8_lossy(&data));
            }
        }
    }
}

#[tokio::test]
async fn stream_chi_phat_cho_nguoi_co_quyen() {
    TestDatabase::run(async |test| {
        let h = harness_with(test, false, |builder| {
            builder
                .setting("AUTOXIDE_EVENTS_STREAM", "true")
                .setting("AUTOXIDE_EVENTS_STREAM_POLL_MS", "100")
        })
        .await;
        let id = start(&h.app, "buyer", "Bí mật").await;
        let mut buyer = Stream::open(&h.app, "buyer").await;
        let mut other = Stream::open(&h.app, "other").await;
        let mut agent = Stream::open(&h.app, "agent").await;
        let mut branch = Stream::open(&h.app, "branch").await;
        reply(&h.app, "agent2", &id, "Bí mật trả lời").await;
        let got = buyer.names(Duration::from_secs(3)).await;
        assert!(got.contains(&"conversations.message_created".to_owned()), "{got:?}");
        assert!(agent.names(Duration::from_millis(500)).await.contains(&"conversations.message_created".to_owned()));
        assert!(other.names(Duration::from_millis(500)).await.is_empty());
        assert!(branch.names(Duration::from_millis(500)).await.is_empty());
    })
    .await
    .unwrap();
}
