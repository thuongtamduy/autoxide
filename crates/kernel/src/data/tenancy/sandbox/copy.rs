//! Sao cấu hình vào sandbox và đẩy thứ đã thử sang khách thật (bước 38, D-232,
//! D-20261119-step38-sandbox-copy).
//!
//! - Module khai [`SandboxCopy`] qua `Module::sandbox_copy`: job `kernel.sandbox_copy` gọi theo thứ
//!   tự đăng ký, sau khi gieo dữ liệu khách mới cho sandbox và trước khi nhân viên vào được. Chỉ sao
//!   CẤU HÌNH — không bao giờ người dùng, người mua, đơn, chứng từ, thông báo, file riêng tư, secret.
//!   Bản sao có ID mới; nối về bản gốc bằng [`link_copy`] (bảng `kernel_sandbox_links`), nhờ đó chạy
//!   lại không tạo bản trùng và tham chiếu giữa bản ghi đổi theo bản gốc ([`copied`]).
//! - Module khai [`Promotable`] qua `Module::promotable`: `POST /kernel/sandboxes/{id}/promote` gọi
//!   [`PromoteApply::promote`] trong ngữ cảnh khách thật, với quyền của chính người gọi. Bên khách
//!   thật luôn nhận BẢN NHÁP (không tự đăng, không tự bật), không bao giờ ghi đè bản đang chạy.

use std::sync::{Arc, OnceLock};

use chrono::{SubsecRound, Utc};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};
use uuid::Uuid;

use crate::access::Principal;
use crate::error::Error;
use crate::tenancy::{ScopeExt, TenantContext, TenantScoped};

pub(crate) mod entity {
    //! Bảng `kernel_sandbox_links` (migration `m0038_create_sandbox_tables`).
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "kernel_sandbox_links")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        /// Sandbox sở hữu dòng.
        pub tenant_id: Uuid,
        /// Loại bản ghi `<module>.<loại>`.
        pub kind: String,
        /// Bản ghi ở sandbox.
        pub record_id: Uuid,
        /// Bản ghi gốc ở khách thật mà nó được sao từ; `None`: tạo mới trong sandbox.
        pub source_id: Option<Uuid>,
        /// Bản ghi ở khách thật lần đẩy gần nhất tạo / sửa.
        pub promoted_id: Option<Uuid>,
        /// Dấu vân tay nội dung lần đẩy gần nhất (đẩy lại cùng nội dung → `unchanged`).
        pub promoted_fingerprint: Option<String>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

impl TenantScoped for entity::Entity {
    fn tenant_column() -> Self::Column {
        entity::Column::TenantId
    }
}

/// Tuỳ chọn khi tạo sandbox.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SandboxOptions {
    /// Số sản phẩm tối đa được sao (0 khi không nêu).
    pub products: u32,
}

/// Những gì một lần sao nhận.
#[non_exhaustive]
pub struct CopyContext<'a> {
    /// Khách thật (đọc).
    pub source: &'a TenantContext,
    /// Sandbox (ghi, qua service của module).
    pub sandbox: &'a TenantContext,
    pub options: SandboxOptions,
    /// Bên làm của job sao: mọi permission ở sandbox (sandbox là của chính khách, job do chủ khách
    /// tạo). Không bao giờ dùng ở khách thật.
    pub principal: &'a Principal,
}

/// Module sao cấu hình của mình vào sandbox.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait SandboxCopy: Send + Sync {
    /// Sao cấu hình từ `context.source` sang `context.sandbox`; trả số bản ghi đã sao lần này. Phải
    /// chạy lại được: bản đã sao ([`copied`]) thì bỏ qua. Không bao giờ sao dữ liệu cá nhân, giao
    /// dịch hay secret.
    async fn copy(&self, context: &CopyContext<'_>) -> Result<u64, Error>;
}

/// Một mục cần đẩy sang khách thật.
#[non_exhaustive]
pub struct PromoteRequest<'a> {
    pub sandbox: &'a TenantContext,
    /// Khách thật (khách gốc của sandbox).
    pub target: &'a TenantContext,
    /// Người gọi, ở khách thật: module kiểm quyền của chính module như REST.
    pub principal: &'a Principal,
    /// Bản ghi ở sandbox.
    pub id: Uuid,
    /// Bản ghi ở khách thật nên nhận thay đổi: bản lần đẩy trước tạo, không có thì bản gốc mà bản
    /// ghi sandbox được sao từ. `None`: tạo bản nháp mới. Module kiểm bản này còn tồn tại.
    pub existing: Option<Uuid>,
}

/// Kết quả đẩy một mục.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Promoted {
    /// Tạo bản nháp mới ở khách thật.
    CreatedDraft(Uuid),
    /// Sửa bản nháp / bản sửa đổi chờ đăng của bản ghi ở khách thật (bản đang chạy giữ nguyên).
    UpdatedDraft(Uuid),
    /// Không có gì khác bản đã đẩy.
    Unchanged(Uuid),
    /// Trùng tên / đường dẫn / mã ở khách thật: mục này không đẩy, mục khác vẫn chạy.
    Conflict { field: String, message: String },
}

/// Module đẩy một loại bản ghi từ sandbox sang khách thật.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait PromoteApply: Send + Sync {
    /// Ghi bản ghi `request.id` của sandbox sang khách thật qua service của module, LUÔN ở dạng nháp
    /// / tắt. `fingerprint` của nội dung đã đẩy được kernel giữ: module trả `Unchanged` khi nội
    /// dung bằng `previous` (cùng hàm băm module dùng).
    async fn promote(
        &self,
        request: &PromoteRequest<'_>,
        previous: Option<&str>,
    ) -> Result<(Promoted, Option<String>), Error>;
}

