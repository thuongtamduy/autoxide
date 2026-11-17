//! Bảng của module `ledger` — mọi bảng là cấp khách hàng (`tenant_id`).
//!
//! `ledger_entries` và `ledger_lines` **chỉ thêm**: module không bao giờ `UPDATE` hay `DELETE` hai
//! bảng này (sửa sai bằng bút toán đảo); chuỗi băm (`ledger_entry_seals`) phát hiện ai sửa ngoài
//! module.

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

pub(crate) mod book {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "ledger_books")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub org_id: Uuid,
        pub base_currency: String,
        pub fiscal_year_start_month: i32,
        /// Ngày cuối của kỳ khoá gần nhất: không bút toán nào được mang ngày ≤ ngày này.
        pub closed_through: Option<Date>,
        pub created_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod account {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "ledger_accounts")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub book_id: Uuid,
        /// Nút của sổ (chép từ sổ, để lọc theo phạm vi).
        pub org_id: Uuid,
        pub purpose: String,
        /// Chuỗi rỗng khi không có đối tượng: unique chạy như nhau trên hai database.
        pub counterparty_kind: String,
        pub counterparty_id: String,
        pub currency: String,
        /// `debit` hay `credit`.
        pub normal_side: String,
        pub floor_minor: Option<i64>,
        pub external_code: Option<String>,
        pub name: Option<String>,
        /// `active` hay `closed`.
        pub status: String,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod balance {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "ledger_account_balances")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub account_id: Uuid,
        pub tenant_id: Uuid,
        /// Theo bên thường của tài khoản.
        pub balance_minor: i64,
        pub currency: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod entry {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "ledger_entries")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub book_id: Uuid,
        pub org_id: Uuid,
        pub period_id: Uuid,
        pub effective_date: Date,
        pub posted_at: DateTimeUtc,
        pub description: String,
        pub source_module: String,
        pub source_kind: String,
        pub source_id: Option<String>,
        pub reference: String,
        pub reference_fingerprint: String,
        pub reverses_entry_id: Option<Uuid>,
        pub actor_user_id: Option<Uuid>,
        /// `user`, `api_key` hay `system`.
        pub actor_kind: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod line {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "ledger_lines")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub entry_id: Uuid,
        pub line_no: i32,
        pub account_id: Uuid,
        pub side: String,
        pub amount_minor: i64,
        pub amount_currency: String,
        pub base_minor: i64,
        pub base_currency: String,
        pub exchange_rate: Option<String>,
        pub effective_date: Date,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod period {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "ledger_periods")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub book_id: Uuid,
        pub starts_on: Date,
        pub ends_on: Date,
        /// `open` hay `closed`.
        pub status: String,
        pub closed_at: Option<DateTimeUtc>,
        pub closed_by: Option<Uuid>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod period_balance {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "ledger_period_balances")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub period_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub account_id: Uuid,
        pub tenant_id: Uuid,
        /// Số dư cuối kỳ theo bên thường.
        pub closing_minor: i64,
        pub currency: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod head {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "ledger_chain_heads")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub book_id: Uuid,
        pub last_sequence: i64,
        pub last_hash: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod seal {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "ledger_entry_seals")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub book_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub sequence: i64,
        pub entry_id: Uuid,
        pub entry_hash: String,
        pub chain_hash: String,
        pub sealed_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}
