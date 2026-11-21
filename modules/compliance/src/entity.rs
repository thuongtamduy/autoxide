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
    use autoxide_kernel::time::Timestamp;
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
        pub effective_from: Timestamp,
        pub effective_to: Option<Timestamp>,
        /// JSON.
        pub body: String,
        pub legal_basis: String,
        pub needs_legal_review: bool,
        /// `bundled` hay `import`.
        pub source: String,
        pub imported_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod policy {
    use autoxide_kernel::time::Timestamp;
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
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod kyc_profile {
    use autoxide_kernel::time::Timestamp;
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
        pub verified_at: Option<Timestamp>,
        pub expires_at: Option<Timestamp>,
        pub reason_code: Option<String>,
        pub reviewed_by: Option<Uuid>,
        pub created_at: Timestamp,
        pub updated_at: Timestamp,
        /// Ghi chú của người duyệt khi từ chối / thu hồi (cho người được duyệt đọc).
        pub review_note: Option<String>,
        /// `document_hash` khi nhà cung cấp xác thực hồ sơ, trống ở trạng thái khác; unique theo
        /// khách — một giấy tờ chỉ xác thực tự động cho một hồ sơ (m0020).
        pub verified_document_hash: Option<String>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod kyc_attempt {
    use autoxide_kernel::time::Timestamp;
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
        pub created_at: Timestamp,
        pub completed_at: Option<Timestamp>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod kyc_callback {
    use autoxide_kernel::time::Timestamp;
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
        pub received_at: Timestamp,
        pub processed_at: Option<Timestamp>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod document {
    use autoxide_kernel::time::Timestamp;
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
        pub created_at: Timestamp,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod version {
    use autoxide_kernel::time::Timestamp;
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
        pub effective_from: Timestamp,
        pub requires_reacceptance: bool,
        pub published_by: Option<Uuid>,
        pub published_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod acceptance {
    use autoxide_kernel::time::Timestamp;
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
        pub accepted_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod consent {
    use autoxide_kernel::time::Timestamp;
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
        pub granted_at: Option<Timestamp>,
        pub withdrawn_at: Option<Timestamp>,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod consent_record {
    use autoxide_kernel::time::Timestamp;
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
        pub recorded_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod account_verification {
    use autoxide_kernel::time::Timestamp;
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
        pub phone_verified_at: Option<Timestamp>,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod phone_challenge {
    use autoxide_kernel::time::Timestamp;
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
        pub expires_at: Timestamp,
        pub consumed_at: Option<Timestamp>,
        pub created_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod legal_profile {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "compliance_legal_profiles")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        /// Nút sở hữu; nút gốc = cả khách.
        pub org_id: Uuid,
        /// `individual`, `household_business`, `organization`.
        pub entity_type: String,
        pub legal_name: String,
        pub trade_name: Option<String>,
        pub tax_code: Option<String>,
        pub registration_number: Option<String>,
        /// `YYYY-MM-DD`.
        pub registration_issued_on: Option<String>,
        pub registration_issued_by: Option<String>,
        /// JSON `Address` của module `address`.
        pub address: Option<String>,
        pub representative_name: Option<String>,
        pub representative_title: Option<String>,
        pub phone: Option<String>,
        pub email: Option<String>,
        pub kyc_profile_id: Option<Uuid>,
        pub created_at: Timestamp,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod website {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "compliance_websites")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        /// Nút của website; nút gốc = website của cả khách.
        pub org_id: Uuid,
        /// Chữ thường, tên quốc tế dạng punycode.
        pub domain: String,
        /// `website` hay `mobile_app`.
        pub channel: String,
        /// `sales` (thông báo, logo xanh) hay `platform` (đăng ký, logo đỏ).
        pub website_kind: String,
        /// `not_submitted`, `submitted`, `needs_supplement`, `notified`, `registered`, `rejected`,
        /// `revoked`.
        pub moit_status: String,
        pub moit_reference: Option<String>,
        pub submitted_at: Option<Timestamp>,
        pub decided_at: Option<Timestamp>,
        pub logo_link_url: Option<String>,
        pub logo_image_url: Option<String>,
        /// Phiên bản danh sách kiểm lúc nộp.
        pub checklist_version: Option<i32>,
        pub created_at: Timestamp,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod website_page {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "compliance_website_pages")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub website_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub page_kind: String,
        /// `content.entry:<id>`.
        pub resource: String,
        pub linked_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

// ---- Dữ liệu cá nhân (D-360) -------------------------------------------------------------------

pub(crate) mod purpose {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "compliance_purposes")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub key: String,
        /// `service`, `marketing`, `analytics`, `legal`, `kyc`.
        pub category: String,
        /// `consent`, `contract`, `legal_obligation`, `vital_interest`, `public_interest`.
        pub legal_basis: String,
        /// JSON: mảng tên loại dữ liệu (vd `["email", "name"]`).
        pub data_categories: String,
        /// JSON: mảng bên nhận (`{"name", "country"}`).
        pub recipients: String,
        /// JSON: `{"en": …, "vi": …}`.
        pub description: String,
        /// Thời hạn lưu viết bằng chữ (vd "until the account is deleted").
        pub retention: Option<String>,
        pub active: bool,
        pub updated_by: Option<Uuid>,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod incident {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "compliance_incidents")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub title: String,
        pub description: String,
        /// `low`, `medium`, `high`, `critical`.
        pub severity: String,
        /// `open`, `contained`, `reported`, `closed`.
        pub status: String,
        pub detected_at: Timestamp,
        /// `detected_at` + giờ của quy tắc `privacy.incident`.
        pub report_due_at: Timestamp,
        /// Mốc nhắc (giờ sau khi phát hiện) đã gửi gần nhất; 0 khi chưa nhắc.
        pub reminded_hours: i32,
        pub authority_notified_at: Option<Timestamp>,
        pub authority_reference: Option<String>,
        pub subjects_notified_at: Option<Timestamp>,
        /// JSON: mảng loại dữ liệu bị ảnh hưởng.
        pub data_categories: String,
        pub affected_count: Option<i64>,
        pub created_by: Option<Uuid>,
        pub created_at: Timestamp,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod incident_entry {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "compliance_incident_entries")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub incident_id: Uuid,
        pub body: String,
        pub author_user_id: Option<Uuid>,
        pub created_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod subject_request {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "compliance_subject_requests")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        /// `access`, `erasure`, `rectification`, `objection`, `restriction`, `withdrawal`.
        pub kind: String,
        /// `email`, `post`, `phone`, `authority`, `other`.
        pub channel: String,
        /// `email`, `phone` hay `user`.
        pub subject_kind: String,
        /// Email / số đã chuẩn hoá / ID người dùng.
        pub subject: String,
        pub received_at: Timestamp,
        pub due_at: Timestamp,
        /// `received`, `in_progress`, `fulfilled`, `rejected`.
        pub status: String,
        pub reason: Option<String>,
        pub notes: Option<String>,
        pub handled_by: Option<Uuid>,
        pub closed_at: Option<Timestamp>,
        pub created_by: Option<Uuid>,
        pub created_at: Timestamp,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

// ---- Mã theo dõi quảng cáo và đồng ý cookie (D-235, D-361) --------------------------------------

pub(crate) mod tracking_tag {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "compliance_tracking_tags")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        /// Nút của website (gốc = cả khách).
        pub org_id: Uuid,
        /// Khoá nhà cung cấp cho sẵn (`ga4`, `gtm`, `google_ads`, `meta`, `tiktok`, `zalo`).
        pub provider: String,
        pub tracking_id: String,
        /// `active` hay `paused`.
        pub status: String,
        pub created_by: Option<Uuid>,
        pub created_at: Timestamp,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod cookie_setting {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "compliance_cookie_settings")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub org_id: Uuid,
        pub enabled: bool,
        /// JSON: nhóm tuỳ chọn khách bật (`preferences`, `analytics`, `marketing`).
        pub categories: String,
        pub policy_version: i32,
        /// SHA-256 của (nhóm hiệu lực, nhà cung cấp đang bật): đổi thì tăng `policy_version`.
        pub signature: String,
        /// `bottom`, `top` hay `center`.
        pub position: String,
        pub updated_by: Option<Uuid>,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod cookie_consent {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "compliance_cookie_consents")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub org_id: Uuid,
        /// Mã ngẫu nhiên do giao diện tạo, giữ trong cookie cần thiết.
        pub consent_id: String,
        /// JSON: nhóm người xem đã chọn.
        pub categories: String,
        pub policy_version: i32,
        /// Băm khối IP (`ip_hash::fingerprint`), không giữ IP thô.
        pub ip_hash: Option<String>,
        pub user_id: Option<Uuid>,
        pub created_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

// ---- Vụ việc: báo vi phạm, khiếu nại, cung cấp thông tin cho cơ quan (D-363) --------------------

pub(crate) mod case {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "compliance_cases")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        /// `ip_infringement`, `counterfeit`, `prohibited_goods`, `consumer_complaint`, `other`.
        pub kind: String,
        /// Đích `<module>.<loại>:<id>` hay `''`.
        pub target: String,
        /// Nút bị phản ánh (người bán trên sàn); `None`: không nêu.
        pub respondent_org_id: Option<Uuid>,
        /// `guest`, `user`, `rights_holder`, `authority`.
        pub reporter_kind: String,
        pub reporter_user_id: Option<Uuid>,
        pub reporter_name: Option<String>,
        pub reporter_email: Option<String>,
        pub reporter_phone: Option<String>,
        /// Kênh tiếp nhận: `public`, `staff`.
        pub channel: String,
        pub claim: String,
        /// JSON: văn bằng, link chứng cứ.
        pub rights: String,
        /// `received`, `under_review`, `awaiting_response`, `decided`, `closed`, `withdrawn`.
        pub status: String,
        pub restricted: bool,
        /// `upheld`, `rejected`, `partially_upheld` hay `''`.
        pub decision: String,
        pub decision_reason: Option<String>,
        pub ack_due_at: Timestamp,
        pub acknowledged_at: Option<Timestamp>,
        pub respond_by: Timestamp,
        pub decide_by: Timestamp,
        pub decided_at: Option<Timestamp>,
        pub decided_by: Option<Uuid>,
        /// SHA-256 của mã tra cứu trả cho người báo (chỉ lưu băm).
        pub tracking_hash: String,
        /// Lớp chống spam giữ lại (lý do); `None`: bình thường.
        pub spam_reason: Option<String>,
        /// Mốc quá hạn đã nhắc: `''`, `ack`, `decide`.
        pub overdue_notified: String,
        pub created_by: Option<Uuid>,
        pub created_at: Timestamp,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod case_respondent {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "compliance_case_respondents")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub case_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub user_id: Uuid,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod case_entry {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "compliance_case_entries")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub case_id: Uuid,
        /// `note`, `status`, `response`, `evidence`, `restriction`, `decision`, `disclosure`.
        pub kind: String,
        /// `staff`, `reporter`, `respondent`, `system`.
        pub author_kind: String,
        pub author_user_id: Option<Uuid>,
        pub body: String,
        /// JSON: ID file (module `files`).
        pub files: String,
        pub created_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod disclosure {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "compliance_disclosures")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub case_id: Option<Uuid>,
        pub authority: String,
        pub request_reference: String,
        pub legal_basis: String,
        pub requested_at: Timestamp,
        /// `org:<id>` hay `user:<id>`.
        pub subject: String,
        /// JSON: TÊN trường đã cung cấp, không giá trị.
        pub fields: String,
        pub disclosed_by: Uuid,
        pub disclosed_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}
