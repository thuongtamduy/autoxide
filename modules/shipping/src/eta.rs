//! Ngày giao dự kiến (bước 44 mục 6, D-201, D-223…D-227): ngày tuyệt đối theo múi giờ của lịch
//! giao; ngày làm việc trong tuần, ngày nghỉ và giờ chốt riêng là dữ liệu (`shipping_calendars`,
//! `shipping_calendar_days`), không viết cứng. Hàm ở đây thuần (không chạm database) để test dễ.

use std::collections::BTreeMap;

use autoxide_kernel::time::{self, Date, Timestamp};
use jiff::SignedDuration;
use jiff::tz::TimeZone;

use crate::api::Eta;

/// Múi giờ mặc định của lịch giao.
pub(crate) const DEFAULT_TIME_ZONE: &str = "Asia/Ho_Chi_Minh";
/// Thứ Hai … thứ Sáu (bit 0 = thứ Hai).
pub(crate) const WEEKDAYS: i32 = 0b001_1111;

/// Một ngày riêng của lịch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DayRule {
    Closed,
    /// Làm việc với giờ chốt riêng (phút từ 0 giờ); `None`: không chốt.
    Special(Option<i32>),
}

/// Lịch đã nạp.
#[derive(Debug, Clone)]
pub(crate) struct Calendar {
    pub time_zone: TimeZone,
    pub working_weekdays: i32,
    pub days: BTreeMap<Date, DayRule>,
}

impl Calendar {
    /// Lịch mặc định khi phương thức không gắn lịch: thứ Hai – thứ Bảy, giờ Việt Nam.
    pub(crate) fn standard() -> Self {
        Calendar {
            time_zone: time::zone(DEFAULT_TIME_ZONE).unwrap_or(TimeZone::UTC),
            working_weekdays: 0b011_1111,
            days: BTreeMap::new(),
        }
    }

    fn works(&self, day: Date) -> bool {
        match self.days.get(&day) {
            Some(DayRule::Closed) => false,
            Some(DayRule::Special(_)) => true,
            None => self.working_weekdays & (1 << day.weekday().to_monday_zero_offset()) != 0,
        }
    }

    fn cutoff(&self, day: Date, method: Option<i32>) -> Option<i32> {
        match self.days.get(&day) {
            Some(DayRule::Special(cutoff)) => cutoff.or(method),
            _ => method,
        }
    }

    /// Ngày làm việc đầu tiên từ `day` (gồm `day`), tối đa một năm.
    fn next_working(&self, day: Date) -> Date {
        let mut current = day;
        for _ in 0..366 {
            if self.works(current) {
                return current;
            }
            current = current.succ().unwrap_or(current);
        }
        day
    }

    /// Cộng `count` ngày làm việc sau `day`.
    fn add_working(&self, day: Date, count: u16) -> Date {
        let mut current = day;
        let mut left = count;
        let mut guard = 0;
        while left > 0 && guard < 800 {
            current = current.succ().unwrap_or(current);
            if self.works(current) {
                left -= 1;
            }
            guard += 1;
        }
        current
    }

    /// Lúc `minute` phút của ngày `day` theo múi giờ lịch, ra UTC.
    fn at(&self, day: Date, minute: i32) -> Option<Timestamp> {
        let minute = minute.clamp(0, 1439);
        let clock = jiff::civil::Time::new(
            i8::try_from(minute / 60).ok()?,
            i8::try_from(minute % 60).ok()?,
            0,
            0,
        )
        .ok()?;
        time::from_local(&self.time_zone, day.to_jiff().to_datetime(clock))
    }
}

/// Ngày giao dự kiến của một đơn đặt lúc `at`: hàng rời kho ngày làm việc đầu tiên còn kịp giờ chốt;
/// giao sau `days = (ít nhất, nhiều nhất)` ngày làm việc (`same_day`: 0 ngày).
pub(crate) fn estimate(
    calendar: &Calendar,
    at: Timestamp,
    cutoff_minute: Option<i32>,
    same_day: bool,
    days: Option<(u16, u16)>,
) -> Option<Eta> {
    let (min, max) = match (same_day, days) {
        (true, None) => (0, 0),
        (true, Some((_, max))) => (0, max),
        (false, Some(days)) => days,
        (false, None) => return None,
    };
    let local = at.in_zone(&calendar.time_zone);
    let today = Date::from(local.date());
    let minute = i32::from(local.hour()) * 60 + i32::from(local.minute());
    let cutoff = calendar.cutoff(today, cutoff_minute);
    let in_time = calendar.works(today) && cutoff.is_none_or(|limit| minute < limit);
    let dispatch =
        if in_time { today } else { calendar.next_working(today.succ().unwrap_or(today)) };
    let order_by = if in_time { cutoff.and_then(|limit| calendar.at(today, limit)) } else { None };
    Some(Eta::new(
        calendar.add_working(dispatch, min),
        calendar.add_working(dispatch, max),
        order_by,
        calendar.time_zone.iana_name().unwrap_or("UTC"),
    ))
}

/// Giờ mở cửa mặc định (phút từ 0 giờ) khi cửa hàng chưa khai trong hồ sơ công khai.
pub(crate) const DEFAULT_OPENS: i32 = 8 * 60;
pub(crate) const DEFAULT_CLOSES: i32 = 21 * 60;

/// Giờ mở cửa theo ngày trong tuần (1 = thứ Hai), phút từ 0 giờ; `None` = không khai (dùng mặc
/// định cho ngày làm việc của lịch).
#[derive(Debug, Clone, Default)]
pub(crate) struct Hours(pub Option<BTreeMap<u8, (i32, i32)>>);

