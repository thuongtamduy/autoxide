//! Khối nền của `content` cho trang dựng từ khối (bước 67, D-229, D-20261122-base-blocks): bố cục,
//! chữ, media, thu khách, nhúng. Chỉ token của theme (`primary`…), không mã màu tự do; ảnh và video
//! là ID `media` (`x-media`); link theo luật link của thân bài (`x-link`); chữ là chữ thuần (không
//! HTML). Khối nào cần dữ liệu lúc đọc công khai (`content.placement`) có [`data`].

pub(crate) mod data;

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

/// Cột: mỗi khối con là một cột; điện thoại xếp chồng.
pub(crate) const COLUMNS: BlockType = BlockType::new(
    "content.columns",
    BlockCategory::Layout,
    "Columns",
    "Use to put 2 to 4 blocks side by side; each child is one column and they stack on phones.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "widths": {"enum": ["equal", "wide_left", "wide_right"], "description": "How the width is shared between the columns."},
            "gap": {"enum": ["none", "small", "medium", "large"], "x-responsive": true},
            "align": {"enum": ["top", "center", "bottom"], "description": "Vertical alignment of the columns."},
            "stack_on": {"enum": ["base", "md", "never"], "description": "Screen size up to which the columns stack (default `base`: phones)."}
        }
    }"#,
)
.with_children();

/// Khoảng trống.
pub(crate) const SPACER: BlockType = BlockType::new(
    "content.spacer",
    BlockCategory::Layout,
    "Spacer",
    "Use to add empty space between two blocks.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "size": {"enum": ["small", "medium", "large", "xlarge"], "x-responsive": true}
        }
    }"#,
);

/// Đường kẻ ngang.
pub(crate) const DIVIDER: BlockType = BlockType::new(
    "content.divider",
    BlockCategory::Layout,
    "Divider",
    "Use to separate two parts of a section with a horizontal line.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "style": {"enum": ["solid", "dashed", "dotted"]},
            "width": {"enum": ["short", "full"]}
        }
    }"#,
);

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

/// Đoạn chữ thuần (dòng trống tách đoạn). Thân `rich` có định dạng vẫn là thân bài viết.
pub(crate) const TEXT: BlockType = BlockType::new(
    "content.text",
    BlockCategory::Text,
    "Text",
    "Use for paragraphs of plain text; a blank line starts a new paragraph. Never write prices, discounts or dates of a sale here: use the block that carries the data.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "required": ["text"],
        "properties": {
            "text": {"type": "string", "minLength": 1, "maxLength": 10000},
            "size": {"enum": ["small", "normal", "large"]},
            "align": {"enum": ["left", "center", "right"], "x-responsive": true},
            "color": {"enum": ["text", "muted", "primary", "secondary"], "description": "Theme color token."}
        }
    }"#,
);

/// Nút: link trong site hay ra ngoài.
pub(crate) const BUTTON: BlockType = BlockType::new(
    "content.button",
    BlockCategory::Text,
    "Button",
    "Use for a call to action that opens a page of the website or another address.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "required": ["label", "link"],
        "properties": {
            "label": {"type": "string", "minLength": 1, "maxLength": 60},
            "link": {"type": "string", "x-link": true, "maxLength": 2048},
            "style": {"enum": ["primary", "secondary", "outline", "text"]},
            "size": {"enum": ["small", "medium", "large"]},
            "align": {"enum": ["left", "center", "right", "full"], "x-responsive": true},
            "new_tab": {"type": "boolean"}
        }
    }"#,
);

/// Hỏi đáp; giao diện sinh JSON-LD `FAQPage` từ chính các cặp này.
pub(crate) const FAQ: BlockType = BlockType::new(
    "content.faq",
    BlockCategory::Text,
    "FAQ",
    "Use for a list of questions and answers; search engines read it as an FAQ.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "required": ["items"],
        "properties": {
            "title": {"type": "string", "maxLength": 200},
            "items": {"type": "array", "minItems": 1, "maxItems": 50, "items": {
                "type": "object",
                "additionalProperties": false,
                "required": ["question", "answer"],
                "properties": {
                    "question": {"type": "string", "minLength": 1, "maxLength": 300},
                    "answer": {"type": "string", "minLength": 1, "maxLength": 3000}
                }
            }},
            "open_first": {"type": "boolean", "description": "Show the first answer opened."}
        }
    }"#,
);

