//! Thân luật — kích hoạt, điều kiện, hành động — đọc từ JSON và chạy, không chạm database. Kiểm phần
//! cần sổ của chương trình (event có thật, tool có thật, quyền) nằm ở `service::rules`.

pub(crate) mod action;
pub(crate) mod condition;
pub(crate) mod fill;
pub(crate) mod schema;
pub(crate) mod trigger;

use autoxide_kernel::error::FieldError;
use chrono_tz::Tz;
use serde_json::Value;

pub(crate) use action::Action;
pub(crate) use condition::Condition;
pub(crate) use trigger::{MetricOperator, MetricTrigger, Trigger};

/// Thân luật đã đọc.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RuleBody {
    pub trigger: Trigger,
    /// Múi giờ khai trong kích hoạt, nếu có (không thì múi giờ của nút).
    pub timezone: Option<Tz>,
    pub conditions: Option<Condition>,
    pub actions: Vec<Action>,
}

impl RuleBody {
    /// Đọc ba phần của luật; mọi lỗi cấu trúc gom vào `errors`.
    pub(crate) fn parse(
        trigger: &Value,
        conditions: &Value,
        actions: &Value,
        errors: &mut Vec<FieldError>,
    ) -> Option<RuleBody> {
        let trigger = Trigger::parse(trigger, "trigger", errors);
        let before = errors.len();
        let conditions = Condition::parse(conditions, "conditions", errors);
        let conditions_ok = errors.len() == before;
        let actions = Action::parse_all(actions, "actions", errors);
        let (trigger, timezone) = trigger?;
        if !conditions_ok {
            return None;
        }
        Some(RuleBody { trigger, timezone, conditions, actions: actions? })
    }
}
