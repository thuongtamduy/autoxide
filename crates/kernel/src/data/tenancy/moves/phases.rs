//! Ba pha dữ liệu: chép (copy), chuyển (switch), dọn nguồn (clean).

use super::files::{copy_files, delete_files, tenant_prefix, verify};
use super::*;

impl TenantMoves {
    /// `copying` → `copied`.
    pub(super) async fn copy(&self, moving: &Model, runner: &str) -> Result<Model, Error> {
        let tenant = TenantId::from_uuid(moving.tenant_id);
        let row = self.tenant_row(&moving.tenant_id.to_string()).await?;
        let (from, to) = (
            self.placement(moving.from_placement_id).await?,
            self.placement(moving.to_placement_id).await?,
        );
        // Máy chủ khác thấy `moving` sau tối đa TTL cache; request đã bắt đầu trước đó chạy xong
        // trong thời gian tối đa của một request.
        let ready_at = moving.moving_since
            + chrono::Duration::from_std(self.settle).map_err(Error::internal)?;
        if let Ok(wait) = (ready_at - now()).to_std() {
            tracing::info!(move_id = %moving.id, seconds = wait.as_secs(), "waiting for other servers to see the move");
            tokio::time::sleep(wait).await;
        }
        let source = self.database(&from).await?;
        let target = self.database(&to).await?;
        if let Some(problem) =
            self.schema_problems(&from, &source, &to, &target).await?.into_iter().next()
        {
            return Err(problem);
        }
        let tables = self.tables(&source, &target).await?;
        wait_claims(&source, &tables, tenant).await?;
        // File trước dòng: chép file hỏng thì đích chưa có gì đổi; chạy lại chép lại từ đầu.
        let files = match self.file_stores(&from, &to).await? {
            Some((from_store, to_store)) => {
                copy_files(&*from_store, &*to_store, &tenant_prefix(moving.tenant_id))
                    .await
                    .map_err(|error| {
                        errors::tenant_move_storage_unavailable(&to.name, &error.to_string())
                    })?
            }
            None => 0,
        };
        let context = TenantContext::new(
            tenant,
            row.slug.clone(),
            row.name.clone(),
            TenantStatus::Moving,
            PlacementId::from_uuid(to.id),
            target.clone(),
        );
        let actor = moving.requested_by.map_or_else(Actor::system, Actor::user);
        let transaction = target.begin_write().await?;
        let copied = tables.copy(&source, &transaction, tenant).await.map_err(transfer)?;
        let event = TenantMoved {
            tenant_id: moving.tenant_id,
            from: from.name.clone(),
            to: to.name.clone(),
            rows: copied.iter().map(|table| table.rows).sum(),
        };
        let subject = format!("kernel.tenant:{}", moving.tenant_id);
        self.outbox.record(&transaction, &context, actor, Some(subject), &event).await?;
        transaction.commit().await?;
        verify(&tables, &source, &target, tenant).await?;
        self.set_state(self.control().write(), moving, runner, &to.name, COPIED, Some(&copied))
            .await?;
        tracing::info!(move_id = %moving.id, tenant_id = %tenant, rows = event.rows, files, "tenant rows and files copied");
        self.find(moving.id).await
    }

    /// `copied` → `switched`: khách dùng placement mới, `active`.
    pub(super) async fn switch(&self, moving: &Model, runner: &str) -> Result<Model, Error> {
        let tenant = TenantId::from_uuid(moving.tenant_id);
        let (from, to) = (
            self.placement(moving.from_placement_id).await?,
            self.placement(moving.to_placement_id).await?,
        );
        let source = self.database(&from).await?;
        let target = self.database(&to).await?;
        let tables = self.tables(&source, &target).await?;
        if let Err(error) = verify(&tables, &source, &target, tenant).await {
            // Nguồn đổi sau khi chép (không được xảy ra khi khách `moving`): lần chạy sau chép lại.
            let _ = self
                .set_state(self.control().write(), moving, runner, &to.name, COPYING, None)
                .await;
            return Err(error);
        }
        let at = now();
        let transaction = self.control().begin_write().await?;
        let switched = tenant::Entity::update_many()
            .col_expr(tenant::Column::PlacementId, Expr::value(to.id))
            .col_expr(tenant::Column::Status, Expr::value(TenantStatus::Active.as_str()))
            .col_expr(tenant::Column::UpdatedAt, Expr::value(at))
            .filter(tenant::Column::Id.eq(moving.tenant_id))
            .filter(tenant::Column::PlacementId.eq(from.id))
            .filter(tenant::Column::Status.eq(TenantStatus::Moving.as_str()))
            .exec(&transaction)
            .await?;
        if switched.rows_affected != 1 {
            return Err(Error::internal(format!(
                "tenant {} is no longer `moving` on `{}`; the move cannot switch it",
                moving.tenant_id, from.name
            )));
        }
        self.set_state(&transaction, moving, runner, &to.name, SWITCHED, None).await?;
        transaction.commit().await?;
        self.registry.invalidate(tenant);
        tracing::info!(move_id = %moving.id, tenant_id = %tenant, placement = %to.name, "tenant switched to its new placement");
        self.find(moving.id).await
    }

    /// `switched` → `done`: xoá dòng của khách ở nguồn (`keep_source`: chỉ hàng đợi job và outbox).
    pub(super) async fn clean(&self, moving: &Model, runner: &str) -> Result<Model, Error> {
        let tenant = TenantId::from_uuid(moving.tenant_id);
        let (from, to) = (
            self.placement(moving.from_placement_id).await?,
            self.placement(moving.to_placement_id).await?,
        );
        let source = self.database(&from).await?;
        let target = self.database(&to).await?;
        // Chặn lần nữa ngay trước khi xoá: xoá "nguồn" là cùng database với đích thì mất dữ liệu.
        if same_database(&source, &target).await.map_err(transfer)? {
            return Err(errors::tenant_move_same_database(&to.name));
        }
        let tables = self.tables(&source, &target).await?;
        let keep = moving.keep_source;
        let transaction = source.begin_write().await?;
        let deleted = tables
            .delete(&transaction, tenant, |table| !keep || tables.kind(table) != TableKind::Data)
            .await
            .map_err(transfer)?;
        transaction.commit().await?;
        let mut files = 0;
        if !keep && let Some((from_store, to_store)) = self.file_stores(&from, &to).await? {
            files = delete_files(&*from_store, &*to_store, &tenant_prefix(moving.tenant_id))
                .await
                .map_err(|error| {
                    errors::tenant_move_storage_unavailable(&from.name, &error.to_string())
                })?;
        }
        self.set_state(self.control().write(), moving, runner, &to.name, DONE, None).await?;
        tracing::info!(move_id = %moving.id, tenant_id = %tenant, deleted, files, keep_source = keep, "tenant move done");
        self.find(moving.id).await
    }
}
