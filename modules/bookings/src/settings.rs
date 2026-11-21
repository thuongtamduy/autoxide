//! Chính sách đặt lịch theo nút (cài đặt của cây tổ chức, kế thừa theo cây) và múi giờ của nút. Đọc
//! qua `OrgDirectory::setting` của `organizations`; không có module đó (test riêng) thì mặc định.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{OrgId, OrgSetting, TenantContext};
use autoxide_organizations::api::OrgDirectory;
use jiff::tz::TimeZone;
use serde_json::Value;

pub(crate) const FREE_CANCEL_HOURS: OrgSetting = OrgSetting::new(
    "bookings.free_cancel_hours",
    "Customers who cancel at least this many hours before the start get the whole deposit back.",
)
.with_default("24")
.with_schema(r#"{"type":"integer","minimum":0,"maximum":720}"#)
.checked(hours);

pub(crate) const LATE_CANCEL_RETAIN_PERCENT: OrgSetting = OrgSetting::new(
    "bookings.late_cancel_retain_percent",
    "Share of the deposit kept when a customer cancels later than the free cancellation window.",
)
.with_default("100")
.with_schema(r#"{"type":"integer","minimum":0,"maximum":100}"#)
.checked(percent);

pub(crate) const NO_SHOW_RETAIN_PERCENT: OrgSetting = OrgSetting::new(
    "bookings.no_show_retain_percent",
    "Share of the deposit kept when the customer does not come.",
)
.with_default("100")
.with_schema(r#"{"type":"integer","minimum":0,"maximum":100}"#)
.checked(percent);

pub(crate) const REQUIRE_APPROVAL: OrgSetting = OrgSetting::new(
    "bookings.require_approval",
    "Bookings without a deposit wait for the shop to confirm them instead of being confirmed at once.",
)
.with_default("false")
.with_schema(r#"{"type":"boolean"}"#)
.checked(boolean);

pub(crate) const HOLD_MINUTES: OrgSetting = OrgSetting::new(
    "bookings.hold_minutes",
    "Minutes a booking is held while the customer pays the deposit; unpaid holds are released.",
)
.with_default("10")
.with_schema(r#"{"type":"integer","minimum":2,"maximum":60}"#)
.checked(hold);

pub(crate) fn all() -> Vec<OrgSetting> {
    vec![
        FREE_CANCEL_HOURS,
        LATE_CANCEL_RETAIN_PERCENT,
        NO_SHOW_RETAIN_PERCENT,
        REQUIRE_APPROVAL,
        HOLD_MINUTES,
    ]
}

fn integer(value: &Value, min: i64, max: i64) -> Result<Value, String> {
    value
        .as_i64()
        .filter(|number| (min..=max).contains(number))
        .map(Value::from)
        .ok_or_else(|| format!("must be a whole number from {min} to {max}"))
}

fn hours(value: &Value) -> Result<Value, String> {
    integer(value, 0, 720)
}

fn percent(value: &Value) -> Result<Value, String> {
    integer(value, 0, 100)
}

fn hold(value: &Value) -> Result<Value, String> {
    integer(value, 2, 60)
}

fn boolean(value: &Value) -> Result<Value, String> {
    value.as_bool().map(Value::from).ok_or_else(|| "must be `true` or `false`".to_owned())
}

/// Chính sách hiệu lực ở một nút.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Policy {
    pub free_cancel_hours: i64,
    pub late_cancel_retain_percent: i64,
    pub no_show_retain_percent: i64,
    pub require_approval: bool,
    pub hold_minutes: i64,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            free_cancel_hours: 24,
            late_cancel_retain_percent: 100,
            no_show_retain_percent: 100,
            require_approval: false,
            hold_minutes: 10,
        }
    }
}

/// Đọc cài đặt của nút qua `organizations`.
#[derive(Clone, Default)]
pub(crate) struct Settings {
    pub orgs: Option<Arc<dyn OrgDirectory>>,
}

impl Settings {
    async fn value(
        &self,
        tenant: &TenantContext,
        org: OrgId,
        key: &str,
    ) -> Result<Option<Value>, Error> {
        let Some(orgs) = &self.orgs else {
            return Ok(None);
        };
        Ok(orgs.setting(tenant, org, key).await?.and_then(|found| found.value))
    }

    /// Chính sách hiệu lực ở nút `org`.
    pub(crate) async fn policy(&self, tenant: &TenantContext, org: OrgId) -> Result<Policy, Error> {
        let defaults = Policy::default();
        let number =
            |value: Option<Value>, default: i64| value.and_then(|v| v.as_i64()).unwrap_or(default);
        Ok(Policy {
            free_cancel_hours: number(
                self.value(tenant, org, FREE_CANCEL_HOURS.key()).await?,
                defaults.free_cancel_hours,
            ),
            late_cancel_retain_percent: number(
                self.value(tenant, org, LATE_CANCEL_RETAIN_PERCENT.key()).await?,
                defaults.late_cancel_retain_percent,
            ),
            no_show_retain_percent: number(
                self.value(tenant, org, NO_SHOW_RETAIN_PERCENT.key()).await?,
                defaults.no_show_retain_percent,
            ),
            require_approval: self
                .value(tenant, org, REQUIRE_APPROVAL.key())
                .await?
                .and_then(|value| value.as_bool())
                .unwrap_or(defaults.require_approval),
            hold_minutes: number(
                self.value(tenant, org, HOLD_MINUTES.key()).await?,
                defaults.hold_minutes,
            ),
        })
    }

    /// Múi giờ của nút `org` (`organizations.timezone`, kế thừa theo cây); chưa đặt hay không đọc
    /// được thì UTC.
    pub(crate) async fn timezone(
        &self,
        tenant: &TenantContext,
        org: OrgId,
    ) -> Result<TimeZone, Error> {
        Ok(self
            .value(tenant, org, "organizations.timezone")
            .await?
            .and_then(|value| value.as_str().and_then(autoxide_kernel::time::zone))
            .unwrap_or(TimeZone::UTC))
    }
}
