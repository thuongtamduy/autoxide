//! The organization tree within a tenant (step 34, D-185): head office → region → store / warehouse / seller…
//!
//! The kernel keeps the **structure** of the tree: two tenant-level tables, present in every database of the fleet:
//!
//! - `kernel_org_units`: one row per node: parent node, depth (root = 0), and the `writable` flag (`false`
//!   when the node is no longer active: pending approval, suspended, archived).
//! - `kernel_org_closure`: the ancestor-descendant table (closure table): one row per (ancestor, descendant)
//!   pair, with the distance; each node has one self-pointing row with distance 0. "Does node X belong to
//!   branch N" or "every record in branch N" is one indexed statement, with no recursion, behaving the same
//!   on SQLite and Postgres (no `ltree`, no `WITH RECURSIVE`).
//!
//! The **meaning** of a node (name, kind, status, address, settings) belongs to the `organizations` module.
//! The structure lives in the kernel because other modules filter their data by branch with one subquery on
//! `kernel_org_closure` ([`OrgTree::subtree_condition`]); a module must not read another module's tables
//! (architecture rule 1), but the kernel may.
//!
//! The root node of a tenant has an ID equal to the tenant's ID: tenant-level data carries
//! `org_id = tenant_id`, and upgrading old data only needs that assignment.
//!
//! Every function that changes the tree calls [`OrgTree::lock`] (locks the tenant's root row) BEFORE any
//! other row: moving a node and changing the status of the same tenant queue up, with no deadlock and no
//! cycle when two crossing move commands run at once.
use crate::time::Timestamp;
use sea_orm::sea_query::{Alias, Condition, Expr, ExprTrait as _, JoinType, Query};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, Set,
};
use uuid::Uuid;

use super::{OrgId, TenantId};
use crate::error::{Error, ErrorKind};
use crate::rows;

pub(crate) mod unit {
    //! The `kernel_org_units` table.
    use crate::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "kernel_org_units")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        /// `None` only at the root node.
        pub parent_id: Option<Uuid>,
        /// Distance to the root (root = 0).
        pub depth: i32,
        /// `false` when the node is inactive: neither the node nor its whole branch can be written.
        pub writable: bool,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod closure {
    //! The `kernel_org_closure` table.
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "kernel_org_closure")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub ancestor_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub descendant_id: Uuid,
        /// Distance from ancestor to descendant (self-pointing row: 0).
        pub depth: i32,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// ID of the root node of tenant `tenant` (equal to the tenant's ID).
pub const fn root_of(tenant: TenantId) -> OrgId {
    OrgId::from_uuid(tenant.as_uuid())
}

/// Tool for the organization tree structure. Every function takes a connection to the database that holds the
/// tenant's data (`tenant.database()`); functions that change the tree take a WRITE transaction
/// (`Database::begin_write`).
pub struct OrgTree;

impl OrgTree {
    /// Locks the tenant's root row until the end of `transaction`: every tree-changing command of the same
    /// tenant queues up. Call before any other row. A tenant with no root yet locks nothing (on SQLite the
    /// write permission on the whole database is held from the start of the transaction).
    pub async fn lock<C: crate::db::Executor>(
        transaction: &C,
        tenant: TenantId,
    ) -> Result<(), Error> {
        let transaction = &crate::db::Via::of(transaction);
        rows::lock_one(transaction, unit_by_id(tenant, root_of(tenant))).await?;
        Ok(())
    }

    /// Creates the tenant's root node (ID = the tenant's ID) if missing; re-runnable.
    pub async fn insert_root<C: crate::db::Executor>(
        transaction: &C,
        tenant: TenantId,
    ) -> Result<OrgId, Error> {
        let transaction = &crate::db::Via::of(transaction);
        let root = root_of(tenant);
        if unit_by_id(tenant, root).one(transaction).await?.is_some() {
            return Ok(root);
        }
        insert_unit(transaction, tenant, root, None, 0).await?;
        insert_closure(transaction, tenant, root, root, 0).await?;
        Ok(root)
    }

