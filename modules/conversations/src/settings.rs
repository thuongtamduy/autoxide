//! Chính sách chat theo nút (cài đặt của cây tổ chức, kế thừa theo cây). Đọc qua
//! `OrgDirectory::setting` của `organizations`; không có module đó thì mặc định (tắt hết).

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{OrgId, OrgSetting, TenantContext};
use autoxide_organizations::api::OrgDirectory;
use serde_json::Value;

pub(crate) const REQUIRE_VERIFIED: OrgSetting = OrgSetting::new(
    "conversations.require_verified_account",
    "Customers must have a verified account (phone number) before they write to this unit, as \
     Decree 147/2024 asks for accounts that post content.",
)
.with_default("false")
.with_schema(r#"{"type":"boolean"}"#)
.checked(boolean);

pub(crate) const MASK_CONTACTS: OrgSetting = OrgSetting::new(
    "conversations.mask_contacts",
    "Hide phone numbers, email addresses and links in chat messages of this unit (marketplaces use \
     it to keep deals on the platform).",
)
.with_default("false")
.with_schema(r#"{"type":"boolean"}"#)
.checked(boolean);

pub(crate) const MASK_TERMS: OrgSetting = OrgSetting::new(
    "conversations.mask_terms",
    "Extra words hidden in chat messages when contacts are hidden (e.g. names of other chat apps).",
)
.with_default("[]")
.with_schema(r#"{"type":"array","items":{"type":"string","maxLength":50},"maxItems":50}"#)
.checked(terms);

pub(crate) fn all() -> Vec<OrgSetting> {
    vec![REQUIRE_VERIFIED, MASK_CONTACTS, MASK_TERMS]
}

fn boolean(value: &Value) -> Result<Value, String> {
    value.as_bool().map(Value::from).ok_or_else(|| "must be `true` or `false`".to_owned())
}

fn terms(value: &Value) -> Result<Value, String> {
    let problem = || "must be a list of at most 50 words of 1 to 50 characters".to_owned();
    let list = value.as_array().filter(|list| list.len() <= 50).ok_or_else(problem)?;
    let mut out = Vec::new();
    for item in list {
        let term = item.as_str().map(str::trim).ok_or_else(problem)?;
        if !(1..=50).contains(&term.chars().count()) {
            return Err(problem());
        }
        out.push(Value::from(term.to_owned()));
    }
    Ok(Value::Array(out))
}

/// Chính sách hiệu lực ở một nút.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Policy {
    pub require_verified: bool,
    pub mask_contacts: bool,
    pub mask_terms: Vec<String>,
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
        if self.orgs.is_none() {
            return Ok(Policy::default());
        }
        let flag = |value: Option<Value>| value.and_then(|v| v.as_bool()).unwrap_or(false);
        let mask_contacts = flag(self.value(tenant, org, MASK_CONTACTS.key()).await?);
        let mask_terms = if mask_contacts {
            self.value(tenant, org, MASK_TERMS.key())
                .await?
                .and_then(|value| serde_json::from_value::<Vec<String>>(value).ok())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        Ok(Policy {
            require_verified: flag(self.value(tenant, org, REQUIRE_VERIFIED.key()).await?),
            mask_contacts,
            mask_terms,
        })
    }
}
