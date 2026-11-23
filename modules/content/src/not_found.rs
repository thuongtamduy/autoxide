//! Sức khoẻ website, phần sau (bước 67, D-20261123-site-health-daily):
//!
//! - **Đếm đường dẫn 404.** `GET /public/resolve` không thấy gì thì cộng một lượt vào dòng (website,
//!   ngôn ngữ + đường dẫn, ngày UTC) của `content_not_found_paths` — một câu `UPDATE` cộng dồn
//!   (`rows::adjust`), lần đầu trong ngày thì `INSERT … ON CONFLICT DO NOTHING`. Không lưu IP hay gì
//!   về người xem. Mỗi website tối đa [`MAX_PATHS_PER_DAY`] đường dẫn khác nhau mỗi ngày (bot quét
//!   đường dẫn ngẫu nhiên không làm bảng phình); giữ [`KEEP_DAYS`] ngày. Số đo
//!   `content_not_found_total` không có nhãn đường dẫn.
//! - **Gợi ý chuyển hướng.** Kiểm tra sức khoẻ cộng lượt 7 ngày gần nhất; đường dẫn đủ
//!   `AUTOXIDE_CONTENT_SITE_HEALTH_NOT_FOUND_MIN_HITS` lượt, chưa có chuyển hướng và vẫn chưa có trang
//!   thì gợi ý trang công khai có slug gần nhất cùng ngôn ngữ; người bấm tạo chuyển hướng.
//! - **Job hằng ngày** `content.site_health_daily`: kiểm mọi website có bài đã đăng hay có lượt 404
//!   (bỏ website đã kiểm trong [`DAILY_SKIP`]), rồi xoá dòng đếm cũ.

use std::collections::{BTreeMap, BTreeSet};

use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::rows::{self, Bounds};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, Site, TenantContext, TenantId, root_of};
use autoxide_kernel::time::{SignedDuration, Timestamp};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, Condition, EntityTrait, PaginatorTrait, QueryFilter, QuerySelect, Set};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::entity::{not_found_path, published, site_health};
use crate::health::Finding;
use crate::service::{ContentService, now};

pub(crate) const SITE_HEALTH_DAILY: &str = "content.site_health_daily";
/// Đường dẫn dài hơn thì không đếm (không phải đường dẫn thật của website).
const PATH_MAX: usize = 512;
/// Số đường dẫn khác nhau tối đa mỗi website mỗi ngày.
pub(crate) const MAX_PATHS_PER_DAY: u64 = 1000;
/// Giữ dòng đếm chừng này ngày.
const KEEP_DAYS: i64 = 30;
/// Cửa sổ cộng lượt khi kiểm.
const WINDOW_DAYS: i64 = 7;
/// Website đã kiểm (theo yêu cầu hay job) gần hơn chừng này thì job hằng ngày bỏ qua.
const DAILY_SKIP: SignedDuration = SignedDuration::from_hours(20);
/// Mỗi lần kiểm gợi ý tối đa chừng này đường dẫn.
const MAX_SUGGESTIONS: usize = 200;

/// Ngày UTC `YYYY-MM-DD` của `at`.
fn day_of(at: Timestamp) -> String {
    at.to_string().chars().take(10).collect()
}

fn path_hash(locale: &str, path: &str) -> String {
    let digest = Sha256::digest(format!("{locale}\n{path}").as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Phần cuối của đường dẫn (slug), để so độ gần.
fn last_segment(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or_default()
}

/// Khoảng cách Levenshtein (theo ký tự) — slug ngắn nên tính thẳng là đủ.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, left) in a.chars().enumerate() {
        let mut current = vec![i + 1];
        for (j, right) in b.iter().enumerate() {
            let cost = usize::from(left != *right);
            let value = (previous[j] + cost).min(previous[j + 1] + 1).min(current[j] + 1);
            current.push(value);
        }
        previous = current;
    }
    previous.last().copied().unwrap_or_default()
}

/// Trang công khai gần nhất với đường dẫn `missing` trong `pages` (cùng ngôn ngữ): slug cách nhau ít
/// nhất, không quá nửa độ dài slug (xa hơn thì không gợi ý).
fn nearest<'a>(missing: &str, pages: &[&'a published::Model]) -> Option<&'a published::Model> {
    let wanted = last_segment(missing).to_lowercase();
    if wanted.is_empty() {
        return None;
    }
    let limit = wanted.chars().count().div_ceil(2);
    pages
        .iter()
        .map(|row| (distance(&wanted, &last_segment(&row.path).to_lowercase()), *row))
        .filter(|(gap, _)| *gap <= limit)
        .min_by(|left, right| left.0.cmp(&right.0).then(left.1.path.cmp(&right.1.path)))
        .map(|(_, row)| row)
}

