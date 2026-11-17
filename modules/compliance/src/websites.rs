//! Website theo tên miền và trạng thái với Bộ Công Thương (bước 37, D-357): loại `sales` (website bán
//! hàng → thông báo, logo xanh) hay `platform` (sàn / trung gian → đăng ký, logo đỏ); trang bắt buộc
//! gắn với website; trạng thái và logo cập nhật tay (con người nộp hồ sơ trên online.gov.vn — không
//! có API công khai).
//!
//! Đoạn mã logo là HTML người dùng dán: không bao giờ lưu hay trả lại nguyên văn — chỉ lấy hai URL
//! (link, ảnh) đã kiểm miền theo quy tắc `moit.checklist.*` rồi dựng lại.

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{
    OrgId, OrgTree, ScopeExt, TenantContext, normalize_domain, root_of,
};
use chrono::{DateTime, Utc};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::PAGE_KINDS;
use crate::entity::{website, website_page};
use crate::events;
use crate::permissions::{LEGAL_MANAGE, LEGAL_READ};
use crate::service::{ComplianceService, now};

pub(crate) const KINDS: [&str; 2] = ["sales", "platform"];
pub(crate) const CHANNELS: [&str; 2] = ["website", "mobile_app"];
pub(crate) const STATUSES: [&str; 7] = [
    "not_submitted",
    "submitted",
    "needs_supplement",
    "notified",
    "registered",
    "rejected",
    "revoked",
];

/// Loại đối tượng của trang bắt buộc (module `content`).
const PAGE_RESOURCE: &str = "content.entry";

pub(crate) fn website_not_found() -> Error {
    Error::not_found("compliance.website_not_found", "There is no such website.")
}

fn domain_taken(domain: &str) -> Error {
    Error::conflict(
        "compliance.domain_taken",
        "The domain `{domain}` already has a website record in this tenant.",
    )
    .with_param("domain", domain)
}

fn logo_snippet_invalid(reason: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "compliance.logo_snippet_invalid",
        "The logo code is not the one the Ministry of Industry and Trade gives ({reason}). Paste it \
         unchanged, or send `link_url` and `image_url` instead.",
    )
    .with_param("reason", reason)
}

/// Website mới.
#[derive(Debug, Clone)]
pub(crate) struct NewWebsite {
    pub org_id: Option<Uuid>,
    pub domain: String,
    pub kind: String,
    pub channel: Option<String>,
}

/// Sửa website; `None` giữ nguyên.
#[derive(Debug, Clone, Default)]
pub(crate) struct WebsiteChange {
    pub org_id: Option<Uuid>,
    pub domain: Option<String>,
    pub kind: Option<String>,
    pub channel: Option<String>,
}

/// Trạng thái mới với Bộ Công Thương.
#[derive(Debug, Clone, Default)]
pub(crate) struct StatusChange {
    pub status: String,
    pub reference: Option<String>,
    pub link_url: Option<String>,
    pub image_url: Option<String>,
    pub snippet: Option<String>,
}

/// Logo đã dựng lại từ URL đã kiểm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Logo {
    pub color: String,
    pub alt: String,
    pub link_url: String,
    pub image_url: String,
    pub snippet: String,
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\'', "&#39;")
}

/// Đoạn mã logo dựng lại: chỉ thẻ `a` và `img`, giá trị đã thoát.
pub(crate) fn logo_snippet(link: &str, image: &str, alt: &str) -> String {
    format!(
        "<a href=\"{}\" target=\"_blank\" rel=\"noopener\"><img src=\"{}\" alt=\"{}\" title=\"{}\"></a>",
        escape(link),
        escape(image),
        escape(alt),
        escape(alt)
    )
}

/// Một thẻ đã đọc: tên (chữ thường) và thuộc tính.
struct Tag {
    name: String,
    attributes: Vec<(String, String)>,
}

