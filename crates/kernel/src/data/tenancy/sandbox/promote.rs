//! Service sao cấu hình vào sandbox và đẩy sang khách thật (bước 38, D-232,
//! D-20261119-step38-sandbox-copy). REST và tool MCP cùng gọi vào đây.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::copy::{CopyContext, PromoteRequest, Promoted, SandboxOptions, record_promotion};
use super::service::TenantSandboxes;
use super::{SANDBOXES_MANAGE, SANDBOXES_READ};
use crate::access::Principal;
use crate::error::{Error, FieldError};
use crate::event::{Actor, Event};
use crate::tenancy::{TenantContext, TenantId};

/// Số mục tối đa của một lần đẩy.
pub const MAX_PROMOTE_ITEMS: usize = 100;

/// A kind of record that can be promoted from a sandbox to the real tenant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PromotableKind {
    /// `<module>.<kind>`, e.g. `content.page`.
    #[schema(example = "content.page")]
    pub kind: String,
    /// The module that owns it.
    pub module: String,
    pub description: String,
}

/// One record of the sandbox to promote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PromoteItem {
    /// A kind from `GET /api/v1/kernel/sandboxes/promotable`.
    #[schema(example = "content.page")]
    pub kind: String,
    /// Id of the record in the sandbox.
    pub id: Uuid,
}

/// What to promote.
#[derive(Debug, Clone, Deserialize, ToSchema, validator::Validate)]
#[serde(deny_unknown_fields)]
pub struct PromoteInput {
    /// 1 to 100 records of the sandbox.
    pub items: Vec<PromoteItem>,
}

/// Outcome of one promoted record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PromoteOutcome {
    pub kind: String,
    /// Id of the record in the sandbox.
    pub id: Uuid,
    /// `created_draft` (a new draft in the real tenant), `updated_draft` (the draft or pending
    /// change of the record in the real tenant was replaced; what is live is untouched),
    /// `unchanged` (already promoted with the same content) or `conflict` (a name, path or code
    /// is taken in the real tenant; see `conflict`).
    #[schema(example = "created_draft")]
    pub outcome: String,
    /// The record in the real tenant; `null` on `conflict`.
    pub target_id: Option<Uuid>,
    /// Why it was not promoted, on `conflict`.
    pub conflict: Option<PromoteConflict>,
}

/// Why a record could not be promoted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PromoteConflict {
    /// The field that clashes, e.g. `slug`.
    pub field: String,
    pub message: String,
}

/// Result of a promotion: one outcome per item, in the order sent. Nothing is published or enabled
/// in the real tenant: people there publish the drafts themselves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PromoteResult {
    pub items: Vec<PromoteOutcome>,
}

#[derive(Debug, Serialize)]
struct SandboxPromoted {
    sandbox_id: Uuid,
    created_draft: usize,
    updated_draft: usize,
    unchanged: usize,
    conflict: usize,
}

impl Event for SandboxPromoted {
    const NAME: &'static str = "kernel.sandbox_promoted";
}

/// Mục dạng `<kind>:<id>` của tool MCP thành [`PromoteItem`]; sai dạng → 422 gom mọi mục.
pub(crate) fn parse_items(raw: &[String]) -> Result<Vec<PromoteItem>, Error> {
    let mut items = Vec::new();
    let mut problems = Vec::new();
    for (index, text) in raw.iter().enumerate() {
        match text.rsplit_once(':').and_then(|(kind, id)| Some((kind, id.parse::<Uuid>().ok()?))) {
            Some((kind, id)) => items.push(PromoteItem { kind: kind.to_owned(), id }),
            None => problems.push(FieldError::new(
                format!("items[{index}]"),
                "invalid",
                "must be `<kind>:<id>`, e.g. `content.page:0190…`",
            )),
        }
    }
    if problems.is_empty() { Ok(items) } else { Err(Error::validation(problems)) }
}

/// `tenancy.promote_kind_unknown` (422) của mục thứ `index`.
fn kind_unknown(index: usize, kind: &str) -> FieldError {
    FieldError::new(
        format!("items[{index}].kind"),
        "tenancy.promote_kind_unknown",
        "`{kind}` cannot be promoted; see `GET /api/v1/kernel/sandboxes/promotable`",
    )
    .with_param("kind", kind.to_owned())
}

impl TenantSandboxes {
    /// Sao cấu hình của kernel rồi của mọi module từ `source` sang `sandbox`.
    pub(super) async fn copy_configuration(
        &self,
        source: &TenantContext,
        sandbox: &TenantContext,
        options: SandboxOptions,
    ) -> Result<(), Error> {
        let mut copied = crate::tenancy::settings::copy_plain(source, sandbox).await?;
        // Bên làm của job: mọi permission, chỉ ở sandbox (bản sao của chính khách).
        let principal = Principal::with_all_permissions(Uuid::nil());
        let context = CopyContext { source, sandbox, options, principal: &principal };
        for (module, copy) in self.catalog.copies() {
            let count = copy.copy(&context).await.map_err(|error| {
                tracing::warn!(module, sandbox_id = %sandbox.id(), "sandbox copy failed");
                error
            })?;
            copied += count;
        }
        tracing::info!(sandbox_id = %sandbox.id(), copied, "sandbox configuration copied");
        Ok(())
    }

