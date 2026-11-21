//! Cấp phát khách mới theo gói (bước 35 mục 5, D-338) — phần nhân sự nền tảng làm, chưa cần KYC:
//! tạo khách → gán gói → dữ liệu mẫu của module gói bật → mời quản trị viên (role `admin` của
//! khách mới) → xong.
//!
//! - Mỗi lượt là một dòng `plans_provisioning` (mặt điều khiển) và chạy bằng job `plans.provision`
//!   (REST / MCP) hay ngay trong lệnh (`autoxide tenant create … --plan`, qua `api::Provisioning`).
//! - Từng bước ghi lại khi xong (`step`, lịch sử trong `steps`): hỏng giữa chừng thì `failed` kèm lỗi;
//!   `retry` chạy tiếp từ bước đã ghi, không làm lại bước đã xong. Tạo khách hỏng sau khi kernel đã
//!   ghi dòng khách (gieo dữ liệu ban đầu lỗi): lần chạy lại nhận lại khách `provisioning` cùng slug
//!   tạo sau lượt này và cấp phát tiếp.
//! - Vị trí dữ liệu theo gói: `shared` → placement `main` (hay placement nêu); `dedicated` → phải nêu
//!   một placement `dedicated` đã thêm bằng `autoxide placement add` (URL có mật khẩu, không qua
//!   API — đặc tả 33).
//! - Gói `requires_kyc`: nhân sự không cấp phát thẳng được (`plans.kyc_required`) — khách tự đăng ký
//!   và qua định danh ở `signup.rs` (D-349), rồi đến đây bằng chính đường này.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest, fetch_page};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::{NewTenant, TenantId};
use axum::extract::{Path, Query, State};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::{Validate, ValidateEmail};

use crate::api::{ProvisionRequest, ProvisioningStatus};
use crate::entity::{plan, price, subscription};
use crate::http::operator;
use crate::permissions::{PROVISION, SUBSCRIPTIONS_READ};
use crate::service::{AssignInput, PlansService, now, plan_not_found, price_not_found};

pub(crate) mod entity {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "plans_provisioning")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        /// Khách đã tạo (sau bước `tenant_created`).
        pub tenant_id: Option<Uuid>,
        pub slug: String,
        pub tenant_name: String,
        pub parent_tenant_id: Option<Uuid>,
        pub plan_id: Uuid,
        pub price_id: Option<Uuid>,
        /// Tên placement; `None`: `main`.
        pub placement: Option<String>,
        pub admin_email: String,
        pub admin_name: Option<String>,
        pub sample_data: bool,
        pub trial: bool,
        /// Bước đã xong gần nhất: `created`, `tenant_created`, `subscribed`, `seeded`, `invited`,
        /// `done`.
        pub step: String,
        /// `pending`, `running`, `failed`, `done`.
        pub status: String,
        pub error_code: Option<String>,
        pub error: Option<String>,
        /// JSON: `[{"step": …, "at": …}]`.
        pub steps: String,
        pub requested_by: Option<Uuid>,
        pub created_at: Timestamp,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) const PROVISION_JOB: &str = "plans.provision";

const STEPS: [&str; 6] = ["created", "tenant_created", "subscribed", "seeded", "invited", "done"];

#[derive(Debug, Serialize, Deserialize)]
struct ProvisionJob {
    id: Uuid,
}

impl Job for ProvisionJob {
    const KIND: &'static str = PROVISION_JOB;
}

/// Chạy một lượt cấp phát trong job.
pub(crate) struct ProvisionRunner(pub Arc<PlansService>);

#[async_trait::async_trait]
impl JobHandler for ProvisionRunner {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let payload: ProvisionJob =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        // Lỗi của từng bước đã ghi vào lượt cấp phát (`failed`): job không thử lại — nhân sự xem
        // lỗi, sửa rồi `retry`.
        let _ = self.0.run_provisioning(payload.id).await?;
        Ok(())
    }
}

