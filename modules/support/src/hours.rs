//! Giờ làm việc của nút: tính hạn trả lời (SLA) theo phút làm việc, bỏ qua ngoài giờ và ngày nghỉ.
//! Hàm thuần (không chạm database) để test kỹ và dùng chung cho tạo phiếu, đổi ưu tiên, dừng /
//! chạy lại đồng hồ.

use autoxide_kernel::time::{Date, Timestamp};
use jiff::SignedDuration;
use jiff::civil;
use jiff::tz::Offset;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Số ngày tối đa xét khi cộng giờ làm (tránh lặp vô hạn khi lịch gần như không có giờ làm).
const MAX_DAYS: i64 = 3660;

/// One range of working time on a day of the week.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = SupportWorkingHours)]
pub(crate) struct WorkingHours {
    /// 1 = Monday … 7 = Sunday.
    #[schema(minimum = 1, maximum = 7)]
    pub day: u8,
    /// Local time `HH:MM`.
    #[schema(example = "08:00")]
    pub opens: String,
    /// Local time `HH:MM`, after `opens`.
    #[schema(example = "17:00")]
    pub closes: String,
}

/// Lịch làm việc đã kiểm: khung giờ theo thứ, ngày nghỉ, múi giờ của nút.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Calendar {
    /// (thứ 1–7, mở, đóng), giờ địa phương.
    ranges: Vec<(u8, civil::Time, civil::Time)>,
    holidays: Vec<Date>,
    offset: Offset,
}

impl Calendar {
    /// Dựng lịch; `None` khi dữ liệu sai (đã kiểm lúc ghi chính sách nên hiếm).
    pub(crate) fn new(
        hours: &[WorkingHours],
        holidays: &[Date],
        utc_offset_minutes: i32,
    ) -> Option<Self> {
        // Lệch UTC dưới một ngày (như `chrono::FixedOffset` trước đây; `jiff` tự nó cho tới 26 giờ).
        let seconds = utc_offset_minutes.checked_mul(60)?;
        if seconds.abs() >= 86_400 {
            return None;
        }
        let offset = Offset::from_seconds(seconds).ok()?;
        let mut ranges = Vec::new();
        for range in hours {
            let (opens, closes) = (parse_time(&range.opens)?, parse_time(&range.closes)?);
            if !(1..=7).contains(&range.day) || closes <= opens {
                return None;
            }
            ranges.push((range.day, opens, closes));
        }
        ranges.sort();
        Some(Calendar { ranges, holidays: holidays.to_vec(), offset })
    }

    /// Ngày và giờ địa phương của `at`.
    fn local(&self, at: Timestamp) -> (Date, civil::Time) {
        let local = self.offset.to_datetime(at.to_jiff());
        (Date::from(local.date()), local.time())
    }

    /// Thời điểm sau `minutes` phút làm việc kể từ `start`. `None` khi lịch không có giờ làm nào.
    pub(crate) fn add(&self, start: Timestamp, minutes: i64) -> Option<Timestamp> {
        if self.ranges.is_empty() {
            return None;
        }
        // Đếm theo giây để không làm tròn mất phần lẻ của phút ở mỗi khung.
        let mut left = SignedDuration::from_mins(minutes.max(0));
        let (mut date, mut cursor) = self.local(start);
        for _ in 0..MAX_DAYS {
            if !self.holidays.contains(&date) {
                let weekday = weekday(date);
                for (day, opens, closes) in &self.ranges {
                    if *day != weekday || *closes <= cursor {
                        continue;
                    }
                    let from = cursor.max(*opens);
                    let available = closes.duration_since(from);
                    if available >= left {
                        let at = date.to_jiff().to_datetime(from).checked_add(left).ok()?;
                        return self.offset.to_timestamp(at).ok().map(Timestamp::from);
                    }
                    left -= available;
                }
            }
            date = date.succ()?;
            cursor = civil::Time::MIN;
        }
        None
    }

    /// Số phút làm việc giữa `from` và `to` (0 khi `to` trước `from`).
    pub(crate) fn between(&self, from: Timestamp, to: Timestamp) -> i64 {
        if to <= from {
            return 0;
        }
        let ((from_date, from_time), (to_date, to_time)) = (self.local(from), self.local(to));
        let mut date = from_date;
        let mut total = SignedDuration::ZERO;
        for _ in 0..MAX_DAYS {
            if date > to_date {
                break;
            }
            if !self.holidays.contains(&date) {
                let weekday = weekday(date);
                for (day, opens, closes) in &self.ranges {
                    if *day != weekday {
                        continue;
                    }
                    let start = if date == from_date { from_time.max(*opens) } else { *opens };
                    let end = if date == to_date { to_time.min(*closes) } else { *closes };
                    if end > start {
                        total += end.duration_since(start);
                    }
                }
            }
            let Some(next) = date.succ() else { break };
            date = next;
        }
        total.as_mins()
    }
}

/// Thứ trong tuần, 1 = thứ Hai … 7 = Chủ nhật.
fn weekday(date: Date) -> u8 {
    date.weekday().to_monday_one_offset().unsigned_abs()
}

/// Giờ làm mặc định: thứ Hai tới thứ Sáu, 08:00–17:00.
pub(crate) fn default_hours() -> Vec<WorkingHours> {
    (1..=5)
        .map(|day| WorkingHours { day, opens: "08:00".to_owned(), closes: "17:00".to_owned() })
        .collect()
}

pub(crate) fn parse_time(raw: &str) -> Option<civil::Time> {
    autoxide_kernel::time::parse_clock(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calendar(holidays: &[&str]) -> Calendar {
        let holidays: Vec<Date> = holidays.iter().map(|day| day.parse().unwrap()).collect();
        // Giờ Việt Nam (UTC+7).
        Calendar::new(&default_hours(), &holidays, 420).unwrap()
    }

    fn at(raw: &str) -> Timestamp {
        Timestamp::parse_rfc3339(raw).unwrap()
    }

    #[test]
    fn sla_tinh_theo_gio_lam_viec_va_ngay_nghi() {
        let calendar = calendar(&["2026-11-23"]);
        // Thứ Sáu 20/11/2026 16:00 (+07): còn 60 phút hôm đó; thứ Hai 23/11 nghỉ; tiếp thứ Ba.
        let due = calendar.add(at("2026-11-20T16:00:00+07:00"), 8 * 60).unwrap();
        assert_eq!(due, at("2026-11-24T15:00:00+07:00"));
        // Bắt đầu ngoài giờ (tối thứ Tư): tính từ 08:00 sáng thứ Năm.
        let due = calendar.add(at("2026-11-18T22:00:00+07:00"), 30).unwrap();
        assert_eq!(due, at("2026-11-19T08:30:00+07:00"));
        // Ngược lại: số phút làm việc giữa hai mốc.
        let minutes =
            calendar.between(at("2026-11-20T16:00:00+07:00"), at("2026-11-24T15:00:00+07:00"));
        assert_eq!(minutes, 8 * 60);
    }

    #[test]
    fn lich_khong_co_gio_lam_thi_khong_co_han() {
        let empty = Calendar::new(&[], &[], 0).unwrap();
        assert_eq!(empty.add(Timestamp::now(), 10), None);
        let bad = WorkingHours { day: 1, opens: "17:00".to_owned(), closes: "08:00".to_owned() };
        assert!(Calendar::new(&[bad], &[], 0).is_none());
    }
}
