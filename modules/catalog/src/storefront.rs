//! Bảng dẫn xuất `catalog_storefront_entries` cho đọc công khai: một dòng mỗi (nút bán, sản
//! phẩm), ghi sẵn hiện / ẩn, giá thấp nhất / cao nhất, giá gạch, cột sắp xếp — để danh sách công khai
//! lọc và sắp theo giá có index, không giải giá cho từng sản phẩm lúc đọc (D-222).
//!
//! **Nút bán** (pricing site): gốc của khách, và mọi nút có bảng giá đang bật hay tuỳ biến ẩn /
//! hiện riêng. Website của một nút đọc dòng của nút bán gần nhất trong chuỗi tổ tiên của nó, rồi lọc
//! sản phẩm của chính chuỗi đó và của nhánh bên dưới (người bán trên website của chuỗi).
//!
//! Mọi thay đổi ảnh hưởng (trạng thái, giá, bảng giá, ẩn / hiện) xếp job `catalog.refresh_storefront`
//! trong cùng transaction; job tính lại từ đầu nên chạy lại được; task
//! `catalog.rebuild_storefront` dựng lại cả khách; lịch mỗi phút dựng lại sản phẩm có giá hẹn giờ
//! vừa bắt đầu hay hết hạn.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use autoxide_kernel::money::Currency;
use autoxide_kernel::task::{Task, TaskArgs, TaskArgument, TaskRun};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, TenantId, root_of};
use chrono::{Duration, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter,
    QueryOrder, QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::PriceQuery;
use crate::entity::{price, price_list, product, store_override, storefront_entry, variant};
use crate::errors;
use crate::permissions::{PRODUCTS_READ, PRODUCTS_WRITE};
use crate::pricing::ancestors;
use crate::service::{CatalogService, now, writable};

/// Loại job dựng lại dòng storefront.
pub(crate) const REFRESH: &str = "catalog.refresh_storefront";
/// Lịch mỗi phút: sản phẩm có giá hẹn giờ vừa bắt đầu / hết hạn.
pub(crate) const SCHEDULED_PRICES: &str = "catalog.apply_scheduled_prices";
/// Số sản phẩm mỗi transaction khi dựng lại cả khách.
const BATCH: u64 = 200;

/// Job: dựng lại dòng storefront của `products` và của sản phẩm chứa `variants`; `all`: cả khách.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Refresh {
    #[serde(default)]
    pub all: bool,
    #[serde(default)]
    pub products: Vec<Uuid>,
    #[serde(default)]
    pub variants: Vec<Uuid>,
}

impl Job for Refresh {
    const KIND: &'static str = REFRESH;
}

