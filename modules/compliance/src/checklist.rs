//! Danh sách kiểm và bộ hồ sơ Bộ Công Thương của một website (bước 37, D-357, D-358).
//!
//! Danh sách kiểm là DỮ LIỆU: bản quy tắc `moit.checklist.<sales|platform>` hiệu lực hôm nay. Mỗi mục
//! có `check.type` thuộc tập cố định trong code: `profile_field`, `page`, `document`, `kyc`, `policy`,
//! `fact`, `manual`. Kết quả mỗi mục: `met`, `missing`, `unknown` (chưa trả lời được, vd chưa module
//! nào khai "fact") hay `manual` (người tự kiểm).
//!
//! Bộ hồ sơ (`dossier`) điền từ dữ liệu thật: tờ khai, nháp đề án và quy chế (sàn) dạng Markdown, dựng
//! bằng bộ dựng mẫu của kernel (D-351). Mọi thứ là bản nháp cần người có chuyên môn pháp lý duyệt;
//! con người nộp trên online.gov.vn (không có API công khai).

use autoxide_kernel::time::Timestamp;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use autoxide_kernel::doctor::{Finding, Probe};
use autoxide_kernel::error::Error;
use autoxide_kernel::template::Output;
use autoxide_kernel::tenancy::{DatabaseRegistry, OrgId, TenantContext, TenantId};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde::Serialize;
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::api::{Fact, Subject, WebsiteRef};
use crate::entity::{kyc_profile, legal_profile, website};
use crate::legal::{address_line, address_of};
use crate::service::ComplianceService;

/// Sổ "fact" do module khác đăng ký (qua `dyn ComplianceFacts`).
#[derive(Default)]
pub(crate) struct FactRegistry {
    // Ghi chỉ lúc khởi động; đọc mỗi lần tính danh sách kiểm. Không giữ khoá qua `await`.
    facts: RwLock<HashMap<&'static str, Arc<dyn Fact>>>,
}

impl FactRegistry {
    pub(crate) fn register(&self, key: &'static str, fact: Arc<dyn Fact>) -> Result<(), Error> {
        let mut facts =
            self.facts.write().map_err(|_| Error::internal("the fact registry is poisoned"))?;
        if facts.contains_key(key) {
            return Err(Error::internal(format!("compliance fact `{key}` is registered twice")));
        }
        facts.insert(key, fact);
        Ok(())
    }

    fn get(&self, key: &str) -> Option<Arc<dyn Fact>> {
        self.facts.read().ok().and_then(|facts| facts.get(key).cloned())
    }
}

/// One line of the checklist.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = ComplianceChecklistItem)]
pub(crate) struct ChecklistItem {
    #[schema(example = "page.return_policy")]
    pub key: String,
    /// `{"en": "…", "vi": "…"}`.
    #[schema(value_type = Object)]
    pub title: Value,
    /// `profile_field`, `page`, `document`, `kyc`, `policy`, `fact` or `manual`.
    pub check: String,
    /// `met`, `missing`, `unknown` (cannot be answered yet, e.g. no module answers this fact) or
    /// `manual` (a person checks it).
    pub status: String,
    /// What to do when it is not met.
    pub hint: Option<String>,
    pub legal_basis: String,
    /// Check against the original legal text before relying on it.
    pub needs_legal_review: bool,
}

/// What a website still lacks before it is notified to, or registered with, the Ministry of
/// Industry and Trade (data: the rule `moit.checklist.<kind>` in force).
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = ComplianceChecklist)]
pub(crate) struct Checklist {
    pub website_id: Uuid,
    pub domain: String,
    /// `sales` or `platform`.
    pub kind: String,
    /// `notification` (sales website, blue logo) or `registration` (marketplace or other
    /// intermediary, red logo).
    pub procedure: String,
    /// Current status with the ministry.
    pub status: String,
    /// Version of the rule `moit.checklist.<kind>` used; `null` when no rule is in force.
    pub rule_version: Option<i64>,
    pub legal_basis: String,
    pub needs_legal_review: bool,
    /// Items `missing`.
    pub missing: usize,
    /// Items `unknown`.
    pub unknown: usize,
    /// Selling before the procedure is done is shown as "required by law" (`label`), not blocked:
    /// see `selling_note` (decision D-358; check with a legal professional).
    #[schema(example = "label")]
    pub selling_before_done: String,
    pub selling_note: String,
    pub items: Vec<ChecklistItem>,
}

