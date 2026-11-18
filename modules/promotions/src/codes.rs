//! Mã của chương trình: coupon (`purpose = redeem`, chương trình `code`) và mã nhận voucher
//! (`purpose = claim`, chương trình `voucher`). Một mã đặt tay, hay một lô mã ngẫu nhiên dài ≥ 8
//! ký tự từ bảng chữ không nhầm (bỏ `0/O`, `1/I/L`); unique index `(tenant_id, code_key)` quyết
//! định trùng, đụng thì sinh lại. Lô > 500 mã chạy bằng job `promotions.generate_codes`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::{code, promotion};
use crate::evaluate::code_key;
use crate::events::{self, Change};
use crate::model::{Purpose, Trigger};
use crate::permissions::CODES_MANAGE;
use crate::programs::not_found;
use crate::service::{PromotionsService, is_unique_violation, now, readable, writable};

/// Bảng chữ của mã sinh: không có `0`, `O`, `1`, `I`, `L`.
const ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";

/// Lô lớn hơn chừng này chạy bằng job.
pub(crate) const INLINE_LIMIT: u32 = 500;

/// Codes to add to a promotion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, validator::Validate)]
#[serde(deny_unknown_fields)]
#[schema(as = PromotionsNewCodes)]
pub(crate) struct NewCodes {
    /// `redeem` (coupon of a `code` promotion) or `claim` (puts a voucher of a `voucher`
    /// promotion in the wallet).
    pub purpose: Purpose,
    /// One code you choose (4 to 64 letters, digits, `-` or `_`). Or give `count`.
    pub code: Option<String>,
    /// Random codes to generate, 1 to 10000. More than 500 are generated in the background.
    pub count: Option<u32>,
    /// Start of every generated code (up to 16 letters or digits).
    pub prefix: Option<String>,
    /// Uses of each code; none = only the promotion's limits.
    pub usage_limit: Option<i64>,
}

/// Kết quả sinh mã.
#[derive(Debug, Clone)]
pub(crate) enum Generated {
    /// Đã tạo ngay.
    Created(Vec<code::Model>),
    /// Lô lớn: đã đưa vào job.
    Queued { count: u32 },
}

/// Job sinh lô mã lớn.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct GenerateCodes {
    pub promotion_id: Uuid,
    pub purpose: Purpose,
    pub count: u32,
    pub prefix: String,
    pub usage_limit: Option<i64>,
}

impl Job for GenerateCodes {
    const KIND: &'static str = "promotions.generate_codes";
}

fn random_code(length: usize, prefix: &str) -> Result<String, Error> {
    let token = autoxide_kernel::secret::random_token(length * 2)?;
    let mut out = prefix.to_owned();
    for byte in token.bytes() {
        if out.len() >= prefix.len() + length {
            break;
        }
        out.push(char::from(ALPHABET[usize::from(byte) % ALPHABET.len()]));
    }
    Ok(out)
}

fn check(input: &NewCodes) -> Result<(), Error> {
    let mut problems = Vec::new();
    match (&input.code, input.count) {
        (Some(code), None) => {
            let valid = (4..=64).contains(&code.chars().count())
                && code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if !valid {
                problems.push(FieldError::new(
                    "code",
                    "invalid",
                    "4 to 64 letters, digits, `-` or `_`",
                ));
            }
        }
        (None, Some(count)) if (1..=10_000).contains(&count) => {}
        (None, Some(_)) => problems.push(FieldError::new("count", "range", "must be 1 to 10000")),
        _ => problems.push(FieldError::new("code", "required", "give either `code` or `count`")),
    }
    if input.prefix.as_ref().is_some_and(|prefix| {
        prefix.len() > 16 || !prefix.bytes().all(|b| b.is_ascii_alphanumeric())
    }) {
        problems.push(FieldError::new("prefix", "invalid", "up to 16 letters or digits"));
    }
    if input.usage_limit.is_some_and(|limit| limit < 1) {
        problems.push(FieldError::new("usage_limit", "range", "must be at least 1"));
    }
    if problems.is_empty() { Ok(()) } else { Err(Error::validation(problems)) }
}

