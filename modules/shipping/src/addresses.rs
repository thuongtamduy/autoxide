//! Sổ địa chỉ của người mua (bước 44 mục 1). Địa chỉ Việt Nam kiểm bằng `AdministrativeUnits` của
//! module `address` (xã thuộc tỉnh, còn hiệu lực — D-193); địa chỉ nước ngoài dạng chung (vùng,
//! thành phố, mã bưu chính), không kiểm danh mục. Mỗi người mua có đúng một địa chỉ mặc định khi sổ
//! không rỗng: cột `default_key` (unique) giữ điều đó kể cả khi hai lệnh chạy cùng lúc.
//!
//! Sổ địa chỉ là dữ liệu cá nhân (Luật 91/2025/QH15): chỉ người mua và nhân viên có
//! `shipping.addresses.read` đọc được; xuất và xoá theo yêu cầu ở `privacy.rs`.

use autoxide_address::api::{Address, AddressInput};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, Set,
};
use serde_json::json;
use uuid::Uuid;

use crate::api::{AddressBook, BuyerAddress};
use crate::entity::address;
use crate::errors;
use crate::service::{ShippingService, check_length, now};

/// Một địa chỉ bưu chính người gọi gửi.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PostalInput {
    /// ISO 3166-1 alpha-2.
    pub country: String,
    /// Việt Nam: mã tỉnh. Nước ngoài: vùng (bang, tỉnh…).
    pub province_code: String,
    /// Việt Nam: mã xã.
    pub commune_code: Option<String>,
    /// Nước ngoài: thành phố.
    pub city: Option<String>,
    pub line: String,
    pub postal_code: Option<String>,
}

/// Nội dung một địa chỉ của sổ.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AddressDraft {
    pub label: String,
    pub recipient_name: String,
    pub phone: String,
    pub address: PostalInput,
    pub is_default: bool,
}

/// Sửa một phần.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AddressPatch {
    pub label: Option<String>,
    pub recipient_name: Option<String>,
    pub phone: Option<String>,
    pub address: Option<PostalInput>,
}

/// Địa chỉ đã kiểm: `Address` + mã bưu chính.
pub(crate) struct CheckedAddress {
    pub address: Address,
    pub postal_code: Option<String>,
}

/// Dựng `Address` (kiểu của module `address`, `#[non_exhaustive]` nên không dựng trực tiếp được
/// từ crate này) từ các cột đã lưu.
pub(crate) fn address_of(
    country: &str,
    province_code: &str,
    province_name: &str,
    commune_code: &str,
    commune_name: &str,
    line: &str,
) -> Result<Address, Error> {
    serde_json::from_value(json!({
        "country": country,
        "province_code": province_code,
        "province_name": province_name,
        "commune_code": commune_code,
        "commune_name": commune_name,
        "line": line,
    }))
    .map_err(|error| Error::internal(format!("stored address cannot be read: {error}")))
}

/// Số điện thoại người nhận: chữ số, có thể bắt đầu bằng `+`, 6 tới 20 ký tự (không chuẩn hoá thêm:
/// hãng vận chuyển tự chuẩn hoá theo nước).
pub(crate) fn check_phone(problems: &mut Vec<FieldError>, field: &str, phone: &str) {
    let digits = phone.strip_prefix('+').unwrap_or(phone);
    let valid = (6..=20).contains(&phone.len())
        && !digits.is_empty()
        && digits.bytes().all(|byte| byte.is_ascii_digit() || byte == b' ' || byte == b'.');
    if !valid {
        problems.push(FieldError::new(
            field,
            "phone",
            "must be a phone number: digits, optionally starting with `+`",
        ));
    }
}

fn to_api(row: address::Model) -> Result<BuyerAddress, Error> {
    let address = address_of(
        &row.country,
        &row.province_code,
        &row.province_name,
        &row.commune_code,
        &row.commune_name,
        &row.line,
    )?;
    Ok(BuyerAddress {
        id: row.id,
        buyer_id: row.buyer_id,
        label: row.label,
        recipient_name: row.recipient_name,
        phone: row.phone,
        address,
        postal_code: row.postal_code,
        is_default: row.is_default,
        updated_at: row.updated_at,
    })
}

