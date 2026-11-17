//! Cài đặt kế thừa theo cây tổ chức (bước 34 mục 4): múi giờ, tiền tệ, ngôn ngữ, hồ sơ thuế,
//! chính sách… Module khai cài đặt của mình bằng [`OrgSetting`] (`Module::org_settings`); một nút
//! đặt giá trị thì mọi nút dưới nó dùng giá trị đó, trừ khi tự đặt giá trị khác (chuỗi đặt, cửa
//! hàng ghi đè). Không nút nào trên chuỗi đặt → giá trị mặc định của cài đặt.
//!
//! Kernel chỉ giữ **danh mục** (tên, mô tả, mặc định, bộ kiểm); giá trị do module `organizations`
//! lưu và phục vụ. Module có dữ liệu kế thừa riêng (vd giá theo cửa hàng ở bảng của chính nó) dùng
//! cùng khuôn: đọc chuỗi tổ tiên bằng [`OrgTree::ancestors`](super::OrgTree::ancestors) rồi lấy giá
//! trị gần nhất bằng [`nearest`](super::nearest).

use std::sync::Arc;

use serde_json::Value;

/// Bộ kiểm giá trị: nhận giá trị JSON người gọi gửi, trả giá trị đã chuẩn hoá hay câu tiếng Anh nói
/// rõ phải sửa gì.
pub type OrgSettingCheck = fn(&Value) -> Result<Value, String>;

/// Một cài đặt theo nút do module khai.
#[derive(Clone, Copy)]
pub struct OrgSetting {
    key: &'static str,
    description: &'static str,
    /// Giá trị mặc định, viết dạng JSON (vd `"\"UTC\""`).
    default: Option<&'static str>,
    check: Option<OrgSettingCheck>,
    /// Dạng giá trị, viết dạng JSON Schema (vd `{"type":"boolean"}`): giao diện dựng ô nhập đúng
    /// loại. Bộ kiểm vẫn là nơi quyết định giá trị hợp lệ.
    schema: Option<&'static str>,
}

impl std::fmt::Debug for OrgSetting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OrgSetting").field("key", &self.key).finish_non_exhaustive()
    }
}

impl OrgSetting {
    /// Cài đặt `key` (`<module>.<tên>`), mô tả tiếng Anh cho người quản trị.
    pub const fn new(key: &'static str, description: &'static str) -> Self {
        OrgSetting { key, description, default: None, check: None, schema: None }
    }

    /// Giá trị mặc định khi không nút nào đặt, viết dạng JSON: `.with_default("\"UTC\"")`.
    pub const fn with_default(mut self, json: &'static str) -> Self {
        self.default = Some(json);
        self
    }

    /// Bộ kiểm (và chuẩn hoá) giá trị.
    pub const fn checked(mut self, check: OrgSettingCheck) -> Self {
        self.check = Some(check);
        self
    }

    /// Dạng giá trị viết bằng JSON Schema, cho giao diện dựng ô nhập:
    /// `.with_schema(r#"{"type":"boolean"}"#)`. Phải là object có `type`; mặc định phải khớp `type`.
    pub const fn with_schema(mut self, json: &'static str) -> Self {
        self.schema = Some(json);
        self
    }

    /// JSON Schema của giá trị; `None` khi module không khai.
    pub fn schema(&self) -> Option<Value> {
        self.schema.and_then(|json| serde_json::from_str(json).ok())
    }

    pub fn key(&self) -> &'static str {
        self.key
    }

    pub fn description(&self) -> &'static str {
        self.description
    }

    /// Giá trị mặc định; `None` khi không có (hay JSON khai sai — `declaration_problems` báo).
    pub fn default_value(&self) -> Option<Value> {
        self.default.and_then(|json| serde_json::from_str(json).ok())
    }

    /// Kiểm `value`; trả giá trị đã chuẩn hoá. Không có bộ kiểm: nhận mọi giá trị trừ `null`.
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

/// Mọi cài đặt theo nút của các module đã đăng ký, theo thứ tự tên. Clone rẻ.
#[derive(Debug, Clone, Default)]
pub struct OrgSettingCatalog(Arc<Vec<OrgSetting>>);

impl OrgSettingCatalog {
    pub(crate) fn new(mut settings: Vec<OrgSetting>) -> Self {
        settings.sort_by_key(|setting| setting.key);
        OrgSettingCatalog(Arc::new(settings))
    }

    /// Danh mục dựng sẵn (test, module dựng riêng).
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

/// Vấn đề khai báo của module `module`: tên phải là `<module>.<tên>` (chữ thường, số, `_`), không
/// trùng, có mô tả, mặc định là JSON hợp lệ và qua bộ kiểm.
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
            problems.push(format!("organization setting `{key}` is declared twice"));
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

/// Schema phải là object JSON có `type` thuộc các kiểu JSON Schema; mặc định (nếu có) khớp `type`.
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
