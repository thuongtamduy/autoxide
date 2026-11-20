//! Hồ sơ khách (bước 47 mục 1): nhận diện theo tài khoản người mua hay số điện thoại, đăng ký tại
//! quầy, đọc / sửa theo phạm vi của nhân viên, che liên hệ (D-226).
//!
//! Một hồ sơ cho mỗi người mua trong khách: `user_id` (tài khoản) và / hoặc `phone_key` (HMAC của
//! số theo khách — cùng hàm `orders` dùng cho `buyer_key`). Hồ sơ theo số chỉ gắn vào tài khoản khi
//! số của tài khoản đã xác thực (bước 37) — không thì ai cũng nhận được lịch sử, điểm của người khác.

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::sms::PhoneNumber;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, root_of};
use chrono::NaiveDate;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::entity::profile;
use crate::errors;
use crate::events::{Change, ProfileCreated, ProfileLinked, ProfileUpdated};
use crate::permissions::{MANAGE, READ};
use crate::service::{CUSTOMER, CustomersService, DEFAULT_COUNTRY, now};

/// Giới tính khách tự khai (tuỳ chọn).
pub(crate) const GENDERS: [&str; 3] = ["female", "male", "other"];

/// Thông tin của khách nhân viên nhập hay người mua tự sửa. `None`: giữ nguyên.
#[derive(Debug, Clone, Default)]
pub(crate) struct Details {
    pub full_name: Option<String>,
    pub email: Option<String>,
    pub birthday: Option<NaiveDate>,
    pub gender: Option<String>,
    pub custom_fields: Option<Map<String, Value>>,
}

/// Đăng ký tại quầy.
#[derive(Debug, Clone)]
pub(crate) struct Registration {
    pub phone: String,
    /// Nút nơi khách đăng ký; `None`: nút gốc.
    pub org_id: Option<Uuid>,
    pub details: Details,
}

/// Liên hệ đầy đủ (chỉ cho người có `customers.contact.reveal`, có audit).
#[derive(Debug, Clone)]
pub(crate) struct Contact {
    pub phone: Option<String>,
    pub email: Option<String>,
}

/// Bộ lọc danh sách hồ sơ; mọi trường tuỳ chọn, kết hợp AND.
#[derive(Debug, Clone, Default)]
pub(crate) struct ProfileFilter {
    /// Tên (không dấu), số điện thoại (chuẩn hoá rồi băm) hay email.
    pub q: Option<String>,
    pub group_id: Option<Uuid>,
    pub tier: Option<String>,
    pub tag_ids: Vec<Uuid>,
    pub org_id: Option<Uuid>,
}

impl CustomersService {
    /// Hồ sơ `id` của khách, không kiểm quyền.
    pub(crate) async fn find_profile<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<Option<profile::Model>, Error> {
        Ok(profile::Entity::find_by_id(id).scoped(tenant).one(connection).await?)
    }

    async fn by_user<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<Option<profile::Model>, Error> {
        Ok(profile::Entity::find()
            .filter(profile::Column::UserId.eq(user_id))
            .scoped(tenant)
            .one(connection)
            .await?)
    }