impl CatalogService {
    /// Xếp job dựng lại storefront trong transaction của thay đổi.
    pub(crate) async fn queue_storefront<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        refresh: Refresh,
    ) -> Result<(), Error> {
        self.jobs.enqueue(connection, tenant, &refresh, EnqueueOptions::default()).await?;
        Ok(())
    }

    /// Xếp job dựng lại storefront của các sản phẩm `products`.
    pub(crate) async fn queue_products<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        products: Vec<Uuid>,
    ) -> Result<(), Error> {
        self.queue_storefront(connection, tenant, Refresh { products, ..Refresh::default() }).await
    }

    /// Xếp job dựng lại storefront của sản phẩm chứa các biến thể `variants` (giá đổi).
    pub(crate) async fn queue_variants<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        variants: Vec<Uuid>,
    ) -> Result<(), Error> {
        self.queue_storefront(connection, tenant, Refresh { variants, ..Refresh::default() }).await
    }

    /// Xếp job dựng lại storefront của cả khách (bảng giá hay nút bán đổi).
    pub(crate) async fn queue_all<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
    ) -> Result<(), Error> {
        self.queue_storefront(connection, tenant, Refresh { all: true, ..Refresh::default() }).await
    }

    /// Sản phẩm chứa các biến thể `variants`.
    async fn products_of(
        &self,
        tenant: &TenantContext,
        variants: Vec<Uuid>,
    ) -> Result<Vec<Uuid>, Error> {
        let mut products = Vec::new();
        for chunk in variants.chunks(crate::api::MAX_IDS) {
            let found: Vec<Uuid> = variant::Entity::find()
                .select_only()
                .column(variant::Column::ProductId)
                .filter(variant::Column::Id.is_in(chunk.iter().copied()))
                .scoped(tenant)
                .distinct()
                .into_tuple()
                .all(tenant.database().read())
                .await?;
            products.extend(found);
        }
        products.sort_unstable();
        products.dedup();
        Ok(products)
    }

    /// Nút bán của khách: gốc, nút có bảng giá đang bật, nút có tuỳ biến ẩn / hiện.
    pub(crate) async fn pricing_sites<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
    ) -> Result<Vec<OrgId>, Error> {
        let mut sites: BTreeSet<Uuid> = BTreeSet::new();
        sites.insert(root_of(tenant.id()).as_uuid());
        let lists: Vec<Uuid> = price_list::Entity::find()
            .select_only()
            .column(price_list::Column::OrgId)
            .filter(price_list::Column::Active.eq(true))
            .scoped(tenant)
            .distinct()
            .into_tuple()
            .all(connection)
            .await?;
        sites.extend(lists);
        let overrides: Vec<Uuid> = store_override::Entity::find()
            .select_only()
            .column(store_override::Column::OrgId)
            .scoped(tenant)
            .distinct()
            .into_tuple()
            .all(connection)
            .await?;
        sites.extend(overrides);
        Ok(sites.into_iter().map(OrgId::from_uuid).collect())
    }

    /// Tiền tệ bán của nút `site`: tiền tệ của bảng giá chung (không lọc nhóm khách, công ty,
    /// kênh, thị trường) đang bật ở nút gần nhất trong chuỗi; không có bảng nào thì `None`.
    async fn site_currency<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        site: OrgId,
    ) -> Result<Option<Currency>, Error> {
        let chain = ancestors(connection, tenant, site).await?;
        let lists = price_list::Entity::find()
            .filter(price_list::Column::Active.eq(true))
            .filter(price_list::Column::OrgId.is_in(chain.iter().map(OrgId::as_uuid)))
            .filter(price_list::Column::CustomerGroup.is_null())
            .filter(price_list::Column::CompanyId.is_null())
            .filter(price_list::Column::Channel.is_null())
            .filter(price_list::Column::Market.is_null())
            .scoped(tenant)
            .order_by_desc(price_list::Column::Priority)
            .order_by_asc(price_list::Column::Id)
            .all(connection)
            .await?;
        let best = lists.iter().min_by_key(|list| {
            chain.iter().position(|org| org.as_uuid() == list.org_id).unwrap_or(usize::MAX)
        });
        Ok(best.and_then(|list| Currency::parse(&list.currency).ok()))
    }

    /// Dựng lại dòng storefront của các sản phẩm `ids` ở mọi nút bán.
    pub(crate) async fn refresh_storefront(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
    ) -> Result<(), Error> {
        if ids.is_empty() {
            return Ok(());
        }
        let read = tenant.database().read();
        let sites = self.pricing_sites(read, tenant).await?;
        let products = product::Entity::find()
            .filter(product::Column::Id.is_in(ids.iter().copied()))
            .scoped(tenant)
            .all(read)
            .await?;
        let variants = variant::Entity::find()
            .filter(variant::Column::ProductId.is_in(ids.iter().copied()))
            .filter(variant::Column::Active.eq(true))
            .scoped(tenant)
            .all(read)
            .await?;
        let hidden: Vec<(Uuid, Uuid)> = store_override::Entity::find()
            .filter(store_override::Column::ProductId.is_in(ids.iter().copied()))
            .filter(store_override::Column::Hidden.eq(true))
            .scoped(tenant)
            .all(read)
            .await?
            .into_iter()
            .map(|row| (row.org_id, row.product_id))
            .collect();
        let at = now();
        let mut entries = Vec::new();
        for site in &sites {
            let Some(currency) = self.site_currency(read, tenant, *site).await? else {
                // Nút chưa có giá chung: không sản phẩm nào bán được ở đó.
                for product in &products {
                    entries.push((empty_entry(tenant, *site, product, "VND"), false));
                }
                continue;
            };
            let chain = ancestors(read, tenant, *site).await?;
            let variant_ids: Vec<Uuid> = variants.iter().map(|row| row.id).collect();
            let mut prices = Vec::new();
            for chunk in variant_ids.chunks(crate::api::MAX_IDS) {
                let query = PriceQuery::new(*site, currency, at, chunk.to_vec());
                prices.extend(self.resolve_prices(tenant, &query, false).await?);
            }
            let by_variant: HashMap<Uuid, &crate::api::PriceSnapshot> =
                prices.iter().map(|price| (price.variant_id, price)).collect();
            let sales = self.sale_prices_at(tenant, *site, at, &prices).await?;
            for product in &products {
                let own: Vec<&variant::Model> =
                    variants.iter().filter(|row| row.product_id == product.id).collect();
                let amounts: Vec<&crate::api::PriceSnapshot> =
                    own.iter().filter_map(|row| by_variant.get(&row.id).copied()).collect();
                let is_hidden = hidden.iter().any(|(org, id)| {
                    *id == product.id && chain.iter().any(|ancestor| ancestor.as_uuid() == *org)
                });
                let visible = product.status == "active" && !amounts.is_empty() && !is_hidden;
                let min = amounts.iter().map(|price| price.amount.minor()).min().unwrap_or(0);
                let max = amounts.iter().map(|price| price.amount.minor()).max().unwrap_or(0);
                let compare_at = amounts
                    .iter()
                    .filter_map(|price| price.compare_at.map(|money| money.minor()))
                    .filter(|compare| *compare > min)
                    .max();
                let mut entry = empty_entry(tenant, *site, product, currency.code());
                entry.visible = Set(visible);
                entry.min_price_minor = Set(min);
                entry.max_price_minor = Set(max);
                entry.compare_at_minor = Set(compare_at);
                entry.variant_count = Set(i32::try_from(own.len()).unwrap_or(i32::MAX));
                // Đơn giá theo đơn vị cơ sở (D-201): của biến thể rẻ nhất có lượng tịnh.
                let unit = own
                    .iter()
                    .filter_map(|row| {
                        let price = by_variant.get(&row.id)?;
                        let quantity = row.net_quantity?;
                        crate::variants::unit_price(
                            price.amount.minor(),
                            quantity,
                            row.net_unit.as_deref()?,
                        )
                        .map(|unit| (price.amount.minor(), unit))
                    })
                    .min_by_key(|(price, _)| *price)
                    .map(|(_, unit)| unit);
                entry.unit_price_minor = Set(unit.map(|(value, _)| value));
                entry.unit_price_unit = Set(unit.map(|(_, base)| base.to_owned()));
                if let Some(sales) = &sales {
                    // Giá giảm thấp nhất trên các biến thể, và lúc nó hết.
                    let best = own
                        .iter()
                        .filter_map(|row| sales.get(&row.id))
                        .min_by_key(|sale| sale.sale_price.minor());
                    entry.sale_price_minor = Set(best.map(|sale| sale.sale_price.minor()));
                    entry.sale_ends_at = Set(best.map(|sale| sale.ends_at));
                }
                entries.push((entry, sales.is_some()));
            }
        }
        let transaction = tenant.database().begin_write().await?;
        let existing = storefront_entry::Entity::find()
            .filter(storefront_entry::Column::ProductId.is_in(ids.iter().copied()))
            .scoped(tenant)
            .all(&transaction)
            .await?;
        // Giữ cột do module khác ghi (bán chạy, đánh giá, giá sau giảm): chỉ ghi đè cột tính ở đây.
        let keep: HashMap<(Uuid, Uuid), &storefront_entry::Model> =
            existing.iter().map(|row| ((row.org_id, row.product_id), row)).collect();
        storefront_entry::Entity::delete_many()
            .filter(storefront_entry::Column::ProductId.is_in(ids.iter().copied()))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        for (mut entry, sale_computed) in entries {
            let key = (*entry.org_id.as_ref(), *entry.product_id.as_ref());
            if let Some(old) = keep.get(&key) {
                entry.best_selling = Set(old.best_selling);
                entry.rating_x100 = Set(old.rating_x100);
                entry.rating_count = Set(old.rating_count);
                if !sale_computed {
                    entry.sale_price_minor = Set(old.sale_price_minor);
                    entry.sale_ends_at = Set(old.sale_ends_at);
                }
            }
            entry.insert(&transaction).await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Giá sau giảm tự động theo biến thể ở nút bán `site`; `None` khi chưa có nguồn nào cắm vào
    /// (giữ giá giảm đã ghi). Gọi ngoài transaction: nguồn có thể đọc database của nó.
    async fn sale_prices_at(
        &self,
        tenant: &TenantContext,
        site: OrgId,
        at: chrono::DateTime<Utc>,
        prices: &[crate::api::PriceSnapshot],
    ) -> Result<Option<HashMap<Uuid, crate::api::SalePrice>>, Error> {
        let Some(source) = self.sale_prices.get() else { return Ok(None) };
        let mut found = HashMap::new();
        for chunk in prices.chunks(crate::api::MAX_IDS) {
            let listed: Vec<(Uuid, autoxide_kernel::money::Money)> =
                chunk.iter().map(|price| (price.variant_id, price.amount)).collect();
            for sale in source.sale_prices(tenant, site, at, &listed).await? {
                // Chỉ nhận giá giảm thật sự thấp hơn giá niêm yết, cùng tiền tệ.
                let listed = listed.iter().find(|(id, _)| *id == sale.variant_id);
                if listed.is_some_and(|(_, price)| {
                    price.currency() == sale.sale_price.currency()
                        && sale.sale_price.minor() < price.minor()
                        && sale.ends_at > at
                }) {
                    found.insert(sale.variant_id, sale);
                }
            }
        }
        Ok(Some(found))
    }

    /// Dựng lại cả khách, theo lô [`BATCH`] sản phẩm.
    pub(crate) async fn rebuild_storefront(&self, tenant: &TenantContext) -> Result<u64, Error> {
        let mut after: Option<Uuid> = None;
        let mut done = 0;
        loop {
            let mut select = product::Entity::find()
                .select_only()
                .column(product::Column::Id)
                .scoped(tenant)
                .order_by_asc(product::Column::Id)
                .limit(BATCH);
            if let Some(after) = after {
                select = select.filter(product::Column::Id.gt(after));
            }
            let ids: Vec<Uuid> = select.into_tuple().all(tenant.database().read()).await?;
            let Some(last) = ids.last() else { break };
            after = Some(*last);
            self.refresh_storefront(tenant, &ids).await?;
            done += ids.len() as u64;
        }
        Ok(done)
    }

    /// Cửa hàng `org` ẩn (hay hiện lại) sản phẩm `product_id` của chuỗi.
    pub(crate) async fn set_store_override(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Uuid,
        product_id: Uuid,
        hidden: bool,
    ) -> Result<store_override::Model, Error> {
        let transaction = tenant.database().begin_write().await?;
        writable(principal, &transaction, &PRODUCTS_READ, &PRODUCTS_WRITE, org, || {
            autoxide_kernel::tenancy::site_not_found(OrgId::from_uuid(org))
        })
        .await?;
        product::Entity::find_by_id(product_id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| errors::product_not_found(product_id))?;
        store_override::Entity::delete_many()
            .filter(store_override::Column::OrgId.eq(org))
            .filter(store_override::Column::ProductId.eq(product_id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let row = store_override::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(org),
            product_id: Set(product_id),
            hidden: Set(hidden),
            updated_at: Set(now()),
        }
        .insert(&transaction)
        .await?;
        // Nút có tuỳ biến thành nút bán: dựng lại cả khách (thêm dòng cho nút mới).
        self.queue_all(&transaction, tenant).await?;
        transaction.commit().await?;
        Ok(row)
    }
}

fn empty_entry(
    tenant: &TenantContext,
    site: OrgId,
    product: &product::Model,
    currency: &str,
) -> storefront_entry::ActiveModel {
    storefront_entry::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant.id().as_uuid()),
        org_id: Set(site.as_uuid()),
        product_id: Set(product.id),
        owner_org_id: Set(product.org_id),
        visible: Set(false),
        currency: Set(currency.to_owned()),
        min_price_minor: Set(0),
        max_price_minor: Set(0),
        compare_at_minor: Set(None),
        sale_price_minor: Set(None),
        sale_ends_at: Set(None),
        best_selling: Set(0),
        rating_x100: Set(0),
        rating_count: Set(0),
        variant_count: Set(0),
        published_at: Set(product.activated_at.unwrap_or(product.created_at)),
        refreshed_at: Set(now()),
        unit_price_minor: Set(None),
        unit_price_unit: Set(None),
    }
}

