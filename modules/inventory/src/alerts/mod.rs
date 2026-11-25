//! Báo khi có hàng lại theo cửa hàng (bước 42 mục 9, D-201, D-20261119-step42-restock-alerts).
//!
//! - Đăng ký: `POST /public/restock-alerts`. Người đã đăng nhập (không nêu email / số) được báo vào tài
//!   khoản ngay. Khách vãng lai nêu email hay số điện thoại, qua lớp chống spam (`SpamGuard`), rồi
//!   **xác nhận hai bước**: nhận link có token (trang của website ở
//!   `AUTOXIDE_INVENTORY_RESTOCK_CONFIRM_URL`), trang đó gửi token về `/confirm`. Đồng ý riêng cho mục
//!   đích này (`consent: true`, giữ chữ và phiên bản làm bằng chứng). Câu trả lời luôn GIỐNG HỆT (202)
//!   dù địa chỉ mới, đang chờ, đã xác nhận hay bị bỏ vì spam: không ai dò được ai đã đăng ký.
//! - Gửi: nghe chính event `inventory.level_changed`. `available` đi từ ≤ 0 lên > 0 ở một kho thì xét
//!   các đăng ký đang "chờ đợt kế" (`armed`) của biến thể: cửa hàng của đăng ký bán được (cùng cách tính
//!   với tình trạng công khai) thì tắt `armed` (câu `UPDATE` có điều kiện — event đến lại không gửi lần
//!   hai), rồi gửi qua `notifications`. Cửa hàng hết hàng lại thì `armed` bật lại cho đợt sau.
//! - Mỗi tin có link dừng (token mới mỗi lần gửi; database chỉ giữ bản băm).

pub(crate) mod migration;
mod notify;
mod privacy;

use std::time::Duration;

use autoxide_kernel::access::Principal;
use autoxide_kernel::antispam::{Claim, Limits, Submission as Check, Verdict};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::http::ClientInfo;
use autoxide_kernel::secret::{random_token, sha256};
use autoxide_kernel::sms::PhoneNumber;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext, link_token, resolve_link_token};
use autoxide_kernel::throttle::{Outcome, Throttle};
use autoxide_notifications::api::{Notification, Recipient};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use serde_json::json;
use uuid::Uuid;

pub(crate) use notify::{declare, subscriptions};
pub(crate) use privacy::AlertPrivacy;

use crate::entity::restock_alert;
use crate::service::{InventoryService, now};

/// Phạm vi token form (`GET /api/v1/antispam/token?scope=inventory:restock_alerts`).
pub(crate) const FORM_SCOPE: &str = "inventory:restock_alerts";
const MODULE: &str = "inventory";
const HOUR: Duration = Duration::from_secs(3600);
/// Đăng ký từ một IP / một email mỗi giờ.
const PER_HOUR: u32 = 20;
/// Tin xác nhận gửi lại cho một liên lạc mỗi giờ.
const CONFIRMS_PER_HOUR: u32 = 3;
const DEFAULT_COUNTRY: &str = "84";

pub(crate) const PENDING: &str = "pending";
pub(crate) const CONFIRMED: &str = "confirmed";
pub(crate) const STOPPED: &str = "stopped";

/// Một yêu cầu đăng ký.
#[derive(Debug, Clone, Default)]
pub(crate) struct AlertRequest {
    pub variant_id: Uuid,
    pub store_id: Option<Uuid>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub consent: bool,
    pub consent_text: Option<String>,
    pub consent_version: Option<String>,
    pub locale: Option<String>,
    pub form_token: Option<String>,
    pub trap: Option<String>,
    pub challenge: Option<String>,
}

/// Người nhận của một đăng ký.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Contact {
    User(Uuid),
    Email(String),
    Phone(String),
}

impl Contact {
    fn parts(&self) -> (&'static str, String) {
        match self {
            Contact::User(id) => ("user", id.to_string()),
            Contact::Email(email) => ("email", email.clone()),
            Contact::Phone(phone) => ("phone", phone.clone()),
        }
    }
}

pub(crate) fn recipient(row: &restock_alert::Model) -> Option<Recipient> {
    match row.contact_kind.as_str() {
        "user" => row.user_id.map(Recipient::User),
        "email" => Some(Recipient::Email(row.contact.clone())),
        "phone" => Some(Recipient::Phone(row.contact.clone())),
        _ => None,
    }
}