/// A document of the dossier.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = ComplianceDossierDocument)]
pub(crate) struct DossierDocument {
    /// `declaration`, `project_plan` or `platform_rules`.
    pub key: String,
    pub title: String,
    /// Always `markdown`.
    pub format: String,
    pub body: String,
}

/// Draft dossier for the Ministry of Industry and Trade, filled from the tenant's data. A person
/// submits it on online.gov.vn; have a legal professional review it first.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = ComplianceDossier)]
pub(crate) struct Dossier {
    pub website_id: Uuid,
    pub generated_at: Timestamp,
    pub disclaimer: String,
    /// Fields of the declaration form (owner, website, identity verification, pages).
    #[schema(value_type = Object)]
    pub form: Value,
    pub documents: Vec<DossierDocument>,
    pub checklist: Checklist,
}

const DISCLAIMER: &str = "Bản nháp tự động từ dữ liệu của bạn — cần người có chuyên môn pháp lý duyệt và \
                          đối chiếu văn bản gốc trước khi nộp. Draft generated from your data: have a \
                          legal professional review it before submitting.";

fn is_blank(value: &Option<String>) -> bool {
    value.as_deref().is_none_or(|value| value.trim().is_empty())
}

/// Trường hồ sơ `field` có giá trị không.
fn profile_has(profile: &legal_profile::Model, field: &str) -> Option<bool> {
    Some(match field {
        "legal_name" => !profile.legal_name.trim().is_empty(),
        "trade_name" => !is_blank(&profile.trade_name),
        "tax_code" => !is_blank(&profile.tax_code),
        "registration_number" => !is_blank(&profile.registration_number),
        "registration_issued_on" => !is_blank(&profile.registration_issued_on),
        "registration_issued_by" => !is_blank(&profile.registration_issued_by),
        "address" => profile.address.is_some(),
        "representative_name" => !is_blank(&profile.representative_name),
        "representative_title" => !is_blank(&profile.representative_title),
        "phone" => !is_blank(&profile.phone),
        "email" => !is_blank(&profile.email),
        _ => return None,
    })
}

/// Trang công khai đã gắn: (loại trang, tiêu đề, URL đầy đủ).
#[derive(Debug, Clone, Serialize)]
struct LinkedPage {
    kind: String,
    title: String,
    url: Option<String>,
    public: bool,
}

/// Mẫu tờ khai (Markdown, tiếng Việt — tờ khai nộp bằng tiếng Việt).
const DECLARATION: &str = r#"# Thông tin khai báo website {{ website.domain }}

> {{ disclaimer }}

Thủ tục: **{% if website.kind == "platform" %}đăng ký website cung cấp dịch vụ thương mại điện tử{% else %}thông báo website thương mại điện tử bán hàng{% endif %}** với Bộ Công Thương ({{ portal_url }}).

## Chủ sở hữu

- Tên: {{ owner.legal_name or "(chưa có)" }}
- Tên giao dịch: {{ owner.trade_name or "(chưa có)" }}
- Mã số thuế: {{ owner.tax_code or "(chưa có)" }}
- Giấy chứng nhận đăng ký: {{ owner.registration_number or "(chưa có)" }}{% if owner.registration_issued_on %}, cấp ngày {{ owner.registration_issued_on }}{% endif %}{% if owner.registration_issued_by %}, nơi cấp {{ owner.registration_issued_by }}{% endif %}
- Địa chỉ: {{ owner.address or "(chưa có)" }}
- Người đại diện: {{ owner.representative_name or "(chưa có)" }}{% if owner.representative_title %} — {{ owner.representative_title }}{% endif %}
- Điện thoại: {{ contact.phone or "(chưa có)" }}
- Email: {{ contact.email or "(chưa có)" }}