    /// Adds node `id` under `parent`; a tree deeper than `max_depth` levels (root = 0) gives
    /// `org_depth_exceeded`. A new node is writable; for an inactive node, also call
    /// [`set_writable`](Self::set_writable) in the same transaction.
    pub async fn insert<C: crate::db::Executor>(
        transaction: &C,
        tenant: TenantId,
        id: OrgId,
        parent: OrgId,
        max_depth: u8,
    ) -> Result<(), Error> {
        let transaction = &crate::db::Via::of(transaction);
        Self::lock(transaction, tenant).await?;
        let parent_row = find_unit(transaction, tenant, parent).await?;
        let depth = parent_row.depth + 1;
        if depth > i32::from(max_depth) {
            return Err(depth_exceeded(max_depth));
        }
        insert_unit(transaction, tenant, id, Some(parent), depth).await?;
        // Tổ tiên của nút mới = tổ tiên của cha (kể cả cha) + chính nó. Nhánh chỉ dài tối đa
        // `max_depth` dòng nên đọc vào bộ nhớ.
        let ancestors = closure::Entity::find()
            .filter(closure::Column::TenantId.eq(tenant.as_uuid()))
            .filter(closure::Column::DescendantId.eq(parent.as_uuid()))
            .all(transaction)
            .await?;
        let mut rows: Vec<closure::ActiveModel> = ancestors
            .into_iter()
            .map(|row| closure_row(tenant, row.ancestor_id, id.as_uuid(), row.depth + 1))
            .collect();
        rows.push(closure_row(tenant, id.as_uuid(), id.as_uuid(), 0));
        closure::Entity::insert_many(rows).exec(transaction).await?;
        Ok(())
    }

    /// Moves node `id` (with its whole branch) under `new_parent`. The root cannot be moved
    /// (`org_root_immutable`); moving into itself or its own branch → `org_cycle`; a tree deeper than
    /// `max_depth` → `org_depth_exceeded`.
    pub async fn move_subtree<C: crate::db::Executor>(
        transaction: &C,
        tenant: TenantId,
        id: OrgId,
        new_parent: OrgId,
        max_depth: u8,
    ) -> Result<(), Error> {
        let transaction = &crate::db::Via::of(transaction);
        Self::lock(transaction, tenant).await?;
        let node = find_unit(transaction, tenant, id).await?;
        if node.parent_id.is_none() {
            return Err(root_immutable());
        }
        let parent = find_unit(transaction, tenant, new_parent).await?;
        if node.parent_id == Some(new_parent.as_uuid()) {
            return Ok(());
        }
        let tenant_uuid = tenant.as_uuid();
        let subtree = closure::Entity::find()
            .filter(closure::Column::TenantId.eq(tenant_uuid))
            .filter(closure::Column::AncestorId.eq(id.as_uuid()));
        if subtree
            .clone()
            .filter(closure::Column::DescendantId.eq(new_parent.as_uuid()))
            .count(transaction)
            .await?
            > 0
        {
            return Err(cycle());
        }
        // Chiều cao của nhánh: khoảng cách xa nhất từ `id` tới con cháu.
        let height: Option<i32> = subtree
            .clone()
            .select_only()
            .column_as(closure::Column::Depth.max(), "height")
            .into_tuple()
            .one(transaction)
            .await?
            .flatten();
        let new_depth = parent.depth + 1;
        if new_depth + height.unwrap_or(0) > i32::from(max_depth) {
            return Err(depth_exceeded(max_depth));
        }

        // 1. Bỏ các cặp (tổ tiên ngoài nhánh, con cháu trong nhánh). Subquery được tính trước khi
        //    xoá trên cả hai database (không tương quan với dòng đang xoá).
        let in_subtree = || {
            Query::select()
                .column(closure::Column::DescendantId)
                .from(closure::Entity)
                .and_where(closure::Column::TenantId.eq(tenant_uuid))
                .and_where(closure::Column::AncestorId.eq(id.as_uuid()))
                .to_owned()
        };
        closure::Entity::delete_many()
            .filter(closure::Column::TenantId.eq(tenant_uuid))
            .filter(closure::Column::DescendantId.in_subquery(in_subtree()))
            .filter(closure::Column::AncestorId.not_in_subquery(in_subtree()))
            .exec(transaction)
            .await?;

        // 2. Nối mọi tổ tiên của cha mới (gồm cha mới) với mọi nút trong nhánh: tích chéo hai lượt
        //    đọc bảng tổ tiên (viết bằng `INNER JOIN … ON` điều kiện của lượt thứ hai).
        let (a, d) = (Alias::new("a"), Alias::new("d"));
        let pairs = Query::select()
            .expr(Expr::val(tenant_uuid))
            .column((a.clone(), closure::Column::AncestorId))
            .column((d.clone(), closure::Column::DescendantId))
            .expr(
                Expr::col((a.clone(), closure::Column::Depth))
                    .add(Expr::col((d.clone(), closure::Column::Depth)))
                    .add(1),
            )
            .from_as(closure::Entity, a.clone())
            .join_as(
                JoinType::InnerJoin,
                closure::Entity,
                d.clone(),
                Condition::all()
                    .add(Expr::col((d.clone(), closure::Column::TenantId)).eq(tenant_uuid))
                    .add(Expr::col((d, closure::Column::AncestorId)).eq(id.as_uuid())),
            )
            .and_where(Expr::col((a.clone(), closure::Column::TenantId)).eq(tenant_uuid))
            .and_where(Expr::col((a, closure::Column::DescendantId)).eq(new_parent.as_uuid()))
            .to_owned();
        let insert = Query::insert()
            .into_table(closure::Entity)
            .columns([
                closure::Column::TenantId,
                closure::Column::AncestorId,
                closure::Column::DescendantId,
                closure::Column::Depth,
            ])
            .select_from(pairs)
            .map_err(Error::internal)?
            .to_owned();
        let backend = transaction.get_database_backend();
        transaction.execute_raw(backend.build(&insert)).await?;

        // 3. Độ sâu của cả nhánh đổi cùng một lượng; cha của `id` đổi.
        let delta = new_depth - node.depth;
        let now = Timestamp::now();
        unit::Entity::update_many()
            .col_expr(unit::Column::Depth, Expr::col(unit::Column::Depth).add(delta))
            .col_expr(unit::Column::UpdatedAt, Expr::val(now))
            .filter(unit::Column::TenantId.eq(tenant_uuid))
            .filter(unit::Column::Id.in_subquery(in_subtree()))
            .exec(transaction)
            .await?;
        unit::Entity::update_many()
            .col_expr(unit::Column::ParentId, Expr::val(Some(new_parent.as_uuid())))
            .filter(unit::Column::TenantId.eq(tenant_uuid))
            .filter(unit::Column::Id.eq(id.as_uuid()))
            .exec(transaction)
            .await?;
        Ok(())
    }

