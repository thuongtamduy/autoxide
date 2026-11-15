//! Kiểm schema theo khách (bước 33, D-253) trên một database đã migrate:
//!
//! - bảng cấp khách hàng (mọi bảng trừ bảng mặt điều khiển module khai qua
//!   `Module::control_plane_tables` và bảng lịch sử `<m>_migrations`) có `tenant_id` NOT NULL,
//!   không `DEFAULT`;
//! - mọi khoá unique (kể cả khoá chính) bắt đầu bằng `tenant_id`, trừ khoá chỉ gồm cột UUID (ID
//!   sinh trong ứng dụng, duy nhất toàn cục — vd khoá chính `id`);
//! - mọi index thường bắt đầu bằng `tenant_id`, trừ index quét cả database có tên `…_sweep_idx`.
//!
//! Trong lúc chuyển module (phần B của bước 33), bảng chưa chuyển nằm trong [`PENDING_TENANCY`];
//! mỗi commit chuyển module xoá bảng của nó khỏi danh sách, commit cuối xoá cả danh sách.

use crate::schema::{SchemaSnapshot, TableSnapshot};

/// Bảng cấp khách hàng chưa có `tenant_id` (chưa chuyển). Bảng đã có `tenant_id` mà còn trong danh
/// sách cũng là vi phạm — để danh sách chỉ ngắn đi.
pub const PENDING_TENANCY: &[&str] = &[
    // content
    "content_authors",
    "content_embedding_state",
    "content_embeddings",
    "content_entries",
    "content_entry_authors",
    "content_entry_terms",
    "content_menu_items",
    "content_menus",
    "content_published",
    "content_redirects",
    "content_revisions",
    "content_schedules",
    "content_term_translations",
    "content_terms",
    "content_translation_requests",
    "content_tree_locks",
    // forms
    // media
    // menus
    // newsletter
];

/// Các vi phạm (đã sắp xếp); rỗng nghĩa là hợp lệ. `control_plane`: bảng mặt điều khiển đã khai;
/// `pending`: bảng chưa chuyển (thường là [`PENDING_TENANCY`]).
pub fn schema_tenancy_violations(
    snapshot: &SchemaSnapshot,
    control_plane: &[&str],
    pending: &[&str],
) -> Vec<String> {
    let mut violations = Vec::new();
    for table in &snapshot.tables {
        let name = table.name.as_str();
        if name.ends_with("_migrations") || control_plane.contains(&name) {
            continue;
        }
        let tenant = table.columns.iter().find(|column| column.name == "tenant_id");
        if pending.contains(&name) {
            if tenant.is_some() {
                violations.push(format!(
                    "table `{name}` has `tenant_id`: remove it from `PENDING_TENANCY`"
                ));
            }
            continue;
        }
        match tenant {
            None => {
                // Thiếu cột thì báo một lần; khoá và index của bảng xét sau khi nâng.
                violations.push(format!(
                    "table `{name}` has no `tenant_id`: add one (`tenancy::migration::adopt`), or \
                     declare it in `Module::control_plane_tables` if it is shared by the whole \
                     platform"
                ));
                continue;
            }
            Some(column) if column.nullable || column.default.is_some() => {
                violations.push(format!(
                    "`{name}.tenant_id` must be NOT NULL without a DEFAULT (a forgotten tenant \
                     must fail, not land in the default tenant)"
                ))
            }
            Some(_) => {}
        }
        violations.extend(index_violations(table));
    }
    violations.sort();
    violations
}

