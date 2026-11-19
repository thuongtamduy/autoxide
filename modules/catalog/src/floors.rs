//! Sàn giá (D-222): quy tắc giá tối thiểu và biên lợi nhuận tối thiểu trên giá vốn, theo nút và
//! danh mục; giá vốn dẫn xuất từ event `inventory.cost_changed`. `Catalog::price_floors` cho
//! `promotions`; thêm giá dưới sàn `block` bị chặn, `warn` ghi kèm cảnh báo.

use std::collections::HashMap;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, Set};
use uuid::Uuid;

use crate::api::{FloorAction, MAX_IDS, PriceFloor};
use crate::entity::{price_guard, product_category, variant, variant_cost};
use crate::errors;
use crate::permissions::{PRICES_READ, PRICES_WRITE};
use crate::pricing::ancestors;
use crate::service::{CatalogService, default_org, fail_if_any, now, writable};

impl CatalogService {
    /// Sàn giá đang áp cho các biến thể ở nút `org` (D-222): quy tắc của nút gần nhất thắng;
    /// trong cùng nút, quy tắc của danh mục gần nhất (danh mục của sản phẩm hay tổ tiên gần
    /// nhất) thắng quy tắc không danh mục.
    pub(crate) async fn floors(
        &self,
        tenant: &TenantContext,
        org: OrgId,
        currency: Currency,
        variant_ids: &[Uuid],
    ) -> Result<Vec<PriceFloor>, Error> {
        if variant_ids.len() > MAX_IDS {
            return Err(errors::too_many_ids(MAX_IDS));
        }
        if variant_ids.is_empty() {
            return Ok(Vec::new());
        }
        let read = tenant.database().read();
        let chain = ancestors(read, tenant, org).await?;
        let guards = price_guard::Entity::find()
            .filter(price_guard::Column::OrgId.is_in(chain.iter().map(OrgId::as_uuid)))
            .filter(price_guard::Column::Currency.eq(currency.code()))
            .scoped(tenant)
            .all(read)
            .await?;
        if guards.is_empty() {
            return Ok(Vec::new());
        }
        let variants = variant::Entity::find()
            .filter(variant::Column::Id.is_in(variant_ids.iter().copied()))
            .scoped(tenant)
            .all(read)
            .await?;
        let product_ids: Vec<Uuid> = variants.iter().map(|row| row.product_id).collect();
        let links = product_category::Entity::find()
            .filter(product_category::Column::ProductId.is_in(product_ids))
            .scoped(tenant)
            .all(read)
            .await?;
        let category_ids: Vec<Uuid> = links.iter().map(|link| link.category_id).collect();
        // (danh mục của sản phẩm, tổ tiên, khoảng cách)
        let paths = self.with_ancestors(read, tenant, &category_ids).await?;
        let costs = variant_cost::Entity::find()
            .filter(variant_cost::Column::VariantId.is_in(variant_ids.iter().copied()))
            .filter(variant_cost::Column::Currency.eq(currency.code()))
            .filter(variant_cost::Column::OrgId.is_in(chain.iter().map(OrgId::as_uuid)))
            .scoped(tenant)
            .all(read)
            .await?;
        let mut out = Vec::new();
        for variant_id in variant_ids {
            let Some(row) = variants.iter().find(|row| row.id == *variant_id) else { continue };
            let own: Vec<Uuid> = links
                .iter()
                .filter(|link| link.product_id == row.product_id)
                .map(|link| link.category_id)
                .collect();
            // Khoảng cách từ danh mục của sản phẩm tới mỗi tổ tiên (nhỏ nhất).
            let mut distance: HashMap<Uuid, i32> = HashMap::new();
            for (descendant, ancestor, depth) in &paths {
                if own.contains(descendant) {
                    let entry = distance.entry(*ancestor).or_insert(*depth);
                    *entry = (*entry).min(*depth);
                }
            }
            let best = guards
                .iter()
                .filter_map(|guard| {
                    let rank = chain.iter().position(|org| org.as_uuid() == guard.org_id)?;
                    let category_rank = match guard.category_id {
                        None => i32::MAX,
                        Some(category) => *distance.get(&category)?,
                    };
                    Some(((rank, category_rank, guard.id), guard))
                })
                .min_by(|a, b| a.0.cmp(&b.0))
                .map(|(_, guard)| guard);
            let Some(guard) = best else { continue };
            // Giá vốn của nút gần nhất có giá vốn.
            let unit_cost = chain
                .iter()
                .find_map(|org| {
                    costs
                        .iter()
                        .find(|cost| cost.variant_id == *variant_id && cost.org_id == org.as_uuid())
                })
                .map(|cost| Money::new(cost.unit_cost_minor, currency));
            let from_margin = match (guard.min_margin_bp, unit_cost) {
                (Some(bp), Some(cost)) => Some(margin_floor(cost.minor(), bp)),
                _ => None,
            };
            let floor = [guard.min_price_minor, from_margin].into_iter().flatten().max();
            out.push(PriceFloor {
                variant_id: *variant_id,
                guard_id: guard.id,
                min_price: guard.min_price_minor.map(|minor| Money::new(minor, currency)),
                min_margin_bp: guard.min_margin_bp,
                unit_cost,
                floor: floor.map(|minor| Money::new(minor, currency)),
                action: if guard.action == "block" {
                    FloorAction::Block
                } else {
                    FloorAction::Warn
                },
            });
        }
        Ok(out)
    }

