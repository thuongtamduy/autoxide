//! Thân `layout` của trang (bước 67, D-229, D-20261122-page-layout): cây khối JSON cạnh thân `rich`
//! (ProseMirror, `body.rs`). Cùng cột `body`, phân biệt bằng `type` ở gốc — nên nháp, revision,
//! đăng, hẹn giờ, xem trước dùng nguyên cơ chế sẵn có:
//!
//! ```json
//! {"type": "layout", "schema": 1, "sections": [
//!   {"id": "hero", "block": "content.section", "props": {}, "visibility": {"base": true},
//!    "children": [{"id": "title", "block": "content.heading", "props": {"text": "Sale"}}]}
//! ]}
//! ```
//!
//! TỪ CHỐI MẶC ĐỊNH như thân `rich` (D-102): khối phải có trong sổ khối của kernel và khách đang
//! dùng được; props đúng schema của khối (kernel kiểm, gồm giá trị theo cỡ màn hình
//! `{base, md?, lg?}`, ID `media`, luật link, không HTML); `id` duy nhất trong trang; chỉ khối chứa
//! khối mới có `children`; tối đa [`MAX_BLOCKS`] khối, sâu [`MAX_DEPTH`], [`MAX_BYTES`] byte JSON.
//! Mọi lỗi báo một lần, mỗi lỗi kèm đường dẫn JSON (D-083).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use autoxide_kernel::blocks::schema::BREAKPOINTS;
use autoxide_kernel::blocks::{BlockInfo, Blocks, Children};
use autoxide_kernel::error::FieldError;
use serde_json::{Map, Value};
use uuid::Uuid;

use super::body::BodyInfo;

/// `type` ở gốc của thân `layout`.
pub(crate) const TYPE: &str = "layout";
/// Phiên bản của dạng thân `layout`.
pub(crate) const SCHEMA: i64 = 1;
/// Số khối tối đa của một trang.
pub(crate) const MAX_BLOCKS: usize = 500;
/// Độ sâu lồng tối đa (khối ở `sections` là tầng 1).
pub(crate) const MAX_DEPTH: usize = 6;
/// Kích thước JSON tối đa của thân `layout`.
pub(crate) const MAX_BYTES: usize = 256 * 1024;
const MAX_ERRORS: usize = 50;
const NODE_KEYS: [&str; 5] = ["id", "block", "props", "visibility", "children"];

/// Thân có phải thân `layout` không (theo `type` ở gốc).
pub(crate) fn is_layout(body: &Value) -> bool {
    body.get("type").and_then(Value::as_str) == Some(TYPE)
}

/// Bố cục trống.
pub(crate) fn empty() -> Value {
    serde_json::json!({"type": TYPE, "schema": SCHEMA, "sections": []})
}

/// Khách đang dùng được những khối nào, theo tên (lấy một lần cho mỗi lần kiểm).
pub(crate) type Usable = BTreeMap<&'static str, Arc<BlockInfo>>;

struct Checker<'a> {
    blocks: &'a Blocks,
    usable: &'a Usable,
    errors: Vec<FieldError>,
    ids: BTreeSet<String>,
    count: usize,
    info: BodyInfo,
}

/// Kiểm thân `layout` `body`; hợp lệ thì trả chữ (tìm kiếm, số chữ) và ID `media` trích được.
pub(crate) fn check(
    body: &Value,
    blocks: &Blocks,
    usable: &Usable,
) -> Result<BodyInfo, Vec<FieldError>> {
    let size = serde_json::to_vec(body).map(|bytes| bytes.len()).unwrap_or(usize::MAX);
    if size > MAX_BYTES {
        return Err(vec![
            FieldError::new(
                "body",
                "too_large",
                "is {size} bytes of JSON; at most {max_bytes} are accepted",
            )
            .with_param("size", size)
            .with_param("max_bytes", MAX_BYTES),
        ]);
    }
    let mut checker = Checker {
        blocks,
        usable,
        errors: Vec::new(),
        ids: BTreeSet::new(),
        count: 0,
        info: BodyInfo::default(),
    };
    let Some(root) = body.as_object() else {
        return Err(vec![FieldError::new("body", "invalid", "must be a JSON object")]);
    };
    for key in root.keys() {
        if !["type", "schema", "sections"].contains(&key.as_str()) {
            checker.error(FieldError::new(format!("body.{key}"), "unknown", "is not allowed here"));
        }
    }
    if root.get("schema").and_then(Value::as_i64) != Some(SCHEMA) {
        checker.error(
            FieldError::new("body.schema", "invalid", "must be {schema}")
                .with_param("schema", SCHEMA),
        );
    }
    match root.get("sections") {
        Some(Value::Array(sections)) => checker.nodes(sections, "body.sections", 1),
        _ => {
            checker.error(FieldError::new("body.sections", "required", "must be a list of blocks"))
        }
    }
    if checker.count > MAX_BLOCKS {
        checker.error(
            FieldError::new(
                "body.sections",
                "too_large",
                "has {count} blocks; at most {max} are accepted",
            )
            .with_param("count", checker.count)
            .with_param("max", MAX_BLOCKS),
        );
    }
    if checker.errors.is_empty() {
        let info = &mut checker.info;
        info.plain_text = info.plain_text.trim().to_owned();
        info.word_count =
            i32::try_from(info.plain_text.split_whitespace().count()).unwrap_or(i32::MAX);
        Ok(checker.info)
    } else {
        Err(checker.errors)
    }
}

