//! Khối `comments.thread` cho trang dựng từ khối (bước 67): luồng bình luận của một đối tượng, đọc
//! và gửi qua `GET /api/v1/comments/public/threads` và `POST /public/comments`. Không cần dữ liệu lúc
//! đọc công khai: bình luận đổi liên tục, giao diện đọc trực tiếp (có pagination, có cache riêng).

use autoxide_kernel::blocks::{BlockCategory, BlockType};

pub(crate) const THREAD: BlockType = BlockType::new(
    "comments.thread",
    BlockCategory::Social,
    "Comments",
    "Use to let visitors read and write comments under the page; leave the target out to use the page itself.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "target_type": {"type": "string", "minLength": 3, "maxLength": 64, "description": "Kind of the target, e.g. `content.entry`; left out: the page shown."},
            "target_id": {"type": "string", "minLength": 1, "maxLength": 200},
            "title": {"type": "string", "maxLength": 120},
            "replies_limit": {"type": "integer", "minimum": 0, "maximum": 10},
            "order": {"enum": ["desc", "asc"], "description": "`desc`: newest first (default), as the `order` of the public threads list."}
        }
    }"#,
);

pub(crate) fn all() -> Vec<BlockType> {
    vec![THREAD]
}