pub(crate) fn provisioning_not_found(id: Uuid) -> Error {
    Error::not_found("plans.provisioning_not_found", "There is no provisioning `{id}`.")
        .with_param("id", id)
}

fn kyc_required(code: &str) -> Error {
    Error::new(
        ErrorKind::Conflict,
        "plans.kyc_required",
        "Plan `{code}` requires identity checks before a tenant is created: the tenant signs up by \
         itself (`POST /api/v1/plans/signups`) and passes them there.",
    )
    .with_param("code", code)
}

fn not_retryable(status: &str) -> Error {
    Error::new(
        ErrorKind::Conflict,
        "plans.provisioning_not_retryable",
        "This provisioning is `{status}`: only a failed one can be run again.",
    )
    .with_param("status", status)
}

/// Lỗi của một bước, ghi vào lượt cấp phát.
fn step_failed(step: &str, error: &Error) -> Error {
    Error::new(
        ErrorKind::Conflict,
        "plans.provisioning_failed",
        "Provisioning stopped at step `{step}`: {reason}. Fix it, then run it again.",
    )
    .with_param("step", step)
    .with_param("reason", error.message())
}

pub(crate) fn is_slug(slug: &str) -> bool {
    (1..=63).contains(&slug.len())
        && !slug.starts_with('-')
        && !slug.ends_with('-')
        && slug.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn next_step(step: &str) -> &'static str {
    let index = STEPS.iter().position(|known| *known == step).unwrap_or(0);
    STEPS.get(index + 1).copied().unwrap_or("done")
}

impl PlansService {
    /// Kiểm và ghi một lượt cấp phát mới (`pending`); `enqueue`: xếp job chạy nền.
    pub(crate) async fn start_provisioning(
        &self,
        actor: Actor,
        request: ProvisionRequest,
        enqueue: bool,
    ) -> Result<entity::Model, Error> {
        self.start_provisioning_for(actor, request, enqueue, None).await
    }