fn contact(
    principal: Option<&Principal>,
    email: Option<&str>,
    phone: Option<&str>,
    problems: &mut Vec<FieldError>,
) -> Contact {
    let email = email.map(str::trim).filter(|email| !email.is_empty());
    let phone = phone.map(str::trim).filter(|phone| !phone.is_empty());
    match (email, phone, principal) {
        (None, None, Some(principal)) => Contact::User(principal.user_id()),
        (Some(email), None, _) => {
            if !autoxide_kernel::email::is_valid(email) {
                problems.push(FieldError::new("email", "invalid", "is not an email address"));
            }
            Contact::Email(autoxide_kernel::email::normalize(email))
        }
        (None, Some(raw), _) => match PhoneNumber::parse(raw, DEFAULT_COUNTRY) {
            Some(phone) => Contact::Phone(phone.as_str().to_owned()),
            None => {
                problems.push(FieldError::new("phone", "invalid", "is not a phone number"));
                Contact::Phone(String::new())
            }
        },
        _ => {
            problems.push(FieldError::new(
                "email",
                "contact_required",
                "give either `email` or `phone` (not both), or sign in",
            ));
            Contact::Email(String::new())
        }
    }
}

/// Link của website cho token `raw` (kèm khách khi không phải khách mặc định).
pub(crate) fn token_link(url: &str, tenant: &TenantContext, raw: &str) -> String {
    url.replace("{token}", &link_token(tenant, raw))
}

pub(crate) fn unavailable() -> Error {
    Error::unavailable(
        "inventory.restock_alerts_unavailable",
        "Back-in-stock alerts need module `notifications`, and for visitors who are not signed in \
         AUTOXIDE_INVENTORY_RESTOCK_CONFIRM_URL (a page of your website with `{token}`).",
    )
}

fn token_invalid() -> Error {
    Error::bad_request(
        "inventory.restock_token_invalid",
        "This link is invalid or was already used. Ask for the alert again to get a new one.",
    )
}

