//! Mẫu trang bắt buộc theo pháp luật Việt Nam (bước 37, D-359): thông tin chủ sở hữu, chính sách dữ
//! liệu cá nhân, đổi trả, vận chuyển, thanh toán, kiểm hàng, giải quyết khiếu nại, điều khoản, điều
//! kiện giao dịch chung; sàn thêm quy chế, mẫu hợp đồng với người bán, thông tin người bán.
//!
//! Mẫu là file JSON trong `templates/legal/<khoá>.<en|vi>.json`: thân ProseMirror theo allowlist của
//! `content` (D-102), chữ có chỗ điền `{{ owner.legal_name }}`… Từ mẫu tạo một trang NHÁP: biến lấy
//! từ hồ sơ pháp lý của nút (qua `autoxide_compliance::api::LegalProfiles` — không đọc bảng của
//! `compliance`), rồi biến người gọi gửi (thắng). Thay biến bằng bộ dựng mẫu của kernel (D-351), chỉ
//! trong từng text node, ở chế độ chữ thuần: giá trị không bao giờ thành thẻ hay node mới.
//!
//! Module `content` vẫn trung lập với nghiệp vụ: mẫu chỉ là chữ, khái niệm "chủ sở hữu" thuộc
//! `compliance`. Mọi mẫu mở đầu bằng câu "cần người có chuyên môn pháp lý duyệt".

use std::sync::LazyLock;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::template::Output;
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use autoxide_spike_macros::Schema as ToSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::entries::Entry;
use crate::entries::{EntryFields, Kind, NewEntry};
use crate::permissions;
use crate::service::ContentService;

/// Ngôn ngữ có mẫu.
pub(crate) const LOCALES: [&str; 2] = ["en", "vi"];

/// Mọi file mẫu, nhúng vào binary lúc biên dịch.
const FILES: [&str; 24] = [
    include_str!("../templates/legal/owner_info.en.json"),
    include_str!("../templates/legal/owner_info.vi.json"),
    include_str!("../templates/legal/privacy_policy.en.json"),
    include_str!("../templates/legal/privacy_policy.vi.json"),
    include_str!("../templates/legal/return_policy.en.json"),
    include_str!("../templates/legal/return_policy.vi.json"),
    include_str!("../templates/legal/shipping_policy.en.json"),
    include_str!("../templates/legal/shipping_policy.vi.json"),
    include_str!("../templates/legal/payment_policy.en.json"),
    include_str!("../templates/legal/payment_policy.vi.json"),
    include_str!("../templates/legal/inspection_policy.en.json"),
    include_str!("../templates/legal/inspection_policy.vi.json"),
    include_str!("../templates/legal/complaint_policy.en.json"),
    include_str!("../templates/legal/complaint_policy.vi.json"),
    include_str!("../templates/legal/terms_of_use.en.json"),
    include_str!("../templates/legal/terms_of_use.vi.json"),
    include_str!("../templates/legal/general_conditions.en.json"),
    include_str!("../templates/legal/general_conditions.vi.json"),
    include_str!("../templates/legal/platform_rules.en.json"),
    include_str!("../templates/legal/platform_rules.vi.json"),
    include_str!("../templates/legal/seller_agreement.en.json"),
    include_str!("../templates/legal/seller_agreement.vi.json"),
    include_str!("../templates/legal/seller_info.en.json"),
    include_str!("../templates/legal/seller_info.vi.json"),
];

/// A variable a template uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = ContentTemplateVariable)]
pub(crate) struct TemplateVariable {
    /// Dotted path, e.g. `owner.legal_name` or `policy.return_days`.
    pub path: String,
    /// The draft cannot be made without it.
    pub required: bool,
    pub description: String,
}

/// Một file mẫu.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct TemplateFile {
    pub key: String,
    pub locale: String,
    pub title: String,
    pub marketplace: bool,
    pub variables: Vec<TemplateVariable>,
    pub body: Value,
}

/// Mẫu đã đọc. File hỏng là lỗi lúc build (test `moi_mau_doc_duoc_va_than_hop_le`), nên ở đây bỏ
/// qua file không đọc được thay vì làm hỏng tiến trình.
static TEMPLATES: LazyLock<Vec<TemplateFile>> = LazyLock::new(|| {
    FILES.iter().filter_map(|text| serde_json::from_str::<TemplateFile>(text).ok()).collect()
});

pub(crate) fn templates() -> &'static [TemplateFile] {
    &TEMPLATES
}

/// A mandatory page template.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ContentTemplate)]
pub(crate) struct TemplateSummary {
    /// e.g. `return_policy`; also the page kind to link in
    /// `PUT /api/v1/compliance/websites/{id}/pages/{page_kind}` (except `seller_info`, a block
    /// for product pages).
    pub key: String,
    /// Title per language, e.g. `{"en": "…", "vi": "…"}`.
    #[schema(value_type = Object)]
    pub titles: Value,
    /// Only a marketplace needs it.
    pub marketplace: bool,
    /// Always `true`: have a legal professional review the text before use.
    pub needs_legal_review: bool,
    pub variables: Vec<TemplateVariable>,
}

