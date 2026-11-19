//! Entity (SeaORM) của các bảng `promotions_…`. Chỉ dùng trong module này. Bảng có `tenant_id`
//! cài `TenantScoped` (lọc bằng `.scoped(tenant)`); ba bảng dữ liệu luật là mặt điều khiển.

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

pub(crate) mod promotion {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "promotions_promotions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub org_id: Uuid,
        pub kind: String,
        pub scope: String,
        pub trigger: String,
        pub status: String,
        pub currency: String,
        pub value_bp: Option<i32>,
        pub value_minor: Option<i64>,
        pub min_order_minor: Option<i64>,
        pub min_quantity: Option<i32>,
        pub max_discount_minor: Option<i64>,
        pub buy_quantity: Option<i32>,
        pub get_quantity: Option<i32>,
        pub reward_selection: Option<String>,
        pub reward_value_bp: Option<i32>,
        pub max_applications: Option<i32>,
        pub bundle_quantity: Option<i32>,
        pub bundle_price_minor: Option<i64>,
        pub max_add_on_quantity: Option<i32>,
        pub gift_quantity: Option<i32>,
        pub first_order_only: bool,
        pub starts_at: DateTimeUtc,
        pub ends_at: DateTimeUtc,
        pub priority: i32,
        pub exclusivity: String,
        pub combines_with: String,
        pub stop_after: bool,
        pub usage_limit: Option<i64>,
        pub usage_count: i64,
        pub per_customer_limit: Option<i64>,
        pub per_customer_period: String,
        pub daily_usage_limit: Option<i64>,
        pub budget_minor: Option<i64>,
        pub budget_used_minor: i64,
        pub return_usage_on_cancel: bool,
        pub gift_return_policy: String,
        pub program_kind: String,
        pub voucher_quota: Option<i64>,
        pub issued_count: i64,
        pub per_user_claim_limit: Option<i32>,
        pub voucher_valid_days: Option<i32>,
        pub claimable: bool,
        pub claim_requires_verified_phone: bool,
        pub public: bool,
        pub version: i32,
        pub created_by: Option<Uuid>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
        pub activated_at: Option<DateTimeUtc>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod translation {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "promotions_translations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub promotion_id: Uuid,
        pub locale: String,
        pub title: String,
        pub terms: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod target {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "promotions_targets")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub promotion_id: Uuid,
        pub target_type: String,
        pub target_id: Uuid,
        pub role: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod customer_group {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "promotions_customer_groups")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub promotion_id: Uuid,
        pub group_id: Uuid,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod channel {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "promotions_channels")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub promotion_id: Uuid,
        pub channel: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod condition {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "promotions_conditions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub promotion_id: Uuid,
        pub kind: String,
        pub operator: String,
        pub values: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod schedule {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "promotions_schedules")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub promotion_id: Uuid,
        pub weekdays: i32,
        pub start_minute: i32,
        pub end_minute: i32,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod tier {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "promotions_tiers")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub promotion_id: Uuid,
        pub threshold_kind: String,
        pub threshold: i64,
        pub value_bp: Option<i32>,
        pub value_minor: Option<i64>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod gift_option {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "promotions_gift_options")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub promotion_id: Uuid,
        pub variant_id: Uuid,
        pub stock_limit: Option<i64>,
        pub quantity_used: i64,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod code {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "promotions_codes")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub promotion_id: Uuid,
        pub code: String,
        pub code_key: String,
        pub purpose: String,
        pub usage_limit: Option<i64>,
        pub usage_count: i64,
        pub created_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod voucher {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "promotions_vouchers")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub promotion_id: Uuid,
        pub holder_user_id: Option<Uuid>,
        pub status: String,
        pub source: String,
        pub source_ref: String,
        pub valid_from: DateTimeUtc,
        pub expires_at: DateTimeUtc,
        pub issued_at: DateTimeUtc,
        pub used_order_id: Option<Uuid>,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod flash_item {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "promotions_flash_items")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub promotion_id: Uuid,
        pub variant_id: Uuid,
        pub sale_price_minor: Option<i64>,
        pub value_bp: Option<i32>,
        pub quantity_limit: Option<i64>,
        pub quantity_used: i64,
        pub per_customer_quantity_limit: Option<i64>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod funding {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "promotions_funding")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub promotion_id: Uuid,
        pub payer_kind: String,
        pub payer_org_id: Option<Uuid>,
        pub share_bp: i32,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod participation {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "promotions_participations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub promotion_id: Uuid,
        pub seller_org_id: Uuid,
        pub status: String,
        /// JSON: danh sách `ParticipationItem`.
        pub items: String,
        pub reviewed_by: Option<Uuid>,
        pub reviewed_at: Option<DateTimeUtc>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod customer_usage {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "promotions_customer_usage")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub promotion_id: Uuid,
        pub subject: String,
        pub buyer_key: String,
        pub period_key: String,
        pub used: i64,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod daily_usage {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "promotions_daily_usage")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub promotion_id: Uuid,
        pub day: String,
        pub used: i64,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod redemption {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "promotions_redemptions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub order_id: Uuid,
        pub org_id: Uuid,
        pub buyer_id: Option<Uuid>,
        pub status: String,
        pub fingerprint: String,
        pub request_fingerprint: String,
        pub evaluation: String,
        pub total_discount_minor: i64,
        pub order_value_minor: i64,
        pub currency: String,
        pub day: String,
        pub expires_at: Option<DateTimeUtc>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod hold {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "promotions_redemption_holds")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub redemption_id: Uuid,
        pub counter: String,
        pub counter_id: String,
        pub quantity: i64,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod stats {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "promotions_stats_daily")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub promotion_id: Uuid,
        pub day: String,
        pub redemptions: i64,
        pub discount_minor: i64,
        pub order_value_minor: i64,
        pub platform_funded_minor: i64,
        pub seller_funded_minor: i64,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod version {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "promotions_versions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub promotion_id: Uuid,
        pub version: i32,
        pub action: String,
        pub snapshot: String,
        pub changed_by: Option<Uuid>,
        pub changed_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod notice {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "promotions_notices")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub promotion_id: Uuid,
        pub authority_code: String,
        pub status: String,
        pub reference: Option<String>,
        pub submitted_at: Option<DateTimeUtc>,
        pub submitted_by: Option<Uuid>,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod reminder {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "promotions_reminders")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub promotion_id: Uuid,
        pub user_id: Uuid,
        pub channel: String,
        pub consent_id: Option<Uuid>,
        pub remind_at: DateTimeUtc,
        pub sent_at: Option<DateTimeUtc>,
        pub created_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod legal_pack {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "promotions_legal_packs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub code: String,
        pub version: i64,
        pub checked_on: String,
        pub source_note: String,
        pub content_sha256: String,
        pub imported_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod legal_limit {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "promotions_legal_limits")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub jurisdiction: String,
        pub promotion_kind: String,
        pub program_kind: String,
        pub max_bp: i32,
        pub max_days_per_program: Option<i32>,
        pub max_days_per_year: Option<i32>,
        pub effective_from: String,
        pub effective_to: Option<String>,
        pub legal_basis: String,
        pub source_url: String,
        pub note: String,
        pub pack_version: i64,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod notice_rule {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "promotions_notice_rules")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub jurisdiction: String,
        pub promotion_kind: String,
        pub notice_required: bool,
        pub days_before: i32,
        pub authority: String,
        pub effective_from: String,
        pub effective_to: Option<String>,
        pub legal_basis: String,
        pub note: String,
        pub pack_version: i64,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod revision {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "promotions_revisions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        pub revision: i64,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}
