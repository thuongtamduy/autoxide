//! Hàng số: file của biến thể và quyền tải theo dòng đơn đã trả tiền.
//!
//! - **File.** Người bán tải file lên qua module `files` (`/api/v1/files`, có giới hạn cỡ / loại, tải
//!   thẳng lên S3 được), rồi gắn `file_id` vào biến thể hàng số. `catalog` chép nội dung vào chỗ của
//!   mình (`catalog/<asset_id>/file` trong nơi lưu của khách): xoá file gốc hay xoá tài khoản người
//!   tải lên không làm người mua mất hàng đã trả tiền.
//! - **Quyền tải.** Đơn được xác nhận với `payment_status = paid` (event `orders.order_confirmed`)
//!   → mỗi (dòng đơn, file) một quyền tải: `remaining` lượt, hết hạn sau `grant_days`. Khoá chống
//!   trùng là unique (`tenant_id`, `order_line_id`, `asset_id`): nhận lại event không phát thêm.
//!   Token ngẫu nhiên chỉ lưu bản băm SHA-256.
//! - **Tải.** `POST /public/downloads/{token}` trừ một lượt bằng một câu `UPDATE` có điều kiện
//!   (`rows::adjust`, không đọc trước — D-181): 20 request cùng lúc với 5 lượt thì đúng 5 thành
//!   công. Trả địa chỉ file ký HMAC bằng khoá riêng của khách (`SecretKey::derive_for`), sống
//!   `AUTOXIDE_CATALOG_DOWNLOAD_LINK_SECONDS`; địa chỉ đó đọc file (nhận `Range`) mà không trừ lượt.
//!   Địa chỉ ký của khách A mang sang khách B thì sai chữ ký.
//! - **Thu hồi.** Đơn huỷ, hoàn tiền không gắn trả hàng, hay người quản lý thu hồi → `revoked_at`;
//!   token và địa chỉ ký đã phát hết dùng ngay.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::storage::{Storage as _, StoredFile, TenantStorage};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use chrono::{DateTime, Duration, Utc};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, Set,
};
use serde_json::json;
use uuid::Uuid;

use crate::api::ProductKind;
use crate::entity::{digital_asset, download_grant, product, variant};
use crate::events::{Change, DownloadsIssued, DownloadsRevoked};
use crate::permissions::{PRODUCTS_READ, PRODUCTS_WRITE};
use crate::service::{CatalogService, DIGITAL_FEATURE, fail_if_any, now, readable};
use crate::{errors, products::parse_kind};

/// Gắn một file vào biến thể hàng số.
#[derive(Debug, Clone)]
pub(crate) struct NewAsset {
    /// File đã tải lên qua module `files`.
    pub file_id: Uuid,
    pub download_limit: Option<i64>,
    pub grant_days: Option<i64>,
}

/// Một dòng của đơn đã trả tiền, đọc từ event của `orders`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PaidLine {
    pub line_id: Uuid,
    pub variant_id: Uuid,
}

/// Quyền tải kèm tên file; `token` chỉ có ở lần phát (hay cấp lại) — sau đó không đọc lại được.
#[derive(Debug, Clone)]
pub(crate) struct GrantView {
    pub grant: download_grant::Model,
    pub file_name: String,
    pub token: Option<String>,
}

/// Địa chỉ file đã ký, sau khi trừ một lượt.
#[derive(Debug, Clone)]
pub(crate) struct FileLink {
    pub url: String,
    pub expires_at: DateTime<Utc>,
    pub remaining: i64,
    pub file_name: String,
}

/// File phục vụ cho một địa chỉ ký: thông tin file, đoạn đã đọc (nếu xin `Range`), nội dung.
pub(crate) struct ServedFile {
    pub asset: digital_asset::Model,
    pub range: Option<Range<u64>>,
    pub file: StoredFile,
}

/// Đường dẫn công khai (dưới API) để người mua dùng token.
pub(crate) fn token_path(token: &str) -> String {
    format!("{}/catalog/public/downloads/{token}", autoxide_kernel::http::API_PREFIX)
}

/// Hàng số phải có ít nhất một file trước khi mở bán (`catalog.digital_asset_missing`).
pub(crate) async fn require_assets<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    product_id: Uuid,
) -> Result<(), Error> {
    let count = digital_asset::Entity::find()
        .filter(digital_asset::Column::ProductId.eq(product_id))
        .scoped(tenant)
        .count(connection)
        .await?;
    if count == 0 { Err(asset_missing()) } else { Ok(()) }
}

