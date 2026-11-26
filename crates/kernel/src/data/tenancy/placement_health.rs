//! Bước kiểm `kernel.placements` của `autoxide doctor`: placement (database riêng của khách) nào chưa sẵn sàng —
//! khách trên đó nhận 503 `tenant_maintenance`. Chỉ đọc trạng thái đã ghi ở database điều khiển (không mở kết nối
//! tới từng placement), nên nhanh cả khi có nhiều khách.
//!
//! Placement `migrating` mà lấy được khoá migration ngay thì không ai đang migrate nó: tiến trình đã dừng giữa
//! chừng (D-20261126-placement-migration-exclusive). Lần migrate sau tự dọn; doctor nói rõ để người vận hành chạy.

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use super::admin::placement;
use super::{DatabaseRegistry, MAIN_PLACEMENT, SchemaState};
use crate::doctor::{Finding, Probe};
use crate::migration::MigrationLock;

pub(crate) struct PlacementHealth {
    pub registry: DatabaseRegistry,
}

const FIX: &str = "Run `autoxide migrate up --all` (or `POST /api/v1/kernel/placements/migrations`); it \
                   names the placement that fails and why.";

#[async_trait::async_trait]
impl Probe for PlacementHealth {
    async fn probe(&self) -> Finding {
        let rows = match placement::Entity::find()
            .filter(placement::Column::Id.ne(MAIN_PLACEMENT.as_uuid()))
            .order_by_asc(placement::Column::Name)
            .all(self.registry.control().read())
            .await
        {
            Ok(rows) => rows,
            Err(error) => {
                return Finding::failed(format!("cannot read kernel_placements: {error}"));
            }
        };
        let named = |state: SchemaState| -> Vec<String> {
            rows.iter()
                .filter(|row| row.schema_state == state.as_str())
                .map(|row| row.name.clone())
                .collect()
        };
        let behind = named(SchemaState::Behind);
        let migrating = named(SchemaState::Migrating);
        if behind.is_empty() && migrating.is_empty() {
            return Finding::ok(format!("{} placement(s), all ready.", rows.len()));
        }
        let mut problems = Vec::new();
        if !behind.is_empty() {
            problems.push(format!(
                "{} placement(s) behind, their tenants get 503: {}",
                behind.len(),
                behind.join(", ")
            ));
        }
        if !migrating.is_empty() {
            if self.nobody_migrating().await {
                problems.push(format!(
                    "{} placement(s) left `migrating` by a process that stopped midway: {}",
                    migrating.len(),
                    migrating.join(", ")
                ));
            } else if behind.is_empty() {
                return Finding::warning(format!(
                    "Migrating now: {}. Their tenants get 503 until it finishes.",
                    migrating.join(", ")
                ));
            }
        }
        Finding::failed(problems.join("; ")).with_hint(FIX)
    }
}

impl PlacementHealth {
    /// Lấy được khoá migration ngay (rồi nhả): không tiến trình nào đang migrate.
    async fn nobody_migrating(&self) -> bool {
        let Some(config) = self.registry.control_config() else {
            return true;
        };
        match MigrationLock::acquire(config, std::time::Duration::ZERO).await {
            Ok(lock) => {
                super::placements::release(lock).await;
                true
            }
            Err(_) => false,
        }
    }
}
