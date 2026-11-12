//! Lưu file: trait chung để module nào cũng lưu và đọc được file mà không phụ thuộc
//! nơi lưu (ổ đĩa cục bộ, S3 hay dịch vụ tương thích S3).
//!
//! Một module (`files`) công bố `dyn Storage`; module cần lưu file lấy nó bằng
//! `context.require::<dyn Storage>()` (đăng ký SAU module `files`). Nội dung đi dưới
//! dạng luồng byte ([`ByteStream`]): file lớn không bao giờ nằm trọn trong bộ nhớ.

use std::ops::Range;
use std::pin::Pin;

use bytes::Bytes;
use futures_util::{Stream, StreamExt as _};

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

    /// Khoá của mọi file đang lưu, theo thứ tự tên (dùng cho backup).
    async fn keys(&self) -> Result<Vec<String>, Error>;

    /// Đoạn byte `range` (`start..end`, không gồm `end`) của file ở `key`, hoặc `None` nếu
    /// không có file. `size` của kết quả là số byte của đoạn. Bên gọi bảo đảm
    /// `start < end <= kích thước file` (vd từ header `Range` đã kiểm).
    ///
    /// Cài đặt mặc định đọc từ đầu file rồi bỏ phần ngoài đoạn — đúng nhưng tốn; nơi lưu
    /// đọc được từng đoạn (ổ đĩa, S3) nên cài lại.
    async fn get_range(&self, key: &str, range: Range<u64>) -> Result<Option<StoredFile>, Error> {
        let Some(file) = self.get(key).await? else {
            return Ok(None);
        };
        let size = range.end.min(file.size).saturating_sub(range.start);
        Ok(Some(StoredFile { size, content: slice(file.content, range) }))
    }

    /// Khoá bắt đầu bằng `prefix` (vd `media/`), theo thứ tự tên. Module dùng để dọn file
    /// của riêng mình mà không phải đọc khoá của module khác.
    ///
    /// Cài đặt mặc định lọc từ [`keys`](Self::keys); nơi lưu liệt kê được theo tiền tố nên
    /// cài lại.
    async fn keys_with_prefix(&self, prefix: &str) -> Result<Vec<String>, Error> {
        let mut keys = self.keys().await?;
        keys.retain(|key| key.starts_with(prefix));
        Ok(keys)
    }
}

/// Chỉ giữ các byte trong `range` của luồng `content` (đọc từ đầu file).
pub fn slice(content: ByteStream, range: Range<u64>) -> ByteStream {
    let Range { start, end } = range;
    content
        // `scan` mang theo vị trí đã đọc; trả `None` thì luồng dừng (đã qua `end`, khỏi
        // đọc tiếp phần còn lại của file).
        .scan(0u64, move |position, chunk| {
            let item = match chunk {
                Err(error) => Some(Err(error)),
                Ok(bytes) => {
                    let chunk_start = *position;
                    let length = bytes.len() as u64;
                    *position += length;
                    if chunk_start >= end {
                        None
                    } else {
                        let from = start.saturating_sub(chunk_start).min(length);
                        let to = (end - chunk_start).min(length);
                        // `from`, `to` <= `length` (một `usize`), đổi kiểu không mất gì.
                        Some(Ok(bytes.slice(from as usize..to as usize)))
                    }
                }
            };
            std::future::ready(item)
        })
        .filter(|chunk| std::future::ready(!matches!(chunk, Ok(bytes) if bytes.is_empty())))
        .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::TryStreamExt as _;

    #[tokio::test]
    async fn cat_doan_qua_nhieu_khuc() {
        for (range, expected) in
            [(0..5, "hello"), (4..8, "o wo"), (6..9, "wor"), (9..12, "ld!"), (11..12, "!")]
        {
            let chunks =
                ["hello ", "wor", "ld!"].map(|text| Ok(Bytes::from_static(text.as_bytes())));
            let content = futures_util::stream::iter(chunks).boxed();
            let parts: Vec<Bytes> = slice(content, range.clone()).try_collect().await.unwrap();
            assert_eq!(parts.concat(), expected.as_bytes(), "{range:?}");
        }
    }
}