    /// Như [`start_provisioning`](Self::start_provisioning); `signup`: lượt này cho một đăng ký của
    /// khách tự đăng ký (D-349) — định danh (nếu gói cần) đã xong ở đó, và đăng ký chuyển sang
    /// `provisioning` trong cùng transaction (đăng ký không còn chờ → `plans.signup_not_waiting`).
    pub(crate) async fn start_provisioning_for(
        &self,
        actor: Actor,
        request: ProvisionRequest,
        enqueue: bool,
        signup: Option<crate::signup::SignupLink>,
    ) -> Result<entity::Model, Error> {
        let mut problems = Vec::new();
        let slug = request.slug.trim().to_owned();
        if !is_slug(&slug) {
            problems.push(FieldError::new(
                "slug",
                "invalid",
                "must be 1 to 63 lower-case letters, digits and `-`, not starting or ending with `-`",
            ));
        }
        let name = request.name.trim().to_owned();
        if name.is_empty() || name.chars().count() > 200 {
            problems.push(FieldError::new("name", "length", "must be 1 to 200 characters long"));
        }
        let email = request.admin_email.trim().to_owned();
        if !ValidateEmail::validate_email(&email) || email.len() > 254 {
            problems.push(FieldError::new("admin_email", "email", "must be an email address"));
        }
        let admin_name =
            request.admin_name.map(|name| name.trim().to_owned()).filter(|n| !n.is_empty());
        if admin_name.as_ref().is_some_and(|name| name.chars().count() > 200) {
            problems.push(FieldError::new(
                "admin_name",
                "length",
                "must be 1 to 200 characters long",
            ));
        }
        let read = self.databases.control().read();
        let plan = plan::Entity::find_by_id(request.plan_id)
            .one(read)
            .await?
            .ok_or_else(|| plan_not_found(request.plan_id))?;
        if plan.status != "active" || plan.kind != "base" {
            return Err(crate::service::plan_not_active(&plan.code));
        }
        if plan.requires_kyc && signup.is_none() {
            return Err(kyc_required(&plan.code));
        }
        if let Some(price_id) = request.price_id {
            let found = price::Entity::find_by_id(price_id).one(read).await?;
            if !found
                .is_some_and(|row| row.plan_id == plan.id && row.active && row.kind == "recurring")
            {
                return Err(price_not_found(price_id));
            }
        }
        let placement =
            request.placement.map(|name| name.trim().to_owned()).filter(|n| !n.is_empty());
        if plan.placement == "dedicated" && placement.is_none() {
            problems.push(FieldError::new(
                "placement",
                "required",
                "the plan keeps each tenant in its own database: name a `dedicated` placement \
                 added with `autoxide placement add`",
            ));
        }
        if let Some(wanted) = &placement {
            let tenants = self.tenant_admin()?;
            let known = tenants.placements().await?;
            match known.iter().find(|found| &found.name == wanted) {
                None => problems.push(FieldError::new(
                    "placement",
                    "unknown",
                    "there is no placement with this name",
                )),
                Some(found) if found.kind != plan.placement => problems.push(
                    FieldError::new("placement", "invalid", "must be a placement of kind `{kind}`")
                        .with_param("kind", plan.placement.clone()),
                ),
                Some(_) => {}
            }
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        // Một slug một lượt cấp phát, và không trùng khách đã có (kể cả khách đã xoá: slug không
        // dùng lại). Hai lượt cùng slug chạy cùng lúc thì không bao giờ cùng nhận một khách.
        let taken = || autoxide_kernel::tenancy::errors::tenant_slug_taken(&request.slug);
        match self.tenant_admin()?.get(request.slug.trim()).await {
            Ok(_) => return Err(taken()),
            Err(error) if error.code() == "tenant_not_found" => {}
            Err(error) => return Err(error),
        }
        let at = now();
        let row = entity::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(None),
            slug: Set(slug),
            tenant_name: Set(name),
            parent_tenant_id: Set(request.parent_id),
            plan_id: Set(plan.id),
            price_id: Set(request.price_id),
            placement: Set(placement),
            admin_email: Set(email),
            admin_name: Set(admin_name),
            sample_data: Set(request.sample_data.unwrap_or(plan.sample_data)),
            trial: Set(request.trial),
            step: Set("created".to_owned()),
            status: Set("pending".to_owned()),
            error_code: Set(None),
            error: Set(None),
            steps: Set(json!([{"step": "created", "at": at}]).to_string()),
            requested_by: Set(actor.user_id),
            created_at: Set(at),
            updated_at: Set(at),
        };
        let transaction = self.databases.control().begin_write().await?;
        let row = row.insert(&transaction).await.map_err(|error| {
            if matches!(error.sql_err(), Some(sea_orm::SqlErr::UniqueConstraintViolation(_))) {
                taken()
            } else {
                Error::from(error)
            }
        })?;
        if let Some(link) = &signup
            && !crate::signup::claim_for_provisioning(&transaction, link, row.id).await?
        {
            transaction.rollback().await?;
            return Err(Error::new(
                ErrorKind::Conflict,
                "plans.signup_not_waiting",
                "This sign-up already went on; read it again.",
            ));
        }
        if enqueue {
            self.jobs
                .enqueue_database(
                    &transaction,
                    &ProvisionJob { id: row.id },
                    EnqueueOptions::default().max_attempts(1),
                )
                .await?;
        }
        transaction.commit().await?;
        tracing::info!(provisioning_id = %row.id, slug = %row.slug, "tenant provisioning requested");
        Ok(row)
    }

    /// Chạy lại lượt cấp phát `failed` từ bước đã ghi.
    pub(crate) async fn retry_provisioning(&self, id: Uuid) -> Result<entity::Model, Error> {
        let row = self.provisioning(id).await?;
        if row.status != "failed" {
            return Err(not_retryable(&row.status));
        }
        let transaction = self.databases.control().begin_write().await?;
        let mut active = row.into_active_model();
        active.status = Set("pending".to_owned());
        active.error_code = Set(None);
        active.error = Set(None);
        active.updated_at = Set(now());
        let row = active.update(&transaction).await?;
        self.jobs
            .enqueue_database(
                &transaction,
                &ProvisionJob { id },
                EnqueueOptions::default().max_attempts(1),
            )
            .await?;
        transaction.commit().await?;
        Ok(row)
    }