    async fn by_phone_key<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        key: &str,
    ) -> Result<Option<profile::Model>, Error> {
        Ok(profile::Entity::find()
            .filter(profile::Column::PhoneKey.eq(key))
            .scoped(tenant)
            .one(connection)
            .await?)
    }

    /// Hồ sơ của tài khoản `user_id`, tạo khi chưa có (`org` là nút của lần gặp đầu). Số đã xác
    /// thực của tài khoản trùng một hồ sơ theo số chưa có chủ → gắn hồ sơ đó vào tài khoản.
    pub(crate) async fn profile_for_user(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        org: OrgId,
        actor: Actor,
    ) -> Result<profile::Model, Error> {
        if let Some(found) = Self::by_user(tenant.database().read(), tenant, user_id).await? {
            return Ok(found);
        }
        // Gọi ra ngoài transaction ghi: đọc số đã xác thực của `compliance`.
        let verified = match &self.accounts {
            Some(accounts) => accounts.verified_phone(tenant.id(), user_id).await?,
            None => None,
        };
        let key = match &verified {
            Some(phone) => Some(self.phone_key(tenant.id(), phone).await?),
            None => None,
        };
        let database = tenant.database();
        let transaction = database.begin_write().await?;
        if let Some(found) = Self::by_user(&transaction, tenant, user_id).await? {
            return Ok(found);
        }
        let at = now();
        if let (Some(phone), Some(key)) = (&verified, &key) {
            // Hồ sơ theo số chưa có chủ: gắn vào tài khoản (câu cập nhật có điều kiện — hai lần gắn
            // cùng lúc chỉ một bên thắng).
            let linked = profile::Entity::update_many()
                .col_expr(profile::Column::UserId, Expr::value(user_id))
                .col_expr(profile::Column::Phone, Expr::value(phone.as_str()))
                .col_expr(profile::Column::UpdatedAt, Expr::value(at))
                .filter(profile::Column::PhoneKey.eq(key.as_str()))
                .filter(profile::Column::UserId.is_null())
                .scoped(tenant)
                .exec(&transaction)
                .await?
                .rows_affected;
            if linked == 1 {
                let row = Self::by_user(&transaction, tenant, user_id)
                    .await?
                    .ok_or_else(|| Error::internal("linked customer profile vanished"))?;
                self.outbox
                    .record(
                        &transaction,
                        tenant,
                        actor,
                        Some(subject(row.id)),
                        &ProfileLinked(Change::after(
                            json!({"profile_id": row.id, "user_id": user_id}),
                        )),
                    )
                    .await?;
                transaction.commit().await?;
                return Ok(row);
            }
        }
        // Số đã thuộc hồ sơ của tài khoản khác: tạo hồ sơ không mang số.
        let key = match &key {
            Some(key) if Self::by_phone_key(&transaction, tenant, key).await?.is_none() => {
                Some(key.clone())
            }
            _ => None,
        };
        let phone = key.as_ref().and(verified.as_ref()).map(|phone| phone.as_str().to_owned());
        let row = new_row(tenant, org, Some(user_id), key, phone, at);
        let Some(row) = self.insert_profile(&transaction, tenant, row, actor, "account").await?
        else {
            drop(transaction);
            // Request khác vừa tạo hồ sơ của tài khoản (hay vừa lấy số này): đọc lại.
            return Self::by_user(tenant.database().write(), tenant, user_id)
                .await?
                .ok_or_else(errors::busy);
        };
        transaction.commit().await?;
        Ok(row)
    }

    /// Hồ sơ theo khoá số điện thoại `key` (đơn của khách vãng lai), tạo khi chưa có.
    pub(crate) async fn profile_for_phone_key(
        &self,
        tenant: &TenantContext,
        key: &str,
        org: OrgId,
    ) -> Result<profile::Model, Error> {
        if let Some(found) = Self::by_phone_key(tenant.database().read(), tenant, key).await? {
            return Ok(found);
        }
        let transaction = tenant.database().begin_write().await?;
        if let Some(found) = Self::by_phone_key(&transaction, tenant, key).await? {
            return Ok(found);
        }
        let row = new_row(tenant, org, None, Some(key.to_owned()), None, now());
        let Some(row) =
            self.insert_profile(&transaction, tenant, row, Actor::system(), "order").await?
        else {
            drop(transaction);
            return Self::by_phone_key(tenant.database().write(), tenant, key)
                .await?
                .ok_or_else(|| Error::internal("customer profile by phone vanished"));
        };
        transaction.commit().await?;
        Ok(row)
    }

    async fn insert_profile<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        row: profile::ActiveModel,
        actor: Actor,
        source: &str,
    ) -> Result<Option<profile::Model>, Error> {
        // Chèn "trùng thì bỏ qua" (unique theo tài khoản / theo số): hai request tạo cùng lúc trên
        // Postgres không thành lỗi 500; bên thua đọc lại hồ sơ của bên thắng.
        let id =
            row.id.clone().take().ok_or_else(|| Error::internal("customer profile without id"))?;
        let inserted = profile::Entity::insert(row)
            .on_conflict(OnConflict::new().do_nothing().to_owned())
            .exec_without_returning(connection)
            .await?;
        if inserted == 0 {
            return Ok(None);
        }
        let row = self
            .find_profile(connection, tenant, id)
            .await?
            .ok_or_else(|| Error::internal("new customer profile vanished"))?;
        self.outbox
            .record(
                connection,
                tenant,
                actor,
                Some(subject(row.id)),
                &ProfileCreated(Change::after(json!({
                    "profile_id": row.id, "org_id": row.org_id, "user_id": row.user_id,
                    "source": source,
                }))),
            )
            .await?;
        self.schedule_evaluation(connection, tenant, row.id).await?;
        Ok(Some(row))
    }

    /// Đăng ký khách tại quầy theo số điện thoại (`customers.manage` ở nút đăng ký). Số đã có hồ
    /// sơ → 409 `customers.phone_taken` kèm ID hồ sơ đó. Hồ sơ chưa thuộc tài khoản nào cho tới khi
    /// chủ số xác thực số.
    pub(crate) async fn register(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: Registration,
    ) -> Result<profile::Model, Error> {
        let mut problems = Vec::new();
        let phone = PhoneNumber::parse(&input.phone, DEFAULT_COUNTRY);
        if phone.is_none() {
            problems.push(FieldError::new(
                "phone",
                "phone",
                "must be a phone number like `0912345678` or `+84912345678`",
            ));
        }
        let details = self.check_details(tenant, input.details, None, true, &mut problems).await?;
        let phone = match phone {
            Some(phone) if problems.is_empty() => phone,
            _ => return Err(Error::validation(problems)),
        };
        let org = OrgId::from_uuid(input.org_id.unwrap_or_else(|| root_of(tenant.id()).as_uuid()));
        let key = self.phone_key(tenant.id(), &phone).await?;
        let transaction = tenant.database().begin_write().await?;
        principal.require_write_at(&transaction, &MANAGE, OrgTarget::new(org, None)).await?;
        if let Some(existing) = Self::by_phone_key(&transaction, tenant, &key).await? {
            return Err(errors::phone_taken(existing.id));
        }
        let mut row =
            new_row(tenant, org, None, Some(key.clone()), Some(phone.as_str().to_owned()), now());
        apply_details(&mut row, details);
        let Some(row) = self
            .insert_profile(&transaction, tenant, row, Actor::user(principal.user_id()), "counter")
            .await?
        else {
            drop(transaction);
            // Request khác vừa đăng ký cùng số.
            let existing = Self::by_phone_key(tenant.database().write(), tenant, &key).await?;
            return Err(existing.map_or_else(errors::busy, |row| errors::phone_taken(row.id)));
        };
        transaction.commit().await?;
        Ok(row)
    }

    /// Hồ sơ `id` mà người gọi được xem (`customers.read` ở nút của hồ sơ). Ngoài phạm vi: 404.
    pub(crate) async fn get(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<profile::Model, Error> {
        self.visible(tenant, principal, &READ, id).await
    }

    async fn visible(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        permission: &autoxide_kernel::access::Permission,
        id: Uuid,
    ) -> Result<profile::Model, Error> {
        let read = tenant.database().read();
        let row =
            self.find_profile(read, tenant, id).await?.ok_or_else(errors::profile_not_found)?;
        principal
            .require_at(read, permission, OrgTarget::new(OrgId::from_uuid(row.org_id), None))
            .await
            .map_err(|error| {
                if error.code() == "org_out_of_scope" { errors::profile_not_found() } else { error }
            })?;
        Ok(row)
    }

    /// Sửa hồ sơ `id` (`customers.manage`); `expected`: `If-Match`.
    pub(crate) async fn update(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        details: Details,
        expected: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<profile::Model, Error> {
        let current = self.visible(tenant, principal, &MANAGE, id).await?;
        let mut problems = Vec::new();
        let details =
            self.check_details(tenant, details, Some(&current), false, &mut problems).await?;
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        self.save_details(tenant, Actor::user(principal.user_id()), id, details, expected).await
    }

    /// Kiểm thông tin nhập; gom lỗi vào `problems` (D-083).
    async fn check_details(
        &self,
        tenant: &TenantContext,
        mut details: Details,
        current: Option<&profile::Model>,
        creating: bool,
        problems: &mut Vec<FieldError>,
    ) -> Result<Details, Error> {
        if let Some(name) = &details.full_name {
            let name = name.trim().to_owned();
            if name.chars().count() > 200 {
                problems.push(FieldError::new(
                    "full_name",
                    "length",
                    "must be at most 200 characters",
                ));
            }
            details.full_name = Some(name);
        }
        if let Some(email) = &details.email {
            details.email = if email.trim().is_empty() {
                Some(String::new())
            } else {
                autoxide_kernel::email::checked(email, "email", problems)
            };
        }
        if let Some(gender) = &details.gender
            && !gender.is_empty()
            && !GENDERS.contains(&gender.as_str())
        {
            problems.push(FieldError::new(
                "gender",
                "invalid",
                "must be `female`, `male` or `other`",
            ));
        }
        if let Some(birthday) = details.birthday
            && birthday > now().date_naive()
        {
            problems.push(FieldError::new("birthday", "invalid", "must not be in the future"));
        }
        if details.custom_fields.is_some() || creating {
            let stored: Option<Map<String, Value>> =
                current.and_then(|row| serde_json::from_str(&row.custom_fields).ok());
            match self
                .custom_fields
                .validate(
                    tenant,
                    CUSTOMER,
                    details.custom_fields.as_ref(),
                    stored.as_ref(),
                    creating,
                )
                .await
            {
                Ok(values) => details.custom_fields = Some(values),
                Err(error) if !error.fields().is_empty() => {
                    problems.extend(error.fields().iter().cloned());
                }
                Err(error) => return Err(error),
            }
        }
        Ok(details)
    }

    async fn save_details(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
        details: Details,
        expected: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<profile::Model, Error> {
        let transaction = tenant.database().begin_write().await?;
        let current = autoxide_kernel::rows::lock_one(
            &transaction,
            profile::Entity::find_by_id(id).scoped(tenant),
        )
        .await?
        .ok_or_else(errors::profile_not_found)?;
        if let Some(expected) = expected
            && expected != current.updated_at
        {
            return Err(autoxide_kernel::version::changed());
        }
        let mut row: profile::ActiveModel = current.clone().into();
        let changed = apply_details(&mut row, details);
        row.updated_at = Set(now());
        let updated = row.update(&transaction).await?;
        self.outbox
            .record(
                &transaction,
                tenant,
                actor,
                Some(subject(id)),
                &ProfileUpdated(Change::after(json!({"profile_id": id, "changed": changed}))),
            )
            .await?;
        transaction.commit().await?;
        Ok(updated)
    }

    /// Liên hệ đầy đủ của hồ sơ `id` (`customers.contact.reveal`); mỗi lần xem ghi event
    /// `customers.contact_revealed` → audit (người xem, hồ sơ, lý do).
    pub(crate) async fn reveal_contact(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        reason: Option<String>,
    ) -> Result<Contact, Error> {
        let row = self.visible(tenant, principal, &crate::permissions::CONTACT_REVEAL, id).await?;
        let reason = reason.map(|text| text.chars().take(200).collect::<String>());
        let transaction = tenant.database().begin_write().await?;
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::user(principal.user_id()),
                Some(subject(id)),
                &crate::events::ContactRevealed(Change::after(json!({
                    "profile_id": id, "viewer_id": principal.user_id(), "reason": reason,
                }))),
            )
            .await?;
        transaction.commit().await?;
        Ok(Contact { phone: row.phone, email: row.email })
    }

    /// Hồ sơ của chính người mua (tạo khi chưa có, ở nút gốc).
    pub(crate) async fn me(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<profile::Model, Error> {
        self.profile_for_user(tenant, user_id, root_of(tenant.id()), Actor::user(user_id)).await
    }

    /// Người mua tự sửa tên, email, ngày sinh, giới tính (không sửa số điện thoại — số đến từ bước
    /// xác thực của tài khoản).
    pub(crate) async fn update_me(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        mut details: Details,
    ) -> Result<profile::Model, Error> {
        let current = self.me(tenant, user_id).await?;
        // Trường tự định nghĩa nội bộ do nhân viên điền.
        details.custom_fields = None;
        let mut problems = Vec::new();
        let details =
            self.check_details(tenant, details, Some(&current), false, &mut problems).await?;
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        self.save_details(tenant, Actor::user(user_id), current.id, details, None).await
    }

    /// Điều kiện tìm theo `q`: số điện thoại (băm rồi so khoá), email hay tên không dấu.
    pub(crate) async fn search_condition(
        &self,
        tenant: &TenantContext,
        q: &str,
    ) -> Result<sea_orm::Condition, Error> {
        let q = q.trim();
        let digits = q.chars().filter(char::is_ascii_digit).count();
        if digits >= 8
            && let Some(phone) = PhoneNumber::parse(q, DEFAULT_COUNTRY)
        {
            let key = self.phone_key(tenant.id(), &phone).await?;
            return Ok(sea_orm::Condition::all().add(profile::Column::PhoneKey.eq(key)));
        }
        if q.contains('@') {
            return Ok(sea_orm::Condition::all()
                .add(profile::Column::EmailLower.eq(autoxide_kernel::email::normalize(q))));
        }
        Ok(sea_orm::Condition::all()
            .add(autoxide_kernel::search::contains(profile::Column::FullNameSearch, q)))
    }
}

/// `customers.customer:<id>` — chủ thể của event.
pub(crate) fn subject(id: Uuid) -> String {
    format!("{CUSTOMER}:{id}")
}

fn new_row(
    tenant: &TenantContext,
    org: OrgId,
    user_id: Option<Uuid>,
    phone_key: Option<String>,
    phone: Option<String>,
    at: chrono::DateTime<chrono::Utc>,
) -> profile::ActiveModel {
    profile::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant.id().as_uuid()),
        org_id: Set(org.as_uuid()),
        user_id: Set(user_id),
        phone_key: Set(phone_key),
        phone: Set(phone),
        email: Set(None),
        email_lower: Set(None),
        full_name: Set(None),
        full_name_search: Set(None),
        birthday: Set(None),
        gender: Set(None),
        tier_code: Set(None),
        currency: Set(None),
        total_spent_minor: Set(0),
        orders_count: Set(0),
        first_order_at: Set(None),
        last_order_at: Set(None),
        custom_fields: Set("{}".to_owned()),
        created_at: Set(at),
        updated_at: Set(at),
    }
}

/// Ghi `details` vào dòng; trả tên các trường đã đổi (cho event, không mang giá trị).
fn apply_details(row: &mut profile::ActiveModel, details: Details) -> Vec<&'static str> {
    let mut changed = Vec::new();
    if let Some(name) = details.full_name {
        let empty = name.is_empty();
        row.full_name_search = Set((!empty).then(|| autoxide_kernel::search::fold(&name)));
        row.full_name = Set((!empty).then_some(name));
        changed.push("full_name");
    }
    if let Some(email) = details.email {
        let empty = email.is_empty();
        row.email_lower = Set((!empty).then(|| autoxide_kernel::email::normalize(&email)));
        row.email = Set((!empty).then_some(email));
        changed.push("email");
    }
    if let Some(birthday) = details.birthday {
        row.birthday = Set(Some(birthday));
        changed.push("birthday");
    }
    if let Some(gender) = details.gender {
        row.gender = Set((!gender.is_empty()).then_some(gender));
        changed.push("gender");
    }
    if let Some(values) = details.custom_fields {
        row.custom_fields = Set(Value::Object(values).to_string());
        changed.push("custom_fields");
    }
    changed
}