impl InventoryService {
    /// Đăng ký báo khi có hàng lại (xem đầu file). Trả `Ok` như nhau cho mọi trường hợp đã nhận.
    pub(crate) async fn subscribe_restock(
        &self,
        tenant: &TenantContext,
        principal: Option<&Principal>,
        client: &ClientInfo,
        input: AlertRequest,
    ) -> Result<(), Error> {
        let mut problems = Vec::new();
        if !input.consent {
            problems.push(FieldError::new(
                "consent",
                "required",
                "must be true: the visitor agrees to get a message when this item is back in stock",
            ));
        }
        if input.consent_text.as_deref().is_some_and(|text| text.chars().count() > 2000) {
            problems.push(
                FieldError::new("consent_text", "length", "must be at most {max} characters")
                    .with_param("max", 2000),
            );
        }
        if let Some(locale) = input.locale.as_deref()
            && !matches!(locale, "en" | "vi")
        {
            problems.push(FieldError::new("locale", "invalid", "must be `en` or `vi`"));
        }
        let who = contact(principal, input.email.as_deref(), input.phone.as_deref(), &mut problems);
        crate::service::fail_if_any(problems)?;
        let Some(notifier) = &self.notifier else { return Err(unavailable()) };
        let confirm_url = self.config.restock_confirm_url.as_deref();
        let is_user = matches!(who, Contact::User(_));
        if !is_user && confirm_url.is_none() {
            return Err(unavailable());
        }

        let throttle = Throttle::new(MODULE, PER_HOUR, HOUR)?;
        let limits = match principal {
            Some(_) => Limits::default().user(Some(throttle.clone())),
            None => Limits::default().ip(Some(throttle.clone())).email(Some(throttle.clone())),
        };
        let email = match &who {
            Contact::Email(email) => Some(email.as_str()),
            _ => None,
        };
        if let Contact::Phone(phone) = &who
            && let Outcome::Limited { .. } = throttle.hit(tenant, &format!("phone:{phone}")).await?
        {
            // Quá nhiều lần cho một số: im lặng như bài đã nhận (không dạy bot).
            return Ok(());
        }
        let checked = self
            .guard
            .check(
                tenant,
                &Check::new(FORM_SCOPE, client)
                    .principal(principal)
                    .honeypot(input.trap.as_deref())
                    .form_token(input.form_token.as_deref())
                    .challenge(input.challenge.as_deref())
                    .email(email),
                &limits,
            )
            .await?;
        if let Verdict::Discard { reason } = &checked.verdict {
            tracing::info!(reason, "inventory restock alert dropped");
            return Ok(());
        }
        // Biến thể không có / chưa bán / không theo dõi tồn: nhận như thường, không lưu gì.
        let Some(variant) = self
            .catalog
            .variants(tenant, &[input.variant_id], None)
            .await?
            .into_iter()
            .find(|variant| crate::public::visible(variant) && variant.track_inventory)
        else {
            return Ok(());
        };
        // Cửa hàng phải là nút của khách (PT-8): không lưu ID bịa.
        if let Some(store) = input.store_id {
            autoxide_kernel::tenancy::OrgTree::ensure_exists(
                tenant.database().read(),
                tenant.id(),
                autoxide_kernel::tenancy::OrgId::from_uuid(store),
            )
            .await?;
        }
        let store_key = input.store_id.unwrap_or(Uuid::nil());
        let (kind, value) = who.parts();
        let consent = json!({
            "text": input.consent_text,
            "version": input.consent_version,
            "purpose": "inventory.restock_alert",
            "at": now(),
        })
        .to_string();
        let raw = random_token(24)?;
        let hash = sha256(&raw);
        let user_id = match who {
            Contact::User(id) => Some(id),
            _ => principal.map(Principal::user_id),
        };
        let initial = if is_user { CONFIRMED } else { PENDING };

        let database = tenant.database();
        let transaction = database.begin_write().await?;
        let existing = restock_alert::Entity::find()
            .filter(restock_alert::Column::VariantId.eq(variant.variant_id))
            .filter(restock_alert::Column::StoreKey.eq(store_key))
            .filter(restock_alert::Column::ContactKind.eq(kind))
            .filter(restock_alert::Column::Contact.eq(value.clone()))
            .scoped(tenant)
            .one(&transaction)
            .await?;
        let stamp = now();
        let (id, send_confirmation) = match existing {
            None => {
                let id = Uuid::now_v7();
                restock_alert::Entity::insert(restock_alert::ActiveModel {
                    id: Set(id),
                    tenant_id: Set(tenant.id().as_uuid()),
                    variant_id: Set(variant.variant_id),
                    store_key: Set(store_key),
                    contact_kind: Set(kind.to_owned()),
                    contact: Set(value.clone()),
                    user_id: Set(user_id),
                    status: Set(initial.to_owned()),
                    token_hash: Set(Some(hash.clone())),
                    consent: Set(consent),
                    locale: Set(input.locale.clone()),
                    armed: Set(true),
                    sent_count: Set(0),
                    confirmed_at: Set(is_user.then_some(stamp)),
                    created_at: Set(stamp),
                    updated_at: Set(stamp),
                })
                .exec_without_returning(&transaction)
                .await?;
                (id, !is_user)
            }
            Some(row) if row.status == CONFIRMED => (row.id, false),
            Some(row) => {
                // Đang chờ (gửi lại link) hay đã dừng (đăng ký lại: phải xác nhận lại).
                restock_alert::Entity::update_many()
                    .col_expr(restock_alert::Column::Status, Expr::value(initial))
                    .col_expr(restock_alert::Column::TokenHash, Expr::value(Some(hash.clone())))
                    .col_expr(restock_alert::Column::Consent, Expr::value(consent))
                    .col_expr(restock_alert::Column::Locale, Expr::value(input.locale.clone()))
                    .col_expr(restock_alert::Column::Armed, Expr::value(true))
                    .col_expr(
                        restock_alert::Column::ConfirmedAt,
                        Expr::value(is_user.then_some(stamp)),
                    )
                    .col_expr(restock_alert::Column::UpdatedAt, Expr::value(stamp))
                    .filter(restock_alert::Column::Id.eq(row.id))
                    .scoped(tenant)
                    .exec(&transaction)
                    .await?;
                (row.id, !is_user)
            }
        };
        if let Claim::Replay { .. } =
            checked.claim(&transaction, tenant, MODULE, Some(&id.to_string())).await?
        {
            transaction.rollback().await?;
            return Ok(());
        }
        transaction.commit().await?;

        let Some(url) = confirm_url.filter(|_| send_confirmation) else { return Ok(()) };
        let limit = Throttle::new(MODULE, CONFIRMS_PER_HOUR, HOUR)?;
        if let Outcome::Limited { .. } =
            limit.hit(tenant, &format!("confirm:{kind}:{value}")).await?
        {
            return Ok(());
        }
        let recipient = match who {
            Contact::Email(email) => Recipient::Email(email),
            Contact::Phone(phone) => Recipient::Phone(phone),
            Contact::User(user) => Recipient::User(user),
        };
        let mut notification = Notification::new(
            notify::RESTOCK_CONFIRM.key(),
            recipient,
            json!({"product": notify::product_name(&variant)}),
            format!("inventory.restock_confirm:{id}:{}", &hash[..16]),
        )
        .link(token_link(url, tenant, &raw));
        if let Some(locale) = input.locale {
            notification = notification.locale(locale);
        }
        if let Err(error) = notifier.notify(tenant.id(), notification).await {
            // Đăng ký đã lưu; người mua gửi lại form để nhận link mới.
            tracing::warn!(alert_id = %id, code = error.code(), "restock alert confirmation not sent");
        }
        Ok(())
    }

