//! Văn bản pháp lý có phiên bản (Luật Giao dịch điện tử 20/2023 — cần đối chiếu): điều khoản, chính
//! sách dữ liệu cá nhân, điều kiện giao dịch chung, quy chế sàn, hợp đồng người bán… Mỗi lần phát hành
//! là một **ảnh chụp bất biến** (tiêu đề, nội dung, SHA-256); chấp nhận ghi ai, bản nào, lúc nào, kênh
//! nào, kèm SHA-256 của chính bản đã hiện. Bản đòi chấp nhận lại chặn ở `AcceptanceGate` tới khi chấp
//! nhận.

use autoxide_kernel::time::Timestamp;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{
    Order, Page, PageRequest, Pagination, SortValue, fetch_page, fetch_page_by, page_of,
    text_key_cursor,
};
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::OnConflict;
use sea_orm::sea_query::{Alias, Expr, Query};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
    QueryTrait as _, Set,
};
use serde_json::json;
use uuid::Uuid;

use crate::api::{DOCUMENT_KINDS, Subject};
use crate::entity::{acceptance, document, version};
use crate::events;
use crate::permissions::{DOCUMENTS_MANAGE, DOCUMENTS_READ};
use crate::service::{ComplianceService, now};

pub(crate) fn document_not_found() -> Error {
    Error::not_found("compliance.document_not_found", "There is no such legal document.")
}

fn version_not_found() -> Error {
    Error::not_found(
        "compliance.version_not_found",
        "There is no such version of a legal document.",
    )
}

fn acceptance_required(title: &str, version: i32) -> Error {
    Error::new(
        ErrorKind::Forbidden,
        "compliance.acceptance_required",
        "Accept {document}, version {version}, first.",
    )
    .with_param("document", title)
    .with_param("version", version)
}

/// Một phiên bản cần phát hành.
#[derive(Debug, Clone)]
pub(crate) struct NewVersion {
    pub locale: String,
    pub title: String,
    pub body: String,
    pub effective_from: Option<Timestamp>,
    pub requires_reacceptance: bool,
    /// Thêm bản dịch cho số phiên bản đã có.
    pub version: Option<i32>,
}

/// Một lần chấp nhận cần ghi.
#[derive(Debug, Clone)]
pub(crate) struct NewAcceptance {
    pub version_id: Uuid,
    pub channel: String,
    pub method: String,
    pub ip_fingerprint: Option<String>,
    pub user_agent: Option<String>,
}

/// Tên thứ tự trong con trỏ của số chấp nhận theo phiên bản.
const STATS_CURSOR: &str = "acceptance_stats:version";

fn invalid_cursor() -> Error {
    Error::validation(vec![FieldError::new(
        "cursor",
        "invalid",
        "must be the `next_cursor` value of a previous page of the same list and sort, unchanged",
    )])
}

impl ComplianceService {
    pub(crate) async fn create_document(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        kind: String,
        audience: String,
        title: String,
    ) -> Result<document::Model, Error> {
        principal.require(&DOCUMENTS_MANAGE)?;
        let mut problems = Vec::new();
        if !DOCUMENT_KINDS.contains(&kind.as_str()) {
            problems.push(
                FieldError::new("kind", "invalid", "must be one of {values}")
                    .with_param("values", DOCUMENT_KINDS.join(", ")),
            );
        }
        if !["user", "buyer", "seller"].contains(&audience.as_str()) {
            problems.push(FieldError::new(
                "audience",
                "invalid",
                "must be `user`, `buyer` or `seller`",
            ));
        }
        let title = title.trim().to_owned();
        if !(1..=200).contains(&title.chars().count()) {
            problems.push(FieldError::new("title", "length", "must be 1 to 200 characters long"));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let at = now();
        let inserted = document::Entity::insert(document::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            org_key: Set(String::new()),
            kind: Set(kind.clone()),
            audience: Set(audience),
            title: Set(title),
            current_version: Set(None),
            created_at: Set(at),
            updated_at: Set(at),
        })
        .on_conflict(
            OnConflict::columns([
                document::Column::TenantId,
                document::Column::OrgKey,
                document::Column::Kind,
            ])
            .do_nothing()
            .to_owned(),
        )
        .exec_without_returning(tenant.database().write())
        .await?;
        if inserted == 0 {
            return Err(Error::conflict(
                "compliance.document_exists",
                "The tenant already has a document of kind `{kind}`; publish a new version of it.",
            )
            .with_param("kind", kind));
        }
        Self::document_by_kind(tenant.database().write(), tenant, &kind)
            .await?
            .ok_or_else(|| Error::internal("a legal document vanished"))
    }

