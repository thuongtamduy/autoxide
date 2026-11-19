//! Mời người dùng qua email (bước 34 mục 12, D-312).
//!
//! Người có `auth.invitations.manage` ở một nút mời một email kèm các role ở nút đó (không mời
//! kèm role có permission mình không có, không mời vào nút ngoài phạm vi). Job
//! `auth.send_invitation` phát token dùng một lần (hạn = hạn lời mời) và gửi mail; gửi lại thu hồi
//! token cũ. Chấp nhận: email chưa có tài khoản nhân sự → tạo (email coi là đã xác minh, đặt mật
//! khẩu nếu gửi) và đăng nhập luôn; đã có → phải đang đăng nhập đúng tài khoản đó, chỉ thêm role.

use std::collections::BTreeSet;
use std::sync::Arc;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::i18n::{Language, Translations};
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler, JobQueue};
use autoxide_kernel::mail::{MailTemplate, Mailer};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{
    DatabaseRegistry, OrgId, ScopeExt, TenantContext, link_token, root_of,
};
use autoxide_kernel::tokens::{OneTimeTokens, TokenRejection};
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::events;
use crate::permissions::INVITATIONS_MANAGE;
use crate::session::{SessionService, Tokens};
use crate::user::{NewUser, UserService};

const PURPOSE: &str = "invitation";
/// Lời mời hết hạn quá chừng này ngày thì job dọn xoá dòng (event audit giữ).
pub(crate) const KEEP_DAYS: i64 = 90;

fn tokens() -> OneTimeTokens {
    OneTimeTokens::new("auth")
}

pub(crate) mod entity {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_invitation")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub email: String,
        pub name: String,
        pub org_id: Uuid,
        /// JSON: mảng ID role.
        pub role_ids: String,
        pub reference: Option<String>,
        pub invited_by: Option<Uuid>,
        /// `pending`, `accepted`, `revoked`.
        pub status: String,
        pub expires_at: DateTimeUtc,
        pub accepted_user_id: Option<Uuid>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl autoxide_kernel::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}

pub(crate) type Invitation = entity::Model;

impl entity::Model {
    pub(crate) fn roles(&self) -> Vec<Uuid> {
        serde_json::from_str(&self.role_ids).unwrap_or_default()
    }

    /// Còn chấp nhận được lúc `at`.
    fn open(&self, at: DateTime<Utc>) -> bool {
        self.status == "pending" && self.expires_at > at
    }
}

/// Lời mời mới.
pub(crate) struct NewInvitation {
    pub email: String,
    pub name: Option<String>,
    /// `None`: nút gốc.
    pub org_id: Option<Uuid>,
    pub role_ids: Vec<Uuid>,
    pub reference: Option<String>,
}

const INVITE_LINK: MailTemplate = MailTemplate::new(
    "auth.mail.invitation_link",
    "You are invited to {tenant}",
    "{inviter} invited you to work in {tenant}. Accept within {days} days by opening this \
     link:\n\n{link}\n\nIf you do not want to, ignore this mail.",
);

const INVITE_CODE: MailTemplate = MailTemplate::new(
    "auth.mail.invitation_code",
    "You are invited to {tenant}",
    "{inviter} invited you to work in {tenant}. Accept within {days} days with this token \
     (`POST /api/v1/auth/invitations/accept`):\n\n{token}\n\nIf you do not want to, ignore this \
     mail.",
);

/// Loại job gửi mail mời.
pub(crate) const SEND_INVITATION: &str = "auth.send_invitation";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct InvitationJob {
    pub invitation_id: Uuid,
    pub language: String,
}

impl Job for InvitationJob {
    const KIND: &'static str = SEND_INVITATION;
}

pub(crate) struct InvitationMailer(pub Arc<Invitations>);

#[async_trait::async_trait]
impl JobHandler for InvitationMailer {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let tenant = job
            .tenant
            .as_ref()
            .ok_or_else(|| Error::internal("an invitation mail job must belong to a tenant"))?;
        let job: InvitationJob =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        self.0.send(tenant, &job).await
    }
}

