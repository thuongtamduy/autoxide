//! Từ chính sách của mọi module tới việc cụ thể của từng bảng trong một lượt tách / gộp: điều kiện
//! SQL chọn dòng đi (đọc ở database nguồn), đi hẳn hay chỉ chép, bảng nào chờ rỗng, bảng nào module
//! tự làm.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use sea_orm::sea_query::{Alias, Condition, Expr, ExprTrait as _, Query};

use super::{Merge, RelocateTable, Relocation, RelocationKind, SealedColumn, Split, TablePolicy};
use crate::error::Error;
use crate::tenancy::{OrgId, TenantId};
use crate::transfer::{TableKind, TenantTables, tenant_is};

/// Dòng đi hẳn (chép rồi xoá ở nguồn) hay chỉ được chép.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Move,
    Copy,
}

/// Việc của một bảng.
#[derive(Clone)]
pub(crate) enum Treatment {
    /// Dòng thoả `condition` (ở nguồn) được chép sang đích; `Move` thì sau đó xoá ở nguồn.
    Select {
        mode: Mode,
        condition: Condition,
    },
    /// Không đổi gì.
    Leave,
    /// Hàng đợi: phần đến hạn của khách phải chạy hết trước khi chép.
    Drain,
    /// Bảng do bảng khác của module xử lý.
    Handled,
    Custom(Arc<dyn RelocateTable>),
}

/// Việc của một bảng, kèm chính sách gốc.
#[derive(Clone)]
pub(crate) struct Resolved {
    pub(crate) table: String,
    pub(crate) policy: &'static str,
    pub(crate) treatment: Treatment,
    /// Không module nào khai bảng này: chính sách suy từ cột.
    pub(crate) inferred: bool,
    /// Bảng soi cây tổ chức: (cột ID nút, cột nút cha).
    pub(crate) org_tree: Option<(&'static str, &'static str)>,
}

/// Chính sách của mọi module, theo bảng.
#[derive(Clone, Default)]
pub(crate) struct Policies {
    tables: HashMap<&'static str, TablePolicy>,
    sealed: Vec<SealedColumn>,
}

impl Policies {
    /// Gom chính sách của các module; bảng phải thuộc chính module đó và không khai hai lần.
    pub(crate) fn collect(declared: Vec<(&'static str, Relocation)>) -> Result<Self, Vec<String>> {
        let mut problems = Vec::new();
        let mut policies = Policies::default();
        for (module, relocation) in declared {
            for (table, policy) in relocation.tables() {
                if !table.starts_with(&format!("{module}_")) {
                    problems.push(format!(
                        "module `{module}` declares a relocation policy for `{table}`, which is \
                         not one of its tables (`{module}_…`)"
                    ));
                } else if policies.tables.insert(table, policy.clone()).is_some() {
                    problems.push(format!("table `{table}` has two relocation policies"));
                }
            }
            for sealed in relocation.sealed_columns() {
                if !sealed.table.starts_with(&format!("{module}_")) {
                    problems.push(format!(
                        "module `{module}` declares a sealed column of `{}`, which is not one of \
                         its tables",
                        sealed.table
                    ));
                }
                policies.sealed.push(*sealed);
            }
        }
        if problems.is_empty() { Ok(policies) } else { Err(problems) }
    }

    /// Cột secret của `table`.
    pub(crate) fn sealed(&self, table: &str) -> Vec<SealedColumn> {
        self.sealed.iter().filter(|sealed| sealed.table == table).copied().collect()
    }

    /// Bảng cấp khách (trừ hàng đợi) không module nào khai chính sách.
    pub(crate) fn uncovered(&self, tables: &TenantTables) -> Vec<String> {
        tables
            .names()
            .into_iter()
            .filter(|name| tables.kind(name) == TableKind::Data && !self.tables.contains_key(name))
            .map(str::to_owned)
            .collect()
    }

    /// Chính sách của `table`: đã khai, hay suy từ cột.
    fn policy(&self, tables: &TenantTables, table: &str) -> (TablePolicy, bool) {
        if let Some(policy) = self.tables.get(table) {
            return (policy.clone(), false);
        }
        let inferred = if tables.kind(table) != TableKind::Data {
            TablePolicy::MustBeEmpty
        } else if tables.has_column(table, "org_id") {
            TablePolicy::ByOrg { column: "org_id" }
        } else {
            TablePolicy::TenantWide { on_split: Split::Stay, on_merge: Merge::Import }
        };
        (inferred, true)
    }

    /// Việc của mọi bảng cấp khách trong `tables` (theo thứ tự tạo bảng) cho lượt `kind` từ khách
    /// `source`; `org` là gốc nhánh khi tách.
    pub(crate) fn resolve(
        &self,
        tables: &TenantTables,
        kind: RelocationKind,
        source: TenantId,
        org: OrgId,
    ) -> Result<Vec<Resolved>, Error> {
        let mut resolver = Resolver {
            policies: self,
            tables,
            kind,
            source,
            org,
            done: HashMap::new(),
            visiting: HashSet::new(),
        };
        let mut resolved = Vec::new();
        let order: HashMap<&str, usize> =
            tables.names().into_iter().enumerate().map(|(index, name)| (name, index)).collect();
        for name in tables.names() {
            let (policy, inferred) = self.policy(tables, name);
            // Dọn xoá theo thứ tự ngược thứ tự tạo bảng: bảng con phải được tạo sau bảng cha nó
            // theo, không thì điều kiện của con không còn thấy dòng cha khi tới lượt xoá.
            if let TablePolicy::FollowParent { parent_table, .. } = &policy
                && order.get(parent_table).is_some_and(|parent| *parent > order[name])
            {
                return Err(Error::internal(format!(
                    "table `{name}` follows `{parent_table}`, which is created after it"
                )));
            }
            let treatment = resolver.treatment(name)?;
            let org_tree = match policy {
                TablePolicy::OrgTree { id, parent } => Some((id, parent)),
                _ => None,
            };
            resolved.push(Resolved {
                table: name.to_owned(),
                policy: policy.name(),
                treatment,
                inferred,
                org_tree,
            });
        }
        Ok(resolved)
    }
}

struct Resolver<'a> {
    policies: &'a Policies,
    tables: &'a TenantTables,
    kind: RelocationKind,
    source: TenantId,
    org: OrgId,
    done: HashMap<String, Treatment>,
    visiting: HashSet<String>,
}