    /// Sets the writable flag of node `id` (the `organizations` module calls it when the node status changes).
    pub async fn set_writable<C: crate::db::Executor>(
        transaction: &C,
        tenant: TenantId,
        id: OrgId,
        writable: bool,
    ) -> Result<(), Error> {
        let transaction = &crate::db::Via::of(transaction);
        Self::lock(transaction, tenant).await?;
        let mut row: unit::ActiveModel = find_unit(transaction, tenant, id).await?.into();
        row.writable = Set(writable);
        row.updated_at = Set(Timestamp::now());
        row.update(transaction).await?;
        Ok(())
    }

    /// The ancestor chain of node `id`, **including itself**, nearest first, root last. Empty when the node
    /// is not in the tree.
    pub async fn ancestors<C: crate::db::Executor>(
        connection: &C,
        tenant: TenantId,
        id: OrgId,
    ) -> Result<Vec<OrgId>, Error> {
        let connection = &crate::db::Via::of(connection);
        let rows = closure::Entity::find()
            .filter(closure::Column::TenantId.eq(tenant.as_uuid()))
            .filter(closure::Column::DescendantId.eq(id.as_uuid()))
            .order_by_asc(closure::Column::Depth)
            .all(connection)
            .await?;
        Ok(rows.into_iter().map(|row| OrgId::from_uuid(row.ancestor_id)).collect())
    }

    /// Whether `id` is a unit of `tenant`: the tenant's root (which exists even before a tree is built) or a
    /// node of its tree. A permission decision uses it so that a caller with a tenant-wide grant cannot
    /// attach a record to a unit that does not exist or belongs to another tenant (D-20261125-org-must-exist).
    pub async fn contains<C: crate::db::Executor>(
        connection: &C,
        tenant: TenantId,
        id: OrgId,
    ) -> Result<bool, Error> {
        let connection = &crate::db::Via::of(connection);
        if id == root_of(tenant) {
            return Ok(true);
        }
        let count = closure::Entity::find()
            .filter(closure::Column::TenantId.eq(tenant.as_uuid()))
            .filter(closure::Column::DescendantId.eq(id.as_uuid()))
            .filter(closure::Column::AncestorId.eq(id.as_uuid()))
            .count(connection)
            .await?;
        Ok(count > 0)
    }

