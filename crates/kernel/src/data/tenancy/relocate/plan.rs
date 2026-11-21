//! Kế hoạch của một lượt tách / gộp (dry-run): số dòng mỗi bảng, và **xung đột** tìm trước khi
//! chép — tham chiếu vắt qua ranh giới (theo foreign key trong module) và giá trị unique đã có ở
//! khách nhận. Không đổi gì.

use crate::time::Timestamp;
use sea_orm::sea_query::{Alias, Condition, Expr, ExprTrait as _, Query};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use super::policy::{Mode, Resolved, Treatment};
use super::{RelocationConflict, RelocationContext, RelocationTable, RelocationWarning};
use crate::database::Database;
use crate::error::Error;
use crate::tenancy::{OrgId, TenantId};
use crate::transfer::{
    TableKind, TenantTables, TransferError, foreign_keys, primary_key, tenant_is, unique_indexes,
};

fn transfer(error: TransferError) -> Error {
    Error::internal(error.to_string())
}

/// Cột khách — không so trong khoá (đổi khách là chủ đích).
const TENANT: &str = "tenant_id";

/// Mọi nút của nhánh `org` trong cây của khách `tenant` (gồm chính nó).
pub(crate) async fn branch(
    database: &Database,
    tenant: TenantId,
    org: OrgId,
) -> Result<Vec<Uuid>, Error> {
    let nodes = crate::tenancy::OrgTree::descendants(database.write(), tenant, &[org]).await?;
    Ok(nodes.into_iter().map(|node| node.as_uuid()).collect())
}

/// Khoá chính của các dòng đi / được chép (bảng có cột `id` kiểu UUID) cùng các nút của nhánh —
/// cho bảng `Custom` biết thứ gì đi theo (audit, dãy số).
pub(crate) async fn moved_ids(
    tables: &TenantTables,
    resolved: &[Resolved],
    source: &Database,
    branch: &[Uuid],
) -> Result<HashSet<Uuid>, Error> {
    let mut moved: HashSet<Uuid> = branch.iter().copied().collect();
    for table in resolved {
        let Treatment::Select { condition, .. } = &table.treatment else { continue };
        if !tables.is_uuid_column(&table.table, "id") {
            continue;
        }
        let ids = tables
            .values(source.write(), &table.table, &["id".to_owned()], condition.clone())
            .await
            .map_err(transfer)?;
        moved.extend(ids.into_iter().filter_map(|row| row[0].as_deref()?.parse::<Uuid>().ok()));
    }
    Ok(moved)
}

/// Điều kiện "dòng đến hạn còn chờ" của một hàng đợi của khách `tenant`.
pub(crate) fn pending(kind: TableKind, tenant: TenantId) -> Condition {
    let condition = tenant_is(tenant).add(Expr::col(Alias::new("failed_at")).is_null());
    match kind {
        TableKind::Jobs => condition
            .add(Expr::col(Alias::new("completed_at")).is_null())
            .add(Expr::col(Alias::new("run_at")).lte(Timestamp::now())),
        _ => condition.add(Expr::col(Alias::new("delivered_at")).is_null()),
    }
}

/// Kết quả kế hoạch của mọi bảng.
pub(crate) struct Report {
    pub(crate) tables: Vec<RelocationTable>,
    pub(crate) conflicts: Vec<RelocationConflict>,
    pub(crate) warnings: Vec<RelocationWarning>,
}

