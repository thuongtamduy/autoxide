//! Khách tự đăng ký (bước 35 mục 5, D-349): người lạ chọn một gói `self_signup` → xác nhận email →
//! (gói `requires_kyc`) định danh qua `compliance` → cấp phát bằng ĐÚNG đường của nhân sự vận hành
//! (`start_provisioning`, job `plans.provision`) với email đăng ký làm quản trị viên khách mới.
//!
//! - `POST /signups` theo hợp đồng chung cho dữ liệu người lạ (như đăng ký bản tin, D-154): kiểm dữ
//!   liệu (gom lỗi một lần), `SpamGuard::check` ngoài transaction, rồi một transaction ngắn GIỐNG NHAU
//!   cho mọi địa chỉ: xếp job [`SIGNUP_JOB`] và chèn nonce của token form. Câu trả lời luôn 202.
//! - Job tạo (hay giữ, khi cùng email + slug còn chờ) dòng `plans_signup`, phát token mới (chỉ lưu
//!   bản băm) và gửi mail qua `dyn Mailer` — không qua `dyn Notifier`: token không được nằm trong
//!   nhật ký gửi của `notifications`.
//! - Token trong mail vừa xác nhận email (`POST /signups/confirm`, gọi lại vô hại) vừa là chìa khoá xem
//!   trạng thái (`GET /signups/{id}?token=`).
//! - Chấp nhận điều khoản ghi qua `dyn AcceptanceGate` (khách nền tảng, `Subject::Email`) lúc xác
//!   nhận — lúc email đã được chứng minh. Định danh: `dyn KycGate::start` với
//!   `Subject::Reference("plans.signup:<id>")`; nghe `compliance.kyc_verified` / `kyc_rejected`.
//! - Slug chỉ thật sự được giữ khi lượt cấp phát được ghi (unique): hai người chờ cùng slug thì người
//!   đến bước cấp phát sau bị từ chối (`tenant_slug_taken`) — không lộ email nào đang chờ slug nào.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;
use std::time::Duration;

use autoxide_compliance::api::Subject;
use autoxide_kernel::antispam::{Claim, Limits, Submission as Check, Verdict};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Delivered, EventHandler};
use autoxide_kernel::http::ClientInfo;
use autoxide_kernel::i18n::Language;
use autoxide_kernel::identity::PartyType;
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use autoxide_kernel::mail::MailTemplate;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::secret::{random_token, sha256};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, TenantContext};
use autoxide_kernel::throttle::Throttle;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::api::ProvisionRequest;
use crate::entity::{plan, price};
use crate::events;
use crate::provisioning::{self, is_slug};
use crate::service::{PlansService, now};

pub(crate) mod entity {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "plans_signup")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub email: String,
        pub admin_name: Option<String>,
        pub tenant_name: String,
        pub slug: String,
        pub plan_id: Uuid,
        pub price_id: Option<Uuid>,
        pub party_type: String,
        pub locale: Option<String>,
        /// `pending_email`, `awaiting_kyc`, `provisioning`, `completed`, `rejected`, `expired`.
        pub status: String,
        pub token_hash: String,
        /// JSON `{loại văn bản: id phiên bản}` đã chấp nhận lúc xác nhận email.
        pub terms: Option<String>,
        pub kyc_profile_id: Option<Uuid>,
        pub provisioning_id: Option<Uuid>,
        pub tenant_id: Option<Uuid>,
        pub reason: Option<String>,
        pub confirmed_at: Option<Timestamp>,
        pub expires_at: Timestamp,
        pub created_at: Timestamp,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// Job gửi link xác nhận (tạo hay giữ dòng đăng ký).