fn index_violations(table: &TableSnapshot) -> Vec<String> {
    let uuid = |column: &str| {
        table.columns.iter().any(|known| {
            known.name == column && matches!(known.kind.as_str(), "uuid" | "uuid_text")
        })
    };
    let mut violations = Vec::new();
    for index in &table.indexes {
        let scoped = index.columns.first().is_some_and(|column| column == "tenant_id");
        if scoped {
            continue;
        }
        if index.unique {
            if !index.columns.iter().all(|column| uuid(column)) {
                violations.push(format!(
                    "unique index `{}` of `{}` ({}) must start with `tenant_id`",
                    index.name,
                    table.name,
                    index.columns.join(", ")
                ));
            }
        } else if !index.name.ends_with("_sweep_idx") {
            violations.push(format!(
                "index `{}` of `{}` ({}) must start with `tenant_id`, or be named `…_sweep_idx` \
                 when it serves a sweep of the whole database",
                index.name,
                table.name,
                index.columns.join(", ")
            ));
        }
    }
    violations
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{ColumnSnapshot, IndexSnapshot};

    fn column(name: &str, kind: &str, nullable: bool, default: Option<&str>) -> ColumnSnapshot {
        ColumnSnapshot {
            name: name.to_owned(),
            kind: kind.to_owned(),
            nullable,
            default: default.map(str::to_owned),
        }
    }

    fn index(name: &str, columns: &[&str], unique: bool) -> IndexSnapshot {
        IndexSnapshot {
            name: name.to_owned(),
            columns: columns.iter().map(|column| (*column).to_owned()).collect(),
            unique,
        }
    }

    fn table(
        name: &str,
        columns: Vec<ColumnSnapshot>,
        indexes: Vec<IndexSnapshot>,
    ) -> TableSnapshot {
        TableSnapshot { name: name.to_owned(), columns, indexes }
    }

    #[test]
    fn luat_cot_khoa_va_index_theo_khach() {
        let tenant = || column("tenant_id", "uuid", false, None);
        let id = || column("id", "uuid_text", false, None);
        let snapshot = SchemaSnapshot {
            tables: vec![
                // Hợp lệ: khoá `id`, unique theo khách, index quét, khoá chỉ gồm UUID.
                table(
                    "shop_item",
                    vec![
                        tenant(),
                        id(),
                        column("sku", "text", false, None),
                        column("other_id", "uuid", false, None),
                    ],
                    vec![
                        index("shop_item_pkey", &["id"], true),
                        index("shop_item_sku", &["tenant_id", "sku"], true),
                        index("shop_item_link", &["id", "other_id"], true),
                        index("shop_item_expires_sweep_idx", &["sku"], false),
                    ],
                ),
                table(
                    "shop_bad",
                    vec![
                        column("tenant_id", "uuid", true, None),
                        id(),
                        column("sku", "text", false, None),
                    ],
                    vec![
                        index("shop_bad_sku", &["sku"], true),
                        index("shop_bad_sku_idx", &["sku"], false),
                    ],
                ),
                table(
                    "shop_defaulted",
                    vec![column("tenant_id", "uuid", false, Some("'x'"))],
                    vec![],
                ),
                table("shop_plain", vec![id()], vec![index("shop_plain_x", &["x"], false)]),
                table("shop_moved", vec![tenant(), id()], vec![]),
                table("shop_waiting", vec![id()], vec![]),
                table("shop_migrations", vec![], vec![]),
                table("kernel_secrets", vec![id()], vec![]),
            ],
        };
        let violations = schema_tenancy_violations(
            &snapshot,
            &["kernel_secrets"],
            &["shop_moved", "shop_waiting"],
        );
        let found: Vec<&str> =
            violations.iter().map(|text| text.split(':').next().unwrap_or_default()).collect();
        assert_eq!(violations.len(), 6, "{violations:#?}");
        assert!(violations.iter().any(|v| v.contains("`shop_bad.tenant_id` must be NOT NULL")));
        assert!(
            violations.iter().any(|v| v.contains("`shop_defaulted.tenant_id` must be NOT NULL"))
        );
        assert!(violations.iter().any(|v| v.contains("unique index `shop_bad_sku`")));
        assert!(violations.iter().any(|v| v.contains("index `shop_bad_sku_idx`")));
        assert!(violations.iter().any(|v| v.contains("table `shop_plain` has no `tenant_id`")));
        assert!(violations.iter().any(|v| v.contains("`shop_moved` has `tenant_id`: remove it")));
        assert!(!found.iter().any(|v| v.contains("shop_item") || v.contains("kernel_secrets")));
    }
}
