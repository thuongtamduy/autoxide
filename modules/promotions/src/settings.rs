//! Cài đặt kế thừa theo nút của `promotions` (bước 34): đặt ở chuỗi, cửa hàng ghi đè. Đọc qua
//! `dyn OrgDirectory` của `organizations`; chưa cài module đó thì dùng mặc định.

use autoxide_kernel::tenancy::OrgSetting;
use serde_json::Value;

/// Cách chọn khi nhiều chương trình cùng áp (D-222).
pub(crate) const STACKING_MODE: OrgSetting = OrgSetting::new(
    "promotions.stacking_mode",
    "How promotions are chosen when several apply: `priority` (by priority, predictable) or \
     `best_for_buyer` (the biggest discount among the stackable set and each exclusive promotion).",
)
.with_default("\"priority\"")
.with_schema(r#"{"type":"string","enum":["priority","best_for_buyer"]}"#)
.checked(|value| match value.as_str() {
    Some("priority" | "best_for_buyer") => Ok(value.clone()),
    _ => Err("must be `priority` or `best_for_buyer`".to_owned()),
});

/// Trả hàng làm đơn rớt dưới ngưỡng thì thu lại khoản giảm lớp `order` không (D-222).
pub(crate) const CLAWBACK_ON_RETURN: OrgSetting = OrgSetting::new(
    "promotions.clawback_on_return",
    "When a return drops an order below a promotion's minimum: `none` (keep the discount) or \
     `proportional` (take back the discount of the items kept).",
)
.with_default("\"none\"")
.with_schema(r#"{"type":"string","enum":["none","proportional"]}"#)
.checked(|value| match value.as_str() {
    Some("none" | "proportional") => Ok(value.clone()),
    _ => Err("must be `none` or `proportional`".to_owned()),
});

/// Tỉnh nơi tổ chức khuyến mại (nơi nhận thông báo, D-225).
pub(crate) const NOTICE_PROVINCE: OrgSetting = OrgSetting::new(
    "promotions.notice_province",
    "Code of the province whose Department of Industry and Trade receives the promotion notices \
     of this unit (two digits in Viet Nam, e.g. `79`).",
)
.with_schema(r#"{"type":"string"}"#)
.checked(|value| match value.as_str() {
    Some(code)
        if !code.is_empty()
            && code.len() <= 8
            && code.bytes().all(|b| b.is_ascii_alphanumeric()) =>
    {
        Ok(Value::String(code.to_owned()))
    }
    _ => Err("must be a province code like `79`".to_owned()),
});

pub(crate) fn all() -> Vec<OrgSetting> {
    vec![STACKING_MODE, CLAWBACK_ON_RETURN, NOTICE_PROVINCE]
}