pub(crate) const SIGNUP_JOB: &str = "plans.send_signup_link";
/// Lịch hằng giờ: đăng ký quá hạn → `expired`; xoá dòng đã kết thúc lâu; dọn bộ đếm, nonce.
pub(crate) const EXPIRE_SIGNUPS: &str = "plans.expire_signups";
/// Phạm vi của token form (`GET /api/v1/antispam/token?scope=plans:signup`).
pub(crate) const SCOPE: &str = "plans:signup";
const MODULE: &str = "plans";
/// Link xác nhận sống chừng này giờ.
const CONFIRM_HOURS: i64 = 24;
/// Chờ định danh tối đa chừng này ngày.
const KYC_DAYS: i64 = 30;
/// Dòng `expired` / `rejected` giữ chừng này ngày rồi xoá (dữ liệu cá nhân không giữ quá mục đích).
const KEEP_DAYS: i64 = 30;
const PER_IP_PER_HOUR: u32 = 10;
const PER_EMAIL_PER_HOUR: u32 = 3;
const HOUR: Duration = Duration::from_secs(3600);
/// Văn bản pháp lý người đăng ký chấp nhận (chỉ những loại khách nền tảng đã đăng).
const TERMS: [&str; 2] = ["terms_of_use", "privacy_policy"];
/// Trạng thái còn chờ (chưa cấp phát).
const WAITING: [&str; 2] = ["pending_email", "awaiting_kyc"];
pub(crate) const STATUSES: [&str; 6] =
    ["pending_email", "awaiting_kyc", "provisioning", "completed", "rejected", "expired"];

const SIGNUP_LINK: MailTemplate = MailTemplate::new(
    "plans.mail.signup_link",
    "Confirm your sign-up for {tenant}",
    "Someone, we hope you, asked to create {tenant} on the plan {plan} with this email address. \
     Confirm it within {hours} hours by opening this link:\n\n{link}\n\nIf it was not you, ignore \
     this mail: nothing is created.",
);

const SIGNUP_CODE: MailTemplate = MailTemplate::new(
    "plans.mail.signup_code",
    "Confirm your sign-up for {tenant}",
    "Someone, we hope you, asked to create {tenant} on the plan {plan} with this email address. \
     Confirm it within {hours} hours with this token (`POST /api/v1/plans/signups/confirm`):\n\n\
     {token}\n\nIf it was not you, ignore this mail: nothing is created.",
);

pub(crate) fn signup_disabled() -> Error {
    Error::not_found("plans.signup_disabled", "This server does not take sign-ups of new tenants.")
}

fn token_invalid(reason: &'static str) -> Error {
    Error::bad_request(
        "plans.signup_token_invalid",
        "The confirmation link is wrong, used up or expired. Sign up again.",
    )
    .with_reason(reason)
}

fn signup_not_found() -> Error {
    Error::not_found(
        "plans.signup_not_found",
        "There is no sign-up with this id and token; use the token from the confirmation mail.",
    )
}

/// Một yêu cầu đăng ký, như client gửi.
#[derive(Debug, Clone, Default)]
pub(crate) struct SignupRequest {
    pub email: String,
    pub admin_name: Option<String>,
    pub tenant_name: String,
    pub slug: String,
    pub plan_code: String,
    pub price_id: Option<Uuid>,
    pub party_type: Option<String>,
    pub locale: Option<String>,
    pub accept_terms: bool,
    pub form_token: Option<String>,
    pub trap: Option<String>,
    pub challenge: Option<String>,
}

/// Job [`SIGNUP_JOB`]: yêu cầu đã kiểm, chờ lưu và gửi mail. Mang email — job queue xoá job đã xong
/// sau thời hạn của nó.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SignupJob {
    email: String,
    admin_name: Option<String>,
    tenant_name: String,
    slug: String,
    plan_id: Uuid,
    price_id: Option<Uuid>,
    party_type: String,
    locale: Option<String>,
}

impl Job for SignupJob {
    const KIND: &'static str = SIGNUP_JOB;
}

pub(crate) struct SignupLinkJob(pub Arc<PlansService>);

#[async_trait::async_trait]
impl JobHandler for SignupLinkJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let job: SignupJob =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        self.0.apply_signup(&job).await
    }
}

pub(crate) struct ExpireSignupsJob(pub Arc<PlansService>);

#[async_trait::async_trait]
impl JobHandler for ExpireSignupsJob {
    async fn run(&self, _job: &JobContext) -> Result<(), Error> {
        self.0.expire_signups(now()).await.map(|_| ())
    }
}

