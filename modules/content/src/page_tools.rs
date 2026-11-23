//! Tool MCP của trang dựng từ khối và AI tạo trang (bước 67): cùng service, cùng kiểm quyền với REST
//! (luật kiến trúc 8) — agent bên ngoài làm được đúng những gì người có cùng quyền làm qua REST.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::tenancy::TenantContext;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::ai::campaign::CampaignRequest;
use crate::http::campaign_drafts::{CampaignDraftsResponse, response};
use crate::permissions::{CONTENT_WRITE, CONTENT_WRITE_OWN};
use crate::service::ContentService;

/// A request for campaign drafts made from one promotion.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CampaignDraftsInput {
    /// The promotion (module `promotions`) to draft the campaign for.
    promotion_id: Uuid,
    /// Website (organization node) of the landing page and the banner. Omit for your own node.
    org_id: Option<Uuid>,
    /// Language of the drafts; omit for the default language.
    locale: Option<String>,
    /// What to draft: any of `page`, `banner`, `newsletter`.
    channels: Vec<String>,
    /// Extra wishes in plain words, at most 2000 characters.
    instruction: Option<String>,
    /// `banner` channel: key of the banner placement.
    banner_placement: Option<String>,
    /// `newsletter` channel: the list the campaign is written for.
    newsletter_list_id: Option<Uuid>,
}

/// A request to read.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CampaignDraftsId {
    /// Id of the request (from `create_campaign_drafts`).
    id: Uuid,
}

pub(crate) fn all(service: &Arc<ContentService>) -> Vec<Tool> {
    let (create, get) = (service.clone(), service.clone());
    vec![
        Tool::new(
            "create_campaign_drafts",
            "Draft a campaign from a promotion: a landing page (`page`), a text banner (`banner`, \
             needs `banner_placement`) and a newsletter campaign (`newsletter`, needs \
             `newsletter_list_id`). Everything is a draft in its own module — nothing is published or \
             sent; a person reviews each one. Runs in the background: read the outcome with \
             `get_campaign_drafts`. Counts once toward the plan's monthly AI page builds.",
            move |tenant: TenantContext, principal: Principal, input: CampaignDraftsInput| {
                let service = create.clone();
                async move {
                    let request = CampaignRequest {
                        promotion_id: input.promotion_id,
                        org_id: input.org_id,
                        locale: input.locale,
                        channels: input.channels,
                        instruction: input.instruction,
                        banner_placement: input.banner_placement,
                        newsletter_list_id: input.newsletter_list_id,
                    };
                    let row = service.request_campaign_drafts(&tenant, &principal, request).await?;
                    Ok::<CampaignDraftsResponse, Error>(response(row))
                }
            },
        )
        .requires(CONTENT_WRITE)
        .or_requires(CONTENT_WRITE_OWN),
        Tool::new(
            "get_campaign_drafts",
            "Get a campaign draft request: its status and, once finished, the draft made for each \
             channel (`kind` and `id`) or why that channel failed (`error_code`).",
            move |tenant: TenantContext, principal: Principal, input: CampaignDraftsId| {
                let service = get.clone();
                async move {
                    let row = service.campaign_drafts(&tenant, &principal, input.id).await?;
                    Ok::<CampaignDraftsResponse, Error>(response(row))
                }
            },
        )
        .requires(CONTENT_WRITE)
        .or_requires(CONTENT_WRITE_OWN)
        .read_only(),
    ]
}
