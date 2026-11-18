//! Bảng của module `reporting` — mọi bảng là cấp khách hàng (`tenant_id`).
//!
//! - `reporting_local_facts`: số liệu rút từ event, ở database của khách NGUỒN (`tenant_id` là
//!   khách nguồn); `forwarded_at` null = chưa đẩy lên kho của cây.
//! - `reporting_facts`, `reporting_fact_dimensions`, `reporting_daily`: kho của cây, ở database
//!   của khách GỐC (`tenant_id` là khách gốc, `source_tenant_id` là khách nguồn).

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

pub(crate) mod local_fact {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "reporting_local_facts")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub source_event_id: Uuid,
        /// Thứ tự của số liệu trong event (một event có thể sinh nhiều số liệu).
        pub fact_index: i32,
        pub kind: String,
        /// Nút sở hữu; số liệu không nêu nút thì là nút gốc (= ID của khách).
        pub org_id: Uuid,
        pub occurred_at: DateTimeUtc,
        /// Ngày theo múi giờ của nút nguồn lúc rút.
        pub local_day: Date,
        pub amount_minor: Option<i64>,
        pub amount_currency: Option<String>,
        pub quantity: Option<i64>,
        /// JSON `{tên chiều: giá trị}`.
        pub dimensions: String,
        pub created_at: DateTimeUtc,
        pub forwarded_at: Option<DateTimeUtc>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod fact {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "reporting_facts")]
    pub struct Model {
        /// Bằng ID của số liệu cục bộ.
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        /// Khách gốc của cây (chủ kho).
        pub tenant_id: Uuid,
        pub source_tenant_id: Uuid,
        pub kind: String,
        pub org_id: Uuid,
        pub occurred_at: DateTimeUtc,
        pub local_day: Date,
        pub amount_minor: Option<i64>,
        pub amount_currency: Option<String>,
        pub quantity: Option<i64>,
        pub dimensions: String,
        pub received_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod dimension {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "reporting_fact_dimensions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub fact_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub name: String,
        pub value: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod daily {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "reporting_daily")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub source_tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub org_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub kind: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub local_day: Date,
        /// Chuỗi rỗng khi số liệu không có tiền: khoá chạy như nhau trên hai database.
        #[sea_orm(primary_key, auto_increment = false)]
        pub currency: String,
        pub amount_minor: i64,
        pub quantity: i64,
        pub fact_count: i64,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod rehome {
    //! Nhật ký dời kho báo cáo (bước 38): một dòng mỗi lượt tách / gộp, ở database của khách nhận.
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "reporting_rehomes")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        /// Khách nhận của lượt tách / gộp.
        pub tenant_id: Uuid,
        pub relocation_id: Uuid,
        pub from_root: Uuid,
        pub to_root: Uuid,
        /// `running` / `done`.
        pub status: String,
        pub moved_facts: i64,
        pub started_at: DateTimeUtc,
        pub finished_at: Option<DateTimeUtc>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}