/// Nghe `compliance.kyc_verified` / `compliance.kyc_rejected` của hồ sơ `plans.signup:<id>`.
pub(crate) struct KycListener(pub Arc<PlansService>);

#[async_trait::async_trait]
impl EventHandler for KycListener {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        // Chỉ hồ sơ mở ở khách nền tảng (do chính module này mở): khách khác không giả được.
        if event.tenant.id() != DEFAULT_TENANT || event.payload["subject_kind"] != "reference" {
            return Ok(());
        }
        let Some(id) = event.payload["subject_id"]
            .as_str()
            .and_then(|subject| subject.strip_prefix("plans.signup:"))
            .and_then(|id| id.parse::<Uuid>().ok())
        else {
            return Ok(());
        };
        let Some(row) =
            entity::Entity::find_by_id(id).one(self.0.databases.control().write()).await?
        else {
            return Ok(());
        };
        // Đã đi tiếp / đã kết thúc (event đến lại): không làm gì.
        if row.status != "awaiting_kyc" {
            return Ok(());
        }
        if event.name == autoxide_compliance::api::KYC_VERIFIED {
            self.0.continue_signup(row).await.map(|_| ())
        } else {
            self.0.reject_signup(row, "kyc_rejected").await.map(|_| ())
        }
    }
}

/// Lượt định danh vừa mở cho người đăng ký.
#[derive(Debug, Clone)]
pub(crate) struct StartedKyc {
    pub kind: String,
    pub url: Option<String>,
    pub token: Option<String>,
    pub expires_at: Option<Timestamp>,
}

impl PlansService {
    /// Đăng ký công khai có mở ở request này không: bật bằng cấu hình, có bên gửi mail, và request
    /// tới khách nền tảng (đăng ký thuộc nhà vận hành, không thuộc khách nào).
    pub(crate) fn require_signup(&self, tenant: &TenantContext) -> Result<(), Error> {
        if self.signup.enabled && self.mailer.is_some() && tenant.id() == DEFAULT_TENANT {
            Ok(())
        } else {
            Err(signup_disabled())
        }
    }

    /// Nhận một yêu cầu đăng ký. `Ok(())` như nhau cho mọi trường hợp không phải lỗi dữ liệu / token
    /// form (kể cả bị bỏ vì spam).
    pub(crate) async fn submit_signup(
        &self,
        tenant: &TenantContext,
        client: &ClientInfo,
        input: SignupRequest,
    ) -> Result<(), Error> {
        self.require_signup(tenant)?;
        let mut problems = Vec::new();
        let email = autoxide_kernel::email::checked(&input.email, "email", &mut problems);
        let tenant_name = input.tenant_name.trim().to_owned();
        if tenant_name.is_empty() || tenant_name.chars().count() > 200 {
            problems.push(FieldError::new(
                "tenant_name",
                "length",
                "must be 1 to 200 characters long",
            ));
        }
        let admin_name =
            input.admin_name.map(|name| name.trim().to_owned()).filter(|name| !name.is_empty());
        if admin_name.as_ref().is_some_and(|name| name.chars().count() > 200) {
            problems.push(FieldError::new(
                "admin_name",
                "length",
                "must be 1 to 200 characters long",
            ));
        }
        let slug = input.slug.trim().to_owned();
        let slug_ok = is_slug(&slug);
        if !slug_ok {
            problems.push(FieldError::new(
                "slug",
                "invalid",
                "must be 1 to 63 lower-case letters, digits and `-`, not starting or ending with `-`",
            ));
        }
        let party = input.party_type.as_deref().unwrap_or("organization");
        if PartyType::parse(party).is_none() {
            problems.push(FieldError::new(
                "party_type",
                "invalid",
                "must be one of individual, household_business, organization",
            ));
        }
        let locale = match input.locale.as_deref().map(str::trim).filter(|l| !l.is_empty()) {
            None => None,
            Some(raw) => match Language::from_tag(raw) {
                Some(language) => Some(language.tag().to_owned()),
                None => {
                    problems.push(FieldError::new("locale", "invalid", "must be one of en, vi"));
                    None
                }
            },
        };
        if !input.accept_terms {
            problems.push(FieldError::new(
                "accept_terms",
                "required",
                "must be `true`: the terms of use and privacy policy must be accepted",
            ));
        }
        let read = self.databases.control().read();
        let plan = plan::Entity::find()
            .filter(plan::Column::Code.eq(input.plan_code.trim()))
            .one(read)
            .await?
            .filter(|plan| plan.status == "active" && plan.kind == "base" && plan.self_signup);
        match &plan {
            None => problems.push(FieldError::new(
                "plan_code",
                "invalid",
                "no plan open to sign-up has this code",
            )),
            Some(plan) if plan.requires_kyc && self.kyc.is_none() => {
                problems.push(FieldError::new(
                    "plan_code",
                    "unavailable",
                    "this plan needs identity checks, which this server cannot run",
                ));
            }
            Some(plan) => {
                if let Some(price_id) = input.price_id {
                    let found = price::Entity::find_by_id(price_id).one(read).await?;
                    if !found.is_some_and(|row| {
                        row.plan_id == plan.id && row.active && row.kind == "recurring"
                    }) {
                        problems.push(FieldError::new(
                            "price_id",
                            "invalid",
                            "is not an active price of this plan",
                        ));
                    }
                }
            }
        }
        if slug_ok && self.slug_taken(&slug).await? {
            problems.push(FieldError::new("slug", "taken", "is already taken"));
        }
        let (Some(email), Some(plan)) = (email, plan) else {
            return Err(Error::validation(problems));
        };
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }

        // Pha một: chống spam, ngoài transaction.
        let throttle = |limit: u32| Throttle::new(MODULE, limit, HOUR);
        let limits = Limits::default()
            .ip(Some(throttle(PER_IP_PER_HOUR)?))
            .email(Some(throttle(PER_EMAIL_PER_HOUR)?));
        let checked = self
            .guard
            .check(
                tenant,
                &Check::new(SCOPE, client)
                    .honeypot(input.trap.as_deref())
                    .form_token(input.form_token.as_deref())
                    .challenge(input.challenge.as_deref())
                    .email(Some(&email)),
                &limits,
            )
            .await?;
        match &checked.verdict {
            Verdict::Discard { reason } => {
                tracing::info!(reason, "tenant sign-up dropped");
                return Ok(());
            }
            // Bị giữ lại (gửi quá nhanh…): vẫn phải xác nhận email như mọi đăng ký — bot không làm được.
            Verdict::Quarantine { reasons } => {
                tracing::info!(reasons = %reasons.join(","), "tenant sign-up held for email check");
            }
            Verdict::Accept => {}
        }
        let job = SignupJob {
            email,
            admin_name,
            tenant_name,
            slug,
            plan_id: plan.id,
            price_id: input.price_id,
            party_type: party.to_owned(),
            locale,
        };
        // Pha hai: cùng một việc cho mọi địa chỉ (D-154). Nonce đã có (gửi lại cùng token form) →
        // rollback, không xếp job thứ hai.
        let transaction = tenant.database().begin_write().await?;
        self.jobs.enqueue_database(&transaction, &job, EnqueueOptions::default()).await?;
        if let Claim::Replay { .. } = checked.claim(&transaction, tenant, MODULE, None).await? {
            transaction.rollback().await?;
            return Ok(());
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Slug đã thuộc một khách (kể cả khách đã xoá) hay một lượt cấp phát.
    async fn slug_taken(&self, slug: &str) -> Result<bool, Error> {
        match self.tenant_admin()?.get(slug).await {
            Ok(_) => return Ok(true),
            Err(error) if error.code() == "tenant_not_found" => {}
            Err(error) => return Err(error),
        }
        let found = provisioning::entity::Entity::find()
            .filter(provisioning::entity::Column::Slug.eq(slug))
            .one(self.databases.control().read())
            .await?;
        Ok(found.is_some())
    }

    /// Job [`SIGNUP_JOB`]: tạo dòng đăng ký (hay giữ dòng cùng email + slug còn chờ xác nhận), phát
    /// token mới, gửi mail. Chạy lại thì như người đó gửi lại yêu cầu: không thêm dòng, link cũ hết
    /// dùng, mail mới đi.
    async fn apply_signup(&self, job: &SignupJob) -> Result<(), Error> {
        let mailer = self.mailer.clone().ok_or_else(signup_disabled)?;
        let Some(plan) =
            plan::Entity::find_by_id(job.plan_id).one(self.databases.control().read()).await?
        else {
            return Ok(());
        };
        let token = random_token(32)?;
        let at = now();
        let expires_at = at + SignedDuration::from_hours(CONFIRM_HOURS);
        let transaction = self.databases.control().begin_write().await?;
        let existing = entity::Entity::find()
            .filter(entity::Column::Email.eq(job.email.as_str()))
            .filter(entity::Column::Status.eq("pending_email"))
            .filter(entity::Column::Slug.eq(job.slug.as_str()))
            .filter(entity::Column::ExpiresAt.gt(at))
            .one(&transaction)
            .await?;
        let row = match existing {
            Some(row) => {
                let mut active = row.into_active_model();
                active.token_hash = Set(sha256(&token));
                active.tenant_name = Set(job.tenant_name.clone());
                active.admin_name = Set(job.admin_name.clone());
                active.plan_id = Set(job.plan_id);
                active.price_id = Set(job.price_id);
                active.party_type = Set(job.party_type.clone());
                active.locale = Set(job.locale.clone());
                active.expires_at = Set(expires_at);
                active.updated_at = Set(at);
                active.update(&transaction).await?
            }
            None => {
                let row = entity::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    email: Set(job.email.clone()),
                    admin_name: Set(job.admin_name.clone()),
                    tenant_name: Set(job.tenant_name.clone()),
                    slug: Set(job.slug.clone()),
                    plan_id: Set(job.plan_id),
                    price_id: Set(job.price_id),
                    party_type: Set(job.party_type.clone()),
                    locale: Set(job.locale.clone()),
                    status: Set("pending_email".to_owned()),
                    token_hash: Set(sha256(&token)),
                    terms: Set(None),
                    kyc_profile_id: Set(None),
                    provisioning_id: Set(None),
                    tenant_id: Set(None),
                    reason: Set(None),
                    confirmed_at: Set(None),
                    expires_at: Set(expires_at),
                    created_at: Set(at),
                    updated_at: Set(at),
                }
                .insert(&transaction)
                .await?;
                let event =
                    events::SignupCreated(json!({"signup_id": row.id, "plan_id": row.plan_id}));
                self.record(&transaction, Actor::public(), subject(row.id), &event).await?;
                row
            }
        };
        transaction.commit().await?;
        // Gửi mail SAU commit (bên gửi mail ghi hàng đợi bằng kết nối riêng).
        let language = row.locale.as_deref().and_then(Language::from_tag).unwrap_or(Language::En);
        let hours = CONFIRM_HOURS.to_string();
        let mail = match &self.signup.url {
            Some(url) => {
                let link = url.replace("{token}", &token);
                SIGNUP_LINK.mail(
                    &row.email,
                    &self.translations,
                    language,
                    &[
                        ("tenant", row.tenant_name.as_str()),
                        ("plan", plan.name.as_str()),
                        ("hours", hours.as_str()),
                        ("link", link.as_str()),
                    ],
                )
            }
            None => SIGNUP_CODE.mail(
                &row.email,
                &self.translations,
                language,
                &[
                    ("tenant", row.tenant_name.as_str()),
                    ("plan", plan.name.as_str()),
                    ("hours", hours.as_str()),
                    ("token", token.as_str()),
                ],
            ),
        };
        mailer.send(&self.databases.default_tenant(), mail).await?;
        tracing::info!(signup_id = %row.id, "tenant sign-up link sent");
        Ok(())
    }

