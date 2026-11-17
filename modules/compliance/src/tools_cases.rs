//! Tool MCP của vụ việc (D-363): cùng service, cùng permission với REST. Không có tool quyết định vụ
//! việc hay cung cấp thông tin cho cơ quan — hai việc đó cần phiên đăng nhập của người (REST với API
//! key cũng không làm được, nên không vượt luật kiến trúc 8).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::mcp::{PageInput, Tool};
use autoxide_kernel::pagination::Page;
use autoxide_kernel::tenancy::TenantContext;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::cases::CaseFilter;
use crate::http_cases::{CaseEntryResponse, CaseResponse, describe_case, describe_detail};
use crate::permissions::{CASES_HANDLE, CASES_READ};
use crate::service::ComplianceService;

/// Which cases.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CasesInput {
    /// Only cases with this status (`received`, `under_review`, `awaiting_response`, `decided`,
    /// `closed`, `withdrawn`).
    status: Option<String>,
    /// Only cases of this kind (`ip_infringement`, `counterfeit`, `prohibited_goods`,
    /// `consumer_complaint`, `other`).
    kind: Option<String>,
    /// Only cases about this object (`<module>.<type>:<id>`).
    target: Option<String>,
    /// Only open cases past their decision deadline.
    #[serde(default)]
    overdue: bool,
    /// Maximum number of items, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
}

/// One case.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CaseInput {
    case_id: Uuid,
}

/// A note for the log of a case.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct NoteInput {
    case_id: Uuid,
    /// 1 to 20000 characters (internal: the reporter and the reported party do not see notes).
    body: String,
}

/// Hide the target of a case.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RestrictInput {
    case_id: Uuid,
    /// Why, kept in the log.
    reason: String,
}

pub(crate) fn all(service: &Arc<ComplianceService>) -> Vec<Tool> {
    let (list, get, note, restrict) =
        (service.clone(), service.clone(), service.clone(), service.clone());
    vec![
        Tool::new(
            "list_cases",
            "List reports of infringements (intellectual property, counterfeit, prohibited goods) \
             and consumer complaints, with their deadlines (`overdue` for those late).",
            move |tenant: TenantContext, principal: Principal, input: CasesInput| {
                let service = list.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None).page()?;
                    let filter = CaseFilter {
                        status: input.status,
                        kind: input.kind,
                        target: input.target,
                        overdue: input.overdue,
                        held: false,
                    };
                    let found = service.list_cases(&tenant, &principal, &filter, &page).await?;
                    Ok::<Page<CaseResponse>, Error>(
                        found.map(|row| describe_case(row, Vec::new(), None)),
                    )
                }
            },
        )
        .requires(CASES_READ)
        .read_only(),
        Tool::new(
            "get_case",
            "Get a report or complaint with its log (notes, answers of the reported party, \
             restrictions, decision).",
            move |tenant: TenantContext, principal: Principal, input: CaseInput| {
                let service = get.clone();
                async move {
                    let detail = service.case_detail(&tenant, &principal, input.case_id).await?;
                    Ok::<CaseResponse, Error>(describe_detail(detail))
                }
            },
        )
        .requires(CASES_READ)
        .read_only(),
        Tool::new(
            "add_case_note",
            "Add an internal note to the log of a case (what was checked, who was contacted).",
            move |tenant: TenantContext, principal: Principal, input: NoteInput| {
                let service = note.clone();
                async move {
                    let row = service
                        .add_case_entry(
                            &tenant,
                            &principal,
                            input.case_id,
                            "note",
                            &input.body,
                            &[],
                        )
                        .await?;
                    Ok::<CaseEntryResponse, Error>(row.into())
                }
            },
        )
        .requires(CASES_HANDLE),
        Tool::new(
            "restrict_case_target",
            "Hide the target of an open case while it is handled (the owning module hides it, \
             e.g. a page answers 451). Deciding the case needs a person through REST.",
            move |tenant: TenantContext, principal: Principal, input: RestrictInput| {
                let service = restrict.clone();
                async move {
                    service
                        .restrict_case_target(&tenant, &principal, input.case_id, &input.reason)
                        .await?;
                    let detail = service.case_after(&tenant, input.case_id).await?;
                    Ok::<CaseResponse, Error>(describe_detail(detail))
                }
            },
        )
        .requires(CASES_HANDLE),
    ]
}
