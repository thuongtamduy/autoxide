//! Sổ đồng ý theo mục đích (Luật Bảo vệ dữ liệu cá nhân 91/2025, NĐ 356/2025 — cần đối chiếu): mỗi
//! (đối tượng, mục đích, phạm vi, kênh) một trạng thái `granted` / `withdrawn`, mỗi lần đổi thêm một bản
//! ghi bằng chứng (chỉ thêm, không trùng theo `source_ref`). Email / số điện thoại khoá bằng SHA-256;
//! giá trị gốc giữ để gửi tin, bỏ khi xoá theo yêu cầu.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde_json::json;
use uuid::Uuid;

use crate::api::{Consent, ConsentEvidence, ConsentKey, ConsentSubject};
use crate::entity::{consent, consent_record};
use crate::events;
use crate::service::{ComplianceService, now};

/// (`subject_kind`, `subject_key`, `subject_value`, `user_id`) của đối tượng đồng ý.
fn subject_columns(
    subject: &ConsentSubject,
) -> (&'static str, String, Option<String>, Option<Uuid>) {
    match subject {
        ConsentSubject::User(id) => ("user", id.to_string(), None, Some(*id)),
        ConsentSubject::Email(email) => {
            let email = email.trim().to_lowercase();
            (
                "email",
                autoxide_kernel::secret::sha256(&format!("consent-email|{email}")),
                Some(email),
                None,
            )
        }
        ConsentSubject::Phone(phone) => (
            "phone",
            autoxide_kernel::secret::sha256(&format!("consent-phone|{phone}")),
            Some(phone.clone()),
            None,
        ),
    }
}

pub(crate) fn describe_consent(row: &consent::Model) -> Consent {
    Consent {
        id: row.id,
        purpose: row.purpose.clone(),
        scope: row.scope.clone(),
        channel: row.channel.clone(),
        state: row.state.clone(),
        granted_at: row.granted_at,
        withdrawn_at: row.withdrawn_at,
        subject_value: row.subject_value.clone(),
    }
}

/// Một lần đổi trạng thái đồng ý.
struct Change<'a> {
    /// `grant` hay `withdraw`.
    action: &'static str,
    evidence: serde_json::Value,
    source: &'a str,
    source_ref: &'a str,
}

fn check_key(key: &ConsentKey) -> Result<(), Error> {
    let mut problems = Vec::new();
    if !(1..=64).contains(&key.purpose.len()) {
        problems.push(FieldError::new("purpose", "length", "must be 1 to 64 characters"));
    }
    if key.scope.len() > 100 {
        problems.push(FieldError::new("scope", "length", "must be at most 100 characters"));
    }
    if !["", "email", "sms", "zns", "push", "in_app"].contains(&key.channel.as_str()) {
        problems.push(FieldError::new(
            "channel",
            "invalid",
            "must be empty, `email`, `sms`, `zns`, `push` or `in_app`",
        ));
    }
    if problems.is_empty() { Ok(()) } else { Err(Error::validation(problems)) }
}

