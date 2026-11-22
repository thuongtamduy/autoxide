//! Kiểm tra sức khoẻ website (bước 67, D-229, D-20261122-site-health): quét bản đã đăng của một website,
//! tìm việc cần sửa và đưa vào hộp việc (bước 51 P2) — mỗi phát hiện kèm cách sửa bằng endpoint sẵn có:
//!
//! - thiếu mô tả SEO, tiêu đề trùng trong cùng ngôn ngữ;
//! - ảnh thiếu `alt` (ảnh trong thân `rich`, khối ảnh / ảnh bìa / bộ ảnh của bố cục);
//! - link nội bộ gãy (đường dẫn `/…` không còn công khai và không có chuyển hướng);
//! - khối hay node nhúng bản ghi không còn công khai (sổ đối tượng);
//! - trang còn công khai mà khối đếm ngược đã qua mốc (chiến dịch đã hết).
//!
//! Chạy theo yêu cầu (`POST /site-health/{org_id}/run`, tối đa một lần mỗi giờ mỗi website); kết quả lần
//! gần nhất lưu một dòng mỗi website. Không gọi AI, không ghi gì vào bài.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use autoxide_kernel::access::{OrgTarget, Permission, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, Site, TenantContext};
use autoxide_kernel::time::{SignedDuration, Timestamp};
use autoxide_kernel::work::{Severity, WorkItem, WorkSource};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::{published, redirect, site_health};
use crate::entries::{body, layout};
use crate::permissions::{CONTENT_READ, CONTENT_WRITE};
use crate::publishing::reading::on_site;
use crate::service::ContentService;

/// Quét tối đa chừng này bản đã đăng mỗi lần (website rất lớn: phần còn lại ở lần sau).
const MAX_ENTRIES: u64 = 5000;
/// Mỗi website chạy lại sau ít nhất chừng này.
const MIN_INTERVAL: SignedDuration = SignedDuration::from_hours(1);
/// Mỗi loại phát hiện báo tối đa chừng này mục.
const MAX_PER_KIND: usize = 200;

pub(crate) type HealthReport = site_health::Model;

/// One problem found on the website.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub(crate) struct Finding {
    /// `seo_description_missing`, `title_duplicate`, `image_alt_missing`, `link_broken`,
    /// `record_not_public` or `countdown_over`.
    pub kind: String,
    /// The published entry it is on.
    pub entry_id: Uuid,
    pub locale: String,
    /// Public address of the entry.
    pub path: String,
    /// What is wrong: the broken link, the hidden record (`kind:id`), the image id…
    pub detail: String,
    /// How to fix it with the API, e.g. `PATCH /api/v1/content/entries/{id}`.
    pub fix: String,
}

fn finding(kind: &str, row: &published::Model, detail: impl Into<String>, fix: String) -> Finding {
    Finding {
        kind: kind.to_owned(),
        entry_id: row.entry_id,
        locale: row.locale.clone(),
        path: row.path.clone(),
        detail: detail.into(),
        fix,
    }
}

fn patch_fix(row: &published::Model) -> String {
    format!("PATCH /api/v1/content/entries/{} then publish it again", row.entry_id)
}

impl ContentService {
    /// Lần kiểm gần nhất của website `org`; `None` khi chưa kiểm. Cần `content.read` ở nút đó.
    pub(crate) async fn site_health(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Uuid,
    ) -> Result<Option<HealthReport>, Error> {
        principal.require(&CONTENT_READ)?;
        let site = Site::of(tenant, OrgId::from_uuid(org)).await?;
        let read = tenant.database().read();
        principal.require_at(read, &CONTENT_READ, OrgTarget::new(site.org(), None)).await?;
        Ok(site_health::Entity::find()
            .scoped(tenant)
            .filter(site_health::Column::OrgId.eq(org))
            .one(read)
            .await?)
    }

