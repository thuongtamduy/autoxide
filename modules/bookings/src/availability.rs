//! Tính giờ trống — hàm THUẦN, không chạm database (D-20261118-step60-bookings): giờ làm việc lặp
//! theo thứ (giờ địa phương của nút) + ca thêm − ngày nghỉ, rồi trừ ô giờ đã bị chiếm, tính cả thời
//! gian đệm trước / sau. Kết quả là thời điểm UTC; giao diện hiện theo múi giờ của nút.
//!
//! Đặt chỗ dùng lại [`Plan::within_hours`] để báo `bookings.outside_hours`; còn "đã có người đặt"
//! do khoá duy nhất của `bookings_slots` quyết (không tin kết quả tính ở đây khi ghi).

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use uuid::Uuid;

/// Độ mịn của ô giờ bị chiếm.
pub(crate) const SLOT_MINUTES: i64 = 5;
/// Giờ bắt đầu được chào theo lưới này (mọi múi giờ thật đều lệch UTC bội số 15 phút, nên lưới
/// UTC trùng lưới giờ địa phương).
pub(crate) const START_STEP_MINUTES: i64 = 15;

/// Một khoảng [start, end) theo UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Span {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

impl Span {
    pub(crate) fn new(start: DateTime<Utc>, end: DateTime<Utc>) -> Self {
        Span { start, end }
    }

    fn contains(&self, other: &Span) -> bool {
        self.start <= other.start && other.end <= self.end
    }
}

/// Một khung giờ lặp: thứ (ISO, 1 = thứ Hai … 7 = Chủ nhật) và phút trong ngày [start, end).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Weekly {
    pub weekday: u32,
    pub start_minute: i64,
    pub end_minute: i64,
}

/// Hình dạng thời gian của một dịch vụ, theo phút.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Shape {
    pub duration: i64,
    pub buffer_before: i64,
    pub buffer_after: i64,
}

impl Shape {
    /// Khoảng phục vụ khách khi bắt đầu lúc `start`.
    pub(crate) fn service_span(&self, start: DateTime<Utc>) -> Span {
        Span::new(start, start + Duration::minutes(self.duration))
    }

    /// Khoảng tài nguyên bị chiếm (gồm đệm) khi bắt đầu lúc `start`.
    pub(crate) fn occupied_span(&self, start: DateTime<Utc>) -> Span {
        Span::new(
            start - Duration::minutes(self.buffer_before),
            start + Duration::minutes(self.duration + self.buffer_after),
        )
    }
}

/// Lịch của một tài nguyên trong khoảng đang xét.
#[derive(Debug, Clone, Default)]
pub(crate) struct Plan {
    pub resource_id: Uuid,
    pub weekly: Vec<Weekly>,
    /// Không có lịch thì luôn mở (phòng, thiết bị); nhân viên luôn `false`.
    pub always_open: bool,
    pub open: Vec<Span>,
    pub closed: Vec<Span>,
    /// Đầu các ô 5 phút đã bị chiếm.
    pub busy: BTreeSet<DateTime<Utc>>,
}

