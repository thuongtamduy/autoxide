//! Bảng của module `notifications`, đều cấp khách hàng (`tenant_id`).

macro_rules! tenant_scoped {
    () => {
        impl autoxide_kernel::tenancy::TenantScoped for Entity {
            fn tenant_column() -> Self::Column {
                Column::TenantId
            }
        }
    };
}

/// Nhật ký gửi: mỗi (thông báo, kênh) một dòng.
pub(crate) mod message {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "notifications_messages")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub kind: String,
        /// `security`, `transactional` hay `marketing`.
        pub category: String,
        pub channel: String,
        pub user_id: Option<Uuid>,
        /// Email / số E.164 (xoá sau hạn lưu); push và in-app: `None` (gửi theo `user_id`).
        pub address: Option<String>,
        /// Mục đích đồng ý (tin quảng cáo), rỗng với tin khác.
        pub purpose: String,
        pub locale: String,
        /// JSON (xoá thành `{}` sau hạn lưu).
        pub params: String,
        pub link: Option<String>,
        /// `queued`, `deferred`, `sent`, `failed`, `skipped` hay `canceled`.
        pub status: String,
        pub skip_reason: String,
        pub not_before: Option<DateTimeUtc>,
        pub attempts: i32,
        pub provider_reference: Option<String>,
        pub error_code: Option<String>,
        /// SHA-256 của phần thô token huỷ nhận (email quảng cáo).
        pub unsubscribe_token_hash: Option<String>,
        pub dedupe_key: String,
        pub created_at: DateTimeUtc,
        pub sent_at: Option<DateTimeUtc>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

/// Hộp thư trong ứng dụng.
pub(crate) mod inbox {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "notifications_inbox")]
    pub struct Model {
        /// Bằng ID của tin in-app: gửi lại không tạo dòng thứ hai.
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub user_id: Uuid,
        pub kind: String,
        pub category: String,
        pub title: String,
        pub body: String,
        pub link: Option<String>,
        pub read_at: Option<DateTimeUtc>,
        pub created_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

/// Tuỳ chọn nhận của người dùng; không có dòng là bật.
pub(crate) mod preference {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "notifications_preferences")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub user_id: Uuid,
        /// `kind:<khoá>` hay `category:<nhóm>`.
        #[sea_orm(primary_key, auto_increment = false)]
        pub target: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub channel: String,
        pub enabled: bool,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

/// Câu chữ thay mẫu mặc định của khách.
pub(crate) mod template {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "notifications_templates")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub kind: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub channel: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub locale: String,
        pub title: String,
        pub body: String,
        pub short: String,
        /// Mã mẫu ZNS đã được Zalo duyệt (kênh `zns`).
        pub zns_template_id: Option<String>,
        pub updated_by: Option<Uuid>,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

/// Đăng ký web push của trình duyệt.
pub(crate) mod push_subscription {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "notifications_push_subscriptions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub user_id: Uuid,
        pub endpoint: String,
        /// SHA-256 của `endpoint`.
        pub endpoint_key: String,
        pub p256dh: String,
        /// Secret xác thực, mã hoá bằng `Sealer` của kernel (D-257).
        pub auth_secret: String,
        pub user_agent: Option<String>,
        pub failures: i32,
        pub created_at: DateTimeUtc,
        pub last_used_at: Option<DateTimeUtc>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

/// Token thiết bị di động (D-220).
pub(crate) mod device_token {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "notifications_device_tokens")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub user_id: Uuid,
        /// `ios` hay `android`.
        pub platform: String,
        /// Token, mã hoá bằng `Sealer` của kernel (D-257).
        pub token: String,
        /// SHA-256 của token.
        pub token_key: String,
        pub app_id: String,
        pub locale: Option<String>,
        pub failures: i32,
        pub created_at: DateTimeUtc,
        pub last_used_at: Option<DateTimeUtc>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

/// Tin mời đăng ký nhận quảng cáo đã gửi: mỗi (kênh, địa chỉ) một lần.
pub(crate) mod invitation {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "notifications_invitations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub channel: String,
        /// SHA-256 của địa chỉ (hay ID người dùng).
        #[sea_orm(primary_key, auto_increment = false)]
        pub subject_key: String,
        pub sent_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

/// Bản nháp mẫu khách tự soạn (D-206): mỗi (loại, kênh, ngôn ngữ) một bản.
pub(crate) mod template_draft {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "notifications_template_drafts")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub kind: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub channel: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub locale: String,
        pub subject: String,
        pub body_text: String,
        pub body_html: Option<String>,
        pub short: String,
        pub zns_template_id: Option<String>,
        pub updated_by: Option<Uuid>,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

/// Bản đã xuất bản của mẫu khách tự soạn: ghi một lần, không sửa (giữ 20 bản gần nhất).
pub(crate) mod template_version {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "notifications_template_versions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub kind: String,
        pub channel: String,
        pub locale: String,
        pub version: i32,
        pub subject: String,
        pub body_text: String,
        pub body_html: Option<String>,
        pub short: String,
        pub zns_template_id: Option<String>,
        pub published_by: Option<Uuid>,
        pub published_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

/// Bản đang dùng của mẫu khách tự soạn: số phiên bản trong `notifications_template_versions`.
pub(crate) mod template_live {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "notifications_template_live")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub kind: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub channel: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub locale: String,
        pub version: i32,
        pub updated_by: Option<Uuid>,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

/// Thương hiệu của mail HTML của khách (logo, màu, chân thư).
pub(crate) mod branding {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "notifications_branding")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        pub brand_name: Option<String>,
        pub logo_url: Option<String>,
        /// `#rrggbb`.
        pub primary_color: String,
        /// `#rrggbb`.
        pub background_color: String,
        pub footer_text: Option<String>,
        pub updated_by: Option<Uuid>,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}
