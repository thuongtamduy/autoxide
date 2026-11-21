//! Hàng số — phần người mua và người quản lý dùng quyền tải: trừ lượt, ký và phục vụ địa chỉ file,
//! quyền của người mua đang đăng nhập, xem / thu hồi / cấp lại token theo đơn. Phần gắn file và
//! phát / thu hồi theo event ở `digital.rs`.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::rows::{self, Bounds};
use autoxide_kernel::storage::Storage as _;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::Duration;
use hmac::{Hmac, KeyInit, Mac};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};
use sha2::Sha256;
use uuid::Uuid;

use crate::digital::{FileLink, GrantView, ServedFile, download_not_found, limit_reached};
use crate::entity::{digital_asset, download_grant};
use crate::permissions::DOWNLOADS_MANAGE;
use crate::service::{CatalogService, now, readable};

type HmacSha256 = Hmac<Sha256>;

/// Nhãn dẫn khoá ký địa chỉ file (theo khách).
const LINK_PURPOSE: &str = "autoxide/catalog/download-link/v1";

impl CatalogService {
    /// Dùng một lượt của token `token` (không cần đăng nhập).
    pub(crate) async fn use_token(
        &self,
        tenant: &TenantContext,
        token: &str,
    ) -> Result<FileLink, Error> {
        let hash = autoxide_kernel::secret::sha256(token);
        let grant = download_grant::Entity::find()
            .filter(download_grant::Column::TokenHash.eq(hash))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(download_not_found)?;
        self.spend(tenant, grant).await
    }

    /// Dùng một lượt của quyền `grant_id` của người mua đang đăng nhập.
    pub(crate) async fn use_own_grant(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        grant_id: Uuid,
    ) -> Result<FileLink, Error> {
        let user = principal.user_id();
        let grant = download_grant::Entity::find_by_id(grant_id)
            .filter(download_grant::Column::BuyerId.eq(user))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(download_not_found)?;
        self.spend(tenant, grant).await
    }

    /// Trừ một lượt (một câu `UPDATE` có điều kiện) rồi ký địa chỉ file.
    async fn spend(
        &self,
        tenant: &TenantContext,
        grant: download_grant::Model,
    ) -> Result<FileLink, Error> {
        let write = tenant.database().write();
        let usable = sea_orm::Condition::all()
            .add(download_grant::Column::Id.eq(grant.id))
            .add(download_grant::Column::TenantId.eq(tenant.id().as_uuid()))
            .add(download_grant::Column::RevokedAt.is_null())
            .add(download_grant::Column::ExpiresAt.gt(now()));
        let changed = rows::adjust::<download_grant::Entity, _>(
            write,
            download_grant::Column::Remaining,
            usable,
            -1,
            Bounds::NON_NEGATIVE,
        )
        .await?;
        let current = download_grant::Entity::find_by_id(grant.id)
            .scoped(tenant)
            .one(write)
            .await?
            .ok_or_else(download_not_found)?;
        if changed == 0 {
            let alive = current.revoked_at.is_none() && current.expires_at > now();
            return Err(if alive && current.remaining == 0 {
                limit_reached()
            } else {
                download_not_found()
            });
        }
        let asset = digital_asset::Entity::find_by_id(current.asset_id)
            .scoped(tenant)
            .one(write)
            .await?
            .ok_or_else(download_not_found)?;
        let expires_at = now() + Duration::seconds(self.config.download_link_seconds);
        let signature = self.link_signature(tenant, current.id, expires_at.timestamp()).await?;
        let url = format!(
            "{}/catalog/public/downloads/{}/file?exp={}&sig={signature}",
            autoxide_kernel::http::API_PREFIX,
            current.id,
            expires_at.timestamp(),
        );
        metrics::counter!("catalog_downloads_total").increment(1);
        Ok(FileLink { url, expires_at, remaining: current.remaining, file_name: asset.file_name })
    }

    async fn link_mac(
        &self,
        tenant: &TenantContext,
        grant_id: Uuid,
        expires: i64,
    ) -> Result<HmacSha256, Error> {
        let key = self.secrets.derive_for(tenant.id(), LINK_PURPOSE).await?;
        let mut mac = <HmacSha256 as KeyInit>::new_from_slice(&key)
            .map_err(|error| Error::internal(format!("hmac: {error}")))?;
        mac.update(grant_id.as_bytes());
        mac.update(&expires.to_be_bytes());
        Ok(mac)
    }