impl ShippingService {
    /// Kiểm một địa chỉ bưu chính; lỗi gắn tiền tố `field` (vd `address.`).
    pub(crate) async fn check_address(
        &self,
        input: &PostalInput,
        field: &str,
    ) -> Result<CheckedAddress, Vec<FieldError>> {
        let mut problems = Vec::new();
        let country = input.country.trim().to_ascii_uppercase();
        if country.len() != 2 || !country.bytes().all(|byte| byte.is_ascii_uppercase()) {
            problems.push(FieldError::new(
                format!("{field}country"),
                "country",
                "must be a two-letter ISO 3166-1 country code like `VN`",
            ));
            return Err(problems);
        }
        let line = input.line.trim();
        check_length(&mut problems, &format!("{field}line"), line, 1, 300);
        let postal_code =
            input.postal_code.as_deref().map(str::trim).filter(|code| !code.is_empty());
        if let Some(code) = postal_code {
            check_length(&mut problems, &format!("{field}postal_code"), code, 1, 20);
        }
        if country == "VN" {
            let Some(units) = &self.units else {
                problems.push(FieldError::new(
                    format!("{field}country"),
                    "unsupported",
                    "addresses in Vietnam cannot be checked: the `address` module is not installed",
                ));
                return Err(problems);
            };
            let commune = input.commune_code.as_deref().unwrap_or_default().trim();
            if commune.is_empty() {
                problems.push(FieldError::new(
                    format!("{field}commune_code"),
                    "required",
                    "is required for an address in Vietnam",
                ));
            }
            if !problems.is_empty() {
                return Err(problems);
            }
            let checked = units
                .validate(AddressInput::new("VN", input.province_code.trim(), commune, line), field)
                .await?;
            return Ok(CheckedAddress {
                address: checked,
                postal_code: postal_code.map(str::to_owned),
            });
        }
        let region = input.province_code.trim();
        let city = input.city.as_deref().unwrap_or_default().trim();
        check_length(&mut problems, &format!("{field}province_code"), region, 1, 64);
        check_length(&mut problems, &format!("{field}city"), city, 1, 200);
        if !problems.is_empty() {
            return Err(problems);
        }
        let address = address_of(&country, region, region, "", city, line).map_err(|_| {
            vec![FieldError::new(field.trim_end_matches('.'), "invalid", "is not valid")]
        })?;
        Ok(CheckedAddress { address, postal_code: postal_code.map(str::to_owned) })
    }

    /// Kiểm phần không phải địa chỉ của một mục sổ.
    fn check_contact(problems: &mut Vec<FieldError>, label: &str, name: &str, phone: &str) {
        check_length(problems, "label", label, 0, 100);
        check_length(problems, "recipient_name", name, 1, 200);
        check_phone(problems, "phone", phone);
    }

    /// Địa chỉ của người mua, mới trước (theo `order` của trang).
    pub(crate) async fn list_addresses(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
        page: &PageRequest,
    ) -> Result<Page<BuyerAddress>, Error> {
        let select =
            address::Entity::find().filter(address::Column::BuyerId.eq(buyer)).scoped(tenant);
        let page =
            fetch_page(select, address::Column::Id, |row| row.id, page, tenant.database().read())
                .await?;
        let mut data = Vec::with_capacity(page.data.len());
        for row in page.data {
            data.push(to_api(row)?);
        }
        Ok(Page { data, pagination: page.pagination })
    }

    /// Địa chỉ `id` của người mua `buyer`; của người khác → 404.
    pub(crate) async fn get_address(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
        id: Uuid,
    ) -> Result<BuyerAddress, Error> {
        let row = find(tenant.database().read(), tenant, buyer, id).await?;
        to_api(row)
    }