/// Đọc một thẻ mở `<name a="…" b='…' c=…>` (hay `/>`) ở đầu `input`; trả thẻ và phần còn lại.
fn read_tag(input: &str) -> Result<(Tag, &str), &'static str> {
    let rest = input.trim_start().strip_prefix('<').ok_or("expected a tag")?;
    let name_end =
        rest.find(|c: char| !c.is_ascii_alphanumeric()).ok_or("the tag is not closed")?;
    let name = rest[..name_end].to_ascii_lowercase();
    if name.is_empty() {
        return Err("expected a tag name");
    }
    let mut rest = &rest[name_end..];
    let mut attributes = Vec::new();
    loop {
        rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix("/>") {
            return Ok((Tag { name, attributes }, after));
        }
        if let Some(after) = rest.strip_prefix('>') {
            return Ok((Tag { name, attributes }, after));
        }
        let key_end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .ok_or("the tag is not closed")?;
        if key_end == 0 {
            return Err("unexpected character in a tag");
        }
        let key = rest[..key_end].to_ascii_lowercase();
        rest = rest[key_end..].trim_start();
        let value = if let Some(after) = rest.strip_prefix('=') {
            let after = after.trim_start();
            let (value, remaining) = match after.chars().next() {
                Some(quote @ ('"' | '\'')) => {
                    let body = &after[1..];
                    let end = body.find(quote).ok_or("an attribute value is not closed")?;
                    (&body[..end], &body[end + 1..])
                }
                Some(_) => {
                    let end = after
                        .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
                        .ok_or("the tag is not closed")?;
                    (&after[..end], &after[end..])
                }
                None => return Err("the tag is not closed"),
            };
            rest = remaining;
            value.to_owned()
        } else {
            String::new()
        };
        if value.contains(['<', '>']) {
            return Err("an attribute holds markup");
        }
        attributes.push((key, value));
    }
}

/// Đọc đoạn mã logo: đúng một `<a href=…>` bọc đúng một `<img src=…>`, chỉ thuộc tính vô hại.
/// Trả (link, ảnh). Mọi thứ khác (script, `onerror`, `style`, nội dung thêm) bị từ chối.
pub(crate) fn parse_logo_snippet(snippet: &str) -> Result<(String, String), &'static str> {
    if snippet.len() > 2000 {
        return Err("too long");
    }
    let (anchor, rest) = read_tag(snippet)?;
    if anchor.name != "a" {
        return Err("it must start with an `a` element");
    }
    let (image, rest) = read_tag(rest)?;
    if image.name != "img" {
        return Err("the link must hold only an `img` element");
    }
    let rest = rest.trim_start();
    let rest = rest.strip_prefix("</img>").unwrap_or(rest).trim_start();
    let rest = rest
        .get(..4)
        .filter(|close| close.eq_ignore_ascii_case("</a>"))
        .map(|_| &rest[4..])
        .ok_or("the link must hold only an `img` element")?;
    if !rest.trim().is_empty() {
        return Err("there is more after the link");
    }
    let pick = |tag: &Tag, wanted: &str, allowed: &[&str]| -> Result<String, &'static str> {
        let mut found = None;
        for (key, value) in &tag.attributes {
            if !allowed.contains(&key.as_str()) {
                return Err("an attribute is not allowed");
            }
            if key == wanted {
                if found.is_some() {
                    return Err("an attribute is repeated");
                }
                found = Some(value.trim().to_owned());
            }
        }
        found.filter(|value| !value.is_empty()).ok_or("a link or image address is missing")
    };
    let link = pick(&anchor, "href", &["href", "target", "rel", "title"])?;
    let image = pick(&image, "src", &["src", "alt", "title", "width", "height", "border"])?;
    Ok((link, image))
}

/// URL `http(s)://<host trong danh sách>/…`, không có tài khoản, cổng lạ hay ký tự điều khiển.
pub(crate) fn url_allowed(url: &str, hosts: &[String]) -> bool {
    if url.len() > 500 || url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return false;
    }
    let Some(rest) = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")) else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return false;
    }
    let host = authority.to_ascii_lowercase();
    hosts.iter().any(|allowed| allowed.eq_ignore_ascii_case(&host))
}

