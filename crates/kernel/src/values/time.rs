//! Thời gian của Autoxide (D-20261121-time-library): [`Timestamp`] (một thời điểm, UTC) và [`Date`] (một ngày
//! theo lịch, không giờ), dựng trên `jiff`.
//!
//! Vì sao kernel tự bọc thay vì để module dùng thẳng kiểu của thư viện:
//!
//! - **Dạng lưu trên đĩa không đổi.** Trước đây cột thời gian ghi bằng `chrono` qua `sqlx`: SQLite lưu chuỗi
//!   `2026-11-21T10:00:00.123456+00:00` (phần lẻ 0, 3, 6 hoặc 9 chữ số, đuôi `+00:00`), Postgres lưu
//!   `timestamptz`. Bản `jiff` / `time` sẵn có của `sqlx` / `sea-query` ghi SQLite dạng khác (`…Z`, cắt số 0),
//!   mà SQLite so sánh thời gian như chuỗi — trộn hai dạng là sắp xếp và lọc theo khoảng sai. Kiểu ở đây tự mã
//!   hoá đúng dạng cũ (xem `sql.rs`), nên dữ liệu cũ và mới nằm chung một cột vẫn so sánh đúng.
//! - **JSON không đổi**: RFC 3339, UTC, đuôi `Z`, phần lẻ 0/3/6/9 chữ số — như `chrono` trả trước đây.
//! - **OpenAPI không đổi**: `utoipa` nhận ra kiểu theo TÊN (`Timestamp` → `string`, `date-time`; `Date` →
//!   `string`, `date`), không cần khai gì thêm ở từng trường.
//! - **Hai database như nhau**: [`Timestamp::now`] cắt về micro giây — độ chính xác của Postgres — nên giá trị
//!   vừa tạo trong bộ nhớ bằng đúng giá trị đọc lại từ database nào cũng vậy.
//!
//! Múi giờ, khoảng thời gian, tính lịch (thêm tháng, thứ trong tuần…) dùng thẳng `jiff` (`jiff::tz::TimeZone`,
//! `jiff::SignedDuration`, `jiff::civil`): không có gì phải lưu hay trả ra nên không cần bọc.
//!
//! Cầu nối tạm sang `chrono` (`From` hai chiều) giữ cho các module chuyển dần từng cái một; gỡ cùng `chrono`.

mod bridge;
mod date;
mod format;
mod sql;
mod timestamp;

#[cfg(test)]
mod tests;

use std::fmt;

pub use date::Date;
pub use timestamp::Timestamp;

/// Chuỗi không đọc được thành thời điểm hay ngày.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    input: String,
    expected: &'static str,
}

impl ParseError {
    fn new(input: &str, expected: &'static str) -> Self {
        // Giữ tối đa 64 ký tự của đầu vào: đủ để biết sai ở đâu, không chép cả thân request vào log.
        ParseError { input: input.chars().take(64).collect(), expected }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{}` is not {}", self.input, self.expected)
    }
}

impl std::error::Error for ParseError {}
