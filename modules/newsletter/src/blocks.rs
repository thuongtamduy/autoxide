//! Khối `newsletter.signup` cho trang dựng từ khối (bước 67): ô đăng ký nhận tin gửi tới
//! `POST /api/v1/newsletter/public/subscribe`. Luôn có câu đồng ý và link chính sách (bước 37, Luật
//! 91/2025/QH15 — cần đối chiếu văn bản gốc): chưa tích ô đồng ý thì không gửi gì. Không cần dữ
//! liệu lúc đọc công khai (danh sách công khai ở `GET /public/lists`).

use autoxide_kernel::blocks::{BlockCategory, BlockType};

pub(crate) const SIGNUP: BlockType = BlockType::new(
    "newsletter.signup",
    BlockCategory::Lead,
    "Newsletter sign-up",
    "Use to let visitors join the newsletter with their email; it always shows a consent checkbox and the privacy policy link.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "required": ["consent_text", "policy_link"],
        "properties": {
            "heading": {"type": "string", "maxLength": 120},
            "text": {"type": "string", "maxLength": 500},
            "lists": {"type": "array", "maxItems": 10, "items": {"type": "string", "minLength": 1, "maxLength": 64}, "description": "Keys of public lists the address joins (`GET /api/v1/newsletter/public/lists`); left out: the server's default."},
            "ask_name": {"type": "boolean"},
            "button_label": {"type": "string", "maxLength": 60},
            "consent_text": {"type": "string", "minLength": 1, "maxLength": 500, "description": "Text beside the consent checkbox, sent as `consent_text` with the sign-up."},
            "policy_link": {"type": "string", "x-link": true, "maxLength": 2048, "description": "The privacy policy."},
            "layout": {"enum": ["inline", "stacked"], "x-responsive": true}
        }
    }"#,
);

pub(crate) fn all() -> Vec<BlockType> {
    vec![SIGNUP]
}
