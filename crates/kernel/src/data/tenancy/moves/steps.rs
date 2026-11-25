//! Starting, running step by step, leasing, and recording the state of one move.

use super::*;
use crate::time::Timestamp;
use jiff::SignedDuration;

impl TenantMoves {
    /// Starts moving tenant `tenant` to `to` (the tenant becomes `moving`), or returns the move to `to` that
    /// stopped midway so it can continue. The first blocking reason becomes the error. Copies nothing yet:
    /// call [`run`](Self::run) (CLI); with `enqueue`, a background job runs it (REST / MCP).
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

    /// Runs move `id` to completion (`done` / `aborted`). `runner` identifies who runs it (CLI, job).
    /// On error: the move stops in its current state, the error is written to `error`; calling again continues.
    pub async fn run(&self, id: Uuid, runner: &str) -> Result<TenantMove, Error> {
        loop {
            let moving = self.step(id, runner).await?;
            if !UNFINISHED.contains(&moving.state.as_str()) {
                return Ok(moving);
            }
        }
    }

    /// Performs one step of move `id` (`copying` → `copied` → `switched` → `done`) then returns the move.
    /// An error is written to the move's `error`.
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

    /// Takes the lease of move `id`. `None`: the move has ended. Held by another party:
    /// `tenant_move_running`.
    pub(super) async fn acquire(&self, id: Uuid, runner: &str) -> Result<Option<Model>, Error> {
        let at = now();
        let until = at + SignedDuration::try_from(LEASE).map_err(Error::internal)?;
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
            .col_expr(Column::RunnerUntil, Expr::value(Option::<Timestamp>::None))
            .filter(Column::Id.eq(id))
            .filter(Column::Runner.eq(runner))
            .exec(self.control().write())
            .await?;
        Ok(())
    }

    /// Runs `work`, renewing the lease of move `id` every third of its duration until it finishes.
    /// A failed renewal (e.g. the SQLite control database is busy because of the copy step itself) is only
    /// logged: the other party also needs that write connection to take the lease, so it cannot interleave.
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
                    + SignedDuration::from_secs(
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
        self.setup.prepare_dormant(source, target).await?;
        let (modules, control_plane) = self.setup.tenant_tables()?;
        TenantTables::read(source, target, &modules, &control_plane).await.map_err(transfer)
    }

    /// Records the new state of the move, only when this party still holds the lease and the state is unchanged.
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
                .col_expr(Column::RunnerUntil, Expr::value(Option::<Timestamp>::None));
        }
        if update.exec(connection).await?.rows_affected != 1 {
            // Bên này mất lease (giữ quá lâu mà không gia hạn được) và bên khác đã nhận lượt dời.
            return Err(errors::tenant_move_running(to));
        }
        Ok(())
    }
}
