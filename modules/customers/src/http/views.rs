//! Hình dạng response của `customers` (tên kiểu tiếng Anh cho OpenAPI). Nhân viên luôn nhận liên hệ
//! đã che (D-226); chỉ `GET /{id}/contact` và `/me` trả liên hệ đầy đủ.

use autoxide_kernel::error::Error;
use autoxide_kernel::money::Money;
use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use serde_json::{Map, Value};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::{group, order_fact, profile};
use crate::segments::{GroupRule, GroupView};
use crate::service::{mask_email, mask_phone};

/// A customer profile, as staff see it: phone and email are masked.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = CustomerProfile)]
pub(crate) struct CustomerResponse {
    pub id: Uuid,
    /// Unit where the customer registered or first bought.
    pub org_id: Uuid,
    /// The buyer account of the profile, if linked.
    pub user_id: Option<Uuid>,
    pub full_name: Option<String>,
    /// Masked phone, e.g. `09•• ••• 412`; `GET /customers/{id}/contact` shows it in full.
    pub phone_masked: Option<String>,
    /// Masked email, e.g. `n•••@gmail.com`.
    pub email_masked: Option<String>,
    pub birthday: Option<NaiveDate>,
    /// `female`, `male` or `other`.
    pub gender: Option<String>,
    /// Membership tier code.
    pub tier: Option<String>,
    /// Paid for goods of completed orders minus refunds (after discounts, with tax, without
    /// shipping). Absent before the first completed order.
    pub total_spent: Option<Money>,
    /// Orders placed and not cancelled.
    pub orders_count: i64,
    pub first_order_at: Option<DateTime<Utc>>,
    pub last_order_at: Option<DateTime<Utc>>,
    /// Values of the shop's custom fields for customers.
    #[schema(value_type = Object)]
    pub custom_fields: Map<String, Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn spent(row: &profile::Model) -> Result<Option<Money>, Error> {
    row.currency.as_deref().map(|code| Money::from_columns(row.total_spent_minor, code)).transpose()
}

fn fields(row: &profile::Model) -> Map<String, Value> {
    serde_json::from_str(&row.custom_fields).unwrap_or_default()
}

impl CustomerResponse {
    pub(crate) fn new(row: profile::Model) -> Result<Self, Error> {
        Ok(CustomerResponse {
            total_spent: spent(&row)?,
            custom_fields: fields(&row),
            phone_masked: row.phone.as_deref().map(mask_phone),
            email_masked: row.email.as_deref().map(mask_email),
            id: row.id,
            org_id: row.org_id,
            user_id: row.user_id,
            full_name: row.full_name,
            birthday: row.birthday,
            gender: row.gender,
            tier: row.tier_code,
            orders_count: row.orders_count,
            first_order_at: row.first_order_at,
            last_order_at: row.last_order_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

/// The buyer's own profile: nothing is masked.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct MyCustomerProfile {
    pub id: Uuid,
    pub full_name: Option<String>,
    /// The verified phone of the account, in E.164 form.
    pub phone: Option<String>,
    pub email: Option<String>,
    pub birthday: Option<NaiveDate>,
    pub gender: Option<String>,
    pub tier: Option<String>,
    pub total_spent: Option<Money>,
    pub orders_count: i64,
    pub created_at: DateTime<Utc>,
}

impl MyCustomerProfile {
    pub(crate) fn new(row: profile::Model) -> Result<Self, Error> {
        Ok(MyCustomerProfile {
            total_spent: spent(&row)?,
            id: row.id,
            full_name: row.full_name,
            phone: row.phone,
            email: row.email,
            birthday: row.birthday,
            gender: row.gender,
            tier: row.tier_code,
            orders_count: row.orders_count,
            created_at: row.created_at,
        })
    }
}

/// Full contact details of a customer.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct CustomerContact {
    /// E.164 form, e.g. `+84912345678`.
    pub phone: Option<String>,
    pub email: Option<String>,
}

/// One order in a customer's purchase history.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct CustomerOrder {
    pub order_id: Uuid,
    pub number: String,
    /// Selling unit.
    pub org_id: Uuid,
    /// `web`, `pos` or a channel code.
    pub channel: String,
    /// Order status as last reported by orders.
    pub status: String,
    /// Paid for goods: after discounts, with tax, without shipping.
    pub net: Money,
    pub refunded: Money,
    pub placed_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

impl CustomerOrder {
    pub(crate) fn new(row: order_fact::Model) -> Result<Self, Error> {
        Ok(CustomerOrder {
            net: Money::from_columns(row.net_minor, &row.currency)?,
            refunded: Money::from_columns(row.refunded_minor, &row.currency)?,
            order_id: row.order_id,
            number: row.number,
            org_id: row.org_id,
            channel: row.channel,
            status: row.status,
            placed_at: row.placed_at,
            completed_at: row.completed_at,
        })
    }
}

/// A customer group.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct CustomerGroupResponse {
    pub id: Uuid,
    /// Unit of the group; the root unit means the whole shop.
    pub org_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    /// `manual` (staff add members) or `rule` (members follow the rule).
    pub kind: String,
    pub rule: Option<GroupRule>,
    /// Customer group code for price lists, if any.
    pub price_group: Option<String>,
    /// Number of members (only when reading one group).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub members: Option<u64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<group::Model> for CustomerGroupResponse {
    fn from(row: group::Model) -> Self {
        CustomerGroupResponse {
            rule: row.rule.as_deref().and_then(|text| serde_json::from_str(text).ok()),
            id: row.id,
            org_id: row.org_id,
            name: row.name,
            description: row.description,
            kind: row.kind,
            price_group: row.price_group,
            members: None,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

impl From<GroupView> for CustomerGroupResponse {
    fn from(view: GroupView) -> Self {
        let mut response = CustomerGroupResponse::from(view.row);
        response.members = Some(view.members);
        response
    }
}

/// How many customers match a group now.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct CustomerGroupPreview {
    pub matching: u64,
}

/// Đổi từng dòng của trang bằng hàm có thể lỗi, giữ thông tin trang.
pub(crate) fn try_page<T, U>(
    page: autoxide_kernel::pagination::Page<T>,
    convert: impl FnMut(T) -> Result<U, Error>,
) -> Result<autoxide_kernel::pagination::Page<U>, Error> {
    Ok(autoxide_kernel::pagination::Page {
        data: page.data.into_iter().map(convert).collect::<Result<_, _>>()?,
        pagination: page.pagination,
    })
}