/// Phần logo của quy tắc `moit.checklist.<kind>`.
pub(crate) struct LogoRule {
    pub color: String,
    pub alt: Value,
    pub link_hosts: Vec<String>,
    pub image_hosts: Vec<String>,
    /// Trạng thái khi xong thủ tục: `notified` (bán hàng) hay `registered` (sàn).
    pub done_status: String,
}

impl LogoRule {
    pub(crate) fn of(kind: &str, body: Option<&Value>) -> LogoRule {
        let hosts = |key: &str| -> Vec<String> {
            body.and_then(|body| body["logo"][key].as_array())
                .map(|items| items.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                .unwrap_or_default()
        };
        let default_status = if kind == "platform" { "registered" } else { "notified" };
        LogoRule {
            color: body
                .and_then(|body| body["logo"]["color"].as_str())
                .unwrap_or(if kind == "platform" { "red" } else { "blue" })
                .to_owned(),
            alt: body.map(|body| body["logo"]["alt"].clone()).unwrap_or(Value::Null),
            link_hosts: hosts("link_hosts"),
            image_hosts: hosts("image_hosts"),
            done_status: body
                .and_then(|body| body["status_when_done"].as_str())
                .unwrap_or(default_status)
                .to_owned(),
        }
    }

    fn alt(&self, locale: &str) -> String {
        self.alt[locale].as_str().or_else(|| self.alt["vi"].as_str()).unwrap_or_default().to_owned()
    }

    pub(crate) fn logo(&self, row: &website::Model, locale: &str) -> Option<Logo> {
        let (Some(link), Some(image)) = (&row.logo_link_url, &row.logo_image_url) else {
            return None;
        };
        if row.moit_status != self.done_status {
            return None;
        }
        let alt = self.alt(locale);
        Some(Logo {
            color: self.color.clone(),
            snippet: logo_snippet(link, image, &alt),
            alt,
            link_url: link.clone(),
            image_url: image.clone(),
        })
    }
}

impl ComplianceService {
    /// Quy tắc logo của loại website `kind` hiệu lực hôm nay.
    pub(crate) async fn logo_rule(
        &self,
        tenant: &TenantContext,
        kind: &str,
    ) -> Result<LogoRule, Error> {
        let rule = self.rule_for(tenant, &format!("moit.checklist.{kind}"), Utc::now()).await?;
        let body = rule.and_then(|row| serde_json::from_str::<Value>(&row.body).ok());
        Ok(LogoRule::of(kind, body.as_ref()))
    }

    pub(crate) async fn website_org(
        &self,
        tenant: &TenantContext,
        org: Option<Uuid>,
    ) -> Result<OrgId, Error> {
        let root = root_of(tenant.id());
        match org.map(OrgId::from_uuid) {
            None => Ok(root),
            Some(org) if org == root => Ok(root),
            Some(org) => {
                let lineage =
                    OrgTree::ancestors(tenant.database().read(), tenant.id(), org).await?;
                if lineage.is_empty() {
                    Err(autoxide_kernel::tenancy::site_not_found(org))
                } else {
                    Ok(org)
                }
            }
        }
    }

    fn check_website_fields(
        domain: Option<&str>,
        kind: Option<&str>,
        channel: Option<&str>,
        problems: &mut Vec<FieldError>,
    ) -> Option<String> {
        let mut normalized = None;
        if let Some(domain) = domain {
            match normalize_domain(domain) {
                Some(domain) => normalized = Some(domain),
                None => problems.push(FieldError::new(
                    "domain",
                    "invalid",
                    "must be a domain name such as `shop.example` (international names in punycode)",
                )),
            }
        }
        if kind.is_some_and(|kind| !KINDS.contains(&kind)) {
            problems.push(FieldError::new("kind", "invalid", "must be `sales` or `platform`"));
        }
        if channel.is_some_and(|channel| !CHANNELS.contains(&channel)) {
            problems.push(FieldError::new(
                "channel",
                "invalid",
                "must be `website` or `mobile_app`",
            ));
        }
        normalized
    }