## Website

- Tên miền: {{ website.domain }}
- Kênh: {{ website.channel }}

## Trang bắt buộc trên website
{% for page in pages %}
- {{ page.title }}: {{ page.url or "(chưa gắn trang công khai)" }}{% endfor %}
"#;

/// Nháp đề án cung cấp dịch vụ (sàn / trung gian).
const PROJECT_PLAN: &str = r#"# Đề án cung cấp dịch vụ thương mại điện tử — bản nháp

> {{ disclaimer }}

## 1. Thông tin chung

- Thương nhân, tổ chức: {{ owner.legal_name or "(chưa có)" }} — mã số thuế {{ owner.tax_code or "(chưa có)" }}
- Địa chỉ: {{ owner.address or "(chưa có)" }}
- Người đại diện: {{ owner.representative_name or "(chưa có)" }}
- Website: {{ website.domain }}

## 2. Mô hình hoạt động

Website là sàn giao dịch thương mại điện tử: người bán đã định danh đăng bán hàng hoá, dịch vụ; người
mua đặt hàng trên website. (Mô tả cụ thể đối tượng tham gia, quy trình, phí — cần hoàn thiện.)

## 3. Quy trình giao dịch

Mô tả quy trình đặt hàng, xác nhận, giao nhận, thanh toán, đổi trả theo các trang đã công bố:
{% for page in pages %}
- {{ page.title }}: {{ page.url or "(chưa gắn trang công khai)" }}{% endfor %}

## 4. Phân định quyền và trách nhiệm

Trách nhiệm của chủ sàn, người bán, người mua theo quy chế hoạt động của sàn (bản nháp kèm theo).

## 5. Tiếp nhận và giải quyết khiếu nại

- Đầu mối: {{ complaints.contact or "(chưa đặt chính sách `complaints.contact`)" }}

## 6. Bảo vệ thông tin cá nhân

Theo chính sách bảo vệ dữ liệu cá nhân đã công bố trên website.
"#;

/// Nháp quy chế quản lý hoạt động của sàn.
const PLATFORM_RULES: &str = r#"# Quy chế quản lý hoạt động của sàn {{ website.domain }} — bản nháp

> {{ disclaimer }}

## I. Nguyên tắc chung

Sàn do {{ owner.legal_name or "(chưa có)" }} (mã số thuế {{ owner.tax_code or "(chưa có)" }}) sở hữu và vận
hành. Người bán phải định danh trước khi đăng bán; thông tin người bán hiện trên trang sản phẩm.

## II. Quy định và quy trình giao dịch

Theo các chính sách đã công bố:
{% for page in pages %}
- {{ page.title }}: {{ page.url or "(chưa gắn trang công khai)" }}{% endfor %}

## III. Quyền và nghĩa vụ của chủ sàn, người bán, người mua

(Cần hoàn thiện theo mô hình kinh doanh.)

## IV. Giải quyết khiếu nại, tranh chấp

Đầu mối tiếp nhận: {{ complaints.contact or "(chưa đặt chính sách `complaints.contact`)" }}.

## V. Liên hệ

{{ owner.legal_name or "(chưa có)" }} — {{ owner.address or "(chưa có)" }} — {{ contact.phone or "" }} {{ contact.email or "" }}
"#;

