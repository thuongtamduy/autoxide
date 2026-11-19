//! Kiểm số tồn khớp sổ: `on_hand` = tổng sổ biến động của dòng, `available` = `on_hand` − chỗ đang
//! giữ. Chỉ báo, không sửa: lệch là lỗi của code, người vận hành xem rồi điều chỉnh có lý do.
//!
//! - Lệnh `autoxide task inventory.verify_levels [tenant=] [org_id=]`.
//! - Bước kiểm của `autoxide doctor` (`Module::checks`): mẫu tối đa 1.000 dòng mỗi khách, chỉ đọc,
//!   trong một transaction snapshot.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::doctor::{Check, Finding, Probe};
use autoxide_kernel::error::Error;
use autoxide_kernel::task::{Task, TaskArgs, TaskArgument, TaskRun};
use autoxide_kernel::tenancy::{DatabaseRegistry, ScopeExt, TenantContext};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use uuid::Uuid;

use crate::entity::{level, movement, reservation, reservation_line};
use crate::service::InventoryService;

/// Số dòng tối đa mỗi khách của bước kiểm mẫu.
const SAMPLE: u64 = 1000;

/// Một dòng tồn lệch sổ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Mismatch {
    pub level_id: Uuid,
    pub org_id: Uuid,
    pub variant_id: Uuid,
    pub on_hand: i64,
    /// Tổng sổ biến động của dòng.
    pub ledger: i64,
    pub available: i64,
    /// `on_hand` − chỗ đang giữ.
    pub expected_available: i64,
}

impl InventoryService {
    /// Dòng tồn lệch sổ của khách (chỉ kho `org`, khi có), tối đa `limit` dòng xét.
    pub(crate) async fn verify_levels(
        &self,
        tenant: &TenantContext,
        org: Option<Uuid>,
        limit: Option<u64>,
    ) -> Result<Vec<Mismatch>, Error> {
        let snapshot = tenant.database().begin_snapshot().await?;
        let mut select = level::Entity::find().scoped(tenant).order_by_asc(level::Column::Id);
        if let Some(org) = org {
            select = select.filter(level::Column::OrgId.eq(org));
        }
        if let Some(limit) = limit {
            select = select.limit(limit);
        }
        let levels = select.all(&snapshot).await?;
        let mut mismatches = Vec::new();
        for chunk in levels.chunks(500) {
            let ids: Vec<Uuid> = chunk.iter().map(|row| row.id).collect();
            let ledger = ledger_sums(&snapshot, tenant, &ids).await?;
            let held = crate::verify::held_sums(&snapshot, tenant, &ids).await?;
            for row in chunk {
                let sum = ledger.get(&row.id).copied().unwrap_or(0);
                let expected = row.on_hand - held.get(&row.id).copied().unwrap_or(0);
                if sum != row.on_hand || expected != row.available {
                    mismatches.push(Mismatch {
                        level_id: row.id,
                        org_id: row.org_id,
                        variant_id: row.variant_id,
                        on_hand: row.on_hand,
                        ledger: sum,
                        available: row.available,
                        expected_available: expected,
                    });
                }
            }
        }
        Ok(mismatches)
    }
}

/// Tổng `quantity` của sổ biến động theo dòng tồn. `CAST(… AS BIGINT)`: Postgres trả `numeric`
/// cho `SUM(bigint)`.
async fn ledger_sums<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, i64>, Error> {
    let rows: Vec<(Uuid, i64)> = movement::Entity::find()
        .select_only()
        .column(movement::Column::LevelId)
        .column_as(Expr::cust("CAST(COALESCE(SUM(quantity), 0) AS BIGINT)"), "total")
        .filter(movement::Column::LevelId.is_in(ids.iter().copied()))
        .scoped(tenant)
        .group_by(movement::Column::LevelId)
        .into_tuple()
        .all(connection)
        .await?;
    Ok(rows.into_iter().collect())
}