/// Chèn một mã; `Ok(None)` khi trùng.
async fn insert_code<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    promotion_id: Uuid,
    text: &str,
    purpose: Purpose,
    usage_limit: Option<i64>,
) -> Result<Option<code::Model>, Error> {
    let inserted = code::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant.id().as_uuid()),
        promotion_id: Set(promotion_id),
        code: Set(text.to_owned()),
        code_key: Set(code_key(text)),
        purpose: Set(purpose.as_str().to_owned()),
        usage_limit: Set(usage_limit),
        usage_count: Set(0),
        created_at: Set(now()),
    }
    .insert(connection)
    .await;
    match inserted {
        Ok(row) => Ok(Some(row)),
        Err(error) if is_unique_violation(&error) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

impl PromotionsService {
    /// Thêm mã cho chương trình `promotion_id`.
    pub(crate) async fn generate_codes(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        promotion_id: Uuid,
        input: &NewCodes,
    ) -> Result<Generated, Error> {
        check(input)?;
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        let program = promotion::Entity::find_by_id(promotion_id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| not_found(promotion_id))?;
        writable(principal, &transaction, &CODES_MANAGE, program.org_id, || {
            not_found(promotion_id)
        })
        .await?;
        let wanted = match input.purpose {
            Purpose::Redeem => Trigger::Code,
            Purpose::Claim => Trigger::Voucher,
        };
        if program.trigger != wanted.as_str()
            || matches!(program.status.as_str(), "ended" | "cancelled")
        {
            return Err(Error::validation(vec![FieldError::new(
                "purpose",
                "invalid",
                "`redeem` codes belong to `code` promotions, `claim` codes to `voucher` promotions \
                 that have not ended",
            )]));
        }
        if let Some(text) = &input.code {
            let Some(row) = insert_code(
                &transaction,
                tenant,
                promotion_id,
                text,
                input.purpose,
                input.usage_limit,
            )
            .await?
            else {
                return Err(Error::conflict(
                    "promotions.code_taken",
                    "Code `{code}` already exists.",
                )
                .with_param("code", text));
            };
            self.codes_event(&transaction, tenant, actor, promotion_id, 1).await?;
            transaction.commit().await?;
            return Ok(Generated::Created(vec![row]));
        }
        let count = input.count.unwrap_or(1);
        let job = GenerateCodes {
            promotion_id,
            purpose: input.purpose,
            count,
            prefix: input.prefix.clone().unwrap_or_default(),
            usage_limit: input.usage_limit,
        };
        if count > INLINE_LIMIT {
            self.jobs.enqueue(&transaction, tenant, &job, EnqueueOptions::default()).await?;
            transaction.commit().await?;
            return Ok(Generated::Queued { count });
        }
        let rows = self.insert_batch(&transaction, tenant, &job).await?;
        self.codes_event(&transaction, tenant, actor, promotion_id, rows.len()).await?;
        transaction.commit().await?;
        Ok(Generated::Created(rows))
    }

    async fn insert_batch<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        job: &GenerateCodes,
    ) -> Result<Vec<code::Model>, Error> {
        let mut rows = Vec::with_capacity(usize::try_from(job.count).unwrap_or(0));
        let mut attempts = 0;
        while rows.len() < usize::try_from(job.count).unwrap_or(0) {
            attempts += 1;
            if attempts > job.count.saturating_mul(3) + 10 {
                return Err(Error::internal("could not generate unique promotion codes"));
            }
            let text = random_code(self.config.code_length, &job.prefix)?;
            if let Some(row) = insert_code(
                connection,
                tenant,
                job.promotion_id,
                &text,
                job.purpose,
                job.usage_limit,
            )
            .await?
            {
                rows.push(row);
            }
        }
        Ok(rows)
    }

    async fn codes_event<C: ConnectionTrait>(
        &self,
        transaction: &C,
        tenant: &TenantContext,
        actor: Actor,
        promotion_id: Uuid,
        count: usize,
    ) -> Result<(), Error> {
        self.outbox
            .record(
                transaction,
                tenant,
                actor,
                crate::events::promotion_subject(promotion_id),
                &events::CodesGenerated(Change::after(json!({"id": promotion_id, "count": count}))),
            )
            .await
    }

    /// Một trang mã của chương trình.
    pub(crate) async fn codes(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        promotion_id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<code::Model>, Error> {
        let read = tenant.database().read();
        let program = promotion::Entity::find_by_id(promotion_id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| not_found(promotion_id))?;
        readable(principal, read, &CODES_MANAGE, program.org_id, || not_found(promotion_id))
            .await?;
        let select =
            code::Entity::find().filter(code::Column::PromotionId.eq(promotion_id)).scoped(tenant);
        fetch_page(select, code::Column::Id, |row| row.id, page, read).await
    }

    /// Mọi mã của chương trình dạng CSV (chống CSV injection bằng `autoxide_kernel::csv`).
    pub(crate) async fn export_codes(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        promotion_id: Uuid,
    ) -> Result<String, Error> {
        let read = tenant.database().read();
        let program = promotion::Entity::find_by_id(promotion_id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| not_found(promotion_id))?;
        readable(principal, read, &CODES_MANAGE, program.org_id, || not_found(promotion_id))
            .await?;
        let rows = code::Entity::find()
            .filter(code::Column::PromotionId.eq(promotion_id))
            .scoped(tenant)
            .order_by_asc(code::Column::Id)
            .all(read)
            .await?;
        let header: Vec<String> = ["code", "purpose", "usage_limit", "usage_count", "created_at"]
            .map(str::to_owned)
            .to_vec();
        let lines: Vec<Vec<String>> = rows
            .into_iter()
            .map(|row| {
                vec![
                    row.code,
                    row.purpose,
                    row.usage_limit.map(|limit| limit.to_string()).unwrap_or_default(),
                    row.usage_count.to_string(),
                    row.created_at.to_rfc3339(),
                ]
            })
            .collect();
        Ok(autoxide_kernel::csv::write(&header, &lines))
    }
}

/// Bộ chạy job `promotions.generate_codes`.
pub(crate) struct GenerateCodesJob(pub Arc<PromotionsService>);

#[async_trait::async_trait]
impl JobHandler for GenerateCodesJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let payload: GenerateCodes =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        let tenant =
            job.tenant.as_ref().ok_or_else(|| Error::internal("code job without a tenant"))?;
        let transaction = tenant.database().begin_write().await?;
        let rows = self.0.insert_batch(&transaction, tenant, &payload).await?;
        self.0
            .codes_event(&transaction, tenant, Actor::system(), payload.promotion_id, rows.len())
            .await?;
        transaction.commit().await?;
        Ok(())
    }
}