/// Tên trang theo loại (tiếng Việt, cho bộ hồ sơ).
fn page_title(kind: &str) -> &'static str {
    match kind {
        "owner_info" => "Thông tin chủ sở hữu",
        "privacy_policy" => "Chính sách bảo vệ dữ liệu cá nhân",
        "return_policy" => "Chính sách đổi trả, hoàn tiền",
        "shipping_policy" => "Chính sách vận chuyển, giao nhận",
        "payment_policy" => "Phương thức thanh toán",
        "inspection_policy" => "Chính sách kiểm hàng",
        "complaint_policy" => "Cơ chế giải quyết khiếu nại",
        "terms_of_use" => "Điều khoản sử dụng",
        "general_conditions" => "Điều kiện giao dịch chung",
        "platform_rules" => "Quy chế hoạt động",
        "seller_agreement" => "Hợp đồng với người bán",
        _ => "Trang",
    }
}

impl ComplianceService {
    /// Trang đã gắn với website, tra trạng thái công khai qua sổ đối tượng (gọi NGOÀI transaction).
    async fn linked_pages(
        &self,
        tenant: &TenantContext,
        site: &website::Model,
    ) -> Result<HashMap<String, LinkedPage>, Error> {
        let mut pages = HashMap::new();
        for row in self.website_pages(tenant, site.id).await? {
            let resolved = match row.resource.split_once(':') {
                Some((kind, id)) if self.resources.knows(kind) => {
                    self.resources.resolve(tenant, kind, id).await?
                }
                _ => None,
            };
            let (public, url) = match resolved {
                Some(resource) if resource.public => {
                    let url = resource.url.map(|path| {
                        if path.starts_with("http://") || path.starts_with("https://") {
                            path
                        } else {
                            format!("https://{}{}", site.domain, path)
                        }
                    });
                    (url.is_some(), url)
                }
                _ => (false, None),
            };
            pages.insert(
                row.page_kind.clone(),
                LinkedPage {
                    title: page_title(&row.page_kind).to_owned(),
                    kind: row.page_kind,
                    url,
                    public,
                },
            );
        }
        Ok(pages)
    }

    /// Hồ sơ định danh của chủ sở hữu đã xác thực và còn hạn chưa.
    async fn owner_verified(
        &self,
        tenant: &TenantContext,
        profile: &legal_profile::Model,
    ) -> Result<bool, Error> {
        if let Some(id) = profile.kyc_profile_id {
            let found = kyc_profile::Entity::find_by_id(id)
                .filter(kyc_profile::Column::TenantId.eq(tenant.id().as_uuid()))
                .one(tenant.database().read())
                .await?;
            return Ok(found.is_some_and(|row| {
                row.status == "verified" && row.expires_at.is_none_or(|at| at > Timestamp::now())
            }));
        }
        let status =
            self.kyc_status(tenant, &Subject::Org(OrgId::from_uuid(profile.org_id))).await?;
        Ok(status.status == "verified")
    }

