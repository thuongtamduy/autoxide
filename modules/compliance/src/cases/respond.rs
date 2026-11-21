//! Bên bị phản ánh: tra vụ của mình và gửi phản hồi; tra vụ bằng mã của người báo.

use super::*;
use autoxide_kernel::time::Timestamp;

impl ComplianceService {
    /// `principal` là bên bị phản ánh của vụ: chủ đối tượng, hay người có `compliance.cases.respond`
    /// trên nút bị phản ánh.
    pub(super) async fn is_respondent(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        row: &case::Model,
    ) -> Result<bool, Error> {
        let read = tenant.database().read();
        let listed = case_respondent::Entity::find_by_id((
            tenant.id().as_uuid(),
            row.id,
            principal.user_id(),
        ))
        .one(read)
        .await?
        .is_some();
        if listed {
            return Ok(true);
        }
        match row.respondent_org_id {
            Some(org) if principal.has(&CASES_RESPOND) => Ok(principal
                .require_at(read, &CASES_RESPOND, OrgTarget::new(OrgId::from_uuid(org), None))
                .await
                .is_ok()),
            _ => Ok(false),
        }
    }

    /// Vụ việc mà `principal` là bên bị phản ánh (không gồm vụ lớp chống spam giữ lại).
    pub(crate) async fn respondent_cases(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<CaseDetail>, Error> {
        let mut listed = Query::select();
        listed
            .column(case_respondent::Column::CaseId)
            .from(case_respondent::Entity)
            .and_where(case_respondent::Column::TenantId.eq(tenant.id().as_uuid()))
            .and_where(case_respondent::Column::UserId.eq(principal.user_id()));
        let mut any = Condition::any().add(case::Column::Id.in_subquery(listed.to_owned()));
        if principal.has(&CASES_RESPOND)
            && let Ok(scope) =
                principal.scope_condition(&CASES_RESPOND, case::Column::RespondentOrgId, None)
        {
            any = any
                .add(Condition::all().add(case::Column::RespondentOrgId.is_not_null()).add(scope));
        }
        let select = case::Entity::find()
            .scoped(tenant)
            .filter(case::Column::SpamReason.is_null())
            .filter(any);
        let page =
            fetch_page(select, case::Column::Id, |row| row.id, page, tenant.database().read())
                .await?;
        let Page { data: rows, pagination } = page;
        let mut data = Vec::with_capacity(rows.len());
        for row in rows {
            data.push(self.detail(tenant, row).await?);
        }
        Ok(Page { data, pagination })
    }

    /// Phản hồi của bên bị phản ánh, trước `respond_by`.
    pub(crate) async fn respond_case(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        body: &str,
        files: &[Uuid],
    ) -> Result<case_entry::Model, Error> {
        let before = self.find_case(tenant, id).await?;
        // Không phải bên bị phản ánh: như không có vụ này (không lộ vụ việc của người khác).
        if before.spam_reason.is_some() || !self.is_respondent(tenant, principal, &before).await? {
            return Err(case_not_found());
        }
        check_body(body, files)?;
        if !OPEN.contains(&before.status.as_str()) || Timestamp::now() > before.respond_by {
            return Err(response_closed());
        }
        let line =
            entry(tenant, id, "response", "respondent", Some(principal.user_id()), body, files);
        let mut after = before.clone();
        let mut entries = vec![line.clone()];
        if after.status == "awaiting_response" {
            after.status = "under_review".to_owned();
            entries.push(entry(tenant, id, "status", "system", None, "under_review", &[]));
        }
        self.save_case(tenant, Actor::from(principal), &before, after, entries, None, false)
            .await?;
        Ok(line)
    }

    /// Vụ việc theo mã tra cứu của người báo; sai mã → như không có.
    pub(crate) async fn tracked_case(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        token: &str,
    ) -> Result<CaseDetail, Error> {
        let row = self.find_case(tenant, id).await?;
        if token.is_empty() || autoxide_kernel::secret::sha256(token) != row.tracking_hash {
            return Err(case_not_found());
        }
        self.detail(tenant, row).await
    }
}