    /// Đăng ký có token `token` (đúng bản băm), hay `None`.
    async fn signup_by_token(&self, token: &str) -> Result<Option<entity::Model>, Error> {
        Ok(entity::Entity::find()
            .filter(entity::Column::TokenHash.eq(sha256(token.trim())))
            .one(self.databases.control().write())
            .await?)
    }

    /// Xác nhận email bằng token trong mail; gọi lại vô hại. Ghi chấp nhận điều khoản; gói cần định
    /// danh → mở lượt định danh (gọi lại khi đang chờ: mở lượt mới); không → cấp phát ngay.
    pub(crate) async fn confirm_signup(
        &self,
        tenant: &TenantContext,
        token: &str,
    ) -> Result<(entity::Model, Option<StartedKyc>), Error> {
        self.require_signup(tenant)?;
        let Some(row) = self.signup_by_token(token).await? else {
            return Err(token_invalid("unknown"));
        };
        let at = now();
        match row.status.as_str() {
            "pending_email" if row.expires_at <= at => Err(token_invalid("expired")),
            "expired" => Err(token_invalid("expired")),
            "pending_email" => {
                let terms = self.accept_terms(&row.email).await?;
                let plan = plan::Entity::find_by_id(row.plan_id)
                    .one(self.databases.control().read())
                    .await?;
                if self.slug_taken(&row.slug).await? {
                    let row = self.reject_signup(row, "tenant_slug_taken").await?;
                    return Ok((row, None));
                }
                if plan.is_some_and(|plan| plan.requires_kyc) {
                    // Chờ định danh: chuyển trạng thái trước (chỉ một request thắng), rồi mở lượt.
                    let moved = entity::Entity::update_many()
                        .col_expr(entity::Column::Status, Expr::value("awaiting_kyc"))
                        .col_expr(entity::Column::ConfirmedAt, Expr::value(Some(at)))
                        .col_expr(entity::Column::Terms, Expr::value(terms))
                        .col_expr(
                            entity::Column::ExpiresAt,
                            Expr::value(at + SignedDuration::from_hours(24 * KYC_DAYS)),
                        )
                        .col_expr(entity::Column::UpdatedAt, Expr::value(at))
                        .filter(entity::Column::Id.eq(row.id))
                        .filter(entity::Column::Status.eq("pending_email"))
                        .exec(self.databases.control().write())
                        .await?
                        .rows_affected;
                    let row = self.signup(row.id).await?;
                    if moved == 0 && row.status != "awaiting_kyc" {
                        return Ok((row, None));
                    }
                    self.start_kyc(row).await
                } else {
                    let row = self.provision_signup(row, terms, Some(at)).await?;
                    Ok((row, None))
                }
            }
            "awaiting_kyc" => self.start_kyc(row).await,
            _ => Ok((row, None)),
        }
    }

