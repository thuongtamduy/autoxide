//! Huỷ, xoá bản sao dự phòng, đọc và mô tả một lượt dời.

use super::files::{delete_files, tenant_prefix};
use super::*;

impl TenantMoves {
    /// Huỷ lượt dời chưa đổi placement của khách `tenant`: xoá dòng đã chép ở đích, khách trở lại
    /// `active` ở placement cũ. Đã đổi placement: `tenant_move_not_abortable` (chạy tiếp thay vì huỷ).
    pub async fn abort(&self, tenant: &str) -> Result<TenantMove, Error> {
        let row = self.tenant_row(tenant).await?;
        let moving = self
            .unfinished(row.id)
            .await?
            .ok_or_else(|| errors::tenant_move_not_found("unfinished"))?;
        if moving.state == SWITCHED {
            return Err(errors::tenant_move_not_abortable());
        }
        let runner = format!("abort-{}", Uuid::now_v7());
        let Some(moving) = self.acquire(moving.id, &runner).await? else {
            return self.describe(self.find(moving.id).await?).await;
        };
        let tenant_id = TenantId::from_uuid(moving.tenant_id);
        let (from, to) = (
            self.placement(moving.from_placement_id).await?,
            self.placement(moving.to_placement_id).await?,
        );
        let source = self.database(&from).await?;
        // Đích hỏng hẳn (lý do hay gặp để huỷ): vẫn huỷ được; dòng đã chép còn ở đó thì xoá sau bằng
        // `purge`.
        let cleaned = async {
            let target = self.database(&to).await?;
            let tables = self.tables(&source, &target).await?;
            let transaction = target.begin_write().await?;
            tables.delete(&transaction, tenant_id, |_| true).await.map_err(transfer)?;
            transaction.commit().await?;
            // File đã chép sang nơi lưu của đích (nếu khác nơi lưu của nguồn).
            if let Some((from_store, to_store)) = self.file_stores(&from, &to).await? {
                delete_files(&*to_store, &*from_store, &tenant_prefix(moving.tenant_id)).await?;
            }
            Ok::<_, Error>(())
        }
        .await;
        if let Err(error) = &cleaned {
            tracing::warn!(move_id = %moving.id, error = %error, "cannot delete the copied rows at the target; purge them later");
        }
        let at = now();
        let transaction = self.control().begin_write().await?;
        tenant::Entity::update_many()
            .col_expr(tenant::Column::Status, Expr::value(TenantStatus::Active.as_str()))
            .col_expr(tenant::Column::UpdatedAt, Expr::value(at))
            .filter(tenant::Column::Id.eq(moving.tenant_id))
            .filter(tenant::Column::PlacementId.eq(from.id))
            .filter(tenant::Column::Status.eq(TenantStatus::Moving.as_str()))
            .exec(&transaction)
            .await?;
        self.set_state(&transaction, &moving, &runner, &to.name, ABORTED, None).await?;
        if let Err(error) = cleaned {
            Entity::update_many()
                .col_expr(Column::Error, Expr::value(describe(&error)))
                .filter(Column::Id.eq(moving.id))
                .exec(&transaction)
                .await?;
        }
        transaction.commit().await?;
        self.registry.invalidate(tenant_id);
        tracing::info!(move_id = %moving.id, tenant_id = %tenant_id, "tenant move aborted");
        self.describe(self.find(moving.id).await?).await
    }

    /// Xoá bản sao dữ liệu của khách `tenant` còn ở placement `placement` (để lại bởi `keep_source`
    /// hay lượt dời huỷ mà không xoá được đích). Không bao giờ xoá placement khách đang dùng. Trả
    /// số dòng đã xoá mỗi bảng.
    pub async fn purge(
        &self,
        tenant: &str,
        placement: &str,
    ) -> Result<Vec<TenantTableRows>, Error> {
        let row = self.tenant_row(tenant).await?;
        let placements = self.placements().await?;
        let copy = placements
            .iter()
            .find(|found| found.name == placement)
            .ok_or_else(|| errors::tenant_placement_not_found(placement))?;
        if copy.id == row.placement_id {
            return Err(errors::tenant_purge_current(placement));
        }
        if self.unfinished(row.id).await?.is_some() {
            return Err(errors::tenant_moving());
        }
        let current = placements
            .iter()
            .find(|found| found.id == row.placement_id)
            .ok_or_else(|| errors::tenant_placement_not_found(&row.placement_id.to_string()))?;
        let live = self.database(current).await?;
        let database = self.database(copy).await?;
        if same_database(&live, &database).await.map_err(transfer)? {
            return Err(errors::tenant_move_same_database(placement));
        }
        let tables = self.tables(&live, &database).await?;
        let tenant_id = TenantId::from_uuid(row.id);
        let counts = tables.count(&database, tenant_id).await.map_err(transfer)?;
        let transaction = database.begin_write().await?;
        tables.delete(&transaction, tenant_id, |_| true).await.map_err(transfer)?;
        transaction.commit().await?;
        let mut files = 0;
        if let Some((copy_store, live_store)) = self.file_stores(copy, current).await? {
            files =
                delete_files(&*copy_store, &*live_store, &tenant_prefix(row.id)).await.map_err(
                    |error| errors::tenant_move_storage_unavailable(placement, &error.to_string()),
                )?;
        }
        tracing::info!(tenant_id = %row.id, placement, files, "copy of the tenant's data deleted");
        Ok(counts.into_iter().map(TenantTableRows::from).filter(|table| table.rows > 0).collect())
    }

    /// Lượt dời `id` của khách `tenant` (ID hay slug).
    pub async fn get(&self, tenant: &str, id: Uuid) -> Result<TenantMove, Error> {
        let row = self.tenant_row(tenant).await?;
        let moving = Entity::find_by_id(id)
            .filter(Column::TenantId.eq(row.id))
            .one(self.control().write())
            .await?
            .ok_or_else(|| errors::tenant_move_not_found(&id.to_string()))?;
        self.describe(moving).await
    }

    pub(super) async fn describe(&self, moving: Model) -> Result<TenantMove, Error> {
        let placements = self.placements().await?;
        let name_of = |id: Uuid| {
            placements
                .iter()
                .find(|placement| placement.id == id)
                .map_or_else(|| id.to_string(), |placement| placement.name.clone())
        };
        let tables: Vec<TenantTableRows> =
            serde_json::from_str(&moving.tables).map_err(Error::internal)?;
        Ok(TenantMove {
            id: moving.id,
            tenant_id: moving.tenant_id,
            from: name_of(moving.from_placement_id),
            to: name_of(moving.to_placement_id),
            keep_source: moving.keep_source,
            state: moving.state,
            rows: tables.iter().map(|table| table.rows).sum(),
            tables,
            error: moving.error,
            created_at: moving.created_at,
            updated_at: moving.updated_at,
            finished_at: moving.finished_at,
        })
    }
}
