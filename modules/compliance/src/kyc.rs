//! Định danh (KYC, Luật TMĐT 122/2025 Điều 17 — cần đối chiếu): hồ sơ theo đối tượng (người dùng,
//! nút, email, mã của module gọi), lượt xác thực ở nhà cung cấp (VNeID, eKYC), callback có chữ ký, duyệt
//! tay, cổng chặn theo quy tắc `kyc.required_activities`, hết hạn.
//!
//! Lưu tối thiểu (Luật 91/2025): kết quả, mã tham chiếu, tên đã xác thực, băm + 4 số cuối của số giấy
//! tờ. Không lưu ảnh khuôn mặt, dữ liệu sinh trắc học, số giấy tờ đầy đủ.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::identity::{
    OutcomeStatus, PartyType, VerificationMethod, VerificationOutcome, VerificationRequest,
    VerificationSession, verification_disabled,
};
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, TenantId};
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{Activity, KycSession, KycStatus, Subject};
use crate::entity::{kyc_attempt, kyc_callback, kyc_profile};
use crate::events;
use crate::permissions::{KYC_READ, KYC_REVIEW, KYC_SUBMIT};
use crate::service::{ComplianceService, now, require_session};

pub(crate) const KYC_PROCESS: &str = "compliance.kyc_process";
pub(crate) const KYC_EXPIRY: &str = "compliance.kyc_expiry";

pub(crate) fn kyc_not_found() -> Error {
    Error::not_found("compliance.kyc_not_found", "There is no such identity verification profile.")
}

pub(crate) fn kyc_state_invalid(status: &str) -> Error {
    Error::conflict(
        "compliance.kyc_state_invalid",
        "The identity verification profile is `{status}`; this cannot be done now.",
    )
    .with_param("status", status)
}

fn kyc_required(activity: Activity, status: &str) -> Error {
    Error::new(
        ErrorKind::Forbidden,
        "compliance.kyc_required",
        "A verified identity is required to {activity}; the verification profile is `{profile_status}`.",
    )
    .with_param("activity", activity.as_str())
    .with_param("profile_status", status)
}

/// Job xử lý một callback đã lưu.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ProcessCallback {
    pub callback_id: Uuid,
}

impl Job for ProcessCallback {
    const KIND: &'static str = KYC_PROCESS;
}

/// Băm số giấy tờ đã chuẩn hoá (chỉ chữ và số, chữ hoa) và 4 ký tự cuối.
pub(crate) fn document_fingerprint(number: &str) -> (String, String) {
    let normalized: String = number
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let last4 = normalized.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    (autoxide_kernel::secret::sha256(&format!("compliance-document|{normalized}")), last4)
}

fn profile_payload(row: &kyc_profile::Model) -> Value {
    json!({
        "profile_id": row.id, "subject_kind": row.subject_kind, "subject_id": row.subject_id,
        "status": row.status, "reason_code": row.reason_code, "expires_at": row.expires_at,
    })
}

fn subject_of(row: &kyc_profile::Model) -> Option<String> {
    Some(format!("compliance.kyc_profile:{}", row.id))
}

