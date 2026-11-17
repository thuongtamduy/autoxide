//! Nguồn tìm xuyên phân hệ (bước 51 P3): nhân viên (không gồm tài khoản người mua) có email hay tên
//! chứa từ tìm (cột `search_text`, không dấu), cho người có `auth.users.read`.

use autoxide_kernel::access::{OrgTarget, Permission, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::find::{FinderSource, Found};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

use crate::permissions::USERS_READ;
use crate::user::entity::{Column, Entity, realm_str};

pub(crate) struct Users;

#[async_trait::async_trait]
impl FinderSource for Users {
    fn kind(&self) -> &'static str {
        "auth.user"
    }

    fn permission(&self) -> Permission {
        USERS_READ
    }

    async fn find(
        &self,
        tenant: &TenantContext,
        _: &Principal,
        _: OrgTarget,
        query: &str,
        limit: u16,
    ) -> Result<Vec<Found>, Error> {
        let rows = Entity::find()
            .filter(Column::Realm.eq(realm_str(autoxide_kernel::access::Realm::Staff)))
            .scoped(tenant)
            .filter(autoxide_kernel::search::contains(Column::SearchText, query))
            .order_by_desc(Column::Id)
            .limit(u64::from(limit))
            .all(tenant.database().read())
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                Found::new(row.id.to_string(), row.name, "organization.access").subtitle(row.email)
            })
            .collect())
    }
}