impl CatalogService {
    pub(crate) fn digital_store(&self, tenant: &TenantContext) -> Result<TenantStorage, Error> {
        let storage = self.storage.clone().ok_or_else(crate::transfers::storage_unavailable)?;
        Ok(TenantStorage::new(storage, tenant))
    }

    /// Gắn file `input.file_id` (của `files`) vào biến thể `variant_id` của một sản phẩm hàng số.
    pub(crate) async fn attach_asset(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        variant_id: Uuid,
        input: NewAsset,
    ) -> Result<digital_asset::Model, Error> {
        self.require_feature(tenant, &DIGITAL_FEATURE).await?;
        let mut problems = Vec::new();
        let limit = input.download_limit.unwrap_or(self.config.default_download_limit);
        if !(1..=1_000).contains(&limit) {
            problems.push(FieldError::new("download_limit", "range", "must be from 1 to 1000"));
        }
        let days = input.grant_days.unwrap_or(self.config.default_grant_days);
        if !(1..=3_650).contains(&days) {
            problems.push(FieldError::new("grant_days", "range", "must be from 1 to 3650"));
        }
        fail_if_any(problems)?;
        // Kiểm biến thể trước khi chép file: sai thì báo ngay, khỏi chép rồi xoá.
        let read = tenant.database().read();
        let (_, product) = self.variant_and_product(read, tenant, variant_id).await?;
        readable(principal, read, &PRODUCTS_WRITE, product.org_id, || {
            errors::variant_not_found(variant_id)
        })
        .await?;
        if parse_kind(&product.kind) != ProductKind::Digital {
            return Err(not_digital());
        }
        let files = self.files.clone().ok_or_else(files_unavailable)?;
        let store = self.digital_store(tenant)?;
        let opened = files.open(tenant, principal, input.file_id).await?;
        let id = Uuid::now_v7();
        let key = format!("catalog/{id}/file");
        // Chép NGOÀI transaction ghi (luật dữ liệu: transaction ngắn); ghi dòng hỏng thì xoá bản chép.
        let size = store.put(&key, opened.content).await?;
        let saved = async {
            let transaction = tenant.database().begin_write().await?;
            let (variant, product) =
                self.variant_for_write(&transaction, tenant, principal, variant_id).await?;
            if parse_kind(&product.kind) != ProductKind::Digital {
                return Err(not_digital());
            }
            let row = digital_asset::ActiveModel {
                id: Set(id),
                tenant_id: Set(tenant.id().as_uuid()),
                product_id: Set(product.id),
                variant_id: Set(variant.id),
                file_name: Set(opened.info.name.clone()),
                content_type: Set(opened.info.content_type.clone()),
                size: Set(i64::try_from(size).map_err(Error::internal)?),
                sha256: Set(opened.info.sha256.clone()),
                storage_key: Set(key.clone()),
                download_limit: Set(limit),
                grant_days: Set(days),
                created_by: Set(Some(principal.user_id())),
                created_at: Set(now()),
            }
            .insert(&transaction)
            .await?;
            self.touch(&transaction, tenant, Actor::from(principal), &product, None).await?;
            transaction.commit().await?;
            Ok(row)
        }
        .await;
        if saved.is_err()
            && let Err(error) = store.delete(&key).await
        {
            tracing::warn!(asset_id = %id, error = %error, "copy of a digital file left behind");
        }
        let row = saved?;
        tracing::info!(asset_id = %row.id, variant_id = %variant_id, "digital file attached");
        Ok(row)
    }

    /// File của biến thể `variant_id`.
    pub(crate) async fn list_assets(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        variant_id: Uuid,
    ) -> Result<Vec<digital_asset::Model>, Error> {
        let read = tenant.database().read();
        let (_, product) = self.variant_and_product(read, tenant, variant_id).await?;
        readable(principal, read, &PRODUCTS_READ, product.org_id, || {
            errors::variant_not_found(variant_id)
        })
        .await?;
        Ok(digital_asset::Entity::find()
            .filter(digital_asset::Column::VariantId.eq(variant_id))
            .scoped(tenant)
            .order_by_asc(digital_asset::Column::Id)
            .all(read)
            .await?)
    }