    /// Đặt quy tắc sàn giá ở nút `org_id` (theo danh mục nếu nêu).
    pub(crate) async fn create_guard(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewGuard,
    ) -> Result<price_guard::Model, Error> {
        let mut problems = Vec::new();
        if input.min_price.is_none() && input.min_margin_bp.is_none() {
            problems.push(FieldError::new(
                "min_price",
                "required",
                "give `min_price`, `min_margin_bp` or both",
            ));
        }
        if input.min_price.is_some_and(|money| money.is_negative()) {
            problems.push(FieldError::new("min_price", "range", "must not be negative"));
        }
        if input.min_margin_bp.is_some_and(|bp| !(0..=100_000).contains(&bp)) {
            problems.push(FieldError::new("min_margin_bp", "range", "must be 0 to 100000"));
        }
        if input.min_price.is_some_and(|money| money.currency() != input.currency) {
            problems.push(FieldError::new("min_price", "currency", "must be in `currency`"));
        }
        fail_if_any(problems)?;
        let org = match input.org_id {
            Some(org) => OrgId::from_uuid(org),
            None => default_org(tenant, principal, &PRICES_WRITE)?,
        };
        let transaction = tenant.database().begin_write().await?;
        principal
            .require_write_at(&transaction, &PRICES_WRITE, crate::service::at(org.as_uuid()))
            .await?;
        if let Some(category) = input.category_id {
            self.existing_categories(&transaction, tenant, &[category]).await?;
        }
        let now = now();
        let row = price_guard::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(org.as_uuid()),
            category_id: Set(input.category_id),
            currency: Set(input.currency.code().to_owned()),
            min_price_minor: Set(input.min_price.map(Money::minor)),
            min_margin_bp: Set(input.min_margin_bp),
            action: Set(match input.action {
                FloorAction::Block => "block".to_owned(),
                FloorAction::Warn => "warn".to_owned(),
            }),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&transaction)
        .await?;
        transaction.commit().await?;
        Ok(row)
    }

    /// Quy tắc sàn giá trong phạm vi đọc giá.
    pub(crate) async fn list_guards(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<price_guard::Model>, Error> {
        let scope = principal.scope_condition(&PRICES_READ, price_guard::Column::OrgId, None)?;
        let select = price_guard::Entity::find().filter(scope).scoped(tenant);
        fetch_page(select, price_guard::Column::Id, |row| row.id, page, tenant.database().read())
            .await
    }

    /// Bỏ quy tắc sàn giá `id`.
    pub(crate) async fn delete_guard(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        let not_found = || {
            Error::not_found("catalog.price_guard_not_found", "There is no price floor `{id}`.")
                .with_param("id", id)
        };
        let row = price_guard::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(not_found)?;
        writable(principal, &transaction, &PRICES_READ, &PRICES_WRITE, row.org_id, not_found)
            .await?;
        price_guard::Entity::delete_many()
            .filter(price_guard::Column::Id.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Ghi giá vốn đơn vị (từ event `inventory.cost_changed`). Nhận lại cùng event chỉ ghi đè
    /// cùng giá trị.
    pub(crate) async fn record_cost(
        &self,
        tenant: &TenantContext,
        org: Uuid,
        variant_id: Uuid,
        cost: Money,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        let existing = variant_cost::Entity::find()
            .filter(variant_cost::Column::VariantId.eq(variant_id))
            .filter(variant_cost::Column::OrgId.eq(org))
            .filter(variant_cost::Column::Currency.eq(cost.currency().code()))
            .scoped(tenant)
            .one(&transaction)
            .await?;
        match existing {
            Some(row) => {
                let mut row = row.into_active_model();
                row.unit_cost_minor = Set(cost.minor());
                row.updated_at = Set(now());
                row.update(&transaction).await?;
            }
            None => {
                variant_cost::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    org_id: Set(org),
                    variant_id: Set(variant_id),
                    unit_cost_minor: Set(cost.minor()),
                    currency: Set(cost.currency().code().to_owned()),
                    updated_at: Set(now()),
                }
                .insert(&transaction)
                .await?;
            }
        }
        transaction.commit().await?;
        Ok(())
    }
}

/// Quy tắc sàn giá mới.
#[derive(Debug, Clone)]
pub(crate) struct NewGuard {
    pub org_id: Option<Uuid>,
    pub category_id: Option<Uuid>,
    pub currency: Currency,
    pub min_price: Option<Money>,
    pub min_margin_bp: Option<i64>,
    pub action: FloorAction,
}

/// Giá tối thiểu để đạt biên `bp` phần vạn trên giá vốn `cost`: `cost × (10000 + bp) / 10000`,
/// làm tròn lên tới đơn vị nhỏ nhất (số nguyên, không số thực).
pub(crate) fn margin_floor(cost: i64, bp: i64) -> i64 {
    let product = i128::from(cost) * i128::from(10_000 + bp);
    let floor = (product + 9_999).div_euclid(10_000);
    i64::try_from(floor).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::margin_floor;

    #[test]
    fn san_theo_bien_loi_nhuan_lam_tron_len() {
        assert_eq!(margin_floor(100_000, 1_500), 115_000);
        assert_eq!(margin_floor(99_999, 1_500), 114_999);
        assert_eq!(margin_floor(1, 1), 2);
    }
}
