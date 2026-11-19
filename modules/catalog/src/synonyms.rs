//! Từ đồng nghĩa theo khách cho tìm sản phẩm (D-201): "áo phông" tìm ra "áo thun", "tee". Lưu theo
//! nhóm; mỗi từ trong nhóm tìm ra mọi từ khác của nhóm. Từ lưu dạng đã bỏ dấu, chữ thường
//! (`search::fold`) vì tìm so trên `search_key` cùng dạng.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::search;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ActiveModelTrait, EntityTrait, QueryOrder, Set};
use uuid::Uuid;

use crate::entity::synonym;
use crate::permissions::CATEGORIES_MANAGE;
use crate::service::{CatalogService, now};

/// Số nhóm tối đa của một khách.
pub(crate) const MAX_GROUPS: usize = 500;
/// Số từ tối đa của một nhóm.
pub(crate) const MAX_TERMS: usize = 20;
/// Độ dài tối đa của một từ.
const TERM_MAX: usize = 60;
/// Số cách nói tối đa của một câu tìm (mỗi cách một điều kiện `LIKE`).
const MAX_VARIANTS: usize = 10;

impl CatalogService {
    /// Mọi nhóm từ đồng nghĩa của khách, theo thứ tự đã lưu.
    pub(crate) async fn synonym_groups(
        &self,
        tenant: &TenantContext,
    ) -> Result<Vec<Vec<String>>, Error> {
        let rows = synonym::Entity::find()
            .scoped(tenant)
            .order_by_asc(synonym::Column::Position)
            .all(tenant.database().read())
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| serde_json::from_str(&row.terms).unwrap_or_default())
            .collect())
    }

    /// Thay cả tập nhóm từ đồng nghĩa. Từ được chuẩn hoá (bỏ dấu, chữ thường, bỏ trùng); nhóm còn
    /// dưới hai từ thì lỗi. Cần `catalog.categories.manage`.
    pub(crate) async fn put_synonym_groups(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        groups: Vec<Vec<String>>,
    ) -> Result<Vec<Vec<String>>, Error> {
        principal.require(&CATEGORIES_MANAGE)?;
        let mut problems = Vec::new();
        if groups.len() > MAX_GROUPS {
            problems.push(
                FieldError::new("groups", "count", "at most {max} groups")
                    .with_param("max", MAX_GROUPS),
            );
        }
        let mut clean: Vec<Vec<String>> = Vec::new();
        for (index, group) in groups.iter().enumerate() {
            let field = format!("groups[{index}]");
            let mut terms: Vec<String> = Vec::new();
            for term in group {
                let folded = search::fold(term).split_whitespace().collect::<Vec<_>>().join(" ");
                if folded.is_empty() || folded.chars().count() > TERM_MAX {
                    problems.push(
                        FieldError::new(
                            field.clone(),
                            "length",
                            "every term must be 1 to {max} characters",
                        )
                        .with_param("max", TERM_MAX),
                    );
                    continue;
                }
                if !terms.contains(&folded) {
                    terms.push(folded);
                }
            }
            if terms.len() < 2 || terms.len() > MAX_TERMS {
                problems.push(
                    FieldError::new(field, "count", "a group needs 2 to {max} different terms")
                        .with_param("max", MAX_TERMS),
                );
                continue;
            }
            clean.push(terms);
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let transaction = tenant.database().begin_write().await?;
        synonym::Entity::delete_many().scoped(tenant).exec(&transaction).await?;
        for (position, terms) in clean.iter().enumerate() {
            synonym::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                position: Set(i32::try_from(position).unwrap_or(i32::MAX)),
                terms: Set(serde_json::to_string(terms).map_err(Error::internal)?),
                updated_at: Set(now()),
            }
            .insert(&transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(clean)
    }

    /// Câu tìm `q` và các cách nói khác: mỗi từ của một nhóm có trong câu được thay bằng từng từ
    /// khác của nhóm. Tối đa [`MAX_VARIANTS`] cách, câu gốc luôn đứng đầu.
    pub(crate) async fn expand_query(
        &self,
        tenant: &TenantContext,
        q: &str,
    ) -> Result<Vec<String>, Error> {
        let groups = self.synonym_groups(tenant).await?;
        Ok(expand(q, &groups))
    }
}

/// Phần thuần của [`CatalogService::expand_query`] (test không cần database).
pub(crate) fn expand(q: &str, groups: &[Vec<String>]) -> Vec<String> {
    let folded = search::fold(q).split_whitespace().collect::<Vec<_>>().join(" ");
    let mut variants = vec![q.trim().to_owned()];
    // Khớp theo từ: bọc khoảng trắng để "ao" không khớp trong "bao".
    let padded = format!(" {folded} ");
    for group in groups {
        for term in group {
            let needle = format!(" {term} ");
            if !padded.contains(&needle) {
                continue;
            }
            for other in group.iter().filter(|other| *other != term) {
                let replaced = padded.replacen(&needle, &format!(" {other} "), 1);
                let replaced = replaced.trim().to_owned();
                if !variants.contains(&replaced) && variants.len() < MAX_VARIANTS {
                    variants.push(replaced);
                }
            }
        }
    }
    variants
}

#[cfg(test)]
mod unit {
    use super::expand;

    #[test]
    fn mo_rong_theo_tu_dong_nghia() {
        let groups = vec![vec!["ao thun".to_owned(), "ao phong".to_owned(), "tee".to_owned()]];
        assert_eq!(
            expand("Áo phông trắng", &groups),
            vec!["Áo phông trắng".to_owned(), "ao thun trang".to_owned(), "tee trang".to_owned(),]
        );
        // Khớp theo từ, không theo một phần của từ.
        assert_eq!(expand("teen", &groups), vec!["teen".to_owned()]);
        assert_eq!(
            expand("bao", &[vec!["ao".to_owned(), "shirt".to_owned()]]),
            vec!["bao".to_owned()]
        );
    }
}