/// Một trang mẫu theo khoá (REST `GET /templates` và tool `content.legal_templates` cùng gọi). Mẫu nằm trong
/// bản build, không trong database: `page_of` theo khoá (duy nhất sau khi gộp hai ngôn ngữ). Cần `content.read`.
pub(crate) fn summary_page(
    principal: &autoxide_kernel::access::Principal,
    page: &autoxide_kernel::pagination::PageRequest,
) -> Result<autoxide_kernel::pagination::Page<TemplateSummary>, autoxide_kernel::error::Error> {
    principal.require(&crate::permissions::CONTENT_READ)?;
    autoxide_kernel::pagination::page_of(summaries(), |item| item.key.clone(), page)
}

/// Danh sách mẫu (gộp hai ngôn ngữ của cùng khoá), theo thứ tự file.
pub(crate) fn summaries() -> Vec<TemplateSummary> {
    let mut out: Vec<TemplateSummary> = Vec::new();
    for file in templates() {
        match out.iter_mut().find(|item| item.key == file.key) {
            Some(item) => {
                item.titles[&file.locale] = Value::String(file.title.clone());
            }
            None => {
                let mut titles = Map::new();
                titles.insert(file.locale.clone(), Value::String(file.title.clone()));
                out.push(TemplateSummary {
                    key: file.key.clone(),
                    titles: Value::Object(titles),
                    marketplace: file.marketplace,
                    needs_legal_review: true,
                    variables: file.variables.clone(),
                });
            }
        }
    }
    out
}

fn template_not_found(key: &str) -> Error {
    Error::not_found("content.template_not_found", "There is no page template `{key}`.")
        .with_param("key", key)
}

/// Gộp `extra` vào `base` (đối tượng lồng nhau gộp theo khoá; giá trị khác thì `extra` thắng).
fn merge(base: &mut Value, extra: &Value) {
    match (base, extra) {
        (Value::Object(base), Value::Object(extra)) => {
            for (key, value) in extra {
                merge(base.entry(key.clone()).or_insert(Value::Null), value);
            }
        }
        (base, extra) => *base = extra.clone(),
    }
}

/// Giá trị ở đường dẫn chấm, nếu có và không rỗng.
fn present(context: &Value, path: &str) -> bool {
    let mut node = context;
    for part in path.split('.') {
        match node.get(part) {
            Some(next) => node = next,
            None => return false,
        }
    }
    match node {
        Value::Null => false,
        Value::String(text) => !text.trim().is_empty(),
        _ => true,
    }
}

/// Một trang nháp cần tạo từ mẫu.
#[derive(Debug, Clone)]
pub(crate) struct TemplateDraft {
    pub locale: String,
    pub variables: Value,
    pub parent_id: Option<Uuid>,
    pub org_id: Option<Uuid>,
}

impl ContentService {
    /// Thay biến trong mọi text node của `node` (đệ quy). Chỉ đổi chuỗi `text`, không thêm node.
    fn fill(&self, node: &mut Value, locale: &str, context: &Value) -> Result<(), Error> {
        if let Some(Value::String(text)) = node.get_mut("text")
            && text.contains("{{")
        {
            *text = self.templates.render(text, Output::Text, locale, context)?;
        }
        if let Some(Value::Array(children)) = node.get_mut("content") {
            for child in children {
                self.fill(child, locale, context)?;
            }
        }
        Ok(())
    }