impl Resolver<'_> {
    /// Dòng của khách nguồn có `column` là một nút của nhánh.
    fn in_branch(&self, column: &str) -> Condition {
        tenant_is(self.source).add(
            Expr::col(Alias::new(column)).in_subquery(
                Query::select()
                    .column(Alias::new("descendant_id"))
                    .from(Alias::new("kernel_org_closure"))
                    .and_where(Expr::col(Alias::new("tenant_id")).eq(self.source.as_uuid()))
                    .and_where(Expr::col(Alias::new("ancestor_id")).eq(self.org.as_uuid()))
                    .to_owned(),
            ),
        )
    }

    fn treatment(&mut self, table: &str) -> Result<Treatment, Error> {
        if let Some(found) = self.done.get(table) {
            return Ok(found.clone());
        }
        if !self.tables.contains(table) {
            return Err(Error::internal(format!(
                "a relocation policy refers to table `{table}`, which is not a tenant-level table \
                 of the installed modules"
            )));
        }
        if !self.visiting.insert(table.to_owned()) {
            return Err(Error::internal(format!(
                "the relocation policies of `{table}` refer to each other in a loop"
            )));
        }
        let (policy, _) = self.policies.policy(self.tables, table);
        let source = self.source;
        let all = move || Treatment::Select { mode: Mode::Move, condition: tenant_is(source) };
        let merge = self.kind == RelocationKind::Merge;
        let treatment = match policy {
            TablePolicy::MustBeEmpty => Treatment::Drain,
            TablePolicy::HandledBy { .. } => Treatment::Handled,
            TablePolicy::Custom(handler) => Treatment::Custom(handler),
            TablePolicy::TenantWide { on_merge: Merge::Skip, .. } if merge => Treatment::Leave,
            TablePolicy::TenantWide { on_split: Split::Stay, .. } if !merge => Treatment::Leave,
            TablePolicy::TenantWide { on_split: Split::Copy, .. } if !merge => {
                Treatment::Select { mode: Mode::Copy, condition: tenant_is(self.source) }
            }
            TablePolicy::TenantWide { .. } => all(),
            TablePolicy::ByOrg { .. } | TablePolicy::OrgTree { .. } if merge => all(),
            TablePolicy::ByOrg { column } | TablePolicy::OrgTree { id: column, .. } => {
                Treatment::Select { mode: Mode::Move, condition: self.in_branch(column) }
            }
            TablePolicy::FollowParent { parent_table, column, parent_column } => {
                match self.treatment(parent_table)? {
                    Treatment::Select { mode, condition } => Treatment::Select {
                        mode,
                        condition: if merge {
                            tenant_is(self.source)
                        } else {
                            follow(self.source, column, parent_table, parent_column, condition)
                        },
                    },
                    Treatment::Leave => Treatment::Leave,
                    _ => {
                        return Err(Error::internal(format!(
                            "table `{table}` follows `{parent_table}`, whose rows are not chosen \
                             by a plain policy"
                        )));
                    }
                }
            }
            TablePolicy::Referenced { by_table, by_column, column } => {
                match self.treatment(by_table)? {
                    Treatment::Select { .. } if merge => all(),
                    Treatment::Select { condition, .. } => Treatment::Select {
                        mode: Mode::Copy,
                        condition: follow(self.source, column, by_table, by_column, condition),
                    },
                    Treatment::Leave => Treatment::Leave,
                    _ => {
                        return Err(Error::internal(format!(
                            "table `{table}` is referenced by `{by_table}`, whose rows are not \
                             chosen by a plain policy"
                        )));
                    }
                }
            }
        };
        self.visiting.remove(table);
        self.done.insert(table.to_owned(), treatment.clone());
        Ok(treatment)
    }
}

/// Dòng của khách `source` có `column` nằm trong `other_column` của các dòng `other` thoả
/// `condition`.
fn follow(
    source: TenantId,
    column: &str,
    other: &str,
    other_column: &str,
    condition: Condition,
) -> Condition {
    tenant_is(source).add(
        Expr::col(Alias::new(column)).in_subquery(
            Query::select()
                .column(Alias::new(other_column))
                .from(Alias::new(other))
                .cond_where(condition)
                .to_owned(),
        ),
    )
}
