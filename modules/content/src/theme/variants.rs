//! Bản chủ đề theo lịch (bước 67, D-234, D-20261122-theme-variants): ghi đè một phần theme gốc của một
//! website (cùng bộ kiểm token), hiệu ứng trang trí trong danh sách cho phép, lịch, độ ưu tiên, trạng
//! thái `draft` / `active` / `paused`.
//!
//! Chọn bản hiệu lực lúc `t`: các bản `active` của website (và của nút cấp trên — chuỗi đặt Tết cho mọi
//! cửa hàng) có lịch phủ `t`; ưu tiên cao thắng, bằng nhau thì bản bắt đầu muộn hơn, rồi theo `id` — luôn
//! đúng MỘT kết quả, mọi máy chủ như nhau. Không bản nào phủ: theme gốc. Chỉ một bản áp tại một lúc.

use std::collections::BTreeMap;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, Site, TenantContext};
use autoxide_kernel::time::{self, Timestamp};
use jiff::tz::TimeZone;
use sea_orm::sea_query::Condition;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, Set,
};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::schedule::{self, Schedule};
use super::{presets, tokens};
use crate::entity::theme_variant;
use crate::events::{self, Change};
use crate::permissions::CONTENT_THEMES;
use crate::service::{ContentService, now};

/// Hiệu ứng trang trí cho phép: core chỉ lưu và trả TÊN, giao diện vẽ (`docs/PAGES.md`).
pub(crate) const EFFECTS: [&str; 6] =
    ["snow", "petals", "lanterns", "fireworks", "confetti", "hearts"];
pub(crate) const INTENSITIES: [&str; 2] = ["low", "medium"];
const NAME_MAX: usize = 120;

pub(crate) type Variant = theme_variant::Model;

/// Dữ liệu tạo / thay một bản chủ đề. Khi có `preset_key`, trường bỏ trống lấy từ chủ đề mẫu (lịch
/// theo năm `year`, mặc định năm nay ở múi giờ của website).
#[derive(Debug, Clone, Default)]
pub(crate) struct VariantInput {
    pub org_id: Option<Uuid>,
    pub name: Option<String>,
    pub overrides: Option<Map<String, Value>>,
    pub effect: Option<String>,
    pub effect_intensity: Option<String>,
    pub schedule: Option<Value>,
    pub priority: Option<i32>,
    pub preset_key: Option<String>,
    pub year: Option<i16>,
}

/// Bản đang áp cho một website lúc `t`, và mốc đổi kế tiếp.
#[derive(Debug, Clone)]
pub(crate) struct Applied {
    pub variant: Option<Variant>,
    /// Mốc bắt đầu / kết thúc gần nhất của các bản đang bật, trong 7 ngày tới.
    pub next_change_at: Option<Timestamp>,
}

pub(crate) fn variant_not_found(id: Uuid) -> Error {
    Error::not_found("content.theme_variant_not_found", "There is no theme variant {id}.")
        .with_param("id", id)
}

fn target(row: &Variant) -> OrgTarget {
    OrgTarget::new(OrgId::from_uuid(row.org_id), None)
}

pub(crate) fn overrides_of(row: &Variant) -> BTreeMap<String, Value> {
    serde_json::from_str(&row.overrides).unwrap_or_default()
}

pub(crate) fn schedule_of(row: &Variant) -> Option<Schedule> {
    serde_json::from_str::<Value>(&row.schedule).ok().and_then(|value| schedule::check(&value).ok())
}

/// Bản hiệu lực trong `rows` lúc `now` (đã lọc `active` hay bản xem trước), và mốc đổi kế tiếp.
pub(crate) fn select(rows: &[Variant], now: Timestamp, zone: &TimeZone) -> Applied {
    let mut best: Option<(&Variant, Timestamp)> = None;
    let mut next: Option<Timestamp> = None;
    for row in rows {
        let Some(schedule) = schedule_of(row) else { continue };
        if let Some(boundary) = schedule.next_boundary(now, zone) {
            next = Some(next.map_or(boundary, |known| known.min(boundary)));
        }
        let Some(window) = schedule.current(now, zone) else { continue };
        let better = best.is_none_or(|(known, known_start)| {
            (row.priority, window.start, row.id) > (known.priority, known_start, known.id)
        });
        if better {
            best = Some((row, window.start));
        }
    }
    Applied { variant: best.map(|(row, _)| row.clone()), next_change_at: next }
}

