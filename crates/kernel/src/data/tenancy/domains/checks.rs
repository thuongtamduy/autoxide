//! Background checks of tenant domains: the job `kernel.verify_domains` rechecks `pending` domains and
//! verified ones past their daily recheck, and looks up the TXT record.
//! Moved out of `domains.rs` unchanged (D-20261118-file-length).

use super::*;

impl TenantDomains {
    /// Rechecks the `pending` domains on `database` (job `kernel.verify_domains`): the right record
    /// → `verified`; waiting more than 7 days → `failed`. Returns the number of domains examined.
    pub async fn check_pending(&self, database: &Database) -> Result<usize, Error> {
        let rows = Entity::find()
            .filter(Column::Status.eq(PENDING))
            .order_by_asc(Column::LastCheckedAt)
            .order_by_asc(Column::Id)
            .limit(SWEEP_BATCH)
            .all(database.read())
            .await?;
        let deadline = Timestamp::now() - SignedDuration::from_hours(24 * PENDING_DAYS);
        let count = rows.len();
        for row in rows {
            if row.updated_at <= deadline {
                self.expire(database, row).await?;
            } else {
                self.check_on(database, Actor::system(), row).await?;
            }
        }
        Ok(count)
    }

    /// `pending` past its deadline → `failed`.
    async fn expire(&self, database: &Database, row: Model) -> Result<(), Error> {
        let now = now();
        let reason = format!(
            "Not verified within {PENDING_DAYS} days. Set the TXT record, then verify again."
        );
        let updated = Entity::update_many()
            .col_expr(Column::Status, Expr::value(FAILED))
            .col_expr(Column::CheckError, Expr::value(reason.clone()))
            .col_expr(Column::UpdatedAt, Expr::value(now))
            .filter(Column::Id.eq(row.id))
            .filter(Column::Status.eq(PENDING))
            .exec(database.write())
            .await?;
        if updated.rows_affected > 0 {
            let after = Model {
                status: FAILED.to_owned(),
                check_error: Some(reason),
                updated_at: now,
                ..row
            };
            self.record_failed(&after).await?;
            tracing::info!(domain = %after.domain, "tenant domain not verified in time; marked failed");
        }
        Ok(())
    }

    /// Rechecks the `verified` domains on `database` more than [`RECHECK_HOURS`] hours past their previous
    /// check (job `kernel.verify_domains`): record found → count restarts from 0; not found for the
    /// [`MAX_CHECK_FAILURES`]-th consecutive time → `failed`. Returns the number of domains examined.
    pub async fn recheck_verified(&self, database: &Database) -> Result<usize, Error> {
        let due = Timestamp::now() - SignedDuration::from_hours(RECHECK_HOURS);
        let rows = Entity::find()
            .filter(Column::Status.eq(VERIFIED))
            .filter(
                sea_orm::Condition::any()
                    .add(Column::LastCheckedAt.is_null())
                    .add(Column::LastCheckedAt.lt(due)),
            )
            .order_by_asc(Column::LastCheckedAt)
            .order_by_asc(Column::Id)
            .limit(SWEEP_BATCH)
            .all(database.read())
            .await?;
        let count = rows.len();
        for row in rows {
            self.recheck(database, row).await?;
        }
        Ok(count)
    }