    /// Danh sách kiểm của website `site` theo quy tắc hiệu lực lúc `at`.
    pub(crate) async fn checklist_of(
        &self,
        tenant: &TenantContext,
        site: &website::Model,
        at: Timestamp,
    ) -> Result<Checklist, Error> {
        let rule =
            self.rule_for(tenant, &format!("moit.checklist.{}", site.website_kind), at).await?;
        let body = rule
            .as_ref()
            .and_then(|row| serde_json::from_str::<Value>(&row.body).ok())
            .unwrap_or(Value::Null);
        let profile = self.profile_for_org(tenant, OrgId::from_uuid(site.org_id)).await?;
        let pages = self.linked_pages(tenant, site).await?;
        let reference = WebsiteRef {
            id: site.id,
            org: OrgId::from_uuid(site.org_id),
            domain: site.domain.clone(),
            kind: site.website_kind.clone(),
        };
        let mut items = Vec::new();
        for item in body["items"].as_array().map(Vec::as_slice).unwrap_or_default() {
            let check = &item["check"];
            let check_type = check["type"].as_str().unwrap_or("manual").to_owned();
            let (status, hint): (&str, Option<String>) = match check_type.as_str() {
                "profile_field" => {
                    let field = check["field"].as_str().unwrap_or_default();
                    match profile.as_ref().map(|profile| profile_has(profile, field)) {
                        None => ("missing", Some(
                            "Create the legal profile: `POST /api/v1/compliance/legal-profiles`."
                                .to_owned(),
                        )),
                        Some(None) => ("unknown", Some(format!("Unknown profile field `{field}`."))),
                        Some(Some(true)) => ("met", None),
                        Some(Some(false)) => ("missing", Some(format!(
                            "Fill `{field}` of the legal profile: `PATCH /api/v1/compliance/legal-profiles/{{id}}`."
                        ))),
                    }
                }
                "page" => {
                    let kind = check["page_kind"].as_str().unwrap_or_default();
                    match pages.get(kind) {
                        Some(page) if page.public => ("met", None),
                        Some(_) => ("missing", Some(format!(
                            "The page linked as `{kind}` is not published; publish it."
                        ))),
                        None => ("missing", Some(format!(
                            "Publish the page (a template: `POST /api/v1/content/templates/{kind}/drafts`), then link it: `PUT /api/v1/compliance/websites/{}/pages/{kind}`.",
                            site.id
                        ))),
                    }
                }
                "document" => {
                    let kind = check["kind"].as_str().unwrap_or_default();
                    match self.current_document_version(tenant, kind).await? {
                        true => ("met", None),
                        false => ("missing", Some(format!(
                            "Create the legal document `{kind}` and publish a version: `POST /api/v1/compliance/documents`."
                        ))),
                    }
                }
                "kyc" => match &profile {
                    Some(profile) if self.owner_verified(tenant, profile).await? => ("met", None),
                    _ => ("missing", Some(
                        "Verify the owner's identity: `POST /api/v1/compliance/kyc/profiles`, then link it as `kyc_profile_id` of the legal profile."
                            .to_owned(),
                    )),
                },
                "policy" => {
                    let key = check["key"].as_str().unwrap_or_default();
                    match self.policy(tenant, key).await? {
                        Some(value) if !value.is_null() => ("met", None),
                        _ => ("missing", Some(format!(
                            "Set the policy: `PUT /api/v1/compliance/policies/{key}`."
                        ))),
                    }
                }
                "fact" => {
                    let key = check["key"].as_str().unwrap_or_default();
                    match self.facts.get(key) {
                        None => ("unknown", Some(format!(
                            "No installed module answers `{key}` yet; check it yourself."
                        ))),
                        Some(fact) => match fact.holds(tenant.id(), &reference).await {
                            Ok(true) => ("met", None),
                            Ok(false) => ("missing", None),
                            Err(error) => ("unknown", Some(error.message())),
                        },
                    }
                }
                _ => ("manual", Some("Check this yourself before submitting.".to_owned())),
            };
            items.push(ChecklistItem {
                key: item["key"].as_str().unwrap_or_default().to_owned(),
                title: item["title"].clone(),
                check: check_type,
                status: status.to_owned(),
                hint,
                legal_basis: item["legal_basis"].as_str().unwrap_or_default().to_owned(),
                needs_legal_review: item["needs_legal_review"].as_bool().unwrap_or(true),
            });
        }
        Ok(Checklist {
            website_id: site.id,
            domain: site.domain.clone(),
            kind: site.website_kind.clone(),
            procedure: body["registration"]
                .as_str()
                .unwrap_or(if site.website_kind == "platform" {
                    "registration"
                } else {
                    "notification"
                })
                .to_owned(),
            status: site.moit_status.clone(),
            rule_version: rule.as_ref().map(|row| row.version),
            legal_basis: rule.as_ref().map(|row| row.legal_basis.clone()).unwrap_or_default(),
            needs_legal_review: rule.as_ref().is_none_or(|row| row.needs_legal_review),
            missing: items.iter().filter(|item| item.status == "missing").count(),
            unknown: items.iter().filter(|item| item.status == "unknown").count(),
            selling_before_done: body["sell_before_done"].as_str().unwrap_or("label").to_owned(),
            selling_note: body["sell_before_done_note"].as_str().unwrap_or_default().to_owned(),
            items,
        })
    }

