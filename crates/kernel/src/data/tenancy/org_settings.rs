//! Settings inherited down the organization tree (step 34 item 4): time zone, currency, language, tax
//! profile, policy... A module declares its settings with [`OrgSetting`] (`Module::org_settings`); when a
//! node sets a value, every node below it uses that value unless it sets a different one itself (chain of
//! settings, a store overrides). If no node on the chain sets it, the setting's default value applies.
//!
//! The kernel holds only the **catalog** (name, description, default, validator); values are stored and
//! served by the `organizations` module. A module with its own inherited data (e.g. per-store prices in its
//! own table) uses the same pattern: read the ancestor chain with
//! [`OrgTree::ancestors`](super::OrgTree::ancestors) then take the nearest value with
//! [`nearest`](super::nearest).

use std::sync::Arc;

use serde_json::Value;

/// Value validator: takes the JSON value the caller sent, returns the normalized value or an English
/// sentence saying clearly what to fix.
pub type OrgSettingCheck = fn(&Value) -> Result<Value, String>;

/// A per-node setting declared by a module.
#[derive(Clone, Copy)]
pub struct OrgSetting {
    key: &'static str,
    description: &'static str,
    /// Default value, written as JSON (e.g. `"\"UTC\""`).
    default: Option<&'static str>,
    check: Option<OrgSettingCheck>,
    /// Shape of the value, written as JSON Schema (e.g. `{"type":"boolean"}`): the UI builds the right kind of
    /// input. The validator remains the place that decides which values are valid.
    schema: Option<&'static str>,
}

impl std::fmt::Debug for OrgSetting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OrgSetting").field("key", &self.key).finish_non_exhaustive()
    }
}

impl OrgSetting {
    /// Setting `key` (`<module>.<name>`), with an English description for the administrator.
    pub const fn new(key: &'static str, description: &'static str) -> Self {
        OrgSetting { key, description, default: None, check: None, schema: None }
    }

    /// Default value when no node sets it, written as JSON: `.with_default("\"UTC\"")`.
    pub const fn with_default(mut self, json: &'static str) -> Self {
        self.default = Some(json);
        self
    }

    /// Validator (and normalizer) of the value.
    pub const fn checked(mut self, check: OrgSettingCheck) -> Self {
        self.check = Some(check);
        self
    }

    /// Shape of the value written as JSON Schema, for the UI to build an input:
    /// `.with_schema(r#"{"type":"boolean"}"#)`. Must be an object with a `type`; the default must match `type`.
    pub const fn with_schema(mut self, json: &'static str) -> Self {
        self.schema = Some(json);
        self
    }

    /// JSON Schema of the value; `None` when the module does not declare one.
    pub fn schema(&self) -> Option<Value> {
        self.schema.and_then(|json| serde_json::from_str(json).ok())
    }

    pub fn key(&self) -> &'static str {
        self.key
    }

    pub fn description(&self) -> &'static str {
        self.description
    }

    /// Default value; `None` when there is none (or the JSON was declared wrongly, which
    /// `declaration_problems` reports).
    pub fn default_value(&self) -> Option<Value> {
        self.default.and_then(|json| serde_json::from_str(json).ok())
    }

    /// Validates `value`; returns the normalized value. With no validator: accepts every value except `null`.
    pub fn check(&self, value: &Value) -> Result<Value, String> {
        if value.is_null() {
            return Err("must not be null; delete the setting to inherit it instead".to_owned());
        }
        match self.check {
            Some(check) => check(value),
            None => Ok(value.clone()),
        }
    }
}

/// Every per-node setting of the registered modules, in name order. Cheap to clone.
#[derive(Debug, Clone, Default)]
pub struct OrgSettingCatalog(Arc<Vec<OrgSetting>>);

impl OrgSettingCatalog {
    pub(crate) fn new(mut settings: Vec<OrgSetting>) -> Self {
        settings.sort_by_key(|setting| setting.key);
        OrgSettingCatalog(Arc::new(settings))
    }

    /// A prebuilt catalog (tests, modules that build their own).
    pub fn of(settings: Vec<OrgSetting>) -> Self {
        OrgSettingCatalog::new(settings)
    }

    pub fn get(&self, key: &str) -> Option<&OrgSetting> {
        self.0.iter().find(|setting| setting.key == key)
    }

    pub fn iter(&self) -> impl Iterator<Item = &OrgSetting> {
        self.0.iter()
    }
}

