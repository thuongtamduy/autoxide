//! Tool MCP của module `promotions`. Mỗi tool: kiểu đầu vào kiểm như thân JSON của REST,
//! permission như endpoint REST tương ứng, thân là lời gọi service — không có logic riêng (luật
//! kiến trúc 8).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::mcp::{PageInput, Tool};
use autoxide_kernel::pagination::Page;
use autoxide_kernel::tenancy::TenantContext;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::api::{Cart, Evaluation, Voucher, VoucherStatus};
use crate::codes::{Generated, NewCodes};
use crate::evaluate::Options;
use crate::http::dto::{CodeDto, CodesAdded, PromotionListItem, PromotionResponse};
use crate::model::{Kind, Purpose, Status};
use crate::permissions::{CODES_MANAGE, PUBLISH, READ, VOUCHERS_GRANT, WRITE};
use crate::programs::transitions::Transition;
use crate::programs::{PromotionFilter, PromotionInput};
use crate::service::PromotionsService;

/// Promotions to list.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = PromotionsListInput)]
pub(crate) struct ListInput {
    #[schema(inline)]
    pub status: Option<Status>,
    #[schema(inline)]
    pub kind: Option<Kind>,
    pub org_id: Option<Uuid>,
    /// Words in the title.
    pub q: Option<String>,
    /// Maximum number of promotions, 1 to 200. Defaults to 50.
    pub limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    pub cursor: Option<String>,
}

/// A promotion to act on.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = PromotionsPromotionId)]
pub(crate) struct PromotionId {
    /// Id of the promotion.
    pub id: Uuid,
}

/// Codes to add to a promotion.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = PromotionsGenerateCodesInput)]
pub(crate) struct GenerateCodesInput {
    pub id: Uuid,
    #[schema(inline)]
    pub purpose: Purpose,
    pub code: Option<String>,
    pub count: Option<u32>,
    pub prefix: Option<String>,
    pub usage_limit: Option<i64>,
}

/// A cart to compute.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = PromotionsEvaluateInput)]
pub(crate) struct EvaluateInput {
    /// The cart, as the body of `POST /api/v1/promotions/evaluate` (`PromotionsCart`).
    #[schema(value_type = Object)]
    pub cart: serde_json::Value,
}

/// A promotion to create.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = PromotionsCreateInput)]
pub(crate) struct CreateInput {
    /// The promotion, as the body of `POST /api/v1/promotions` (`PromotionsPromotionInput`).
    #[schema(value_type = Object)]
    pub promotion: serde_json::Value,
}

/// Đọc một object JSON thành kiểu của REST; sai thì lỗi trường như thân REST.
fn parse<T: serde::de::DeserializeOwned>(
    field: &str,
    value: serde_json::Value,
) -> Result<T, Error> {
    serde_json::from_value(value).map_err(|error| {
        Error::validation(vec![
            autoxide_kernel::error::FieldError::new(field, "invalid", "{reason}")
                .with_param("reason", error.to_string()),
        ])
    })
}

/// A voucher to give by hand.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = PromotionsGrantVoucherInput)]
pub(crate) struct GrantInput {
    /// Id of the `voucher` promotion.
    pub promotion_id: Uuid,
    pub user_id: Uuid,
    /// Why, 3 to 500 characters (kept in the audit log).
    pub reason: String,
    /// Your reference; the same one again returns the same voucher.
    pub reference: Option<String>,
}

/// Whose wallet to read.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = PromotionsVouchersOfUserInput)]
pub(crate) struct VouchersOfUserInput {
    pub user_id: Uuid,
    #[schema(inline)]
    pub status: Option<VoucherStatus>,
}

/// A wallet.
#[derive(Debug, Serialize)]
pub(crate) struct Wallet {
    vouchers: Vec<Voucher>,
}