impl Hours {
    /// Gộp các khoảng của mỗi ngày thành (mở sớm nhất, đóng muộn nhất).
    pub(crate) fn from_ranges(
        ranges: Option<Vec<autoxide_organizations::api::OpeningRange>>,
    ) -> Self {
        let Some(ranges) = ranges else { return Hours(None) };
        let mut days: BTreeMap<u8, (i32, i32)> = BTreeMap::new();
        for range in ranges {
            let minutes =
                |time: jiff::civil::Time| i32::from(time.hour()) * 60 + i32::from(time.minute());
            let (opens, closes) = (minutes(range.opens), minutes(range.closes));
            let entry = days.entry(range.day).or_insert((opens, closes));
            entry.0 = entry.0.min(opens);
            entry.1 = entry.1.max(closes);
        }
        Hours(Some(days))
    }

    fn window(&self, calendar: &Calendar, day: Date) -> Option<(i32, i32)> {
        if !calendar.works(day) {
            return None;
        }
        let weekday = day.weekday().to_monday_one_offset().unsigned_abs();
        match &self.0 {
            None => Some((DEFAULT_OPENS, DEFAULT_CLOSES)),
            Some(days) => days.get(&weekday).copied(),
        }
    }
}

/// Giờ hàng nhận tại cửa hàng sẵn sàng: `at` + `prep_hours`, lùi tới giờ mở cửa kế tiếp nếu rơi
/// ngoài giờ (ngày nghỉ của lịch và ngày cửa hàng không mở được bỏ qua; tối đa 60 ngày).
pub(crate) fn ready_at(
    calendar: &Calendar,
    hours: &Hours,
    at: Timestamp,
    prep_hours: i32,
) -> Timestamp {
    let ready = at + SignedDuration::from_hours(i64::from(prep_hours.max(0)));
    let local = ready.in_zone(&calendar.time_zone);
    let mut day = Date::from(local.date());
    let minute = i32::from(local.hour()) * 60 + i32::from(local.minute());
    if let Some((opens, closes)) = hours.window(calendar, day) {
        if minute >= opens && minute < closes {
            return ready;
        }
        if minute < opens {
            return calendar.at(day, opens).unwrap_or(ready);
        }
    }
    for _ in 0..60 {
        day = day.succ().unwrap_or(day);
        if let Some((opens, _)) = hours.window(calendar, day) {
            return calendar.at(day, opens).unwrap_or(ready);
        }
    }
    ready
}

#[cfg(test)]
mod tests {

    use super::*;

    fn vn(day: i8, hour: i8, minute: i8) -> Timestamp {
        let vietnam = time::zone("Asia/Ho_Chi_Minh").unwrap();
        time::from_local(&vietnam, jiff::civil::datetime(2026, 11, day, hour, minute, 0, 0))
            .unwrap()
    }

    fn date(day: i8) -> Date {
        Date::new(2026, 11, day).unwrap()
    }

    #[test]
    fn truoc_va_sau_gio_chot() {
        // 18/11/2026 là thứ Tư. Giờ chốt 11:00, giao 1–2 ngày làm việc (thứ Hai – thứ Bảy).
        let calendar = Calendar::standard();
        let before = estimate(&calendar, vn(18, 10, 0), Some(660), false, Some((1, 2))).unwrap();
        assert_eq!(before.earliest_date, date(19));
        assert_eq!(before.latest_date, date(20));
        assert_eq!(before.order_by, Some(vn(18, 11, 0)));
        let after = estimate(&calendar, vn(18, 12, 0), Some(660), false, Some((1, 2))).unwrap();
        assert_eq!(after.earliest_date, date(20));
        assert_eq!(after.order_by, None);
    }

    #[test]
    fn ngay_nghi_va_chu_nhat() {
        let mut calendar = Calendar::standard();
        calendar.days.insert(date(20), DayRule::Closed);
        // Thứ Năm 19 đặt sau giờ chốt: thứ Sáu 20 nghỉ → rời kho thứ Bảy 21, giao thứ Hai 23 (bỏ Chủ nhật).
        let eta = estimate(&calendar, vn(19, 15, 0), Some(660), false, Some((1, 1))).unwrap();
        assert_eq!(eta.earliest_date, date(23));
        // Giao trong ngày.
        let same = estimate(&calendar, vn(18, 9, 0), Some(660), true, None).unwrap();
        assert_eq!(same.earliest_date, date(18));
        assert_eq!(same.latest_date, date(18));
    }

    #[test]
    fn san_sang_nhan_tai_cua_hang() {
        let calendar = Calendar::standard();
        let hours = Hours::default();
        // Đặt 20:00, chuẩn bị 2 giờ → 22:00, cửa hàng 8:00–21:00 → 8:00 hôm sau.
        assert_eq!(ready_at(&calendar, &hours, vn(18, 20, 0), 2), vn(19, 8, 0));
        assert_eq!(ready_at(&calendar, &hours, vn(18, 9, 0), 2), vn(18, 11, 0));
        // Thứ Bảy 21 tối → Chủ nhật nghỉ → thứ Hai 23.
        assert_eq!(ready_at(&calendar, &hours, vn(21, 22, 0), 1), vn(23, 8, 0));
        // Cửa hàng khai giờ: chỉ mở thứ Hai 9:00–17:00.
        let monday = Hours(Some([(1u8, (540, 1020))].into_iter().collect()));
        assert_eq!(ready_at(&calendar, &monday, vn(18, 9, 0), 1), vn(23, 9, 0));
    }
}
