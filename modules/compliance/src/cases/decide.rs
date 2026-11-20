//! Ẩn / hiện đích, quyết định và đóng vụ việc.

use super::*;

impl ComplianceService {
    /// Ẩn đích của vụ (chờ xử lý). Cần `compliance.cases.handle`. Đã ẩn thì không đổi gì.
    pub(crate) async fn restrict_case_target(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        reason: &str,
    ) -> Result<case::Model, Error> {
        principal.require(&CASES_HANDLE)?;
        let before = self.find_case(tenant, id).await?;
        check_body(reason, &[]).map_err(|_| {
            Error::validation(vec![
                FieldError::new("reason", "length", "must be 1 to {max} characters")
                    .with_param("max", CLAIM_MAX),
            ])
        })?;
        if !OPEN.contains(&before.status.as_str()) || before.target.is_empty() {
            return Err(state_invalid(&before.status));
        }
        if before.restricted {
            return Ok(before);
        }
        let mut after = before.clone();
        let mut entries = Vec::new();
        self.acknowledge(&mut after, &mut entries, tenant);
        after.restricted = true;
        entries.push(entry(
            tenant,
            id,
            "restriction",
            "staff",
            Some(principal.user_id()),
            format!("restricted: {}", reason.trim()),
            &[],
        ));
        self.save_case(tenant, Actor::from(principal), &before, after, entries, Some(true), false)
            .await
    }

    /// Hiện lại đích trước khi quyết định. Cần `compliance.cases.handle`.
    pub(crate) async fn restore_case_target(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        reason: &str,
    ) -> Result<case::Model, Error> {
        principal.require(&CASES_HANDLE)?;
        let before = self.find_case(tenant, id).await?;
        if reason.trim().is_empty() || reason.chars().count() > CLAIM_MAX {
            return Err(Error::validation(vec![
                FieldError::new("reason", "length", "must be 1 to {max} characters")
                    .with_param("max", CLAIM_MAX),
            ]));
        }
        if !OPEN.contains(&before.status.as_str()) || !before.restricted {
            return Err(state_invalid(&before.status));
        }
        let mut after = before.clone();
        let mut entries = Vec::new();
        self.acknowledge(&mut after, &mut entries, tenant);
        after.restricted = false;
        entries.push(entry(
            tenant,
            id,
            "restriction",
            "staff",
            Some(principal.user_id()),
            format!("restored: {}", reason.trim()),
            &[],
        ));
        self.save_case(tenant, Actor::from(principal), &before, after, entries, Some(false), false)
            .await
    }

    /// Quyết định (phiên đăng nhập của người có `compliance.cases.handle`, không nhận API key):
    /// `upheld` / `partially_upheld` giữ (hay đặt) ẩn đích, `rejected` hiện lại.
    pub(crate) async fn decide_case(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        decision: &str,
        reason: &str,
    ) -> Result<case::Model, Error> {
        principal.require(&CASES_HANDLE)?;
        require_session(principal)?;
        let before = self.find_case(tenant, id).await?;
        let mut problems = Vec::new();
        if !DECISIONS.contains(&decision) {
            problems.push(FieldError::new(
                "decision",
                "invalid",
                "must be `upheld`, `rejected` or `partially_upheld`",
            ));
        }
        if reason.trim().is_empty() || reason.chars().count() > CLAIM_MAX {
            problems.push(
                FieldError::new("reason", "length", "must be 1 to {max} characters")
                    .with_param("max", CLAIM_MAX),
            );
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        if !OPEN.contains(&before.status.as_str()) {
            return Err(state_invalid(&before.status));
        }
        let mut after = before.clone();
        let mut entries = Vec::new();
        self.acknowledge(&mut after, &mut entries, tenant);
        let restriction = match decision {
            "rejected" if before.restricted => Some(false),
            "upheld" | "partially_upheld" if !before.restricted && !before.target.is_empty() => {
                Some(true)
            }
            _ => None,
        };
        if let Some(restricted) = restriction {
            after.restricted = restricted;
            let text = if restricted { "restricted: decision" } else { "restored: decision" };
            entries.push(entry(
                tenant,
                id,
                "restriction",
                "staff",
                Some(principal.user_id()),
                text,
                &[],
            ));
        }
        after.status = "decided".to_owned();
        after.decision = decision.to_owned();
        after.decision_reason = Some(reason.trim().to_owned());
        after.decided_at = Some(now());
        after.decided_by = Some(principal.user_id());
        entries.push(entry(
            tenant,
            id,
            "decision",
            "staff",
            Some(principal.user_id()),
            format!("{decision}: {}", reason.trim()),
            &[],
        ));
        let saved = self
            .save_case(tenant, Actor::from(principal), &before, after, entries, restriction, true)
            .await?;
        self.mail_decided(tenant, &saved).await;
        Ok(saved)
    }

    /// Đóng vụ (`withdrawn`: người báo rút). Cần `compliance.cases.handle`. Vụ chưa kết luận có vi
    /// phạm thì hiện lại đích.
    pub(crate) async fn close_case(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        withdrawn: bool,
        reason: &str,
    ) -> Result<case::Model, Error> {
        principal.require(&CASES_HANDLE)?;
        let before = self.find_case(tenant, id).await?;
        if reason.trim().is_empty() || reason.chars().count() > CLAIM_MAX {
            return Err(Error::validation(vec![
                FieldError::new("reason", "length", "must be 1 to {max} characters")
                    .with_param("max", CLAIM_MAX),
            ]));
        }
        let closable =
            OPEN.contains(&before.status.as_str()) || (before.status == "decided" && !withdrawn);
        if !closable {
            return Err(state_invalid(&before.status));
        }
        let mut after = before.clone();
        let mut entries = Vec::new();
        if after.acknowledged_at.is_none() {
            after.acknowledged_at = Some(now());
        }
        let keep = matches!(before.decision.as_str(), "upheld" | "partially_upheld");
        let restriction = (before.restricted && !keep).then_some(false);
        if restriction.is_some() {
            after.restricted = false;
            entries.push(entry(
                tenant,
                id,
                "restriction",
                "staff",
                Some(principal.user_id()),
                "restored: case closed",
                &[],
            ));
        }
        after.status = if withdrawn { "withdrawn" } else { "closed" }.to_owned();
        entries.push(entry(
            tenant,
            id,
            "status",
            "staff",
            Some(principal.user_id()),
            format!("{}: {}", after.status, reason.trim()),
            &[],
        ));
        self.save_case(tenant, Actor::from(principal), &before, after, entries, restriction, false)
            .await
    }
}
