//! Gói luật hàng cấm / có điều kiện (`data/vn/restrictions.json`): nạp quy tắc vào database điều
//! khiển và bộ thuộc tính nhãn hàng hoá mẫu cho một khách (`autoxide task
//! catalog.load_restrictions`), và job hằng ngày tính lại `readiness` của sản phẩm có giấy tờ.
//! Căn cứ pháp lý ghi trong từng quy tắc của gói — cần đối chiếu văn bản gốc (D-186).

use autoxide_kernel::time::Date;
use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::task::{Task, TaskArgs, TaskArgument, TaskRun};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext, TenantId};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QuerySelect, Set,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::entity::{attribute, product_document, restriction_rule};
use crate::service::{CatalogService, now, texts_json};

/// Gói luật Việt Nam phát hành kèm máy chủ.
const BUNDLED_VN: &str = include_str!("../data/vn/restrictions.json");

/// Loại job: tính lại `readiness` của sản phẩm có giấy tờ (hằng ngày).
pub(crate) const REFRESH_DOCUMENTS: &str = "catalog.refresh_document_readiness";

impl CatalogService {
    /// Nạp gói luật (quy tắc hàng cấm / có điều kiện) vào database điều khiển. Nạp lại cùng gói
    /// không đổi gì; quy tắc có cùng (vùng, mã, ngày hiệu lực) được ghi đè.
    pub(crate) async fn load_restrictions(&self, text: &str) -> Result<usize, Error> {
        let pack: Pack = serde_json::from_str(text).map_err(|error| {
            Error::bad_request(
                "catalog.restriction_pack_invalid",
                "The rules pack is not valid: {reason}.",
            )
            .with_param("reason", error.to_string())
        })?;
        let transaction = self.databases.control().begin_write().await?;
        let mut written = 0;
        for rule in &pack.rules {
            let existing = restriction_rule::Entity::find()
                .filter(restriction_rule::Column::Jurisdiction.eq(pack.jurisdiction.as_str()))
                .filter(restriction_rule::Column::Code.eq(rule.code.as_str()))
                .filter(restriction_rule::Column::EffectiveFrom.eq(rule.effective_from))
                .one(&transaction)
                .await?;
            let names = texts_json(&rule.names);
            let documents = serde_json::to_string(&rule.required_documents).unwrap_or_default();
            let model = restriction_rule::ActiveModel {
                id: Set(existing.as_ref().map_or_else(Uuid::now_v7, |row| row.id)),
                jurisdiction: Set(pack.jurisdiction.clone()),
                code: Set(rule.code.clone()),
                level: Set(rule.level.clone()),
                names: Set(names),
                required_documents: Set(documents),
                legal_basis: Set(rule.legal_basis.clone()),
                source_url: Set(rule.source_url.clone()),
                effective_from: Set(rule.effective_from),
                effective_to: Set(rule.effective_to),
                pack_version: Set(pack.pack_version.clone()),
            };
            match existing {
                Some(_) => {
                    model.update(&transaction).await?;
                }
                None => {
                    model.insert(&transaction).await?;
                    written += 1;
                }
            }
        }
        transaction.commit().await?;
        Ok(written)
    }

    /// Tạo bộ thuộc tính nhãn hàng hoá mẫu của gói cho khách `tenant` (bỏ qua mã đã có).
    pub(crate) async fn load_label_attributes(
        &self,
        tenant: &TenantContext,
        text: &str,
    ) -> Result<usize, Error> {
        let pack: Pack = serde_json::from_str(text).map_err(|error| {
            Error::bad_request(
                "catalog.restriction_pack_invalid",
                "The rules pack is not valid: {reason}.",
            )
            .with_param("reason", error.to_string())
        })?;
        let write = tenant.database().write();
        let mut created = 0;
        for label in &pack.label_attributes {
            let exists = attribute::Entity::find()
                .filter(attribute::Column::Code.eq(label.code.as_str()))
                .scoped(tenant)
                .count(write)
                .await?;
            if exists > 0 {
                continue;
            }
            attribute::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                code: Set(label.code.clone()),
                data_type: Set(label.data_type.clone()),
                scale: Set(0),
                unit: Set(None),
                names: Set(texts_json(&label.names)),
                filterable: Set(false),
                legal_basis: Set(Some(label.legal_basis.clone())),
                updated_at: Set(now()),
            }
            .insert(write)
            .await?;
            created += 1;
        }
        Ok(created)
    }
}

