//! Entity của các bảng m0019: hàng số.

/// Một file của biến thể hàng số. Nội dung là bản chép của file người bán tải lên qua `files`,
/// nằm ở `storage_key` trong nơi lưu của khách.
pub(crate) mod digital_asset {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_digital_assets")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub product_id: Uuid,
        pub variant_id: Uuid,
        pub file_name: String,
        pub content_type: String,
        pub size: i64,
        pub sha256: String,
        pub storage_key: String,
        /// Số lượt tải của mỗi quyền phát cho dòng đơn.
        pub download_limit: i64,
        /// Quyền tải còn hiệu lực bao nhiêu ngày kể từ lúc phát.
        pub grant_days: i64,
        pub created_by: Option<Uuid>,
        pub created_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

/// Quyền tải một file cho một dòng đơn đã trả tiền.
pub(crate) mod download_grant {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_download_grants")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub order_id: Uuid,
        pub order_line_id: Uuid,
        /// Người mua có tài khoản; `None` với khách vãng lai hay khi tài khoản đã xoá.
        pub buyer_id: Option<Uuid>,
        pub org_id: Uuid,
        pub asset_id: Uuid,
        pub variant_id: Uuid,
        /// SHA-256 (hex) của token trong link tải; token dạng đọc được không lưu.
        pub token_hash: String,
        pub remaining: i64,
        pub expires_at: Timestamp,
        pub revoked_at: Option<Timestamp>,
        pub created_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}