    /// Looks up DNS for the `verified` domain `row` then writes the result; returns the row after writing.
    /// Every write only writes while the failure count has not been changed by another party: two
    /// concurrent checks count only once and only one writes the event.
    pub(super) async fn recheck(&self, database: &Database, row: Model) -> Result<Model, Error> {
        // Tra DNS NGOÀI mọi transaction: gọi mạng có thể chậm.
        let probe = self.probe(&row.domain, &row.verification_token).await;
        let now = now();
        let unchanged = |update: sea_orm::UpdateMany<Entity>| {
            update
                .filter(Column::Id.eq(row.id))
                .filter(Column::Status.eq(VERIFIED))
                .filter(Column::CheckFailures.eq(row.check_failures))
        };
        match probe {
            Probe::Found => {
                unchanged(Entity::update_many())
                    .col_expr(Column::CheckFailures, Expr::value(0))
                    .col_expr(Column::LastCheckedAt, Expr::value(now))
                    .col_expr(Column::CheckError, Expr::value(Option::<String>::None))
                    .exec(database.write())
                    .await?;
            }
            Probe::Unreachable(reason) => {
                // Không biết bản ghi còn hay không: không tính là trượt, kiểm lại sau một ngày.
                let reason: String = reason.chars().take(CHECK_ERROR_MAX).collect();
                unchanged(Entity::update_many())
                    .col_expr(Column::LastCheckedAt, Expr::value(now))
                    .col_expr(Column::CheckError, Expr::value(reason))
                    .exec(database.write())
                    .await?;
            }
            Probe::Missing(reason) => {
                let failures = row.check_failures.saturating_add(1);
                let reason: String = reason.chars().take(CHECK_ERROR_MAX).collect();
                let mut update = unchanged(Entity::update_many())
                    .col_expr(Column::CheckFailures, Expr::value(failures))
                    .col_expr(Column::LastCheckedAt, Expr::value(now))
                    .col_expr(Column::CheckError, Expr::value(reason));
                let failed = failures >= MAX_CHECK_FAILURES;
                if failed {
                    update = update
                        .col_expr(Column::Status, Expr::value(FAILED))
                        .col_expr(Column::UpdatedAt, Expr::value(now));
                }
                let updated = update.exec(database.write()).await?;
                if failed && updated.rows_affected > 0 {
                    self.registry.invalidate_domain(&row.domain);
                    let after = self.find_in(database, row.id).await?;
                    self.record_failed(&after).await?;
                    tracing::warn!(
                        tenant_id = %after.tenant_id,
                        domain = %after.domain,
                        failures,
                        "tenant domain lost its DNS record; marked failed"
                    );
                    return Ok(after);
                }
            }
        }
        self.find_in(database, row.id).await
    }