    /// Văn bản pháp lý `kind` cấp khách đã có phiên bản hiện hành chưa.
    async fn current_document_version(
        &self,
        tenant: &TenantContext,
        kind: &str,
    ) -> Result<bool, Error> {
        use crate::entity::document;
        use autoxide_kernel::tenancy::ScopeExt;
        Ok(document::Entity::find()
            .scoped(tenant)
            .filter(document::Column::OrgKey.eq(""))
            .filter(document::Column::Kind.eq(kind))
            .one(tenant.database().read())
            .await?
            .is_some_and(|row| row.current_version.is_some()))
    }

    pub(crate) async fn checklist(
        &self,
        tenant: &TenantContext,
        principal: &autoxide_kernel::access::Principal,
        id: Uuid,
    ) -> Result<Checklist, Error> {
        let site = self.get_website(tenant, principal, id).await?;
        self.checklist_of(tenant, &site, Timestamp::now()).await
    }

    /// Bộ hồ sơ nháp của website `id`.
    pub(crate) async fn dossier(
        &self,
        tenant: &TenantContext,
        principal: &autoxide_kernel::access::Principal,
        id: Uuid,
    ) -> Result<Dossier, Error> {
        let site = self.get_website(tenant, principal, id).await?;
        let at = Timestamp::now();
        let checklist = self.checklist_of(tenant, &site, at).await?;
        let profile = self.profile_for_org(tenant, OrgId::from_uuid(site.org_id)).await?;
        let kyc = match &profile {
            Some(profile) => self.owner_verified(tenant, profile).await?,
            None => false,
        };
        let mut pages: Vec<LinkedPage> =
            self.linked_pages(tenant, &site).await?.into_values().collect();
        pages.sort_by(|a, b| a.kind.cmp(&b.kind));
        let complaints = self.policy(tenant, "complaints.contact").await?;
        let variables = profile
            .as_ref()
            .map(Self::template_variables_of)
            .unwrap_or_else(|| json!({"owner": {}, "contact": {}}));
        let address = profile.as_ref().and_then(address_of);
        let form = json!({
            "procedure": checklist.procedure,
            "owner": profile.as_ref().map(|profile| json!({
                "entity_type": profile.entity_type,
                "legal_name": profile.legal_name,
                "trade_name": profile.trade_name,
                "tax_code": profile.tax_code,
                "registration": {
                    "number": profile.registration_number,
                    "issued_on": profile.registration_issued_on,
                    "issued_by": profile.registration_issued_by,
                },
                "address": address,
                "address_line": address.as_ref().map(address_line),
                "representative": {
                    "name": profile.representative_name,
                    "title": profile.representative_title,
                },
                "phone": profile.phone,
                "email": profile.email,
            })),
            "website": {
                "domain": site.domain,
                "channel": site.channel,
                "kind": site.website_kind,
                "status": site.moit_status,
                "reference": site.moit_reference,
            },
            "identity_verified": kyc,
            "pages": pages.iter().map(|page| json!({
                "kind": page.kind, "title": page.title, "url": page.url,
            })).collect::<Vec<_>>(),
            "complaints_contact": complaints,
        });
        let context = json!({
            "disclaimer": DISCLAIMER,
            "portal_url": "https://online.gov.vn",
            "owner": variables["owner"],
            "contact": variables["contact"],
            "website": {"domain": site.domain, "channel": site.channel, "kind": site.website_kind},
            "pages": pages,
            "complaints": {"contact": complaints.as_ref().map(|value| match value {
                Value::Object(map) => map
                    .values()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", "),
                other => other.to_string(),
            })},
        });
        let render = |source: &str| self.templates.render(source, Output::Text, "vi", &context);
        let mut documents = vec![DossierDocument {
            key: "declaration".to_owned(),
            title: "Thông tin khai báo".to_owned(),
            format: "markdown".to_owned(),
            body: render(DECLARATION)?,
        }];
        if site.website_kind == "platform" {
            documents.push(DossierDocument {
                key: "project_plan".to_owned(),
                title: "Đề án cung cấp dịch vụ (nháp)".to_owned(),
                format: "markdown".to_owned(),
                body: render(PROJECT_PLAN)?,
            });
            documents.push(DossierDocument {
                key: "platform_rules".to_owned(),
                title: "Quy chế quản lý hoạt động (nháp)".to_owned(),
                format: "markdown".to_owned(),
                body: render(PLATFORM_RULES)?,
            });
        }
        Ok(Dossier {
            website_id: site.id,
            generated_at: crate::service::now(),
            disclaimer: DISCLAIMER.to_owned(),
            form,
            documents,
            checklist,
        })
    }
}