    /// Like [`contains`](Self::contains), as an error: 404 `org_not_found` when `id` is not a unit of `tenant`.
    /// For a module that takes a unit from the caller outside a permission decision (a public endpoint, a
    /// unit used only as a label).
    pub async fn ensure_exists<C: crate::db::Executor>(
        connection: &C,
        tenant: TenantId,
        id: OrgId,
    ) -> Result<(), Error> {
        let connection = &crate::db::Via::of(connection);
        if Self::contains(connection, tenant, id).await? { Ok(()) } else { Err(org_not_found(id)) }
    }

    /// Whether node `id` belongs to the branch of one of `roots` (including those nodes themselves).
    pub async fn is_within<C: crate::db::Executor>(
        connection: &C,
        tenant: TenantId,
        id: OrgId,
        roots: &[OrgId],
    ) -> Result<bool, Error> {
        let connection = &crate::db::Via::of(connection);
        if roots.is_empty() {
            return Ok(false);
        }
        let count = closure::Entity::find()
            .filter(closure::Column::TenantId.eq(tenant.as_uuid()))
            .filter(closure::Column::DescendantId.eq(id.as_uuid()))
            .filter(closure::Column::AncestorId.is_in(roots.iter().map(OrgId::as_uuid)))
            .count(connection)
            .await?;
        Ok(count > 0)
    }

    /// `Ok` when node `id` and all its ancestors are writable; otherwise `org_frozen`. A node not in the tree
    /// (the tenant has not built a tree) counts as writable. Call INSIDE the module's write transaction:
    /// checking first and then opening the transaction leaves a gap between the two moments.
    ///
    /// Postgres: the rows of the ancestor chain are share-locked (`FOR SHARE`) until the transaction ends, so
    /// a node-locking command ([`set_writable`](Self::set_writable)) waits for every write in progress to
    /// finish; a write that starts after the locking command commits sees the new flag. SQLite: the write
    /// transaction already holds the write permission on the whole database.
    pub async fn ensure_writable<C: crate::db::Executor>(
        connection: &C,
        tenant: TenantId,
        id: OrgId,
    ) -> Result<(), Error> {
        let connection = &crate::db::Via::of(connection);
        let lineage = Query::select()
            .column(closure::Column::AncestorId)
            .from(closure::Entity)
            .and_where(closure::Column::TenantId.eq(tenant.as_uuid()))
            .and_where(closure::Column::DescendantId.eq(id.as_uuid()))
            .to_owned();
        // Khoá dòng gốc trước (như `lock`, nhưng chia sẻ): cùng thứ tự khoá với lệnh đổi cây —
        // gốc rồi mới tới nút khác — nên không deadlock.
        unit_by_id(tenant, root_of(tenant)).lock_shared().one(connection).await?;
        let rows = unit::Entity::find()
            .filter(unit::Column::TenantId.eq(tenant.as_uuid()))
            .filter(unit::Column::Id.in_subquery(lineage))
            .order_by_asc(unit::Column::Id)
            .lock_shared()
            .all(connection)
            .await?;
        if rows.iter().any(|row| !row.writable) {
            return Err(org_frozen(id));
        }
        Ok(())
    }

    /// Like [`ensure_writable`](Self::ensure_writable) but read-only, no lock: for places with no write
    /// transaction (an early check before opening a transaction, read connections). It cannot block a
    /// node-locking command in between: the actual write should still re-check with `ensure_writable` inside
    /// the transaction.
    pub async fn ensure_writable_unlocked<C: crate::db::Executor>(
        connection: &C,
        tenant: TenantId,
        id: OrgId,
    ) -> Result<(), Error> {
        let connection = &crate::db::Via::of(connection);
        let lineage = Query::select()
            .column(closure::Column::AncestorId)
            .from(closure::Entity)
            .and_where(closure::Column::TenantId.eq(tenant.as_uuid()))
            .and_where(closure::Column::DescendantId.eq(id.as_uuid()))
            .to_owned();
        let frozen = unit::Entity::find()
            .filter(unit::Column::TenantId.eq(tenant.as_uuid()))
            .filter(unit::Column::Id.in_subquery(lineage))
            .filter(unit::Column::Writable.eq(false))
            .count(connection)
            .await?;
        if frozen > 0 {
            return Err(org_frozen(id));
        }
        Ok(())
    }

