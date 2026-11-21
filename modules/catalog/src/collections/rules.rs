//! Quy tắc của bộ sưu tập `rule`: JSON có schema cố định, kiểm ở service. Mọi điều kiện cùng phải
//! đúng (`match: all`); tối đa [`MAX_CONDITIONS`] điều kiện để câu lọc luôn đi theo index sẵn có.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::collections::BTreeMap;

use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entity::category;
use crate::public::PublicFilter;
use crate::service::CatalogService;

/// Số điều kiện tối đa của một quy tắc.
pub(crate) const MAX_CONDITIONS: usize = 10;

/// Quy tắc của bộ sưu tập.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rules {
    /// Thuộc một trong các danh mục này (gồm danh mục con).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub category_ids: Vec<Uuid>,
    /// Mã thuộc tính → giá trị (khớp một trong số đó).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, Vec<String>>,
    /// Giá thấp nhất, đơn vị nhỏ nhất của tiền tệ bảng giá mặc định.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_min: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_max: Option<i64>,
    /// Lên kệ trong số ngày gần đây.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_within_days: Option<i64>,
    /// Đang có giá khuyến mại.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub on_promotion: bool,
    /// Chỉ `all`.
    #[serde(default = "all", rename = "match")]
    pub matching: String,
}

fn all() -> String {
    "all".to_owned()
}

impl Rules {
    fn conditions(&self) -> usize {
        usize::from(!self.category_ids.is_empty())
            + self.attributes.len()
            + usize::from(self.price_min.is_some())
            + usize::from(self.price_max.is_some())
            + usize::from(self.new_within_days.is_some())
            + usize::from(self.on_promotion)
    }

    /// Bộ lọc công khai tương ứng lúc `now`.
    pub(crate) fn filter(&self, locale: &str, now: Timestamp) -> PublicFilter {
        PublicFilter {
            locale: locale.to_owned(),
            attributes: self.attributes.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            price_min: self.price_min,
            price_max: self.price_max,
            on_sale: self.on_promotion,
            category_ids: self.category_ids.clone(),
            published_after: self
                .new_within_days
                .map(|days| now - SignedDuration::from_hours(24 * days)),
            ..Default::default()
        }
    }
}

fn rule_invalid(problems: Vec<FieldError>) -> Error {
    problems.into_iter().fold(
        Error::new(
            ErrorKind::Validation,
            "catalog.rule_invalid",
            "The rule of the collection is not valid; see `errors`.",
        ),
        Error::with_field,
    )
}

impl CatalogService {
    /// Kiểm JSON quy tắc `raw` và trả bản đã chuẩn hoá. Gom mọi lỗi (D-083).
    pub(crate) async fn check_rules(
        &self,
        tenant: &TenantContext,
        raw: &serde_json::Value,
    ) -> Result<Rules, Error> {
        let rules: Rules = serde_json::from_value(raw.clone()).map_err(|error| {
            rule_invalid(vec![
                FieldError::new("rules", "invalid", "is not a valid rule: {reason}")
                    .with_param("reason", error),
            ])
        })?;
        let mut problems = Vec::new();
        if rules.matching != "all" {
            problems.push(FieldError::new("rules.match", "invalid", "must be `all`"));
        }
        let count = rules.conditions();
        if count == 0 {
            problems.push(FieldError::new("rules", "required", "needs at least one condition"));
        }
        if count > MAX_CONDITIONS {
            problems.push(
                FieldError::new("rules", "count", "has more than {max} conditions")
                    .with_param("max", MAX_CONDITIONS),
            );
        }
        for (field, value) in
            [("rules.price_min", rules.price_min), ("rules.price_max", rules.price_max)]
        {
            if value.is_some_and(|value| value < 0) {
                problems.push(FieldError::new(field, "range", "must not be negative"));
            }
        }
        if let (Some(min), Some(max)) = (rules.price_min, rules.price_max)
            && max < min
        {
            problems.push(FieldError::new(
                "rules.price_max",
                "range",
                "must not be below `price_min`",
            ));
        }
        if rules.new_within_days.is_some_and(|days| !(1..=365).contains(&days)) {
            problems.push(FieldError::new("rules.new_within_days", "range", "must be 1 to 365"));
        }
        if !rules.category_ids.is_empty() {
            let found: Vec<Uuid> = category::Entity::find()
                .select_only()
                .column(category::Column::Id)
                .filter(category::Column::Id.is_in(rules.category_ids.clone()))
                .scoped(tenant)
                .into_tuple()
                .all(tenant.database().read())
                .await?;
            for (index, id) in rules.category_ids.iter().enumerate() {
                if !found.contains(id) {
                    problems.push(FieldError::new(
                        format!("rules.category_ids[{index}]"),
                        "not_found",
                        "is not a category",
                    ));
                }
            }
        }
        let attributes: Vec<(String, Vec<String>)> =
            rules.attributes.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        self.attribute_conditions(tenant, &attributes, &mut problems).await?;
        if !problems.is_empty() {
            return Err(rule_invalid(problems));
        }
        // Lọc "đang khuyến mại" cần giá khuyến mại do `promotions` tính.
        if rules.on_promotion && self.sale_prices.get().is_none() {
            return Err(Error::new(
                ErrorKind::Validation,
                "catalog.rule_needs_module",
                "The condition `{condition}` needs module `{module}`, which is not installed.",
            )
            .with_param("condition", "on_promotion")
            .with_param("module", "promotions"));
        }
        Ok(rules)
    }
}
