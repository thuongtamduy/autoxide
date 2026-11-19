//! Người bán tham gia chương trình của sàn (flash sale, ngày hội — D-222): người bán đăng ký sản
//! phẩm của mình kèm giá / mức giảm đề nghị và số suất (`promotions.participate`), sàn duyệt
//! (`promotions.publish`); duyệt xong mới thành suất flash sale. Phần giảm của sản phẩm người bán
//! góp do chính người bán trả (`funding`).

use std::collections::{BTreeMap, BTreeSet};

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::tenancy::{OrgId, OrgTree, ScopeExt, TenantContext};
use chrono::{DateTime, Utc};
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::{flash_item, participation, promotion};
use crate::events::{self, Change};
use crate::model::{Kind, Status};
use crate::permissions::{PARTICIPATE, PUBLISH, READ};
use crate::programs::{not_found, record_version};
use crate::service::{PromotionsService, bump_revision, now, readable, writable};

/// Số sản phẩm tối đa một lần đăng ký.
const MAX_ITEMS: usize = 500;

/// One product a seller puts into a platform promotion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = PromotionsParticipationItem)]
pub(crate) struct ParticipationItem {
    pub variant_id: Uuid,
    /// Sale price of one unit, in minor units (or `value_bp`).
    pub sale_price_minor: Option<i64>,
    /// Discount in basis points, 1 to 10000 (or `sale_price_minor`).
    pub value_bp: Option<i32>,
    /// Units offered at that price; omitted: not limited.
    pub quantity_limit: Option<i64>,
    /// Most units one buyer can get at that price.
    pub per_customer_quantity_limit: Option<i64>,
}

/// A seller's application to join a platform promotion.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = PromotionsParticipationRequest)]
pub(crate) struct ParticipationRequest {
    /// Your unit (the seller), below the unit that runs the promotion.
    pub org_id: Uuid,
    /// 1 to 500 of your products.
    pub items: Vec<ParticipationItem>,
}

