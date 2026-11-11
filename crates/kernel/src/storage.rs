//! Lưu file: trait chung để module nào cũng lưu và đọc được file mà không phụ thuộc
//! nơi lưu (ổ đĩa cục bộ, S3 hay dịch vụ tương thích S3).
//!
//! Một module (`files`) công bố `dyn Storage`; module cần lưu file lấy nó bằng
//! `context.require::<dyn Storage>()` (đăng ký SAU module `files`). Nội dung đi dưới
//! dạng luồng byte ([`ByteStream`]): file lớn không bao giờ nằm trọn trong bộ nhớ.

use std::pin::Pin;

use bytes::Bytes;
use futures_util::Stream;

use crate::error::Error;

/// Luồng byte của một file, đọc dần từng đoạn.
//
// `Pin<Box<dyn Stream…>>`: kiểu cụ thể của luồng khác nhau theo nơi lưu; đóng hộp để
// mọi cài đặt trả cùng một kiểu. `Pin`: luồng async có thể tự trỏ vào mình, nên không
// được di chuyển sau khi bắt đầu đọc.
pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, Error>> + Send>>;

/// Một file đọc ra: kích thước và nội dung.
pub struct StoredFile {
    /// Số byte.
    pub size: u64,
    pub content: ByteStream,
}

/// Nơi lưu file của chương trình. Khoá (`key`) là đường dẫn tương đối gồm chữ, số,
/// `-`, `_`, `.` và `/` (vd `notes/0193…`); module nên đặt tên module của mình ở đầu.
#[async_trait::async_trait]
pub trait Storage: Send + Sync {
    /// Ghi `content` vào `key` (đè nếu đã có) và trả số byte đã ghi. Luồng báo lỗi giữa
    /// chừng thì không có gì được ghi và lỗi đó được trả lại nguyên vẹn.
    async fn put(&self, key: &str, content: ByteStream) -> Result<u64, Error>;

    /// File ở `key`, hoặc `None` nếu không có.
    async fn get(&self, key: &str) -> Result<Option<StoredFile>, Error>;

    /// Xoá file ở `key`. Không có file thì coi như xong.
    async fn delete(&self, key: &str) -> Result<(), Error>;
}
