//! `m0025_subject_request_fulfil`: hạn thực hiện yêu cầu của chủ thể dữ liệu (LG-4b, NĐ 356/2025/NĐ-CP Điều 5 —
//! cần đối chiếu văn bản gốc).

use autoxide_kernel::migration::prelude::*;

/// Cột `fulfil_due_at` (trống được: yêu cầu ghi trước bản này, hay quy tắc đang có hiệu lực không đặt hạn thực
/// hiện cho loại đó) và index `(tenant_id, status, fulfil_due_at)` cho bộ lọc "quá hạn".
pub(super) struct SubjectRequestFulfil;

const TABLE: &str = "compliance_subject_requests";
const INDEX: &str = "compliance_subject_requests_fulfil_idx";

impl MigrationName for SubjectRequestFulfil {
    fn name(&self) -> &str {
        "m0025_subject_request_fulfil"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for SubjectRequestFulfil {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(TABLE)
                    .add_column(schema::timestamp_with_time_zone_null("fulfil_due_at"))
                    .to_owned(),
            )
            .await?;
        super::index(manager, TABLE, INDEX, &["tenant_id", "status", "fulfil_due_at"], false).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_index(Index::drop().name(INDEX).table(Alias::new(TABLE)).to_owned()).await?;
        manager
            .alter_table(Table::alter().table(TABLE).drop_column("fulfil_due_at").to_owned())
            .await
    }
}
