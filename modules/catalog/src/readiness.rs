//! `readiness` của sản phẩm (D-223…D-227): cột dẫn xuất cho biết sản phẩm đã mở bán được chưa.
//!
//! - `restricted`: một danh mục của sản phẩm (hay tổ tiên) gắn quy tắc hàng **cấm** đang hiệu lực;
//! - `missing_attributes`: thiếu giá trị của thuộc tính bắt buộc đang hiệu lực hôm nay (của danh
//!   mục sản phẩm, hay của tổ tiên khi `applies_to_descendants`);
//! - `missing_documents`: hàng **có điều kiện** chưa đủ giấy tờ còn hạn;
//! - `ready`: còn lại.
//!
//! Tính lại khi sản phẩm, danh mục của nó, thuộc tính bắt buộc của danh mục, quy tắc hàng có điều
//! kiện hay giấy tờ đổi, và bằng job hằng ngày cho giấy tờ hết hạn. Mở bán tính lại tại chỗ để báo
//! đúng thứ còn thiếu.

use autoxide_kernel::time::Date;
use std::collections::BTreeSet;

use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::entity::{
    attribute, category_attribute, category_path, category_restriction, product, product_attribute,
    product_category, product_document, restriction_rule,
};

pub(crate) const READY: &str = "ready";
pub(crate) const MISSING_ATTRIBUTES: &str = "missing_attributes";
pub(crate) const MISSING_DOCUMENTS: &str = "missing_documents";
pub(crate) const RESTRICTED: &str = "restricted";

/// Mọi giá trị của `readiness`, cho bộ lọc và số liệu.
pub(crate) const ALL: [&str; 4] = [READY, MISSING_ATTRIBUTES, MISSING_DOCUMENTS, RESTRICTED];

/// Kết quả tính: giá trị cột và chi tiết để báo lỗi khi mở bán.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Readiness {
    /// Mã quy tắc hàng cấm khớp.
    pub prohibited: Vec<String>,
    /// Mã thuộc tính bắt buộc còn thiếu.
    pub missing_attributes: Vec<String>,
    /// Loại giấy tờ còn thiếu hay hết hạn.
    pub missing_documents: Vec<String>,
}

impl Readiness {
    pub(crate) fn value(&self) -> &'static str {
        if !self.prohibited.is_empty() {
            RESTRICTED
        } else if !self.missing_attributes.is_empty() {
            MISSING_ATTRIBUTES
        } else if !self.missing_documents.is_empty() {
            MISSING_DOCUMENTS
        } else {
            READY
        }
    }

    /// Lỗi khi mở bán sản phẩm chưa sẵn sàng; `Ok` khi `ready`.
    pub(crate) fn require(&self) -> Result<(), Error> {
        if !self.prohibited.is_empty() {
            return Err(Error::conflict(
                "catalog.prohibited_goods",
                "The product belongs to a category of prohibited goods ({rule}) and cannot be sold.",
            )
            .with_param("rule", self.prohibited.join(", ")));
        }
        if !self.missing_attributes.is_empty() {
            return Err(Error::new(
                ErrorKind::Validation,
                "catalog.required_attributes_missing",
                "Required attributes of the product's categories are missing: {attributes}.",
            )
            .with_param("attributes", self.missing_attributes.join(", ")));
        }
        if !self.missing_documents.is_empty() {
            return Err(Error::conflict(
                "catalog.documents_required",
                "This restricted product needs valid documents: {documents}.",
            )
            .with_param("documents", self.missing_documents.join(", ")));
        }
        Ok(())
    }
}

/// Tính `readiness` của sản phẩm `product` vào ngày `today`.
pub(crate) async fn compute<C: ConnectionTrait>(
    connection: &C,
    control: &impl ConnectionTrait,
    tenant: &TenantContext,
    product: &product::Model,
    today: Date,
) -> Result<Readiness, Error> {
    let own: Vec<Uuid> = product_category::Entity::find()
        .filter(product_category::Column::ProductId.eq(product.id))
        .scoped(tenant)
        .all(connection)
        .await?
        .into_iter()
        .map(|link| link.category_id)
        .collect();
    let paths = category_path::Entity::find()
        .filter(category_path::Column::DescendantId.is_in(own.clone()))
        .scoped(tenant)
        .all(connection)
        .await?;
    // Danh mục của sản phẩm và mọi tổ tiên; `depth` 0 là chính danh mục của sản phẩm.
    let all: BTreeSet<Uuid> = paths.iter().map(|path| path.ancestor_id).collect();
    let mut out = Readiness::default();

    // Thuộc tính bắt buộc.
    let rules = category_attribute::Entity::find()
        .filter(category_attribute::Column::CategoryId.is_in(all.iter().copied()))
        .filter(category_attribute::Column::Required.eq(true))
        .scoped(tenant)
        .all(connection)
        .await?;
    let required: BTreeSet<Uuid> = rules
        .iter()
        .filter(|rule| {
            rule.effective_from <= today && rule.effective_to.is_none_or(|to| to > today)
        })
        .filter(|rule| {
            paths.iter().any(|path| {
                path.ancestor_id == rule.category_id
                    && (path.depth == 0 || rule.applies_to_descendants)
            })
        })
        .map(|rule| rule.attribute_id)
        .collect();
    if !required.is_empty() {
        let present: BTreeSet<Uuid> = product_attribute::Entity::find()
            .filter(product_attribute::Column::ProductId.eq(product.id))
            .scoped(tenant)
            .all(connection)
            .await?
            .into_iter()
            .map(|value| value.attribute_id)
            .collect();
        let missing: Vec<Uuid> = required.difference(&present).copied().collect();
        if !missing.is_empty() {
            out.missing_attributes = attribute::Entity::find()
                .filter(attribute::Column::Id.is_in(missing))
                .scoped(tenant)
                .all(connection)
                .await?
                .into_iter()
                .map(|row| row.code)
                .collect();
            out.missing_attributes.sort();
        }
    }

    // Hàng cấm / có điều kiện (quy tắc là dữ liệu dùng chung ở database điều khiển).
    let codes: Vec<String> = category_restriction::Entity::find()
        .filter(category_restriction::Column::CategoryId.is_in(all.iter().copied()))
        .scoped(tenant)
        .all(connection)
        .await?
        .into_iter()
        .map(|link| link.rule_code)
        .collect();
    if !codes.is_empty() {
        let rules = restriction_rule::Entity::find()
            .filter(restriction_rule::Column::Code.is_in(codes))
            .filter(restriction_rule::Column::EffectiveFrom.lte(today))
            .all(control)
            .await?
            .into_iter()
            .filter(|rule| rule.effective_to.is_none_or(|to| to > today));
        let mut needed = BTreeSet::new();
        for rule in rules {
            if rule.level == "prohibited" {
                out.prohibited.push(rule.code.clone());
            } else {
                let documents: Vec<String> =
                    serde_json::from_str(&rule.required_documents).unwrap_or_default();
                needed.extend(documents);
            }
        }
        out.prohibited.sort();
        out.prohibited.dedup();
        if !needed.is_empty() {
            let valid: BTreeSet<String> = product_document::Entity::find()
                .filter(product_document::Column::ProductId.eq(product.id))
                .scoped(tenant)
                .all(connection)
                .await?
                .into_iter()
                .filter(|document| document.expires_on.is_none_or(|day| day >= today))
                .map(|document| document.document_type)
                .collect();
            out.missing_documents = needed.difference(&valid).cloned().collect();
        }
    }
    Ok(out)
}