/// Ảnh của thư viện `media`.
pub(crate) const IMAGE: BlockType = BlockType::new(
    "content.image",
    BlockCategory::Media,
    "Image",
    "Use for one picture from the media library; describe it in `alt`. Leave `media_id` out when no suitable picture exists.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "media_id": {"type": "string", "format": "uuid", "x-media": true},
            "alt": {"type": "string", "maxLength": 300},
            "caption": {"type": "string", "maxLength": 300},
            "ratio": {"enum": ["auto", "1:1", "4:3", "3:2", "16:9", "21:9"], "x-responsive": true},
            "link": {"type": "string", "x-link": true, "maxLength": 2048},
            "rounded": {"type": "boolean"}
        }
    }"#,
);

/// Nhiều ảnh: lưới hay băng chuyền.
pub(crate) const GALLERY: BlockType = BlockType::new(
    "content.gallery",
    BlockCategory::Media,
    "Gallery",
    "Use for several pictures from the media library shown as a grid or a carousel.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "required": ["items"],
        "properties": {
            "items": {"type": "array", "minItems": 1, "maxItems": 30, "items": {
                "type": "object",
                "additionalProperties": false,
                "required": ["media_id"],
                "properties": {
                    "media_id": {"type": "string", "format": "uuid", "x-media": true},
                    "alt": {"type": "string", "maxLength": 300},
                    "caption": {"type": "string", "maxLength": 300}
                }
            }},
            "display": {"enum": ["grid", "carousel"]},
            "columns": {"type": "integer", "minimum": 1, "maximum": 6, "x-responsive": true}
        }
    }"#,
);

/// Video: của thư viện `media` hay nhúng từ host cho phép (`AUTOXIDE_CONTENT_EMBED_HOSTS`).
pub(crate) const VIDEO: BlockType = BlockType::new(
    "content.video",
    BlockCategory::Media,
    "Video",
    "Use for a video from the media library (`media_id`) or from an allowed video site (`embed_url`, e.g. YouTube); exactly one of the two.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "media_id": {"type": "string", "format": "uuid", "x-media": true},
            "embed_url": {"type": "string", "x-link": true, "maxLength": 2048, "description": "An `https://` address on a host of `AUTOXIDE_CONTENT_EMBED_HOSTS`."},
            "poster_media_id": {"type": "string", "format": "uuid", "x-media": true},
            "title": {"type": "string", "maxLength": 200},
            "autoplay_muted": {"type": "boolean"},
            "ratio": {"enum": ["16:9", "4:3", "1:1", "9:16"]}
        }
    }"#,
);

/// Ảnh nền lớn kèm tiêu đề, chữ và nút.
pub(crate) const HERO: BlockType = BlockType::new(
    "content.hero",
    BlockCategory::Media,
    "Hero",
    "Use at the top of a landing page: a large background picture with a title, a short text and one button.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "required": ["heading"],
        "properties": {
            "media_id": {"type": "string", "format": "uuid", "x-media": true},
            "mobile_media_id": {"type": "string", "format": "uuid", "x-media": true},
            "alt": {"type": "string", "maxLength": 300},
            "heading": {"type": "string", "minLength": 1, "maxLength": 200},
            "text": {"type": "string", "maxLength": 500},
            "button_label": {"type": "string", "maxLength": 60},
            "button_link": {"type": "string", "x-link": true, "maxLength": 2048},
            "align": {"enum": ["left", "center", "right"], "x-responsive": true},
            "height": {"enum": ["small", "medium", "large", "screen"], "x-responsive": true},
            "overlay": {"enum": ["none", "light", "dark"], "description": "Shade over the picture so the text stays readable."}
        }
    }"#,
);