/// Bộ chạy job [`Refresh`].
pub(crate) struct Refresher(pub(crate) Arc<CatalogService>);

#[async_trait::async_trait]
impl JobHandler for Refresher {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let Some(tenant) = &job.tenant else { return Ok(()) };
        let input: Refresh = serde_json::from_value(job.payload.clone()).unwrap_or_default();
        if input.all {
            self.0.rebuild_storefront(tenant).await?;
            return Ok(());
        }
        let mut products = input.products;
        products.extend(self.0.products_of(tenant, input.variants).await?);
        products.sort_unstable();
        products.dedup();
        for chunk in products.chunks(BATCH as usize) {
            self.0.refresh_storefront(tenant, chunk).await?;
        }
        Ok(())
    }
}

/// Lịch mỗi phút: dựng lại sản phẩm có dòng giá vừa bắt đầu hay vừa hết hạn trong hai phút qua
/// (chồng lên lần chạy trước để không sót; dựng lại là tính lại, chạy hai lần không sao).
pub(crate) struct ScheduledPrices(pub(crate) Arc<CatalogService>);

#[async_trait::async_trait]
impl JobHandler for ScheduledPrices {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let to = Utc::now();
        let from = to - Duration::minutes(2);
        let rows: Vec<(Uuid, Uuid)> = price::Entity::find()
            .select_only()
            .column(price::Column::TenantId)
            .column(price::Column::VariantId)
            .filter(
                Condition::any()
                    .add(price::Column::ValidFrom.between(from, to))
                    .add(price::Column::ValidTo.between(from, to)),
            )
            .distinct()
            .into_tuple()
            .all(job.database.read())
            .await?;
        let mut by_tenant: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        for (tenant, variant) in rows {
            by_tenant.entry(tenant).or_default().push(variant);
        }
        for (tenant, variants) in by_tenant {
            let tenant = self.0.databases.tenant(TenantId::from_uuid(tenant)).await?;
            for chunk in variants.chunks(crate::api::MAX_IDS) {
                let transaction = tenant.database().begin_write().await?;
                let event = crate::events::PriceEffective(crate::events::Change {
                    before: None,
                    after: Some(serde_json::json!({"variant_ids": chunk, "from": from, "to": to})),
                });
                self.0
                    .outbox
                    .record(
                        &transaction,
                        &tenant,
                        autoxide_kernel::event::Actor::system(),
                        None,
                        &event,
                    )
                    .await?;
                transaction.commit().await?;
            }
            let products = self.0.products_of(&tenant, variants).await?;
            for chunk in products.chunks(BATCH as usize) {
                self.0.refresh_storefront(&tenant, chunk).await?;
            }
        }
        Ok(())
    }
}

/// Lệnh `autoxide task catalog.rebuild_storefront [tenant=<slug>]`.
pub(crate) fn task(service: Arc<CatalogService>) -> Task {
    Task::new(
        "rebuild_storefront",
        "Rebuild the public storefront data (prices, visibility) of every product of a tenant. \
         Safe to run again; run it after restoring or moving a tenant.",
        Arc::new(RebuildTask(service)),
    )
    .argument(TaskArgument::optional("tenant", "Slug of the tenant; default: the default tenant."))
}

struct RebuildTask(Arc<CatalogService>);

#[async_trait::async_trait]
impl TaskRun for RebuildTask {
    async fn run(&self, args: &TaskArgs) -> Result<String, Error> {
        let tenant = match args.get("tenant") {
            Some(slug) => self.0.databases.tenant_by_slug(slug).await?,
            None => self.0.databases.default_tenant(),
        };
        let done = self.0.rebuild_storefront(&tenant).await?;
        Ok(format!("Rebuilt the storefront of {done} products of tenant `{}`.", tenant.slug()))
    }
}
