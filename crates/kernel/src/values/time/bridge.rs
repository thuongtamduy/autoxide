//! Cầu nối TẠM giữa `chrono` và kiểu thời gian mới, để chuyển từng module một (giai đoạn 2 của
//! D-20261121-time-library). Gỡ cùng `chrono` ở PR cuối.

use chrono::{DateTime, NaiveDate, Utc};

use super::{Date, Timestamp};

impl From<DateTime<Utc>> for Timestamp {
    /// Giữ nguyên độ chính xác: giá trị `chrono` thường vừa đọc từ database (xem tài liệu của [`Timestamp`]).
    fn from(value: DateTime<Utc>) -> Self {
        let nanos = i64::from(value.timestamp_subsec_nanos());
        // `chrono` và `jiff` cùng biểu diễn năm −9999…9999 với mọi giá trị Autoxide tạo; ngoài khoảng đó (không
        // xảy ra) thì kẹp về mốc 1970 thay vì dừng chương trình.
        let inner = i32::try_from(nanos)
            .ok()
            .and_then(|nanos| jiff::Timestamp::new(value.timestamp(), nanos).ok())
            .unwrap_or(jiff::Timestamp::UNIX_EPOCH);
        Timestamp::exact(inner)
    }
}

impl From<Timestamp> for DateTime<Utc> {
    fn from(value: Timestamp) -> Self {
        let inner = value.to_jiff();
        // `jiff` cho phần lẻ cùng dấu với phần giây (trước 1970 thì cả hai âm); `chrono` chờ phần lẻ luôn dương
        // nên mượn một giây khi phần lẻ âm.
        let (seconds, nanos) = match inner.subsec_nanosecond() {
            nanos if nanos < 0 => (inner.as_second() - 1, 1_000_000_000 + nanos),
            nanos => (inner.as_second(), nanos),
        };
        u32::try_from(nanos)
            .ok()
            .and_then(|nanos| DateTime::from_timestamp(seconds, nanos))
            .unwrap_or(DateTime::UNIX_EPOCH)
    }
}

impl From<NaiveDate> for Date {
    fn from(value: NaiveDate) -> Self {
        use chrono::Datelike as _;
        // Năm của `chrono` rộng hơn `jiff` (±262 143); ngoài ±9999 (không xảy ra) thì kẹp về 1970-01-01.
        i16::try_from(value.year())
            .ok()
            .and_then(|year| {
                Date::new(year, i8::try_from(value.month()).ok()?, i8::try_from(value.day()).ok()?)
            })
            .unwrap_or(Date::from(jiff::civil::Date::constant(1970, 1, 1)))
    }
}

impl From<Date> for NaiveDate {
    fn from(value: Date) -> Self {
        NaiveDate::from_ymd_opt(
            i32::from(value.year()),
            u32::try_from(value.month()).unwrap_or(1),
            u32::try_from(value.day()).unwrap_or(1),
        )
        .unwrap_or(NaiveDate::MIN)
    }
}