/// Thanh thông báo ngang (chữ cố định của trang; banner có lịch là `content.placement`).
pub(crate) const ANNOUNCEMENT_BAR: BlockType = BlockType::new(
    "content.announcement_bar",
    BlockCategory::Lead,
    "Announcement bar",
    "Use for one short line across the page, such as a free shipping notice; scheduled banners use a placement instead.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "required": ["text"],
        "properties": {
            "text": {"type": "string", "minLength": 1, "maxLength": 200},
            "link": {"type": "string", "x-link": true, "maxLength": 2048},
            "tone": {"enum": ["primary", "secondary", "success", "warning", "neutral"]},
            "dismissible": {"type": "boolean"}
        }
    }"#,
);

/// Đếm ngược tới một mốc giờ UTC.
pub(crate) const COUNTDOWN: BlockType = BlockType::new(
    "content.countdown",
    BlockCategory::Lead,
    "Countdown",
    "Use to count down to a fixed moment, such as an event; for a sale use the promotion's own block, which knows its real end.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "required": ["ends_at"],
        "properties": {
            "ends_at": {"type": "string", "format": "date-time"},
            "label": {"type": "string", "maxLength": 120},
            "ended_text": {"type": "string", "maxLength": 200, "description": "Shown once the moment has passed; empty: the block hides."},
            "show": {"enum": ["days", "hours", "minutes", "seconds"], "description": "Smallest unit shown."}
        }
    }"#,
);

/// Popup thu email: luôn có ô đồng ý và link chính sách (bước 37); gửi tới `newsletter`.
pub(crate) const POPUP: BlockType = BlockType::new(
    "content.popup",
    BlockCategory::Lead,
    "Email sign-up popup",
    "Use to ask visitors for their email in a popup; it always shows a consent checkbox and the privacy policy link, and never blocks closing.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "required": ["heading", "consent_text", "policy_link"],
        "properties": {
            "heading": {"type": "string", "minLength": 1, "maxLength": 120},
            "text": {"type": "string", "maxLength": 500},
            "media_id": {"type": "string", "format": "uuid", "x-media": true},
            "button_label": {"type": "string", "maxLength": 60},
            "lists": {"type": "array", "maxItems": 10, "items": {"type": "string", "minLength": 1, "maxLength": 64}, "description": "Keys of public newsletter lists the address joins (`GET /api/v1/newsletter/public/lists`); left out: the server's default."},
            "consent_text": {"type": "string", "minLength": 1, "maxLength": 500, "description": "Text beside the consent checkbox: what the address is used for. Nothing is sent before the box is ticked."},
            "policy_link": {"type": "string", "x-link": true, "maxLength": 2048, "description": "The privacy policy."},
            "trigger": {"enum": ["on_load", "after_seconds", "on_exit", "on_scroll"]},
            "delay_seconds": {"type": "integer", "minimum": 0, "maximum": 600},
            "frequency": {"enum": ["once", "every_visit", "daily", "weekly"], "description": "How often the same visitor sees it after closing it."}
        }
    }"#,
);

/// Bản ghi của module khác qua sổ đối tượng (D-123), như node `embed` của thân `rich`.
pub(crate) const EMBED: BlockType = BlockType::new(
    "content.embed",
    BlockCategory::Embed,
    "Embedded record",
    "Use to show a card of another record of the website, such as a product or an article, by its kind and id.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "required": ["kind", "id"],
        "properties": {
            "kind": {"type": "string", "minLength": 3, "maxLength": 64, "description": "A record type of `GET /api/v1/kernel/resources`, e.g. `catalog.product`."},
            "id": {"type": "string", "minLength": 1, "maxLength": 200}
        }
    }"#,
);

/// Ô tìm kiếm của website.
pub(crate) const SEARCH_BOX: BlockType = BlockType::new(
    "content.search_box",
    BlockCategory::Navigation,
    "Search box",
    "Use for a box that searches the website, usually in the header.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "placeholder": {"type": "string", "maxLength": 80},
            "scope": {"enum": ["all", "posts", "products"]}
        }
    }"#,
);

