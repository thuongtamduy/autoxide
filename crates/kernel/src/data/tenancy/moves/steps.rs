//! Bắt đầu, chạy từng bước, lease và ghi trạng thái của một lượt dời.

use super::*;

impl TenantMoves {
    /// Bắt đầu dời khách `tenant` sang `to` (khách thành `moving`), hay trả lượt dời sang `to` đã
    /// dừng giữa chừng để chạy tiếp. Lý do chặn đầu tiên thành lỗi. Chưa chép gì: gọi
    /// [`run`](Self::run) (CLI) — `enqueue` thì một job nền chạy (REST / MCP).
    pub async fn start(
        &self,
        actor: Actor,
        tenant: &str,
        to: &str,
        keep_source: bool,
        enqueue: bool,
    ) -> Result<TenantMove, Error> {
        let checked = self.check(tenant, to).await?;
        if let Some(blocker) = checked.blockers.into_iter().next() {
            return Err(blocker);
        }
        if let Some(moving) = checked.resume {
            tracing::info!(move_id = %moving.id, tenant_id = %moving.tenant_id, state = %moving.state, "resuming a tenant move");
            if enqueue {
                self.jobs
                    .enqueue_database(
                        self.control().write(),
                        &MoveTenant { move_id: moving.id },
                        EnqueueOptions::default(),
                    )
                    .await?;
            }
            return self.describe(moving).await;
        }
        let row = checked.tenant;
        let at = now();
        let model = entity::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(row.id),
            from_placement_id: Set(row.placement_id),
            to_placement_id: Set(checked.to.id),
            keep_source: Set(keep_source),
            state: Set(COPYING.to_owned()),
            tables: Set("[]".to_owned()),
            error: Set(None),
            runner: Set(None),
            runner_until: Set(None),
            moving_since: Set(at),
            requested_by: Set(actor.user_id),
            created_at: Set(at),
            updated_at: Set(at),
            finished_at: Set(None),
        };
        // Khách thành `moving` và lượt dời được ghi cùng lúc (cùng database điều khiển): hai lệnh dời
        // cùng lúc thì điều kiện `status = active` chỉ cho một bên qua.
        let transaction = self.control().begin_write().await?;
        let updated = tenant::Entity::update_many()
            .col_expr(tenant::Column::Status, Expr::value(TenantStatus::Moving.as_str()))
            .col_expr(tenant::Column::UpdatedAt, Expr::value(at))
            .filter(tenant::Column::Id.eq(row.id))
            .filter(tenant::Column::Status.eq(TenantStatus::Active.as_str()))
            .filter(tenant::Column::PlacementId.eq(row.placement_id))
            .exec(&transaction)
            .await?;
        if updated.rows_affected != 1 {
            return Err(errors::tenant_move_not_active(TenantStatus::Moving.as_str()));
        }
        let moving = model.insert(&transaction).await?;
        if enqueue {
            self.jobs
                .enqueue_database(
                    &transaction,
                    &MoveTenant { move_id: moving.id },
                    EnqueueOptions::default(),
                )
                .await?;
        }
        transaction.commit().await?;
        self.registry.invalidate(TenantId::from_uuid(row.id));
        tracing::info!(
            move_id = %moving.id,
            tenant_id = %row.id,
            from = %checked.from.name,
            to = %checked.to.name,
            keep_source,
            "tenant move started; the tenant answers 503 until the copy is done"
        );
        self.describe(moving).await
    }

    /// Chạy lượt dời `id` tới xong (`done` / `aborted`). `runner` phân biệt bên chạy (CLI, job).
    /// Lỗi: lượt dời dừng ở trạng thái đang có, lỗi được ghi vào `error`; gọi lại thì chạy tiếp.
    pub async fn run(&self, id: Uuid, runner: &str) -> Result<TenantMove, Error> {
        loop {
            let moving = self.step(id, runner).await?;
            if !UNFINISHED.contains(&moving.state.as_str()) {
                return Ok(moving);
            }
        }
    }

    /// Làm một bước của lượt dời `id` (`copying` → `copied` → `switched` → `done`) rồi trả lượt dời.
    /// Lỗi được ghi vào `error` của lượt dời.
    pub async fn step(&self, id: Uuid, runner: &str) -> Result<TenantMove, Error> {
        let Some(moving) = self.acquire(id, runner).await? else {
            let moving = self.find(id).await?;
            return self.describe(moving).await;
        };
        let advanced = self.holding(id, runner, self.advance(&moving, runner)).await;
        let released = self.release(id, runner).await;
        match advanced {
            Ok(moving) => {
                released?;
                self.describe(moving).await
            }
            Err(error) => {
                let message = describe(&error);
                tracing::warn!(move_id = %id, error = %message, "tenant move stopped");
                let _ = Entity::update_many()
                    .col_expr(Column::Error, Expr::value(message))
                    .col_expr(Column::UpdatedAt, Expr::value(now()))
                    .filter(Column::Id.eq(id))
                    .exec(self.control().write())
                    .await;
                Err(error)
            }
        }
    }

    pub(super) async fn find(&self, id: Uuid) -> Result<Model, Error> {
        Entity::find_by_id(id)
            .one(self.control().write())
            .await?
            .ok_or_else(|| errors::tenant_move_not_found(&id.to_string()))
    }

    /// Nhận lease của lượt dời `id`. `None`: lượt dời đã kết thúc. Bên khác đang giữ:
    /// `tenant_move_running`.
    pub(super) async fn acquire(&self, id: Uuid, runner: &str) -> Result<Option<Model>, Error> {
        let at = now();
        let until = at + chrono::Duration::from_std(LEASE).map_err(Error::internal)?;
        let taken = Entity::update_many()
            .col_expr(Column::Runner, Expr::value(runner))
            .col_expr(Column::RunnerUntil, Expr::value(until))
            .filter(Column::Id.eq(id))
            .filter(Column::State.is_in(UNFINISHED))
            .filter(
                Condition::any()
                    .add(Column::RunnerUntil.is_null())
                    .add(Column::RunnerUntil.lt(at))
                    .add(Column::Runner.eq(runner)),
            )
            .exec(self.control().write())
            .await?;
        let moving = self.find(id).await?;
        if taken.rows_affected == 1 {
            return Ok(Some(moving));
        }
        if UNFINISHED.contains(&moving.state.as_str()) {
            let placements = self.placements().await?;
            let to = placements
                .iter()
                .find(|placement| placement.id == moving.to_placement_id)
                .map_or_else(|| moving.to_placement_id.to_string(), |p| p.name.clone());
            return Err(errors::tenant_move_running(&to));
        }
        Ok(None)
    }

    pub(super) async fn release(&self, id: Uuid, runner: &str) -> Result<(), Error> {
        Entity::update_many()
            .col_expr(Column::RunnerUntil, Expr::value(Option::<DateTime<Utc>>::None))
            .filter(Column::Id.eq(id))
            .filter(Column::Runner.eq(runner))
            .exec(self.control().write())
            .await?;
        Ok(())
    }

    /// Chạy `work`, gia hạn lease của lượt dời `id` mỗi một phần ba thời hạn cho tới khi xong.
    /// Gia hạn hỏng (vd database điều khiển SQLite đang bận vì chính bước chép) chỉ ghi log: bên
    /// khác cũng cần kết nối ghi đó để nhận lease, nên không chen vào được.
    pub(super) async fn holding<T>(
        &self,
        id: Uuid,
        runner: &str,
        work: impl Future<Output = Result<T, Error>>,
    ) -> Result<T, Error> {
        let control = self.control().clone();
        let runner = runner.to_owned();
        let renew = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(LEASE / 3);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                let until = now()
                    + chrono::Duration::seconds(
                        <i64 as TryFrom<u64>>::try_from(LEASE.as_secs()).unwrap_or(120),
                    );
                let renewed = Entity::update_many()
                    .col_expr(Column::RunnerUntil, Expr::value(until))
                    .filter(Column::Id.eq(id))
                    .filter(Column::Runner.eq(runner.as_str()))
                    .exec(control.write())
                    .await;
                if let Err(error) = renewed {
                    tracing::warn!(move_id = %id, error = %error, "cannot extend the tenant move lease");
                }
            }
        });
        let outcome = work.await;
        renew.abort();
        outcome
    }

    pub(super) async fn advance(&self, moving: &Model, runner: &str) -> Result<Model, Error> {
        match moving.state.as_str() {
            COPYING => self.copy(moving, runner).await,
            COPIED => self.switch(moving, runner).await,
            SWITCHED => self.clean(moving, runner).await,
            _ => Ok(moving.clone()),
        }
    }

    pub(super) async fn placement(&self, id: Uuid) -> Result<placement::Model, Error> {
        placement::Entity::find_by_id(id)
            .one(self.control().write())
            .await?
            .ok_or_else(|| errors::tenant_placement_not_found(&id.to_string()))
    }

    pub(super) async fn tables(
        &self,
        source: &Database,
        target: &Database,
    ) -> Result<TenantTables, Error> {
        let setup = self.setup.get()?;
        TenantTables::read(source, target, &setup.modules, &setup.control_plane)
            .await
            .map_err(transfer)
    }

    /// Ghi trạng thái mới của lượt dời — chỉ khi bên này còn giữ lease và trạng thái chưa đổi.
    pub(super) async fn set_state<C: ConnectionTrait>(
        &self,
        connection: &C,
        moving: &Model,
        runner: &str,
        to: &str,
        state: &str,
        tables: Option<&[CopiedTable]>,
    ) -> Result<(), Error> {
        let at = now();
        let mut update = Entity::update_many()
            .col_expr(Column::State, Expr::value(state))
            .col_expr(Column::Error, Expr::value(Option::<String>::None))
            .col_expr(Column::UpdatedAt, Expr::value(at))
            .filter(Column::Id.eq(moving.id))
            .filter(Column::State.eq(moving.state.as_str()))
            .filter(Column::Runner.eq(runner));
        if let Some(tables) = tables {
            let rows: Vec<TenantTableRows> =
                tables.iter().cloned().map(TenantTableRows::from).collect();
            let json = serde_json::to_string(&rows).map_err(Error::internal)?;
            update = update.col_expr(Column::Tables, Expr::value(json));
        }
        if state == DONE || state == ABORTED {
            update = update
                .col_expr(Column::FinishedAt, Expr::value(at))
                .col_expr(Column::RunnerUntil, Expr::value(Option::<DateTime<Utc>>::None));
        }
        if update.exec(connection).await?.rows_affected != 1 {
            // Bên này mất lease (giữ quá lâu mà không gia hạn được) và bên khác đã nhận lượt dời.
            return Err(errors::tenant_move_running(to));
        }
        Ok(())
    }
}
