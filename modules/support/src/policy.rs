//! Chính sách hỗ trợ theo nút (bước 53 mục 3), kế thừa theo cây: nút không có chính sách riêng
//! dùng của tổ tiên gần nhất, không có thì mặc định. Chính sách là dữ liệu: loại phiếu, hạn trả
//! lời theo ưu tiên (phút làm việc), giờ làm, ngày nghỉ, múi giờ, số ngày tự đóng, cho khách vãng
//! lai mở phiếu, cài đặt trả lời tự động.

use autoxide_kernel::time::Date;
use std::collections::BTreeMap;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::tenancy::{OrgId, OrgTree, ScopeExt, TenantContext, root_of};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::policy;
use crate::hours::{Calendar, WorkingHours, default_hours, parse_time};
use crate::permissions::SETTINGS_MANAGE;
use crate::service::{SupportService, finish, now, org_exists};
use crate::status::Priority;
use crate::tickets::DEFAULT_CATEGORIES;

/// Support policy of an organization unit. Units without their own policy use the nearest unit
/// above that has one, else the defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = SupportPolicy)]
pub(crate) struct Policy {
    /// Ticket categories customers choose from (codes; labels are translated by the website).
    pub categories: Vec<String>,
    /// Business minutes to the first answer, per priority (`low`, `normal`, `high`, `urgent`).
    #[schema(value_type = Object, example = json!({"low": 960, "normal": 480, "high": 240, "urgent": 60}))]
    pub first_response_minutes: BTreeMap<String, i64>,
    /// Business minutes to solve the ticket, per priority; time waiting for the customer does not
    /// count.
    #[schema(value_type = Object, example = json!({"low": 2700, "normal": 1620, "high": 960, "urgent": 480}))]
    pub resolve_minutes: BTreeMap<String, i64>,
    /// When staff work.
    pub business_hours: Vec<WorkingHours>,
    /// Days off (`YYYY-MM-DD`, local date).
    #[schema(value_type = Vec<String>, example = json!(["2026-09-02"]))]
    pub holidays: Vec<Date>,
    /// Offset of the unit's local time from UTC, in minutes (Vietnam: 420).
    pub utc_offset_minutes: i32,
    /// A solved ticket closes after this many days without a new message (1 to 90).
    pub auto_close_days: i32,
    /// Guests (no account) may open tickets.
    pub guest_tickets: bool,
    /// Email replies to ticket notifications go back into the ticket (needs an inbound mail module).
    pub inbound_email: bool,
    /// Automatic answers to simple questions (off by default).
    pub auto_reply: bool,
    /// Lowest confidence (basis points, 0 to 10000) of the classification to answer automatically.
    pub auto_reply_min_confidence_bp: i32,
    /// At most this many automatic answers in a row on one ticket, then a person takes over.
    pub auto_reply_max_per_thread: i32,
    /// Words that always stop an automatic answer.
    pub auto_reply_blocked_terms: Vec<String>,
}

impl Default for Policy {
    fn default() -> Self {
        let minutes = |values: [i64; 4]| {
            Priority::ORDER
                .iter()
                .zip(values)
                .map(|(priority, value)| (priority.as_str().to_owned(), value))
                .collect()
        };
        Policy {
            categories: DEFAULT_CATEGORIES.iter().map(|code| (*code).to_owned()).collect(),
            // Khuyến nghị của đặc tả: phản hồi đầu 8 giờ làm việc, giải quyết 3 ngày làm việc.
            first_response_minutes: minutes([960, 480, 240, 60]),
            resolve_minutes: minutes([2700, 1620, 960, 480]),
            business_hours: default_hours(),
            holidays: Vec::new(),
            utc_offset_minutes: 420,
            auto_close_days: 7,
            guest_tickets: true,
            inbound_email: false,
            auto_reply: false,
            auto_reply_min_confidence_bp: 9000,
            auto_reply_max_per_thread: 2,
            auto_reply_blocked_terms: Vec::new(),
        }
    }
}

impl Policy {
    pub(crate) fn calendar(&self) -> Option<Calendar> {
        Calendar::new(&self.business_hours, &self.holidays, self.utc_offset_minutes)
    }

    pub(crate) fn first_response(&self, priority: Priority) -> Option<i64> {
        self.first_response_minutes.get(priority.as_str()).copied()
    }

    pub(crate) fn resolve(&self, priority: Priority) -> Option<i64> {
        self.resolve_minutes.get(priority.as_str()).copied()
    }