/// A seller's application, as it stands.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = PromotionsParticipation)]
pub(crate) struct ParticipationView {
    pub id: Uuid,
    pub promotion_id: Uuid,
    pub seller_org_id: Uuid,
    /// `pending`, `approved` or `rejected`.
    pub status: String,
    pub items: Vec<ParticipationItem>,
    pub reviewed_by: Option<Uuid>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn items_of(row: &participation::Model) -> Vec<ParticipationItem> {
    serde_json::from_str(&row.items).unwrap_or_default()
}

fn present(row: &participation::Model) -> ParticipationView {
    ParticipationView {
        id: row.id,
        promotion_id: row.promotion_id,
        seller_org_id: row.seller_org_id,
        status: row.status.clone(),
        items: items_of(row),
        reviewed_by: row.reviewed_by,
        reviewed_at: row.reviewed_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

/// Biến thể → nút người bán, từ các lần tham gia đã duyệt.
pub(crate) fn sponsors(rows: &[participation::Model]) -> BTreeMap<Uuid, Uuid> {
    rows.iter()
        .filter(|row| row.status == "approved")
        .flat_map(|row| items_of(row).into_iter().map(|item| (item.variant_id, row.seller_org_id)))
        .collect()
}

fn closed() -> Error {
    Error::conflict(
        "promotions.participation_closed",
        "Sellers can join only a platform flash sale that has not started yet.",
    )
}

fn already_reviewed() -> Error {
    Error::conflict(
        "promotions.participation_already_reviewed",
        "This application was already approved; its products are in the promotion.",
    )
}

/// Chương trình nhận người bán tham gia: flash sale chưa bắt đầu (`draft` hay `scheduled`).
fn open_for_sellers(row: &promotion::Model) -> bool {
    row.kind == Kind::FlashSale.as_str()
        && matches!(row.status.as_str(), "draft" | "scheduled")
        && row.starts_at > now()
}

impl PromotionsService {
    /// Người bán đăng ký (hay sửa lần đăng ký chưa duyệt) sản phẩm vào chương trình `id` của sàn.
    pub(crate) async fn participate(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        request: &ParticipationRequest,
    ) -> Result<Submitted<ParticipationView>, Error> {
        let mut problems = Vec::new();
        if request.items.is_empty() || request.items.len() > MAX_ITEMS {
            problems.push(
                FieldError::new("items", "length", "send 1 to {max} products")
                    .with_param("max", MAX_ITEMS),
            );
        }
        let mut seen = BTreeSet::new();
        for (index, item) in request.items.iter().enumerate() {
            let one = item.sale_price_minor.is_some() != item.value_bp.is_some();
            if !one
                || item.sale_price_minor.is_some_and(|price| price < 0)
                || item.value_bp.is_some_and(|bp| !(1..=10_000).contains(&bp))
                || item.quantity_limit.is_some_and(|limit| limit < 1)
                || item.per_customer_quantity_limit.is_some_and(|limit| limit < 1)
                || !seen.insert(item.variant_id)
            {
                problems.push(FieldError::new(
                    format!("items[{index}]"),
                    "invalid",
                    "exactly one of `sale_price_minor` / `value_bp`; limits at least 1; each product once",
                ));
            }
        }
        // Sản phẩm phải của chính người bán (catalog, ngoài transaction).
        let ids: Vec<Uuid> = request.items.iter().map(|item| item.variant_id).collect();
        let owned: BTreeSet<Uuid> = self
            .catalog
            .variants(tenant, &ids, None)
            .await?
            .into_iter()
            .filter(|variant| variant.org_id.as_uuid() == request.org_id)
            .map(|variant| variant.variant_id)
            .collect();
        for (index, item) in request.items.iter().enumerate() {
            if !owned.contains(&item.variant_id) {
                problems.push(FieldError::new(
                    format!("items[{index}].variant_id"),
                    "not_yours",
                    "not a product of your unit",
                ));
            }
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let seller = request.org_id;
        let transaction = tenant.database().begin_write().await?;
        let program = promotion::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| not_found(id))?;
        // Người bán thấy chương trình của sàn ở nút cấp trên của mình; nút khác thì như không có.
        let lineage =
            OrgTree::ancestors(&transaction, tenant.id(), OrgId::from_uuid(seller)).await?;
        if seller == program.org_id || !lineage.iter().any(|org| org.as_uuid() == program.org_id) {
            return Err(not_found(id));
        }
        writable(principal, &transaction, &PARTICIPATE, seller, || not_found(id)).await?;
        if !open_for_sellers(&program) {
            return Err(closed());
        }
        let existing = participation::Entity::find()
            .filter(participation::Column::PromotionId.eq(id))
            .filter(participation::Column::SellerOrgId.eq(seller))
            .scoped(tenant)
            .one(&transaction)
            .await?;
        if existing.as_ref().is_some_and(|found| found.status == "approved") {
            return Err(already_reviewed());
        }
        let taken: BTreeSet<Uuid> = flash_item::Entity::find()
            .filter(flash_item::Column::PromotionId.eq(id))
            .scoped(tenant)
            .all(&transaction)
            .await?
            .into_iter()
            .map(|row| row.variant_id)
            .collect();
        let clashes: Vec<FieldError> = request
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| taken.contains(&item.variant_id))
            .map(|(index, _)| {
                FieldError::new(
                    format!("items[{index}].variant_id"),
                    "taken",
                    "this product is already in the promotion",
                )
            })
            .collect();
        if !clashes.is_empty() {
            return Err(Error::validation(clashes));
        }
        let items = serde_json::to_string(&request.items).map_err(Error::internal)?;
        let at = now();
        let result = match existing {
            Some(found) => {
                let mut row: participation::ActiveModel = found.into();
                row.items = Set(items);
                row.status = Set("pending".to_owned());
                row.reviewed_by = Set(None);
                row.reviewed_at = Set(None);
                row.updated_at = Set(at);
                Submitted::Replayed(row.update(&transaction).await?)
            }
            None => {
                // Hai lần đăng ký cùng lúc: khoá duy nhất (khách, chương trình, người bán) giữ một dòng.
                participation::Entity::insert(participation::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    promotion_id: Set(id),
                    seller_org_id: Set(seller),
                    status: Set("pending".to_owned()),
                    items: Set(items),
                    reviewed_by: Set(None),
                    reviewed_at: Set(None),
                    created_at: Set(at),
                    updated_at: Set(at),
                })
                .on_conflict(
                    OnConflict::columns([
                        participation::Column::TenantId,
                        participation::Column::PromotionId,
                        participation::Column::SellerOrgId,
                    ])
                    .do_nothing()
                    .to_owned(),
                )
                .exec_without_returning(&transaction)
                .await?;
                let row = participation::Entity::find()
                    .filter(participation::Column::PromotionId.eq(id))
                    .filter(participation::Column::SellerOrgId.eq(seller))
                    .scoped(tenant)
                    .one(&transaction)
                    .await?
                    .ok_or_else(|| {
                        Error::internal("the application just written cannot be read")
                    })?;
                Submitted::Created(row)
            }
        };
        transaction.commit().await?;
        Ok(match result {
            Submitted::Created(row) => Submitted::Created(present(&row)),
            Submitted::Replayed(row) => Submitted::Replayed(present(&row)),
        })
    }

    /// Lần đăng ký của chương trình `id`: sàn (đọc được chương trình) thấy mọi lần; người bán thấy
    /// lần của nút mình.
    pub(crate) async fn participations(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Vec<ParticipationView>, Error> {
        let read = tenant.database().read();
        let program = promotion::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| not_found(id))?;
        let mut select = participation::Entity::find()
            .filter(participation::Column::PromotionId.eq(id))
            .scoped(tenant);
        if readable(principal, read, &READ, program.org_id, || not_found(id)).await.is_err() {
            select = select.filter(principal.scope_condition(
                &PARTICIPATE,
                participation::Column::SellerOrgId,
                None,
            )?);
        }
        let rows = select.order_by_asc(participation::Column::Id).all(read).await?;
        Ok(rows.iter().map(present).collect())
    }

    /// Sàn duyệt (`approve`) hay từ chối lần đăng ký; duyệt thì sản phẩm thành suất flash sale.
    pub(crate) async fn review_participation(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        participation_id: Uuid,
        approve: bool,
    ) -> Result<ParticipationView, Error> {
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        let program = promotion::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| not_found(id))?;
        writable(principal, &transaction, &PUBLISH, program.org_id, || not_found(id)).await?;
        let found = participation::Entity::find_by_id(participation_id)
            .filter(participation::Column::PromotionId.eq(id))
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| not_found(id))?;
        if found.status == "approved" {
            return Err(already_reviewed());
        }
        if !open_for_sellers(&program) {
            return Err(closed());
        }
        let at = now();
        if approve {
            for item in items_of(&found) {
                flash_item::Entity::insert(flash_item::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    promotion_id: Set(id),
                    variant_id: Set(item.variant_id),
                    sale_price_minor: Set(item.sale_price_minor),
                    value_bp: Set(item.value_bp),
                    quantity_limit: Set(item.quantity_limit),
                    quantity_used: Set(0),
                    per_customer_quantity_limit: Set(item.per_customer_quantity_limit),
                })
                .on_conflict(
                    OnConflict::columns([
                        flash_item::Column::TenantId,
                        flash_item::Column::PromotionId,
                        flash_item::Column::VariantId,
                    ])
                    .do_nothing()
                    .to_owned(),
                )
                .exec_without_returning(&transaction)
                .await?;
            }
        }
        let status = if approve { "approved" } else { "rejected" };
        let mut row: participation::ActiveModel = found.into();
        row.status = Set(status.to_owned());
        row.reviewed_by = Set(Some(principal.user_id()));
        row.reviewed_at = Set(Some(at));
        row.updated_at = Set(at);
        let row = row.update(&transaction).await?;
        if program.status != Status::Draft.as_str() {
            let view = crate::programs::store_load(&transaction, tenant, id)
                .await?
                .ok_or_else(|| not_found(id))?;
            record_version(&transaction, tenant, &view, "participation", &actor).await?;
        }
        bump_revision(&transaction, tenant).await?;
        self.outbox
            .record(
                &transaction,
                tenant,
                actor,
                events::promotion_subject(id),
                &events::ParticipationReviewed(Change::after(json!({
                    "promotion_id": id,
                    "participation_id": row.id,
                    "seller_org_id": row.seller_org_id,
                    "status": status,
                }))),
            )
            .await?;
        transaction.commit().await?;
        self.compiled.invalidate(tenant.id());
        Ok(present(&row))
    }
}
