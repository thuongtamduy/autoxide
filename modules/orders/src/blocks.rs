//! Khối của `orders` cho trang dựng từ khối (bước 67, bước 45): nút thêm vào giỏ, nút mua ngay,
//! link giỏ hàng ở đầu trang (ba khối chỉ có props — giao diện gọi API giỏ hàng sẵn có) và đánh
//! giá của một sản phẩm (có dữ liệu: số sao và vài đánh giá mới nhất đã đăng).

use std::sync::Arc;

use autoxide_kernel::blocks::{BlockCategory, BlockData, BlockRef, BlockRequest, BlockType};
use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Order, PageRequest};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::http::reviews::{ReviewResponse, stats_response};
use crate::reviews::PublicFilter;
use crate::service::OrdersService;

/// Số đánh giá tối đa của khối.
const MAX_REVIEWS: u32 = 20;
const DEFAULT_REVIEWS: u32 = 3;

pub(crate) const ADD_TO_CART: BlockType = BlockType::new(
    "orders.add_to_cart",
    BlockCategory::Commerce,
    "Add to cart",
    "Use for a button that puts a product in the cart; pair it with a product block so buyers see the price.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "required": ["product_id"],
        "properties": {
            "product_id": {"type": "string", "format": "uuid"},
            "variant_id": {"type": "string", "format": "uuid", "description": "The variant to add; omitted: the buyer picks one."},
            "quantity": {"type": "integer", "minimum": 1, "maximum": 99},
            "label": {"type": "string", "maxLength": 60}
        }
    }"#,
);

pub(crate) const BUY_NOW: BlockType = BlockType::new(
    "orders.buy_now",
    BlockCategory::Commerce,
    "Buy now",
    "Use for a button that takes the buyer straight to checkout with one product.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "required": ["product_id"],
        "properties": {
            "product_id": {"type": "string", "format": "uuid"},
            "variant_id": {"type": "string", "format": "uuid", "description": "The variant to buy; omitted: the buyer picks one."},
            "label": {"type": "string", "maxLength": 60}
        }
    }"#,
);

pub(crate) const CART_LINK: BlockType = BlockType::new(
    "orders.cart_link",
    BlockCategory::Navigation,
    "Cart",
    "Use in the header for a link to the cart with the number of items.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "show_count": {"type": "boolean"},
            "label": {"type": "string", "maxLength": 60}
        }
    }"#,
);

pub(crate) const REVIEWS: BlockType = BlockType::new(
    "orders.reviews",
    BlockCategory::Social,
    "Reviews",
    "Use to show what verified buyers say about a product: star counts and the latest reviews.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "required": ["product_id"],
        "properties": {
            "product_id": {"type": "string", "format": "uuid"},
            "limit": {"type": "integer", "minimum": 1, "maximum": 20}
        }
    }"#,
)
.with_data();

pub(crate) fn all() -> Vec<BlockType> {
    vec![ADD_TO_CART, BUY_NOW, CART_LINK, REVIEWS]
}

/// Dữ liệu của khối `orders.reviews`.
pub(crate) struct ReviewBlocks(pub(crate) Arc<OrdersService>);

#[async_trait::async_trait]
impl BlockData for ReviewBlocks {
    async fn resolve(
        &self,
        request: &BlockRequest<'_>,
        _block: &str,
        items: &[BlockRef],
    ) -> Result<Vec<Option<Value>>, Error> {
        let mut answers = Vec::with_capacity(items.len());
        for item in items {
            let product = item
                .props
                .get("product_id")
                .and_then(Value::as_str)
                .and_then(|id| Uuid::parse_str(id).ok());
            let Some(product) = product else {
                answers.push(None);
                continue;
            };
            let limit = item
                .props
                .get("limit")
                .and_then(Value::as_u64)
                .and_then(|limit| u32::try_from(limit).ok())
                .unwrap_or(DEFAULT_REVIEWS)
                .clamp(1, MAX_REVIEWS);
            let filter = PublicFilter { rating: None, has_media: None, helpful_first: false };
            let page = PageRequest::first(limit, Order::Desc);
            let reviews = self.0.public_reviews(request.tenant, product, &filter, &page).await?;
            let stats = self.0.review_stats(request.tenant, product, request.locale).await?;
            let reviews: Vec<ReviewResponse> =
                reviews.data.into_iter().map(|row| ReviewResponse::new(row, None)).collect();
            answers.push(Some(json!({"stats": stats_response(stats), "reviews": reviews})));
        }
        Ok(answers)
    }
}