    async fn document_by_kind<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        kind: &str,
    ) -> Result<Option<document::Model>, Error> {
        Ok(document::Entity::find()
            .scoped(tenant)
            .filter(document::Column::OrgKey.eq(""))
            .filter(document::Column::Kind.eq(kind))
            .one(connection)
            .await?)
    }

    pub(crate) async fn list_documents(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<document::Model>, Error> {
        principal.require(&DOCUMENTS_READ)?;
        // Theo loại rồi id (một loại có thể có văn bản của từng nút), keyset trong database (index
        // `compliance_documents_page_idx`).
        fetch_page_by(
            document::Entity::find().scoped(tenant),
            document::Column::Kind,
            |row| SortValue::Text(row.kind.clone()),
            document::Column::Id,
            |row| row.id,
            page,
            tenant.database().read(),
        )
        .await
    }

    pub(crate) async fn document_for(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<document::Model, Error> {
        principal.require(&DOCUMENTS_READ)?;
        document::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(document_not_found)
    }

    pub(crate) async fn versions_of(
        &self,
        tenant: &TenantContext,
        document_id: Uuid,
    ) -> Result<Vec<version::Model>, Error> {
        Ok(version::Entity::find()
            .scoped(tenant)
            .filter(version::Column::DocumentId.eq(document_id))
            .order_by_desc(version::Column::Version)
            .order_by_asc(version::Column::Locale)
            .all(tenant.database().read())
            .await?)
    }

    /// Phát hành phiên bản mới (hay bản dịch của phiên bản đã có) của văn bản `id`.
    pub(crate) async fn publish_version(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        input: NewVersion,
    ) -> Result<version::Model, Error> {
        principal.require(&DOCUMENTS_MANAGE)?;
        let mut problems = Vec::new();
        let locale = input.locale.trim().to_owned();
        if !(2..=16).contains(&locale.len()) {
            problems.push(FieldError::new(
                "locale",
                "length",
                "must be a language code, e.g. `vi` or `en`",
            ));
        }
        let title = input.title.trim().to_owned();
        if !(1..=200).contains(&title.chars().count()) {
            problems.push(FieldError::new("title", "length", "must be 1 to 200 characters long"));
        }
        if input.body.trim().is_empty() || input.body.len() > 500_000 {
            problems.push(FieldError::new("body", "length", "must be 1 to 500000 bytes"));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let sha = autoxide_kernel::secret::sha256(&format!("{title}\n{}", input.body));
        let database = tenant.database();
        let (outbox, input, locale, title, sha) = (&self.outbox, &input, &locale, &title, &sha);
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let doc =
                    rows::lock_one(&transaction, document::Entity::find_by_id(id).scoped(tenant))
                        .await?
                        .ok_or_else(document_not_found)?;
                let latest = doc.current_version.unwrap_or(0);
                let number = match input.version {
                    Some(number) if number >= 1 && number <= latest => number,
                    Some(_) => {
                        return Err(Error::validation(vec![FieldError::new(
                            "version",
                            "range",
                            "must be a version already published, to add a translation",
                        )]));
                    }
                    None => latest + 1,
                };
                let at = now();
                let inserted = version::Entity::insert(version::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    document_id: Set(doc.id),
                    version: Set(number),
                    locale: Set(locale.clone()),
                    source: Set(String::new()),
                    title: Set(title.clone()),
                    body: Set(input.body.clone()),
                    sha256: Set(sha.clone()),
                    effective_from: Set(input.effective_from.unwrap_or(at)),
                    requires_reacceptance: Set(input.requires_reacceptance || number == 1),
                    published_by: Set(Some(principal.user_id())),
                    published_at: Set(at),
                })
                .on_conflict(
                    OnConflict::columns([
                        version::Column::TenantId,
                        version::Column::DocumentId,
                        version::Column::Version,
                        version::Column::Locale,
                    ])
                    .do_nothing()
                    .to_owned(),
                )
                .exec_without_returning(&transaction)
                .await?;
                if inserted == 0 {
                    return Err(Error::conflict(
                        "compliance.version_exists",
                        "Version {version} already has a `{locale}` text; versions never change.",
                    )
                    .with_param("version", number)
                    .with_param("locale", locale));
                }
                if number > latest {
                    let mut active: document::ActiveModel = doc.clone().into();
                    active.current_version = Set(Some(number));
                    active.updated_at = Set(at);
                    document::Entity::update(active).exec(&transaction).await?;
                }
                let saved = version::Entity::find()
                    .scoped(tenant)
                    .filter(version::Column::DocumentId.eq(doc.id))
                    .filter(version::Column::Version.eq(number))
                    .filter(version::Column::Locale.eq(locale.as_str()))
                    .one(&transaction)
                    .await?
                    .ok_or_else(|| Error::internal("a document version vanished"))?;
                let event = events::DocumentVersionPublished(json!({
                    "document_id": doc.id, "kind": doc.kind, "version": number,
                    "requires_reacceptance": saved.requires_reacceptance,
                }));
                outbox
                    .record(
                        &transaction,
                        tenant,
                        Actor::from(principal),
                        Some(format!("compliance.document:{}", doc.id)),
                        &event,
                    )
                    .await?;
                transaction.commit().await?;
                Ok(saved)
            })
            .await
    }

    /// Phiên bản mà đối tượng phải chấp nhận (bản mới nhất đòi chấp nhận lại, có hiệu lực tới `at`).
    async fn required_version<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        doc: &document::Model,
        at: Timestamp,
    ) -> Result<Option<i32>, Error> {
        let rows = version::Entity::find()
            .scoped(tenant)
            .filter(version::Column::DocumentId.eq(doc.id))
            .filter(version::Column::RequiresReacceptance.eq(true))
            .filter(version::Column::EffectiveFrom.lte(at))
            .order_by_desc(version::Column::Version)
            .one(connection)
            .await?;
        Ok(rows.map(|row| row.version))
    }

    /// Đối tượng đã chấp nhận phiên bản `required` (hay mới hơn) của văn bản `doc` chưa.
    async fn has_accepted<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        doc: &document::Model,
        subject: &Subject,
        required: i32,
    ) -> Result<bool, Error> {
        let (kind, id) = subject.columns();
        Ok(acceptance::Entity::find()
            .scoped(tenant)
            .filter(acceptance::Column::DocumentId.eq(doc.id))
            .filter(acceptance::Column::SubjectKind.eq(kind))
            .filter(acceptance::Column::SubjectId.eq(id))
            .filter(acceptance::Column::Version.gte(required))
            .one(connection)
            .await?
            .is_some())
    }

    /// Văn bản đối tượng còn phải chấp nhận: (văn bản, phiên bản phải chấp nhận).
    pub(crate) async fn pending_acceptances(
        &self,
        tenant: &TenantContext,
        subject: &Subject,
        page: &PageRequest,
    ) -> Result<Page<(document::Model, version::Model)>, Error> {
        let read = tenant.database().read();
        let documents = document::Entity::find()
            .scoped(tenant)
            .filter(document::Column::OrgKey.eq(""))
            .all(read)
            .await?;
        let at = Timestamp::now();
        let mut pending = Vec::new();
        for doc in documents {
            let Some(required) = Self::required_version(read, tenant, &doc, at).await? else {
                continue;
            };
            if Self::has_accepted(read, tenant, &doc, subject, required).await? {
                continue;
            }
            // Bản để chấp nhận: bản mới nhất đã có hiệu lực (có thể mới hơn bản bắt buộc).
            let current = version::Entity::find()
                .scoped(tenant)
                .filter(version::Column::DocumentId.eq(doc.id))
                .filter(version::Column::EffectiveFrom.lte(at))
                .order_by_desc(version::Column::Version)
                .one(read)
                .await?;
            if let Some(current) = current {
                pending.push((doc, current));
            }
        }
        // Một trang (`page_of`): việc "còn phải chấp nhận" tính trong code cho từng văn bản, và tập này bị chặn
        // theo cấu trúc — chỉ văn bản cấp khách (`org_key` rỗng), mỗi loại của `DOCUMENT_KINDS` tối đa một.
        page_of(pending, |(doc, _)| doc.kind.clone(), page)
    }

    /// Ghi chấp nhận (gọi lại vô hại).
    pub(crate) async fn accept(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        subject: &Subject,
        input: NewAcceptance,
    ) -> Result<acceptance::Model, Error> {
        if !["web", "mobile_app", "api", "mcp", "in_store"].contains(&input.channel.as_str()) {
            return Err(Error::validation(vec![FieldError::new(
                "channel",
                "invalid",
                "must be `web`, `mobile_app`, `api`, `mcp` or `in_store`",
            )]));
        }
        if !["checkbox", "click_wrap", "signature"].contains(&input.method.as_str()) {
            return Err(Error::validation(vec![FieldError::new(
                "method",
                "invalid",
                "must be `checkbox`, `click_wrap` or `signature`",
            )]));
        }
        let read = tenant.database().read();
        let found = version::Entity::find_by_id(input.version_id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(version_not_found)?;
        let (kind, id) = subject.columns();
        let transaction = tenant.database().begin_write().await?;
        let inserted = acceptance::Entity::insert(acceptance::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            document_id: Set(found.document_id),
            version: Set(found.version),
            version_id: Set(found.id),
            subject_kind: Set(kind.to_owned()),
            subject_id: Set(id.clone()),
            accepted_by: Set(actor.user_id),
            channel: Set(input.channel.clone()),
            method: Set(input.method.clone()),
            ip_fingerprint: Set(input.ip_fingerprint.clone()),
            user_agent: Set(input
                .user_agent
                .as_ref()
                .map(|agent| agent.chars().take(300).collect())),
            sha256: Set(found.sha256.clone()),
            accepted_at: Set(now()),
        })
        .on_conflict(
            OnConflict::columns([
                acceptance::Column::TenantId,
                acceptance::Column::VersionId,
                acceptance::Column::SubjectKind,
                acceptance::Column::SubjectId,
            ])
            .do_nothing()
            .to_owned(),
        )
        .exec_without_returning(&transaction)
        .await?;
        if inserted == 1 {
            let event = events::DocumentAccepted(json!({
                "document_id": found.document_id, "version": found.version, "subject_kind": kind,
                "subject_id": if kind == "email" { serde_json::Value::Null } else { json!(id) },
                "channel": input.channel,
            }));
            self.outbox
                .record(
                    &transaction,
                    tenant,
                    actor,
                    Some(format!("compliance.document:{}", found.document_id)),
                    &event,
                )
                .await?;
        }
        let saved = acceptance::Entity::find()
            .scoped(tenant)
            .filter(acceptance::Column::VersionId.eq(found.id))
            .filter(acceptance::Column::SubjectKind.eq(kind))
            .filter(acceptance::Column::SubjectId.eq(id))
            .one(&transaction)
            .await?
            .ok_or_else(|| Error::internal("an acceptance vanished"))?;
        transaction.commit().await?;
        Ok(saved)
    }

    /// `AcceptanceGate::require_accepted`.
    pub(crate) async fn require_accepted(
        &self,
        tenant: &TenantContext,
        subject: &Subject,
        kind: &str,
    ) -> Result<(), Error> {
        let read = tenant.database().read();
        let Some(doc) = Self::document_by_kind(read, tenant, kind).await? else { return Ok(()) };
        let Some(required) = Self::required_version(read, tenant, &doc, Timestamp::now()).await?
        else {
            return Ok(());
        };
        if Self::has_accepted(read, tenant, &doc, subject, required).await? {
            Ok(())
        } else {
            Err(acceptance_required(&doc.title, required))
        }
    }

    /// `AcceptanceGate::accept_current`.
    pub(crate) async fn accept_current(
        &self,
        tenant: &TenantContext,
        subject: &Subject,
        kind: &str,
        channel: &str,
    ) -> Result<Option<Uuid>, Error> {
        let read = tenant.database().read();
        let Some(doc) = Self::document_by_kind(read, tenant, kind).await? else { return Ok(None) };
        let current = version::Entity::find()
            .scoped(tenant)
            .filter(version::Column::DocumentId.eq(doc.id))
            .filter(version::Column::EffectiveFrom.lte(Timestamp::now()))
            .order_by_desc(version::Column::Version)
            .one(read)
            .await?;
        let Some(current) = current else { return Ok(None) };
        let input = NewAcceptance {
            version_id: current.id,
            channel: channel.to_owned(),
            method: "checkbox".to_owned(),
            ip_fingerprint: None,
            user_agent: None,
        };
        self.accept(tenant, Actor::system(), subject, input).await?;
        Ok(Some(current.id))
    }

    pub(crate) async fn list_acceptances(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        document_id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<acceptance::Model>, Error> {
        self.document_for(tenant, principal, document_id).await?;
        let select = acceptance::Entity::find()
            .scoped(tenant)
            .filter(acceptance::Column::DocumentId.eq(document_id));
        fetch_page(select, acceptance::Column::Id, |row| row.id, page, tenant.database().read())
            .await
    }

    /// Số đối tượng đã chấp nhận theo phiên bản.
    pub(crate) async fn acceptance_stats(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        document_id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<(i32, u64)>, Error> {
        self.document_for(tenant, principal, document_id).await?;
        // `GROUP BY version` trong SQL, keyset theo số phiên bản (khoá của nhóm) ngay trong `WHERE`: database chỉ
        // trả `limit + 1` nhóm (index `compliance_acceptances_stats_idx`).
        let after = page
            .text_key_after(STATS_CURSOR)?
            .map(|key| key.parse::<i32>().map_err(|_| invalid_cursor()))
            .transpose()?;
        let groups = acceptance::Entity::find()
            .scoped(tenant)
            .filter(acceptance::Column::DocumentId.eq(document_id))
            .select_only()
            .column(acceptance::Column::Version)
            .column_as(acceptance::Column::Id.count(), "accepted")
            .group_by(acceptance::Column::Version);
        let read = tenant.database().read();
        let total = if page.wants_total() {
            let count = Query::select()
                .expr(Expr::cust("COUNT(*)"))
                .from_subquery(groups.clone().into_query(), Alias::new("groups"))
                .to_owned();
            let total: i64 = match read.query_one(&count).await? {
                Some(row) => row.try_get_by_index(0)?,
                None => 0,
            };
            Some(u64::try_from(total).unwrap_or_default())
        } else {
            None
        };
        let ascending = page.order() == Order::Asc;
        let mut select = groups;
        if let Some(after) = after {
            select = select.filter(if ascending {
                acceptance::Column::Version.gt(after)
            } else {
                acceptance::Column::Version.lt(after)
            });
        }
        let select = if ascending {
            select.order_by_asc(acceptance::Column::Version)
        } else {
            select.order_by_desc(acceptance::Column::Version)
        };
        let mut rows: Vec<(i32, i64)> =
            select.limit(u64::from(page.limit()) + 1).into_tuple().all(read).await?;
        let has_more = rows.len() > page.limit() as usize;
        rows.truncate(page.limit() as usize);
        let next = has_more
            .then(|| rows.last())
            .flatten()
            .map(|(version, _)| text_key_cursor(STATS_CURSOR, &version.to_string()));
        Ok(Page {
            data: rows
                .into_iter()
                .map(|(version, accepted)| (version, u64::try_from(accepted).unwrap_or_default()))
                .collect(),
            pagination: Pagination::new(page.limit(), next).with_total(total),
        })
    }

    /// Bản hiện hành công khai của văn bản `kind` theo ngôn ngữ (không có bản ngôn ngữ đó: bản đầu
    /// tiên của phiên bản hiện hành).
    pub(crate) async fn public_document(
        &self,
        tenant: &TenantContext,
        kind: &str,
        locale: Option<&str>,
    ) -> Result<(document::Model, version::Model), Error> {
        let read = tenant.database().read();
        let doc =
            Self::document_by_kind(read, tenant, kind).await?.ok_or_else(document_not_found)?;
        let latest = version::Entity::find()
            .scoped(tenant)
            .filter(version::Column::DocumentId.eq(doc.id))
            .filter(version::Column::EffectiveFrom.lte(Timestamp::now()))
            .order_by_desc(version::Column::Version)
            .one(read)
            .await?
            .ok_or_else(document_not_found)?;
        let localized = match locale {
            Some(locale) => {
                version::Entity::find()
                    .scoped(tenant)
                    .filter(version::Column::DocumentId.eq(doc.id))
                    .filter(version::Column::Version.eq(latest.version))
                    .filter(version::Column::Locale.eq(locale))
                    .one(read)
                    .await?
            }
            None => None,
        };
        Ok((doc, localized.unwrap_or(latest)))
    }
}
