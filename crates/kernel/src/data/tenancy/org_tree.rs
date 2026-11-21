//! Cây tổ chức trong một khách (bước 34, D-185): trụ sở → vùng → cửa hàng / kho / người bán…
//!
//! Kernel giữ **cấu trúc** của cây — hai bảng cấp khách hàng, có ở mọi database của đội:
//!
//! - `kernel_org_units`: mỗi nút một dòng — nút cha, độ sâu (gốc = 0), cờ `writable` (`false` khi
//!   nút không còn hoạt động: chờ duyệt, tạm khoá, lưu trữ).
//! - `kernel_org_closure`: bảng tổ tiên – con cháu (closure table): mỗi cặp (tổ tiên, con cháu) một
//!   dòng, kèm khoảng cách; mỗi nút có một dòng tự trỏ khoảng cách 0. Câu "nút X có thuộc nhánh N
//!   không" hay "mọi bản ghi trong nhánh N" là một câu có index, không đệ quy — chạy như nhau trên
//!   SQLite và Postgres (không `ltree`, không `WITH RECURSIVE`).
//!
//! Phần **nghĩa** của nút (tên, loại, trạng thái, địa chỉ, cài đặt) là của module `organizations`.
//! Cấu trúc ở kernel vì module khác lọc dữ liệu của mình theo nhánh bằng một subquery trên
//! `kernel_org_closure` ([`OrgTree::subtree_condition`]) — module không được đọc bảng của module
//! khác (luật kiến trúc 1), còn kernel thì được.
//!
//! Nút gốc của khách có ID = ID của khách: dữ liệu cấp khách mang `org_id = tenant_id`, và nâng cấp
//! dữ liệu cũ chỉ cần gán như vậy.
//!
//! Mọi hàm đổi cây gọi [`OrgTree::lock`] (khoá dòng gốc của khách) TRƯỚC mọi dòng khác: dời nút và
//! đổi trạng thái của cùng khách nối đuôi nhau, không deadlock, không tạo vòng khi hai lệnh dời
//! chéo nhau chạy cùng lúc.
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
    //! Bảng `kernel_org_units`.
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
        /// `None` chỉ ở nút gốc.
        pub parent_id: Option<Uuid>,
        /// Khoảng cách tới gốc (gốc = 0).
        pub depth: i32,
        /// `false` khi nút không hoạt động: nút và cả nhánh dưới nó không ghi được.
        pub writable: bool,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod closure {
    //! Bảng `kernel_org_closure`.
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
        /// Khoảng cách từ tổ tiên tới con cháu (dòng tự trỏ: 0).
        pub depth: i32,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// ID của nút gốc của khách `tenant` (bằng ID của khách).
pub const fn root_of(tenant: TenantId) -> OrgId {
    OrgId::from_uuid(tenant.as_uuid())
}

/// Công cụ cho cấu trúc cây tổ chức. Mọi hàm nhận kết nối của database chứa dữ liệu của khách
/// (`tenant.database()`); hàm đổi cây nhận transaction GHI (`Database::begin_write`).
pub struct OrgTree;

impl OrgTree {
    /// Khoá dòng gốc của khách tới hết `transaction`: mọi lệnh đổi cây của cùng khách nối đuôi
    /// nhau. Gọi trước mọi dòng khác. Khách chưa có gốc thì không khoá gì (trên SQLite quyền ghi
    /// cả database đã giữ từ đầu transaction).
    pub async fn lock<C: ConnectionTrait>(transaction: &C, tenant: TenantId) -> Result<(), Error> {
        rows::lock_one(transaction, unit_by_id(tenant, root_of(tenant))).await?;
        Ok(())
    }

    /// Tạo nút gốc của khách (ID = ID của khách) nếu chưa có; chạy lại được.
    pub async fn insert_root<C: ConnectionTrait>(
        transaction: &C,
        tenant: TenantId,
    ) -> Result<OrgId, Error> {
        let root = root_of(tenant);
        if unit_by_id(tenant, root).one(transaction).await?.is_some() {
            return Ok(root);
        }
        insert_unit(transaction, tenant, root, None, 0).await?;
        insert_closure(transaction, tenant, root, root, 0).await?;
        Ok(root)
    }

    /// Thêm nút `id` dưới `parent`; cây sâu quá `max_depth` cấp (gốc = 0) thì
    /// `org_depth_exceeded`. Nút mới ghi được; nút chưa hoạt động thì gọi thêm
    /// [`set_writable`](Self::set_writable) trong cùng transaction.
    pub async fn insert<C: ConnectionTrait>(
        transaction: &C,
        tenant: TenantId,
        id: OrgId,
        parent: OrgId,
        max_depth: u8,
    ) -> Result<(), Error> {
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

    /// Dời nút `id` (cùng cả nhánh dưới nó) sang dưới `new_parent`. Gốc không dời được
    /// (`org_root_immutable`); dời vào chính nó hay nhánh của nó → `org_cycle`; cây sâu quá
    /// `max_depth` → `org_depth_exceeded`.
    pub async fn move_subtree<C: ConnectionTrait>(
        transaction: &C,
        tenant: TenantId,
        id: OrgId,
        new_parent: OrgId,
        max_depth: u8,
    ) -> Result<(), Error> {
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

    /// Đặt cờ ghi được của nút `id` (module `organizations` gọi khi trạng thái nút đổi).
    pub async fn set_writable<C: ConnectionTrait>(
        transaction: &C,
        tenant: TenantId,
        id: OrgId,
        writable: bool,
    ) -> Result<(), Error> {
        Self::lock(transaction, tenant).await?;
        let mut row: unit::ActiveModel = find_unit(transaction, tenant, id).await?.into();
        row.writable = Set(writable);
        row.updated_at = Set(Timestamp::now());
        row.update(transaction).await?;
        Ok(())
    }

    /// Chuỗi tổ tiên của nút `id`, **gồm chính nó**, gần nhất trước, gốc cuối cùng. Rỗng khi nút
    /// không có trong cây.
    pub async fn ancestors<C: ConnectionTrait>(
        connection: &C,
        tenant: TenantId,
        id: OrgId,
    ) -> Result<Vec<OrgId>, Error> {
        let rows = closure::Entity::find()
            .filter(closure::Column::TenantId.eq(tenant.as_uuid()))
            .filter(closure::Column::DescendantId.eq(id.as_uuid()))
            .order_by_asc(closure::Column::Depth)
            .all(connection)
            .await?;
        Ok(rows.into_iter().map(|row| OrgId::from_uuid(row.ancestor_id)).collect())
    }

    /// Nút `id` có thuộc nhánh của một trong `roots` (gồm chính các nút đó) không.
    pub async fn is_within<C: ConnectionTrait>(
        connection: &C,
        tenant: TenantId,
        id: OrgId,
        roots: &[OrgId],
    ) -> Result<bool, Error> {
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

    /// `Ok` khi nút `id` và mọi tổ tiên của nó ghi được; không thì `org_frozen`. Nút không có
    /// trong cây (khách chưa dựng cây) coi như ghi được. Gọi TRONG transaction ghi của module —
    /// kiểm trước rồi mới mở transaction thì có khe giữa hai lúc.
    ///
    /// Postgres: các dòng của chuỗi tổ tiên được khoá chia sẻ (`FOR SHARE`) tới hết transaction,
    /// nên lệnh khoá nút ([`set_writable`](Self::set_writable)) chờ mọi lệnh ghi đang dở xong; lệnh
    /// ghi bắt đầu sau khi lệnh khoá commit thì thấy cờ mới. SQLite: transaction ghi đã giữ quyền
    /// ghi cả database.
    pub async fn ensure_writable<C: ConnectionTrait>(
        connection: &C,
        tenant: TenantId,
        id: OrgId,
    ) -> Result<(), Error> {
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

    /// Như [`ensure_writable`](Self::ensure_writable) nhưng chỉ đọc, không khoá — cho chỗ không có
    /// transaction ghi (kiểm sớm trước khi mở transaction, kết nối đọc). Không chặn được lệnh khoá
    /// nút xen giữa: lệnh ghi thật vẫn nên kiểm lại bằng `ensure_writable` trong transaction.
    pub async fn ensure_writable_unlocked<C: ConnectionTrait>(
        connection: &C,
        tenant: TenantId,
        id: OrgId,
    ) -> Result<(), Error> {
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

    /// Mọi cặp (tổ tiên, con cháu) của cây của khách, kể cả cặp tự trỏ của mỗi nút — cho phép tính
    /// gộp theo cây trong bộ nhớ bằng một câu (vd số người làm được ở mỗi nút). Số dòng = số nút ×
    /// độ sâu trung bình.
    pub async fn pairs<C: ConnectionTrait>(
        connection: &C,
        tenant: TenantId,
    ) -> Result<Vec<(OrgId, OrgId)>, Error> {
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

    /// Mọi nút trong nhánh của một trong `roots` (gồm chính các nút đó), không trùng — cho chỗ phải
    /// lọc theo nhánh ở một database KHÁC database chứa cây (vd kho báo cáo gộp ở database của
    /// khách gốc, bước 38), nơi subquery của [`subtree_condition`](Self::subtree_condition) không
    /// chạy được.
    pub async fn descendants<C: ConnectionTrait>(
        connection: &C,
        tenant: TenantId,
        roots: &[OrgId],
    ) -> Result<Vec<OrgId>, Error> {
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

    /// Điều kiện SeaORM "cột `column` là một nút trong nhánh của một trong `roots`": `column IN
    /// (SELECT descendant_id FROM kernel_org_closure WHERE tenant_id = … AND ancestor_id IN …)`.
    /// Một subquery có index, không đổ danh sách ID ra bộ nhớ.
    pub fn subtree_condition<Col: ColumnTrait>(
        tenant: TenantId,
        roots: &[OrgId],
        column: Col,
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

/// Khuôn "giá trị kế thừa theo cây": giá trị của nút gần nhất có giá trị. `ancestors` gần nhất
/// trước (như [`OrgTree::ancestors`]), `values` là các cặp (nút, giá trị) đã đặt. Trả nút nguồn
/// kèm giá trị.
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

/// Lỗi `org_root_immutable` (409): nút gốc không dời, khoá hay lưu trữ được. Module quản lý cây
/// dùng chung mã này.
pub fn root_immutable() -> Error {
    Error::conflict(
        "org_root_immutable",
        "The root unit of a tenant cannot be moved, suspended or archived.",
    )
}

/// Migration tạo hai bảng của cây (module nội bộ `kernel`, cấp khách hàng).
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