pub(crate) fn all(service: &Arc<PromotionsService>) -> Vec<Tool> {
    // Mỗi tool giữ một bản `Arc` của service (clone `Arc` chỉ tăng bộ đếm tham chiếu).
    let s = || service.clone();
    let (list, get, create, activate, end, codes) = (s(), s(), s(), s(), s(), s());
    let (evaluate, grant, wallet) = (s(), s(), s());
    vec![
        Tool::new(
            "list_promotions",
            "List the promotions in your scope, newest first; filter by status, kind, unit or \
             words in the title; follow `pagination.next_cursor` for more.",
            move |tenant: TenantContext, principal: Principal, input: ListInput| {
                let service = list.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None).page()?;
                    let filter = PromotionFilter {
                        status: input.status,
                        kind: input.kind,
                        org_id: input.org_id,
                        q: input.q,
                    };
                    let found =
                        service.list_promotions(&tenant, &principal, &filter, &page).await?;
                    Ok::<Page<PromotionListItem>, Error>(found.map(|(row, title)| {
                        PromotionListItem {
                            id: row.id,
                            org_id: row.org_id,
                            title,
                            kind: Kind::parse(&row.kind).unwrap_or(Kind::PercentOff),
                            status: Status::parse(&row.status).unwrap_or(Status::Draft),
                            starts_at: row.starts_at,
                            ends_at: row.ends_at,
                            usage_count: row.usage_count,
                        }
                    }))
                }
            },
        )
        .requires(READ)
        .read_only(),
        Tool::new(
            "get_promotion",
            "Get a promotion with its rule, texts, targets, flash-sale prices and counters.",
            move |tenant: TenantContext, principal: Principal, input: PromotionId| {
                let service = get.clone();
                async move {
                    let view = service.promotion(&tenant, &principal, input.id).await?;
                    Ok::<_, Error>(PromotionResponse::from(&view))
                }
            },
        )
        .requires(READ)
        .read_only(),
        Tool::new(
            "create_promotion",
            "Create a promotion as a draft (rule, window with a required end, limits, title and \
             terms). Turn it on with `activate_promotion`.",
            move |tenant: TenantContext, principal: Principal, input: CreateInput| {
                let service = create.clone();
                async move {
                    let rule: PromotionInput = parse("promotion", input.promotion)?;
                    let view = service.create_promotion(&tenant, &principal, rule).await?;
                    Ok::<_, Error>(PromotionResponse::from(&view))
                }
            },
        )
        .requires(WRITE),
        Tool::new(
            "activate_promotion",
            "Turn a draft promotion on after the legal checks (discount limit and days in force, \
             notices to authorities, price floors). Failed checks come back as errors.",
            move |tenant: TenantContext, principal: Principal, input: PromotionId| {
                let service = activate.clone();
                async move {
                    let activated = service.activate(&tenant, &principal, input.id).await?;
                    Ok::<_, Error>(PromotionResponse::from(&activated.view))
                }
            },
        )
        .requires(PUBLISH),
        Tool::new(
            "end_promotion",
            "End a promotion now; usage already held for orders stays valid.",
            move |tenant: TenantContext, principal: Principal, input: PromotionId| {
                let service = end.clone();
                async move {
                    let view =
                        service.transition(&tenant, &principal, input.id, Transition::End).await?;
                    Ok::<_, Error>(PromotionResponse::from(&view))
                }
            },
        )
        .requires(PUBLISH),
        Tool::new(
            "generate_codes",
            "Add one chosen code or a batch of random codes to a promotion: `redeem` coupons for \
             `code` promotions, `claim` codes for `voucher` promotions.",
            move |tenant: TenantContext, principal: Principal, input: GenerateCodesInput| {
                let service = codes.clone();
                async move {
                    let body = NewCodes {
                        purpose: input.purpose,
                        code: input.code,
                        count: input.count,
                        prefix: input.prefix,
                        usage_limit: input.usage_limit,
                    };
                    Ok::<_, Error>(
                        match service.generate_codes(&tenant, &principal, input.id, &body).await? {
                            Generated::Created(rows) => CodesAdded {
                                codes: rows.iter().map(CodeDto::from).collect(),
                                queued: 0,
                            },
                            Generated::Queued { count } => {
                                CodesAdded { codes: Vec::new(), queued: count }
                            }
                        },
                    )
                }
            },
        )
        .requires(CODES_MANAGE),
        Tool::new(
            "evaluate_cart",
            "Compute the promotions of a cart without holding anything: discounts per line with \
             reasons, gifts, codes and vouchers that do not apply, and the fingerprint.",
            move |tenant: TenantContext, _principal: Principal, input: EvaluateInput| {
                let service = evaluate.clone();
                async move {
                    let cart: Cart = parse("cart", input.cart)?;
                    let outcome =
                        service.evaluate_cart(&tenant, &cart, &Options::default()).await?;
                    Ok::<Evaluation, Error>(outcome.evaluation)
                }
            },
        )
        .requires(READ)
        .read_only(),
        Tool::new(
            "grant_voucher",
            "Put a voucher of a `voucher` promotion into a user's wallet, with a reason. The same \
             `reference` again returns the voucher already given.",
            move |tenant: TenantContext, principal: Principal, input: GrantInput| {
                let service = grant.clone();
                async move {
                    let granted = service
                        .grant_by_hand(
                            &tenant,
                            &principal,
                            input.promotion_id,
                            &input.reason,
                            input.user_id,
                            input.reference,
                        )
                        .await?;
                    Ok::<Voucher, Error>(granted.into_inner())
                }
            },
        )
        .requires(VOUCHERS_GRANT),
        Tool::new(
            "list_vouchers_of_user",
            "List the vouchers in a user's wallet, soonest to expire first (for support staff).",
            move |tenant: TenantContext, _principal: Principal, input: VouchersOfUserInput| {
                let service = wallet.clone();
                async move {
                    let vouchers =
                        service.vouchers_of(&tenant, input.user_id, input.status).await?;
                    Ok::<_, Error>(Wallet { vouchers })
                }
            },
        )
        .requires(VOUCHERS_GRANT)
        .read_only(),
    ]
}
