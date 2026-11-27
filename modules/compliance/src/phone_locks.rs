//! Mở khoá mã đăng nhập SMS của một số (PT-7, D-20261126-otp-nist): đường lấy lại tài khoản cho người chỉ đăng nhập
//! bằng số (người mua tạo tài khoản bằng số, không email, không mật khẩu). Quản trị viên của khách làm, cần
//! `compliance.phone_locks.manage`; mỗi lần ghi event `compliance.phone_unlocked` (audit). REST, tool MCP và lệnh
//! `autoxide task compliance.unlock_phone` dùng chung một hàm service (luật 8).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::sms::PhoneNumber;
use autoxide_kernel::task::{Task, TaskArgs, TaskArgument, TaskRun};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use validator::Validate;

use crate::events;
use crate::permissions::PHONE_LOCKS_MANAGE;
use crate::service::ComplianceService;

/// A phone number whose sign-in codes should be unlocked.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UnlockPhone {
    /// The number, e.g. `0912345678` or `+84912345678`.
    #[schema(example = "0912345678")]
    pub phone: String,
}

/// The result of an unlock.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct PhoneUnlocked {
    /// `true` when the number was locked and is now unlocked; `false` when it was not locked.
    pub unlocked: bool,
}

impl ComplianceService {
    /// Mở khoá mã đăng nhập SMS của số `raw` (xoá bộ đếm lần sai liên tiếp). Cần `compliance.phone_locks.manage`.
    pub(crate) async fn unlock_phone(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        raw: &str,
    ) -> Result<bool, Error> {
        principal.require(&PHONE_LOCKS_MANAGE)?;
        self.unlock_phone_as(tenant, Actor::from(principal), raw).await
    }

    /// Như [`unlock_phone`](Self::unlock_phone), với người làm `actor` đã được kiểm (lệnh CLI chạy bằng quyền người
    /// vận hành máy chủ).
    pub(crate) async fn unlock_phone_as(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        raw: &str,
    ) -> Result<bool, Error> {
        let phone = PhoneNumber::parse(raw, "84").ok_or_else(|| {
            Error::validation(vec![FieldError::new("phone", "invalid", "is not a phone number")])
        })?;
        let phone_key = self.secret.phone_key(tenant.id(), &phone).await?;
        // Qua lớp `db` của kernel (handle của khách), không qua database thô.
        let transaction = tenant.db().begin_write().await?;
        let unlocked = crate::phone_lock::clear(&transaction, tenant, &phone_key).await?;
        if unlocked {
            // Không ghi số vào event: chỉ khoá của số (HMAC theo khách).
            let event = events::PhoneUnlocked(json!({"phone_key": phone_key}));
            self.outbox.record(&transaction, tenant, actor, None, &event).await?;
        }
        transaction.commit().await?;
        if unlocked {
            tracing::info!(phone = %phone.masked(), "sign-in codes of a phone number unlocked");
        }
        Ok(unlocked)
    }
}

pub(crate) fn routes(service: Arc<ComplianceService>) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(unlock_phone)).with_state(service)
}

/// Unlock sign-in codes for a phone number.
///
/// Requires `compliance.phone_locks.manage`. After 100 wrong sign-in codes in a row for a number
/// its sign-in codes are locked (`compliance.otp_locked`); this clears the count so the owner can
/// sign in with a code again — the way back in for an account that signs in only with its number.
/// Check who you are talking to first. The audit log records who unlocked it.
#[utoipa::path(
    post,
    path = "/phone-locks/unlock",
    extensions(("x-permissions" = json!(["compliance.phone_locks.manage"]))),
    operation_id = "unlock_compliance_phone",
    security(("bearer" = [])),
    request_body = UnlockPhone,
    responses(
        (status = 200, description = "Done; `unlocked` says whether the number was locked.", body = Data<PhoneUnlocked>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `compliance.phone_locks.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "`phone` is not a phone number; see `errors`.", body = Problem),
    )
)]
async fn unlock_phone(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    autoxide_kernel::http::ValidJson(input): autoxide_kernel::http::ValidJson<UnlockPhone>,
) -> Result<Data<PhoneUnlocked>, Error> {
    let unlocked = service.unlock_phone(&tenant, &principal, &input.phone).await?;
    Ok(Data { data: PhoneUnlocked { unlocked } })
}

/// Tool MCP: cùng service, cùng permission với REST.
pub(crate) fn tool(service: &Arc<ComplianceService>) -> Tool {
    let service = service.clone();
    Tool::new(
        "unlock_phone",
        "Unlock sign-in codes for a phone number locked after 100 wrong codes in a row, so its owner \
         can sign in with a code again. Check who asks first; the audit log records who unlocked it.",
        move |tenant: TenantContext, principal: Principal, input: UnlockPhone| {
            let service = service.clone();
            async move {
                let unlocked = service.unlock_phone(&tenant, &principal, &input.phone).await?;
                Ok::<_, Error>(PhoneUnlocked { unlocked })
            }
        },
    )
    .requires(PHONE_LOCKS_MANAGE)
}

/// Lệnh `autoxide task compliance.unlock_phone phone=<số> [tenant=<slug>]`.
pub(crate) fn task(service: Arc<ComplianceService>) -> Task {
    Task::new(
        "unlock_phone",
        "Unlock sign-in codes for a phone number locked after 100 wrong codes in a row. Writes an \
         audit event; changes nothing when the number is not locked.",
        Arc::new(UnlockTask(service)),
    )
    .argument(TaskArgument::required("phone", "The phone number, e.g. 0912345678."))
    .argument(TaskArgument::optional("tenant", "Slug of the tenant; default: the default tenant."))
}

struct UnlockTask(Arc<ComplianceService>);

#[async_trait::async_trait]
impl TaskRun for UnlockTask {
    async fn run(&self, args: &TaskArgs) -> Result<String, Error> {
        let tenant = match args.get("tenant") {
            Some(slug) => self.0.databases.tenant_by_slug(slug).await?,
            None => self.0.databases.default_tenant(),
        };
        let phone = args.get("phone").unwrap_or_default();
        let unlocked = self.0.unlock_phone_as(&tenant, Actor::system(), phone).await?;
        Ok(if unlocked {
            format!("Sign-in codes of the number are unlocked in tenant `{}`.", tenant.slug())
        } else {
            format!("The number was not locked in tenant `{}`; nothing changed.", tenant.slug())
        })
    }
}