/// Kết quả chấp nhận: tài khoản, và cặp token khi vừa tạo tài khoản mới.
pub(crate) struct Accepted {
    pub tenant: TenantContext,
    pub user_id: Uuid,
    pub tokens: Option<Tokens>,
}

pub(crate) struct Invitations {
    pub users: Arc<UserService>,
    pub sessions: Arc<SessionService>,
    pub jobs: JobQueue,
    pub mailer: Arc<dyn Mailer>,
    pub translations: Translations,
    pub outbox: Outbox,
    pub databases: DatabaseRegistry,
    /// Link theo tên miền của khách (`kernel.public_url`).
    pub links: autoxide_kernel::tenancy::SiteLinks,
    /// Trang của giao diện, có `{token}`; `None`: mail chứa token.
    pub url: Option<String>,
    /// `AUTOXIDE_AUTH_INVITATION_DAYS`.
    pub days: i64,
    /// Hạn mức số nhân sự theo gói (bước 35).
    pub entitlements: autoxide_kernel::entitlement::EntitlementsHandle,
}

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

impl Invitations {
    /// Mời (người gọi đã có `auth.invitations.manage`; `principal`: người mời, `None` khi lệnh CLI
    /// của người vận hành — khi đó không kiểm phạm vi).
    pub(crate) async fn create(
        &self,
        tenant: &TenantContext,
        principal: Option<&Principal>,
        input: NewInvitation,
        language: Language,
    ) -> Result<Invitation, Error> {
        let email = autoxide_kernel::email::normalize(&input.email);
        let name = input.name.as_deref().map(str::trim).unwrap_or_default().to_owned();
        let org_id = input.org_id.unwrap_or(root_of(tenant.id()).as_uuid());
        let roles: BTreeSet<Uuid> = input.role_ids.into_iter().collect();
        let mut problems: Vec<FieldError> =
            crate::user::check_email_field("email", &email).into_iter().collect();
        if name.chars().count() > 200 {
            problems.push(
                FieldError::new("name", "length", "must be at most {max} characters long")
                    .with_param("max", 200),
            );
        }
        if input.reference.as_deref().is_some_and(|reference| reference.chars().count() > 200) {
            problems.push(
                FieldError::new("reference", "length", "must be at most {max} characters long")
                    .with_param("max", 200),
            );
        }
        let read = tenant.database().read();
        if let Some(actor) = principal {
            actor
                .require_at(
                    read,
                    &INVITATIONS_MANAGE,
                    OrgTarget::new(OrgId::from_uuid(org_id), None),
                )
                .await?;
            problems.extend(
                crate::role::grant_problems(read, tenant, actor, "", org_id, &roles).await?,
            );
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let now = now();
        let row = entity::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            id: Set(Uuid::now_v7()),
            email: Set(email),
            name: Set(name),
            org_id: Set(org_id),
            role_ids: Set(serde_json::to_string(&roles).map_err(Error::internal)?),
            reference: Set(input.reference),
            invited_by: Set(principal.map(Principal::user_id)),
            status: Set("pending".to_owned()),
            expires_at: Set(now + chrono::Duration::days(self.days)),
            accepted_user_id: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        };
        let actor = principal.map_or_else(Actor::system, Actor::from);
        let transaction = tenant.database().begin_write().await?;
        let row = row.insert(&transaction).await?;
        let event = events::InvitationCreated { before: None, after: Some(describe(&row)) };
        self.outbox.record(&transaction, tenant, actor, subject(row.id), &event).await?;
        let job = InvitationJob { invitation_id: row.id, language: language.tag().to_owned() };
        self.jobs.enqueue(&transaction, tenant, &job, EnqueueOptions::default()).await?;
        transaction.commit().await?;
        tracing::info!(invitation_id = %row.id, "invitation created");
        Ok(row)
    }