#[derive(Debug, Deserialize)]
struct Pack {
    jurisdiction: String,
    pack_version: String,
    rules: Vec<PackRule>,
    #[serde(default)]
    label_attributes: Vec<PackAttribute>,
}

#[derive(Debug, Deserialize)]
struct PackRule {
    code: String,
    level: String,
    names: BTreeMap<String, String>,
    required_documents: Vec<String>,
    legal_basis: String,
    source_url: String,
    effective_from: Date,
    effective_to: Option<Date>,
}

#[derive(Debug, Deserialize)]
struct PackAttribute {
    code: String,
    data_type: String,
    names: BTreeMap<String, String>,
    legal_basis: String,
}

/// Lệnh `autoxide task catalog.load_restrictions [pack=vn] [tenant=<slug>]`.
pub(crate) fn task(service: Arc<CatalogService>) -> Task {
    Task::new(
        "load_restrictions",
        "Load the rules of prohibited and conditional goods (with their legal basis and dates) \
         into the shared rules data, and create the sample product-label attributes for a tenant. \
         Loading the same pack again changes nothing.",
        Arc::new(LoadTask(service)),
    )
    .argument(TaskArgument::optional(
        "pack",
        "Rules pack: `vn` (default, shipped with the server).",
    ))
    .argument(TaskArgument::optional(
        "path",
        "JSON file of a rules pack, instead of a shipped one.",
    ))
    .argument(TaskArgument::optional(
        "tenant",
        "Slug of the tenant that gets the sample label attributes; default: the default tenant.",
    ))
}

struct LoadTask(Arc<CatalogService>);

#[async_trait::async_trait]
impl TaskRun for LoadTask {
    async fn run(&self, args: &TaskArgs) -> Result<String, Error> {
        let text = match (args.get("path"), args.get("pack").unwrap_or("vn")) {
            (Some(path), _) => std::fs::read_to_string(path).map_err(|error| {
                Error::bad_request(
                    "catalog.restriction_pack_unreadable",
                    "Cannot read `{file}`: {reason}.",
                )
                .with_param("file", path)
                .with_param("reason", error.to_string())
            })?,
            (None, "vn") => BUNDLED_VN.to_owned(),
            (None, other) => {
                return Err(Error::bad_request(
                    "catalog.restriction_pack_unknown",
                    "There is no shipped rules pack `{pack}`; the shipped pack is `vn`.",
                )
                .with_param("pack", other));
            }
        };
        let rules = self.0.load_restrictions(&text).await?;
        let tenant = match args.get("tenant") {
            Some(slug) => self.0.databases.tenant_by_slug(slug).await?,
            None => self.0.databases.default_tenant(),
        };
        let attributes = self.0.load_label_attributes(&tenant, &text).await?;
        Ok(format!(
            "Loaded {rules} new restricted-goods rules and {attributes} label attributes for \
             tenant `{}`.",
            tenant.slug()
        ))
    }
}

/// Job hằng ngày: tính lại `readiness` của mọi sản phẩm có giấy tờ trên database của job (giấy
/// tờ hết hạn theo ngày, không có event nào báo).
pub(crate) struct RefreshDocuments(pub(crate) Arc<CatalogService>);

#[async_trait::async_trait]
impl JobHandler for RefreshDocuments {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let rows: Vec<(Uuid, Uuid)> = product_document::Entity::find()
            .select_only()
            .column(product_document::Column::TenantId)
            .column(product_document::Column::ProductId)
            .distinct()
            .into_tuple()
            .all(job.database.read())
            .await?;
        let mut by_tenant: BTreeMap<Uuid, Vec<Uuid>> = BTreeMap::new();
        for (tenant, product) in rows {
            by_tenant.entry(tenant).or_default().push(product);
        }
        for (tenant, products) in by_tenant {
            let tenant = self.0.databases.tenant(TenantId::from_uuid(tenant)).await?;
            let transaction = tenant.database().begin_write().await?;
            self.0.refresh_readiness(&transaction, &tenant, &products).await?;
            transaction.commit().await?;
        }
        Ok(())
    }
}