    async fn link_signature(
        &self,
        tenant: &TenantContext,
        grant_id: Uuid,
        expires: i64,
    ) -> Result<String, Error> {
        let mac = self.link_mac(tenant, grant_id, expires).await?;
        Ok(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
    }

    /// File của địa chỉ ký (`exp`, `sig`) cho quyền `grant_id`; không trừ lượt. `range`: header
    /// `Range` thô nếu có. Chữ ký sai, hết hạn, quyền đã thu hồi: `catalog.download_not_found`.
    pub(crate) async fn signed_file(
        &self,
        tenant: &TenantContext,
        grant_id: Uuid,
        expires: i64,
        signature: &str,
        range: Option<&str>,
    ) -> Result<ServedFile, Error> {
        let signature = URL_SAFE_NO_PAD.decode(signature).map_err(|_| download_not_found())?;
        // `verify_slice` so sánh trong thời gian không đổi: không lộ dần chữ ký đúng.
        self.link_mac(tenant, grant_id, expires)
            .await?
            .verify_slice(&signature)
            .map_err(|_| download_not_found())?;
        if expires < now().timestamp() {
            return Err(download_not_found());
        }
        let read = tenant.database().read();
        let grant = download_grant::Entity::find_by_id(grant_id)
            .scoped(tenant)
            .one(read)
            .await?
            .filter(|grant| grant.revoked_at.is_none())
            .ok_or_else(download_not_found)?;
        let asset = digital_asset::Entity::find_by_id(grant.asset_id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(download_not_found)?;
        let size = u64::try_from(asset.size).unwrap_or_default();
        let range = match range.map(|raw| parse_range(raw, size)) {
            Some(RangeAsk::Unsatisfiable) => return Err(range_not_satisfiable(size)),
            Some(RangeAsk::Part(range)) => Some(range),
            Some(RangeAsk::Ignore) | None => None,
        };
        let store = self.digital_store(tenant)?;
        let file = match &range {
            Some(range) => store.get_range(&asset.storage_key, range.clone()).await?,
            None => store.get(&asset.storage_key).await?,
        }
        .ok_or_else(|| {
            Error::internal(format!("the copy of digital file {} is missing", asset.id))
        })?;
        Ok(ServedFile { asset, range, file })
    }

    /// Một trang quyền tải của người mua đang đăng nhập, mới trước.
    pub(crate) async fn own_grants(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<GrantView>, Error> {
        let user = principal.user_id();
        let read = tenant.database().read();
        let select = download_grant::Entity::find()
            .filter(download_grant::Column::BuyerId.eq(user))
            .scoped(tenant);
        let page = fetch_page(select, download_grant::Column::Id, |row| row.id, page, read).await?;
        let names = self.file_names(tenant, page.data.iter().map(|grant| grant.asset_id)).await?;
        Ok(page.map(|grant| {
            let file_name = names.get(&grant.asset_id).cloned().unwrap_or_default();
            GrantView { grant, file_name, token: None }
        }))
    }

    async fn file_names(
        &self,
        tenant: &TenantContext,
        ids: impl Iterator<Item = Uuid>,
    ) -> Result<HashMap<Uuid, String>, Error> {
        let ids: HashSet<Uuid> = ids.collect();
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        Ok(digital_asset::Entity::find()
            .filter(digital_asset::Column::Id.is_in(ids))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?
            .into_iter()
            .map(|asset| (asset.id, asset.file_name))
            .collect())
    }

    /// Quyền tải của đơn `order_id` mà người quản lý được xem (`catalog.downloads.manage` phủ nút
    /// sở hữu sản phẩm). Không thấy quyền nào: `catalog.download_not_found`.
    async fn managed_grants<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        principal: &Principal,
        order_id: Uuid,
    ) -> Result<Vec<download_grant::Model>, Error> {
        principal.require(&DOWNLOADS_MANAGE)?;
        let all = download_grant::Entity::find()
            .filter(download_grant::Column::OrderId.eq(order_id))
            .scoped(tenant)
            .order_by_asc(download_grant::Column::Id)
            .all(connection)
            .await?;
        let mut visible = Vec::new();
        for grant in all {
            let allowed = readable(principal, connection, &DOWNLOADS_MANAGE, grant.org_id, || {
                download_not_found()
            })
            .await;
            match allowed {
                Ok(()) => visible.push(grant),
                Err(error) if error.code() == "catalog.download_not_found" => {}
                Err(error) => return Err(error),
            }
        }
        if visible.is_empty() { Err(download_not_found()) } else { Ok(visible) }
    }

    /// Quyền tải của đơn cho người quản lý.
    pub(crate) async fn order_grants(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        order_id: Uuid,
    ) -> Result<Vec<GrantView>, Error> {
        let read = tenant.database().read();
        let grants = self.managed_grants(read, tenant, principal, order_id).await?;
        let names = self.file_names(tenant, grants.iter().map(|grant| grant.asset_id)).await?;
        Ok(grants
            .into_iter()
            .map(|grant| {
                let file_name = names.get(&grant.asset_id).cloned().unwrap_or_default();
                GrantView { grant, file_name, token: None }
            })
            .collect())
    }

    /// Người quản lý thu hồi các quyền tải của đơn mà họ quản lý. Trả số quyền vừa thu hồi.
    pub(crate) async fn revoke_order_grants(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        order_id: Uuid,
    ) -> Result<u64, Error> {
        let read = tenant.database().read();
        let ids = self.managed_grants(read, tenant, principal, order_id).await?;
        let ids = ids.into_iter().map(|grant| grant.id).collect();
        self.revoke_downloads(tenant, Actor::from(principal), order_id, Some(ids), "staff").await
    }

    /// Người quản lý cấp token mới cho các quyền tải còn dùng được của đơn (vd gửi lại link cho
    /// người mua không có tài khoản). Token cũ hết dùng; lượt còn lại và hạn giữ nguyên.
    pub(crate) async fn reissue_order_grants(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        order_id: Uuid,
    ) -> Result<Vec<GrantView>, Error> {
        let transaction = tenant.database().begin_write().await?;
        let grants = self.managed_grants(&transaction, tenant, principal, order_id).await?;
        let mut views = Vec::new();
        for grant in grants {
            if grant.revoked_at.is_some() || grant.expires_at <= now() {
                continue;
            }
            let token = autoxide_kernel::secret::random_token(32)?;
            let mut row: download_grant::ActiveModel = grant.into();
            row.token_hash = Set(autoxide_kernel::secret::sha256(&token));
            let grant = row.update(&transaction).await?;
            views.push(GrantView { grant, file_name: String::new(), token: Some(token) });
        }
        transaction.commit().await?;
        let names = self.file_names(tenant, views.iter().map(|view| view.grant.asset_id)).await?;
        for view in &mut views {
            view.file_name = names.get(&view.grant.asset_id).cloned().unwrap_or_default();
        }
        tracing::info!(order_id = %order_id, count = views.len(), "download tokens reissued");
        Ok(views)
    }
}

/// Header `Range` đã đọc.
#[derive(Debug, PartialEq, Eq)]
enum RangeAsk {
    /// Một đoạn hợp lệ (`start..end`, không gồm `end`).
    Part(Range<u64>),
    /// Cú pháp không hiểu hay nhiều đoạn: bỏ qua, gửi cả file (RFC 9110 cho phép).
    Ignore,
    /// Đoạn nằm ngoài file: 416.
    Unsatisfiable,
}

/// Đọc `bytes=a-b`, `bytes=a-`, `bytes=-n` (một đoạn) cho file `size` byte.
fn parse_range(raw: &str, size: u64) -> RangeAsk {
    let Some(spec) = raw.trim().strip_prefix("bytes=") else { return RangeAsk::Ignore };
    if spec.contains(',') {
        return RangeAsk::Ignore;
    }
    let Some((first, last)) = spec.trim().split_once('-') else { return RangeAsk::Ignore };
    let number = |text: &str| text.trim().parse::<u64>().ok();
    let range = match (first.trim().is_empty(), last.trim().is_empty()) {
        // `bytes=-n`: n byte cuối.
        (true, false) => match number(last) {
            Some(0) | None => return RangeAsk::Unsatisfiable,
            Some(suffix) => size.saturating_sub(suffix)..size,
        },
        (false, true) => match number(first) {
            Some(start) => start..size,
            None => return RangeAsk::Ignore,
        },
        (false, false) => match (number(first), number(last)) {
            (Some(start), Some(end)) if start <= end => start..end.saturating_add(1).min(size),
            _ => return RangeAsk::Ignore,
        },
        (true, true) => return RangeAsk::Ignore,
    };
    if range.start >= range.end { RangeAsk::Unsatisfiable } else { RangeAsk::Part(range) }
}

pub(crate) fn range_not_satisfiable(size: u64) -> Error {
    Error::new(
        ErrorKind::RangeNotSatisfiable,
        "catalog.range_not_satisfiable",
        "The requested range is outside the file ({size} bytes).",
    )
    .with_param("size", size)
}

#[cfg(test)]
mod tests {
    use super::{RangeAsk, parse_range};

    #[test]
    fn doc_header_range() {
        assert_eq!(parse_range("bytes=0-9", 100), RangeAsk::Part(0..10));
        assert_eq!(parse_range("bytes=90-", 100), RangeAsk::Part(90..100));
        assert_eq!(parse_range("bytes=-10", 100), RangeAsk::Part(90..100));
        assert_eq!(parse_range("bytes=95-200", 100), RangeAsk::Part(95..100));
        assert_eq!(parse_range("bytes=100-", 100), RangeAsk::Unsatisfiable);
        assert_eq!(parse_range("bytes=0-1,5-6", 100), RangeAsk::Ignore);
        assert_eq!(parse_range("items=0-1", 100), RangeAsk::Ignore);
    }
}