    /// Xác nhận (bước hai) hay dừng một đăng ký bằng token trong link. Khách lấy từ token khi token mang
    /// khách; không thì khách của request.
    pub(crate) async fn restock_token(
        &self,
        request: Option<&TenantContext>,
        token: &str,
        confirm: bool,
    ) -> Result<(), Error> {
        let Some((resolved, raw)) = resolve_link_token(&self.databases, request, token).await?
        else {
            return Err(token_invalid());
        };
        let tenant = &resolved;
        let hash = sha256(raw);
        let database = tenant.database();
        let transaction = database.begin_write().await?;
        let Some(row) = restock_alert::Entity::find()
            .filter(restock_alert::Column::TokenHash.eq(hash))
            .scoped(tenant)
            .one(&transaction)
            .await?
        else {
            return Err(token_invalid());
        };
        let stamp = now();
        let mut update = restock_alert::Entity::update_many()
            .col_expr(restock_alert::Column::UpdatedAt, Expr::value(stamp))
            .filter(restock_alert::Column::Id.eq(row.id))
            .scoped(tenant);
        if confirm {
            if row.status == STOPPED {
                return Err(token_invalid());
            }
            if row.status == CONFIRMED {
                return Ok(());
            }
            update = update
                .col_expr(restock_alert::Column::Status, Expr::value(CONFIRMED))
                .col_expr(restock_alert::Column::ConfirmedAt, Expr::value(Some(stamp)))
                .col_expr(restock_alert::Column::Armed, Expr::value(true));
        } else {
            update = update
                .col_expr(restock_alert::Column::Status, Expr::value(STOPPED))
                .col_expr(restock_alert::Column::Armed, Expr::value(false))
                .col_expr(restock_alert::Column::TokenHash, Expr::value(None::<String>));
        }
        update.exec(&transaction).await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Biến thể (trong `variant_ids`) mà người dùng `user` đang có đăng ký đã xác nhận ở kho bán online.
    pub(crate) async fn restock_alerts_for(
        &self,
        tenant: &TenantContext,
        user: Uuid,
        variant_ids: &[Uuid],
    ) -> Result<Vec<Uuid>, Error> {
        if variant_ids.is_empty() {
            return Ok(Vec::new());
        }
        let found: Vec<Uuid> = restock_alert::Entity::find()
            .select_only()
            .column(restock_alert::Column::VariantId)
            .filter(restock_alert::Column::UserId.eq(user))
            .filter(restock_alert::Column::VariantId.is_in(variant_ids.to_vec()))
            .filter(restock_alert::Column::StoreKey.eq(Uuid::nil()))
            .filter(restock_alert::Column::Status.eq(CONFIRMED))
            .scoped(tenant)
            .into_tuple()
            .all(tenant.database().read())
            .await?;
        Ok(variant_ids.iter().copied().filter(|id| found.contains(id)).collect())
    }
}
