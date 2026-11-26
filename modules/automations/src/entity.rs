//! Bảng của module `automations` — mọi bảng là cấp khách hàng (`tenant_id`).

/// `impl TenantScoped` cho entity có cột `tenant_id`.
macro_rules! tenant_scoped {
    () => {
        impl autoxide_kernel::tenancy::TenantScoped for Entity {
            fn tenant_column() -> Self::Column {
                Column::TenantId
            }
        }
    };
}

pub(crate) mod rule {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "automations_rules")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub org_id: Uuid,
        pub name: String,
        pub description: Option<String>,
        /// `event`, `schedule`, `manual`, `metric`.
        pub trigger_type: String,
        /// Tên event (`event`) hay số liệu (`metric`).
        pub trigger_ref: Option<String>,
        /// JSON của kích hoạt, điều kiện (`null`: luôn đúng), hành động.
        pub trigger: String,
        pub conditions: String,
        pub actions: String,
        /// `active`, `disabled`, `pending_approval`, `paused`.
        pub status: String,
        /// Vì sao luật bị tạm dừng (`rate_limit`, `permission_lost`, `owner_unavailable`, `credential_revoked`,
        /// `owner_access_revoked`).
        pub status_reason: Option<String>,
        /// Múi giờ IANA của lịch.
        pub timezone: String,
        /// Hành động chạy với quyền HIỆN TẠI của người này.
        pub run_as: Uuid,
        /// API key đã tạo / sửa / bật luật (`None`: người dùng đăng nhập). Key bị thu hồi thì luật
        /// tạm dừng.
        pub run_as_api_key: Option<Uuid>,
        /// Bản chụp quyền của key (JSON `PrincipalSnapshot`): luật không bao giờ có hơn quyền
        /// của key, kể cả hàng rào nút.
        pub run_as_limit: Option<String>,
        pub created_by: Uuid,
        /// `user`, `agent`, `service`… (loại bên tạo luật).
        pub created_by_kind: String,
        pub updated_by: Uuid,
        pub version: i32,
        /// Lần chạy theo lịch / kiểm số liệu kế tiếp.
        pub next_run_at: Option<Timestamp>,
        pub created_at: Timestamp,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod run {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "automations_runs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub org_id: Uuid,
        pub rule_id: Uuid,
        pub rule_version: i32,
        /// Khoá chống chạy trùng: `event:<id>`, `schedule:<mốc>`, `metric:<ngày>`, `manual:<id>`.
        pub trigger_key: String,
        pub event_id: Option<Uuid>,
        pub event_name: Option<String>,
        /// Độ sâu chuỗi luật: 1 cho lần chạy do người / lịch / event thường gây ra.
        pub depth: i32,
        /// `queued`, `running`, `waiting`, `awaiting_approval`, `succeeded`, `failed`, `skipped`.
        pub status: String,
        /// Bước kế tiếp sẽ chạy (từ 0).
        pub cursor: i32,
        /// JSON: dữ liệu mẫu được dùng (`event`, `trigger`).
        pub context: String,
        /// JSON: bản chụp `conditions` và `actions` lúc kích hoạt.
        pub definition: String,
        /// JSON: nhật ký từng bước.
        pub steps: String,
        pub error_code: Option<String>,
        pub error_message: Option<String>,
        /// Bước đang chờ duyệt / đã được duyệt (chạy được một lần).
        pub approval_step: Option<i32>,
        pub decided_by: Option<Uuid>,
        /// Người gọi chạy tay / chạy lại.
        pub triggered_by: Option<Uuid>,
        /// Lúc chạy tiếp sau `wait`.
        pub resume_at: Option<Timestamp>,
        pub created_at: Timestamp,
        pub updated_at: Timestamp,
        pub finished_at: Option<Timestamp>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod run_key {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "automations_run_keys")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub rule_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub trigger_key: String,
        pub run_id: Uuid,
        pub created_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod metric_hour {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "automations_metric_hours")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        /// `fact:<loại>` hay `summary:<id>`.
        #[sea_orm(primary_key, auto_increment = false)]
        pub source: String,
        /// Đầu giờ UTC.
        #[sea_orm(primary_key, auto_increment = false)]
        pub hour: Timestamp,
        /// Mã tiền; rỗng khi không phải tiền.
        #[sea_orm(primary_key, auto_increment = false)]
        pub currency: String,
        /// Tổng (số liệu báo cáo) hay mẫu đọc gần nhất (số liệu tóm tắt).
        pub value: i64,
        pub samples: i64,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod metric_seen {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "automations_metric_seen")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub event_id: Uuid,
        pub created_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}
