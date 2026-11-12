//! Bảng `content_embeddings`: vector của từng đoạn của mục, cho tìm theo ý nghĩa (D-109).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_embeddings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub entry_id: Uuid,
    /// `draft` (bản nháp, cho người biên tập) hoặc `published` (bản công khai, cho người đọc).
    pub scope: String,
    pub locale: String,
    pub chunk_index: i32,
    /// SHA-256 (hex) của chữ của đoạn: không đổi thì không tính lại embedding.
    pub content_hash: String,
    pub model: String,
    pub dimensions: i32,
    /// `f32` little-endian nối nhau (`autoxide_kernel::ai::vector::encode`), đã chuẩn hoá.
    pub vector: Vec<u8>,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}
