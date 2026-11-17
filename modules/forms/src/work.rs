//! Nguồn việc của hộp việc (bước 51 P2): bài nộp mới (`new`) của những form người gọi xem được
//! (theo phạm vi nút của `forms.review`). Đọc, lưu trữ hay đánh dấu spam thì việc tự hết. Câu đếm
//! dùng index `(tenant_id, form_id, status, id)`.

use std::collections::HashMap;

use autoxide_kernel::access::{OrgTarget, Permission, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::work::{Severity, WorkItem, WorkSource};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};
use uuid::Uuid;

use crate::entity::{form, submission};
use crate::permissions::FORMS_REVIEW;

/// `forms.new_submissions`: bài nộp chưa xử lý.
pub(crate) struct NewSubmissions;

/// (ID, tên) của các form `principal` duyệt được bài.
async fn visible_forms(
    tenant: &TenantContext,
    principal: &Principal,
) -> Result<HashMap<Uuid, String>, Error> {
    let scope = principal.scope_condition(&FORMS_REVIEW, form::Column::OrgId, None)?;
    let rows: Vec<(Uuid, String)> = form::Entity::find()
        .filter(scope)
        .scoped(tenant)
        .select_only()
        .column(form::Column::Id)
        .column(form::Column::Name)
        .into_tuple()
        .all(tenant.database().read())
        .await?;
    Ok(rows.into_iter().collect())
}

#[async_trait::async_trait]
impl WorkSource for NewSubmissions {
    fn id(&self) -> &'static str {
        "forms.new_submissions"
    }

    fn permission(&self) -> Permission {
        FORMS_REVIEW
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        _: OrgTarget,
    ) -> Result<u64, Error> {
        let forms = visible_forms(tenant, principal).await?;
        if forms.is_empty() {
            return Ok(0);
        }
        Ok(submission::Entity::find()
            .scoped(tenant)
            .filter(submission::Column::FormId.is_in(forms.into_keys()))
            .filter(submission::Column::Status.eq("new"))
            .count(tenant.database().read())
            .await?)
    }

    async fn items(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        _: OrgTarget,
        limit: u16,
    ) -> Result<Vec<WorkItem>, Error> {
        let forms = visible_forms(tenant, principal).await?;
        if forms.is_empty() {
            return Ok(Vec::new());
        }
        let rows = submission::Entity::find()
            .scoped(tenant)
            .filter(submission::Column::FormId.is_in(forms.keys().copied()))
            .filter(submission::Column::Status.eq("new"))
            .order_by_asc(submission::Column::Id)
            .limit(u64::from(limit))
            .all(tenant.database().read())
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let name = forms.get(&row.form_id).cloned().unwrap_or_default();
                WorkItem::new(
                    row.id.to_string(),
                    Severity::Attention,
                    "Read the new submission to {form}",
                    "marketing.forms",
                )
                .param("form", name)
            })
            .collect())
    }
}

/// Cùng câu đếm, làm số liệu tóm tắt (bước 51 P4).
#[async_trait::async_trait]
impl autoxide_kernel::summary::SummaryMetric for NewSubmissions {
    fn info(&self) -> autoxide_kernel::summary::MetricInfo {
        autoxide_kernel::summary::MetricInfo::new(
            "forms.new_submissions",
            "New form submissions",
            "Submissions with the status `new` now, in the forms you may review at the chosen unit; spam is not counted.",
            "marketing",
        )
    }

    fn permission(&self) -> Permission {
        FORMS_REVIEW
    }

    async fn value(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        target: OrgTarget,
    ) -> Result<autoxide_kernel::summary::MetricValue, Error> {
        Ok(autoxide_kernel::summary::MetricValue::Count(
            WorkSource::count(self, tenant, principal, target).await?,
        ))
    }
}
