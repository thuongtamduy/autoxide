//! Hình dạng dữ liệu của REST. Doc comment `///` của kiểu có `ToSchema` vào tài liệu OpenAPI:
//! viết tiếng Anh cho người gọi API.

use autoxide_kernel::money::Money;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::{code, flash_item, legal_limit, notice_rule, version};
use crate::model::{Kind, Status};
use crate::programs::{PromotionInput, PromotionView, TargetInput};

/// Title and terms of a promotion in one language.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = PromotionsText)]
pub(crate) struct TextDto {
    pub locale: String,
    pub title: String,
    pub terms: String,
}

/// The sale price of a variant in a flash sale, with the units sold.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = PromotionsFlashItem)]
pub(crate) struct FlashItemDto {
    pub variant_id: Uuid,
    pub sale_price_minor: Option<i64>,
    pub value_bp: Option<i32>,
    pub quantity_limit: Option<i64>,
    /// Units held or sold — a real number.
    pub quantity_used: i64,
    pub per_customer_quantity_limit: Option<i64>,
}

impl From<&flash_item::Model> for FlashItemDto {
    fn from(item: &flash_item::Model) -> Self {
        FlashItemDto {
            variant_id: item.variant_id,
            sale_price_minor: item.sale_price_minor,
            value_bp: item.value_bp,
            quantity_limit: item.quantity_limit,
            quantity_used: item.quantity_used,
            per_customer_quantity_limit: item.per_customer_quantity_limit,
        }
    }
}

/// A promotion with its rule, counters and texts.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = PromotionsPromotion)]
pub(crate) struct PromotionResponse {
    pub id: Uuid,
    pub status: Status,
    /// The rule as given when created (and changed since).
    pub rule: PromotionInput,
    /// Uses held or confirmed.
    pub usage_count: i64,
    /// Budget held or spent.
    pub budget_used: Option<Money>,
    /// Vouchers given out.
    pub issued_count: i64,
    /// Version of the audit trail (0 before it is turned on).
    pub version: i32,
    pub texts: Vec<TextDto>,
    pub targets: Vec<TargetInput>,
    pub flash_items: Vec<FlashItemDto>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub activated_at: Option<DateTime<Utc>>,
}

impl From<&PromotionView> for PromotionResponse {
    fn from(view: &PromotionView) -> Self {
        let row = &view.row;
        PromotionResponse {
            id: row.id,
            status: view.status(),
            rule: crate::programs::input_of(view),
            usage_count: row.usage_count,
            budget_used: row
                .budget_minor
                .and_then(|_| Money::from_columns(row.budget_used_minor, &row.currency).ok()),
            issued_count: row.issued_count,
            version: row.version,
            texts: view
                .texts
                .iter()
                .map(|text| TextDto {
                    locale: text.locale.clone(),
                    title: text.title.clone(),
                    terms: text.terms.clone(),
                })
                .collect(),
            targets: view
                .targets
                .iter()
                .filter_map(|found| {
                    Some(TargetInput {
                        target_type: crate::model::TargetType::parse(&found.target_type)?,
                        target_id: found.target_id,
                        role: crate::model::Role::parse(&found.role),
                    })
                })
                .collect(),
            flash_items: view.flash.iter().map(FlashItemDto::from).collect(),
            created_at: row.created_at,
            updated_at: row.updated_at,
            activated_at: row.activated_at,
        }
    }
}

/// A promotion in a list.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = PromotionsPromotionListItem)]
pub(crate) struct PromotionListItem {
    pub id: Uuid,
    pub org_id: Uuid,
    pub title: String,
    pub kind: Kind,
    pub status: Status,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub usage_count: i64,
}

/// A coupon or claim code.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = PromotionsCode)]
pub(crate) struct CodeDto {
    pub id: Uuid,
    pub code: String,
    pub purpose: crate::model::Purpose,
    pub usage_limit: Option<i64>,
    pub usage_count: i64,
    pub created_at: DateTime<Utc>,
}

