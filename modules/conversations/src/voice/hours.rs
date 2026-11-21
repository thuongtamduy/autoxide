//! Giờ nhận gọi lại của một đường dây: danh sách khung theo thứ trong tuần, giờ địa phương của nút
//! (lệch UTC cố định theo phút). Mặc định 08:00–20:00 mỗi ngày, giờ Việt Nam (D-232).
use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Lệch UTC mặc định: giờ Việt Nam.
pub(crate) const DEFAULT_OFFSET_MINUTES: i32 = 7 * 60;

/// A range of call-back hours, in the local time of the line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = CallbackHoursRange)]
#[serde(deny_unknown_fields)]
pub(crate) struct Range {
    /// Day of the week, 1 (Monday) to 7 (Sunday).
    pub day: u8,
    /// Opening time, `HH:MM`.
    #[schema(example = "08:00")]
    pub from: String,
    /// Closing time, `HH:MM`, after `from`.
    #[schema(example = "20:00")]
    pub to: String,
}

pub(crate) fn default_hours() -> Vec<Range> {
    (1..=7).map(|day| Range { day, from: "08:00".to_owned(), to: "20:00".to_owned() }).collect()
}

fn time(raw: &str) -> Option<jiff::civil::Time> {
    autoxide_kernel::time::parse_clock(raw)
}

/// Khung hợp lệ: thứ 1–7, giờ `HH:MM`, kết thúc sau bắt đầu, tối đa 50 khung.
pub(crate) fn valid(ranges: &[Range]) -> bool {
    ranges.len() <= 50 && ranges.iter().all(|range| {
        (1..=7).contains(&range.day)
            && matches!((time(&range.from), time(&range.to)), (Some(from), Some(to)) if from < to)
    })
}

/// `[start, end]` nằm trọn trong MỘT khung (cùng ngày giờ địa phương).
pub(crate) fn covers(
    ranges: &[Range],
    offset_minutes: i32,
    start: Timestamp,
    end: Timestamp,
) -> bool {
    let shift = SignedDuration::from_mins(i64::from(offset_minutes));
    let utc = |at: Timestamp| jiff::tz::Offset::UTC.to_datetime(at.to_jiff());
    let (local_start, local_end) = (utc(start + shift), utc(end + shift));
    if local_start.date() != local_end.date() {
        return false;
    }
    let day = local_start.weekday().to_monday_one_offset().unsigned_abs();
    ranges.iter().any(|range| {
        range.day == day
            && matches!((time(&range.from), time(&range.to)), (Some(from), Some(to))
                if local_start.time() >= from && local_end.time() <= to)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn khung_gio_theo_gio_dia_phuong() {
        let hours = default_hours();
        // 2026-11-19 là thứ Năm. 02:00 UTC = 09:00 giờ Việt Nam.
        let start = Timestamp::parse_rfc3339("2026-11-19T02:00:00Z").unwrap();
        assert!(covers(
            &hours,
            DEFAULT_OFFSET_MINUTES,
            start,
            start + SignedDuration::from_hours(1)
        ));
        // 13:30 UTC = 20:30: ngoài giờ.
        let late = Timestamp::parse_rfc3339("2026-11-19T13:30:00Z").unwrap();
        assert!(!covers(
            &hours,
            DEFAULT_OFFSET_MINUTES,
            late,
            late + SignedDuration::from_mins(10)
        ));
        assert!(!valid(&[Range { day: 8, from: "08:00".into(), to: "09:00".into() }]));
        assert!(!valid(&[Range { day: 1, from: "10:00".into(), to: "09:00".into() }]));
    }
}