    pub(crate) async fn provisioning(&self, id: Uuid) -> Result<entity::Model, Error> {
        entity::Entity::find_by_id(id)
            .one(self.databases.control().read())
            .await?
            .ok_or_else(|| provisioning_not_found(id))
    }

    pub(crate) async fn list_provisionings(
        &self,
        status: Option<String>,
        page: &PageRequest,
    ) -> Result<Page<entity::Model>, Error> {
        let mut select = entity::Entity::find();
        if let Some(status) = status {
            select = select.filter(entity::Column::Status.eq(status));
        }
        fetch_page(select, entity::Column::Id, |row| row.id, page, self.databases.control().read())
            .await
    }

    pub(crate) fn tenant_admin(&self) -> Result<Arc<autoxide_kernel::tenancy::TenantAdmin>, Error> {
        self.tenants
            .clone()
            .ok_or_else(|| Error::internal("the kernel did not provide the tenant service"))
    }

    /// Chạy lượt cấp phát `id` từ bước đã ghi tới xong. Bước hỏng: ghi `failed` kèm lỗi, trả lỗi.
    pub(crate) async fn run_provisioning(&self, id: Uuid) -> Result<entity::Model, Error> {
        let mut row = self.provisioning(id).await?;
        if row.status == "done" {
            return Ok(row);
        }
        row = self
            .update_provisioning(row, |active| active.status = Set("running".to_owned()))
            .await?;
        while row.step != "done" {
            let step = next_step(&row.step);
            let outcome = self.provision_step(&row, step).await;
            match outcome {
                Ok(tenant_id) => {
                    let at = now();
                    let mut steps: Vec<serde_json::Value> =
                        serde_json::from_str(&row.steps).unwrap_or_default();
                    steps.push(json!({"step": step, "at": at}));
                    row = self
                        .update_provisioning(row, |active| {
                            active.step = Set(step.to_owned());
                            active.steps = Set(serde_json::Value::from(steps).to_string());
                            if let Some(tenant_id) = tenant_id {
                                active.tenant_id = Set(Some(tenant_id));
                            }
                        })
                        .await?;
                }
                Err(error) => {
                    tracing::warn!(provisioning_id = %id, step, error = %error, "tenant provisioning failed");
                    let failed = step_failed(step, &error);
                    let message: String = error.message().chars().take(2000).collect();
                    self.update_provisioning(row, |active| {
                        active.status = Set("failed".to_owned());
                        active.error_code = Set(Some(error.code().to_owned()));
                        active.error = Set(Some(message));
                    })
                    .await?;
                    return Err(failed);
                }
            }
        }
        let row =
            self.update_provisioning(row, |active| active.status = Set("done".to_owned())).await?;
        tracing::info!(provisioning_id = %id, tenant_id = ?row.tenant_id, "tenant provisioned");
        // Lượt của một khách tự đăng ký: đăng ký xong.
        self.complete_signup(&row).await?;
        Ok(row)
    }

    async fn update_provisioning(
        &self,
        row: entity::Model,
        edit: impl FnOnce(&mut entity::ActiveModel),
    ) -> Result<entity::Model, Error> {
        let mut active = row.into_active_model();
        edit(&mut active);
        active.updated_at = Set(now());
        Ok(active.update(self.databases.control().write()).await?)
    }