    /// Kiểm website `org` ngay. Cần `content.write` ở nút đó; mỗi website tối đa một lần mỗi giờ
    /// (`content.site_health_too_soon`). `now` là tham số để test được.
    pub(crate) async fn run_site_health(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Uuid,
        now: Timestamp,
    ) -> Result<HealthReport, Error> {
        principal.require(&CONTENT_WRITE)?;
        let site = Site::of(tenant, OrgId::from_uuid(org)).await?;
        let read = tenant.database().read();
        principal.require_at(read, &CONTENT_WRITE, OrgTarget::new(site.org(), None)).await?;
        let before = site_health::Entity::find()
            .scoped(tenant)
            .filter(site_health::Column::OrgId.eq(org))
            .one(read)
            .await?;
        if let Some(before) = &before
            && now - before.checked_at < MIN_INTERVAL
        {
            return Err(Error::too_many_requests(
                "content.site_health_too_soon",
                "This website was checked less than an hour ago; read the result with GET /api/v1/content/site-health.",
            ));
        }
        let findings = self.check_site(tenant, &site, now).await?;
        let text = serde_json::to_string(&findings).map_err(Error::internal)?;
        let count = i32::try_from(findings.len()).unwrap_or(i32::MAX);
        let transaction = tenant.database().begin_write().await?;
        let saved = match before {
            Some(row) => {
                let mut active: site_health::ActiveModel = row.into();
                active.findings = Set(text);
                active.findings_count = Set(count);
                active.checked_by = Set(Some(principal.user_id()));
                active.checked_at = Set(now);
                active.update(&transaction).await?
            }
            None => {
                site_health::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    org_id: Set(org),
                    findings: Set(text),
                    findings_count: Set(count),
                    checked_by: Set(Some(principal.user_id())),
                    checked_at: Set(now),
                }
                .insert(&transaction)
                .await?
            }
        };
        transaction.commit().await?;
        tracing::info!(org_id = %org, findings = count, "website checked");
        Ok(saved)
    }

    /// Quét bản đã đăng của website: vài câu truy vấn cho cả website, không mỗi bài một câu.
    async fn check_site(
        &self,
        tenant: &TenantContext,
        site: &Site,
        now: Timestamp,
    ) -> Result<Vec<Finding>, Error> {
        let read = tenant.database().read();
        let rows = published::Entity::find()
            .scoped(tenant)
            .filter(published::Column::RestrictedAt.is_null())
            .filter(on_site(&site.lineage_ids()))
            .order_by_asc(published::Column::EntryId)
            .limit(MAX_ENTRIES)
            .all(read)
            .await?;
        let paths: BTreeSet<(String, String)> =
            rows.iter().map(|row| (row.locale.clone(), row.path.clone())).collect();
        let redirects: BTreeSet<(String, String)> = redirect::Entity::find()
            .scoped(tenant)
            .all(read)
            .await?
            .into_iter()
            .map(|row| (row.locale, row.from_path))
            .collect();
        let mut by_kind: BTreeMap<String, Vec<Finding>> = BTreeMap::new();
        let mut push = |item: Finding| {
            let list = by_kind.entry(item.kind.clone()).or_default();
            if list.len() < MAX_PER_KIND {
                list.push(item);
            }
        };
        let mut titles: HashMap<(String, String), Vec<&published::Model>> = HashMap::new();
        let mut records: BTreeMap<String, Vec<(String, &published::Model)>> = BTreeMap::new();
        for row in &rows {
            if row.meta_description.trim().is_empty() && !row.noindex {
                push(finding(
                    "seo_description_missing",
                    row,
                    "no meta description",
                    patch_fix(row),
                ));
            }
            titles
                .entry((row.locale.clone(), row.title.trim().to_lowercase()))
                .or_default()
                .push(row);
            let parsed: Value = serde_json::from_str(&row.body).unwrap_or(Value::Null);
            let mut scan = Scan::default();
            scan.walk(&parsed);
            for id in scan.images_without_alt {
                push(finding("image_alt_missing", row, id, patch_fix(row)));
            }
            for href in scan.links {
                let path = href
                    .split(['?', '#'])
                    .next()
                    .unwrap_or_default()
                    .trim_end_matches('/')
                    .to_owned();
                let path = if path.is_empty() { "/".to_owned() } else { path };
                let key = (row.locale.clone(), path.clone());
                if !paths.contains(&key) && !redirects.contains(&key) {
                    push(finding("link_broken", row, href, patch_fix(row)));
                }
            }
            for ends_at in scan.countdowns {
                if Timestamp::parse_rfc3339(&ends_at).is_ok_and(|ends| ends < now) {
                    push(finding(
                        "countdown_over",
                        row,
                        ends_at,
                        format!(
                            "POST /api/v1/content/entries/{}/unpublish, or change the page",
                            row.entry_id
                        ),
                    ));
                }
            }
            for (kind, id) in body::stored_records(&row.body) {
                records.entry(kind).or_default().push((id, row));
            }
        }
        for ((_, _), same) in titles.iter().filter(|(_, same)| same.len() > 1) {
            for row in same {
                push(finding("title_duplicate", row, row.title.clone(), patch_fix(row)));
            }
        }
        // Bản ghi nhúng: một lần hỏi sổ đối tượng mỗi loại; loại không còn ai cung cấp cũng là không công khai.
        for (kind, uses) in records {
            let ids: Vec<String> = uses
                .iter()
                .map(|(id, _)| id.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            let found = if self.resources.knows(&kind) {
                self.resources.resolve_many(tenant, &kind, &ids, None).await?
            } else {
                Default::default()
            };
            for (id, row) in uses {
                if !found.contains_key(&id) {
                    push(finding("record_not_public", row, format!("{kind}:{id}"), patch_fix(row)));
                }
            }
        }
        Ok(by_kind.into_values().flatten().collect())
    }
}