impl ContentService {
    /// Đếm một lượt hỏi đường dẫn không có (`locale`, `path`) của website `site`. Lỗi chỉ ghi log: không
    /// bao giờ làm hỏng câu trả lời 404.
    pub(crate) async fn record_not_found(
        &self,
        tenant: &TenantContext,
        site: &Site,
        locale: &str,
        path: &str,
    ) {
        metrics::counter!("content_not_found_total").increment(1);
        let path = path.split(['?', '#']).next().unwrap_or_default().trim().trim_end_matches('/');
        let path = if path.is_empty() { "/" } else { path };
        if !path.starts_with('/') || path.len() > PATH_MAX || locale.len() > 16 {
            return;
        }
        if let Err(error) = self.count_not_found(tenant, site, locale, path, now()).await {
            tracing::warn!(error = %error, "content could not count a missing address");
        }
    }

    pub(crate) async fn count_not_found(
        &self,
        tenant: &TenantContext,
        site: &Site,
        locale: &str,
        path: &str,
        at: Timestamp,
    ) -> Result<(), Error> {
        let org = site.org().as_uuid();
        let day = day_of(at);
        let hash = path_hash(locale, path);
        let key = || {
            Condition::all()
                .add(not_found_path::Column::TenantId.eq(tenant.id().as_uuid()))
                .add(not_found_path::Column::OrgId.eq(org))
                .add(not_found_path::Column::PathHash.eq(hash.as_str()))
                .add(not_found_path::Column::Day.eq(day.as_str()))
        };
        let transaction = tenant.database().begin_write().await?;
        let changed = rows::adjust::<not_found_path::Entity, _>(
            &transaction,
            not_found_path::Column::Hits,
            key(),
            1,
            Bounds::ANY,
        )
        .await?;
        if changed == 0 {
            let today = not_found_path::Entity::find()
                .scoped(tenant)
                .filter(not_found_path::Column::OrgId.eq(org))
                .filter(not_found_path::Column::Day.eq(day.as_str()))
                .count(&transaction)
                .await?;
            if today >= MAX_PATHS_PER_DAY {
                return Ok(());
            }
            let inserted = not_found_path::Entity::insert(not_found_path::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                org_id: Set(org),
                path_hash: Set(hash.clone()),
                locale: Set(locale.to_owned()),
                path: Set(path.to_owned()),
                day: Set(day.clone()),
                hits: Set(1),
            })
            .on_conflict(
                OnConflict::columns([
                    not_found_path::Column::TenantId,
                    not_found_path::Column::OrgId,
                    not_found_path::Column::PathHash,
                    not_found_path::Column::Day,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(&transaction)
            .await?;
            if inserted == 0 {
                // Một yêu cầu khác vừa chèn dòng (Postgres, nhiều kết nối): cộng vào dòng đó.
                rows::adjust::<not_found_path::Entity, _>(
                    &transaction,
                    not_found_path::Column::Hits,
                    key(),
                    1,
                    Bounds::ANY,
                )
                .await?;
            }
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Phát hiện `not_found_path` của website `site`: đường dẫn đủ lượt trong 7 ngày, chưa có trang hay
    /// chuyển hướng, có trang gần để gợi ý. `rows`: bản đã đăng của website; `redirects`: (ngôn ngữ,
    /// đường dẫn nguồn) đã có chuyển hướng.
    pub(crate) async fn not_found_findings(
        &self,
        tenant: &TenantContext,
        site: &Site,
        rows: &[published::Model],
        redirects: &BTreeSet<(String, String)>,
        at: Timestamp,
    ) -> Result<Vec<Finding>, Error> {
        let since = day_of(at - SignedDuration::from_hours(24 * (WINDOW_DAYS - 1)));
        let counted = not_found_path::Entity::find()
            .scoped(tenant)
            .filter(not_found_path::Column::OrgId.eq(site.org().as_uuid()))
            .filter(not_found_path::Column::Day.gte(since))
            .all(tenant.database().read())
            .await?;
        let mut hits: BTreeMap<(String, String), i64> = BTreeMap::new();
        for row in counted {
            *hits.entry((row.locale, row.path)).or_default() += row.hits;
        }
        let existing: BTreeSet<(&str, &str)> =
            rows.iter().map(|row| (row.locale.as_str(), row.path.as_str())).collect();
        let mut by_locale: BTreeMap<&str, Vec<&published::Model>> = BTreeMap::new();
        for row in rows {
            by_locale.entry(row.locale.as_str()).or_default().push(row);
        }
        let mut wanted: Vec<((String, String), i64)> = hits
            .into_iter()
            .filter(|(_, count)| *count >= self.config.not_found_min_hits)
            .filter(|(key, _)| !redirects.contains(key))
            .filter(|((locale, path), _)| !existing.contains(&(locale.as_str(), path.as_str())))
            .collect();
        // Nhiều lượt trước.
        wanted.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
        let mut found = Vec::new();
        for ((locale, path), count) in wanted {
            let pages = by_locale.get(locale.as_str()).map_or(&[][..], Vec::as_slice);
            let Some(target) = nearest(&path, pages) else { continue };
            found.push(Finding {
                kind: "not_found_path".to_owned(),
                entry_id: target.entry_id,
                locale: locale.clone(),
                path: path.clone(),
                detail: format!(
                    "{count} requests in the last {WINDOW_DAYS} days; nearest page: {}",
                    target.path
                ),
                fix: format!(
                    "POST /api/v1/content/redirects {{\"locale\": \"{locale}\", \"from_path\": \
                     \"{path}\", \"to_entry_id\": \"{}\"}}",
                    target.entry_id
                ),
            });
            if found.len() >= MAX_SUGGESTIONS {
                break;
            }
        }
        Ok(found)
    }

    /// Job hằng ngày: kiểm mọi website có bài đã đăng hay có lượt 404, rồi xoá dòng đếm cũ. Một website
    /// lỗi không chặn website khác.
    pub(crate) async fn daily_site_health(&self, database: &Database) -> Result<u64, Error> {
        let at = now();
        // tenant-scope: job hệ thống quét cả database để biết khách nào, website nào cần kiểm.
        let mut sites: BTreeSet<(Uuid, Option<Uuid>)> = published::Entity::find()
            .select_only()
            .column(published::Column::TenantId)
            .column(published::Column::OrgId)
            .distinct()
            .into_tuple::<(Uuid, Option<Uuid>)>()
            .all(database.read())
            .await?
            .into_iter()
            .collect();
        // tenant-scope: như trên, website chưa có bài nhưng có người hỏi đường dẫn.
        let asked: Vec<(Uuid, Uuid)> = not_found_path::Entity::find()
            .select_only()
            .column(not_found_path::Column::TenantId)
            .column(not_found_path::Column::OrgId)
            .distinct()
            .into_tuple()
            .all(database.read())
            .await?;
        sites.extend(asked.into_iter().map(|(tenant, org)| (tenant, Some(org))));
        let mut by_tenant: BTreeMap<Uuid, BTreeSet<Uuid>> = BTreeMap::new();
        for (tenant, org) in sites {
            let org = org.unwrap_or_else(|| root_of(TenantId::from_uuid(tenant)).as_uuid());
            by_tenant.entry(tenant).or_default().insert(org);
        }
        let mut checked = 0;
        for (id, orgs) in by_tenant {
            let tenant = match self.databases.tenant(TenantId::from_uuid(id)).await {
                Ok(tenant) => tenant,
                Err(error) => {
                    tracing::warn!(tenant_id = %id, error = %error, "daily website check skipped a tenant");
                    continue;
                }
            };
            for org in orgs {
                match self.daily_site(&tenant, org, at).await {
                    Ok(true) => checked += 1,
                    Ok(false) => {}
                    Err(error) => {
                        tracing::warn!(tenant_id = %id, org_id = %org, error = %error, "daily website check failed");
                    }
                }
            }
            self.prune_not_found(&tenant, at).await?;
        }
        tracing::info!(websites = checked, "daily website checks done");
        Ok(checked)
    }

    /// Kiểm website `org` nếu chưa kiểm gần đây. Trả `true` khi đã kiểm.
    async fn daily_site(
        &self,
        tenant: &TenantContext,
        org: Uuid,
        at: Timestamp,
    ) -> Result<bool, Error> {
        let site = match Site::of(tenant, OrgId::from_uuid(org)).await {
            Ok(site) => site,
            // Nút đã bị xoá: dữ liệu còn lại do việc dọn của nút lo.
            Err(error) if error.code() == "site_not_found" => return Ok(false),
            Err(error) => return Err(error),
        };
        let before = site_health::Entity::find()
            .scoped(tenant)
            .filter(site_health::Column::OrgId.eq(org))
            .one(tenant.database().read())
            .await?;
        if before.as_ref().is_some_and(|row| at - row.checked_at < DAILY_SKIP) {
            return Ok(false);
        }
        self.save_site_health(tenant, &site, before, None, at).await?;
        Ok(true)
    }

    /// Xoá dòng đếm cũ hơn [`KEEP_DAYS`] ngày.
    pub(crate) async fn prune_not_found(
        &self,
        tenant: &TenantContext,
        at: Timestamp,
    ) -> Result<u64, Error> {
        let oldest = day_of(at - SignedDuration::from_hours(24 * KEEP_DAYS));
        let transaction = tenant.database().begin_write().await?;
        let removed = not_found_path::Entity::delete_many()
            .scoped(tenant)
            .filter(not_found_path::Column::Day.lt(oldest))
            .exec(&transaction)
            .await?
            .rows_affected;
        transaction.commit().await?;
        Ok(removed)
    }
}

/// Bộ chạy job hằng ngày.
pub(crate) struct DailyHealth(pub std::sync::Arc<ContentService>);

#[async_trait::async_trait]
impl JobHandler for DailyHealth {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.daily_site_health(&job.database).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn khoang_cach_slug() {
        assert_eq!(distance("ao-thun", "ao-thun"), 0);
        assert_eq!(distance("ao-thun-nam", "ao-thun"), 4);
        assert_eq!(distance("", "abc"), 3);
        assert_eq!(last_segment("/san-pham/ao-thun/"), "ao-thun");
        assert_eq!(day_of(Timestamp::parse_rfc3339("1970-01-01T05:00:00Z").unwrap()), "1970-01-01");
        assert_eq!(path_hash("vi", "/a").len(), 64);
    }
}