    /// Làm bước `step` của lượt `row`; trả ID khách khi bước tạo ra khách.
    async fn provision_step(&self, row: &entity::Model, step: &str) -> Result<Option<Uuid>, Error> {
        let tenants = self.tenant_admin()?;
        let tenant = || {
            row.tenant_id
                .map(TenantId::from_uuid)
                .ok_or_else(|| Error::internal("the provisioning has no tenant yet"))
        };
        match step {
            "tenant_created" => {
                let input = NewTenant {
                    slug: row.slug.clone(),
                    name: row.tenant_name.clone(),
                    parent_id: row.parent_tenant_id,
                    placement: row.placement.clone(),
                };
                match tenants.create(Actor::system(), input).await {
                    Ok(created) => Ok(Some(created.id)),
                    // Lần trước kernel đã ghi khách nhưng gieo dữ liệu lỗi: khách `provisioning`
                    // cùng slug, tạo sau lượt này → nhận lại và cấp phát tiếp.
                    Err(error) if error.code() == "tenant_slug_taken" => {
                        let found = tenants.get(&row.slug).await?;
                        if found.status == "provisioning" && found.created_at >= row.created_at {
                            let done = tenants.provision(Actor::system(), &row.slug).await?;
                            Ok(Some(done.id))
                        } else {
                            Err(error)
                        }
                    }
                    Err(error) => Err(error),
                }
            }
            "subscribed" => {
                let tenant = tenant()?;
                let current = subscription::Entity::find()
                    .filter(subscription::Column::TenantId.eq(tenant.as_uuid()))
                    .one(self.databases.control().read())
                    .await?;
                // Đã gán đúng gói (lần chạy trước) thì thôi.
                if current.is_none_or(|sub| sub.plan_id != row.plan_id) {
                    let input = AssignInput {
                        plan_id: row.plan_id,
                        price_id: row.price_id,
                        trial: row.trial,
                        addons: None,
                    };
                    self.assign(Actor::system(), tenant, input).await?;
                }
                Ok(None)
            }
            "seeded" => {
                if row.sample_data {
                    let tenant = tenant()?;
                    let mut modules = Vec::new();
                    for module in tenants.sample_modules() {
                        if self.entitlements.module_enabled(tenant, module).await? {
                            modules.push(module);
                        }
                    }
                    tenants.seed_samples(tenant, &modules).await?;
                }
                Ok(None)
            }
            "invited" => {
                let tenant = tenant()?;
                let invitations = self.invitations.clone().ok_or_else(|| {
                    Error::unavailable(
                        "plans.invitations_unavailable",
                        "Inviting the administrator needs the `auth` module.",
                    )
                })?;
                let context = self.databases.tenant(tenant).await?;
                let mut invitation = autoxide_auth::api::NewInvitation::new(
                    row.admin_email.clone(),
                    vec!["admin".to_owned()],
                );
                if let Some(name) = &row.admin_name {
                    invitation = invitation.named(name.clone());
                }
                invitations.invite(&context, invitation).await?;
                Ok(None)
            }
            _ => Ok(None),
        }
    }
}

/// `api::Provisioning` cho lệnh CLI: ghi lượt rồi chạy ngay (không qua job).
pub(crate) struct Provisioner(pub Arc<PlansService>);

#[async_trait::async_trait]
impl crate::api::Provisioning for Provisioner {
    async fn provision(&self, request: ProvisionRequest) -> Result<ProvisioningStatus, Error> {
        let row = self.0.start_provisioning(Actor::system(), request, false).await?;
        let row = self.0.run_provisioning(row.id).await?;
        Ok(status_of(&row))
    }
}

fn status_of(row: &entity::Model) -> ProvisioningStatus {
    ProvisioningStatus::new(row.id, row.tenant_id, row.step.clone(), row.status.clone())
}

// ---- REST ----------------------------------------------------------------------------------------

pub(crate) fn routes() -> OpenApiRouter<Arc<PlansService>> {
    OpenApiRouter::new()
        .routes(routes!(list_provisionings, create_provisioning))
        .routes(routes!(get_provisioning))
        .routes(routes!(retry_provisioning))
}

