//! Đăng ký bản tin trên sổ đồng ý chung của `compliance` (bước 37 C3, D-347, D-367).
//!
//! - Mỗi đăng ký `confirmed` có một đồng ý trong sổ: chủ thể là email, mục đích `marketing`, phạm vi
//!   `newsletter.list:<id>`, kênh `email`, bằng chứng là JSON `consent` module đã lưu. Huỷ đăng ký rút
//!   đồng ý đó. Cột `ledger_state` (`''`, `granted`, `withdrawn`) nhớ sổ đã biết tới đâu.
//! - Đồng bộ chạy SAU khi transaction của module đã commit (người đăng ký và danh sách chỉ biết chắc
//!   trong transaction): ngay sau mỗi thay đổi, cộng lịch hằng giờ `newsletter.backfill_consents` cho
//!   phần còn sót (đăng ký có sẵn trước bước này, lần gọi sổ bị lỗi). An toàn vì lúc gửi luôn hỏi sổ
//!   (`granted_many`): đăng ký chưa có trong sổ thì được ghi sổ ngay trước khi hỏi, đồng ý bị rút ở
//!   khoá rộng hơn (vd mọi quảng cáo) vẫn chặn — sổ lỗi thì không gửi.
//! - Rút đồng ý ở `compliance` (event `compliance.consent_withdrawn`) huỷ các đăng ký khớp, không
//!   gọi lại sổ (tránh vòng event).

use std::sync::Arc;

use autoxide_compliance::api::{ConsentEvidence, ConsentKey, ConsentSubject};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Delivered, EventHandler};
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext, TenantId};
use sea_orm::sea_query::{Condition, Expr};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde_json::Value;
use uuid::Uuid;

use crate::audience::subscribers::SubscriptionStatus;
use crate::entity::{subscriber, subscription};
use crate::service::{NewsletterService, now};

/// Lịch đồng bộ phần còn sót.
pub(crate) const BACKFILL: &str = "newsletter.backfill_consents";
/// Số đăng ký mỗi lượt đồng bộ.
const BATCH: u64 = 500;

pub(crate) const GRANTED: &str = "granted";
pub(crate) const WITHDRAWN: &str = "withdrawn";

/// Khoá trong sổ của đăng ký vào danh sách `list_id`.
pub(crate) fn consent_key(list_id: Uuid) -> ConsentKey {
    ConsentKey::new("marketing", format!("newsletter.list:{list_id}"), "email")
}

/// Đăng ký cần đồng bộ: `confirmed` mà sổ chưa biết, hay đã huỷ mà sổ còn ghi đồng ý.
fn needs_sync() -> Condition {
    Condition::any()
        .add(
            Condition::all()
                .add(subscription::Column::Status.eq(SubscriptionStatus::Confirmed.as_str()))
                .add(subscription::Column::LedgerState.ne(GRANTED)),
        )
        .add(
            Condition::all()
                .add(subscription::Column::Status.eq(SubscriptionStatus::Unsubscribed.as_str()))
                .add(subscription::Column::LedgerState.eq(GRANTED)),
        )
}

/// Bằng chứng từ JSON `consent` module đã lưu (`{text, version, source, ip_fingerprint}`). Không có
/// (đăng ký cũ, nhập không nguồn) thì ghi rõ là ghi sổ từ đăng ký đã xác nhận có sẵn.
fn evidence_of(stored: Option<&str>) -> ConsentEvidence {
    let value: Value = stored.and_then(|raw| serde_json::from_str(raw).ok()).unwrap_or(Value::Null);
    let text = |key: &str| {
        value.get(key).and_then(Value::as_str).map(str::trim).filter(|text| !text.is_empty())
    };
    ConsentEvidence {
        text: text("text")
            .unwrap_or(
                "Recorded by the newsletter module from a confirmed subscription that stored no \
                 consent text.",
            )
            .to_owned(),
        text_version: text("version").unwrap_or("newsletter.legacy").to_owned(),
        source: text("source")
            .map_or_else(|| "newsletter".to_owned(), |source| format!("newsletter:{source}")),
        ip_fingerprint: text("ip_fingerprint").map(str::to_owned),
    }
}