impl ContentService {
    /// Múi giờ IANA của website `org` (`organizations.timezone`, kế thừa theo cây); không có
    /// `organizations` hay không đọc được: UTC.
    pub(crate) async fn site_zone(&self, tenant: &TenantContext, org: OrgId) -> TimeZone {
        let Some(orgs) = &self.orgs else { return TimeZone::UTC };
        match orgs.setting(tenant, org, "organizations.timezone").await {
            Ok(found) => found
                .and_then(|setting| setting.value)
                .and_then(|value| value.as_str().and_then(time::zone))
                .unwrap_or(TimeZone::UTC),
            Err(error) => {
                tracing::warn!(%error, "cannot read the time zone of a unit; using UTC");
                TimeZone::UTC
            }
        }
    }

    /// Bản chủ đề đang bật của website `site` (của nút website và các nút cấp trên) có thể chạy lúc
    /// `now` hay sau đó — lịch `once` đã hết bị lọc trong database. Một câu truy vấn.
    pub(crate) async fn active_variants(
        &self,
        tenant: &TenantContext,
        site: &Site,
        now: Timestamp,
    ) -> Result<Vec<Variant>, Error> {
        Ok(theme_variant::Entity::find()
            .scoped(tenant)
            .filter(theme_variant::Column::OrgId.is_in(site.lineage_ids()))
            .filter(theme_variant::Column::Status.eq("active"))
            .filter(
                Condition::any()
                    .add(theme_variant::Column::EndsAt.is_null())
                    .add(theme_variant::Column::EndsAt.gt(now)),
            )
            .all(tenant.database().read())
            .await?)
    }

    /// Một trang bản chủ đề của website `org`. Cần `content.themes.manage` ở nút đó.
    pub(crate) async fn list_variants(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Uuid,
        page: &PageRequest,
    ) -> Result<Page<Variant>, Error> {
        let site = self.themes_site(tenant, principal, org).await?;
        let select = theme_variant::Entity::find()
            .scoped(tenant)
            .filter(theme_variant::Column::OrgId.eq(site.org().as_uuid()));
        let read = tenant.database().read();
        fetch_page(select, theme_variant::Column::Id, |row| row.id, page, read).await
    }

    /// Bản chủ đề `id`.
    pub(crate) async fn variant(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Variant, Error> {
        principal.require(&CONTENT_THEMES)?;
        let read = tenant.database().read();
        let row = find(read, tenant, id).await?;
        hidden(principal.require_at(read, &CONTENT_THEMES, target(&row)).await, id)?;
        Ok(row)
    }

    /// Tạo bản chủ đề NHÁP (bật bằng `activate`). Website có quá
    /// `AUTOXIDE_CONTENT_THEME_VARIANTS_MAX` bản → 409 `content.theme_variants_limit`.
    pub(crate) async fn create_variant(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: VariantInput,
    ) -> Result<Variant, Error> {
        let org = input
            .org_id
            .unwrap_or_else(|| autoxide_kernel::tenancy::root_of(tenant.id()).as_uuid());
        let site = self.themes_site(tenant, principal, org).await?;
        let zone = self.site_zone(tenant, site.org()).await;
        let fields = self.check_variant(tenant, &site, &zone, input, None).await?;
        let at = now();
        let transaction = tenant.database().begin_write().await?;
        principal
            .require_write_at(&transaction, &CONTENT_THEMES, OrgTarget::new(site.org(), None))
            .await?;
        let count = theme_variant::Entity::find()
            .scoped(tenant)
            .filter(theme_variant::Column::OrgId.eq(site.org().as_uuid()))
            .count(&transaction)
            .await?;
        let max = u64::from(self.config.theme_variants_max);
        if count >= max {
            return Err(Error::conflict(
                "content.theme_variants_limit",
                "This website already has {max} theme variants; delete one first.",
            )
            .with_param("max", max));
        }
        let row = theme_variant::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(site.org().as_uuid()),
            name: Set(fields.name),
            overrides: Set(fields.overrides),
            effect: Set(fields.effect),
            effect_intensity: Set(fields.effect_intensity),
            schedule: Set(fields.schedule),
            starts_at: Set(fields.starts_at),
            ends_at: Set(fields.ends_at),
            priority: Set(fields.priority),
            status: Set("draft".to_owned()),
            preset_key: Set(fields.preset_key),
            created_by: Set(Some(principal.user_id())),
            updated_by: Set(Some(principal.user_id())),
            created_at: Set(at),
            updated_at: Set(at),
        }
        .insert(&transaction)
        .await?;
        self.record_variant(&transaction, tenant, principal, None, Some(&row)).await?;
        transaction.commit().await?;
        Ok(row)
    }