    /// Gỡ file `id`. Không gỡ được khi còn quyền tải dùng được (`catalog.digital_asset_in_use`),
    /// hay khi đó là file cuối của sản phẩm đang bán (`catalog.digital_asset_missing`).
    pub(crate) async fn remove_asset(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        let asset = digital_asset::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| asset_not_found(id))?;
        // Khoá sản phẩm (D-181): không chen giữa lúc mở bán kiểm "còn file".
        let (_, product) = self
            .variant_for_write(&transaction, tenant, principal, asset.variant_id)
            .await
            .map_err(|error| {
                if error.code() == "catalog.variant_not_found" {
                    asset_not_found(id)
                } else {
                    error
                }
            })?;
        let in_use = download_grant::Entity::find()
            .filter(download_grant::Column::AssetId.eq(id))
            .filter(download_grant::Column::RevokedAt.is_null())
            .filter(download_grant::Column::ExpiresAt.gt(now()))
            .filter(download_grant::Column::Remaining.gt(0))
            .scoped(tenant)
            .count(&transaction)
            .await?;
        if in_use > 0 {
            return Err(asset_in_use());
        }
        let others = digital_asset::Entity::find()
            .filter(digital_asset::Column::ProductId.eq(product.id))
            .filter(digital_asset::Column::Id.ne(id))
            .scoped(tenant)
            .count(&transaction)
            .await?;
        if others == 0 && product.status == "active" {
            return Err(asset_missing());
        }
        digital_asset::Entity::delete_many()
            .filter(digital_asset::Column::Id.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        self.touch(&transaction, tenant, Actor::from(principal), &product, None).await?;
        transaction.commit().await?;
        if let Err(error) = self.digital_store(tenant)?.delete(&asset.storage_key).await {
            tracing::warn!(asset_id = %id, error = %error, "digital file removed but its copy remains");
        }
        Ok(())
    }

