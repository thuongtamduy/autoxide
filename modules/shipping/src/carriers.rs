//! Tài khoản hãng vận chuyển của khách (bước 44 mục 3). Thông tin đăng nhập mã hoá AEAD bằng
//! `Sealer` của kernel (khoá theo khách, gắn với ID tài khoản — D-190), không bao giờ trả ra API,
//! log hay event. Gọi hãng (kiểm kết nối, báo giá) luôn ngoài transaction ghi, có giới hạn thời
//! gian `AUTOXIDE_SHIPPING_CARRIER_TIMEOUT_SECONDS`.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::secret::{random_token, sha256};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter, Set,
};
use uuid::Uuid;

use crate::api::{
    CarrierAccount, CarrierCredentials, CarrierRegistry, ShippingCarrier, errors as carrier_errors,
};
use crate::entity::carrier_account;
use crate::errors;
use crate::events::{self, Change};
use crate::permissions::CARRIERS_MANAGE;
use crate::service::{ShippingService, check_length, now};
use crate::settings::{readable, writable};

/// Mục đích của khoá mã hoá thông tin đăng nhập hãng.
const PURPOSE: &str = "shipping.carrier_credentials";

/// Nội dung tài khoản hãng gửi lên.
#[derive(Debug, Clone, Default)]
pub(crate) struct AccountDraft {
    pub org_id: Option<Uuid>,
    pub carrier_code: Option<String>,
    pub name: Option<String>,
    /// Thay toàn bộ thông tin đăng nhập; `None` giữ nguyên.
    pub credentials: Option<BTreeMap<String, String>>,
    pub settings: Option<serde_json::Value>,
    pub sandbox: Option<bool>,
    pub active: Option<bool>,
}

/// Tài khoản kèm tên các trường đăng nhập đã đặt (không giá trị).
#[derive(Debug, Clone)]
pub(crate) struct AccountView {
    pub row: carrier_account::Model,
    pub credential_fields: Vec<String>,
    /// Chỉ có ngay sau khi tạo: token trong đường dẫn webhook (lưu băm, không lấy lại được).
    pub webhook_token: Option<String>,
    /// Đi cùng `webhook_token`: định danh của khách cho đường webhook của hãng này
    /// (`ProviderAccounts::webhook_token`) — hãng gọi về không cần header hay tên miền của khách.
    pub webhook_hook: Option<String>,
}

/// Đường nhận webhook của hãng `carrier_code`, chưa có định danh khách và token tài khoản. Cùng
/// một chuỗi lúc cấp định danh (tạo tài khoản) và lúc tìm khách (nhận webhook).
pub(crate) fn webhook_base(carrier_code: &str) -> String {
    format!("/api/v1/shipping/webhooks/{carrier_code}")
}

/// Gọi `future` tối đa `seconds` giây; quá giờ → `shipping.carrier_unavailable`.
pub(crate) async fn within<T>(
    seconds: u64,
    future: impl Future<Output = Result<T, Error>>,
) -> Result<T, Error> {
    match tokio::time::timeout(Duration::from_secs(seconds), future).await {
        Ok(result) => result,
        Err(_) => Err(carrier_errors::carrier_unavailable()),
    }
}

fn snapshot(row: &carrier_account::Model) -> serde_json::Value {
    serde_json::json!({
        "id": row.id,
        "org_id": row.org_id,
        "carrier_code": row.carrier_code,
        "name": row.name,
        "sandbox": row.sandbox,
        "active": row.active,
        "check_status": row.check_status,
    })
}

impl ShippingService {
    fn seal_credentials(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        sealer: &autoxide_kernel::secret::Sealer,
        credentials: &BTreeMap<String, String>,
    ) -> Result<String, Error> {
        let plain = serde_json::to_vec(credentials).map_err(Error::internal)?;
        sealer.seal(tenant.id(), PURPOSE, &id.to_string(), &plain)
    }

    /// Tài khoản đã giải mã, để gọi hãng.
    pub(crate) async fn account(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<(carrier_account::Model, CarrierAccount, Arc<dyn ShippingCarrier>), Error> {
        let row = carrier_account::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| errors::carrier_account_not_found(id))?;
        let carrier = self
            .carriers
            .for_tenant(tenant, &row.carrier_code)
            .ok_or_else(|| errors::carrier_unknown(&row.carrier_code))?;
        let sealer = self.secret.sealer().await?;
        let plain = sealer.open(tenant.id(), PURPOSE, &id.to_string(), &row.credentials_sealed)?;
        let credentials: BTreeMap<String, String> = serde_json::from_slice(&plain)
            .map_err(|_| Error::internal("stored carrier credentials cannot be read"))?;
        let settings = serde_json::from_str(&row.settings).unwrap_or(serde_json::Value::Null);
        let account = CarrierAccount::new(
            row.id,
            row.carrier_code.clone(),
            row.sandbox,
            settings,
            CarrierCredentials::new(credentials),
        );
        Ok((row, account, carrier))
    }