    /// Every (ancestor, descendant) pair of the tenant's tree, including each node's self-pointing pair; lets
    /// in-memory tree aggregation be computed with one statement (e.g. the number of staff at each node).
    /// Row count = nodes × average depth.
    pub async fn pairs<C: crate::db::Executor>(
        connection: &C,
        tenant: TenantId,
    ) -> Result<Vec<(OrgId, OrgId)>, Error> {
        let connection = &crate::db::Via::of(connection);
        let rows: Vec<(Uuid, Uuid)> = closure::Entity::find()
            .select_only()
            .column(closure::Column::AncestorId)
            .column(closure::Column::DescendantId)
            .filter(closure::Column::TenantId.eq(tenant.as_uuid()))
            .into_tuple()
            .all(connection)
            .await?;
        Ok(rows
            .into_iter()
            .map(|(ancestor, descendant)| {
                (OrgId::from_uuid(ancestor), OrgId::from_uuid(descendant))
            })
            .collect())
    }

    /// Every node in the branch of one of `roots` (including those nodes themselves), without duplicates; for
    /// places that must filter by branch in a database OTHER than the one holding the tree (e.g. a combined
    /// reporting store in the root tenant's database, step 38), where the subquery of
    /// [`subtree_condition`](Self::subtree_condition) cannot run.
    pub async fn descendants<C: crate::db::Executor>(
        connection: &C,
        tenant: TenantId,
        roots: &[OrgId],
    ) -> Result<Vec<OrgId>, Error> {
        let connection = &crate::db::Via::of(connection);
        if roots.is_empty() {
            return Ok(Vec::new());
        }
        let rows: Vec<Uuid> = closure::Entity::find()
            .select_only()
            .column(closure::Column::DescendantId)
            .filter(closure::Column::TenantId.eq(tenant.as_uuid()))
            .filter(closure::Column::AncestorId.is_in(roots.iter().map(OrgId::as_uuid)))
            .distinct()
            .into_tuple()
            .all(connection)
            .await?;
        Ok(rows.into_iter().map(OrgId::from_uuid).collect())
    }

    /// SeaORM condition "column `column` is a node in the branch of one of `roots`": `column IN
    /// (SELECT descendant_id FROM kernel_org_closure WHERE tenant_id = … AND ancestor_id IN …)`.
    /// One indexed subquery, without pouring the ID list into memory.
    pub fn subtree_condition<Col: ColumnTrait>(
        tenant: TenantId,
        roots: &[OrgId],
        column: Col,
    ) -> Condition {
        Self::subtree_condition_on(tenant, roots, Expr::col((column.entity_name(), column)))
    }

    /// [`subtree_condition`](Self::subtree_condition) on a column given as an expression (the `db` layer).
    pub(crate) fn subtree_condition_on(
        tenant: TenantId,
        roots: &[OrgId],
        column: Expr,
    ) -> Condition {
        let descendants = Query::select()
            .column(closure::Column::DescendantId)
            .from(closure::Entity)
            .and_where(closure::Column::TenantId.eq(tenant.as_uuid()))
            .and_where(closure::Column::AncestorId.is_in(roots.iter().map(OrgId::as_uuid)))
            .to_owned();
        Condition::all().add(column.in_subquery(descendants))
    }
}

/// Template for "tree-inherited value": the value of the nearest node that has a value. `ancestors` nearest
/// first (as [`OrgTree::ancestors`]), `values` are the (node, value) pairs already set. Returns the source
/// node together with the value.
pub fn nearest<T>(
    ancestors: &[OrgId],
    values: impl IntoIterator<Item = (OrgId, T)>,
) -> Option<(OrgId, T)> {
    let mut best: Option<(usize, OrgId, T)> = None;
    for (org, value) in values {
        if let Some(rank) = ancestors.iter().position(|ancestor| *ancestor == org)
            && best.as_ref().is_none_or(|(current, _, _)| rank < *current)
        {
            best = Some((rank, org, value));
        }
    }
    best.map(|(_, org, value)| (org, value))
}

fn unit_by_id(tenant: TenantId, id: OrgId) -> sea_orm::Select<unit::Entity> {
    unit::Entity::find()
        .filter(unit::Column::TenantId.eq(tenant.as_uuid()))
        .filter(unit::Column::Id.eq(id.as_uuid()))
}

async fn find_unit<C: ConnectionTrait>(
    connection: &C,
    tenant: TenantId,
    id: OrgId,
) -> Result<unit::Model, Error> {
    unit_by_id(tenant, id).one(connection).await?.ok_or_else(|| org_not_found(id))
}