    /// Ghi chấp nhận bản hiện hành của điều khoản sử dụng và chính sách bảo mật (khách nền tảng chưa
    /// đăng loại nào thì bỏ qua loại đó). Trả JSON `{loại: id phiên bản}`.
    async fn accept_terms(&self, email: &str) -> Result<Option<String>, Error> {
        let Some(gate) = &self.acceptance else {
            return Ok(None);
        };
        let mut accepted = serde_json::Map::new();
        for kind in TERMS {
            let subject = Subject::Email(email.to_owned());
            if let Some(version) = gate.accept_current(DEFAULT_TENANT, subject, kind, "web").await?
            {
                accepted.insert(kind.to_owned(), json!(version));
            }
        }
        Ok(Some(serde_json::Value::Object(accepted).to_string()))
    }

    /// Đang chờ định danh: hồ sơ đã có kết quả (event có thể chưa tới) thì đi tiếp ngay; không thì mở
    /// một lượt mới ở nhà cung cấp và trả trang cho người đăng ký.
    async fn start_kyc(
        &self,
        row: entity::Model,
    ) -> Result<(entity::Model, Option<StartedKyc>), Error> {
        let gate = self.kyc.clone().ok_or_else(|| {
            Error::unavailable(
                "plans.kyc_unavailable",
                "Identity checks cannot run on this server right now; try again later.",
            )
        })?;
        let subject = Subject::Reference(format!("plans.signup:{}", row.id));
        let status = gate.status(DEFAULT_TENANT, subject.clone()).await?;
        match status.status.as_str() {
            "verified" => return Ok((self.continue_signup(row).await?, None)),
            "rejected" | "revoked" => {
                return Ok((self.reject_signup(row, "kyc_rejected").await?, None));
            }
            _ => {}
        }
        let party = PartyType::parse(&row.party_type).unwrap_or(PartyType::Organization);
        let return_url =
            self.signup.return_url.as_ref().map(|url| url.replace("{id}", &row.id.to_string()));
        let session = gate.start(DEFAULT_TENANT, subject, party, return_url).await?;
        let mut active = row.into_active_model();
        active.kyc_profile_id = Set(Some(session.profile_id));
        active.updated_at = Set(now());
        let row = active.update(self.databases.control().write()).await?;
        let started = StartedKyc {
            kind: session.kind,
            url: session.url,
            token: session.token,
            expires_at: session.expires_at,
        };
        Ok((row, Some(started)))
    }

