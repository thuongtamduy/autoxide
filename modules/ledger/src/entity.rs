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

pub(crate) mod payment {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "ledger_payments")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        /// Nút nhận tiền (sổ ghi bút toán).
        pub org_id: Uuid,
        pub book_id: Uuid,
        pub provider: String,
        pub provider_ref: Option<String>,
        pub method: String,
        pub description: String,
        pub amount_minor: i64,
        pub amount_currency: String,
        /// Đã hoàn hay đang hoàn (giữ chỗ khi tạo lượt hoàn): không vượt `amount_minor`.
        pub refunded_minor: i64,
        /// `pending`, `succeeded`, `failed`, `expired`, `needs_review`.
        pub status: String,
        pub checkout_url: Option<String>,
        pub qr_payload: Option<String>,
        pub return_url: Option<String>,
        pub expires_at: DateTimeUtc,
        pub captured_at: Option<DateTimeUtc>,
        pub source_module: String,
        pub source_id: Option<String>,
        pub reference: String,
        pub reference_fingerprint: String,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod refund {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "ledger_refunds")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub payment_id: Uuid,
        pub amount_minor: i64,
        pub amount_currency: String,
        /// `pending`, `succeeded`, `failed`.
        pub status: String,
        pub provider_ref: Option<String>,
        pub reason: String,
        pub reference: String,
        pub reference_fingerprint: String,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod gateway_event {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "ledger_gateway_events")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub provider: String,
        pub provider_event_id: String,
        pub kind: String,
        /// Thân webhook nguyên văn.
        pub body: String,
        /// Thông báo đã kiểm chữ ký, dạng JSON (để job xử lý mà không kiểm lại).
        pub notification: String,
        /// `pending`, `processed`, `ignored`, `failed`.
        pub status: String,
        pub error: Option<String>,
        pub received_at: DateTimeUtc,
        pub processed_at: Option<DateTimeUtc>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod statement_line {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "ledger_statement_lines")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub provider: String,
        pub provider_line_id: String,
        pub provider_ref: String,
        pub kind: String,
        pub amount_minor: i64,
        pub amount_currency: String,
        pub fee_minor: i64,
        pub booked_on: Date,
        /// `matched`, `unmatched`, `exception`.
        pub match_status: String,
        pub payment_id: Option<Uuid>,
        pub reconciliation_id: Uuid,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod exception {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "ledger_exceptions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        /// `amount_mismatch`, `unmatched_credit`, `duplicate_payment`, `missing_in_statement`,
        /// `unknown_in_statement`, `late_payment`.
        pub kind: String,
        /// Bản ghi liên quan, vd `ledger.payment:<id>`.
        pub subject: String,
        /// JSON.
        pub details: String,
        /// Mở một lần cho mỗi vấn đề: chạy lại đối soát không mở trùng.
        pub dedupe_key: String,
        /// `open`, `resolved`, `ignored`.
        pub status: String,
        /// `match`, `ignore`, `refunded_outside`.
        pub resolution: Option<String>,
        pub note: Option<String>,
        pub resolved_by: Option<Uuid>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod reconciliation {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "ledger_reconciliations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub provider: String,
        pub on_date: Date,
        /// `completed` hay `failed`.
        pub status: String,
        pub lines: i64,
        pub matched: i64,
        pub exceptions: i64,
        pub error: Option<String>,
        pub created_by: Option<Uuid>,
        pub created_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}
