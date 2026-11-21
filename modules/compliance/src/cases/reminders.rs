//! Nhắc hạn: lịch hằng giờ báo vụ còn mở đã quá hạn xác nhận hay hạn quyết định.

use super::*;
use autoxide_kernel::time::Timestamp;

impl ComplianceService {
    /// Nhắc mọi vụ còn mở đã quá hạn xác nhận hay hạn quyết định trong `database` (lịch hằng giờ):
    /// mỗi mốc một lần (`UPDATE … WHERE overdue_notified = <cũ>` đổi đúng một dòng). Trả số lần nhắc.
    pub(crate) async fn remind_cases(
        &self,
        database: &autoxide_kernel::database::Database,
    ) -> Result<u64, Error> {
        let at = Timestamp::now();
        let due: Vec<(Uuid, Uuid)> = case::Entity::find()
            .select_only()
            .column(case::Column::TenantId)
            .column(case::Column::Id)
            .filter(case::Column::Status.is_in(OPEN))
            .filter(case::Column::SpamReason.is_null())
            .filter(
                Condition::any()
                    .add(
                        Condition::all()
                            .add(case::Column::OverdueNotified.eq(""))
                            .add(case::Column::AcknowledgedAt.is_null())
                            .add(case::Column::AckDueAt.lt(at)),
                    )
                    .add(
                        Condition::all()
                            .add(case::Column::OverdueNotified.ne("decide"))
                            .add(case::Column::DecideBy.lt(at)),
                    ),
            )
            .order_by_asc(case::Column::Id)
            .limit(500)
            .into_tuple()
            .all(database.read())
            .await?;
        let mut sent = 0;
        for (tenant_id, id) in due {
            let Ok(tenant) = self.tenant(TenantId::from_uuid(tenant_id)).await else {
                continue;
            };
            let Some(row) =
                case::Entity::find_by_id(id).scoped(&tenant).one(tenant.database().read()).await?
            else {
                continue;
            };
            let (step, due_at) = if row.decide_by < at {
                ("decide", row.decide_by)
            } else {
                ("ack", row.ack_due_at)
            };
            if row.overdue_notified == step {
                continue;
            }
            let transaction = tenant.database().begin_write().await?;
            let claimed = case::Entity::update_many()
                .col_expr(case::Column::OverdueNotified, Expr::value(step))
                .filter(case::Column::Id.eq(id))
                .filter(case::Column::OverdueNotified.eq(row.overdue_notified.as_str()))
                .scoped(&tenant)
                .exec(&transaction)
                .await?
                .rows_affected;
            if claimed != 1 {
                continue;
            }
            let event = events::CaseOverdue(json!({
                "case_id": row.id, "kind": row.kind, "status": row.status, "step": step, "due_at": due_at,
            }));
            self.outbox
                .record(
                    &transaction,
                    &tenant,
                    Actor::system(),
                    Some(format!("compliance.case:{id}")),
                    &event,
                )
                .await?;
            transaction.commit().await?;
            sent += 1;
            self.mail_overdue(&tenant, &row, step, due_at).await;
        }
        Ok(sent)
    }

    pub(super) async fn mail_overdue(
        &self,
        tenant: &TenantContext,
        row: &case::Model,
        step: &str,
        due_at: Timestamp,
    ) {
        let (Some(mailer), Some(email)) = (self.mailer.clone(), self.staff_email(tenant).await)
        else {
            return;
        };
        let language = self.language(tenant).await;
        let step_text = if step == "ack" { "acknowledge" } else { "decide" };
        let (reference, due, id) =
            (reference(row.id), due_at.format("%Y-%m-%d %H:%M").to_string(), row.id.to_string());
        let mut mail = OVERDUE_MAIL.mail(
            &email,
            &self.translations,
            language,
            &[
                ("reference", &reference),
                ("kind", &row.kind),
                ("step", step_text),
                ("due", &due),
                ("id", &id),
            ],
        );
        mail.dedupe_key = Some(format!("compliance.case:{}:overdue:{step}", row.id));
        if let Err(error) = mailer.send(tenant, mail).await {
            tracing::warn!(case_id = %row.id, error = %error, "cannot queue the case reminder mail");
        }
    }
}
