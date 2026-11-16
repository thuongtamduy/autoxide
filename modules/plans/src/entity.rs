//! Entity của các bảng `plans_*` (mặt điều khiển, D-253).

pub(crate) mod plan {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "plans_plan")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub code: String,
        pub name: String,
        pub description: String,
        /// `base` hay `addon`.
        pub kind: String,
        /// `draft`, `active` hay `retired`.
        pub status: String,
        pub is_default: bool,
        pub trial_days: i32,
        pub grace_days: i32,
        pub suspend_days: i32,
        pub retention_days: i32,
        pub renewal_lead_days: i32,
        /// `shared` hay `dedicated` (vị trí dữ liệu khi cấp phát).
        pub placement: String,
        pub requires_kyc: bool,
        pub sample_data: bool,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod plan_module {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "plans_plan_module")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub plan_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub module: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod plan_feature {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "plans_plan_feature")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub plan_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub feature: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod plan_limit {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "plans_plan_limit")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub plan_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub limit_key: String,
        /// `None` = không giới hạn.
        pub max_value: Option<i64>,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod price {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "plans_price")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub plan_id: Uuid,
        /// `recurring` hay `metered`.
        pub kind: String,
        /// `month` hay `year`.
        pub interval: String,
        pub interval_count: i32,
        pub amount_minor: i64,
        pub amount_currency: String,
        pub limit_key: Option<String>,
        pub included: i64,
        pub tax_category: Option<String>,
        pub active: bool,
        pub created_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod subscription {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "plans_subscription")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub plan_id: Uuid,
        pub price_id: Option<Uuid>,
        /// `trialing`, `active`, `past_due`, `suspended`, `canceled`, `deleted`.
        pub status: String,
        pub current_period_start: DateTimeUtc,
        pub current_period_end: DateTimeUtc,
        pub trial_ends_at: Option<DateTimeUtc>,
        pub grace_ends_at: Option<DateTimeUtc>,
        pub suspended_at: Option<DateTimeUtc>,
        pub canceled_at: Option<DateTimeUtc>,
        pub delete_after: Option<DateTimeUtc>,
        pub cancel_at_period_end: bool,
        pub scheduled_plan_id: Option<Uuid>,
        pub scheduled_price_id: Option<Uuid>,
        pub trial_used: bool,
        pub billing_email: Option<String>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod addon {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "plans_subscription_addon")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub plan_id: Uuid,
        pub quantity: i32,
        pub created_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod grant {
    //! Bảng `plans_override`: ghi đè riêng một khách (`override` là từ khoá của Rust).
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "plans_override")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        /// `module`, `feature` hay `limit`.
        pub kind: String,
        pub key: String,
        pub enabled: Option<bool>,
        pub max_value: Option<i64>,
        pub reason: String,
        pub expires_at: Option<DateTimeUtc>,
        pub created_by: Option<Uuid>,
        pub created_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod settings {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "plans_settings")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i32,
        pub updated_at: Option<DateTimeUtc>,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod usage {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "plans_usage")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub limit_key: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub period_start: String,
        pub used: i64,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod reservation {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "plans_reservation")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub limit_key: String,
        pub reference: String,
        pub amount: i64,
        /// `held`, `confirmed` hay `released`.
        pub status: String,
        pub expires_at: DateTimeUtc,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod usage_record {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "plans_usage_record")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub limit_key: String,
        pub period_start: String,
        pub quantity: i64,
        pub reference: String,
        pub recorded_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod billing_period {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "plans_billing_period")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub subscription_id: Uuid,
        pub kind: String,
        pub plan_id: Uuid,
        pub price_id: Option<Uuid>,
        pub period_start: DateTimeUtc,
        pub period_end: DateTimeUtc,
        pub amount_minor: i64,
        pub amount_currency: String,
        pub reference: String,
        pub invoice_id: Option<Uuid>,
        pub invoice_number: Option<String>,
        pub status: String,
        pub due_at: DateTimeUtc,
        pub paid_at: Option<DateTimeUtc>,
        pub provider_reference: Option<String>,
        pub reminders: String,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
}