    pub(crate) async fn create_website(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewWebsite,
    ) -> Result<website::Model, Error> {
        principal.require(&LEGAL_MANAGE)?;
        let mut problems = Vec::new();
        let domain = Self::check_website_fields(
            Some(&input.domain),
            Some(&input.kind),
            input.channel.as_deref(),
            &mut problems,
        );
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let org = self.website_org(tenant, input.org_id).await?;
        principal
            .require_at(tenant.database().read(), &LEGAL_MANAGE, OrgTarget::new(org, None))
            .await?;
        let domain = domain.unwrap_or_default();
        let at = now();
        let row = website::Model {
            id: Uuid::now_v7(),
            tenant_id: tenant.id().as_uuid(),
            org_id: org.as_uuid(),
            domain: domain.clone(),
            channel: input.channel.unwrap_or_else(|| "website".to_owned()),
            website_kind: input.kind,
            moit_status: "not_submitted".to_owned(),
            moit_reference: None,
            submitted_at: None,
            decided_at: None,
            logo_link_url: None,
            logo_image_url: None,
            checklist_version: None,
            created_at: at,
            updated_at: at,
        };
        let transaction = tenant.database().begin_write().await?;
        let saved =
            website::ActiveModel::from(row).insert(&transaction).await.map_err(|error| {
                if crate::service::is_unique_violation(&error) {
                    domain_taken(&domain)
                } else {
                    error.into()
                }
            })?;
        let event = events::WebsiteCreated(json!({
            "website_id": saved.id, "org_id": saved.org_id, "domain": saved.domain,
            "kind": saved.website_kind,
        }));
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                Some(format!("compliance.website:{}", saved.id)),
                &event,
            )
            .await?;
        transaction.commit().await?;
        Ok(saved)
    }

    /// Website `id` người gọi được dùng với `permission`; ngoài phạm vi → 404.
    pub(crate) async fn website_in_scope(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        permission: &autoxide_kernel::access::Permission,
        id: Uuid,
    ) -> Result<website::Model, Error> {
        principal.require(permission)?;
        let row = website::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(website_not_found)?;
        principal
            .require_at(
                tenant.database().read(),
                permission,
                OrgTarget::new(OrgId::from_uuid(row.org_id), None),
            )
            .await
            .map_err(|error| {
                if error.code() == "org_out_of_scope" { website_not_found() } else { error }
            })?;
        Ok(row)
    }