impl Plan {
    /// Khoảng làm việc giao với [from, to): (lịch lặp ∪ ca thêm) − nghỉ, đã gộp và sắp.
    pub(crate) fn working(&self, zone: Tz, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<Span> {
        let mut spans: Vec<Span> = if self.always_open {
            vec![Span::new(from, to)]
        } else {
            let first = from.with_timezone(&zone).date_naive() - Duration::days(1);
            let last = to.with_timezone(&zone).date_naive() + Duration::days(1);
            let mut found = Vec::new();
            let mut day = first;
            while day <= last {
                let weekday = day.weekday().number_from_monday();
                for range in self.weekly.iter().filter(|range| range.weekday == weekday) {
                    let start = local(zone, day, range.start_minute);
                    let end = local(zone, day, range.end_minute);
                    if start < end {
                        found.push(Span::new(start, end));
                    }
                }
                day += Duration::days(1);
            }
            found
        };
        spans.extend(self.open.iter().copied());
        let merged = merge(spans);
        let cut = subtract(merged, &merge(self.closed.clone()));
        cut.into_iter()
            .filter_map(|span| {
                let start = span.start.max(from);
                let end = span.end.min(to);
                (start < end).then(|| Span::new(start, end))
            })
            .collect()
    }

    /// Cả khoảng chiếm của lịch bắt đầu lúc `start` nằm trong giờ làm việc.
    pub(crate) fn within_hours(&self, zone: Tz, shape: Shape, start: DateTime<Utc>) -> bool {
        let occupied = shape.occupied_span(start);
        // Giờ làm việc quanh lịch là đủ (một ngày mỗi bên cho ca qua nửa đêm).
        self.working(zone, occupied.start - Duration::days(1), occupied.end + Duration::days(1))
            .iter()
            .any(|span| span.contains(&occupied))
    }

    /// Không ô nào của `span` đã bị chiếm.
    pub(crate) fn free(&self, span: Span) -> bool {
        cells(span).iter().all(|cell| !self.busy.contains(cell))
    }
}

/// Đầu các ô 5 phút phủ `span` (làm tròn ra ngoài).
pub(crate) fn cells(span: Span) -> Vec<DateTime<Utc>> {
    let step = SLOT_MINUTES * 60;
    let start = span.start.timestamp().div_euclid(step) * step;
    let end = span.end.timestamp();
    let mut found = Vec::new();
    let mut at = start;
    while at < end {
        if let Some(cell) = DateTime::from_timestamp(at, 0) {
            found.push(cell);
        }
        at += step;
    }
    found
}

/// Giờ địa phương `minute` phút sau nửa đêm ngày `day` ở `zone`, theo UTC. Giờ không tồn tại (giờ
/// mùa hè nhảy tới) lấy thời điểm hợp lệ kế tiếp; giờ lặp hai lần lấy lần đầu.
fn local(zone: Tz, day: NaiveDate, minute: i64) -> DateTime<Utc> {
    let date = day + Duration::days(minute.div_euclid(1440));
    let minute = minute.rem_euclid(1440);
    let time = NaiveTime::from_hms_opt(
        u32::try_from(minute / 60).unwrap_or(0),
        u32::try_from(minute % 60).unwrap_or(0),
        0,
    )
    .unwrap_or(NaiveTime::MIN);
    let mut naive = date.and_time(time);
    for _ in 0..4 {
        if let Some(found) = zone.from_local_datetime(&naive).earliest() {
            return found.with_timezone(&Utc);
        }
        naive += Duration::minutes(30);
    }
    Utc.from_utc_datetime(&naive)
}

/// Gộp các khoảng chồng hay sát nhau.
fn merge(mut spans: Vec<Span>) -> Vec<Span> {
    spans.sort();
    let mut merged: Vec<Span> = Vec::with_capacity(spans.len());
    for span in spans {
        match merged.last_mut() {
            Some(last) if span.start <= last.end => last.end = last.end.max(span.end),
            _ => merged.push(span),
        }
    }
    merged
}

/// `spans` − `holes` (cả hai đã gộp và sắp).
fn subtract(spans: Vec<Span>, holes: &[Span]) -> Vec<Span> {
    let mut result = Vec::new();
    for span in spans {
        let mut start = span.start;
        for hole in holes {
            if hole.end <= start || hole.start >= span.end {
                continue;
            }
            if hole.start > start {
                result.push(Span::new(start, hole.start));
            }
            start = start.max(hole.end);
        }
        if start < span.end {
            result.push(Span::new(start, span.end));
        }
    }
    result
}

/// Thời điểm đầu tiên ≥ `at` nằm trên lưới [`START_STEP_MINUTES`].
fn align_up(at: DateTime<Utc>) -> DateTime<Utc> {
    let step = START_STEP_MINUTES * 60;
    let seconds = at.timestamp() + i64::from(at.nanosecond() > 0);
    let aligned = (seconds + step - 1).div_euclid(step) * step;
    DateTime::from_timestamp(aligned, 0).unwrap_or(at)
}

/// Một giờ còn trống.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Opening {
    pub resource_id: Uuid,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub seats_left: i32,
}

/// Câu hỏi giờ trống cho một dịch vụ.
pub(crate) struct Query<'a> {
    pub zone: Tz,
    pub shape: Shape,
    pub capacity: i32,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub now: DateTime<Utc>,
    /// Nhân viên làm được dịch vụ (chọn một).
    pub performers: &'a [Plan],
    /// Phòng / thiết bị dịch vụ cần (cần cả).
    pub required: &'a [Plan],
    /// Buổi đã mở của dịch vụ (lớp học): (tài nguyên, giờ bắt đầu) → chỗ còn lại.
    pub sessions: &'a BTreeMap<(Uuid, DateTime<Utc>), i32>,
}

/// Mọi giờ còn trống trong [from, to), sắp theo giờ rồi theo tài nguyên.
pub(crate) fn openings(query: &Query<'_>) -> Vec<Opening> {
    let mut found = Vec::new();
    let earliest = query.from.max(query.now);
    for plan in query.performers {
        for span in
            plan.working(query.zone, query.from - Duration::days(1), query.to + Duration::days(1))
        {
            let mut start =
                align_up((span.start + Duration::minutes(query.shape.buffer_before)).max(earliest));
            while start < query.to {
                let occupied = query.shape.occupied_span(start);
                if occupied.end > span.end {
                    break;
                }
                if let Some(opening) = check(query, plan, start) {
                    found.push(opening);
                }
                start += Duration::minutes(START_STEP_MINUTES);
            }
        }
    }
    found.sort_by_key(|opening| (opening.starts_at, opening.resource_id));
    found
}

