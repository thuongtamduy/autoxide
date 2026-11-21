//! Entity của module `organizations` (SeaORM). Chỉ dùng trong module này. Cấu trúc cây (cha, độ
//! sâu, tổ tiên) nằm ở kernel (`OrgTree`); bảng ở đây giữ phần nghĩa của nút.

pub(crate) mod unit {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "organizations_unit")]
    pub struct Model {
        /// = ID của nút trong `kernel_org_units`; nút gốc có ID = ID của khách.
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        /// Bản sao cha của nút (cho lọc danh sách); nguồn đúng là cây của kernel.
        pub parent_id: Option<Uuid>,
        pub unit_type: String,
        pub name: String,
        /// `search::fold(name)`: tìm không dấu.
        pub name_folded: String,
        /// Mã nội bộ (vd mã cửa hàng), duy nhất trong khách khi có.
        pub code: Option<String>,
        /// `pending` / `active` / `suspended` / `archived`.
        pub status: String,
        pub status_reason: Option<String>,
        pub created_by: Option<Uuid>,
        pub created_at: Timestamp,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl autoxide_kernel::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}

pub(crate) mod unit_type {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "organizations_unit_type")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub code: String,
        pub name: String,
        pub description: String,
        pub created_at: Timestamp,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl autoxide_kernel::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}

pub(crate) mod setting {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "organizations_setting")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub org_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub key: String,
        /// JSON đã kiểm.
        pub value: String,
        pub updated_by: Option<Uuid>,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl autoxide_kernel::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}