    /// Writes `kernel.domain_failed` for a domain that has just become `failed` (skipped if the tenant no
    /// longer exists).
    async fn record_failed(&self, row: &Model) -> Result<(), Error> {
        match self.registry.tenant(TenantId::from_uuid(row.tenant_id)).await {
            Ok(tenant) => {
                let event = DomainFailed(changed(row));
                self.record(&tenant, Actor::system(), row, &event).await
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Looks up DNS for `row` (on the control database) and writes the result.
    pub(super) async fn check(&self, actor: Actor, row: Model) -> Result<Model, Error> {
        let database = self.control().clone();
        self.check_on(&database, actor, row).await
    }

    /// Looks up DNS for `row` (a row of `database`) then writes the result; returns the row after writing.
    /// Only one party (a request or a job running at the same time) can move `pending` / `failed` →
    /// `verified` and write the event.
    async fn check_on(
        &self,
        database: &Database,
        actor: Actor,
        row: Model,
    ) -> Result<Model, Error> {
        // Khách khác đã xác minh tên miền này: lượt chờ của khách này thua ngay, khỏi chờ bản ghi DNS vô ích (P-4).
        if self.verified_elsewhere(database, &row).await? {
            return self.lost_race(database, row, now()).await;
        }
        // Tra DNS NGOÀI mọi transaction: gọi mạng có thể chậm.
        let probe = self.probe(&row.domain, &row.verification_token).await;
        let now = now();
        match probe {
            Probe::Found => {
                let verified = Entity::update_many()
                    .col_expr(Column::Status, Expr::value(VERIFIED))
                    .col_expr(Column::VerifiedAt, Expr::value(now))
                    .col_expr(Column::LastCheckedAt, Expr::value(now))
                    .col_expr(Column::CheckError, Expr::value(Option::<String>::None))
                    .col_expr(Column::CheckFailures, Expr::value(0))
                    .col_expr(Column::UpdatedAt, Expr::value(now))
                    .filter(Column::Id.eq(row.id))
                    .filter(Column::Status.is_in([PENDING, FAILED]))
                    .filter(Column::VerificationToken.eq(row.verification_token.as_str()))
                    .exec(database.write())
                    .await;
                let updated = match verified {
                    Ok(updated) => updated,
                    // Khách khác đã xác minh tên miền này trước (unique index một phần trên `verified`, P-4):
                    // lượt chờ của khách này hỏng, kèm lý do.
                    Err(error) if is_unique_violation(&error) => {
                        return self.lost_race(database, row, now).await;
                    }
                    Err(error) => return Err(error.into()),
                };
                self.registry.invalidate_domain(&row.domain);
                let after = self.find_in(database, row.id).await?;
                if updated.rows_affected > 0 {
                    let tenant = self.registry.tenant(TenantId::from_uuid(after.tenant_id)).await?;
                    let event = DomainVerified(changed(&after));
                    self.record(&tenant, actor, &after, &event).await?;
                    tracing::info!(
                        tenant_id = %tenant.id(),
                        domain = %after.domain,
                        "tenant domain verified"
                    );
                }
                Ok(after)
            }
            Probe::Missing(reason) | Probe::Unreachable(reason) => {
                let reason: String = reason.chars().take(CHECK_ERROR_MAX).collect();
                let mut update = Entity::update_many()
                    .col_expr(Column::LastCheckedAt, Expr::value(now))
                    .col_expr(Column::CheckError, Expr::value(reason));
                if row.status == FAILED {
                    // Xác minh lại tên miền đã hỏng: chờ thêm 7 ngày từ bây giờ.
                    update = update
                        .col_expr(Column::Status, Expr::value(PENDING))
                        .col_expr(Column::UpdatedAt, Expr::value(now));
                }
                update
                    .filter(Column::Id.eq(row.id))
                    .filter(Column::Status.eq(row.status.as_str()))
                    .exec(database.write())
                    .await?;
                self.find_in(database, row.id).await
            }
        }
    }

    /// Looks up the TXT record of `domain`, compared with `token`.
    async fn probe(&self, domain: &str, token: &str) -> Probe {
        let name = format!("{CHALLENGE_PREFIX}.{domain}");
        let expected = format!("{CHALLENGE_VALUE_PREFIX}{token}");
        let lookup = self.dns.get();
        match tokio::time::timeout(LOOKUP_TIMEOUT, lookup.txt(&name)).await {
            Ok(Ok(values)) if values.iter().any(|value| value.trim() == expected) => Probe::Found,
            Ok(Ok(values)) if values.is_empty() => {
                Probe::Missing(format!("No TXT record found at {name}."))
            }
            Ok(Ok(_)) => Probe::Missing(format!(
                "The TXT record at {name} does not hold the expected value."
            )),
            Ok(Err(error)) => {
                tracing::warn!(domain, error = %error.message(), "DNS lookup failed");
                Probe::Unreachable(format!("DNS lookup failed: {}", error.message()))
            }
            Err(_) => Probe::Unreachable(format!(
                "DNS lookup timed out after {} seconds.",
                LOOKUP_TIMEOUT.as_secs()
            )),
        }
    }
}

/// Schedule of the job `kernel.verify_domains`: every 10 minutes, only on the control database (the only place
/// that has domains).
pub(crate) fn schedule() -> Schedule {
    Schedule::new(VERIFY_DOMAINS, VERIFY_DOMAINS, Duration::from_secs(600)).on_control_only()
}

/// Runner of the job `kernel.verify_domains` (a job of the whole control database).
pub(crate) struct VerifyDomains(pub(crate) Arc<TenantDomains>);

#[async_trait::async_trait]
impl JobHandler for VerifyDomains {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let checked = self.0.check_pending(&job.database).await?;
        if checked > 0 {
            tracing::info!(checked, "pending tenant domains checked");
        }
        let rechecked = self.0.recheck_verified(&job.database).await?;
        if rechecked > 0 {
            tracing::info!(rechecked, "verified tenant domains checked again");
        }
        Ok(())
    }
}