    fn view(
        row: carrier_account::Model,
        fields: Vec<String>,
        token: Option<String>,
    ) -> AccountView {
        AccountView { row, credential_fields: fields, webhook_token: token, webhook_hook: None }
    }

    async fn credential_fields(
        &self,
        tenant: &TenantContext,
        row: &carrier_account::Model,
    ) -> Vec<String> {
        let Ok(sealer) = self.secret.sealer().await else { return Vec::new() };
        sealer
            .open(tenant.id(), PURPOSE, &row.id.to_string(), &row.credentials_sealed)
            .ok()
            .and_then(|plain| serde_json::from_slice::<BTreeMap<String, String>>(&plain).ok())
            .map(|values| values.into_keys().collect())
            .unwrap_or_default()
    }

    pub(crate) async fn list_accounts(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<AccountView>, Error> {
        let condition = Condition::all().add(principal.scope_condition(
            &CARRIERS_MANAGE,
            carrier_account::Column::OrgId,
            None,
        )?);
        let select = carrier_account::Entity::find().filter(condition).scoped(tenant);
        let page = fetch_page(
            select,
            carrier_account::Column::Id,
            |row| row.id,
            page,
            tenant.database().read(),
        )
        .await?;
        let mut data = Vec::with_capacity(page.data.len());
        for row in page.data {
            let fields = self.credential_fields(tenant, &row).await;
            data.push(Self::view(row, fields, None));
        }
        Ok(Page { data, pagination: page.pagination })
    }

    pub(crate) async fn get_account(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<AccountView, Error> {
        let read = tenant.database().read();
        let row = carrier_account::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| errors::carrier_account_not_found(id))?;
        readable(principal, read, &CARRIERS_MANAGE, row.org_id, || {
            errors::carrier_account_not_found(id)
        })
        .await?;
        let fields = self.credential_fields(tenant, &row).await;
        Ok(Self::view(row, fields, None))
    }

    /// Thêm tài khoản hãng. Trả token webhook một lần (đường dẫn
    /// `/api/v1/shipping/webhooks/{carrier_code}/{hook}/{token}`: `{hook}` tìm khách, `{token}` tìm
    /// tài khoản).
    pub(crate) async fn create_account(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        draft: AccountDraft,
    ) -> Result<AccountView, Error> {
        let mut problems = Vec::new();
        let code = draft.carrier_code.unwrap_or_default().trim().to_owned();
        let name = draft.name.unwrap_or_default().trim().to_owned();
        check_length(&mut problems, "name", &name, 1, 200);
        let credentials = draft.credentials.unwrap_or_default();
        if credentials.is_empty() {
            problems.push(FieldError::new(
                "credentials",
                "required",
                "give the account credentials",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        if self.carriers.get(&code).is_none() {
            return Err(errors::carrier_unknown(&code));
        }
        let org = match draft.org_id {
            Some(org) => org,
            None => crate::settings::default_org(tenant, principal, &CARRIERS_MANAGE)?.as_uuid(),
        };
        self.entitlements.require_feature(tenant.id(), &crate::service::CARRIERS_FEATURE).await?;
        let id = Uuid::now_v7();
        let sealer = self.secret.sealer().await?;
        let sealed = self.seal_credentials(tenant, id, &sealer, &credentials)?;
        let token = random_token(24)?;
        let at = now();
        let transaction = tenant.database().begin_write().await?;
        principal
            .require_write_at(
                &transaction,
                &CARRIERS_MANAGE,
                OrgTarget::new(OrgId::from_uuid(org), None),
            )
            .await?;
        let row = carrier_account::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(org),
            carrier_code: Set(code),
            name: Set(name),
            credentials_sealed: Set(sealed),
            webhook_token_hash: Set(sha256(&token)),
            settings: Set(draft
                .settings
                .unwrap_or(serde_json::Value::Object(Default::default()))
                .to_string()),
            sandbox: Set(draft.sandbox.unwrap_or(false)),
            active: Set(draft.active.unwrap_or(true)),
            check_status: Set("never".to_owned()),
            checked_at: Set(None),
            check_error_code: Set(None),
            created_at: Set(at),
            updated_at: Set(at),
        }
        .insert(&transaction)
        .await?;
        let event =
            events::CarrierAccountChanged(Change { before: None, after: Some(snapshot(&row)) });
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::user(principal.user_id()),
                events::account_subject(id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        let hook = self.providers.webhook_token(tenant, &webhook_base(&row.carrier_code)).await?;
        let mut view = Self::view(row, credentials.into_keys().collect(), Some(token));
        view.webhook_hook = Some(hook);
        Ok(view)
    }

    pub(crate) async fn update_account(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        draft: AccountDraft,
    ) -> Result<AccountView, Error> {
        let mut problems = Vec::new();
        if draft.carrier_code.is_some() {
            problems.push(FieldError::new(
                "carrier_code",
                "immutable",
                "cannot be changed; add another account",
            ));
        }
        if draft.org_id.is_some() {
            problems.push(FieldError::new(
                "org_id",
                "immutable",
                "cannot be changed; add another account",
            ));
        }
        let name = draft.name.map(|name| name.trim().to_owned());
        if let Some(name) = &name {
            check_length(&mut problems, "name", name, 1, 200);
        }
        if draft.credentials.as_ref().is_some_and(BTreeMap::is_empty) {
            problems.push(FieldError::new(
                "credentials",
                "required",
                "give the account credentials",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let sealer = self.secret.sealer().await?;
        let transaction = tenant.database().begin_write().await?;
        let before = carrier_account::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| errors::carrier_account_not_found(id))?;
        writable(principal, &transaction, &CARRIERS_MANAGE, before.org_id, || {
            errors::carrier_account_not_found(id)
        })
        .await?;
        let mut active: carrier_account::ActiveModel = before.clone().into();
        if let Some(name) = name {
            active.name = Set(name);
        }
        if let Some(credentials) = &draft.credentials {
            active.credentials_sealed =
                Set(self.seal_credentials(tenant, id, &sealer, credentials)?);
            // Thông tin mới: chưa kiểm.
            active.check_status = Set("never".to_owned());
            active.check_error_code = Set(None);
        }
        if let Some(settings) = draft.settings {
            active.settings = Set(settings.to_string());
        }
        if let Some(sandbox) = draft.sandbox {
            active.sandbox = Set(sandbox);
        }
        if let Some(enabled) = draft.active {
            active.active = Set(enabled);
        }
        active.updated_at = Set(now());
        let row = active.update(&transaction).await?;
        let event = events::CarrierAccountChanged(Change {
            before: Some(snapshot(&before)),
            after: Some(snapshot(&row)),
        });
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::user(principal.user_id()),
                events::account_subject(id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        let fields = self.credential_fields(tenant, &row).await;
        Ok(Self::view(row, fields, None))
    }

    /// Kiểm kết nối: gọi `ShippingCarrier::check` (ngoài transaction), ghi `check_status`.
    pub(crate) async fn test_account(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<AccountView, Error> {
        let current = self.get_account(tenant, principal, id).await?;
        principal
            .require_write_at(
                tenant.database().write(),
                &CARRIERS_MANAGE,
                OrgTarget::new(OrgId::from_uuid(current.row.org_id), None),
            )
            .await?;
        let (_, account, carrier) = self.account(tenant, id).await?;
        let outcome = within(self.config.carrier_timeout_seconds, carrier.check(&account)).await;
        self.record_check(tenant, id, outcome.as_ref().err().map(Error::code)).await?;
        self.get_account(tenant, principal, id).await
    }

    /// Ghi kết quả kiểm (cả khi gọi hãng thật nhận `Unauthorized`).
    pub(crate) async fn record_check(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        error_code: Option<&str>,
    ) -> Result<(), Error> {
        carrier_account::Entity::update_many()
            .col_expr(
                carrier_account::Column::CheckStatus,
                Expr::value(if error_code.is_some() { "failed" } else { "ok" }),
            )
            .col_expr(carrier_account::Column::CheckedAt, Expr::value(Some(now())))
            .col_expr(
                carrier_account::Column::CheckErrorCode,
                Expr::value(error_code.map(str::to_owned)),
            )
            .filter(carrier_account::Column::Id.eq(id))
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        Ok(())
    }

    /// Lỗi khi gọi hãng thật: hãng từ chối thông tin đăng nhập → ghi `failed` lên tài khoản.
    pub(crate) async fn note_carrier_error(&self, tenant: &TenantContext, id: Uuid, error: &Error) {
        if error.code() == "shipping.carrier_unauthorized"
            && let Err(failure) = self.record_check(tenant, id, Some(error.code())).await
        {
            tracing::warn!(account_id = %id, error = %failure, "cannot record a failed carrier check");
        }
    }

    /// Tài khoản theo token webhook (đường dẫn công khai), cùng hãng.
    pub(crate) async fn account_by_token<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        carrier_code: &str,
        token: &str,
    ) -> Result<Option<carrier_account::Model>, Error> {
        Ok(carrier_account::Entity::find()
            .filter(carrier_account::Column::WebhookTokenHash.eq(sha256(token)))
            .filter(carrier_account::Column::CarrierCode.eq(carrier_code))
            .filter(carrier_account::Column::Active.eq(true))
            .scoped(tenant)
            .one(connection)
            .await?)
    }
}

/// Dòng tài khoản hãng `id` của khách; không có → `shipping.carrier_account_not_found`.
pub(crate) async fn find_account<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    id: Uuid,
) -> Result<carrier_account::Model, Error> {
    carrier_account::Entity::find_by_id(id)
        .scoped(tenant)
        .one(connection)
        .await?
        .ok_or_else(|| errors::carrier_account_not_found(id))
}