/// Lập kế hoạch cho các bảng `resolved` của lượt `context`.
pub(crate) async fn report(
    tables: &TenantTables,
    resolved: &[Resolved],
    context: &RelocationContext,
) -> Result<Report, Error> {
    let (source, target) = (context.source(), context.target());
    let mut report = Report { tables: Vec::new(), conflicts: Vec::new(), warnings: Vec::new() };
    let by_name: HashMap<&str, &Resolved> =
        resolved.iter().map(|table| (table.table.as_str(), table)).collect();
    for table in resolved {
        let name = table.table.as_str();
        if table.inferred && tables.kind(name) == TableKind::Data {
            report.warnings.push(RelocationWarning {
                table: name.to_owned(),
                code: "relocation_policy_inferred".to_owned(),
                message: format!(
                    "No module declares how `{name}` moves; it is treated as `{}`.",
                    table.policy
                ),
            });
        }
        let (action, rows, existing) = match &table.treatment {
            Treatment::Select { mode, condition } => {
                let rows = tables
                    .count_matching(source, name, condition.clone())
                    .await
                    .map_err(transfer)?;
                let keys = Keys::read(tables, name, source, target, condition, context).await?;
                report.conflicts.extend(keys.unique_conflicts(name));
                let action = if *mode == Mode::Move { "move" } else { "copy" };
                (action, rows, keys.existing())
            }
            Treatment::Leave => {
                let rows = tables
                    .count_matching(source, name, tenant_is(context.source_tenant()))
                    .await
                    .map_err(transfer)?;
                ("stay", rows, 0)
            }
            Treatment::Drain => {
                let rows = tables
                    .count_matching(
                        source,
                        name,
                        pending(tables.kind(name), context.source_tenant()),
                    )
                    .await
                    .map_err(transfer)?;
                ("drain", rows, 0)
            }
            Treatment::Handled => ("custom", 0, 0),
            Treatment::Custom(handler) => {
                let planned = handler.plan(context).await?;
                report.conflicts.extend(planned.conflicts);
                report.warnings.extend(planned.warnings.into_iter().map(|message| {
                    RelocationWarning { table: name.to_owned(), code: "custom".to_owned(), message }
                }));
                ("custom", planned.rows, 0)
            }
        };
        report.tables.push(RelocationTable {
            table: name.to_owned(),
            policy: table.policy.to_owned(),
            action: action.to_owned(),
            rows,
            existing,
        });
        report.conflicts.extend(reference_conflicts(tables, table, &by_name, context).await?);
    }
    Ok(report)
}

/// Tham chiếu vắt qua ranh giới theo foreign key của `child`: dòng đi mà dòng cha không có ở đích,
/// hay dòng ở lại mà dòng cha bị xoá ở nguồn.
async fn reference_conflicts(
    tables: &TenantTables,
    child: &Resolved,
    by_name: &HashMap<&str, &Resolved>,
    context: &RelocationContext,
) -> Result<Vec<RelocationConflict>, Error> {
    let source = context.source();
    let name = child.table.as_str();
    let mut found = Vec::new();
    let keys = foreign_keys(source.write(), name).await.map_err(transfer)?;
    if keys.is_empty() {
        return Ok(found);
    }
    let pk: Vec<String> = primary_key(source.write(), name)
        .await
        .map_err(transfer)?
        .into_iter()
        .filter(|column| column != TENANT)
        .collect();
    let mine = tenant_is(context.source_tenant());
    for key in keys {
        let Some(parent) = by_name.get(key.parent.as_str()) else { continue };
        let parent_rows = |condition: Condition| {
            Expr::col(Alias::new(key.column.as_str())).in_subquery(
                Query::select()
                    .column(Alias::new(key.parent_column.as_str()))
                    .from(Alias::new(key.parent.as_str()))
                    .cond_where(condition)
                    .to_owned(),
            )
        };
        // Dòng đi mà cha không đi theo (cũng không được chép).
        if let Treatment::Select { condition, .. } = &child.treatment {
            let mut orphans =
                condition.clone().add(Expr::col(Alias::new(key.column.as_str())).is_not_null());
            if let Treatment::Select { condition: parent_condition, .. } = &parent.treatment {
                orphans = orphans.add(parent_rows(parent_condition.clone()).not());
            }
            let rows = tables.values(source.write(), name, &pk, orphans).await.map_err(transfer)?;
            if !rows.is_empty() {
                found.push(RelocationConflict::new(
                    name,
                    "reference",
                    format!(
                        "Rows of `{name}` would go while the row of `{}` they point to (column \
                         `{}`) stays.",
                        key.parent, key.column
                    ),
                    rows.iter().map(|row| join(row)).collect(),
                ));
            }
        }
        // Dòng ở lại mà cha bị xoá ở nguồn.
        if let Treatment::Select { mode: Mode::Move, condition: parent_condition } =
            &parent.treatment
        {
            let referring = tables
                .values(
                    source.write(),
                    name,
                    &pk,
                    mine.clone().add(parent_rows(parent_condition.clone())),
                )
                .await
                .map_err(transfer)?;
            let leaving: HashSet<Vec<Option<String>>> = match &child.treatment {
                Treatment::Select { mode: Mode::Move, condition } => tables
                    .values(source.write(), name, &pk, condition.clone())
                    .await
                    .map_err(transfer)?
                    .into_iter()
                    .collect(),
                _ => HashSet::new(),
            };
            let staying: Vec<String> = referring
                .iter()
                .filter(|row| !leaving.contains(*row))
                .map(|row| join(row))
                .collect();
            if !staying.is_empty() {
                found.push(RelocationConflict::new(
                    name,
                    "reference",
                    format!(
                        "Rows of `{name}` would stay while the row of `{}` they point to (column \
                         `{}`) goes.",
                        key.parent, key.column
                    ),
                    staying,
                ));
            }
        }
    }
    Ok(found)
}

