//! Thư viện chủ đề mẫu đi kèm bản phát hành (bước 67, D-234): file `templates/theme-presets.json` nhúng
//! vào binary, không cần bảng. Khách chọn mẫu → tạo bản chủ đề NHÁP của mình rồi chỉnh. Lễ theo âm lịch
//! (Tết, Trung thu) và ngày đổi theo năm (Black Friday) ghi ngày cụ thể từng năm (`dates`) — dữ liệu, kiểm
//! lại mỗi lần cập nhật gói mẫu (cần đối chiếu lịch chính thức); lễ ngày cố định ghi `yearly` (`MM-DD`).

use std::collections::BTreeMap;
use std::sync::LazyLock;

use autoxide_kernel::time::{self, Date, Timestamp};
use jiff::tz::TimeZone;
use serde::Deserialize;
use serde_json::{Map, Value, json};

/// Một chủ đề mẫu.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Preset {
    pub key: String,
    pub name: BTreeMap<String, String>,
    pub overrides: Map<String, Value>,
    pub effect: Option<String>,
    pub effect_intensity: Option<String>,
    /// Ngày theo năm: năm → [ngày đầu, ngày cuối] (giờ địa phương, cả hai ngày tính).
    #[serde(default)]
    pub dates: BTreeMap<String, [String; 2]>,
    /// Ngày cố định mỗi năm: [`MM-DD` đầu, `MM-DD` cuối].
    pub yearly: Option<[String; 2]>,
    /// Lịch lặp (giờ vàng…).
    pub schedule: Option<Value>,
}

static PRESETS: LazyLock<Vec<Preset>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../templates/theme-presets.json")).unwrap_or_default()
});

pub(crate) fn all() -> &'static [Preset] {
    &PRESETS
}

pub(crate) fn find(key: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|preset| preset.key == key)
}

impl Preset {
    /// Ngày gợi ý (đầu, cuối) của năm `year`; `None` khi mẫu là lịch lặp hay chưa có ngày năm đó.
    pub(crate) fn dates_in(&self, year: i16) -> Option<(Date, Date)> {
        let parse = |raw: &str| Date::parse_from_str(raw, "%Y-%m-%d").ok();
        if let Some([first, last]) = self.dates.get(&year.to_string()) {
            return Some((parse(first)?, parse(last)?));
        }
        let [first, last] = self.yearly.as_ref()?;
        Some((parse(&format!("{year}-{first}"))?, parse(&format!("{year}-{last}"))?))
    }

    /// Lịch gợi ý cho năm `year` ở múi giờ `zone`: từ 00:00 ngày đầu tới 00:00 ngày sau ngày cuối (giờ
    /// địa phương); mẫu lịch lặp thì chính lịch đó.
    pub(crate) fn schedule_for(&self, year: i16, zone: &TimeZone) -> Option<Value> {
        if let Some(schedule) = &self.schedule {
            return Some(schedule.clone());
        }
        let (first, last) = self.dates_in(year)?;
        let midnight = |date: Date| -> Option<Timestamp> {
            time::from_local(zone, date.to_jiff().to_datetime(jiff::civil::Time::midnight()))
        };
        let starts = midnight(first)?;
        let ends = midnight(last.checked_add_days(1)?)?;
        Some(json!({"kind": "once", "starts_at": starts, "ends_at": ends}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{schedule, tokens, variants::EFFECTS};

    #[test]
    fn moi_mau_hop_le() {
        assert_eq!(all().len(), 9, "the preset file parses");
        let zone = time::zone("Asia/Ho_Chi_Minh").unwrap();
        for preset in all() {
            assert!(
                preset.name.contains_key("en") && preset.name.contains_key("vi"),
                "{}",
                preset.key
            );
            tokens::check(&preset.overrides)
                .unwrap_or_else(|errors| panic!("{}: {errors:?}", preset.key));
            assert!(
                preset.effect.as_deref().is_none_or(|effect| EFFECTS.contains(&effect)),
                "{}",
                preset.key
            );
            let value = preset
                .schedule_for(2027, &zone)
                .unwrap_or_else(|| panic!("{} has no 2027 dates", preset.key));
            schedule::check(&value).unwrap_or_else(|errors| panic!("{}: {errors:?}", preset.key));
        }
        let tet = find("tet").unwrap().schedule_for(2027, &zone).unwrap();
        // 00:00 ngày 27/01/2027 giờ Việt Nam = 17:00 UTC ngày 26/01.
        assert_eq!(tet["starts_at"], "2027-01-26T17:00:00Z");
    }
}