impl ComplianceService {
    async fn find_profile<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        subject: &Subject,
    ) -> Result<Option<kyc_profile::Model>, Error> {
        let (kind, id) = subject.columns();
        Ok(kyc_profile::Entity::find()
            .scoped(tenant)
            .filter(kyc_profile::Column::SubjectKind.eq(kind))
            .filter(kyc_profile::Column::SubjectId.eq(id))
            .one(connection)
            .await?)
    }

    /// Hồ sơ của `subject`, mở (`draft`) nếu chưa có.
    pub(crate) async fn open_profile(
        &self,
        tenant: &TenantContext,
        subject: &Subject,
        party: PartyType,
    ) -> Result<kyc_profile::Model, Error> {
        if let Some(found) = Self::find_profile(tenant.database().read(), tenant, subject).await? {
            return Ok(found);
        }
        let (kind, id) = subject.columns();
        let at = now();
        kyc_profile::Entity::insert(kyc_profile::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            subject_kind: Set(kind.to_owned()),
            subject_id: Set(id),
            party_type: Set(party.as_str().to_owned()),
            status: Set("draft".to_owned()),
            method: Set(None),
            provider: Set(None),
            provider_reference: Set(None),
            verified_name: Set(None),
            document_kind: Set(None),
            document_hash: Set(None),
            document_last4: Set(None),
            assurance: Set(None),
            document_files: Set("[]".to_owned()),
            info_request: Set(None),
            verified_at: Set(None),
            expires_at: Set(None),
            reason_code: Set(None),
            reviewed_by: Set(None),
            review_note: Set(None),
            verified_document_hash: Set(None),
            birth_date: Set(None),
            address: Set(None),
            representative_name: Set(None),
            document_country: Set(None),
            document_expires_on: Set(None),
            created_at: Set(at),
            updated_at: Set(at),
        })
        .on_conflict(
            OnConflict::columns([
                kyc_profile::Column::TenantId,
                kyc_profile::Column::SubjectKind,
                kyc_profile::Column::SubjectId,
            ])
            .do_nothing()
            .to_owned(),
        )
        .exec_without_returning(tenant.database().write())
        .await?;
        Self::find_profile(tenant.database().write(), tenant, subject)
            .await?
            .ok_or_else(|| Error::internal("a KYC profile vanished"))
    }

    /// Bắt đầu một lượt xác thực ở nhà cung cấp cho hồ sơ `profile_id`.
    pub(crate) async fn start_session(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        profile_id: Uuid,
        method: Option<VerificationMethod>,
        return_url: Option<String>,
    ) -> Result<KycSession, Error> {
        let verifier = self.verifier.clone().ok_or_else(verification_disabled)?;
        let method = match method {
            Some(method) => method,
            None => *verifier.methods().first().ok_or_else(verification_disabled)?,
        };
        if !verifier.methods().contains(&method) || method == VerificationMethod::Manual {
            return Err(Error::validation(vec![FieldError::new(
                "method",
                "unsupported",
                "is not a method of the identity provider",
            )]));
        }
        let database = tenant.database();
        let attempt_id = Uuid::now_v7();
        let (outbox, provider) = (&self.outbox, verifier.provider());
        let profile = database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let row = rows::lock_one(
                    &transaction,
                    kyc_profile::Entity::find_by_id(profile_id).scoped(tenant),
                )
                .await?
                .ok_or_else(kyc_not_found)?;
                if matches!(row.status.as_str(), "verified" | "revoked") {
                    return Err(kyc_state_invalid(&row.status));
                }
                kyc_attempt::Entity::insert(kyc_attempt::ActiveModel {
                    id: Set(attempt_id),
                    tenant_id: Set(tenant.id().as_uuid()),
                    profile_id: Set(row.id),
                    method: Set(method.as_str().to_owned()),
                    provider: Set(provider.to_owned()),
                    provider_reference: Set(attempt_id.to_string()),
                    status: Set("pending".to_owned()),
                    result_code: Set(None),
                    created_at: Set(now()),
                    completed_at: Set(None),
                })
                .exec_without_returning(&transaction)
                .await?;
                let mut active: kyc_profile::ActiveModel = row.into();
                active.status = Set("pending".to_owned());
                active.method = Set(Some(method.as_str().to_owned()));
                active.provider = Set(Some(provider.to_owned()));
                active.info_request = Set(None);
                active.updated_at = Set(now());
                let saved = kyc_profile::Entity::update(active).exec(&transaction).await?;
                let event = events::KycSubmitted(profile_payload(&saved));
                outbox.record(&transaction, tenant, actor, subject_of(&saved), &event).await?;
                transaction.commit().await?;
                Ok(saved)
            })
            .await?;
        // Ngoài transaction: gọi nhà cung cấp.
        let party = PartyType::parse(&profile.party_type).unwrap_or(PartyType::Individual);
        let request =
            VerificationRequest::new(tenant.id(), attempt_id, party, method).return_url(return_url);
        let session = verifier.start(request).await?;
        let reference = session.provider_reference().to_owned();
        kyc_attempt::Entity::update_many()
            .scoped(tenant)
            .col_expr(kyc_attempt::Column::ProviderReference, reference.clone().into())
            .filter(kyc_attempt::Column::Id.eq(attempt_id))
            .exec(database.write())
            .await?;
        kyc_profile::Entity::update_many()
            .scoped(tenant)
            .col_expr(kyc_profile::Column::ProviderReference, Some(reference).into())
            .filter(kyc_profile::Column::Id.eq(profile.id))
            .exec(database.write())
            .await?;
        let (kind, url, token, expires_at) = match session {
            VerificationSession::Redirect { url, expires_at, .. } => {
                ("redirect", Some(url), None, Some(expires_at))
            }
            VerificationSession::Sdk { token, expires_at, .. } => {
                ("sdk", None, Some(token), Some(expires_at))
            }
            VerificationSession::Pending { .. } => ("pending", None, None, None),
        };
        Ok(KycSession {
            profile_id: profile.id,
            attempt_id,
            kind: kind.to_owned(),
            url,
            token,
            expires_at,
        })
    }

    /// Nhận callback của nhà cung cấp: kiểm chữ ký, lưu nguyên văn, xếp job xử lý.
    pub(crate) async fn receive_callback(
        &self,
        tenant: &TenantContext,
        provider: &str,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> Result<(), Error> {
        let verifier = self.verifier.clone().ok_or_else(verification_disabled)?;
        if verifier.provider() != provider {
            return Err(verification_disabled());
        }
        let outcome = verifier.verify_callback(headers, body).await?;
        let database = tenant.database();
        let transaction = database.begin_write().await?;
        let id = Uuid::now_v7();
        kyc_callback::Entity::insert(kyc_callback::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            provider: Set(provider.to_owned()),
            provider_reference: Set(outcome.provider_reference.clone()),
            body: Set(String::from_utf8_lossy(body).into_owned()),
            received_at: Set(now()),
            processed_at: Set(None),
        })
        .exec_without_returning(&transaction)
        .await?;
        self.jobs
            .enqueue(
                &transaction,
                tenant,
                &ProcessCallback { callback_id: id },
                EnqueueOptions::default(),
            )
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Xử lý callback đã lưu (job): hỏi lại nhà cung cấp kết quả của lượt rồi áp vào hồ sơ.
    pub(crate) async fn process_callback(
        &self,
        tenant: &TenantContext,
        callback_id: Uuid,
    ) -> Result<(), Error> {
        let database = tenant.database();
        let Some(callback) = kyc_callback::Entity::find_by_id(callback_id)
            .scoped(tenant)
            .one(database.write())
            .await?
        else {
            return Ok(());
        };
        if callback.processed_at.is_some() {
            return Ok(());
        }
        let verifier = self.verifier.clone().ok_or_else(verification_disabled)?;
        let outcome = verifier.fetch(&callback.provider_reference).await?;
        self.apply_outcome(tenant, &callback.provider, outcome).await?;
        kyc_callback::Entity::update_many()
            .scoped(tenant)
            .col_expr(kyc_callback::Column::ProcessedAt, Some(now()).into())
            .filter(kyc_callback::Column::Id.eq(callback_id))
            .exec(database.write())
            .await?;
        Ok(())
    }

    /// Áp kết quả của nhà cung cấp vào lượt thử và hồ sơ. Gọi lại vô hại.
    pub(crate) async fn apply_outcome(
        &self,
        tenant: &TenantContext,
        provider: &str,
        outcome: VerificationOutcome,
    ) -> Result<(), Error> {
        if outcome.status == OutcomeStatus::Pending {
            return Ok(());
        }
        let valid_days = self.kyc_valid_days(tenant).await?;
        let database = tenant.database();
        let (outbox, outcome) = (&self.outbox, &outcome);
        // Hai hồ sơ cùng một giấy tờ xác thực cùng lúc: bên sau vướng unique của
        // `verified_document_hash` (Postgres chờ bên trước commit rồi báo trùng) → chạy lại, lần này
        // thấy bên kia đã `verified` và chuyển hồ sơ sang chờ người duyệt (D-181).
        for _ in 0..3 {
            let applied =
                Self::apply_outcome_once(database, outbox, tenant, provider, outcome, valid_days)
                    .await?;
            if applied {
                return Ok(());
            }
        }
        Err(Error::internal(
            "a KYC outcome kept clashing with another profile of the same document",
        ))
    }

    /// Một lượt của [`Self::apply_outcome`]. `false`: vướng unique của giấy tờ, cần chạy lại.
    async fn apply_outcome_once(
        database: &autoxide_kernel::database::Database,
        outbox: &autoxide_kernel::event::Outbox,
        tenant: &TenantContext,
        provider: &str,
        outcome: &VerificationOutcome,
        valid_days: i64,
    ) -> Result<bool, Error> {
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let Some(attempt) = kyc_attempt::Entity::find()
                    .scoped(tenant)
                    .filter(kyc_attempt::Column::Provider.eq(provider))
                    .filter(
                        kyc_attempt::Column::ProviderReference
                            .eq(outcome.provider_reference.as_str()),
                    )
                    .one(&transaction)
                    .await?
                else {
                    return Ok(true);
                };
                if attempt.status != "pending" {
                    return Ok(true);
                }
                let profile = rows::lock_one(
                    &transaction,
                    kyc_profile::Entity::find_by_id(attempt.profile_id).scoped(tenant),
                )
                .await?
                .ok_or_else(kyc_not_found)?;
                // Đọc lại lượt xác thực sau khi khoá hồ sơ (D-181): hai callback cùng lượt chạy cùng
                // lúc thì bên sau chờ khoá, rồi thấy lượt đã xong và thôi — không thì Postgres cho
                // cả hai cùng thấy `pending` ở câu đọc trên, cùng ghi và phát event hai lần.
                let still_pending = kyc_attempt::Entity::find_by_id(attempt.id)
                    .scoped(tenant)
                    .one(&transaction)
                    .await?
                    .is_some_and(|row| row.status == "pending");
                if !still_pending {
                    return Ok(true);
                }
                let at = now();
                // Kết quả muộn của lượt cũ: hồ sơ không còn chờ lượt này (đã duyệt tay, bị thu hồi,
                // từ chối, hay đã mở lượt mới) thì chỉ ghi lượt là `superseded`, không đụng hồ sơ —
                // trước đây nó xác thực lại hồ sơ vừa bị thu hồi (cả hai DB).
                let latest = kyc_attempt::Entity::find()
                    .scoped(tenant)
                    .filter(kyc_attempt::Column::ProfileId.eq(profile.id))
                    .order_by_desc(kyc_attempt::Column::Id)
                    .one(&transaction)
                    .await?
                    .map(|row| row.id);
                if profile.status != "pending" || latest != Some(attempt.id) {
                    let result_code = match &outcome.status {
                        OutcomeStatus::Rejected { reason_code } => Some(reason_code.clone()),
                        _ => None,
                    };
                    kyc_attempt::Entity::update_many()
                        .scoped(tenant)
                        .col_expr(kyc_attempt::Column::Status, "superseded".into())
                        .col_expr(kyc_attempt::Column::ResultCode, result_code.into())
                        .col_expr(kyc_attempt::Column::CompletedAt, Some(at).into())
                        .filter(kyc_attempt::Column::Id.eq(attempt.id))
                        .exec(&transaction)
                        .await?;
                    transaction.commit().await?;
                    return Ok(true);
                }
                let mut active: kyc_profile::ActiveModel = profile.clone().into();
                let (attempt_status, result_code) = match &outcome.status {
                    OutcomeStatus::Verified => {
                        let fingerprint =
                            outcome.document_number.as_deref().map(document_fingerprint);
                        // Cùng giấy tờ đã xác thực cho đối tượng khác: chờ người duyệt.
                        let duplicate = match &fingerprint {
                            Some((hash, _)) => kyc_profile::Entity::find()
                                .scoped(tenant)
                                .filter(kyc_profile::Column::DocumentHash.eq(hash.as_str()))
                                .filter(kyc_profile::Column::Status.eq("verified"))
                                .filter(kyc_profile::Column::Id.ne(profile.id))
                                .one(&transaction)
                                .await?
                                .is_some(),
                            None => false,
                        };
                        active.verified_name = Set(outcome.verified_name.clone());
                        active.document_kind = Set(outcome.document_kind.clone());
                        let hash = fingerprint.as_ref().map(|(hash, _)| hash.clone());
                        active.document_hash = Set(hash.clone());
                        active.verified_document_hash = Set(if duplicate { None } else { hash });
                        active.document_last4 = Set(fingerprint.map(|(_, last4)| last4));
                        active.assurance = Set(outcome.assurance.clone());
                        // Mục NĐ 248/2026 Điều 18 nhà cung cấp trả (LG-3c): ghi đè khi có, giữ khi không.
                        let date = |value: Option<autoxide_kernel::time::Date>| {
                            value.map(|date| date.format("%Y-%m-%d"))
                        };
                        let details = [
                            (date(outcome.birth_date), profile.birth_date.clone()),
                            (outcome.address.clone(), profile.address.clone()),
                            (
                                outcome.representative_name.clone(),
                                profile.representative_name.clone(),
                            ),
                            (outcome.document_country.clone(), profile.document_country.clone()),
                            (
                                date(outcome.document_expires_on),
                                profile.document_expires_on.clone(),
                            ),
                        ]
                        .map(|(new, old)| new.or(old));
                        let [birth_date, address, representative, country, document_expires_on] =
                            details;
                        active.birth_date = Set(birth_date);
                        active.address = Set(address);
                        active.representative_name = Set(representative);
                        active.document_country = Set(country);
                        active.document_expires_on = Set(document_expires_on.clone());
                        if duplicate {
                            active.status = Set("pending".to_owned());
                            active.reason_code = Set(Some("duplicate_document".to_owned()));
                        } else {
                            active.status = Set("verified".to_owned());
                            active.reason_code = Set(None);
                            active.verified_at = Set(Some(at));
                            // Hồ sơ không sống lâu hơn giấy tờ đã dùng để xác thực.
                            active.expires_at =
                                Set(crate::kyc_details::capped_expiry(
                                    Some(outcome.expires_at.unwrap_or(
                                        at + SignedDuration::from_hours(24 * valid_days),
                                    )),
                                    document_expires_on.as_deref(),
                                ));
                        }
                        ("verified", None)
                    }
                    OutcomeStatus::Rejected { reason_code } => {
                        active.status = Set("rejected".to_owned());
                        active.reason_code = Set(Some(reason_code.clone()));
                        ("rejected", Some(reason_code.clone()))
                    }
                    OutcomeStatus::Pending => return Ok(true),
                };
                active.updated_at = Set(at);
                let saved = match kyc_profile::Entity::update(active).exec(&transaction).await {
                    Ok(saved) => saved,
                    Err(error)
                        if matches!(
                            error.sql_err(),
                            Some(sea_orm::SqlErr::UniqueConstraintViolation(_))
                        ) =>
                    {
                        return Ok(false);
                    }
                    Err(error) => return Err(error.into()),
                };
                kyc_attempt::Entity::update_many()
                    .scoped(tenant)
                    .col_expr(kyc_attempt::Column::Status, attempt_status.into())
                    .col_expr(kyc_attempt::Column::ResultCode, result_code.into())
                    .col_expr(kyc_attempt::Column::CompletedAt, Some(at).into())
                    .filter(kyc_attempt::Column::Id.eq(attempt.id))
                    .exec(&transaction)
                    .await?;
                match saved.status.as_str() {
                    "verified" => {
                        let event = events::KycVerified(profile_payload(&saved));
                        outbox
                            .record(
                                &transaction,
                                tenant,
                                Actor::system(),
                                subject_of(&saved),
                                &event,
                            )
                            .await?;
                    }
                    "rejected" => {
                        let event = events::KycRejected(profile_payload(&saved));
                        outbox
                            .record(
                                &transaction,
                                tenant,
                                Actor::system(),
                                subject_of(&saved),
                                &event,
                            )
                            .await?;
                    }
                    _ => {}
                }
                transaction.commit().await?;
                Ok(true)
            })
            .await
    }

    // ---- Duyệt tay ---------------------------------------------------------------------------

    /// Gửi giấy tờ (ID file của module `files`) để người duyệt xem — cách xác thực tay.
    pub(crate) async fn submit_documents(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        profile_id: Uuid,
        files: Vec<Uuid>,
    ) -> Result<kyc_profile::Model, Error> {
        if files.is_empty() || files.len() > 10 {
            return Err(Error::validation(vec![FieldError::new(
                "file_ids",
                "count",
                "must have 1 to 10 files",
            )]));
        }
        let database = tenant.database();
        let outbox = &self.outbox;
        let files = serde_json::to_string(&files).map_err(Error::internal)?;
        let files = &files;
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let row = rows::lock_one(
                    &transaction,
                    kyc_profile::Entity::find_by_id(profile_id).scoped(tenant),
                )
                .await?
                .ok_or_else(kyc_not_found)?;
                if matches!(row.status.as_str(), "verified" | "revoked") {
                    return Err(kyc_state_invalid(&row.status));
                }
                let mut active: kyc_profile::ActiveModel = row.into();
                active.status = Set("pending".to_owned());
                active.method = Set(Some("manual".to_owned()));
                active.document_files = Set(files.clone());
                active.info_request = Set(None);
                active.updated_at = Set(now());
                let saved = kyc_profile::Entity::update(active).exec(&transaction).await?;
                let event = events::KycSubmitted(profile_payload(&saved));
                outbox.record(&transaction, tenant, actor, subject_of(&saved), &event).await?;
                transaction.commit().await?;
                Ok(saved)
            })
            .await
    }

    /// Người duyệt quyết định hồ sơ `id` (`approve` / `reject` / `revoke` / `request_info`).
    pub(crate) async fn review(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        decision: Decision,
    ) -> Result<kyc_profile::Model, Error> {
        principal.require(&KYC_REVIEW)?;
        require_session(principal)?;
        let valid_days = self.kyc_valid_days(tenant).await?;
        let database = tenant.database();
        let (outbox, decision) = (&self.outbox, &decision);
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let row = rows::lock_one(
                    &transaction,
                    kyc_profile::Entity::find_by_id(id).scoped(tenant),
                )
                .await?
                .ok_or_else(kyc_not_found)?;
                let at = now();
                let mut active: kyc_profile::ActiveModel = row.clone().into();
                active.reviewed_by = Set(Some(principal.user_id()));
                active.updated_at = Set(at);
                let allowed = match decision {
                    Decision::Approve => {
                        matches!(row.status.as_str(), "pending" | "needs_info" | "draft")
                    }
                    Decision::Reject(..) => {
                        matches!(row.status.as_str(), "pending" | "needs_info" | "draft")
                    }
                    Decision::Revoke(..) => row.status == "verified",
                    Decision::RequestInfo { .. } => {
                        matches!(row.status.as_str(), "pending" | "draft")
                    }
                };
                if !allowed {
                    return Err(kyc_state_invalid(&row.status));
                }
                match decision {
                    Decision::Approve => {
                        // Duyệt tay không nhận khoá giấy tờ (`verified_document_hash`): người duyệt
                        // được quyết cả hồ sơ trùng giấy tờ (`duplicate_document`).
                        active.status = Set("verified".to_owned());
                        active.reason_code = Set(None);
                        active.review_note = Set(None);
                        active.verified_at = Set(Some(at));
                        active.expires_at = Set(crate::kyc_details::capped_expiry(
                            Some(at + SignedDuration::from_hours(24 * valid_days)),
                            row.document_expires_on.as_deref(),
                        ));
                        if row.method.is_none() {
                            active.method = Set(Some("manual".to_owned()));
                        }
                    }
                    Decision::Reject(reason, note) => {
                        active.status = Set("rejected".to_owned());
                        active.reason_code = Set(Some(reason.clone()));
                        active.review_note = Set(note.clone());
                    }
                    Decision::Revoke(reason, note) => {
                        active.status = Set("revoked".to_owned());
                        active.verified_document_hash = Set(None);
                        active.reason_code = Set(Some(reason.clone()));
                        active.review_note = Set(note.clone());
                    }
                    Decision::RequestInfo { fields, message } => {
                        active.status = Set("needs_info".to_owned());
                        active.info_request =
                            Set(Some(json!({"fields": fields, "message": message}).to_string()));
                    }
                }
                let saved = kyc_profile::Entity::update(active).exec(&transaction).await?;
                let actor = Actor::from(principal);
                let subject = subject_of(&saved);
                let payload = profile_payload(&saved);
                match decision {
                    Decision::Approve => {
                        outbox
                            .record(
                                &transaction,
                                tenant,
                                actor,
                                subject,
                                &events::KycVerified(payload),
                            )
                            .await?
                    }
                    Decision::Reject(..) => {
                        outbox
                            .record(
                                &transaction,
                                tenant,
                                actor,
                                subject,
                                &events::KycRejected(payload),
                            )
                            .await?
                    }
                    Decision::Revoke(..) => {
                        outbox
                            .record(
                                &transaction,
                                tenant,
                                actor,
                                subject,
                                &events::KycRevoked(payload),
                            )
                            .await?
                    }
                    Decision::RequestInfo { .. } => {
                        outbox
                            .record(
                                &transaction,
                                tenant,
                                actor,
                                subject,
                                &events::KycInfoRequested(payload),
                            )
                            .await?;
                    }
                }
                transaction.commit().await?;
                Ok(saved)
            })
            .await
    }

    // ---- Đọc ---------------------------------------------------------------------------------

    pub(crate) async fn profile_for(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<kyc_profile::Model, Error> {
        principal.require(&KYC_READ)?;
        kyc_profile::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(kyc_not_found)
    }

    /// Tên chủ thể trước khi xác minh, theo lô qua sổ đối tượng của kernel: tài khoản (`auth.user`)
    /// và nút tổ chức (`organizations.unit`). Loại chưa có bộ tra hay lỗi thì bỏ qua — tên chỉ để
    /// hiển thị, không chặn danh sách.
    pub(crate) async fn subject_names(
        &self,
        tenant: &TenantContext,
        rows: &[kyc_profile::Model],
    ) -> std::collections::HashMap<(String, String), String> {
        let mut names = std::collections::HashMap::new();
        for (subject_kind, resource_kind) in [("user", "auth.user"), ("org", "organizations.unit")]
        {
            let ids: Vec<String> = rows
                .iter()
                .filter(|row| row.subject_kind == subject_kind)
                .map(|row| row.subject_id.clone())
                .collect();
            if ids.is_empty() || !self.resources.knows(resource_kind) {
                continue;
            }
            match self.resources.resolve_many(tenant, resource_kind, &ids, None).await {
                Ok(found) => {
                    for (id, resource) in found {
                        names.insert((subject_kind.to_owned(), id), resource.title);
                    }
                }
                Err(error) => {
                    tracing::warn!(kind = resource_kind, error = %error, "cannot read subject names");
                }
            }
        }
        names
    }

    pub(crate) async fn list_profiles(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        status: Option<String>,
        page: &PageRequest,
    ) -> Result<Page<kyc_profile::Model>, Error> {
        principal.require(&KYC_READ)?;
        let mut select = kyc_profile::Entity::find().scoped(tenant);
        if let Some(status) = status {
            select = select.filter(kyc_profile::Column::Status.eq(status));
        }
        fetch_page(select, kyc_profile::Column::Id, |row| row.id, page, tenant.database().read())
            .await
    }

    /// Hồ sơ của nút `org` (mở nếu chưa có) — cần `compliance.kyc.submit` ở nút đó.
    pub(crate) async fn open_org_profile(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: OrgId,
        party: PartyType,
    ) -> Result<kyc_profile::Model, Error> {
        principal.require(&KYC_SUBMIT)?;
        principal
            .require_at(
                tenant.database().read(),
                &KYC_SUBMIT,
                autoxide_kernel::access::OrgTarget::new(org, None),
            )
            .await?;
        self.open_profile(tenant, &Subject::Org(org), party).await
    }

    /// Hồ sơ `id` mà người gọi được nộp: hồ sơ của chính họ, hay của nút họ có `compliance.kyc.submit`.
    pub(crate) async fn submittable(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<kyc_profile::Model, Error> {
        let row = kyc_profile::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(kyc_not_found)?;
        match row.subject_kind.as_str() {
            "user" if row.subject_id == principal.user_id().to_string() => Ok(row),
            "org" => {
                principal.require(&KYC_SUBMIT)?;
                let org = row
                    .subject_id
                    .parse::<Uuid>()
                    .map(OrgId::from_uuid)
                    .map_err(|_| kyc_not_found())?;
                principal
                    .require_at(
                        tenant.database().read(),
                        &KYC_SUBMIT,
                        autoxide_kernel::access::OrgTarget::new(org, None),
                    )
                    .await
                    .map_err(|error| {
                        if error.code() == "org_out_of_scope" { kyc_not_found() } else { error }
                    })?;
                Ok(row)
            }
            _ => Err(kyc_not_found()),
        }
    }

    /// Trạng thái và hồ sơ định danh của chính người gọi.
    pub(crate) async fn my_kyc(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
    ) -> Result<(String, Option<kyc_profile::Model>), Error> {
        let row = Self::find_profile(
            tenant.database().read(),
            tenant,
            &Subject::User(principal.user_id()),
        )
        .await?;
        let status = self.kyc_status(tenant, &Subject::User(principal.user_id())).await?;
        Ok((status.status, row))
    }

    // ---- Cổng ------------------------------------------------------------------------------

    /// Thân quy tắc `kyc.required_activities` đang có hiệu lực; `None` khi vùng pháp lý không có quy tắc này
    /// (không bắt buộc định danh). Thân không phải JSON (quy tắc nhập hỏng) là lỗi, không coi là "không bắt
    /// buộc": cổng không được mở khi không đọc được luật (fail-closed, luật số hai).
    async fn kyc_rule(&self, tenant: &TenantContext) -> Result<Option<Value>, Error> {
        let Some(row) = self.rule_for(tenant, "kyc.required_activities", Timestamp::now()).await?
        else {
            return Ok(None);
        };
        serde_json::from_str::<Value>(&row.body).map(Some).map_err(|error| {
            tracing::error!(rule_version = row.version, %error, "the kyc.required_activities rule body is not JSON");
            Error::internal(format!(
                "the rule kyc.required_activities version {} is not valid JSON: {error}",
                row.version
            ))
        })
    }

    /// Số ngày một định danh còn hiệu lực (`valid_days` của quy tắc, mặc định 730 khi quy tắc không ghi).
    async fn kyc_valid_days(&self, tenant: &TenantContext) -> Result<i64, Error> {
        Ok(self
            .kyc_rule(tenant)
            .await?
            .and_then(|body| body["valid_days"].as_i64())
            .filter(|days| *days > 0)
            .unwrap_or(730))
    }

    pub(crate) async fn kyc_status(
        &self,
        tenant: &TenantContext,
        subject: &Subject,
    ) -> Result<KycStatus, Error> {
        Ok(match Self::find_profile(tenant.database().read(), tenant, subject).await? {
            Some(row) => {
                let expired = row.status == "verified"
                    && row.expires_at.is_some_and(|at| at <= Timestamp::now());
                KycStatus {
                    status: if expired { "expired".to_owned() } else { row.status },
                    profile_id: Some(row.id),
                    verified_at: row.verified_at,
                    expires_at: row.expires_at,
                }
            }
            None => KycStatus {
                status: "none".to_owned(),
                profile_id: None,
                verified_at: None,
                expires_at: None,
            },
        })
    }

    pub(crate) async fn require_verified(
        &self,
        tenant: &TenantContext,
        subject: &Subject,
        activity: Activity,
    ) -> Result<(), Error> {
        let Some(rule) = self.kyc_rule(tenant).await? else { return Ok(()) };
        if rule[activity.as_str()].as_bool() != Some(true) {
            return Ok(());
        }
        let status = self.kyc_status(tenant, subject).await?;
        if status.status != "verified" {
            return Err(kyc_required(activity, &status.status));
        }
        if activity != Activity::Sell {
            return Ok(());
        }
        // Bán: hồ sơ phải có đủ mục NĐ 248/2026 Điều 18 mà quy tắc đang có hiệu lực đòi (LG-3c).
        let row = Self::find_profile(tenant.database().read(), tenant, subject)
            .await?
            .ok_or_else(kyc_not_found)?;
        let missing = crate::kyc_details::missing_for_sale(&rule, &row);
        if missing.is_empty() {
            Ok(())
        } else {
            Err(kyc_required(activity, "incomplete").with_param("missing", missing.join(", ")))
        }
    }

    /// Hồ sơ đã xác thực quá hạn → `expired` (job hằng ngày, mọi database).
    pub(crate) async fn expire_profiles(
        &self,
        database: &autoxide_kernel::database::Database,
    ) -> Result<u64, Error> {
        let due: Vec<(Uuid, Uuid)> = kyc_profile::Entity::find()
            .select_only()
            .column(kyc_profile::Column::TenantId)
            .column(kyc_profile::Column::Id)
            .filter(kyc_profile::Column::Status.eq("verified"))
            .filter(kyc_profile::Column::ExpiresAt.lt(Timestamp::now()))
            .limit(500)
            .into_tuple()
            .all(database.read())
            .await?;
        let mut expired = 0;
        for (tenant, id) in due {
            let tenant = self.tenant(TenantId::from_uuid(tenant)).await?;
            let transaction = tenant.database().begin_write().await?;
            let Some(row) =
                rows::lock_one(&transaction, kyc_profile::Entity::find_by_id(id).scoped(&tenant))
                    .await?
            else {
                continue;
            };
            if row.status != "verified" {
                continue;
            }
            let mut active: kyc_profile::ActiveModel = row.into();
            active.status = Set("expired".to_owned());
            active.verified_document_hash = Set(None);
            active.updated_at = Set(now());
            let saved = kyc_profile::Entity::update(active).exec(&transaction).await?;
            let event = events::KycExpired(profile_payload(&saved));
            self.outbox
                .record(&transaction, &tenant, Actor::system(), subject_of(&saved), &event)
                .await?;
            transaction.commit().await?;
            expired += 1;
        }
        // Callback cũ quá hạn lưu: xoá.
        let before: Timestamp =
            Timestamp::now() - SignedDuration::from_hours(24 * self.callback_retention_days);
        // tenant-scope: việc nền / kiểm tra quét mọi khách của database, chủ ý không lọc khách (rà soát bảo mật 2026-11)
        kyc_callback::Entity::delete_many()
            .filter(kyc_callback::Column::ReceivedAt.lt(before))
            .filter(kyc_callback::Column::ProcessedAt.is_not_null())
            .exec(database.write())
            .await?;
        Ok(expired)
    }
}

/// Quyết định của người duyệt.
#[derive(Debug, Clone)]
pub(crate) enum Decision {
    Approve,
    /// Mã lý do, ghi chú cho người được duyệt.
    Reject(String, Option<String>),
    Revoke(String, Option<String>),
    RequestInfo {
        fields: Vec<String>,
        message: String,
    },
}

/// Job `compliance.kyc_process`.
pub(crate) struct ProcessJob(pub Arc<ComplianceService>);

#[async_trait::async_trait]
impl JobHandler for ProcessJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let payload: ProcessCallback =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        let tenant = job
            .tenant
            .as_ref()
            .ok_or_else(|| Error::internal("compliance.kyc_process needs a tenant"))?;
        self.0.process_callback(tenant, payload.callback_id).await
    }
}

/// Lịch `compliance.kyc_expiry` (hằng ngày, mọi database).
pub(crate) struct ExpiryJob(pub Arc<ComplianceService>);

#[async_trait::async_trait]
impl JobHandler for ExpiryJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.expire_profiles(&job.database).await?;
        // Mã một lần xác thực số điện thoại đã quá hạn giữ: xoá cùng lượt hằng ngày.
        self.0.purge_challenges(&job.database).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bam_so_giay_to_khong_phu_thuoc_dinh_dang() {
        let (a, last4) = document_fingerprint("001 099-012345");
        let (b, _) = document_fingerprint("001099012345");
        assert_eq!(a, b);
        assert_eq!(last4, "2345");
        assert_ne!(a, document_fingerprint("001099012346").0);
    }
}