    /// Các loại bản ghi đẩy được sang khách thật.
    pub fn promotable(&self, principal: &Principal) -> Result<Vec<PromotableKind>, Error> {
        principal.require(&SANDBOXES_READ)?;
        Ok(self
            .catalog
            .promotables()
            .into_iter()
            .map(|(module, promotable)| PromotableKind {
                kind: promotable.kind.to_owned(),
                module: module.to_owned(),
                description: promotable.description.to_owned(),
            })
            .collect())
    }

    /// Đẩy các mục của sandbox `id` sang khách gốc (khách hiện tại của người gọi) thành bản nháp.
    /// Mỗi mục chạy riêng: mục lỗi trùng thành `conflict`, mục khác vẫn chạy.
    pub async fn promote(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        input: PromoteInput,
    ) -> Result<PromoteResult, Error> {
        principal.require(&SANDBOXES_MANAGE)?;
        let row = self.owned(tenant, principal, id).await?;
        // Đích là khách gốc của sandbox, không phải khách bất kỳ người gọi đang ở (nhân sự nền tảng).
        let target_id = row.sandbox_of.ok_or_else(|| Error::internal("a sandbox has no source"))?;
        if tenant.id().as_uuid() != target_id {
            return Err(super::sandbox_forbidden("promote outside the real tenant"));
        }
        let known: BTreeMap<&str, _> = self
            .catalog
            .promotables()
            .into_iter()
            .map(|(_, promotable)| (promotable.kind, promotable.apply))
            .collect();
        let mut problems = Vec::new();
        if input.items.is_empty() || input.items.len() > MAX_PROMOTE_ITEMS {
            problems.push(
                FieldError::new("items", "length", "must list 1 to {max} records")
                    .with_param("max", MAX_PROMOTE_ITEMS),
            );
        }
        for (index, item) in input.items.iter().enumerate() {
            if !known.contains_key(item.kind.as_str()) {
                problems.push(kind_unknown(index, &item.kind));
            }
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let sandbox = self.registry.tenant(TenantId::from_uuid(id)).await?;
        let mut items = Vec::with_capacity(input.items.len());
        for item in input.items {
            let Some(apply) = known.get(item.kind.as_str()) else { continue };
            let link = super::copy::link_of(&sandbox, &item.kind, item.id).await?;
            let previous = link.as_ref().and_then(|row| row.promoted_fingerprint.clone());
            let existing = link.as_ref().and_then(|row| row.promoted_id.or(row.source_id));
            let request = PromoteRequest {
                sandbox: &sandbox,
                target: tenant,
                principal,
                id: item.id,
                existing,
            };
            let (promoted, fingerprint) = apply.promote(&request, previous.as_deref()).await?;
            let outcome = match promoted {
                Promoted::CreatedDraft(target) | Promoted::UpdatedDraft(target) => {
                    let name = if matches!(promoted, Promoted::CreatedDraft(_)) {
                        "created_draft"
                    } else {
                        "updated_draft"
                    };
                    record_promotion(&sandbox, &item.kind, item.id, target, fingerprint).await?;
                    PromoteOutcome {
                        kind: item.kind,
                        id: item.id,
                        outcome: name.to_owned(),
                        target_id: Some(target),
                        conflict: None,
                    }
                }
                Promoted::Unchanged(target) => PromoteOutcome {
                    kind: item.kind,
                    id: item.id,
                    outcome: "unchanged".to_owned(),
                    target_id: Some(target),
                    conflict: None,
                },
                Promoted::Conflict { field, message } => PromoteOutcome {
                    kind: item.kind,
                    id: item.id,
                    outcome: "conflict".to_owned(),
                    target_id: None,
                    conflict: Some(PromoteConflict { field, message }),
                },
            };
            items.push(outcome);
        }
        let count = |name: &str| items.iter().filter(|item| item.outcome == name).count();
        let event = SandboxPromoted {
            sandbox_id: id,
            created_draft: count("created_draft"),
            updated_draft: count("updated_draft"),
            unchanged: count("unchanged"),
            conflict: count("conflict"),
        };
        self.record(tenant, Actor::from(principal), id, &event).await?;
        tracing::info!(sandbox_id = %id, items = items.len(), "sandbox promoted");
        Ok(PromoteResult { items })
    }
}

/// `tenancy.promote_record_not_found` (404): sandbox không có bản ghi `id` loại `kind` — module trả
/// trong `PromoteApply::promote`.
pub fn promote_record_not_found(kind: &str, id: Uuid) -> Error {
    Error::not_found(
        "tenancy.promote_record_not_found",
        "The sandbox has no `{kind}` with id `{id}`.",
    )
    .with_param("kind", kind.to_owned())
    .with_param("id", id)
}