/// One step a provisioning went through.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct ProvisioningStep {
    /// `created`, `tenant_created`, `subscribed`, `seeded`, `invited` or `done`.
    step: String,
    at: Timestamp,
}

/// A request to create a tenant on a plan, and how far it went.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct ProvisioningResponse {
    id: Uuid,
    /// The new tenant, once created.
    tenant_id: Option<Uuid>,
    slug: String,
    tenant_name: String,
    parent_tenant_id: Option<Uuid>,
    plan_id: Uuid,
    price_id: Option<Uuid>,
    /// Name of the placement; `null`: `main`.
    placement: Option<String>,
    admin_email: String,
    admin_name: Option<String>,
    sample_data: bool,
    trial: bool,
    /// The last step done: `created`, `tenant_created`, `subscribed`, `seeded`, `invited`, `done`.
    #[schema(example = "subscribed")]
    step: String,
    /// `pending`, `running`, `failed` (see `error`; fix it, then `POST …/retry`) or `done`.
    #[schema(example = "done")]
    status: String,
    /// Code of the error that stopped it, when `failed`.
    error_code: Option<String>,
    error: Option<String>,
    /// Every step done, oldest first.
    steps: Vec<ProvisioningStep>,
    requested_by: Option<Uuid>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl From<entity::Model> for ProvisioningResponse {
    fn from(row: entity::Model) -> Self {
        ProvisioningResponse {
            id: row.id,
            tenant_id: row.tenant_id,
            slug: row.slug,
            tenant_name: row.tenant_name,
            parent_tenant_id: row.parent_tenant_id,
            plan_id: row.plan_id,
            price_id: row.price_id,
            placement: row.placement,
            admin_email: row.admin_email,
            admin_name: row.admin_name,
            sample_data: row.sample_data,
            trial: row.trial,
            step: row.step,
            status: row.status,
            error_code: row.error_code,
            error: row.error,
            steps: serde_json::from_str(&row.steps).unwrap_or_default(),
            requested_by: row.requested_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

const fn yes() -> bool {
    true
}

/// Create a tenant on a plan.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProvisioningRequest {
    /// Short name of the new tenant: 1 to 63 lower-case letters, digits and `-`.
    #[schema(example = "acme")]
    slug: String,
    /// Name of the new tenant, 1 to 200 characters.
    #[schema(example = "Acme Ltd")]
    name: String,
    /// An active base plan.
    plan_id: Uuid,
    /// An active recurring price of that plan.
    price_id: Option<Uuid>,
    /// Parent tenant, for chains.
    parent_id: Option<Uuid>,
    /// Name of the placement for its data, of the plan's kind; default `main` for a `shared` plan.
    /// Required for a `dedicated` plan.
    placement: Option<String>,
    /// Who administers the new tenant: invited by email with the role `admin`.
    #[schema(example = "owner@example.com")]
    admin_email: String,
    /// Their name, up to 200 characters.
    admin_name: Option<String>,
    /// Add the sample data of the modules the plan turns on; default: the plan's `sample_data`.
    sample_data: Option<bool>,
    /// Start with the plan's free trial; default `true`.
    #[serde(default = "yes")]
    trial: bool,
}

impl From<ProvisioningRequest> for ProvisionRequest {
    fn from(input: ProvisioningRequest) -> Self {
        let mut request =
            ProvisionRequest::new(input.slug, input.name, input.plan_id, input.admin_email);
        request.price_id = input.price_id;
        request.parent_id = input.parent_id;
        request.placement = input.placement;
        request.admin_name = input.admin_name;
        request.sample_data = input.sample_data;
        request.trial = input.trial;
        request
    }
}

/// Filters of the provisioning list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ProvisioningsQuery {
    /// Only provisionings in this state: `pending`, `running`, `failed` or `done`.
    status: Option<String>,
}

/// Create a tenant on a plan.
///
/// Platform staff with `plans.provision`. Runs in the background: creates the tenant, puts it on the
/// plan, adds the sample data of the modules the plan turns on (when asked) and invites the
/// administrator by email with the role `admin`. Follow it with `GET /provisionings/{id}`
/// (`plans.subscriptions.read`). A plan that requires identity checks cannot be provisioned here
/// (`plans.kyc_required`): the tenant signs up by itself (`POST /signups`) and passes them there.
#[utoipa::path(
    post,
    path = "/provisionings",
    extensions(("x-permissions" = json!(["plans.provision"]))),
    operation_id = "create_provisioning",
    security(("bearer" = [])),
    request_body = ProvisioningRequest,
    responses(
        (status = 201, description = "Accepted; it runs in the background.", body = Data<ProvisioningResponse>, headers(("Location" = String, description = "Address of the provisioning."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.provision` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such plan (`plans.plan_not_found`) or price (`plans.price_not_found`).", body = Problem),
        (status = 409, description = "The plan is not sold (`plans.plan_not_active`) or requires identity checks (`plans.kyc_required`).", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn create_provisioning(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    ValidJson(input): ValidJson<ProvisioningRequest>,
) -> Result<Created<ProvisioningResponse>, Error> {
    operator(&principal, &PROVISION)?;
    let row = service.start_provisioning(Actor::from(&principal), input.into(), true).await?;
    Ok(Created::new(format!("/plans/provisionings/{}", row.id), row.into()))
}

/// List tenant provisionings.
///
/// Platform staff with `plans.subscriptions.read`.
#[utoipa::path(
    get,
    path = "/provisionings",
    extensions(("x-permissions" = json!(["plans.subscriptions.read"]))),
    operation_id = "list_provisionings",
    security(("bearer" = [])),
    params(ProvisioningsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of provisionings, newest first.", body = Page<ProvisioningResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.subscriptions.read` (`permission_denied`).", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_provisionings(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Query(filter): Query<ProvisioningsQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<ProvisioningResponse>, Error> {
    operator(&principal, &SUBSCRIPTIONS_READ)?;
    if filter
        .status
        .as_deref()
        .is_some_and(|status| !["pending", "running", "failed", "done"].contains(&status))
    {
        return Err(Error::validation(vec![FieldError::new(
            "status",
            "invalid",
            "must be one of pending, running, failed, done",
        )]));
    }
    let page = service.list_provisionings(filter.status, &PageRequest::try_from(page)?).await?;
    Ok(page.map(ProvisioningResponse::from))
}

/// Get a tenant provisioning.
///
/// Platform staff with `plans.subscriptions.read`.
#[utoipa::path(
    get,
    path = "/provisionings/{id}",
    extensions(("x-permissions" = json!(["plans.subscriptions.read"]))),
    operation_id = "get_provisioning",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the provisioning.")),
    responses(
        (status = 200, description = "The provisioning.", body = Data<ProvisioningResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.subscriptions.read` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such provisioning; code `plans.provisioning_not_found`.", body = Problem),
    )
)]
async fn get_provisioning(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ProvisioningResponse>, Error> {
    operator(&principal, &SUBSCRIPTIONS_READ)?;
    Ok(Data { data: service.provisioning(id).await?.into() })
}

/// Run a failed tenant provisioning again.
///
/// Platform staff with `plans.provision`. It goes on from the last step done.
#[utoipa::path(
    post,
    path = "/provisionings/{id}/retry",
    extensions(("x-permissions" = json!(["plans.provision"]))),
    operation_id = "retry_provisioning",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the provisioning.")),
    responses(
        (status = 200, description = "Accepted again; it runs in the background.", body = Data<ProvisioningResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.provision` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such provisioning; code `plans.provisioning_not_found`.", body = Problem),
        (status = 409, description = "It did not fail; code `plans.provisioning_not_retryable`.", body = Problem),
    )
)]
async fn retry_provisioning(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ProvisioningResponse>, Error> {
    operator(&principal, &PROVISION)?;
    Ok(Data { data: service.retry_provisioning(id).await?.into() })
}