    /// Đi tiếp sau định danh: cấp phát.
    pub(crate) async fn continue_signup(&self, row: entity::Model) -> Result<entity::Model, Error> {
        let terms = row.terms.clone();
        let confirmed = row.confirmed_at;
        self.provision_signup(row, terms, confirmed).await
    }

    /// Ghi lượt cấp phát cho đăng ký (cùng transaction chuyển đăng ký sang `provisioning`) và xếp job
    /// `plans.provision`. Cấp phát không nhận (slug vừa bị lấy, gói thôi bán…) → từ chối đăng ký với
    /// mã lỗi làm lý do.
    async fn provision_signup(
        &self,
        row: entity::Model,
        terms: Option<String>,
        confirmed_at: Option<Timestamp>,
    ) -> Result<entity::Model, Error> {
        let mut request = ProvisionRequest::new(
            row.slug.clone(),
            row.tenant_name.clone(),
            row.plan_id,
            row.email.clone(),
        );
        request.price_id = row.price_id;
        request.admin_name = row.admin_name.clone();
        let link = SignupLink { id: row.id, terms, confirmed_at };
        match self.start_provisioning_for(Actor::system(), request, true, Some(link)).await {
            Ok(_) => self.signup(row.id).await,
            Err(error) if (400..500).contains(&error.kind().status()) => {
                tracing::info!(signup_id = %row.id, code = error.code(), "tenant sign-up turned down");
                let reason: String = error.code().chars().take(100).collect();
                self.reject_signup(row, &reason).await
            }
            Err(error) => Err(error),
        }
    }

    /// Từ chối đăng ký còn chờ (gọi lại vô hại).
    pub(crate) async fn reject_signup(
        &self,
        row: entity::Model,
        reason: &str,
    ) -> Result<entity::Model, Error> {
        let at = now();
        let transaction = self.databases.control().begin_write().await?;
        let moved = entity::Entity::update_many()
            .col_expr(entity::Column::Status, Expr::value("rejected"))
            .col_expr(entity::Column::Reason, Expr::value(Some(reason.to_owned())))
            .col_expr(entity::Column::UpdatedAt, Expr::value(at))
            .filter(entity::Column::Id.eq(row.id))
            .filter(entity::Column::Status.is_in(WAITING))
            .exec(&transaction)
            .await?
            .rows_affected;
        if moved == 1 {
            let event = events::SignupRejected(
                json!({"signup_id": row.id, "plan_id": row.plan_id, "reason": reason}),
            );
            self.record(&transaction, Actor::system(), subject(row.id), &event).await?;
        }
        transaction.commit().await?;
        self.signup(row.id).await
    }

    /// Lượt cấp phát `provisioning` xong: đăng ký của nó (nếu có) thành `completed`.
    pub(crate) async fn complete_signup(
        &self,
        provisioning: &provisioning::entity::Model,
    ) -> Result<(), Error> {
        let transaction = self.databases.control().begin_write().await?;
        let found = entity::Entity::find()
            .filter(entity::Column::ProvisioningId.eq(provisioning.id))
            .filter(entity::Column::Status.eq("provisioning"))
            .one(&transaction)
            .await?;
        let Some(row) = found else {
            transaction.rollback().await?;
            return Ok(());
        };
        let mut active = row.into_active_model();
        active.status = Set("completed".to_owned());
        active.tenant_id = Set(provisioning.tenant_id);
        active.updated_at = Set(now());
        let row = active.update(&transaction).await?;
        let event = events::SignupCompleted(
            json!({"signup_id": row.id, "plan_id": row.plan_id, "tenant_id": row.tenant_id}),
        );
        self.record(&transaction, Actor::system(), subject(row.id), &event).await?;
        transaction.commit().await?;
        tracing::info!(signup_id = %row.id, tenant_id = ?row.tenant_id, "tenant sign-up completed");
        Ok(())
    }