impl Checker<'_> {
    fn error(&mut self, error: FieldError) {
        if self.errors.len() < MAX_ERRORS {
            self.errors.push(error);
        }
    }

    fn nodes(&mut self, nodes: &[Value], at: &str, depth: usize) {
        if depth > MAX_DEPTH {
            self.error(
                FieldError::new(at, "too_deep", "blocks are nested more than {max} levels deep")
                    .with_param("max", MAX_DEPTH),
            );
            return;
        }
        for (index, node) in nodes.iter().enumerate() {
            self.count += 1;
            if self.count > MAX_BLOCKS {
                return;
            }
            self.node(node, &format!("{at}[{index}]"), depth);
        }
    }

    fn node(&mut self, node: &Value, at: &str, depth: usize) {
        let Some(node) = node.as_object() else {
            self.error(FieldError::new(at, "invalid", "must be a block object"));
            return;
        };
        for key in node.keys() {
            if !NODE_KEYS.contains(&key.as_str()) {
                self.error(FieldError::new(
                    format!("{at}.{key}"),
                    "unknown",
                    "is not allowed here",
                ));
            }
        }
        match node.get("id").and_then(Value::as_str) {
            Some(id) if is_block_id(id) => {
                if !self.ids.insert(id.to_owned()) {
                    self.error(
                        FieldError::new(
                            format!("{at}.id"),
                            "duplicate",
                            "`{id}` is used by another block",
                        )
                        .with_param("id", id),
                    );
                }
            }
            _ => self.error(FieldError::new(
                format!("{at}.id"),
                "invalid",
                "must be 1 to 64 letters, digits, `-` or `_`",
            )),
        }
        let name = node.get("block").and_then(Value::as_str).unwrap_or_default();
        // Chép tham chiếu `&'a Usable` ra trước: `info` mượn sổ, không mượn `self` (còn phải ghi lỗi).
        let usable = self.usable;
        let Some(info) = usable.get(name) else {
            self.error(
                FieldError::new(
                    format!("{at}.block"),
                    "unknown",
                    "block `{block}` is not available",
                )
                .with_param("block", name),
            );
            return;
        };
        let props = node.get("props").cloned().unwrap_or_else(|| Value::Object(Map::new()));
        for error in self.blocks.check_props(name, &props, &format!("{at}.props")) {
            self.error(error);
        }
        collect(&info.schema, &props, &mut self.info);
        if let Some(visibility) = node.get("visibility") {
            self.visibility(visibility, &format!("{at}.visibility"));
        }
        match (node.get("children"), info.block.children) {
            (None, _) => {}
            (Some(Value::Array(children)), Children::Blocks) => {
                self.nodes(children, &format!("{at}.children"), depth + 1);
            }
            (Some(_), Children::Blocks) => self.error(FieldError::new(
                format!("{at}.children"),
                "invalid",
                "must be a list of blocks",
            )),
            (Some(_), _) => self.error(
                FieldError::new(
                    format!("{at}.children"),
                    "unknown",
                    "block `{block}` holds no blocks",
                )
                .with_param("block", name),
            ),
        }
    }

    fn visibility(&mut self, visibility: &Value, at: &str) {
        let valid = visibility.as_object().is_some_and(|sizes| {
            sizes
                .iter()
                .all(|(size, shown)| BREAKPOINTS.contains(&size.as_str()) && shown.is_boolean())
        });
        if !valid {
            self.error(FieldError::new(
                at,
                "invalid",
                "must map screen sizes `base`, `md`, `lg` to true or false",
            ));
        }
    }
}

/// `id` của khối: 1–64 chữ cái, chữ số, `-`, `_`.
fn is_block_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Trích chữ (để tìm kiếm) và ID `media` từ props đã hợp lệ, theo schema của khối.
fn collect(schema: &Value, value: &Value, info: &mut BodyInfo) {
    let responsive = schema.get("x-responsive") == Some(&Value::Bool(true));
    match value {
        Value::String(text) => {
            if schema.get("x-media") == Some(&Value::Bool(true)) {
                if let Ok(id) = Uuid::parse_str(text) {
                    info.media_ids.insert(id);
                }
            } else if schema.get("format").is_none()
                && schema.get("x-link").is_none()
                && schema.get("enum").is_none()
            {
                info.plain_text.push_str(text);
                info.plain_text.push('\n');
            }
        }
        Value::Array(items) => {
            if let Some(inner) = schema.get("items") {
                for item in items {
                    collect(inner, item, info);
                }
            }
        }
        Value::Object(fields) if responsive && fields.contains_key("base") => {
            for item in fields.values() {
                collect(schema, item, info);
            }
        }
        Value::Object(fields) => {
            let properties = schema.get("properties").and_then(Value::as_object);
            for (name, item) in fields {
                if let Some(inner) = properties.and_then(|properties| properties.get(name)) {
                    collect(inner, item, info);
                }
            }
        }
        _ => {}
    }
}