/// Số đang giữ (chưa xuất, chưa trả) theo dòng tồn: dòng của chỗ giữ `held` / `confirmed`.
pub(crate) async fn held_sums<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, i64>, Error> {
    let open: Vec<Uuid> = reservation::Entity::find()
        .select_only()
        .column(reservation::Column::Id)
        .filter(reservation::Column::Status.is_in(["held", "confirmed"]))
        .scoped(tenant)
        .into_tuple()
        .all(connection)
        .await?;
    let mut held = HashMap::new();
    for chunk in open.chunks(500) {
        let rows: Vec<(Uuid, i64)> = reservation_line::Entity::find()
            .select_only()
            .column(reservation_line::Column::LevelId)
            .column_as(
                Expr::cust(
                    "CAST(COALESCE(SUM(quantity - fulfilled_quantity - released_quantity), 0) AS BIGINT)",
                ),
                "total",
            )
            .filter(reservation_line::Column::ReservationId.is_in(chunk.iter().copied()))
            .filter(reservation_line::Column::LevelId.is_in(ids.iter().copied()))
            .scoped(tenant)
            .group_by(reservation_line::Column::LevelId)
            .into_tuple()
            .all(connection)
            .await?;
        for (level, total) in rows {
            *held.entry(level).or_insert(0) += total;
        }
    }
    Ok(held)
}

fn describe(tenant: &TenantContext, mismatches: &[Mismatch]) -> Vec<String> {
    mismatches
        .iter()
        .map(|m| {
            format!(
                "Tenant `{}`, level {} (warehouse {}, variant {}): on hand {} vs ledger {}, \
                 available {} vs expected {}.",
                tenant.slug(),
                m.level_id,
                m.org_id,
                m.variant_id,
                m.on_hand,
                m.ledger,
                m.available,
                m.expected_available
            )
        })
        .collect()
}

/// Lệnh `inventory.verify_levels`.
pub(crate) fn task(service: Arc<InventoryService>, databases: DatabaseRegistry) -> Task {
    Task::new(
        "verify_levels",
        "Check that every stock level matches its movement ledger (on hand) and its reservations \
         (available). Only reports; nothing is changed.",
        Arc::new(Verify { service, databases }),
    )
    .argument(TaskArgument::optional(
        "tenant",
        "Short name (slug) of the tenant to check; omitted: the default tenant.",
    ))
    .argument(TaskArgument::optional("org_id", "Only this warehouse or store (its unit id)."))
}

struct Verify {
    service: Arc<InventoryService>,
    databases: DatabaseRegistry,
}

#[async_trait::async_trait]
impl TaskRun for Verify {
    async fn run(&self, args: &TaskArgs) -> Result<String, Error> {
        let tenant = match args.get("tenant") {
            Some(slug) => self.databases.tenant_by_slug(slug.trim()).await?,
            None => self.databases.default_tenant(),
        };
        let org = match args.get("org_id") {
            Some(raw) => Some(raw.trim().parse::<Uuid>().map_err(|_| {
                Error::bad_request(
                    "inventory.invalid_org_id",
                    "`{value}` is not a unit id (a UUID).",
                )
                .with_param("value", raw.to_owned())
            })?),
            None => None,
        };
        let mismatches = self.service.verify_levels(&tenant, org, None).await?;
        if mismatches.is_empty() {
            return Ok(format!("Tenant `{}`: every stock level matches.", tenant.slug()));
        }
        let mut lines = vec![format!(
            "Tenant `{}`: {} stock level(s) do not match.",
            tenant.slug(),
            mismatches.len()
        )];
        lines.extend(describe(&tenant, &mismatches));
        Ok(lines.join("\n"))
    }
}

/// Bước kiểm `inventory.levels` của `autoxide doctor`.
pub(crate) fn check(service: Arc<InventoryService>, databases: DatabaseRegistry) -> Check {
    Check::new(
        "inventory.levels",
        "A sample of stock levels matches the movement ledger and the reservations.",
        Arc::new(Sample { service, databases }),
    )
    .with_timeout(Duration::from_secs(30))
}

struct Sample {
    service: Arc<InventoryService>,
    databases: DatabaseRegistry,
}

#[async_trait::async_trait]
impl Probe for Sample {
    async fn probe(&self) -> Finding {
        let tenants = match self.databases.all_tenants().await {
            Ok(tenants) => tenants,
            Err(error) => return Finding::failed(format!("cannot list tenants: {error}")),
        };
        let mut problems = Vec::new();
        for tenant in &tenants {
            match self.service.verify_levels(tenant, None, Some(SAMPLE)).await {
                Ok(found) => problems.extend(describe(tenant, &found)),
                Err(error) => problems.push(format!("Tenant `{}`: {error}", tenant.slug())),
            }
        }
        if problems.is_empty() {
            Finding::ok("stock levels match the movement ledger")
        } else {
            Finding::failed(problems.join(" "))
                .with_hint("run `autoxide task inventory.verify_levels` and adjust with a reason")
        }
    }
}