impl NewsletterService {
    /// Đồng bộ các đăng ký `rows` (của người có `email`) lên sổ; mỗi dòng xong thì ghi
    /// `ledger_state` (chỉ khi trạng thái chưa đổi từ lúc đọc).
    async fn push_to_ledger(
        &self,
        database: &Database,
        tenant: TenantId,
        email: &str,
        rows: Vec<subscription::Model>,
    ) -> Result<u64, Error> {
        let Some(ledger) = &self.consents else {
            return Ok(0);
        };
        let mut synced = 0;
        for row in rows {
            let subject = ConsentSubject::Email(email.to_owned());
            let key = consent_key(row.list_id);
            let state = if row.status == SubscriptionStatus::Confirmed.as_str() {
                let at = row.confirmed_at.map_or(0, |at| at.timestamp_micros());
                let source_ref =
                    format!("newsletter.subscription:{}:{}:{at}", row.subscriber_id, row.list_id);
                ledger
                    .grant(tenant, subject, key, evidence_of(row.consent.as_deref()), &source_ref)
                    .await?;
                GRANTED
            } else {
                let at = row.unsubscribed_at.map_or(0, |at| at.timestamp_micros());
                let source_ref =
                    format!("newsletter.unsubscribe:{}:{}:{at}", row.subscriber_id, row.list_id);
                ledger.withdraw(tenant, subject, key, "newsletter", &source_ref).await?;
                WITHDRAWN
            };
            synced += subscription::Entity::update_many()
                .col_expr(subscription::Column::LedgerState, Expr::value(state))
                .filter(subscription::Column::TenantId.eq(tenant.as_uuid()))
                .filter(subscription::Column::SubscriberId.eq(row.subscriber_id))
                .filter(subscription::Column::ListId.eq(row.list_id))
                .filter(subscription::Column::Status.eq(row.status.as_str()))
                .exec(database.write())
                .await?
                .rows_affected;
        }
        Ok(synced)
    }

    /// Đồng bộ mọi đăng ký cần đồng bộ của người đăng ký `subscriber_id` (gọi sau commit).
    pub(crate) async fn sync_consents(
        &self,
        tenant: &TenantContext,
        subscriber_id: Uuid,
    ) -> Result<u64, Error> {
        if self.consents.is_none() {
            return Ok(0);
        }
        let read = tenant.database().read();
        let Some(person) =
            subscriber::Entity::find_by_id(subscriber_id).scoped(tenant).one(read).await?
        else {
            return Ok(0);
        };
        let rows = subscription::Entity::find()
            .scoped(tenant)
            .filter(subscription::Column::SubscriberId.eq(subscriber_id))
            .filter(needs_sync())
            .all(read)
            .await?;
        self.push_to_ledger(tenant.database(), tenant.id(), &person.email, rows).await
    }

    /// Như [`sync_consents`](Self::sync_consents) nhưng lỗi chỉ ghi log: lịch hằng giờ làm lại.
    pub(crate) async fn sync_consents_quietly(&self, tenant: &TenantContext, subscriber_id: Uuid) {
        if let Err(error) = self.sync_consents(tenant, subscriber_id).await {
            tracing::warn!(subscriber_id = %subscriber_id, code = error.code(), "newsletter consent not recorded yet; the hourly backfill retries");
        }
    }

    /// Một lượt đồng bộ phần còn sót trên cả `database` (mọi khách): tối đa [`BATCH`] đăng ký.
    pub(crate) async fn backfill_consents(&self, database: &Database) -> Result<u64, Error> {
        if self.consents.is_none() {
            return Ok(0);
        }
        let rows = subscription::Entity::find()
            .filter(needs_sync())
            .order_by_asc(subscription::Column::TenantId)
            .order_by_asc(subscription::Column::SubscriberId)
            .limit(BATCH)
            .all(database.read())
            .await?;
        let mut synced = 0;
        let mut index = 0;
        while index < rows.len() {
            let (tenant_id, subscriber_id) = (rows[index].tenant_id, rows[index].subscriber_id);
            let mut group = Vec::new();
            while index < rows.len()
                && rows[index].tenant_id == tenant_id
                && rows[index].subscriber_id == subscriber_id
            {
                group.push(rows[index].clone());
                index += 1;
            }
            let person = subscriber::Entity::find_by_id(subscriber_id)
                .filter(subscriber::Column::TenantId.eq(tenant_id))
                .one(database.read())
                .await?;
            if let Some(person) = person {
                synced += self
                    .push_to_ledger(database, TenantId::from_uuid(tenant_id), &person.email, group)
                    .await?;
            }
        }
        if synced > 0 {
            tracing::info!(synced, "newsletter consents recorded in the consent ledger");
        }
        Ok(synced)
    }

    /// Người nhận của một lô còn đồng ý ở sổ: ghi sổ trước các đăng ký `confirmed` sổ chưa biết,
    /// rồi hỏi `granted_many`. Không có sổ (`compliance` không có trong chương trình): mọi người.
    pub(crate) async fn consented(
        &self,
        tenant: &TenantContext,
        list_id: Uuid,
        people: &[(Uuid, String)],
    ) -> Result<Option<Vec<bool>>, Error> {
        let Some(ledger) = self.consents.clone() else {
            return Ok(None);
        };
        let ids: Vec<Uuid> = people.iter().map(|(id, _)| *id).collect();
        let pending = subscription::Entity::find()
            .scoped(tenant)
            .filter(subscription::Column::ListId.eq(list_id))
            .filter(subscription::Column::SubscriberId.is_in(ids))
            .filter(needs_sync())
            .all(tenant.database().read())
            .await?;
        for row in pending {
            if let Some((_, email)) = people.iter().find(|(id, _)| *id == row.subscriber_id) {
                self.push_to_ledger(tenant.database(), tenant.id(), email, vec![row]).await?;
            }
        }
        let subjects: Vec<ConsentSubject> =
            people.iter().map(|(_, email)| ConsentSubject::Email(email.clone())).collect();
        Ok(Some(ledger.granted_many(tenant.id(), &subjects, &consent_key(list_id)).await?))
    }