impl From<&code::Model> for CodeDto {
    fn from(row: &code::Model) -> Self {
        CodeDto {
            id: row.id,
            code: row.code.clone(),
            purpose: crate::model::Purpose::parse(&row.purpose)
                .unwrap_or(crate::model::Purpose::Redeem),
            usage_limit: row.usage_limit,
            usage_count: row.usage_count,
            created_at: row.created_at,
        }
    }
}

/// Codes added: listed when created at once, counted when generated in the background.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = PromotionsCodesAdded)]
pub(crate) struct CodesAdded {
    pub codes: Vec<CodeDto>,
    /// Codes being generated in the background (`promotions.generate_codes`).
    pub queued: u32,
}

/// One entry of a promotion's audit trail.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = PromotionsVersion)]
pub(crate) struct VersionDto {
    pub version: i32,
    /// `activated`, `updated`, `translation`, `paused`, `ended`…
    pub action: String,
    /// The promotion as it was after the change.
    #[schema(value_type = Object)]
    pub snapshot: serde_json::Value,
    pub changed_by: Option<Uuid>,
    pub changed_at: DateTime<Utc>,
}

impl From<&version::Model> for VersionDto {
    fn from(row: &version::Model) -> Self {
        VersionDto {
            version: row.version,
            action: row.action.clone(),
            snapshot: serde_json::from_str(&row.snapshot).unwrap_or_default(),
            changed_by: row.changed_by,
            changed_at: row.changed_at,
        }
    }
}

/// A legal limit on promotions in force on a date (shared legal data).
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = PromotionsLegalLimit)]
pub(crate) struct LegalLimitDto {
    pub jurisdiction: String,
    /// `price_reduction`, `gift_value`, `free_shipping`.
    pub promotion_kind: String,
    /// `regular` or `national_focus`.
    pub program_kind: String,
    /// Most discount, in basis points of the price just before the promotion.
    pub max_bp: i32,
    pub max_days_per_program: Option<i32>,
    pub max_days_per_year: Option<i32>,
    pub effective_from: String,
    pub effective_to: Option<String>,
    pub legal_basis: String,
    pub source_url: String,
    /// Notes, including "to be checked against the original text".
    pub note: String,
}

impl From<&legal_limit::Model> for LegalLimitDto {
    fn from(row: &legal_limit::Model) -> Self {
        LegalLimitDto {
            jurisdiction: row.jurisdiction.clone(),
            promotion_kind: row.promotion_kind.clone(),
            program_kind: row.program_kind.clone(),
            max_bp: row.max_bp,
            max_days_per_program: row.max_days_per_program,
            max_days_per_year: row.max_days_per_year,
            effective_from: row.effective_from.clone(),
            effective_to: row.effective_to.clone(),
            legal_basis: row.legal_basis.clone(),
            source_url: row.source_url.clone(),
            note: row.note.clone(),
        }
    }
}

/// A promotion notice rule in force on a date (shared legal data).
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = PromotionsNoticeRule)]
pub(crate) struct NoticeRuleDto {
    pub jurisdiction: String,
    pub promotion_kind: String,
    pub notice_required: bool,
    pub days_before: i32,
    /// `province_of_store` (department of the province where the promotion runs) or an authority code.
    pub authority: String,
    pub effective_from: String,
    pub effective_to: Option<String>,
    pub legal_basis: String,
    pub note: String,
}

impl From<&notice_rule::Model> for NoticeRuleDto {
    fn from(row: &notice_rule::Model) -> Self {
        NoticeRuleDto {
            jurisdiction: row.jurisdiction.clone(),
            promotion_kind: row.promotion_kind.clone(),
            notice_required: row.notice_required,
            days_before: row.days_before,
            authority: row.authority.clone(),
            effective_from: row.effective_from.clone(),
            effective_to: row.effective_to.clone(),
            legal_basis: row.legal_basis.clone(),
            note: row.note.clone(),
        }
    }
}