/// Những gì cần soi trong một thân bài (`rich` hay `layout`).
#[derive(Default)]
struct Scan {
    images_without_alt: Vec<String>,
    links: Vec<String>,
    countdowns: Vec<String>,
}

impl Scan {
    fn walk(&mut self, value: &Value) {
        if layout::is_layout(value) {
            for (block, _, props) in layout::blocks_of(value) {
                match block {
                    "content.image" | "content.hero" => self.image(props),
                    "content.gallery" => {
                        for item in
                            props.get("items").and_then(Value::as_array).into_iter().flatten()
                        {
                            self.image(item);
                        }
                    }
                    "content.countdown" => {
                        if let Some(ends) = props.get("ends_at").and_then(Value::as_str) {
                            self.countdowns.push(ends.to_owned());
                        }
                    }
                    _ => {}
                }
                for key in ["link", "button_link", "policy_link"] {
                    if let Some(link) = props.get(key).and_then(Value::as_str) {
                        self.link(link);
                    }
                }
            }
            return;
        }
        let mut pending = vec![value];
        while let Some(node) = pending.pop() {
            if node.get("type").and_then(Value::as_str) == Some("image") {
                let alt = node.pointer("/attrs/alt").and_then(Value::as_str).unwrap_or_default();
                if alt.trim().is_empty() {
                    let id =
                        node.pointer("/attrs/mediaId").and_then(Value::as_str).unwrap_or_default();
                    self.images_without_alt.push(id.to_owned());
                }
            }
            for mark in node.get("marks").and_then(Value::as_array).into_iter().flatten() {
                if mark.get("type").and_then(Value::as_str) == Some("link")
                    && let Some(href) = mark.pointer("/attrs/href").and_then(Value::as_str)
                {
                    self.link(href);
                }
            }
            if let Some(children) = node.get("content").and_then(Value::as_array) {
                pending.extend(children.iter());
            }
        }
    }

    fn image(&mut self, props: &Value) {
        if let Some(id) = props.get("media_id").and_then(Value::as_str)
            && props.get("alt").and_then(Value::as_str).is_none_or(|alt| alt.trim().is_empty())
        {
            self.images_without_alt.push(id.to_owned());
        }
    }

    /// Chỉ link nội bộ (`/…`, không `//`) mới kiểm được ở đây.
    fn link(&mut self, href: &str) {
        if href.starts_with('/') && !href.starts_with("//") {
            self.links.push(href.to_owned());
        }
    }
}

/// `content.site_health`: website có phát hiện ở lần kiểm gần nhất, trong phạm vi `content.write`.
pub(crate) struct Health;

#[async_trait::async_trait]
impl WorkSource for Health {
    fn id(&self) -> &'static str {
        "content.site_health"
    }

    fn permission(&self) -> Permission {
        CONTENT_WRITE
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        _: OrgTarget,
    ) -> Result<u64, Error> {
        Ok(reports(tenant, principal, 1000)
            .await?
            .iter()
            .map(|row| u64::try_from(row.findings_count).unwrap_or(0))
            .sum())
    }

    async fn items(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        _: OrgTarget,
        limit: u16,
    ) -> Result<Vec<WorkItem>, Error> {
        Ok(reports(tenant, principal, u64::from(limit))
            .await?
            .into_iter()
            .map(|row| {
                WorkItem::new(
                    row.org_id.to_string(),
                    Severity::Info,
                    "Fix {count} problems found on the website",
                    "content.entries",
                )
                .param("count", row.findings_count.to_string())
            })
            .collect())
    }
}

async fn reports(
    tenant: &TenantContext,
    principal: &Principal,
    limit: u64,
) -> Result<Vec<HealthReport>, Error> {
    let scope = principal.scope_condition(&CONTENT_WRITE, site_health::Column::OrgId, None)?;
    Ok(site_health::Entity::find()
        .filter(scope)
        .scoped(tenant)
        .filter(site_health::Column::FindingsCount.gt(0))
        .order_by_asc(site_health::Column::OrgId)
        .limit(limit)
        .all(tenant.database().read())
        .await?)
}
