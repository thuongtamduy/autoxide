//! Xuất dữ liệu thông báo khuyến mại theo từng nơi nhận (D-225): mỗi cơ quan một mục — có cần
//! thông báo không theo quy tắc đang hiệu lực, hạn nộp, căn cứ pháp lý và đã ghi nhận nộp chưa.
//! Người nộp là con người (qua cổng dịch vụ công của Sở Công Thương); ở đây chỉ chuẩn bị dữ liệu
//! và ghi nhận. Bản CSV chống CSV injection bằng `autoxide_kernel::csv`.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use chrono::{DateTime, NaiveDate, Utc};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use autoxide_kernel::tenancy::ScopeExt;

use crate::compiled::program_of;
use crate::compliance::legal_day;
use crate::entity::notice;
use crate::legal::legal_kind;
use crate::service::PromotionsService;

/// The data of a promotion notice, one entry per authority.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = PromotionsNoticeExport)]
pub(crate) struct NoticeExport {
    pub promotion_id: Uuid,
    /// Unit that runs the promotion.
    pub org_id: Uuid,
    /// Kind of the promotion (`percent_off`, `flash_sale`…).
    pub kind: String,
    /// Kind of promotion under the law (`price_reduction`, `gift_value`, `free_shipping`…).
    pub promotion_kind: String,
    /// Locale of `title` and `terms`.
    pub locale: String,
    pub title: String,
    /// Conditions shown to buyers.
    pub terms: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    /// Discount in basis points (1000 = 10%), when the promotion has one.
    pub value_bp: Option<i32>,
    /// Discount amount in minor units of `currency`, when the promotion has one.
    pub value_minor: Option<i64>,
    pub currency: String,
    /// Number of products, variants and categories the promotion targets (0 = the whole store).
    pub target_count: usize,
    /// Number of flash-sale or add-on prices.
    pub flash_item_count: usize,
    /// Status of the promotion.
    pub status: String,
    /// Jurisdiction of the law data used.
    pub jurisdiction: String,
    /// `false` when no notice rule is in force for this kind of promotion on its start day
    /// (load the law data with `autoxide task promotions.load_legal_data`).
    pub rules_found: bool,
    pub authorities: Vec<NoticeEntry>,
}

/// The notice to one authority.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = PromotionsNoticeEntry)]
pub(crate) struct NoticeEntry {
    /// Authority, e.g. `province:79` (the provincial Department of Industry and Trade).
    pub authority_code: String,
    /// Whether the rule in force asks for a notice.
    pub required: bool,
    /// `pending`, `submitted` or `not_required`.
    pub status: String,
    /// Reference given by the authority, when recorded.
    pub reference: Option<String>,
    pub submitted_at: Option<DateTime<Utc>>,
    /// Days before the start the notice is due.
    pub days_before: Option<i32>,
    /// Last day to submit (Vietnam date), when a notice is required.
    pub due_on: Option<NaiveDate>,
    /// Legal basis of the rule, with its date in force.
    pub legal_basis: Option<String>,
}