impl ComplianceService {
    /// Đổi trạng thái đồng ý (`grant` / `withdraw`), gọi lại cùng `source_ref` → không ghi thêm.
    async fn change_consent(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        subject: &ConsentSubject,
        key: &ConsentKey,
        change: Change<'_>,
    ) -> Result<Consent, Error> {
        let Change { action, evidence, source, source_ref } = change;
        check_key(key)?;
        if source_ref.is_empty() || source_ref.len() > 200 {
            return Err(Error::validation(vec![FieldError::new(
                "source_ref",
                "length",
                "must be 1 to 200 characters",
            )]));
        }
        let (kind, subject_key, value, user_id) = subject_columns(subject);
        let database = tenant.database();
        let (outbox, evidence) = (&self.outbox, &evidence);
        let (subject_key, value) = (&subject_key, &value);
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let at = now();
                consent::Entity::insert(consent::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    subject_kind: Set(kind.to_owned()),
                    subject_key: Set(subject_key.clone()),
                    subject_value: Set(value.clone()),
                    user_id: Set(user_id),
                    purpose: Set(key.purpose.clone()),
                    scope: Set(key.scope.clone()),
                    channel: Set(key.channel.clone()),
                    state: Set("withdrawn".to_owned()),
                    granted_at: Set(None),
                    withdrawn_at: Set(None),
                    updated_at: Set(at),
                })
                .on_conflict(
                    OnConflict::columns([
                        consent::Column::TenantId,
                        consent::Column::SubjectKind,
                        consent::Column::SubjectKey,
                        consent::Column::Purpose,
                        consent::Column::Scope,
                        consent::Column::Channel,
                    ])
                    .do_nothing()
                    .to_owned(),
                )
                .exec_without_returning(&transaction)
                .await?;
                let row = rows::lock_one(
                    &transaction,
                    consent::Entity::find()
                        .scoped(tenant)
                        .filter(consent::Column::SubjectKind.eq(kind))
                        .filter(consent::Column::SubjectKey.eq(subject_key.as_str()))
                        .filter(consent::Column::Purpose.eq(key.purpose.as_str()))
                        .filter(consent::Column::Scope.eq(key.scope.as_str()))
                        .filter(consent::Column::Channel.eq(key.channel.as_str())),
                )
                .await?
                .ok_or_else(|| Error::internal("a consent vanished"))?;
                let recorded = consent_record::Entity::insert(consent_record::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    consent_id: Set(row.id),
                    action: Set(action.to_owned()),
                    evidence: Set(evidence.to_string()),
                    source: Set(source.to_owned()),
                    source_ref: Set(source_ref.to_owned()),
                    recorded_at: Set(at),
                })
                .on_conflict(
                    OnConflict::columns([
                        consent_record::Column::TenantId,
                        consent_record::Column::SourceRef,
                        consent_record::Column::Action,
                    ])
                    .do_nothing()
                    .to_owned(),
                )
                .exec_without_returning(&transaction)
                .await?;
                if recorded == 0 {
                    transaction.commit().await?;
                    return Ok(describe_consent(&row));
                }
                let mut active: consent::ActiveModel = row.into();
                if value.is_some() {
                    active.subject_value = Set(value.clone());
                }
                if action == "grant" {
                    active.state = Set("granted".to_owned());
                    active.granted_at = Set(Some(at));
                } else {
                    active.state = Set("withdrawn".to_owned());
                    active.withdrawn_at = Set(Some(at));
                }
                active.updated_at = Set(at);
                let saved = consent::Entity::update(active).exec(&transaction).await?;
                let payload = json!({
                    "consent_id": saved.id, "purpose": saved.purpose, "scope": saved.scope, "channel": saved.channel,
                });
                let subject = Some(format!("compliance.consent:{}", saved.id));
                if action == "grant" {
                    outbox.record(&transaction, tenant, actor, subject, &events::ConsentGranted(payload)).await?;
                } else {
                    outbox.record(&transaction, tenant, actor, subject, &events::ConsentWithdrawn(payload)).await?;
                }
                transaction.commit().await?;
                Ok(describe_consent(&saved))
            })
            .await
    }

    pub(crate) async fn grant_consent(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        subject: &ConsentSubject,
        key: &ConsentKey,
        evidence: &ConsentEvidence,
        source_ref: &str,
    ) -> Result<Consent, Error> {
        if evidence.text.trim().is_empty() || evidence.text_version.trim().is_empty() {
            return Err(Error::validation(vec![FieldError::new(
                "text",
                "required",
                "the text shown to the person and its version are required as proof",
            )]));
        }
        let evidence_json = serde_json::to_value(evidence).map_err(Error::internal)?;
        let change = Change {
            action: "grant",
            evidence: evidence_json,
            source: &evidence.source,
            source_ref,
        };
        self.change_consent(tenant, actor, subject, key, change).await
    }

    pub(crate) async fn withdraw_consent(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        subject: &ConsentSubject,
        key: &ConsentKey,
        source: &str,
        source_ref: &str,
    ) -> Result<Consent, Error> {
        let change =
            Change { action: "withdraw", evidence: json!({"source": source}), source, source_ref };
        self.change_consent(tenant, actor, subject, key, change).await
    }

    async fn candidates<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        subject: &ConsentSubject,
        key: &ConsentKey,
    ) -> Result<Vec<consent::Model>, Error> {
        let (kind, subject_key, _, _) = subject_columns(subject);
        let scopes: Vec<&str> =
            if key.scope.is_empty() { vec![""] } else { vec!["", key.scope.as_str()] };
        let channels: Vec<&str> =
            if key.channel.is_empty() { vec![""] } else { vec!["", key.channel.as_str()] };
        Ok(consent::Entity::find()
            .scoped(tenant)
            .filter(consent::Column::SubjectKind.eq(kind))
            .filter(consent::Column::SubjectKey.eq(subject_key))
            .filter(consent::Column::Purpose.eq(key.purpose.as_str()))
            .filter(consent::Column::Scope.is_in(scopes))
            .filter(consent::Column::Channel.is_in(channels))
            .all(connection)
            .await?)
    }

    /// Đồng ý còn hiệu lực ở đúng khoá hay khoá rộng hơn, không bị rút ở khoá rộng hơn (hay bằng) sau đó.
    fn granted(rows: &[consent::Model]) -> bool {
        rows.iter().filter(|row| row.state == "granted").any(|granted| {
            !rows.iter().filter(|row| row.state == "withdrawn").any(|withdrawn| {
                (withdrawn.scope.is_empty() || withdrawn.scope == granted.scope)
                    && (withdrawn.channel.is_empty() || withdrawn.channel == granted.channel)
                    && withdrawn.withdrawn_at >= granted.granted_at
            })
        })
    }

    pub(crate) async fn is_granted(
        &self,
        tenant: &TenantContext,
        subject: &ConsentSubject,
        key: &ConsentKey,
    ) -> Result<bool, Error> {
        let rows = Self::candidates(tenant.database().read(), tenant, subject, key).await?;
        Ok(Self::granted(&rows))
    }

    pub(crate) async fn consent(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<Option<Consent>, Error> {
        Ok(consent::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .map(|row| describe_consent(&row)))
    }

    /// Đồng ý của người gọi.
    pub(crate) async fn my_consents(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
    ) -> Result<Vec<Consent>, Error> {
        let rows = consent::Entity::find()
            .scoped(tenant)
            .filter(consent::Column::SubjectKind.eq("user"))
            .filter(consent::Column::SubjectKey.eq(principal.user_id().to_string()))
            .order_by_asc(consent::Column::Purpose)
            .all(tenant.database().read())
            .await?;
        Ok(rows.iter().map(describe_consent).collect())
    }
}