    /// Mọi lỗi của chính sách, báo một lần.
    fn problems(&self) -> Vec<FieldError> {
        let mut problems = Vec::new();
        let code_ok = |code: &str| {
            !code.is_empty()
                && code.len() <= 64
                && code.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        };
        if self.categories.is_empty()
            || self.categories.len() > 50
            || !self.categories.iter().all(|code| code_ok(code))
        {
            problems.push(FieldError::new(
                "categories",
                "invalid",
                "list 1 to 50 codes of lower-case letters, digits and `_`",
            ));
        }
        for (field, map) in [
            ("first_response_minutes", &self.first_response_minutes),
            ("resolve_minutes", &self.resolve_minutes),
        ] {
            let complete = Priority::ORDER.iter().all(|priority| {
                map.get(priority.as_str()).is_some_and(|minutes| (1..=525_600).contains(minutes))
            }) && map.len() == Priority::ORDER.len();
            if !complete {
                problems.push(FieldError::new(
                    field,
                    "invalid",
                    "give minutes (1 to 525600) for each of low, normal, high, urgent",
                ));
            }
        }
        let hours_ok = self.business_hours.len() <= 50
            && self.business_hours.iter().all(|range| {
                (1..=7).contains(&range.day)
                    && matches!((parse_time(&range.opens), parse_time(&range.closes)), (Some(o), Some(c)) if c > o)
            });
        if !hours_ok {
            problems.push(FieldError::new(
                "business_hours",
                "invalid",
                "list at most 50 ranges: day 1 (Monday) to 7, times HH:MM, closing after opening",
            ));
        }
        if self.holidays.len() > 400 {
            problems.push(FieldError::new("holidays", "invalid", "list at most 400 days"));
        }
        if !(-720..=840).contains(&self.utc_offset_minutes) {
            problems.push(FieldError::new(
                "utc_offset_minutes",
                "invalid",
                "must be between -720 and 840",
            ));
        }
        if !(1..=90).contains(&self.auto_close_days) {
            problems.push(FieldError::new(
                "auto_close_days",
                "invalid",
                "must be between 1 and 90",
            ));
        }
        if !(0..=10_000).contains(&self.auto_reply_min_confidence_bp) {
            problems.push(FieldError::new(
                "auto_reply_min_confidence_bp",
                "invalid",
                "must be between 0 and 10000",
            ));
        }
        if !(1..=10).contains(&self.auto_reply_max_per_thread) {
            problems.push(FieldError::new(
                "auto_reply_max_per_thread",
                "invalid",
                "must be between 1 and 10",
            ));
        }
        if self.auto_reply_blocked_terms.len() > 200
            || self
                .auto_reply_blocked_terms
                .iter()
                .any(|term| term.trim().is_empty() || term.chars().count() > 100)
        {
            problems.push(FieldError::new(
                "auto_reply_blocked_terms",
                "invalid",
                "list at most 200 words of 1 to 100 characters",
            ));
        }
        problems
    }
}

/// Chính sách đang hiệu lực của một nút và nơi nó được khai.
#[derive(Debug, Clone)]
pub(crate) struct Effective {
    pub policy: Policy,
    /// Nút khai chính sách; `None`: mặc định.
    pub from: Option<Uuid>,
}

impl SupportService {
    /// Chính sách đang hiệu lực ở nút `org` (của nút, của tổ tiên gần nhất, hay mặc định).
    pub(crate) async fn policy(
        &self,
        tenant: &TenantContext,
        org: Uuid,
    ) -> Result<Effective, Error> {
        let read = tenant.database().read();
        let node = OrgId::from_uuid(org);
        let mut lineage = OrgTree::ancestors(read, tenant.id(), node).await?;
        if lineage.is_empty() {
            lineage.push(node);
        }
        let root = root_of(tenant.id());
        if !lineage.contains(&root) {
            lineage.push(root);
        }
        let rows = policy::Entity::find()
            .filter(policy::Column::OrgId.is_in(lineage.iter().map(OrgId::as_uuid)))
            .scoped(tenant)
            .all(read)
            .await?;
        for node in &lineage {
            if let Some(row) = rows.iter().find(|row| row.org_id == node.as_uuid()) {
                let policy = serde_json::from_str::<Policy>(&row.body).map_err(|error| {
                    Error::internal(format!("support policy of {}: {error}", row.org_id))
                })?;
                return Ok(Effective { policy, from: Some(row.org_id) });
            }
        }
        Ok(Effective { policy: Policy::default(), from: None })
    }

    /// Ghi chính sách riêng của nút `org` (`support.settings.manage` ở nút đó).
    pub(crate) async fn set_policy(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Uuid,
        policy: Policy,
    ) -> Result<Effective, Error> {
        let mut policy = policy;
        policy.categories = policy.categories.iter().map(|code| code.trim().to_owned()).collect();
        policy.categories.dedup();
        policy.holidays.sort();
        policy.holidays.dedup();
        let mut problems = policy.problems();
        if !org_exists(tenant, OrgId::from_uuid(org)).await? {
            problems.push(FieldError::new("org_id", "unknown", "is not a unit of this shop"));
        }
        finish(problems)?;
        let body = serde_json::to_string(&policy).map_err(Error::internal)?;
        let transaction = tenant.database().begin_write().await?;
        let target = OrgTarget::new(OrgId::from_uuid(org), None);
        principal.require_write_at(&transaction, &SETTINGS_MANAGE, target).await?;
        let existing = policy::Entity::find()
            .filter(policy::Column::OrgId.eq(org))
            .scoped(tenant)
            .one(&transaction)
            .await?;
        match existing {
            Some(row) => {
                let mut row = row.into_active_model();
                row.body = Set(body);
                row.updated_at = Set(now());
                policy::Entity::update(row).validate()?.scoped(tenant).exec(&transaction).await?;
            }
            None => {
                policy::ActiveModel {
                    tenant_id: Set(tenant.id().as_uuid()),
                    org_id: Set(org),
                    body: Set(body),
                    updated_at: Set(now()),
                }
                .insert(&transaction)
                .await?;
            }
        }
        self.settings_event(&transaction, tenant, principal, json!({"policy": org})).await?;
        transaction.commit().await?;
        Ok(Effective { policy, from: Some(org) })
    }

    /// Bỏ chính sách riêng của nút `org`: nút về dùng chính sách kế thừa.
    pub(crate) async fn reset_policy(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Uuid,
    ) -> Result<Effective, Error> {
        let transaction = tenant.database().begin_write().await?;
        let target = OrgTarget::new(OrgId::from_uuid(org), None);
        principal.require_write_at(&transaction, &SETTINGS_MANAGE, target).await?;
        policy::Entity::delete_many()
            .filter(policy::Column::OrgId.eq(org))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        self.settings_event(&transaction, tenant, principal, json!({"policy": org, "reset": true}))
            .await?;
        transaction.commit().await?;
        self.policy(tenant, org).await
    }
}