    /// Thay bản chủ đề `id` (giữ trạng thái). `expected`: phiên bản đã đọc.
    pub(crate) async fn put_variant(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        input: VariantInput,
        expected: Option<Timestamp>,
    ) -> Result<Variant, Error> {
        let before = self.variant(tenant, principal, id).await?;
        let site = Site::of(tenant, OrgId::from_uuid(before.org_id)).await?;
        let zone = self.site_zone(tenant, site.org()).await;
        let fields = self.check_variant(tenant, &site, &zone, input, Some(&before)).await?;
        self.change_variant(tenant, principal, id, expected, |active| {
            active.name = Set(fields.name);
            active.overrides = Set(fields.overrides);
            active.effect = Set(fields.effect);
            active.effect_intensity = Set(fields.effect_intensity);
            active.schedule = Set(fields.schedule);
            active.starts_at = Set(fields.starts_at);
            active.ends_at = Set(fields.ends_at);
            active.priority = Set(fields.priority);
            active.preset_key = Set(fields.preset_key);
        })
        .await
    }

    /// Bật (`active`) hay tạm dừng (`paused`) bản `id`.
    pub(crate) async fn set_variant_status(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        status: &'static str,
        expected: Option<Timestamp>,
    ) -> Result<Variant, Error> {
        self.change_variant(tenant, principal, id, expected, |active| {
            active.status = Set(status.to_owned());
        })
        .await
    }

    /// Chép bản `id` thành bản NHÁP với lịch mới (vd sang năm sau).
    pub(crate) async fn copy_variant(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        schedule: Value,
    ) -> Result<Variant, Error> {
        let source = self.variant(tenant, principal, id).await?;
        let input = VariantInput {
            org_id: Some(source.org_id),
            name: Some(source.name.clone()),
            overrides: Some(overrides_of(&source).into_iter().collect()),
            effect: source.effect.clone(),
            effect_intensity: source.effect_intensity.clone(),
            schedule: Some(schedule),
            priority: Some(source.priority),
            preset_key: None,
            year: None,
        };
        let mut copied = self.create_variant(tenant, principal, input).await?;
        if source.preset_key.is_some() {
            // Giữ dấu mẫu gốc để biết bản chép từ mẫu nào (không đọc lại mẫu).
            copied = self
                .change_variant(tenant, principal, copied.id, Some(copied.updated_at), |active| {
                    active.preset_key = Set(source.preset_key.clone());
                })
                .await?;
        }
        Ok(copied)
    }