    /// Thêm địa chỉ. Địa chỉ đầu tiên luôn là mặc định.
    pub(crate) async fn create_address(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
        draft: AddressDraft,
    ) -> Result<BuyerAddress, Error> {
        let mut problems = Vec::new();
        let label = draft.label.trim().to_owned();
        let name = draft.recipient_name.trim().to_owned();
        let phone = draft.phone.trim().to_owned();
        Self::check_contact(&mut problems, &label, &name, &phone);
        let checked = match self.check_address(&draft.address, "address.").await {
            Ok(checked) => Some(checked),
            Err(fields) => {
                problems.extend(fields);
                None
            }
        };
        let Some(checked) = checked.filter(|_| problems.is_empty()) else {
            return Err(errors::invalid_address(problems));
        };

        let transaction = tenant.database().begin_write().await?;
        let count = address::Entity::find()
            .filter(address::Column::BuyerId.eq(buyer))
            .scoped(tenant)
            .count(&transaction)
            .await?;
        if count >= self.config.max_addresses_per_buyer {
            return Err(errors::address_limit(self.config.max_addresses_per_buyer));
        }
        let is_default = draft.is_default || count == 0;
        if is_default {
            clear_default(&transaction, tenant, buyer).await?;
        }
        let at = now();
        let id = Uuid::now_v7();
        let address = checked.address;
        address::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            buyer_id: Set(buyer),
            label: Set(label),
            recipient_name: Set(name),
            phone: Set(phone),
            country: Set(address.country.clone()),
            province_code: Set(address.province_code.clone()),
            province_name: Set(address.province_name.clone()),
            commune_code: Set(address.commune_code.clone()),
            commune_name: Set(address.commune_name.clone()),
            line: Set(address.line.clone()),
            postal_code: Set(checked.postal_code),
            is_default: Set(is_default),
            default_key: Set(is_default.then_some(buyer)),
            created_at: Set(at),
            updated_at: Set(at),
        }
        .insert(&transaction)
        .await?;
        let row = find(&transaction, tenant, buyer, id).await?;
        transaction.commit().await?;
        to_api(row)
    }

    /// Sửa địa chỉ `id`. Vận đơn đã tạo giữ bản chụp riêng, không đổi theo.
    pub(crate) async fn update_address(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
        id: Uuid,
        patch: AddressPatch,
    ) -> Result<BuyerAddress, Error> {
        let mut problems = Vec::new();
        let current = find(tenant.database().read(), tenant, buyer, id).await?;
        let label = patch.label.map_or(current.label.clone(), |label| label.trim().to_owned());
        let name = patch
            .recipient_name
            .map_or(current.recipient_name.clone(), |name| name.trim().to_owned());
        let phone = patch.phone.map_or(current.phone.clone(), |phone| phone.trim().to_owned());
        Self::check_contact(&mut problems, &label, &name, &phone);
        let checked = match &patch.address {
            Some(input) => match self.check_address(input, "address.").await {
                Ok(checked) => Some(checked),
                Err(fields) => {
                    problems.extend(fields);
                    None
                }
            },
            None => None,
        };
        if !problems.is_empty() {
            return Err(errors::invalid_address(problems));
        }

        let transaction = tenant.database().begin_write().await?;
        let row = find(&transaction, tenant, buyer, id).await?;
        let mut active: address::ActiveModel = row.into();
        active.label = Set(label);
        active.recipient_name = Set(name);
        active.phone = Set(phone);
        if let Some(checked) = checked {
            let address = checked.address;
            active.country = Set(address.country.clone());
            active.province_code = Set(address.province_code.clone());
            active.province_name = Set(address.province_name.clone());
            active.commune_code = Set(address.commune_code.clone());
            active.commune_name = Set(address.commune_name.clone());
            active.line = Set(address.line.clone());
            active.postal_code = Set(checked.postal_code);
        }
        active.updated_at = Set(now());
        let row = active.update(&transaction).await?;
        transaction.commit().await?;
        to_api(row)
    }

    /// Đặt địa chỉ `id` làm mặc định (bỏ mặc định của các địa chỉ khác, cùng transaction).
    pub(crate) async fn set_default_address(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
        id: Uuid,
    ) -> Result<BuyerAddress, Error> {
        let transaction = tenant.database().begin_write().await?;
        find(&transaction, tenant, buyer, id).await?;
        clear_default(&transaction, tenant, buyer).await?;
        address::Entity::update_many()
            .col_expr(address::Column::IsDefault, Expr::value(true))
            .col_expr(address::Column::DefaultKey, Expr::value(Some(buyer)))
            .col_expr(address::Column::UpdatedAt, Expr::value(now()))
            .filter(address::Column::Id.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let row = find(&transaction, tenant, buyer, id).await?;
        transaction.commit().await?;
        to_api(row)
    }

    /// Xoá địa chỉ `id`. Xoá địa chỉ mặc định thì địa chỉ sửa gần nhất còn lại thành mặc định.
    pub(crate) async fn delete_address(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
        id: Uuid,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        let row = find(&transaction, tenant, buyer, id).await?;
        address::Entity::delete_many()
            .filter(address::Column::Id.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if row.is_default {
            let next = address::Entity::find()
                .filter(address::Column::BuyerId.eq(buyer))
                .scoped(tenant)
                .order_by_desc(address::Column::UpdatedAt)
                .order_by_desc(address::Column::Id)
                .one(&transaction)
                .await?;
            if let Some(next) = next {
                address::Entity::update_many()
                    .col_expr(address::Column::IsDefault, Expr::value(true))
                    .col_expr(address::Column::DefaultKey, Expr::value(Some(buyer)))
                    .filter(address::Column::Id.eq(next.id))
                    .scoped(tenant)
                    .exec(&transaction)
                    .await?;
            }
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Mọi địa chỉ của người mua (xuất dữ liệu cá nhân), cũ trước.
    pub(crate) async fn all_addresses(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
    ) -> Result<Vec<BuyerAddress>, Error> {
        let rows = address::Entity::find()
            .filter(address::Column::BuyerId.eq(buyer))
            .scoped(tenant)
            .order_by_asc(address::Column::Id)
            .all(tenant.database().read())
            .await?;
        rows.into_iter().map(to_api).collect()
    }

    /// Xoá mọi địa chỉ của người mua (tài khoản bị xoá). Gọi lại không sao.
    pub(crate) async fn erase_addresses(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
    ) -> Result<u64, Error> {
        let result = address::Entity::delete_many()
            .filter(address::Column::BuyerId.eq(buyer))
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        Ok(result.rows_affected)
    }
}

/// Bỏ cờ mặc định của mọi địa chỉ của người mua.
async fn clear_default<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    buyer: Uuid,
) -> Result<(), Error> {
    address::Entity::update_many()
        .col_expr(address::Column::IsDefault, Expr::value(false))
        .col_expr(address::Column::DefaultKey, Expr::value(Option::<Uuid>::None))
        .filter(address::Column::BuyerId.eq(buyer))
        .scoped(tenant)
        .exec(connection)
        .await?;
    Ok(())
}

async fn find<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    buyer: Uuid,
    id: Uuid,
) -> Result<address::Model, Error> {
    address::Entity::find_by_id(id)
        .filter(address::Column::BuyerId.eq(buyer))
        .scoped(tenant)
        .one(connection)
        .await?
        .ok_or_else(|| errors::address_not_found(id))
}

#[async_trait::async_trait]
impl AddressBook for ShippingService {
    async fn get(
        &self,
        tenant: &TenantContext,
        buyer_id: Uuid,
        id: Uuid,
    ) -> Result<BuyerAddress, Error> {
        self.get_address(tenant, buyer_id, id).await
    }

    async fn default_for(
        &self,
        tenant: &TenantContext,
        buyer_id: Uuid,
    ) -> Result<Option<BuyerAddress>, Error> {
        let row = address::Entity::find()
            .filter(address::Column::BuyerId.eq(buyer_id))
            .filter(address::Column::IsDefault.eq(true))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        row.map(to_api).transpose()
    }
}
