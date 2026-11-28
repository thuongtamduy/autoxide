//! From the policies of every module to the concrete work for each table in a split / merge run: the SQL
//! condition that picks the rows that go (read at the source database), whether they move or are only copied,
//! which tables wait to be empty, and which tables the module handles itself.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use sea_orm::sea_query::{Alias, Condition, Expr, ExprTrait as _, Query};

use super::{Merge, RelocateTable, Relocation, RelocationKind, SealedColumn, Split, TablePolicy};
use crate::error::Error;
use crate::tenancy::{OrgId, TenantId};
use crate::transfer::{TableKind, TenantTables, tenant_is};

/// Rows move (copied then deleted at the source) or are only copied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Move,
    Copy,
}

/// The work for one table.
#[derive(Clone)]
pub(crate) enum Treatment {
    /// Rows satisfying `condition` (at the source) are copied to the target; with `Move`, they are then deleted at the source.
    Select {
        mode: Mode,
        condition: Condition,
    },
    /// Change nothing.
    Leave,
    /// A queue: the tenant's due part must finish running before copying.
    Drain,
    /// A table handled by another table of the module.
    Handled,
    Custom(Arc<dyn RelocateTable>),
}

/// The work for one table, with its original policy.
#[derive(Clone)]
pub(crate) struct Resolved {
    pub(crate) table: String,
    pub(crate) policy: &'static str,
    pub(crate) treatment: Treatment,
    /// No module declares this table: the policy is inferred from the columns.
    pub(crate) inferred: bool,
    /// A table mirroring the organization tree: (node ID column, parent node column).
    pub(crate) org_tree: Option<(&'static str, &'static str)>,
}

/// The policies of every module, by table.
#[derive(Clone, Default)]
pub(crate) struct Policies {
    tables: HashMap<&'static str, TablePolicy>,
    sealed: Vec<SealedColumn>,
    switch_hooks: Vec<std::sync::Arc<dyn super::SwitchHook>>,
}

impl Policies {
    /// Collects the policies of the modules; a table must belong to that module and must not be declared twice.
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
            policies.switch_hooks.extend(relocation.switch_hooks().iter().cloned());
        }
        if problems.is_empty() { Ok(policies) } else { Err(problems) }
    }

    /// Adds the switch-step hooks modules contributed in `start` (`contribute::<dyn SwitchHook>`).
    pub(crate) fn add_switch_hooks(&mut self, hooks: Vec<std::sync::Arc<dyn super::SwitchHook>>) {
        self.switch_hooks.extend(hooks);
    }

    /// Switch-step hooks, every module.
    pub(crate) fn switch_hooks(&self) -> &[std::sync::Arc<dyn super::SwitchHook>] {
        &self.switch_hooks
    }

    /// Secret columns of `table`.
    pub(crate) fn sealed(&self, table: &str) -> Vec<SealedColumn> {
        self.sealed.iter().filter(|sealed| sealed.table == table).copied().collect()
    }

    /// Tenant-level tables (except queues) for which no module declares a policy.
    pub(crate) fn uncovered(&self, tables: &TenantTables) -> Vec<String> {
        tables
            .names()
            .into_iter()
            .filter(|name| tables.kind(name) == TableKind::Data && !self.tables.contains_key(name))
            .map(str::to_owned)
            .collect()
    }

    /// Policy of `table`: declared, or inferred from the columns.
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

    /// The work for every tenant-level table in `tables` (in table creation order) for run `kind` from tenant
    /// `source`; `org` is the branch root on a split.
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
    /// Rows of the source tenant whose `column` is a node of the branch.
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

/// Rows of tenant `source` whose `column` is in `other_column` of the `other` rows that satisfy
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