/// Declaration problem of module `module`: the name must be `<module>.<name>` (lowercase, digits, `_`), not
/// duplicated, with a description, and the default must be valid JSON that passes the validator.
pub(crate) fn declaration_problems(
    module: &str,
    declared: &[OrgSetting],
    seen: &mut Vec<&'static str>,
) -> Vec<String> {
    let mut problems = Vec::new();
    for setting in declared {
        let key = setting.key;
        let valid =
            key.strip_prefix(module).and_then(|rest| rest.strip_prefix('.')).is_some_and(|name| {
                !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            });
        if !valid {
            problems.push(format!(
                "organization setting `{key}` of module `{module}` must be named `{module}.<name>` \
                 (lower-case letters, digits, `_`)"
            ));
        }
        if seen.contains(&key) {
            problems.push(format!("organization setting `{key}` is declared twice (as a per-node setting, or as a per-tenant setting: a key lives in one scope only)"));
        }
        seen.push(key);
        if setting.description.trim().is_empty() {
            problems.push(format!("organization setting `{key}` has no description"));
        }
        if let Some(json) = setting.default {
            match serde_json::from_str::<Value>(json) {
                Ok(value) => {
                    if let Err(reason) = setting.check(&value) {
                        problems.push(format!(
                            "the default of organization setting `{key}` is invalid: {reason}"
                        ));
                    }
                }
                Err(_) => problems.push(format!(
                    "the default of organization setting `{key}` is not JSON: {json}"
                )),
            }
        }
        if let Some(json) = setting.schema {
            problems.extend(schema_problem(key, json, setting.default_value()));
        }
    }
    problems
}

/// The schema must be a JSON object with a `type` among the JSON Schema types; the default (if any) matches
/// `type`.
fn schema_problem(key: &str, json: &str, default: Option<Value>) -> Option<String> {
    let schema = serde_json::from_str::<Value>(json).ok();
    let kind = schema.as_ref().and_then(|schema| schema.get("type")).and_then(Value::as_str);
    let Some(kind) = kind else {
        return Some(format!(
            "the schema of organization setting `{key}` must be a JSON object with a `type`: {json}"
        ));
    };
    let matches = |value: &Value| match kind {
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => false,
    };
    if !["string", "boolean", "integer", "number", "array", "object"].contains(&kind) {
        return Some(format!(
            "the schema of organization setting `{key}` has an unknown type `{kind}`"
        ));
    }
    match default {
        Some(value) if !matches(&value) => Some(format!(
            "the default of organization setting `{key}` is not of the schema's type `{kind}`"
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upper(value: &Value) -> Result<Value, String> {
        value
            .as_str()
            .map(|text| Value::from(text.to_uppercase()))
            .ok_or_else(|| "must be text".to_owned())
    }

    #[test]
    fn khai_bao_sai_bao_cung_luc_va_kiem_gia_tri() {
        let good = OrgSetting::new("shop.code", "Code.").with_default("\"ab\"").checked(upper);
        let mut seen = Vec::new();
        assert!(declaration_problems("shop", &[good], &mut seen).is_empty());
        assert_eq!(good.default_value(), Some(Value::from("ab")));
        assert_eq!(good.check(&Value::from("x")).unwrap(), Value::from("X"));
        assert!(good.check(&Value::Null).is_err());
        let bad = [
            OrgSetting::new("other.code", "Wrong module."),
            OrgSetting::new("shop.code", "Twice."),
            OrgSetting::new("shop.empty", " "),
            OrgSetting::new("shop.json", "Bad JSON.").with_default("{"),
            OrgSetting::new("shop.check", "Bad default.").with_default("1").checked(upper),
            OrgSetting::new("shop.schema", "Schema without type.").with_schema("{}"),
            OrgSetting::new("shop.kind", "Default of another type.")
                .with_default("1")
                .with_schema(r#"{"type":"boolean"}"#),
        ];
        let problems = declaration_problems("shop", &bad, &mut seen);
        assert_eq!(problems.len(), 7, "{problems:#?}");
        let typed = OrgSetting::new("shop.flag", "Flag.")
            .with_default("true")
            .with_schema(r#"{"type":"boolean"}"#);
        assert!(declaration_problems("shop", &[typed], &mut Vec::new()).is_empty());
        assert_eq!(typed.schema(), Some(serde_json::json!({"type": "boolean"})));
        let catalog = OrgSettingCatalog::new(vec![bad[4], good]);
        let keys: Vec<&str> = catalog.iter().map(OrgSetting::key).collect();
        assert_eq!(keys, ["shop.check", "shop.code"]);
        assert!(catalog.get("shop.code").is_some());
    }
}