fn join(values: &[Option<String>]) -> String {
    values.iter().map(|value| value.as_deref().unwrap_or("null")).collect::<Vec<_>>().join(",")
}

/// Một unique index: cột (bỏ `tenant_id`), giá trị → khoá chính của dòng đi ở nguồn, giá trị → khoá
/// chính của dòng đã có ở đích.
type UniqueKeys = (
    Vec<String>,
    Vec<(Vec<String>, Vec<Option<String>>)>,
    HashMap<Vec<String>, Vec<Option<String>>>,
);

/// Khoá của các dòng đi ở nguồn và của khách nhận ở đích, để đếm dòng đã có và tìm unique trùng.
struct Keys {
    /// Khoá chính (bỏ `tenant_id`) của dòng đi.
    moving: HashSet<Vec<Option<String>>>,
    /// Khoá chính của dòng đã có ở đích.
    existing: HashSet<Vec<Option<String>>>,
    /// Theo từng unique index: (cột, giá trị → khoá chính ở nguồn, giá trị → khoá chính ở đích).
    unique: Vec<UniqueKeys>,
}

impl Keys {
    async fn read(
        tables: &TenantTables,
        name: &str,
        source: &Database,
        target: &Database,
        condition: &Condition,
        context: &RelocationContext,
    ) -> Result<Self, Error> {
        let pk: Vec<String> = primary_key(source.write(), name)
            .await
            .map_err(transfer)?
            .into_iter()
            .filter(|column| column != TENANT)
            .collect();
        let target_rows = tenant_is(context.target_tenant());
        let moving: HashSet<_> = tables
            .values(source.write(), name, &pk, condition.clone())
            .await
            .map_err(transfer)?
            .into_iter()
            .collect();
        let existing: HashSet<_> = if moving.is_empty() {
            HashSet::new()
        } else {
            tables
                .values(target.write(), name, &pk, target_rows.clone())
                .await
                .map_err(transfer)?
                .into_iter()
                .collect()
        };
        let mut unique = Vec::new();
        if !moving.is_empty() {
            for index in unique_indexes(source.write(), name).await.map_err(transfer)? {
                let scoped = index.iter().any(|column| column == TENANT);
                let columns: Vec<String> =
                    index.into_iter().filter(|column| column != TENANT).collect();
                if columns.is_empty() {
                    continue;
                }
                let wanted: Vec<String> = columns.iter().chain(pk.iter()).cloned().collect();
                let split = |rows: Vec<Vec<Option<String>>>| {
                    rows.into_iter()
                        .filter_map(|row| {
                            let (value, key) = row.split_at(columns.len());
                            // NULL không trùng với gì trong unique index.
                            let value: Option<Vec<String>> = value.iter().cloned().collect();
                            Some((value?, key.to_vec()))
                        })
                        .collect::<Vec<_>>()
                };
                let from = split(
                    tables
                        .values(source.write(), name, &wanted, condition.clone())
                        .await
                        .map_err(transfer)?,
                );
                let filter = if scoped { target_rows.clone() } else { Condition::all() };
                let to: HashMap<_, _> = split(
                    tables.values(target.write(), name, &wanted, filter).await.map_err(transfer)?,
                )
                .into_iter()
                .collect();
                unique.push((columns, from, to));
            }
        }
        Ok(Keys { moving, existing, unique })
    }

    /// Dòng đi mà khoá chính đã có ở đích.
    fn existing(&self) -> u64 {
        self.moving.intersection(&self.existing).count() as u64
    }

    fn unique_conflicts(&self, name: &str) -> Vec<RelocationConflict> {
        let mut found = Vec::new();
        for (columns, from, to) in &self.unique {
            let rows: Vec<String> = from
                .iter()
                .filter(|(value, key)| to.get(value).is_some_and(|other| other != key))
                .map(|(value, key)| format!("{} ({})", join(key), value.join(",")))
                .collect();
            if !rows.is_empty() {
                found.push(RelocationConflict::new(
                    name,
                    "unique",
                    format!(
                        "The target tenant already has rows of `{name}` with the same `{}`; rename \
                         them on one side first.",
                        columns.join(", ")
                    ),
                    rows,
                ));
            }
        }
        found
    }
}
