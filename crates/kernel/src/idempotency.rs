//! Idempotency key: gửi lại một lệnh ghi (vì mạng chập chờn, vì AI agent retry) không
//! làm hai lần (D-058).
//!
//! Client gửi header `Idempotency-Key: <chuỗi do client chọn>` kèm `POST`, `PUT`,
//! `PATCH` hay `DELETE` (theo bản nháp IETF "The Idempotency-Key HTTP Header Field").
//! Lần đầu: request chạy bình thường, response được lưu. Gửi lại cùng key (cùng người
//! gọi, cùng request): nhận lại đúng response đã lưu, kèm `Idempotent-Replayed: true`,
//! handler không chạy lại. Cùng key mà request khác: 422. Lần đầu chưa xong: 409.
//!
//! Lưu trữ nằm sau trait [`IdempotencyStore`], do một module cung cấp (kernel không có
//! bảng); lớp HTTP ở `http::idempotency`.

use chrono::{DateTime, Utc};

use crate::error::Error;

/// Response đã lưu của một lệnh ghi.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StoredResponse {
    pub status: u16,
    /// Các header cần trả lại y hệt (`content-type`, `location`…).
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl StoredResponse {
    /// A saved response: status code, headers to send back as they were, and body.
    pub const fn new(status: u16, headers: Vec<(String, String)>, body: Vec<u8>) -> Self {
        StoredResponse { status, headers, body }
    }
}

/// Kết quả khi bắt đầu một request mang idempotency key.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Begin {
    /// Lần đầu: chạy request, rồi gọi [`IdempotencyStore::complete`] (hay `abandon`)
    /// với `id` này.
    Started { id: uuid::Uuid },
    /// Đã có kết quả: trả lại nó.
    Replay(StoredResponse),
    /// Lần đầu vẫn đang chạy.
    InProgress,
    /// Cùng key, nhưng request khác lần đầu.
    Mismatch,
}

/// Nơi lưu idempotency key. `scope` là người gọi (băm), `fingerprint` là request (băm).
#[async_trait::async_trait]
pub trait IdempotencyStore: Send + Sync {
    /// Ghi nhận request; giữ quyền chạy tới `locked_until`.
    async fn begin(
        &self,
        scope: &str,
        key: &str,
        fingerprint: &str,
        locked_until: DateTime<Utc>,
    ) -> Result<Begin, Error>;

    /// Lưu response của lần chạy `id`.
    async fn complete(&self, id: uuid::Uuid, response: StoredResponse) -> Result<(), Error>;

    /// Lần chạy `id` hỏng phía máy chủ: bỏ ghi nhận, để client gửi lại được.
    async fn abandon(&self, id: uuid::Uuid) -> Result<(), Error>;
}
