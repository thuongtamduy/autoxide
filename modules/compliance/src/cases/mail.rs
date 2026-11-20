//! Mail báo đã nhận / đã quyết định cho người báo và người xử lý.

use super::*;

impl ComplianceService {
    pub(super) fn tracking_link(&self, id: Uuid, token: &str) -> String {
        match self.case_tracking_url.as_deref() {
            Some(url) => url.replace("{case}", &id.to_string()).replace("{token}", token),
            None => format!("/api/v1/compliance/public/cases/{id}?token={token}"),
        }
    }

    pub(super) async fn language(&self, tenant: &TenantContext) -> Language {
        match self.jurisdiction(tenant).await.as_deref() {
            Ok("vn") => Language::Vi,
            _ => Language::En,
        }
    }

    pub(super) async fn staff_email(&self, tenant: &TenantContext) -> Option<String> {
        let contact = self.policy(tenant, "complaints.contact").await.ok().flatten()?;
        contact
            .get("email")
            .and_then(Value::as_str)
            .filter(|email| autoxide_kernel::email::is_valid(email))
            .map(str::to_owned)
    }

    /// Mail báo đã nhận cho người báo và cho người xử lý (`complaints.contact.email`). Lỗi gửi chỉ
    /// ghi log: vụ việc đã lưu.
    pub(crate) async fn mail_opened(&self, tenant: &TenantContext, opened: &Opened) {
        let Some(mailer) = self.mailer.clone() else { return };
        let row = &opened.case;
        let language = self.language(tenant).await;
        let (reference, ack_due, decide_by, id) = (
            reference(row.id),
            row.ack_due_at.format("%Y-%m-%d %H:%M").to_string(),
            row.decide_by.format("%Y-%m-%d %H:%M").to_string(),
            row.id.to_string(),
        );
        if let Some(email) = row.reporter_email.as_deref() {
            let link = self.tracking_link(row.id, &opened.token);
            let mut mail = RECEIVED_MAIL.mail(
                email,
                &self.translations,
                language,
                &[
                    ("reference", &reference),
                    ("ack_due", &ack_due),
                    ("decide_by", &decide_by),
                    ("link", &link),
                ],
            );
            mail.dedupe_key = Some(format!("compliance.case:{}:received", row.id));
            if let Err(error) = mailer.send(tenant, mail).await {
                tracing::warn!(case_id = %row.id, error = %error, "cannot queue the case receipt mail");
            }
        }
        if let Some(email) = self.staff_email(tenant).await {
            let target = if row.target.is_empty() { "-" } else { row.target.as_str() };
            let mut mail = OPENED_MAIL.mail(
                &email,
                &self.translations,
                language,
                &[
                    ("reference", &reference),
                    ("kind", &row.kind),
                    ("target", target),
                    ("ack_due", &ack_due),
                    ("decide_by", &decide_by),
                    ("id", &id),
                ],
            );
            mail.dedupe_key = Some(format!("compliance.case:{}:opened", row.id));
            if let Err(error) = mailer.send(tenant, mail).await {
                tracing::warn!(case_id = %row.id, error = %error, "cannot queue the new case mail");
            }
        }
    }

    pub(super) async fn mail_decided(&self, tenant: &TenantContext, row: &case::Model) {
        let (Some(mailer), Some(email)) = (self.mailer.clone(), row.reporter_email.as_deref())
        else {
            return;
        };
        let language = self.language(tenant).await;
        let reason = row.decision_reason.clone().unwrap_or_default();
        let mut mail = DECIDED_MAIL.mail(
            email,
            &self.translations,
            language,
            &[("reference", &reference(row.id)), ("decision", &row.decision), ("reason", &reason)],
        );
        mail.dedupe_key = Some(format!("compliance.case:{}:decided", row.id));
        if let Err(error) = mailer.send(tenant, mail).await {
            tracing::warn!(case_id = %row.id, error = %error, "cannot queue the case decision mail");
        }
    }
}
