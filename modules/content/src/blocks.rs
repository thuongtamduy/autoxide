//! Khối nền của `content` cho trang dựng từ khối (bước 67). Bước này mới có hai khối tối thiểu —
//! `content.section` (khung chứa khối) và `content.heading` (tiêu đề) — đủ để dựng và kiểm bố cục;
//! các khối nền còn lại (chữ, nút, media, FAQ, thanh thông báo…) là mục sau của bước 67.

use autoxide_kernel::blocks::{BlockCategory, BlockType};

/// Phần của trang: nền, khoảng cách, chiều rộng; chứa khối con.
pub(crate) const SECTION: BlockType = BlockType::new(
    "content.section",
    BlockCategory::Layout,
    "Section",
    "Use to group blocks into a band of the page with its own background and spacing.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "background": {"enum": ["none", "surface", "primary", "secondary"], "description": "Theme color token of the background."},
            "padding": {"enum": ["none", "small", "medium", "large"], "x-responsive": true, "description": "Space inside the section."},
            "width": {"enum": ["narrow", "normal", "full"], "description": "Width of the content."}
        }
    }"#,
)
.with_children();

/// Tiêu đề.
pub(crate) const HEADING: BlockType = BlockType::new(
    "content.heading",
    BlockCategory::Text,
    "Heading",
    "Use for the title of the page or of a section; level 1 only once per page.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "required": ["text"],
        "properties": {
            "text": {"type": "string", "minLength": 1, "maxLength": 200},
            "level": {"type": "integer", "minimum": 1, "maximum": 4},
            "align": {"enum": ["left", "center", "right"], "x-responsive": true}
        }
    }"#,
);

pub(crate) fn all() -> Vec<BlockType> {
    vec![SECTION, HEADING]
}