    /// Đồng ý bị rút ở `compliance`: huỷ các đăng ký khớp của email đó (`scope` rỗng: mọi danh
    /// sách), ghi `ledger_state = withdrawn` luôn để không gọi lại sổ.
    pub(crate) async fn consent_withdrawn(
        &self,
        tenant: &TenantContext,
        consent_id: Uuid,
        scope: &str,
    ) -> Result<u64, Error> {
        let Some(ledger) = &self.consents else {
            return Ok(0);
        };
        let list = match scope {
            "" => None,
            other => {
                match other.strip_prefix("newsletter.list:").and_then(|id| id.parse::<Uuid>().ok())
                {
                    Some(id) => Some(id),
                    None => return Ok(0),
                }
            }
        };
        let Some(consent) = ledger.get(tenant.id(), consent_id).await? else {
            return Ok(0);
        };
        let Some(email) = consent.subject_value.map(|email| email.trim().to_lowercase()) else {
            return Ok(0);
        };
        let Some(person) = subscriber::Entity::find()
            .scoped(tenant)
            .filter(subscriber::Column::Email.eq(email.as_str()))
            .one(tenant.database().read())
            .await?
        else {
            return Ok(0);
        };
        let lists = self
            .lists_by_id(
                tenant.database().read(),
                tenant,
                self.lists_of(tenant, person.id, list).await?,
            )
            .await?;
        let transaction = tenant.database().begin_write().await?;
        let at = now();
        let mut changed = 0;
        for found in lists.values() {
            if self
                .unsubscribe_in(&transaction, tenant, Actor::system(), person.id, found, at)
                .await?
            {
                changed += 1;
            }
            subscription::Entity::update_many()
                .col_expr(subscription::Column::LedgerState, Expr::value(WITHDRAWN))
                .filter(subscription::Column::SubscriberId.eq(person.id))
                .filter(subscription::Column::ListId.eq(found.row.id))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
        }
        transaction.commit().await?;
        if changed > 0 {
            tracing::info!(subscriber_id = %person.id, changed, "newsletter subscriptions ended after a consent was withdrawn");
        }
        Ok(changed)
    }

    /// Danh sách người đăng ký `subscriber_id` có đăng ký (chỉ `only` nếu có).
    async fn lists_of(
        &self,
        tenant: &TenantContext,
        subscriber_id: Uuid,
        only: Option<Uuid>,
    ) -> Result<Vec<Uuid>, Error> {
        let mut select = subscription::Entity::find()
            .scoped(tenant)
            .select_only()
            .column(subscription::Column::ListId)
            .filter(subscription::Column::SubscriberId.eq(subscriber_id));
        if let Some(list) = only {
            select = select.filter(subscription::Column::ListId.eq(list));
        }
        Ok(select.into_tuple().all(tenant.database().read()).await?)
    }
}

/// Bộ chạy lịch [`BACKFILL`].
pub(crate) struct Backfill(pub Arc<NewsletterService>);

#[async_trait::async_trait]
impl JobHandler for Backfill {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.backfill_consents(&job.database).await.map(|_| ())
    }
}

/// Nghe `compliance.consent_withdrawn`: rút quảng cáo qua email (mọi kênh hay kênh `email`) ở mục
/// đích `marketing` → huỷ đăng ký bản tin khớp.
pub(crate) struct ConsentWithdrawn(pub Arc<NewsletterService>);

#[async_trait::async_trait]
impl EventHandler for ConsentWithdrawn {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let text = |key: &str| event.payload.get(key).and_then(Value::as_str).unwrap_or_default();
        if text("purpose") != "marketing" || !["", "email"].contains(&text("channel")) {
            return Ok(());
        }
        let Some(consent_id) = text("consent_id").parse::<Uuid>().ok() else {
            return Ok(());
        };
        self.0.consent_withdrawn(&event.tenant, consent_id, text("scope")).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bang_chung_tu_json_da_luu() {
        let stored = r#"{"text": "Send me news", "version": "v2", "source": "footer", "ip_fingerprint": "ab"}"#;
        let evidence = evidence_of(Some(stored));
        assert_eq!(evidence.text, "Send me news");
        assert_eq!(evidence.text_version, "v2");
        assert_eq!(evidence.source, "newsletter:footer");
        assert_eq!(evidence.ip_fingerprint.as_deref(), Some("ab"));
        let legacy = evidence_of(None);
        assert_eq!(legacy.text_version, "newsletter.legacy");
        assert!(!legacy.text.is_empty());
    }
}