/// Vùng banner có lịch, quản lý một chỗ (bước 55, D-236): dữ liệu là banner đang chạy của vị trí.
pub(crate) const PLACEMENT: BlockType = BlockType::new(
    "content.placement",
    BlockCategory::Embed,
    "Banner placement",
    "Use to show the scheduled banners of a placement, managed in one place; use a fixed block for text that belongs to this page only.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "required": ["key"],
        "properties": {
            "key": {"type": "string", "minLength": 1, "maxLength": 64, "description": "Key of a placement, e.g. `home_hero`."}
        }
    }"#,
)
.with_data();

/// Khu dùng chung nhiều trang: lúc đọc công khai được thay bằng nội dung hiện tại của khu.
pub(crate) const SHARED_SECTION: BlockType = BlockType::new(
    "content.shared_section",
    BlockCategory::Layout,
    "Shared section",
    "Use to place a shared section (the same blocks on many pages, edited in one place) by its id.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "required": ["id"],
        "properties": {
            "id": {"type": "string", "format": "uuid", "description": "A shared section of `GET /api/v1/content/shared-sections`."}
        }
    }"#,
);

/// Một trường của mục đang xem — chỉ trong bố cục trang chi tiết của loại nội dung tự định nghĩa
/// (`detail_layout`, D-235); `content` điền dữ liệu lúc đọc công khai.
pub(crate) const ENTRY_FIELD: BlockType = BlockType::new(
    "content.entry_field",
    BlockCategory::Text,
    "Entry field",
    "Only in the detail layout of a custom content type: shows one field of the entry being viewed.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "required": ["field"],
        "properties": {
            "field": {"type": "string", "minLength": 1, "maxLength": 64, "description": "`title`, `excerpt`, `body`, `featured_media`, `published_at`, `authors`, or the key of a public custom field of the type."},
            "label": {"type": "string", "maxLength": 80, "description": "Text shown before the value; left out: none."},
            "style": {"enum": ["heading", "text", "badge", "image", "link"], "description": "How to draw the value."}
        }
    }"#,
);

/// Danh sách mục của một loại nội dung tự định nghĩa (D-235): dữ liệu là các mục đã đăng.
pub(crate) const TYPE_LIST: BlockType = BlockType::new(
    "content.type_list",
    BlockCategory::Embed,
    "Content type list",
    "Use to show the published entries of a custom content type (team, projects, branches…) as cards; filter on its filterable public fields.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "required": ["type"],
        "properties": {
            "type": {"type": "string", "minLength": 1, "maxLength": 64, "description": "Key of a listable content type of `GET /api/v1/content/types`."},
            "title": {"type": "string", "maxLength": 200},
            "filters": {"type": "array", "maxItems": 5, "description": "Only entries matching every filter; only filterable public custom fields.", "items": {
                "type": "object",
                "additionalProperties": false,
                "required": ["field", "value"],
                "properties": {
                    "field": {"type": "string", "minLength": 1, "maxLength": 64, "description": "Key of the custom field."},
                    "value": {"type": "string", "minLength": 1, "maxLength": 200, "description": "`<eq|in|gte|lte|prefix>:<value>`, as `cf.<key>` of `GET /public/types/{key}/entries`."}
                }
            }},
            "sort": {"enum": ["newest", "oldest"]},
            "limit": {"type": "integer", "minimum": 1, "maximum": 24},
            "card_fields": {"type": "array", "maxItems": 6, "items": {"type": "string", "minLength": 1, "maxLength": 64}, "description": "Public custom fields shown on each card."},
            "layout": {"enum": ["grid", "list", "carousel"]}
        }
    }"#,
)
.with_data();

pub(crate) fn all() -> Vec<BlockType> {
    vec![
        SECTION,
        COLUMNS,
        SPACER,
        DIVIDER,
        HEADING,
        TEXT,
        BUTTON,
        FAQ,
        IMAGE,
        GALLERY,
        VIDEO,
        HERO,
        ANNOUNCEMENT_BAR,
        COUNTDOWN,
        POPUP,
        EMBED,
        SEARCH_BOX,
        PLACEMENT,
        SHARED_SECTION,
        ENTRY_FIELD,
        TYPE_LIST,
    ]
}
