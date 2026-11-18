//! `ComplaintIntake`: phiếu hỗ trợ (`support`, bước 53) chuyển thành khiếu nại người tiêu dùng — vụ
//! việc `consumer_complaint` có hạn luật định của module này, không giữ hai bản. Đích của vụ việc là
//! chính phiếu (`support.ticket:<id>`, tra qua sổ đối tượng); gọi lại cùng phiếu trả vụ việc cũ.

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use crate::api::{CaseRef, ComplaintFromTicket};
use crate::cases::{NewCase, Reporter, check_case};
use crate::entity::case;
use crate::service::ComplianceService;

impl ComplianceService {
    pub(crate) async fn complaint_from_ticket(
        &self,
        tenant: &TenantContext,
        complaint: ComplaintFromTicket,
    ) -> Result<CaseRef, Error> {
        if let Some(existing) = self.complaint_of(tenant, &complaint.source_ref).await? {
            return Ok(CaseRef { id: existing, created: false });
        }
        let kind = if complaint.reporter_user_id.is_some() { "user" } else { "guest" };
        let input = NewCase {
            kind: "consumer_complaint".to_owned(),
            target: Some(complaint.source_ref.clone()),
            respondent_org_id: Some(complaint.org_id),
            reporter: Reporter {
                kind: kind.to_owned(),
                user_id: complaint.reporter_user_id,
                name: complaint.name,
                email: complaint.email,
                phone: complaint.phone,
            },
            claim: complaint.claim,
            rights: Vec::new(),
        };
        let mut problems = Vec::new();
        check_case(&input, &mut problems);
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let opened = self
            .insert_case(tenant, Actor::system(), "support", input, None, None)
            .await?
            .ok_or_else(|| Error::internal("a complaint from a ticket cannot be a replay"))?;
        self.mail_opened(tenant, &opened).await;
        Ok(CaseRef { id: opened.case.id, created: true })
    }

    async fn complaint_of(
        &self,
        tenant: &TenantContext,
        source_ref: &str,
    ) -> Result<Option<uuid::Uuid>, Error> {
        Ok(case::Entity::find()
            .filter(case::Column::Kind.eq("consumer_complaint"))
            .filter(case::Column::Target.eq(source_ref))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .map(|row| row.id))
    }
}
