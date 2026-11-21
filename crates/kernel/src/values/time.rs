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
//! **SQL viết tay:** giá trị ghi đi dưới dạng chuỗi kèm tên kiểu `timestamptz` / `date`; Postgres chỉ nhận khi
//! câu lệnh mang tên kiểu đó (`$1::"timestamptz"`). Builder của SeaORM / sea-query tự chèn; câu viết tay thì dựng
//! bằng `autoxide_kernel::database::statement` (luật `raw-statement` của `cargo xtask check` chặn
//! `Statement::from_sql_and_values`). Đừng tự viết `CAST(? AS timestamptz)`: SQLite đọc kiểu lạ thành số.
//!
//! Múi giờ, khoảng thời gian, tính lịch (thêm tháng, thứ trong tuần…) dùng thẳng `jiff` (`jiff::tz::TimeZone`,
//! `jiff::SignedDuration`, `jiff::civil`): không có gì phải lưu hay trả ra nên không cần bọc.
//!
//! `chrono` / `chrono-tz` đã gỡ hẳn; test so với giá trị `chrono` đã ghi lại ở `time/golden.json`.

mod date;
mod format;
mod sql;
mod timestamp;

#[cfg(test)]
mod tests;

use std::fmt;

pub use date::Date;
pub use timestamp::Timestamp;

/// Múi giờ IANA theo tên (`Asia/Ho_Chi_Minh`, `UTC`), lấy từ bộ dữ liệu múi giờ đóng gói trong binary (không phụ
/// thuộc máy chủ). Tên phải viết đúng hoa thường như trong bộ dữ liệu — như `chrono-tz` trước đây; `jiff` tự nó
/// chấp nhận cả `asia/ho_chi_minh`, nhưng tên đã lưu sẽ được đọc lại và so khớp ở nơi khác, nên giữ một dạng.
pub fn zone(name: &str) -> Option<jiff::tz::TimeZone> {
    let zone = jiff::tz::TimeZone::get(name).ok()?;
    (zone.iana_name() == Some(name)).then_some(zone)
}

/// Thời điểm của giờ địa phương `local` ở múi `zone` — như `chrono` `from_local_datetime(…).earliest()`:
/// giờ không có ở địa phương (nhảy giờ mùa hè) thì `None`, giờ lặp lại (lùi giờ) thì lấy lần sớm hơn.
pub fn from_local(zone: &jiff::tz::TimeZone, local: jiff::civil::DateTime) -> Option<Timestamp> {
    use jiff::tz::AmbiguousOffset;
    let offset = match zone.to_ambiguous_timestamp(local).offset() {
        AmbiguousOffset::Unambiguous { offset } => offset,
        // Hai lần cùng giờ: lần đầu còn mang độ lệch trước khi đổi giờ.
        AmbiguousOffset::Fold { before, .. } => before,
        AmbiguousOffset::Gap { .. } => return None,
    };
    offset.to_timestamp(local).ok().map(Timestamp::from)
}

/// Giờ trong ngày viết `HH:MM` (`09:00`, cũng nhận `9:00`), như giờ mở cửa. Không nhận giây, không nhận `24:00`.
pub fn parse_clock(raw: &str) -> Option<jiff::civil::Time> {
    jiff::civil::Time::strptime("%H:%M", raw).ok()
}

/// Số chữ số lẻ của giây khi viết RFC 3339 (như `chrono::SecondsFormat`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precision {
    /// Không phần lẻ.
    Seconds,
    /// 3 chữ số.
    Millis,
    /// 6 chữ số.
    Micros,
    /// 9 chữ số.
    Nanos,
    /// Không có, hay 3, 6, 9 chữ số tuỳ giá trị (`SecondsFormat::AutoSi`).
    Auto,
}

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