async fn insert_unit<C: ConnectionTrait>(
    connection: &C,
    tenant: TenantId,
    id: OrgId,
    parent: Option<OrgId>,
    depth: i32,
) -> Result<(), Error> {
    unit::ActiveModel {
        tenant_id: Set(tenant.as_uuid()),
        id: Set(id.as_uuid()),
        parent_id: Set(parent.map(|parent| parent.as_uuid())),
        depth: Set(depth),
        writable: Set(true),
        updated_at: Set(Timestamp::now()),
    }
    .insert(connection)
    .await?;
    Ok(())
}

async fn insert_closure<C: ConnectionTrait>(
    connection: &C,
    tenant: TenantId,
    ancestor: OrgId,
    descendant: OrgId,
    depth: i32,
) -> Result<(), Error> {
    closure_row(tenant, ancestor.as_uuid(), descendant.as_uuid(), depth).insert(connection).await?;
    Ok(())
}

fn closure_row(
    tenant: TenantId,
    ancestor: Uuid,
    descendant: Uuid,
    depth: i32,
) -> closure::ActiveModel {
    closure::ActiveModel {
        tenant_id: Set(tenant.as_uuid()),
        ancestor_id: Set(ancestor),
        descendant_id: Set(descendant),
        depth: Set(depth),
    }
}

pub(crate) fn org_not_found(id: OrgId) -> Error {
    Error::not_found("org_not_found", "There is no organization unit `{id}` in this tenant.")
        .with_param("id", id.as_uuid())
}

pub(crate) fn org_frozen(id: OrgId) -> Error {
    Error::forbidden(
        "org_frozen",
        "The organization unit `{org}` (or a unit above it) is pending, suspended or archived, so \
         nothing can be written there. Activate it first.",
    )
    .with_param("org", id.as_uuid())
}

fn depth_exceeded(max: u8) -> Error {
    Error::new(
        ErrorKind::Validation,
        "org_depth_exceeded",
        "The organization tree may be at most {max} levels deep. Choose a parent higher up.",
    )
    .with_param("max", max)
}

fn cycle() -> Error {
    Error::new(
        ErrorKind::Validation,
        "org_cycle",
        "A unit cannot be moved into itself or into one of its own sub-units. Choose another parent.",
    )
}

/// Error `org_root_immutable` (409): the root node cannot be moved, locked or archived. Modules that manage
/// the tree share this code.
pub fn root_immutable() -> Error {
    Error::conflict(
        "org_root_immutable",
        "The root unit of a tenant cannot be moved, suspended or archived.",
    )
}

/// Migration that creates the two tree tables (internal `kernel` module, tenant level).
pub(crate) struct CreateOrgTree;

mod migration {
    use crate::migration::prelude::*;

    impl MigrationName for super::CreateOrgTree {
        fn name(&self) -> &str {
            "m0014_create_org_tree"
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for super::CreateOrgTree {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .create_table(
                    Table::create()
                        .table("kernel_org_units")
                        .col(schema::uuid("tenant_id"))
                        .col(schema::uuid("id"))
                        .col(schema::uuid_null("parent_id"))
                        .col(schema::integer("depth"))
                        .col(schema::boolean("writable"))
                        .col(schema::timestamp_with_time_zone("updated_at"))
                        .primary_key(Index::create().col("tenant_id").col("id"))
                        .to_owned(),
                )
                .await?;
            manager
                .create_index(
                    Index::create()
                        .name("kernel_org_units_parent_idx")
                        .table("kernel_org_units")
                        .col("tenant_id")
                        .col("parent_id")
                        .to_owned(),
                )
                .await?;
            manager
                .create_table(
                    Table::create()
                        .table("kernel_org_closure")
                        .col(schema::uuid("tenant_id"))
                        .col(schema::uuid("ancestor_id"))
                        .col(schema::uuid("descendant_id"))
                        .col(schema::integer("depth"))
                        .primary_key(
                            Index::create()
                                .col("tenant_id")
                                .col("ancestor_id")
                                .col("descendant_id"),
                        )
                        .to_owned(),
                )
                .await?;
            manager
                .create_index(
                    Index::create()
                        .name("kernel_org_closure_descendant_idx")
                        .table("kernel_org_closure")
                        .col("tenant_id")
                        .col("descendant_id")
                        .col("depth")
                        .to_owned(),
                )
                .await
        }

        async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager.drop_table(Table::drop().table("kernel_org_closure").to_owned()).await?;
            manager.drop_table(Table::drop().table("kernel_org_units").to_owned()).await
        }
    }
}