    async fn find(&self, tenant: &TenantContext, id: Uuid) -> Result<Invitation, Error> {
        entity::Entity::find_by_id((tenant.id().as_uuid(), id))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| not_found(id))
    }

    /// Người gọi quản lý lời mời ở nút của nó (phạm vi của `auth.invitations.manage`); ngoài phạm
    /// vi thì như không có.
    async fn managed(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Invitation, Error> {
        let row = self.find(tenant, id).await?;
        let target = OrgTarget::new(OrgId::from_uuid(row.org_id), None);
        principal
            .require_at(tenant.database().read(), &INVITATIONS_MANAGE, target)
            .await
            .map_err(|_| not_found(id))?;
        Ok(row)
    }

    /// Danh sách lời mời (`status`: lọc), theo phạm vi của người gọi.
    pub(crate) async fn list(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        status: Option<&str>,
        page: &PageRequest,
    ) -> Result<Page<Invitation>, Error> {
        let scope = principal.scope_condition(&INVITATIONS_MANAGE, entity::Column::OrgId, None)?;
        let mut select = entity::Entity::find().filter(scope).scoped(tenant);
        if let Some(status) = status {
            select = select.filter(entity::Column::Status.eq(status));
        }
        fetch_page(select, entity::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Thu hồi: link đã gửi hết dùng được. Đã thu hồi: giữ nguyên.
    pub(crate) async fn revoke(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Invitation, Error> {
        let current = self.managed(tenant, principal, id).await?;
        if current.status != "pending" {
            return if current.status == "revoked" { Ok(current) } else { Err(not_pending(id)) };
        }
        let transaction = tenant.database().begin_write().await?;
        entity::Entity::update_many()
            .col_expr(entity::Column::Status, Expr::value("revoked"))
            .col_expr(entity::Column::UpdatedAt, Expr::value(now()))
            .filter(entity::Column::Id.eq(id))
            .filter(entity::Column::Status.eq("pending"))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        tokens().revoke(&transaction, tenant, PURPOSE, &id.to_string()).await?;
        let after = entity::Entity::find_by_id((tenant.id().as_uuid(), id))
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| not_found(id))?;
        let event = events::InvitationRevoked {
            before: Some(describe(&current)),
            after: Some(describe(&after)),
        };
        self.outbox
            .record(&transaction, tenant, Actor::from(principal), subject(id), &event)
            .await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Gửi lại: hạn tính lại từ bây giờ, token cũ thu hồi (khi job phát token mới).
    pub(crate) async fn resend(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        language: Language,
    ) -> Result<Invitation, Error> {
        let current = self.managed(tenant, principal, id).await?;
        if current.status != "pending" {
            return Err(not_pending(id));
        }
        let now = now();
        let transaction = tenant.database().begin_write().await?;
        entity::Entity::update_many()
            .col_expr(
                entity::Column::ExpiresAt,
                Expr::value(now + chrono::Duration::days(self.days)),
            )
            .col_expr(entity::Column::UpdatedAt, Expr::value(now))
            .filter(entity::Column::Id.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        tokens().revoke(&transaction, tenant, PURPOSE, &id.to_string()).await?;
        let job = InvitationJob { invitation_id: id, language: language.tag().to_owned() };
        self.jobs.enqueue(&transaction, tenant, &job, EnqueueOptions::default()).await?;
        transaction.commit().await?;
        self.find(tenant, id).await
    }

    /// Job: phát token và gửi mail cho lời mời còn chờ.
    pub(crate) async fn send(
        &self,
        tenant: &TenantContext,
        job: &InvitationJob,
    ) -> Result<(), Error> {
        let Ok(invitation) = self.find(tenant, job.invitation_id).await else { return Ok(()) };
        let now = now();
        if !invitation.open(now) {
            return Ok(());
        }
        let language = if job.language == Language::Vi.tag() { Language::Vi } else { Language::En };
        let inviter = match invitation.invited_by {
            Some(id) => self.users.get(tenant, id).await?.map(|user| user.name),
            None => None,
        }
        .unwrap_or_else(|| tenant.name().to_owned());
        let subject = invitation.id.to_string();
        let transaction = tenant.database().begin_write().await?;
        tokens().revoke(&transaction, tenant, PURPOSE, &subject).await?;
        let ttl = invitation.expires_at - now;
        let raw = tokens().issue(&transaction, tenant, PURPOSE, &subject, None, ttl).await?.token;
        transaction.commit().await?;
        let token = link_token(tenant, &raw);
        let days = self.days.to_string();
        let tenant_name = tenant.name().to_owned();
        let mail = match &self.url {
            Some(url) => {
                let link = self.links.rewrite(tenant, url).await?.replace("{token}", &token);
                let params = [
                    ("tenant", tenant_name.as_str()),
                    ("inviter", &inviter),
                    ("days", &days),
                    ("link", &link),
                ];
                INVITE_LINK.mail(&invitation.email, &self.translations, language, &params)
            }
            None => {
                let params = [
                    ("tenant", tenant_name.as_str()),
                    ("inviter", &inviter),
                    ("days", &days),
                    ("token", &token),
                ];
                INVITE_CODE.mail(&invitation.email, &self.translations, language, &params)
            }
        };
        self.mailer.send(tenant, mail).await
    }

    /// Chấp nhận lời mời bằng `token` (D-312). `principal`: người gọi nếu đã đăng nhập.
    pub(crate) async fn accept(
        &self,
        request: Option<&TenantContext>,
        token: &str,
        name: Option<String>,
        password: Option<String>,
        principal: Option<&Principal>,
        user_agent: Option<&str>,
    ) -> Result<Accepted, Error> {
        if let Some(problem) = FieldError::blank("token", token) {
            return Err(Error::validation(vec![problem]));
        }
        let Some((tenant, raw)) = crate::token::resolve(&self.databases, request, token).await?
        else {
            return Err(invalid().with_reason(TokenRejection::Unknown.as_str()));
        };
        // Xem token (chưa tiêu) để biết lời mời: kiểm mọi đầu vào TRƯỚC khi tiêu token.
        let peeked = tokens().peek(tenant.database().read(), &tenant, PURPOSE, raw).await?;
        let Some(id) = peeked.and_then(|token| token.subject.parse::<Uuid>().ok()) else {
            let why = tokens().rejection(tenant.database().read(), &tenant, PURPOSE, raw).await?;
            return Err(invalid().with_reason(why.as_str()));
        };
        let invitation = self.find(&tenant, id).await.map_err(|_| invalid())?;
        if !invitation.open(now()) {
            return Err(invalid().with_reason("closed"));
        }
        let existing = self.users.find_by_email(&tenant, &invitation.email).await?;
        if existing.is_none() && principal.is_some() {
            // Đang đăng nhập bằng tài khoản khác email được mời.
            return Err(Error::forbidden(
                "auth.invitation_email_mismatch",
                "This invitation was sent to another email than the account you are using.",
            ));
        }
        let new_account = match &existing {
            Some(user) => {
                let Some(caller) = principal.filter(|caller| caller.tenant_id() == tenant.id())
                else {
                    return Err(Error::conflict(
                        "auth.invitation_sign_in_required",
                        "This email already has an account: sign in, then accept the invitation.",
                    ));
                };
                if caller.user_id() != user.id {
                    return Err(Error::forbidden(
                        "auth.invitation_email_mismatch",
                        "This invitation was sent to another email than the account you are using.",
                    ));
                }
                None
            }
            None => {
                let name = name
                    .map(|name| name.trim().to_owned())
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| {
                        if invitation.name.is_empty() {
                            invitation.email.clone()
                        } else {
                            invitation.name.clone()
                        }
                    });
                let problems: Vec<FieldError> = password
                    .as_deref()
                    .and_then(|password| crate::password::check_rules("password", password))
                    .into_iter()
                    .collect();
                if !problems.is_empty() {
                    return Err(Error::validation(problems));
                }
                Some(NewUser { email: invitation.email.clone(), name, password })
            }
        };
        // Chỉ một request thắng (16 request cùng token: một thành công).
        let consumed = tokens().consume(tenant.database().write(), &tenant, PURPOSE, raw).await?;
        if consumed.is_none() {
            let why = tokens().rejection(tenant.database().read(), &tenant, PURPOSE, raw).await?;
            return Err(invalid().with_reason(why.as_str()));
        }
        let user_id = match new_account {
            Some(input) => self.users.create(&tenant, Actor::system(), input).await?.id,
            None => existing.map(|user| user.id).ok_or_else(invalid)?,
        };
        let roles: BTreeSet<Uuid> = invitation.roles().into_iter().collect();
        // Hạn mức số nhân sự (bước 35): người chưa giữ role nào ở khách chiếm thêm một chỗ — giữ chỗ
        // ngoài transaction của module, lỗi thì trả lại (D-326).
        let joining = !roles.is_empty() && !crate::role::has_membership(&tenant, user_id).await?;
        let reference = crate::role::member_reference(user_id);
        let tenant_id = tenant.id();
        if joining {
            self.entitlements
                .reserve(tenant_id, &crate::role::MEMBERS_LIMIT, 1, &reference)
                .await?;
        }
        let accepted =
            self.accept_roles(tenant, user_id, &invitation, &roles, principal, user_agent).await;
        if joining {
            match &accepted {
                Ok(done) => {
                    let limit = &crate::role::MEMBERS_LIMIT;
                    self.entitlements.confirm(done.tenant.id(), limit, &reference).await?;
                }
                Err(_) => {
                    let limit = &crate::role::MEMBERS_LIMIT;
                    self.entitlements.release(tenant_id, limit, &reference).await?;
                }
            }
        }
        accepted
    }

    /// Gán role của lời mời `invitation` cho `user_id` trong một transaction (đã tiêu token).
    async fn accept_roles(
        &self,
        tenant: TenantContext,
        user_id: Uuid,
        invitation: &Invitation,
        roles: &BTreeSet<Uuid>,
        principal: Option<&Principal>,
        user_agent: Option<&str>,
    ) -> Result<Accepted, Error> {
        let id = invitation.id;
        let transaction = tenant.database().begin_write().await?;
        if principal.is_none() {
            // Tài khoản mới: email nhận được mail mời là của người này.
            self.users
                .apply_verified_email(&tenant, &transaction, user_id, &invitation.email, now())
                .await?;
        }
        crate::role::add_roles(&transaction, &tenant, user_id, invitation.org_id, roles).await?;
        entity::Entity::update_many()
            .col_expr(entity::Column::Status, Expr::value("accepted"))
            .col_expr(entity::Column::AcceptedUserId, Expr::value(Some(user_id)))
            .col_expr(entity::Column::UpdatedAt, Expr::value(now()))
            .filter(entity::Column::Id.eq(id))
            .scoped(&tenant)
            .exec(&transaction)
            .await?;
        let event = events::InvitationAccepted {
            before: None,
            after: Some(
                json!({"invitation_id": id, "user_id": user_id, "org_id": invitation.org_id, "role_ids": roles}),
            ),
        };
        self.outbox
            .record(&transaction, &tenant, Actor::user(user_id), subject(id), &event)
            .await?;
        transaction.commit().await?;
        let tokens = if existing_account(principal) {
            None
        } else {
            Some(self.sessions.open_session(&tenant, user_id, "invitation", user_agent).await?)
        };
        Ok(Accepted { tenant, user_id, tokens })
    }
}

fn existing_account(principal: Option<&Principal>) -> bool {
    principal.is_some()
}

fn subject(id: Uuid) -> Option<String> {
    Some(format!("auth.invitation:{id}"))
}

fn describe(row: &Invitation) -> serde_json::Value {
    json!({
        "email": row.email,
        "org_id": row.org_id,
        "role_ids": row.roles(),
        "status": row.status,
        "expires_at": row.expires_at,
        "reference": row.reference,
    })
}

pub(crate) fn not_found(id: Uuid) -> Error {
    Error::not_found("auth.invitation_not_found", "There is no invitation `{id}`.")
        .with_param("id", id)
}

fn not_pending(id: Uuid) -> Error {
    Error::conflict(
        "auth.invitation_not_pending",
        "Invitation `{id}` was already accepted or revoked.",
    )
    .with_param("id", id)
}

fn invalid() -> Error {
    Error::bad_request(
        "auth.invitation_invalid",
        "This invitation is wrong, already used, revoked or expired. Ask for a new one.",
    )
}

#[async_trait::async_trait]
impl crate::api::Invitations for Invitations {
    async fn invite(
        &self,
        tenant: &TenantContext,
        invitation: crate::api::NewInvitation,
    ) -> Result<crate::api::InvitationSummary, Error> {
        let mut role_ids = Vec::new();
        let mut problems = Vec::new();
        for name in &invitation.roles {
            match crate::role::role_by_name(tenant, name).await? {
                Some(id) => role_ids.push(id),
                None => problems.push(
                    FieldError::new("roles", "unknown", "there is no role `{role}`")
                        .with_param("role", name),
                ),
            }
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let input = NewInvitation {
            email: invitation.email,
            name: invitation.name,
            org_id: invitation.org_id,
            role_ids,
            reference: None,
        };
        let row = self.create(tenant, None, input, Language::En).await?;
        Ok(crate::api::InvitationSummary {
            id: row.id,
            email: row.email,
            expires_at: row.expires_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use autoxide_kernel::access::Principal;
    use autoxide_kernel::event::Actor;
    use autoxide_kernel::i18n::Language;
    use autoxide_testkit::TestDatabase;

    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    use super::{InvitationJob, Invitations, NewInvitation, SEND_INVITATION};
    use crate::role::RoleInput;
    use crate::testing::{Services, last_token, services, take_jobs};
    use crate::user::NewUser;

    fn invitations(services: &Services) -> Invitations {
        Invitations {
            links: Default::default(),
            users: services.users.clone(),
            sessions: services.sessions.clone(),
            jobs: autoxide_kernel::job::JobQueue::new("auth"),
            mailer: services.mails.clone(),
            translations: autoxide_kernel::i18n::Translations::default(),
            outbox: autoxide_kernel::event::Outbox::standalone("auth"),
            databases: autoxide_kernel::tenancy::DatabaseRegistry::single(
                services.database.clone(),
            ),
            url: None,
            days: 7,
            entitlements: Default::default(),
        }
    }

    async fn role(services: &Services, permissions: &[&str]) -> uuid::Uuid {
        let input = RoleInput {
            denies: Vec::new(),
            requires_mfa: false,
            name: "helpdesk".to_owned(),
            description: String::new(),
            permissions: permissions.iter().map(|p| (*p).to_owned()).collect(),
            grants: Vec::new(),
        };
        services.roles.create(&services.tenant, Actor::system(), input).await.unwrap().id
    }

    fn admin(services: &Services) -> Principal {
        Principal::with_all_permissions(uuid::Uuid::now_v7()).in_tenant(services.tenant.id())
    }

    fn invite(email: &str, role: uuid::Uuid) -> NewInvitation {
        NewInvitation {
            email: email.to_owned(),
            name: None,
            org_id: None,
            role_ids: vec![role],
            reference: None,
        }
    }

    /// Chạy job gửi mail mời như worker; trả token trong mail cuối.
    async fn deliver(services: &Services, invitations: &Invitations) -> String {
        for payload in take_jobs(services, SEND_INVITATION).await {
            let job: InvitationJob = serde_json::from_str(&payload).unwrap();
            invitations.send(&services.tenant, &job).await.unwrap();
        }
        last_token(services)
    }

    #[tokio::test]
    async fn chap_nhan_loi_moi_dong_thoi() {
        TestDatabase::run(async |test| {
            let services = services(test).await;
            let role = role(&services, &["auth.users.read"]).await;
            let invitations = Arc::new(invitations(&services));
            let boss = admin(&services);
            invitations
                .create(
                    &services.tenant,
                    Some(&boss),
                    invite("new@example.com", role),
                    Language::En,
                )
                .await
                .unwrap();
            let token = deliver(&services, &invitations).await;
            let mut tasks = Vec::new();
            for _ in 0..16 {
                let (invitations, tenant, token) =
                    (invitations.clone(), services.tenant.clone(), token.clone());
                tasks.push(tokio::spawn(async move {
                    invitations.accept(Some(&tenant), &token, None, None, None, None).await.ok()
                }));
            }
            let mut accepted = Vec::new();
            for task in tasks {
                accepted.extend(task.await.unwrap());
            }
            assert_eq!(accepted.len(), 1, "một request thắng");
            assert!(accepted[0].tokens.is_some(), "tài khoản mới: đăng nhập luôn");
            let user =
                services.users.get(&services.tenant, accepted[0].user_id).await.unwrap().unwrap();
            assert!(user.email_verified_at.is_some(), "email được mời coi là đã xác minh");
            let principal = services.roles.principal(&services.tenant, user.id).await.unwrap();
            assert!(principal.allows("auth.users.read"));
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn loi_moi_het_han_thu_hoi_gui_lai() {
        TestDatabase::run(async |test| {
            let services = services(test).await;
            let tenant = &services.tenant;
            let role = role(&services, &[]).await;
            let invitations = invitations(&services);
            let boss = admin(&services);
            let row = invitations
                .create(tenant, Some(&boss), invite("a@example.com", role), Language::En)
                .await
                .unwrap();
            let first = deliver(&services, &invitations).await;
            invitations.resend(tenant, &boss, row.id, Language::En).await.unwrap();
            let second = deliver(&services, &invitations).await;
            assert_ne!(first, second);
            let old = invitations.accept(Some(tenant), &first, None, None, None, None).await;
            assert_eq!(
                old.err().unwrap().code(),
                "auth.invitation_invalid",
                "gửi lại thu hồi link cũ"
            );
            let revoked = invitations.revoke(tenant, &boss, row.id).await.unwrap();
            assert_eq!(revoked.status, "revoked");
            let late = invitations.accept(Some(tenant), &second, None, None, None, None).await;
            assert_eq!(late.err().unwrap().code(), "auth.invitation_invalid");
            let resend = invitations.resend(tenant, &boss, row.id, Language::En).await;
            assert_eq!(resend.unwrap_err().code(), "auth.invitation_not_pending");

            // Hết hạn: không chấp nhận được.
            let other = invitations
                .create(tenant, Some(&boss), invite("b@example.com", role), Language::En)
                .await
                .unwrap();
            let token = deliver(&services, &invitations).await;
            super::entity::Entity::update_many()
                .col_expr(
                    super::entity::Column::ExpiresAt,
                    sea_orm::sea_query::Expr::value(
                        chrono::Utc::now() - chrono::Duration::minutes(1),
                    ),
                )
                .filter(super::entity::Column::Id.eq(other.id))
                .exec(services.database.write())
                .await
                .unwrap();
            let expired = invitations.accept(Some(tenant), &token, None, None, None, None).await;
            assert_eq!(expired.err().unwrap().code(), "auth.invitation_invalid");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn nguoi_moi_khong_cap_duoc_quyen_minh_khong_co() {
        TestDatabase::run(async |test| {
            let services = services(test).await;
            let tenant = &services.tenant;
            let powerful = role(&services, &["auth.users.manage"]).await;
            let invitations = invitations(&services);
            let inviter =
                Principal::new(uuid::Uuid::now_v7(), ["auth.invitations.manage".to_owned()])
                    .in_tenant(tenant.id());
            let refused = invitations
                .create(tenant, Some(&inviter), invite("c@example.com", powerful), Language::En)
                .await
                .unwrap_err();
            assert_eq!(refused.fields()[0].code, "more_than_you_have");

            // Email đã có tài khoản: phải đăng nhập đúng tài khoản đó.
            let input = NewUser {
                email: "dan@example.com".to_owned(),
                name: "Dan".to_owned(),
                password: None,
            };
            let dan = services.users.create(tenant, Actor::system(), input).await.unwrap();
            let boss = admin(&services);
            let existing = invite("dan@example.com", powerful);
            invitations.create(tenant, Some(&boss), existing, Language::En).await.unwrap();
            let token = deliver(&services, &invitations).await;
            let anonymous = invitations.accept(Some(tenant), &token, None, None, None, None).await;
            assert_eq!(anonymous.err().unwrap().code(), "auth.invitation_sign_in_required");
            let stranger = Principal::new(uuid::Uuid::now_v7(), []).in_tenant(tenant.id());
            let wrong =
                invitations.accept(Some(tenant), &token, None, None, Some(&stranger), None).await;
            assert_eq!(wrong.err().unwrap().code(), "auth.invitation_email_mismatch");
            let me = Principal::new(dan.id, []).in_tenant(tenant.id());
            let accepted = invitations
                .accept(Some(tenant), &token, None, None, Some(&me), None)
                .await
                .unwrap();
            assert!(accepted.tokens.is_none(), "tài khoản có sẵn: không mở phiên mới");
            assert!(
                services.roles.principal(tenant, dan.id).await.unwrap().allows("auth.users.manage")
            );
        })
        .await
        .unwrap();
    }
}