    async fn variant_and_product<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        variant_id: Uuid,
    ) -> Result<(variant::Model, product::Model), Error> {
        let found = variant::Entity::find_by_id(variant_id)
            .scoped(tenant)
            .one(connection)
            .await?
            .ok_or_else(|| errors::variant_not_found(variant_id))?;
        let product = product::Entity::find_by_id(found.product_id)
            .scoped(tenant)
            .one(connection)
            .await?
            .ok_or_else(|| errors::variant_not_found(variant_id))?;
        Ok((found, product))
    }

    /// Phát quyền tải cho các dòng đơn đã trả tiền `lines` (dòng không phải hàng số bị bỏ qua).
    /// Gọi lại cùng dòng: trả quyền đã phát, không kèm token.
    pub(crate) async fn issue_downloads(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
        buyer_id: Option<Uuid>,
        lines: &[PaidLine],
    ) -> Result<Vec<GrantView>, Error> {
        let variant_ids: HashSet<Uuid> = lines.iter().map(|line| line.variant_id).collect();
        if variant_ids.is_empty() {
            return Ok(Vec::new());
        }
        let write = tenant.database().write();
        let assets = digital_asset::Entity::find()
            .filter(digital_asset::Column::VariantId.is_in(variant_ids))
            .scoped(tenant)
            .order_by_asc(digital_asset::Column::Id)
            .all(write)
            .await?;
        if assets.is_empty() {
            return Ok(Vec::new());
        }
        let product_ids: HashSet<Uuid> = assets.iter().map(|asset| asset.product_id).collect();
        let orgs: HashMap<Uuid, Uuid> = product::Entity::find()
            .filter(product::Column::Id.is_in(product_ids))
            .scoped(tenant)
            .all(write)
            .await?
            .into_iter()
            .map(|row| (row.id, row.org_id))
            .collect();
        let now = now();
        let transaction = tenant.database().begin_write().await?;
        let mut issued = Vec::new();
        let mut fresh = Vec::new();
        for line in lines {
            for asset in assets.iter().filter(|asset| asset.variant_id == line.variant_id) {
                let Some(org) = orgs.get(&asset.product_id) else { continue };
                let token = autoxide_kernel::secret::random_token(32)?;
                let row = download_grant::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    order_id: Set(order_id),
                    order_line_id: Set(line.line_id),
                    buyer_id: Set(buyer_id),
                    org_id: Set(*org),
                    asset_id: Set(asset.id),
                    variant_id: Set(asset.variant_id),
                    token_hash: Set(autoxide_kernel::secret::sha256(&token)),
                    remaining: Set(asset.download_limit),
                    expires_at: Set(now + Duration::days(asset.grant_days)),
                    revoked_at: Set(None),
                    created_at: Set(now),
                };
                // Khoá chống trùng là unique (khách, dòng đơn, file): trùng thì không ghi gì
                // (cả hai database), rồi đọc quyền đã có.
                let inserted = download_grant::Entity::insert(row)
                    .on_conflict(
                        OnConflict::columns([
                            download_grant::Column::TenantId,
                            download_grant::Column::OrderLineId,
                            download_grant::Column::AssetId,
                        ])
                        .do_nothing()
                        .to_owned(),
                    )
                    .exec_without_returning(&transaction)
                    .await?;
                let grant = download_grant::Entity::find()
                    .filter(download_grant::Column::OrderLineId.eq(line.line_id))
                    .filter(download_grant::Column::AssetId.eq(asset.id))
                    .scoped(tenant)
                    .one(&transaction)
                    .await?
                    .ok_or_else(|| Error::internal("download grant missing after insert"))?;
                let token = (inserted > 0).then_some(token);
                if token.is_some() {
                    fresh.push(json!({
                        "grant_id": grant.id, "asset_id": grant.asset_id,
                        "variant_id": grant.variant_id, "order_line_id": grant.order_line_id,
                        "remaining": grant.remaining, "expires_at": grant.expires_at,
                    }));
                }
                issued.push(GrantView { grant, file_name: asset.file_name.clone(), token });
            }
        }
        if !fresh.is_empty() {
            let after = json!({"order_id": order_id, "buyer_id": buyer_id, "grants": fresh});
            let event = DownloadsIssued(Change { before: None, after: Some(after) });
            let subject = Some(format!("orders.order:{order_id}"));
            self.outbox.record(&transaction, tenant, Actor::system(), subject, &event).await?;
        }
        transaction.commit().await?;
        if !fresh.is_empty() {
            tracing::info!(order_id = %order_id, count = fresh.len(), "download rights issued");
        }
        Ok(issued)
    }

    /// Thu hồi các quyền tải `ids` (hay mọi quyền của đơn khi `ids` là `None`). Trả số quyền vừa
    /// thu hồi; gọi lại trả 0.
    pub(crate) async fn revoke_downloads(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        order_id: Uuid,
        ids: Option<Vec<Uuid>>,
        reason: &str,
    ) -> Result<u64, Error> {
        let transaction = tenant.database().begin_write().await?;
        let mut update = download_grant::Entity::update_many()
            .col_expr(download_grant::Column::RevokedAt, Expr::value(Some(now())))
            .filter(download_grant::Column::OrderId.eq(order_id))
            .filter(download_grant::Column::RevokedAt.is_null());
        if let Some(ids) = ids {
            update = update.filter(download_grant::Column::Id.is_in(ids));
        }
        let revoked = update.scoped(tenant).exec(&transaction).await?.rows_affected;
        if revoked > 0 {
            let after = json!({"order_id": order_id, "revoked": revoked, "reason": reason});
            let event = DownloadsRevoked(Change { before: None, after: Some(after) });
            let subject = Some(format!("orders.order:{order_id}"));
            self.outbox.record(&transaction, tenant, actor, subject, &event).await?;
        }
        transaction.commit().await?;
        if revoked > 0 {
            tracing::info!(order_id = %order_id, revoked, reason, "download rights revoked");
        }
        Ok(revoked)
    }
}

pub(crate) fn download_not_found() -> Error {
    Error::not_found(
        "catalog.download_not_found",
        "This download link is no longer valid: it expired, was revoked or is wrong. Ask the shop \
         for a new link.",
    )
}

pub(crate) fn limit_reached() -> Error {
    Error::conflict(
        "catalog.download_limit_reached",
        "Every download of this link has been used. Ask the shop for more downloads.",
    )
}

pub(crate) fn asset_missing() -> Error {
    Error::conflict(
        "catalog.digital_asset_missing",
        "A digital product needs at least one file before it goes on sale: attach one with \
         `POST /api/v1/catalog/variants/{id}/digital-assets`.",
    )
}

pub(crate) fn asset_not_found(id: Uuid) -> Error {
    Error::not_found("catalog.digital_asset_not_found", "There is no digital file `{id}`.")
        .with_param("id", id)
}

pub(crate) fn asset_in_use() -> Error {
    Error::conflict(
        "catalog.digital_asset_in_use",
        "Buyers can still download this file. Revoke their download rights or wait until they \
         expire before removing it.",
    )
}

pub(crate) fn not_digital() -> Error {
    Error::new(
        ErrorKind::Validation,
        "catalog.not_digital",
        "Files can only be attached to variants of a product of kind `digital`.",
    )
}

pub(crate) fn files_unavailable() -> Error {
    Error::unavailable(
        "catalog.files_unavailable",
        "Digital products need module `files` to receive the uploaded file.",
    )
}