    pub(crate) async fn get_website(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<website::Model, Error> {
        self.website_in_scope(tenant, principal, &LEGAL_READ, id).await
    }

    pub(crate) async fn list_websites(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<website::Model>, Error> {
        let scope = principal.scope_condition(&LEGAL_READ, website::Column::OrgId, None)?;
        let select = website::Entity::find().scoped(tenant).filter(scope);
        fetch_page(select, website::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Ghi `row` nếu bản ghi còn ở phiên bản `version`, kèm event; 0 dòng → 412.
    async fn save_website<E: autoxide_kernel::event::Event>(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        row: &website::Model,
        version: DateTime<Utc>,
        event: &E,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        let result = website::Entity::update_many()
            .set(website::ActiveModel::from(row.clone()).reset_all())
            .filter(website::Column::TenantId.eq(tenant.id().as_uuid()))
            .filter(website::Column::Id.eq(row.id))
            .filter(website::Column::UpdatedAt.eq(version))
            .exec(&transaction)
            .await
            .map_err(|error| {
                if crate::service::is_unique_violation(&error) {
                    domain_taken(&row.domain)
                } else {
                    error.into()
                }
            })?;
        if result.rows_affected == 0 {
            return Err(autoxide_kernel::version::changed());
        }
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                Some(format!("compliance.website:{}", row.id)),
                event,
            )
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub(crate) async fn update_website(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        expected: Option<DateTime<Utc>>,
        change: WebsiteChange,
    ) -> Result<website::Model, Error> {
        let before = self.website_in_scope(tenant, principal, &LEGAL_MANAGE, id).await?;
        let mut problems = Vec::new();
        let domain = Self::check_website_fields(
            change.domain.as_deref(),
            change.kind.as_deref(),
            change.channel.as_deref(),
            &mut problems,
        );
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let mut row = before.clone();
        if let Some(org) = change.org_id {
            let org = self.website_org(tenant, Some(org)).await?;
            principal
                .require_at(tenant.database().read(), &LEGAL_MANAGE, OrgTarget::new(org, None))
                .await?;
            row.org_id = org.as_uuid();
        }
        if let Some(domain) = domain {
            row.domain = domain;
        }
        if let Some(kind) = change.kind {
            if kind != row.website_kind {
                // Loại đổi thì thủ tục đổi (thông báo ↔ đăng ký): làm lại từ đầu.
                row.moit_status = "not_submitted".to_owned();
                row.logo_link_url = None;
                row.logo_image_url = None;
                row.decided_at = None;
            }
            row.website_kind = kind;
        }
        if let Some(channel) = change.channel {
            row.channel = channel;
        }
        row.updated_at = now();
        let event = events::WebsiteUpdated(json!({
            "website_id": row.id, "domain": row.domain, "kind": row.website_kind,
        }));
        self.save_website(tenant, principal, &row, expected.unwrap_or(before.updated_at), &event)
            .await?;
        Ok(row)
    }

    /// Trạng thái với Bộ Công Thương. `notified` / `registered` cần logo (đoạn mã hay hai URL) trên
    /// miền cho phép của quy tắc; trạng thái khác bỏ logo.
    pub(crate) async fn set_website_status(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        expected: Option<DateTime<Utc>>,
        change: StatusChange,
    ) -> Result<website::Model, Error> {
        let before = self.website_in_scope(tenant, principal, &LEGAL_MANAGE, id).await?;
        let rule = self.logo_rule(tenant, &before.website_kind).await?;
        let mut problems = Vec::new();
        if !STATUSES.contains(&change.status.as_str()) {
            problems.push(
                FieldError::new("status", "invalid", "must be one of {values}")
                    .with_param("values", STATUSES.join(", ")),
            );
        }
        let other_done = if before.website_kind == "platform" { "notified" } else { "registered" };
        if change.status == other_done {
            problems.push(
                FieldError::new(
                    "status",
                    "kind_mismatch",
                    "a `{kind}` website ends with `{expected}`, not this status",
                )
                .with_param("kind", &before.website_kind)
                .with_param("expected", &rule.done_status),
            );
        }
        let reference = change.reference.as_deref().map(str::trim).filter(|r| !r.is_empty());
        if reference.is_some_and(|reference| reference.chars().count() > 100) {
            problems.push(FieldError::new("reference", "length", "must be at most 100 characters"));
        }
        let mut row = before.clone();
        let done = change.status == rule.done_status;
        if done {
            let (link, image) = match change.snippet.as_deref().map(str::trim) {
                Some(snippet) if !snippet.is_empty() => {
                    parse_logo_snippet(snippet).map_err(logo_snippet_invalid)?
                }
                _ => match (change.link_url, change.image_url) {
                    (Some(link), Some(image)) => (link.trim().to_owned(), image.trim().to_owned()),
                    _ => {
                        problems.push(FieldError::new(
                            "snippet",
                            "required",
                            "send the logo code the ministry gave (or `link_url` and `image_url`)",
                        ));
                        (String::new(), String::new())
                    }
                },
            };
            if !link.is_empty() && !url_allowed(&link, &rule.link_hosts) {
                return Err(logo_snippet_invalid("the link is not on the ministry's site"));
            }
            if !image.is_empty() && !url_allowed(&image, &rule.image_hosts) {
                return Err(logo_snippet_invalid("the image is not on the ministry's site"));
            }
            row.logo_link_url = Some(link);
            row.logo_image_url = Some(image);
        } else {
            row.logo_link_url = None;
            row.logo_image_url = None;
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let at = now();
        match change.status.as_str() {
            "submitted" => {
                row.submitted_at = Some(at);
                let version = self
                    .rule_for(tenant, &format!("moit.checklist.{}", row.website_kind), Utc::now())
                    .await?
                    .and_then(|rule| i32::try_from(rule.version).ok());
                row.checklist_version = version;
            }
            "notified" | "registered" | "rejected" | "revoked" => row.decided_at = Some(at),
            _ => {}
        }
        if let Some(reference) = reference {
            row.moit_reference = Some(reference.to_owned());
        }
        row.moit_status = change.status;
        row.updated_at = at;
        let event = events::WebsiteStatusChanged(json!({
            "website_id": row.id, "domain": row.domain, "kind": row.website_kind,
            "status": row.moit_status, "previous_status": before.moit_status,
        }));
        self.save_website(tenant, principal, &row, expected.unwrap_or(before.updated_at), &event)
            .await?;
        Ok(row)
    }

    pub(crate) async fn website_pages(
        &self,
        tenant: &TenantContext,
        website: Uuid,
    ) -> Result<Vec<website_page::Model>, Error> {
        Ok(website_page::Entity::find()
            .scoped(tenant)
            .filter(website_page::Column::WebsiteId.eq(website))
            .order_by_asc(website_page::Column::PageKind)
            .all(tenant.database().read())
            .await?)
    }

    /// Gắn trang `entry` (bài / trang của `content`) làm trang `page_kind` của website.
    pub(crate) async fn link_page(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        page_kind: &str,
        entry: Uuid,
    ) -> Result<website_page::Model, Error> {
        let site = self.website_in_scope(tenant, principal, &LEGAL_MANAGE, id).await?;
        if !PAGE_KINDS.contains(&page_kind) {
            return Err(Error::validation(vec![
                FieldError::new("page_kind", "invalid", "must be one of {values}")
                    .with_param("values", PAGE_KINDS.join(", ")),
            ]));
        }
        // Gọi module khác qua sổ đối tượng, NGOÀI transaction ghi.
        let exists = self.resources.knows(PAGE_RESOURCE)
            && self.resources.resolve(tenant, PAGE_RESOURCE, &entry.to_string()).await?.is_some();
        if !exists {
            return Err(Error::validation(vec![FieldError::new(
                "entry_id",
                "not_found",
                "is not a page or post of this tenant",
            )]));
        }
        let row = website_page::Model {
            tenant_id: tenant.id().as_uuid(),
            website_id: site.id,
            page_kind: page_kind.to_owned(),
            resource: format!("{PAGE_RESOURCE}:{entry}"),
            linked_at: now(),
        };
        let transaction = tenant.database().begin_write().await?;
        website_page::Entity::delete_many()
            .filter(website_page::Column::TenantId.eq(row.tenant_id))
            .filter(website_page::Column::WebsiteId.eq(row.website_id))
            .filter(website_page::Column::PageKind.eq(page_kind))
            .exec(&transaction)
            .await?;
        website_page::Entity::insert(website_page::ActiveModel::from(row.clone()))
            .exec_without_returning(&transaction)
            .await?;
        let event = events::WebsiteUpdated(json!({
            "website_id": site.id, "domain": site.domain, "kind": site.website_kind,
            "page_kind": page_kind,
        }));
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                Some(format!("compliance.website:{}", site.id)),
                &event,
            )
            .await?;
        transaction.commit().await?;
        Ok(row)
    }

    pub(crate) async fn unlink_page(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        page_kind: &str,
    ) -> Result<(), Error> {
        let site = self.website_in_scope(tenant, principal, &LEGAL_MANAGE, id).await?;
        let transaction = tenant.database().begin_write().await?;
        let result = website_page::Entity::delete_many()
            .filter(website_page::Column::TenantId.eq(tenant.id().as_uuid()))
            .filter(website_page::Column::WebsiteId.eq(site.id))
            .filter(website_page::Column::PageKind.eq(page_kind))
            .exec(&transaction)
            .await?;
        if result.rows_affected > 0 {
            let event = events::WebsiteUpdated(json!({
                "website_id": site.id, "domain": site.domain, "kind": site.website_kind,
                "page_kind": page_kind,
            }));
            self.outbox
                .record(
                    &transaction,
                    tenant,
                    Actor::from(principal),
                    Some(format!("compliance.website:{}", site.id)),
                    &event,
                )
                .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Website công khai của request: tên miền nêu rõ (`domain`), tên miền của request, rồi website
    /// đã xong thủ tục của nút `org` (website đang xem).
    pub(crate) async fn public_website(
        &self,
        tenant: &TenantContext,
        domains: &[String],
        org: OrgId,
    ) -> Result<website::Model, Error> {
        let read = tenant.database().read();
        for domain in domains {
            if let Some(domain) = normalize_domain(domain) {
                let found = website::Entity::find()
                    .scoped(tenant)
                    .filter(website::Column::Domain.eq(domain))
                    .one(read)
                    .await?;
                if let Some(found) = found {
                    return Ok(found);
                }
            }
        }
        website::Entity::find()
            .scoped(tenant)
            .filter(website::Column::OrgId.eq(org.as_uuid()))
            .filter(website::Column::MoitStatus.is_in(["notified", "registered"]))
            .order_by_asc(website::Column::Id)
            .one(read)
            .await?
            .ok_or_else(website_not_found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hosts() -> Vec<String> {
        vec!["online.gov.vn".to_owned()]
    }

    #[test]
    fn snippet_that_cua_bo_doc_duoc() {
        let snippet = "<a href='http://online.gov.vn/Home/WebDetails/12345'><img alt='' title='' \
                       src='http://online.gov.vn/Content/EndUser/LogoCCDVSaleNoti/logoSaleNoti.png'/></a>";
        let (link, image) = parse_logo_snippet(snippet).unwrap();
        assert_eq!(link, "http://online.gov.vn/Home/WebDetails/12345");
        assert!(url_allowed(&link, &hosts()) && url_allowed(&image, &hosts()));
    }

    #[test]
    fn snippet_la_bi_tu_choi() {
        for bad in [
            "<script>alert(1)</script>",
            "<a href=\"https://online.gov.vn/x\"><img src=\"https://online.gov.vn/a.png\" onerror=\"alert(1)\"></a>",
            "<a href=\"https://online.gov.vn/x\" onclick=\"x()\"><img src=\"https://online.gov.vn/a.png\"></a>",
            "<a href=\"https://online.gov.vn/x\"><img src=\"https://online.gov.vn/a.png\"></a><script>x</script>",
            "<a href=\"https://online.gov.vn/x\"><b>x</b></a>",
            "<a href=\"https://online.gov.vn/x\"><img src=\"https://online.gov.vn/a.png\" style=\"x\"></a>",
            "<a href=\"x\"><img src=\"<svg>\"></a>",
        ] {
            assert!(parse_logo_snippet(bad).is_err(), "{bad}");
        }
        // Miền khác, `javascript:`, có tài khoản trong URL.
        for url in [
            "https://evil.example/x",
            "javascript:alert(1)",
            "https://online.gov.vn@evil.example/x",
            "https://online.gov.vn.evil.example/x",
        ] {
            assert!(!url_allowed(url, &hosts()), "{url}");
        }
    }

    #[test]
    fn snippet_dung_lai_da_thoat() {
        let html =
            logo_snippet("https://online.gov.vn/a?b=1&c=\"", "https://online.gov.vn/i.png", "<x>");
        assert!(html.contains("&amp;c=&quot;") && html.contains("alt=\"&lt;x&gt;\""), "{html}");
    }
}