/// Một loại bản ghi đẩy được sang khách thật.
#[derive(Clone)]
#[non_exhaustive]
pub struct Promotable {
    /// `<module>.<loại>`, vd `content.page`.
    pub kind: &'static str,
    /// Mô tả tiếng Anh cho người gọi API.
    pub description: &'static str,
    pub apply: Arc<dyn PromoteApply>,
}

impl Promotable {
    pub fn new(
        kind: &'static str,
        description: &'static str,
        apply: Arc<dyn PromoteApply>,
    ) -> Self {
        Promotable { kind, description, apply }
    }
}

/// Sổ sao / đẩy của mọi module, điền một lần sau khi mọi module khởi động. Clone rẻ.
#[derive(Clone, Default)]
pub(crate) struct SandboxCatalog(Arc<OnceLock<CatalogInner>>);

struct CatalogInner {
    copies: Vec<(&'static str, Arc<dyn SandboxCopy>)>,
    promotables: Vec<(&'static str, Promotable)>,
}

impl SandboxCatalog {
    pub(crate) fn fill(
        &self,
        copies: Vec<(&'static str, Arc<dyn SandboxCopy>)>,
        promotables: Vec<(&'static str, Promotable)>,
    ) {
        let _ = self.0.set(CatalogInner { copies, promotables });
    }

    pub(crate) fn copies(&self) -> Vec<(&'static str, Arc<dyn SandboxCopy>)> {
        self.0.get().map(|inner| inner.copies.clone()).unwrap_or_default()
    }

    pub(crate) fn promotables(&self) -> Vec<(&'static str, Promotable)> {
        self.0.get().map(|inner| inner.promotables.clone()).unwrap_or_default()
    }
}

fn now() -> chrono::DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

/// Ghi bản ghi `record_id` của sandbox là bản sao của `source_id` (loại `kind`). Gọi lại không thêm
/// dòng thứ hai.
pub async fn link_copy(
    sandbox: &TenantContext,
    kind: &str,
    record_id: Uuid,
    source_id: Uuid,
) -> Result<(), Error> {
    let at = now();
    let row = entity::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(sandbox.id().as_uuid()),
        kind: Set(kind.to_owned()),
        record_id: Set(record_id),
        source_id: Set(Some(source_id)),
        promoted_id: Set(None),
        promoted_fingerprint: Set(None),
        created_at: Set(at),
        updated_at: Set(at),
    };
    entity::Entity::insert(row)
        .on_conflict(
            OnConflict::columns([
                entity::Column::TenantId,
                entity::Column::Kind,
                entity::Column::RecordId,
            ])
            .do_nothing()
            .to_owned(),
        )
        .exec_without_returning(sandbox.database().write())
        .await?;
    Ok(())
}

/// Bản sao ở sandbox của bản ghi gốc `source_id` (loại `kind`), nếu đã sao.
pub async fn copied(
    sandbox: &TenantContext,
    kind: &str,
    source_id: Uuid,
) -> Result<Option<Uuid>, Error> {
    Ok(entity::Entity::find()
        .filter(entity::Column::Kind.eq(kind))
        .filter(entity::Column::SourceId.eq(source_id))
        .scoped(sandbox)
        .one(sandbox.database().read())
        .await?
        .map(|row| row.record_id))
}

/// Bản ghi gốc ở khách thật của bản ghi `record_id` trong sandbox, hay bản lần đẩy trước tạo.
pub async fn promoted_target(
    sandbox: &TenantContext,
    kind: &str,
    record_id: Uuid,
) -> Result<Option<Uuid>, Error> {
    Ok(link_of(sandbox, kind, record_id).await?.and_then(|row| row.promoted_id.or(row.source_id)))
}

pub(crate) async fn link_of(
    sandbox: &TenantContext,
    kind: &str,
    record_id: Uuid,
) -> Result<Option<entity::Model>, Error> {
    Ok(entity::Entity::find()
        .filter(entity::Column::Kind.eq(kind))
        .filter(entity::Column::RecordId.eq(record_id))
        .scoped(sandbox)
        .one(sandbox.database().read())
        .await?)
}

/// Ghi kết quả lần đẩy `record_id` (bản ở khách thật, dấu vân tay nội dung).
pub(crate) async fn record_promotion(
    sandbox: &TenantContext,
    kind: &str,
    record_id: Uuid,
    promoted_id: Uuid,
    fingerprint: Option<String>,
) -> Result<(), Error> {
    let at = now();
    match link_of(sandbox, kind, record_id).await? {
        Some(row) => {
            entity::Entity::update_many()
                .col_expr(entity::Column::PromotedId, Expr::value(Some(promoted_id)))
                .col_expr(entity::Column::PromotedFingerprint, Expr::value(fingerprint))
                .col_expr(entity::Column::UpdatedAt, Expr::value(at))
                .filter(entity::Column::Id.eq(row.id))
                .exec(sandbox.database().write())
                .await?;
        }
        None => {
            entity::Entity::insert(entity::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(sandbox.id().as_uuid()),
                kind: Set(kind.to_owned()),
                record_id: Set(record_id),
                source_id: Set(None),
                promoted_id: Set(Some(promoted_id)),
                promoted_fingerprint: Set(fingerprint),
                created_at: Set(at),
                updated_at: Set(at),
            })
            .exec(sandbox.database().write())
            .await?;
        }
    }
    Ok(())
}