/// Bước kiểm `compliance.moit_readiness` của `autoxide doctor`: website chưa nộp còn thiếu mục, hay
/// chưa có website nào có hồ sơ → cảnh báo (không bao giờ `failed`: đây là việc của chủ website).
pub(crate) struct ReadinessProbe {
    pub service: Arc<ComplianceService>,
    pub databases: DatabaseRegistry,
}

#[async_trait::async_trait]
impl Probe for ReadinessProbe {
    async fn probe(&self) -> Finding {
        let databases = match self.databases.databases().await {
            Ok(databases) => databases,
            Err(error) => return Finding::failed(format!("cannot list the databases: {error}")),
        };
        let (mut total, mut lacking) = (0usize, Vec::new());
        for (_, name, database) in databases {
            // tenant-scope: việc nền / kiểm tra quét mọi khách của database, chủ ý không lọc khách (rà soát bảo mật 2026-11)
            let rows = match website::Entity::find()
                .order_by_asc(website::Column::Id)
                .all(database.read())
                .await
            {
                Ok(rows) => rows,
                Err(error) => {
                    return Finding::failed(format!("cannot read database `{name}`: {error}"));
                }
            };
            total += rows.len();
            for row in rows.into_iter().filter(|row| {
                matches!(row.moit_status.as_str(), "not_submitted" | "needs_supplement")
            }) {
                let tenant = match self.databases.tenant(TenantId::from_uuid(row.tenant_id)).await {
                    Ok(tenant) => tenant,
                    // Khách đã xoá / dời: không phải việc của bước này.
                    Err(_) => continue,
                };
                match self.service.checklist_of(&tenant, &row, Timestamp::now()).await {
                    Ok(checklist) if checklist.missing > 0 => lacking.push(format!(
                        "{} ({} missing; GET /api/v1/compliance/websites/{}/checklist)",
                        row.domain, checklist.missing, row.id
                    )),
                    Ok(_) => {}
                    Err(error) => {
                        return Finding::failed(format!(
                            "cannot check website {}: {error}",
                            row.domain
                        ));
                    }
                }
            }
        }
        if total == 0 {
            return Finding::warning(
                "No website has a legal profile for the Ministry of Industry and Trade yet (Vietnam: \
                 a sales website must be notified, a marketplace registered).",
            )
            .with_hint(
                "Add the owner's legal profile (`POST /api/v1/compliance/legal-profiles`) and the \
                 website (`POST /api/v1/compliance/websites`); see docs/COMPLIANCE.md.",
            );
        }
        if !lacking.is_empty() {
            let count = lacking.len();
            lacking.truncate(5);
            return Finding::warning(format!(
                "{count} website(s) not yet submitted still lack checklist items: {}.",
                lacking.join("; ")
            ))
            .with_hint("Fill what each checklist reports missing, then submit on online.gov.vn and record the status.");
        }
        Finding::ok(format!("{total} website record(s); none not yet submitted lacks an item."))
    }
}