/// Giờ `start` của `plan` còn trống không (đã biết nằm trong giờ làm của `plan`).
fn check(query: &Query<'_>, plan: &Plan, start: DateTime<Utc>) -> Option<Opening> {
    let service = query.shape.service_span(start);
    let opening = |seats_left| Opening {
        resource_id: plan.resource_id,
        starts_at: service.start,
        ends_at: service.end,
        seats_left,
    };
    // Buổi đã mở: ô giờ thuộc buổi, chỉ còn đếm chỗ.
    if let Some(left) = query.sessions.get(&(plan.resource_id, start)) {
        return (*left > 0).then(|| opening(*left));
    }
    let occupied = query.shape.occupied_span(start);
    if !plan.free(occupied) {
        return None;
    }
    for room in query.required {
        if !room.free(occupied) || !room.within_hours(query.zone, query.shape, start) {
            return None;
        }
    }
    Some(opening(query.capacity))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc)
    }

    fn staff(weekly: Vec<Weekly>) -> Plan {
        Plan { resource_id: Uuid::from_u128(1), weekly, ..Plan::default() }
    }

    /// Thứ Hai 9:00–12:00 giờ địa phương.
    fn monday_morning() -> Vec<Weekly> {
        vec![Weekly { weekday: 1, start_minute: 9 * 60, end_minute: 12 * 60 }]
    }

    fn starts(found: &[Opening], zone: Tz) -> Vec<String> {
        found.iter().map(|o| o.starts_at.with_timezone(&zone).format("%H:%M").to_string()).collect()
    }

    fn query<'a>(
        zone: Tz,
        shape: Shape,
        performers: &'a [Plan],
        required: &'a [Plan],
        sessions: &'a BTreeMap<(Uuid, DateTime<Utc>), i32>,
    ) -> Query<'a> {
        Query {
            zone,
            shape,
            capacity: 1,
            // Thứ Hai 2026-11-23, cả ngày theo UTC ± 1 ngày.
            from: at("2026-11-22T00:00:00Z"),
            to: at("2026-11-25T00:00:00Z"),
            now: at("2026-11-01T00:00:00Z"),
            performers,
            required,
            sessions,
        }
    }

    #[test]
    fn gio_trong_tru_nghi_va_dem_thuan() {
        let zone: Tz = "Asia/Ho_Chi_Minh".parse().unwrap();
        let shape = Shape { duration: 60, buffer_before: 0, buffer_after: 15 };
        let mut plan = staff(monday_morning());
        // Nghỉ 10:00–10:30 (giờ VN = 03:00–03:30 UTC).
        plan.closed.push(Span::new(at("2026-11-23T03:00:00Z"), at("2026-11-23T03:30:00Z")));
        let sessions = BTreeMap::new();
        let found = openings(&query(zone, shape, std::slice::from_ref(&plan), &[], &sessions));
        // 9:00 + 60 + 15 đệm = 10:15 > 10:00 nghỉ: không; 9:00 không vừa. Sau nghỉ: 10:30, 10:45.
        assert_eq!(starts(&found, zone), ["10:30", "10:45"]);

        // Đã có lịch chiếm 10:30–11:00: 10:30 trùng, 10:45 (chiếm tới 12:00 vì đệm) chạm ô 10:45–11:00.
        plan.busy.extend(cells(Span::new(at("2026-11-23T03:30:00Z"), at("2026-11-23T04:00:00Z"))));
        let found = openings(&query(zone, shape, std::slice::from_ref(&plan), &[], &sessions));
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn dem_truoc_can_cho_trong_truoc_gio_bat_dau() {
        let zone = Tz::UTC;
        let shape = Shape { duration: 30, buffer_before: 15, buffer_after: 0 };
        let plan = staff(monday_morning());
        let sessions = BTreeMap::new();
        let found = openings(&query(zone, shape, std::slice::from_ref(&plan), &[], &sessions));
        // Đệm trước 15 phút: sớm nhất 9:15; muộn nhất 11:30 (kết thúc 12:00).
        assert_eq!(found.first().map(|o| o.starts_at), Some(at("2026-11-23T09:15:00Z")));
        assert_eq!(found.last().map(|o| o.starts_at), Some(at("2026-11-23T11:30:00Z")));
    }

    #[test]
    fn cung_lich_khac_mui_gio_ra_khac_thoi_diem_utc() {
        let shape = Shape { duration: 60, buffer_before: 0, buffer_after: 0 };
        let plan = staff(monday_morning());
        let sessions = BTreeMap::new();
        let hanoi: Tz = "Asia/Ho_Chi_Minh".parse().unwrap();
        let tokyo: Tz = "Asia/Tokyo".parse().unwrap();
        let in_hanoi = openings(&query(hanoi, shape, std::slice::from_ref(&plan), &[], &sessions));
        let in_tokyo = openings(&query(tokyo, shape, std::slice::from_ref(&plan), &[], &sessions));
        // Cùng 9:00 giờ địa phương.
        assert_eq!(starts(&in_hanoi, hanoi)[0], "09:00");
        assert_eq!(starts(&in_tokyo, tokyo)[0], "09:00");
        assert_eq!(in_hanoi[0].starts_at, at("2026-11-23T02:00:00Z"));
        assert_eq!(in_tokyo[0].starts_at, at("2026-11-23T00:00:00Z"));
    }

    #[test]
    fn gio_mua_he_van_dung_gio_dia_phuong() {
        // New York đổi giờ ngày 2026-11-01 (02:00 EDT → 01:00 EST). Thứ Hai 2/11 9:00 = 14:00 UTC;
        // thứ Hai 26/10 9:00 = 13:00 UTC.
        let zone: Tz = "America/New_York".parse().unwrap();
        let shape = Shape { duration: 60, buffer_before: 0, buffer_after: 0 };
        let plan = staff(monday_morning());
        let sessions = BTreeMap::new();
        let mut query = query(zone, shape, std::slice::from_ref(&plan), &[], &sessions);
        query.now = at("2026-10-01T00:00:00Z");
        query.from = at("2026-10-26T00:00:00Z");
        query.to = at("2026-11-03T00:00:00Z");
        let found = openings(&query);
        let firsts: Vec<_> = found
            .iter()
            .filter(|o| {
                let local = o.starts_at.with_timezone(&zone);
                local.hour() == 9 && local.minute() == 0
            })
            .map(|o| o.starts_at)
            .collect();
        assert_eq!(firsts, [at("2026-10-26T13:00:00Z"), at("2026-11-02T14:00:00Z")]);
    }

    #[test]
    fn phong_can_trong_va_buoi_lop_dem_cho() {
        let zone = Tz::UTC;
        let shape = Shape { duration: 60, buffer_before: 0, buffer_after: 0 };
        let plan = staff(monday_morning());
        let mut room =
            Plan { resource_id: Uuid::from_u128(9), always_open: true, ..Plan::default() };
        room.busy.extend(cells(Span::new(at("2026-11-23T09:00:00Z"), at("2026-11-23T10:00:00Z"))));
        let mut sessions = BTreeMap::new();
        let found = openings(&query(
            zone,
            shape,
            std::slice::from_ref(&plan),
            std::slice::from_ref(&room),
            &sessions,
        ));
        // Phòng bận 9:00–10:00: sớm nhất 10:00.
        assert_eq!(starts(&found, zone)[0], "10:00");

        // Buổi lớp 9:00 đã mở, còn 3 chỗ: chào lại dù ô giờ đã thuộc buổi.
        sessions.insert((plan.resource_id, at("2026-11-23T09:00:00Z")), 3);
        let found = openings(&query(
            zone,
            shape,
            std::slice::from_ref(&plan),
            std::slice::from_ref(&room),
            &sessions,
        ));
        assert_eq!((starts(&found, zone)[0].as_str(), found[0].seats_left), ("09:00", 3));
        sessions.insert((plan.resource_id, at("2026-11-23T09:00:00Z")), 0);
        let found = openings(&query(
            zone,
            shape,
            std::slice::from_ref(&plan),
            std::slice::from_ref(&room),
            &sessions,
        ));
        assert_eq!(starts(&found, zone)[0], "10:00");
    }

    #[test]
    fn ca_them_va_o_gio_lam_tron_ra_ngoai() {
        let plan = Plan {
            resource_id: Uuid::from_u128(1),
            open: vec![Span::new(at("2026-11-24T18:00:00Z"), at("2026-11-24T19:00:00Z"))],
            ..Plan::default()
        };
        let shape = Shape { duration: 60, buffer_before: 0, buffer_after: 0 };
        assert!(plan.within_hours(Tz::UTC, shape, at("2026-11-24T18:00:00Z")));
        assert!(!plan.within_hours(Tz::UTC, shape, at("2026-11-24T18:05:00Z")));
        assert_eq!(
            cells(Span::new(at("2026-11-24T18:02:00Z"), at("2026-11-24T18:11:00Z"))).len(),
            3
        );
    }
}