    pub(crate) async fn signup(&self, id: Uuid) -> Result<entity::Model, Error> {
        entity::Entity::find_by_id(id)
            .one(self.databases.control().write())
            .await?
            .ok_or_else(signup_not_found)
    }

    /// Trạng thái cho người giữ token trong mail.
    pub(crate) async fn signup_status(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        token: &str,
    ) -> Result<entity::Model, Error> {
        self.require_signup(tenant)?;
        let row = entity::Entity::find_by_id(id)
            .one(self.databases.control().read())
            .await?
            .filter(|row| row.token_hash == sha256(token.trim()))
            .ok_or_else(signup_not_found)?;
        Ok(row)
    }

    /// Danh sách cho nhân sự nền tảng, mới trước.
    pub(crate) async fn list_signups(
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

    /// Lịch [`EXPIRE_SIGNUPS`]: đăng ký chờ quá hạn → `expired`; dòng đã kết thúc quá `KEEP_DAYS`
    /// ngày bị xoá; dọn bộ đếm và nonce của chống spam. Trả số đăng ký vừa hết hạn.
    pub(crate) async fn expire_signups(&self, at: Timestamp) -> Result<u64, Error> {
        let control = self.databases.control();
        let expired = entity::Entity::update_many()
            .col_expr(entity::Column::Status, Expr::value("expired"))
            .col_expr(entity::Column::UpdatedAt, Expr::value(at))
            .filter(entity::Column::Status.is_in(WAITING))
            .filter(entity::Column::ExpiresAt.lte(at))
            .exec(control.write())
            .await?
            .rows_affected;
        let before = at - SignedDuration::from_hours(24 * KEEP_DAYS);
        entity::Entity::delete_many()
            .filter(entity::Column::Status.is_in(["expired", "rejected"]))
            .filter(entity::Column::UpdatedAt.lt(before))
            .exec(control.write())
            .await?;
        self.guard.purge_nonces(control.write(), MODULE).await?;
        Throttle::new(MODULE, 1, HOUR)?
            .purge(control, at - SignedDuration::from_hours(24 * 2))
            .await?;
        if expired > 0 {
            tracing::info!(expired, "tenant sign-ups expired");
        }
        Ok(expired)
    }
}

/// Đăng ký đi kèm một lượt cấp phát (ghi cùng transaction).
pub(crate) struct SignupLink {
    pub id: Uuid,
    pub terms: Option<String>,
    pub confirmed_at: Option<Timestamp>,
}

/// Trong transaction ghi lượt cấp phát `provisioning_id`: chuyển đăng ký đang chờ sang
/// `provisioning`. `false` khi đăng ký không còn chờ (request khác đã làm).
pub(crate) async fn claim_for_provisioning<C: ConnectionTrait>(
    connection: &C,
    link: &SignupLink,
    provisioning_id: Uuid,
) -> Result<bool, Error> {
    let moved = entity::Entity::update_many()
        .col_expr(entity::Column::Status, Expr::value("provisioning"))
        .col_expr(entity::Column::ProvisioningId, Expr::value(Some(provisioning_id)))
        .col_expr(entity::Column::Terms, Expr::value(link.terms.clone()))
        .col_expr(entity::Column::ConfirmedAt, Expr::value(link.confirmed_at))
        .col_expr(entity::Column::UpdatedAt, Expr::value(now()))
        .filter(entity::Column::Id.eq(link.id))
        .filter(entity::Column::Status.is_in(WAITING))
        .exec(connection)
        .await?
        .rows_affected;
    Ok(moved == 1)
}

fn subject(id: Uuid) -> String {
    format!("plans.signup:{id}")
}
