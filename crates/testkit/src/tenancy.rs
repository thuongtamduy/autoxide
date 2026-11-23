//! Checks the per-tenant schema (step 33, D-253) on a migrated database:
//!
//! - tenant-level tables (every table except control-plane tables a module declares through
//!   `Module::control_plane_tables` and the history table `<m>_migrations`) have `tenant_id` NOT NULL,
//!   with no `DEFAULT`;
//! - every unique key (primary keys included) starts with `tenant_id`, except a key made only of UUID columns
//!   (IDs generated in the application, globally unique — e.g. the primary key `id`);
//! - every ordinary index starts with `tenant_id`, except an index that sweeps the whole database and is named `…_sweep_idx`.

use std::collections::HashMap;
use std::sync::Mutex;

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{
    SettingValue, TenantContext, TenantId, TenantSetting, TenantSettings,
};

use crate::schema::{SchemaSnapshot, TableSnapshot};

/// In-memory per-tenant settings, for tests that build a module's service by hand (not through `TestApp`).
/// Does not validate values like the kernel: whatever the test sets, the module reads. Tests that need the
/// real kernel (checkers, REST) use `TestApp` and `app.api::<dyn TenantSettings>()`.
#[derive(Debug, Default)]
pub struct FakeTenantSettings(Mutex<HashMap<(TenantId, String), String>>);

impl FakeTenantSettings {
    /// Tenant `tenant` sets `name` = `value`.
    pub fn set(&self, tenant: TenantId, name: &str, value: &str) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert((tenant, name.to_owned()), value.to_owned());
    }

    /// Tenant `tenant` removes `name` (falls back to the shared setting).
    pub fn remove(&self, tenant: TenantId, name: &str) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&(tenant, name.to_owned()));
    }
}

#[async_trait::async_trait]
impl TenantSettings for FakeTenantSettings {
    async fn get(
        &self,
        tenant: &TenantContext,
        setting: &TenantSetting,
    ) -> Result<Option<SettingValue>, Error> {
        let values = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        // Như kernel: tên hiện tại trước, không có thì tên cũ của cấu hình đã đổi tên.
        let value = std::iter::once(setting.name())
            .chain(setting.former_name())
            .find_map(|name| values.get(&(tenant.id(), name.to_owned())));
        Ok(value.map(SettingValue::new))
    }
}

/// The violations (sorted); empty means valid. `control_plane`: the declared control-plane tables.
pub fn schema_tenancy_violations(snapshot: &SchemaSnapshot, control_plane: &[&str]) -> Vec<String> {
    let mut violations = Vec::new();
    for table in &snapshot.tables {
        let name = table.name.as_str();
        if name.ends_with("_migrations") || control_plane.contains(&name) {
            continue;
        }
        let tenant = table.columns.iter().find(|column| column.name == "tenant_id");
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
                table("shop_migrations", vec![], vec![]),
                table("kernel_secrets", vec![id()], vec![]),
            ],
        };
        let violations = schema_tenancy_violations(&snapshot, &["kernel_secrets"]);
        let found: Vec<&str> =
            violations.iter().map(|text| text.split(':').next().unwrap_or_default()).collect();
        assert_eq!(violations.len(), 5, "{violations:#?}");
        assert!(violations.iter().any(|v| v.contains("`shop_bad.tenant_id` must be NOT NULL")));
        assert!(
            violations.iter().any(|v| v.contains("`shop_defaulted.tenant_id` must be NOT NULL"))
        );
        assert!(violations.iter().any(|v| v.contains("unique index `shop_bad_sku`")));
        assert!(violations.iter().any(|v| v.contains("index `shop_bad_sku_idx`")));
        assert!(violations.iter().any(|v| v.contains("table `shop_plain` has no `tenant_id`")));
        assert!(!found.iter().any(|v| v.contains("shop_item") || v.contains("kernel_secrets")));
    }
}
