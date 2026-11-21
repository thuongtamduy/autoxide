//! Bảng `kernel_relocations` (mặt điều khiển): mỗi lượt tách / gộp — hay yêu cầu của khách — một
//! dòng, đi qua các trạng thái của [`super::run`].

use crate::time::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{RelocationKind, RelocationPlan, TenantRelocation};
use crate::error::Error;
use crate::migration::prelude::*;
use crate::tenancy::TenantTableRows;

pub(crate) const REQUESTED: &str = "requested";
pub(crate) const PLANNED: &str = "planned";
pub(crate) const FROZEN: &str = "frozen";
pub(crate) const COPIED: &str = "copied";
pub(crate) const VERIFIED: &str = "verified";
pub(crate) const SWITCHED: &str = "switched";
pub(crate) const CLEANED: &str = "cleaned";
pub(crate) const DONE: &str = "done";
pub(crate) const FAILED: &str = "failed";
/// Trạng thái của lượt đang chạy dở: khách nguồn (và khách nhận khi gộp) không phục vụ.
pub(crate) const UNFINISHED: [&str; 6] = [PLANNED, FROZEN, COPIED, VERIFIED, SWITCHED, CLEANED];

pub(crate) mod entity {
    //! Bảng `kernel_relocations` (mặt điều khiển).
    use crate::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "kernel_relocations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        /// `split` / `merge`.
        pub kind: String,
        pub state: String,
        pub source_tenant_id: Uuid,
        /// Tách: ID khách mới (= nút gốc nhánh). Gộp: khách nhận.
        pub target_tenant_id: Option<Uuid>,
        /// Tách: nút gốc nhánh. Gộp: nút nhận của khách nhận.
        pub org_id: Option<Uuid>,
        /// Tách: placement của khách mới.
        pub placement_id: Option<Uuid>,
        /// JSON [`super::Options`].
        pub options: String,
        /// JSON kế hoạch lúc bắt đầu (hay lúc khách gửi yêu cầu).
        pub plan: Option<String>,
        /// JSON `[{"table": …, "rows": …}]`: dòng đã ghi ở đích mỗi bảng.
        pub tables: String,
        pub error: Option<String>,
        pub runner: Option<String>,
        pub runner_until: Option<Timestamp>,
        /// Người của khách gửi yêu cầu (`requested`).
        pub requested_by: Option<Uuid>,
        pub created_by: Option<Uuid>,
        pub created_at: Timestamp,
        pub updated_at: Timestamp,
        pub finished_at: Option<Timestamp>,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) use entity::{Column, Entity, Model};

/// Tuỳ chọn của một lượt, lưu ở cột `options`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Options {
    /// Tách: slug và tên của khách mới.
    #[serde(default)]
    pub(crate) slug: Option<String>,
    #[serde(default)]
    pub(crate) name: Option<String>,
    /// Tách: khách mới là con của khách nguồn.
    #[serde(default)]
    pub(crate) keep_parent: bool,
    /// Lý do (bắt buộc: chuyển dữ liệu giữa hai khách — căn cứ pháp lý, D-20261118-step38-relocation).
    #[serde(default)]
    pub(crate) reason: String,
    /// Yêu cầu: tên placement khách mong muốn (nhân sự nền tảng quyết).
    #[serde(default)]
    pub(crate) placement: Option<String>,
    /// Gộp: khách nhận (ID hay slug) — yêu cầu chưa có `target_tenant_id`.
    #[serde(default)]
    pub(crate) into: Option<String>,
    /// Khách gốc của cây trước và sau (cho `kernel.relocation_finished`, dời kho báo cáo).
    #[serde(default)]
    pub(crate) from_root: Option<Uuid>,
    #[serde(default)]
    pub(crate) to_root: Option<Uuid>,
    /// Lúc khách bắt đầu bảo trì (chờ hàng đợi rỗng tính từ đây).
    #[serde(default)]
    pub(crate) frozen_at: Option<Timestamp>,
}

impl Options {
    pub(crate) fn parse(raw: &str) -> Result<Self, Error> {
        serde_json::from_str(raw).map_err(Error::internal)
    }

    pub(crate) fn json(&self) -> Result<String, Error> {
        serde_json::to_string(self).map_err(Error::internal)
    }
}

/// Lượt đã lưu, cho người gọi.
pub(crate) fn describe(row: Model, placement: Option<String>) -> Result<TenantRelocation, Error> {
    let options = Options::parse(&row.options)?;
    let plan: Option<RelocationPlan> = match &row.plan {
        Some(raw) => Some(serde_json::from_str(raw).map_err(Error::internal)?),
        None => None,
    };
    let tables: Vec<TenantTableRows> =
        serde_json::from_str(&row.tables).map_err(Error::internal)?;
    Ok(TenantRelocation {
        id: row.id,
        kind: RelocationKind::parse(&row.kind).ok_or_else(|| {
            Error::internal(format!("relocation {} has kind `{}`", row.id, row.kind))
        })?,
        state: row.state,
        source_tenant_id: row.source_tenant_id,
        target_tenant_id: row.target_tenant_id,
        org_id: row.org_id,
        placement: placement.or(options.placement),
        reason: options.reason,
        plan,
        tables,
        error: row.error,
        created_at: row.created_at,
        updated_at: row.updated_at,
        finished_at: row.finished_at,
    })
}

/// Bảng `kernel_relocations` (mặt điều khiển, chỉ database điều khiển có dữ liệu).
pub(crate) struct CreateRelocations;

impl MigrationName for CreateRelocations {
    fn name(&self) -> &str {
        "m0035_create_relocations"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateRelocations {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("kernel_relocations")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("kind", 16))
                    .col(schema::string_len("state", 16))
                    .col(schema::uuid("source_tenant_id"))
                    .col(schema::uuid_null("target_tenant_id"))
                    .col(schema::uuid_null("org_id"))
                    .col(schema::uuid_null("placement_id"))
                    .col(schema::text("options"))
                    .col(schema::text_null("plan"))
                    .col(schema::text("tables"))
                    .col(schema::text_null("error"))
                    .col(schema::string_len_null("runner", 64))
                    .col(schema::timestamp_with_time_zone_null("runner_until"))
                    .col(schema::uuid_null("requested_by"))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .col(schema::timestamp_with_time_zone_null("finished_at"))
                    .to_owned(),
            )
            .await?;
        for (name, columns) in [
            ("kernel_relocations_source_idx", ["source_tenant_id", "state"]),
            ("kernel_relocations_target_idx", ["target_tenant_id", "state"]),
            ("kernel_relocations_state_idx", ["state", "created_at"]),
        ] {
            let mut index = Index::create();
            index.name(name).table("kernel_relocations");
            for column in columns {
                index.col(column);
            }
            manager.create_index(index.to_owned()).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("kernel_relocations").to_owned()).await
    }
}

/// ID mới cho một lượt.
pub(crate) fn new_id() -> Uuid {
    Uuid::now_v7()
}
