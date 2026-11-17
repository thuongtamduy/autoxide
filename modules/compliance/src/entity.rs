//! Bảng của module `compliance`. `compliance_rules` là bảng mặt điều khiển (luật dùng chung mọi khách);
//! mọi bảng khác là cấp khách hàng (`tenant_id`).

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
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "compliance_rules")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub jurisdiction: String,
        pub key: String,
        pub version: i64,
        pub effective_from: DateTimeUtc,
        pub effective_to: Option<DateTimeUtc>,
        /// JSON.
        pub body: String,
        pub legal_basis: String,
        pub needs_legal_review: bool,
        /// `bundled` hay `import`.
        pub source: String,
        pub imported_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod policy {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "compliance_policies")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub key: String,
        /// JSON.
        pub value: String,
        pub updated_by: Option<Uuid>,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod kyc_profile {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "compliance_kyc_profiles")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        /// `user`, `org`, `email` hay `reference`.
        pub subject_kind: String,
        pub subject_id: String,
        pub party_type: String,
        pub status: String,
        pub method: Option<String>,
        pub provider: Option<String>,
        pub provider_reference: Option<String>,
        pub verified_name: Option<String>,
        pub document_kind: Option<String>,
        /// SHA-256 của số giấy tờ đã chuẩn hoá.
        pub document_hash: Option<String>,
        pub document_last4: Option<String>,
        pub assurance: Option<String>,
        /// JSON: ID file giấy tờ (module `files`) cho cách duyệt tay.
        pub document_files: String,
        /// JSON: trường cần bổ sung và lời nhắn (`needs_info`).
        pub info_request: Option<String>,
        pub verified_at: Option<DateTimeUtc>,
        pub expires_at: Option<DateTimeUtc>,
        pub reason_code: Option<String>,
        pub reviewed_by: Option<Uuid>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod kyc_attempt {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "compliance_kyc_attempts")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub profile_id: Uuid,
        pub method: String,
        pub provider: String,
        /// Mã ở nhà cung cấp; chưa có thì là ID lượt thử.
        pub provider_reference: String,
        /// `pending`, `verified`, `rejected`, `failed`.
        pub status: String,
        pub result_code: Option<String>,
        pub created_at: DateTimeUtc,
        pub completed_at: Option<DateTimeUtc>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod kyc_callback {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "compliance_kyc_callbacks")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub provider: String,
        pub provider_reference: String,
        /// Thân callback nguyên văn (đã kiểm chữ ký).
        pub body: String,
        pub received_at: DateTimeUtc,
        pub processed_at: Option<DateTimeUtc>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod document {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "compliance_documents")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        /// ID nút, hay `''` cho văn bản cấp khách.
        pub org_key: String,
        pub kind: String,
        /// `user`, `buyer`, `seller`.
        pub audience: String,
        pub title: String,
        pub current_version: Option<i32>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod version {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "compliance_document_versions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub document_id: Uuid,
        pub version: i32,
        pub locale: String,
        /// Nguồn soạn (vd `content.entry:<id>`), hay `''` khi gửi thẳng nội dung.
        pub source: String,
        pub title: String,
        pub body: String,
        pub sha256: String,
        pub effective_from: DateTimeUtc,
        pub requires_reacceptance: bool,
        pub published_by: Option<Uuid>,
        pub published_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod acceptance {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "compliance_acceptances")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub document_id: Uuid,
        pub version: i32,
        pub version_id: Uuid,
        pub subject_kind: String,
        pub subject_id: String,
        pub accepted_by: Option<Uuid>,
        /// `web`, `mobile_app`, `api`, `mcp`, `in_store`.
        pub channel: String,
        /// `checkbox`, `click_wrap`, `signature`.
        pub method: String,
        pub ip_fingerprint: Option<String>,
        pub user_agent: Option<String>,
        /// SHA-256 của bản đã chấp nhận.
        pub sha256: String,
        pub accepted_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod consent {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "compliance_consents")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        /// `user`, `email`, `phone`.
        pub subject_kind: String,
        /// ID người dùng, hay SHA-256 của email / số đã chuẩn hoá.
        pub subject_key: String,
        /// Email / số; `None` cho người dùng hay sau khi xoá theo yêu cầu.
        pub subject_value: Option<String>,
        pub user_id: Option<Uuid>,
        pub purpose: String,
        pub scope: String,
        pub channel: String,
        /// `granted` hay `withdrawn`.
        pub state: String,
        pub granted_at: Option<DateTimeUtc>,
        pub withdrawn_at: Option<DateTimeUtc>,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod consent_record {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "compliance_consent_records")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub consent_id: Uuid,
        /// `grant` hay `withdraw`.
        pub action: String,
        /// JSON bằng chứng.
        pub evidence: String,
        pub source: String,
        pub source_ref: String,
        pub recorded_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod account_verification {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "compliance_account_verifications")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub user_id: Uuid,
        /// E.164; `None` khi chưa có hay đã gỡ.
        pub phone: Option<String>,
        /// SHA-256 của số; `''` khi chưa có.
        pub phone_key: String,
        pub phone_verified_at: Option<DateTimeUtc>,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod phone_challenge {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "compliance_phone_challenges")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub user_id: Uuid,
        /// E.164.
        pub phone: String,
        /// SHA-256 của (pepper của khách, ID, mã) — mã không bao giờ lưu nguyên văn.
        pub code_hash: String,
        pub attempts: i32,
        pub expires_at: DateTimeUtc,
        pub consumed_at: Option<DateTimeUtc>,
        pub created_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}