    /// Xoá bản `id`; bản đang bật → 409 `content.theme_variant_active` (tạm dừng trước).
    pub(crate) async fn delete_variant(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        expected: Option<Timestamp>,
    ) -> Result<(), Error> {
        principal.require(&CONTENT_THEMES)?;
        let transaction = tenant.database().begin_write().await?;
        let before = find(&transaction, tenant, id).await?;
        hidden(principal.require_at(&transaction, &CONTENT_THEMES, target(&before)).await, id)?;
        principal.require_write_at(&transaction, &CONTENT_THEMES, target(&before)).await?;
        if expected.is_some_and(|expected| expected != before.updated_at) {
            return Err(autoxide_kernel::version::changed());
        }
        if before.status == "active" {
            return Err(Error::conflict(
                "content.theme_variant_active",
                "The theme variant is on; pause it before deleting it.",
            ));
        }
        theme_variant::Entity::delete_many()
            .scoped(tenant)
            .filter(theme_variant::Column::Id.eq(id))
            .exec(&transaction)
            .await?;
        self.record_variant(&transaction, tenant, principal, Some(&before), None).await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn change_variant(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        expected: Option<Timestamp>,
        change: impl FnOnce(&mut theme_variant::ActiveModel),
    ) -> Result<Variant, Error> {
        principal.require(&CONTENT_THEMES)?;
        let transaction = tenant.database().begin_write().await?;
        let before = find(&transaction, tenant, id).await?;
        hidden(principal.require_at(&transaction, &CONTENT_THEMES, target(&before)).await, id)?;
        principal.require_write_at(&transaction, &CONTENT_THEMES, target(&before)).await?;
        if expected.is_some_and(|expected| expected != before.updated_at) {
            return Err(autoxide_kernel::version::changed());
        }
        let mut active: theme_variant::ActiveModel = before.clone().into();
        change(&mut active);
        active.updated_by = Set(Some(principal.user_id()));
        active.updated_at = Set(now());
        let after = active.update(&transaction).await?;
        self.record_variant(&transaction, tenant, principal, Some(&before), Some(&after)).await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Website `org` và quyền `content.themes.manage` ở đó.
    pub(crate) async fn themes_site(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Uuid,
    ) -> Result<Site, Error> {
        principal.require(&CONTENT_THEMES)?;
        let site = Site::of(tenant, OrgId::from_uuid(org)).await?;
        let target = OrgTarget::new(site.org(), None);
        principal.require_at(tenant.database().read(), &CONTENT_THEMES, target).await?;
        Ok(site)
    }

    /// Kiểm dữ liệu của bản chủ đề, mọi lỗi một lần (D-083). Ghi đè đi qua cùng bộ kiểm của theme
    /// gốc; ảnh của ghi đè phải là ảnh công khai.
    async fn check_variant(
        &self,
        tenant: &TenantContext,
        site: &Site,
        zone: &TimeZone,
        input: VariantInput,
        before: Option<&Variant>,
    ) -> Result<Fields, Error> {
        let mut problems: Vec<FieldError> = Vec::new();
        let preset = match input.preset_key.as_deref() {
            Some(key) => {
                let found = presets::find(key);
                if found.is_none() {
                    problems.push(
                        FieldError::new(
                            "preset_key",
                            "unknown",
                            "there is no theme preset `{key}`",
                        )
                        .with_param("key", key),
                    );
                }
                found
            }
            None => None,
        };
        let locale = self
            .site_languages(tenant, site)
            .await
            .map(|languages| languages.default)
            .unwrap_or_default();
        let name = input
            .name
            .or_else(|| {
                preset
                    .and_then(|preset| preset.name.get(&locale).or(preset.name.get("en")).cloned())
            })
            .or_else(|| before.map(|row| row.name.clone()))
            .unwrap_or_default()
            .trim()
            .to_owned();
        if name.is_empty() || name.chars().count() > NAME_MAX {
            problems.push(
                FieldError::new("name", "length", "must be 1 to {max} characters")
                    .with_param("max", NAME_MAX),
            );
        }
        let raw_overrides = input
            .overrides
            .or_else(|| preset.map(|preset| preset.overrides.clone()))
            .unwrap_or_default();
        let overrides = match tokens::check(&raw_overrides) {
            Ok(checked) => checked,
            Err(errors) => {
                problems.extend(errors.into_iter().map(|mut error| {
                    error.field = error.field.replacen("tokens", "overrides", 1);
                    error
                }));
                BTreeMap::new()
            }
        };
        let effect = input.effect.or_else(|| preset.and_then(|preset| preset.effect.clone()));
        if let Some(effect) = &effect
            && !EFFECTS.contains(&effect.as_str())
        {
            problems.push(
                FieldError::new(
                    "effect",
                    "not_allowed",
                    "must be one of the allowed effects: {effects}",
                )
                .with_param("effects", EFFECTS.join(", ")),
            );
        }
        let intensity = input
            .effect_intensity
            .or_else(|| preset.and_then(|preset| preset.effect_intensity.clone()))
            .or_else(|| effect.as_ref().map(|_| "low".to_owned()));
        if let Some(intensity) = &intensity
            && !INTENSITIES.contains(&intensity.as_str())
        {
            problems.push(
                FieldError::new("effect_intensity", "invalid", "must be one of {allowed}")
                    .with_param("allowed", INTENSITIES.join(", ")),
            );
        }
        let year = input.year.unwrap_or_else(|| now().date_in(zone).year());
        let raw_schedule =
            input.schedule.or_else(|| preset.and_then(|preset| preset.schedule_for(year, zone)));
        let schedule = match raw_schedule {
            Some(raw) => match schedule::check(&raw) {
                Ok(schedule) => Some(schedule),
                Err(errors) => {
                    problems.extend(errors);
                    None
                }
            },
            None => {
                problems.push(FieldError::new(
                    "schedule",
                    "required",
                    "give a schedule (or a preset with dates for `year`)",
                ));
                None
            }
        };
        let priority = input.priority.or(before.map(|row| row.priority)).unwrap_or(0);
        if !(0..=1000).contains(&priority) {
            problems.push(FieldError::new("priority", "range", "must be from 0 to 1000"));
        }
        let mut media = Map::new();
        for key in ["logo_media_id", "background_media_id"] {
            if let Some(value) = overrides.get(key) {
                media.insert(key.to_owned(), value.clone());
            }
        }
        if let Err(error) = self.check_theme_media(tenant, &media.into_iter().collect()).await {
            if error.fields().is_empty() {
                return Err(error);
            }
            problems.extend(error.fields().iter().cloned().map(|mut field| {
                field.field = field.field.replacen("tokens", "overrides", 1);
                field
            }));
        }
        let Some(schedule) = schedule.filter(|_| problems.is_empty()) else {
            return Err(Error::validation(problems));
        };
        let (starts_at, ends_at) = schedule.bounds();
        Ok(Fields {
            name,
            overrides: serde_json::to_string(&overrides).unwrap_or_else(|_| "{}".to_owned()),
            effect,
            effect_intensity: intensity,
            schedule: schedule.to_json().to_string(),
            starts_at,
            ends_at,
            priority,
            preset_key: input.preset_key.or_else(|| before.and_then(|row| row.preset_key.clone())),
        })
    }

    async fn record_variant<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        principal: &Principal,
        before: Option<&Variant>,
        after: Option<&Variant>,
    ) -> Result<(), Error> {
        let view = |row: &Variant| json!({"kind": "variant", "id": row.id, "org_id": row.org_id, "name": row.name, "status": row.status});
        // Khung website đã tính trên máy này hết hiệu lực ngay; máy khác thấy trong 30 giây.
        self.site_cache.invalidate();
        let id = after.or(before).map_or(Uuid::nil(), |row| row.id);
        let event = events::ThemeChanged(Change {
            before: before.map(view),
            after: after.map(view),
            published: None,
        });
        self.outbox
            .record(
                connection,
                tenant,
                Actor::from(principal),
                events::subject("theme_variant", id),
                &event,
            )
            .await?;
        Ok(())
    }
}

struct Fields {
    name: String,
    overrides: String,
    effect: Option<String>,
    effect_intensity: Option<String>,
    schedule: String,
    starts_at: Option<Timestamp>,
    ends_at: Option<Timestamp>,
    priority: i32,
    preset_key: Option<String>,
}

async fn find<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    id: Uuid,
) -> Result<Variant, Error> {
    theme_variant::Entity::find_by_id(id)
        .scoped(tenant)
        .one(connection)
        .await?
        .ok_or_else(|| variant_not_found(id))
}

fn hidden(result: Result<(), Error>, id: Uuid) -> Result<(), Error> {
    result.map_err(
        |error| if error.code() == "org_out_of_scope" { variant_not_found(id) } else { error },
    )
}