impl PromotionsService {
    /// Dữ liệu thông báo của chương trình `id`, một mục mỗi nơi nhận (theo quy tắc đang hiệu lực
    /// ngày bắt đầu, cộng các nơi đã ghi nhận tay).
    pub(crate) async fn notice_export(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<NoticeExport, Error> {
        let view = self.promotion(tenant, principal, id).await?;
        let program = program_of(&view);
        let kind = legal_kind(&program);
        let start_day = legal_day(program.starts_at);
        let legal = self.legal().await?;
        let rule = legal.rule(&self.config.jurisdiction, kind, start_day).cloned();
        let recorded = notice::Entity::find()
            .filter(notice::Column::PromotionId.eq(id))
            .scoped(tenant)
            .order_by_asc(notice::Column::AuthorityCode)
            .all(tenant.database().read())
            .await?;
        let mut authorities = Vec::new();
        if let Some(rule) = &rule {
            for code in self.authorities(tenant, &program, &rule.authority).await? {
                let found = recorded.iter().find(|row| row.authority_code == code);
                let status = found.map_or_else(
                    || if rule.notice_required { "pending" } else { "not_required" }.to_owned(),
                    |row| row.status.clone(),
                );
                authorities.push(NoticeEntry {
                    authority_code: code,
                    required: rule.notice_required,
                    status,
                    reference: found.and_then(|row| row.reference.clone()),
                    submitted_at: found.and_then(|row| row.submitted_at),
                    days_before: Some(rule.days_before),
                    due_on: rule
                        .notice_required
                        .then(|| start_day - chrono::Duration::days(i64::from(rule.days_before))),
                    legal_basis: Some(rule.legal_basis.clone()),
                });
            }
        }
        // Nơi đã ghi nhận tay mà quy tắc hiện không nêu (vd đổi tỉnh sau khi nộp): vẫn xuất.
        for row in &recorded {
            if authorities.iter().any(|entry| entry.authority_code == row.authority_code) {
                continue;
            }
            authorities.push(NoticeEntry {
                authority_code: row.authority_code.clone(),
                required: false,
                status: row.status.clone(),
                reference: row.reference.clone(),
                submitted_at: row.submitted_at,
                days_before: None,
                due_on: None,
                legal_basis: None,
            });
        }
        let text =
            view.texts.iter().find(|text| text.locale == "vi").or_else(|| view.texts.first());
        let row = &view.row;
        Ok(NoticeExport {
            promotion_id: row.id,
            org_id: row.org_id,
            kind: row.kind.clone(),
            promotion_kind: kind.to_owned(),
            locale: text.map(|text| text.locale.clone()).unwrap_or_default(),
            title: text.map(|text| text.title.clone()).unwrap_or_default(),
            terms: text.map(|text| text.terms.clone()).unwrap_or_default(),
            starts_at: row.starts_at,
            ends_at: row.ends_at,
            value_bp: row.value_bp,
            value_minor: row.value_minor,
            currency: row.currency.clone(),
            target_count: view.targets.len(),
            flash_item_count: view.flash.len(),
            status: row.status.clone(),
            jurisdiction: self.config.jurisdiction.clone(),
            rules_found: rule.is_some(),
            authorities,
        })
    }
}

impl NoticeExport {
    /// Bản CSV: một dòng mỗi nơi nhận, thông tin chương trình lặp lại ở mỗi dòng (mở được bằng
    /// bảng tính, lọc theo nơi nhận).
    pub(crate) fn to_csv(&self) -> String {
        let header: Vec<String> = [
            "authority_code",
            "required",
            "status",
            "due_on",
            "days_before",
            "reference",
            "submitted_at",
            "legal_basis",
            "promotion_id",
            "org_id",
            "kind",
            "promotion_kind",
            "title",
            "terms",
            "starts_at",
            "ends_at",
            "value_bp",
            "value_minor",
            "currency",
            "target_count",
            "flash_item_count",
        ]
        .map(str::to_owned)
        .to_vec();
        let text = |value: Option<String>| value.unwrap_or_default();
        let lines: Vec<Vec<String>> = self
            .authorities
            .iter()
            .map(|entry| {
                vec![
                    entry.authority_code.clone(),
                    entry.required.to_string(),
                    entry.status.clone(),
                    text(entry.due_on.map(|day| day.to_string())),
                    text(entry.days_before.map(|days| days.to_string())),
                    text(entry.reference.clone()),
                    text(entry.submitted_at.map(|at| at.to_rfc3339())),
                    text(entry.legal_basis.clone()),
                    self.promotion_id.to_string(),
                    self.org_id.to_string(),
                    self.kind.clone(),
                    self.promotion_kind.clone(),
                    self.title.clone(),
                    self.terms.clone(),
                    self.starts_at.to_rfc3339(),
                    self.ends_at.to_rfc3339(),
                    text(self.value_bp.map(|bp| bp.to_string())),
                    text(self.value_minor.map(|minor| minor.to_string())),
                    self.currency.clone(),
                    self.target_count.to_string(),
                    self.flash_item_count.to_string(),
                ]
            })
            .collect();
        autoxide_kernel::csv::write(&header, &lines)
    }
}