    /// Tạo trang nháp từ mẫu `key`. Cần `content.write` (hay `content.write_own`) như tạo trang.
    pub(crate) async fn draft_from_template(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        key: &str,
        input: TemplateDraft,
    ) -> Result<Entry, Error> {
        permissions::require_create(principal)?;
        if !templates().iter().any(|file| file.key == key) {
            return Err(template_not_found(key));
        }
        let mut problems = Vec::new();
        if !LOCALES.contains(&input.locale.as_str()) {
            problems.push(
                FieldError::new("locale", "invalid", "must be one of {values}")
                    .with_param("values", LOCALES.join(", ")),
            );
        }
        if !input.variables.is_object() && !input.variables.is_null() {
            problems.push(FieldError::new("variables", "invalid", "must be an object"));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let Some(file) =
            templates().iter().find(|file| file.key == key && file.locale == input.locale)
        else {
            return Err(template_not_found(key));
        };
        // Hồ sơ pháp lý của nút (hay nút cấp trên gần nhất), qua hợp đồng của `compliance`.
        let mut context =
            serde_json::json!({"owner": {}, "contact": {}, "website": {}, "policy": {}});
        if let Some(legal) = &self.legal {
            let org = input.org_id.map(OrgId::from_uuid);
            if let Some(found) = legal.template_variables(tenant, org).await? {
                merge(&mut context, &found);
            }
        }
        merge(&mut context, &input.variables);
        let missing: Vec<FieldError> = file
            .variables
            .iter()
            .filter(|variable| variable.required && !present(&context, &variable.path))
            .map(|variable| {
                FieldError::new(
                    format!("variables.{}", variable.path),
                    "required",
                    "is needed by this template ({description})",
                )
                .with_param("description", &variable.description)
            })
            .collect();
        if !missing.is_empty() {
            let mut error = Error::new(
                ErrorKind::Validation,
                "content.template_variables_missing",
                "The template needs values the legal profile does not give; send them in \
                 `variables` (see `errors`).",
            );
            for field in missing {
                error = error.with_field(field);
            }
            return Err(error);
        }
        let mut body = file.body.clone();
        self.fill(&mut body, &file.locale, &context)?;
        let new = NewEntry {
            kind: Kind::Page,
            type_id: None,
            locale: Some(file.locale.clone()),
            parent_id: input.parent_id,
            org_id: input.org_id,
            fields: EntryFields {
                title: Some(file.title.clone()),
                body: Some(body),
                ..EntryFields::default()
            },
        };
        self.create(tenant, principal, new).await
    }
}

// ---- Ảnh chụp bản đã đăng (hợp đồng `api::PublishedEntries`) ---------------------------------

#[async_trait::async_trait]
impl crate::api::PublishedEntries for ContentService {
    async fn published_snapshot(
        &self,
        tenant: &TenantContext,
        entry_id: Uuid,
    ) -> Result<Option<crate::api::PublishedSnapshot>, Error> {
        use crate::entity::published;
        use autoxide_kernel::tenancy::ScopeExt;
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
        let row = published::Entity::find()
            .scoped(tenant)
            .filter(published::Column::EntryId.eq(entry_id))
            .one(tenant.database().read())
            .await?;
        Ok(row.map(|row| crate::api::PublishedSnapshot {
            entry_id: row.entry_id,
            body: serde_json::from_str(&row.body).unwrap_or(Value::Null),
            title: row.title,
            locale: row.locale,
            plain_text: row.plain_text,
            revision_id: row.revision_id,
            published_at: row.published_at,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn moi_mau_doc_duoc_va_than_hop_le() {
        assert_eq!(templates().len(), FILES.len(), "a template file does not parse");
        let engine = autoxide_kernel::template::TemplateEngine::default();
        for file in templates() {
            assert!(LOCALES.contains(&file.locale.as_str()));
            // Thân theo allowlist của `content`.
            let checked = crate::entries::body::check(&file.body, 1_000_000, &[]);
            assert!(checked.is_ok(), "{} {}: {checked:?}", file.key, file.locale);
            // Đoạn đầu là lời nhắc cần người có chuyên môn pháp lý duyệt.
            let first = file.body["content"][0].to_string();
            assert!(
                first.contains("legal professional") || first.contains("chuyên môn pháp lý"),
                "{}",
                file.key
            );
            // Mọi biến dùng trong mẫu đã khai.
            let declared: Vec<&str> = file.variables.iter().map(|v| v.path.as_str()).collect();
            let text = file.body.to_string();
            for part in text.split("{{").skip(1) {
                let expression = part.split("}}").next().unwrap_or_default();
                for word in
                    expression.split(|c: char| !(c.is_alphanumeric() || c == '.' || c == '_'))
                {
                    if word.contains('.') {
                        assert!(
                            declared.contains(&word),
                            "{} {}: `{word}` not declared",
                            file.key,
                            file.locale
                        );
                    }
                }
            }
            // Dựng được với ngữ cảnh rỗng (biến tuỳ chọn có giá trị thay thế).
            let context = json!({"owner": {}, "contact": {}, "website": {}, "policy": {}});
            let mut body = file.body.clone();
            fn walk(
                node: &mut Value,
                engine: &autoxide_kernel::template::TemplateEngine,
                context: &Value,
            ) {
                if let Some(Value::String(text)) = node.get_mut("text") {
                    *text = engine.render(text, Output::Text, "vi", context).unwrap();
                }
                if let Some(Value::Array(children)) = node.get_mut("content") {
                    for child in children {
                        walk(child, engine, context);
                    }
                }
            }
            walk(&mut body, &engine, &context);
        }
        // Mỗi khoá có đủ hai ngôn ngữ, cùng danh sách biến.
        for summary in summaries() {
            assert!(
                summary.titles["en"].is_string() && summary.titles["vi"].is_string(),
                "{}",
                summary.key
            );
            let en = templates().iter().find(|f| f.key == summary.key && f.locale == "en").unwrap();
            let vi = templates().iter().find(|f| f.key == summary.key && f.locale == "vi").unwrap();
            assert_eq!(en.variables, vi.variables, "{}", summary.key);
        }
    }

    #[test]
    fn gop_bien_va_kiem_co_mat() {
        let mut base = json!({"owner": {"legal_name": "A", "tax_code": null}, "policy": {}});
        merge(
            &mut base,
            &json!({"owner": {"tax_code": "0101234567"}, "policy": {"return_days": 7}}),
        );
        assert_eq!(base["owner"]["legal_name"], "A");
        assert!(present(&base, "owner.tax_code") && present(&base, "policy.return_days"));
        assert!(!present(&base, "contact.email"));
        assert!(!present(&json!({"a": {"b": "  "}}), "a.b"));
    }
}
