//! Thân bài: JSON theo cấu trúc ProseMirror / Tiptap (`type`, `attrs`, `content`, `marks`,
//! `text`). Máy chủ TỪ CHỐI MẶC ĐỊNH (D-102): chỉ nhận node, mark, thuộc tính trong danh
//! sách dưới đây; link chỉ `http`, `https`, `mailto` hoặc đường dẫn bắt đầu bằng `/`; ảnh
//! chỉ giữ ID của `media` (không `src`); `embed` chỉ từ host cho phép. Mọi lỗi báo một lần,
//! mỗi lỗi kèm vị trí trong JSON (`body.content[2].attrs.href`). Không bao giờ lưu HTML.
//!
//! Kiểm xong thì trích luôn: văn bản thuần (tìm kiếm, tóm tắt), số chữ, thời gian đọc,
//! danh sách ID ảnh (kiểm với `media`).

use std::collections::BTreeSet;

use autoxide_kernel::error::FieldError;
use serde_json::{Map, Value};
use uuid::Uuid;

/// Phiên bản của danh sách node đang dùng. Đổi danh sách thì tăng số và migrate cả revision.
pub(crate) const SCHEMA_VERSION: i32 = 1;
/// Độ sâu lồng tối đa của cây node.
pub(crate) const MAX_DEPTH: usize = 32;
/// Số node tối đa.
pub(crate) const MAX_NODES: usize = 20_000;
/// Báo tối đa chừng này lỗi (đủ để sửa; tránh response khổng lồ cho JSON hỏng hàng loạt).
const MAX_ERRORS: usize = 50;

/// Những gì trích được từ một thân bài hợp lệ.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct BodyInfo {
    pub plain_text: String,
    pub word_count: i32,
    pub media_ids: BTreeSet<Uuid>,
}

/// Thân bài trống (`{"type": "doc", "content": []}`).
pub(crate) fn empty() -> Value {
    serde_json::json!({"type": "doc", "content": []})
}

/// Nhóm node theo chỗ được đứng.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    /// Con trực tiếp của `doc`, `blockquote`, `listItem`, ô bảng.
    Block,
    /// Trong `paragraph`, `heading`.
    Inline,
    /// Trong `codeBlock`: chỉ chữ, không mark.
    PlainText,
    /// Trong danh sách: chỉ `listItem`.
    ListItem,
    /// Trong `table`: chỉ `tableRow`.
    TableRow,
    /// Trong `tableRow`: chỉ ô.
    TableCell,
}

struct Checker<'a> {
    embed_hosts: &'a [String],
    errors: Vec<FieldError>,
    nodes: usize,
    info: BodyInfo,
}

/// ID các ảnh (node `image`) trong thân bài đã lưu `body`, theo thứ tự xuất hiện, không lặp.
///
/// Thân bài đã lưu thì đã qua [`check`], nên ở đây chỉ đi cây; JSON hỏng hay node lạ coi như
/// không có ảnh. Thân bài không có chữ `"image"` thì khỏi phải đọc JSON (đường nóng của danh
/// sách bài công khai).
pub(crate) fn stored_image_ids(body: &str) -> Vec<Uuid> {
    if !body.contains("\"image\"") {
        return Vec::new();
    }
    let Ok(root) = serde_json::from_str::<Value>(body) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    // Duyệt bằng ngăn xếp (không đệ quy): đẩy con theo thứ tự ngược để lấy ra đúng thứ tự.
    let mut pending = vec![&root];
    while let Some(node) = pending.pop() {
        if node.get("type").and_then(Value::as_str) == Some("image") {
            let id = node
                .pointer("/attrs/mediaId")
                .and_then(Value::as_str)
                .and_then(|id| Uuid::parse_str(id).ok());
            if let Some(id) = id.filter(|id| !found.contains(id)) {
                found.push(id);
            }
        }
        if let Some(children) = node.get("content").and_then(Value::as_array) {
            pending.extend(children.iter().rev());
        }
    }
    found
}

/// Kiểm `body`; trả thông tin trích được, hoặc mọi lỗi (trường tên `body…`).
pub(crate) fn check(
    body: &Value,
    max_bytes: usize,
    embed_hosts: &[String],
) -> Result<BodyInfo, Vec<FieldError>> {
    let size = serde_json::to_vec(body).map(|bytes| bytes.len()).unwrap_or(usize::MAX);
    if size > max_bytes {
        return Err(vec![
            FieldError::new(
                "body",
                "too_large",
                "is {size} bytes of JSON; at most {max_bytes} are accepted",
            )
            .with_param("size", size)
            .with_param("max_bytes", max_bytes),
        ]);
    }
    let mut checker =
        Checker { embed_hosts, errors: Vec::new(), nodes: 0, info: BodyInfo::default() };
    match body.as_object() {
        Some(doc) if doc.get("type").and_then(Value::as_str) == Some("doc") => {
            checker.keys(doc, "body", &["type", "content"]);
            checker.children(doc, "body", Place::Block, 1);
        }
        _ => checker.error(FieldError::new(
            "body",
            "invalid",
            "must be an object with `\"type\": \"doc\"`",
        )),
    }
    if checker.nodes > MAX_NODES {
        checker.error(
            FieldError::new("body", "too_many_nodes", "has {nodes} nodes; at most {max_nodes}")
                .with_param("nodes", checker.nodes)
                .with_param("max_nodes", MAX_NODES),
        );
    }
    if !checker.errors.is_empty() {
        return Err(checker.errors);
    }
    let mut info = checker.info;
    info.plain_text = info.plain_text.trim().to_owned();
    info.word_count = i32::try_from(info.plain_text.split_whitespace().count()).unwrap_or(i32::MAX);
    Ok(info)
}

/// Thời gian đọc (phút, làm tròn lên; ít nhất 1 khi có chữ).
pub(crate) fn reading_minutes(words: i32, words_per_minute: u32) -> i32 {
    if words <= 0 {
        return 0;
    }
    let per_minute = i32::try_from(words_per_minute.max(1)).unwrap_or(i32::MAX);
    (words + per_minute - 1) / per_minute
}

impl Checker<'_> {
    fn error(&mut self, error: FieldError) {
        if self.errors.len() < MAX_ERRORS {
            self.errors.push(error);
        }
    }

    /// Báo khoá không nằm trong `allowed`.
    fn keys(&mut self, node: &Map<String, Value>, path: &str, allowed: &[&str]) {
        for key in node.keys() {
            if !allowed.contains(&key.as_str()) {
                self.error(
                    FieldError::new(
                        format!("{path}.{key}"),
                        "not_allowed",
                        "`{key}` is not allowed here",
                    )
                    .with_param("key", key),
                );
            }
        }
    }

    /// Kiểm mảng `content` của `node` (không có thì coi như rỗng).
    fn children(&mut self, node: &Map<String, Value>, path: &str, place: Place, depth: usize) {
        let Some(content) = node.get("content") else { return };
        let Some(items) = content.as_array() else {
            self.error(FieldError::new(format!("{path}.content"), "invalid", "must be an array"));
            return;
        };
        for (index, child) in items.iter().enumerate() {
            self.node(child, &format!("{path}.content[{index}]"), place, depth);
        }
    }

    fn node(&mut self, value: &Value, path: &str, place: Place, depth: usize) {
        self.nodes += 1;
        if depth > MAX_DEPTH {
            self.error(
                FieldError::new(path, "too_deep", "nodes may nest at most {max_depth} levels")
                    .with_param("max_depth", MAX_DEPTH),
            );
            return;
        }
        let Some(node) = value.as_object() else {
            self.error(FieldError::new(path, "invalid", "must be an object"));
            return;
        };
        let Some(kind) = node.get("type").and_then(Value::as_str) else {
            self.error(FieldError::new(format!("{path}.type"), "required", "must be a node type"));
            return;
        };
        let fits = match kind {
            "text" => matches!(place, Place::Inline | Place::PlainText),
            "hardBreak" => place == Place::Inline,
            "listItem" => place == Place::ListItem,
            "tableRow" => place == Place::TableRow,
            "tableHeader" | "tableCell" => place == Place::TableCell,
            "paragraph" | "heading" | "bulletList" | "orderedList" | "blockquote" | "codeBlock"
            | "horizontalRule" | "image" | "table" | "embed" => place == Place::Block,
            _ => {
                self.error(
                    FieldError::new(
                        format!("{path}.type"),
                        "not_allowed",
                        "`{kind}` is not an allowed node type",
                    )
                    .with_param("kind", kind),
                );
                return;
            }
        };
        if !fits {
            self.error(
                FieldError::new(format!("{path}.type"), "misplaced", "`{kind}` cannot appear here")
                    .with_param("kind", kind),
            );
            return;
        }
        let depth = depth + 1;
        match kind {
            "text" => self.text(node, path, place),
            "hardBreak" | "horizontalRule" => self.keys(node, path, &["type"]),
            "paragraph" => {
                self.keys(node, path, &["type", "content"]);
                self.children(node, path, Place::Inline, depth);
            }
            "heading" => {
                self.keys(node, path, &["type", "attrs", "content"]);
                let attrs = self.attrs(node, path, &["level"]);
                let level =
                    attrs.as_ref().and_then(|attrs| attrs.get("level")).and_then(Value::as_i64);
                if !matches!(level, Some(2..=4)) {
                    self.error(FieldError::new(
                        format!("{path}.attrs.level"),
                        "range",
                        "must be 2, 3 or 4",
                    ));
                }
                self.children(node, path, Place::Inline, depth);
            }
            "bulletList" => {
                self.keys(node, path, &["type", "content"]);
                self.children(node, path, Place::ListItem, depth);
            }
            "orderedList" => {
                self.keys(node, path, &["type", "attrs", "content"]);
                if let Some(attrs) = self.attrs(node, path, &["start"])
                    && let Some(start) = attrs.get("start")
                    && !start.as_u64().is_some_and(|start| start >= 1)
                {
                    self.error(FieldError::new(
                        format!("{path}.attrs.start"),
                        "range",
                        "must be a whole number from 1",
                    ));
                }
                self.children(node, path, Place::ListItem, depth);
            }
            "listItem" | "blockquote" | "tableHeader" | "tableCell" => {
                let allowed: &[&str] = if kind.starts_with("table") {
                    &["type", "attrs", "content"]
                } else {
                    &["type", "content"]
                };
                self.keys(node, path, allowed);
                if kind.starts_with("table")
                    && let Some(attrs) = self.attrs(node, path, &["colspan", "rowspan", "colwidth"])
                {
                    for span in ["colspan", "rowspan"] {
                        if let Some(value) = attrs.get(span)
                            && !value.as_u64().is_some_and(|n| (1..=100).contains(&n))
                        {
                            self.error(FieldError::new(
                                format!("{path}.attrs.{span}"),
                                "range",
                                "must be 1 to 100",
                            ));
                        }
                    }
                    if let Some(widths) = attrs.get("colwidth") {
                        let valid = widths.is_null()
                            || widths.as_array().is_some_and(|items| {
                                items.iter().all(|w| w.as_u64().is_some_and(|w| w <= 10_000))
                            });
                        if !valid {
                            self.error(FieldError::new(
                                format!("{path}.attrs.colwidth"),
                                "invalid",
                                "must be a list of widths in pixels, or null",
                            ));
                        }
                    }
                }
                self.children(node, path, Place::Block, depth);
            }
            "codeBlock" => {
                self.keys(node, path, &["type", "attrs", "content"]);
                if let Some(attrs) = self.attrs(node, path, &["language"])
                    && let Some(language) = attrs.get("language")
                {
                    let valid = language.is_null()
                        || language.as_str().is_some_and(|l| {
                            l.len() <= 32
                                && l.chars()
                                    .all(|c| c.is_ascii_alphanumeric() || "+#-_.".contains(c))
                        });
                    if !valid {
                        self.error(FieldError::new(
                            format!("{path}.attrs.language"),
                            "invalid",
                            "must be a short language name like `rust`",
                        ));
                    }
                }
                self.children(node, path, Place::PlainText, depth);
                self.info.plain_text.push('\n');
            }
            "table" => {
                self.keys(node, path, &["type", "content"]);
                self.children(node, path, Place::TableRow, depth);
            }
            "tableRow" => {
                self.keys(node, path, &["type", "content"]);
                self.children(node, path, Place::TableCell, depth);
            }
            "image" => self.image(node, path),
            "embed" => self.embed(node, path),
            _ => {}
        }
        if matches!(
            kind,
            "paragraph" | "heading" | "listItem" | "blockquote" | "tableCell" | "tableHeader"
        ) {
            self.info.plain_text.push('\n');
        }
    }

    /// `attrs` của node, chỉ với khoá trong `allowed`; không có thì `None`.
    fn attrs(
        &mut self,
        node: &Map<String, Value>,
        path: &str,
        allowed: &[&str],
    ) -> Option<Map<String, Value>> {
        match node.get("attrs") {
            None | Some(Value::Null) => None,
            Some(Value::Object(attrs)) => {
                let attrs = attrs.clone();
                self.keys(&attrs, &format!("{path}.attrs"), allowed);
                Some(attrs)
            }
            Some(_) => {
                self.error(FieldError::new(
                    format!("{path}.attrs"),
                    "invalid",
                    "must be an object",
                ));
                None
            }
        }
    }

    fn text(&mut self, node: &Map<String, Value>, path: &str, place: Place) {
        let allowed: &[&str] =
            if place == Place::PlainText { &["type", "text"] } else { &["type", "text", "marks"] };
        self.keys(node, path, allowed);
        match node.get("text").and_then(Value::as_str) {
            Some(text) if !text.is_empty() => self.info.plain_text.push_str(text),
            _ => self.error(FieldError::new(
                format!("{path}.text"),
                "required",
                "must be a non-empty string",
            )),
        }
        if place == Place::PlainText {
            return;
        }
        let Some(marks) = node.get("marks") else { return };
        let Some(marks) = marks.as_array() else {
            self.error(FieldError::new(format!("{path}.marks"), "invalid", "must be an array"));
            return;
        };
        for (index, mark) in marks.iter().enumerate() {
            self.mark(mark, &format!("{path}.marks[{index}]"));
        }
    }

    fn mark(&mut self, value: &Value, path: &str) {
        let Some(mark) = value.as_object() else {
            self.error(FieldError::new(path, "invalid", "must be an object"));
            return;
        };
        match mark.get("type").and_then(Value::as_str) {
            Some("bold" | "italic" | "underline" | "strike" | "code") => {
                self.keys(mark, path, &["type"])
            }
            Some("link") => {
                self.keys(mark, path, &["type", "attrs"]);
                let attrs = self.attrs(mark, path, &["href"]);
                let href =
                    attrs.as_ref().and_then(|attrs| attrs.get("href")).and_then(Value::as_str);
                if !href.is_some_and(safe_link) {
                    self.error(FieldError::new(
                        format!("{path}.attrs.href"),
                        "unsafe_link",
                        "must start with `http://`, `https://`, `mailto:` or `/`",
                    ));
                }
            }
            Some(other) => self.error(
                FieldError::new(
                    format!("{path}.type"),
                    "not_allowed",
                    "`{other}` is not an allowed mark",
                )
                .with_param("other", other),
            ),
            None => self.error(FieldError::new(
                format!("{path}.type"),
                "required",
                "must be a mark type",
            )),
        }
    }

    fn image(&mut self, node: &Map<String, Value>, path: &str) {
        self.keys(node, path, &["type", "attrs"]);
        let attrs = self.attrs(node, path, &["mediaId", "alt", "caption"]).unwrap_or_default();
        match attrs.get("mediaId").and_then(Value::as_str).and_then(|id| Uuid::parse_str(id).ok()) {
            Some(id) => {
                self.info.media_ids.insert(id);
            }
            None => self.error(FieldError::new(
                format!("{path}.attrs.mediaId"),
                "required",
                "must be the id of a media item",
            )),
        }
        for (key, max) in [("alt", 1000), ("caption", 2000)] {
            if let Some(value) = attrs.get(key)
                && !(value.is_null()
                    || value.as_str().is_some_and(|text| text.chars().count() <= max))
            {
                self.error(
                    FieldError::new(
                        format!("{path}.attrs.{key}"),
                        "invalid",
                        "must be text of at most {max} characters",
                    )
                    .with_param("max", max),
                );
            }
        }
        if let Some(caption) = attrs.get("caption").and_then(Value::as_str) {
            self.info.plain_text.push_str(caption);
            self.info.plain_text.push('\n');
        }
    }

    fn embed(&mut self, node: &Map<String, Value>, path: &str) {
        self.keys(node, path, &["type", "attrs"]);
        let attrs = self.attrs(node, path, &["url"]).unwrap_or_default();
        let url = attrs.get("url").and_then(Value::as_str).unwrap_or_default();
        // Trình duyệt coi `\` như `/` trong địa chỉ `https:` (chuẩn WHATWG URL):
        // `https://evil.com\.youtube.com/x` mở `evil.com`. Có `\`, khoảng trắng hay ký tự
        // điều khiển là từ chối, trước khi tách host.
        let clean = !url.chars().any(|c| c == '\\' || c.is_whitespace() || c.is_control());
        let host = clean
            .then_some(url)
            .and_then(|url| url.strip_prefix("https://"))
            .and_then(|rest| rest.split(['/', '?', '#']).next())
            .map(|host| host.trim_start_matches("www.").to_ascii_lowercase());
        let allowed = host.as_deref().is_some_and(|host| {
            !host.contains('@')
                && !host.contains(':')
                && self
                    .embed_hosts
                    .iter()
                    .any(|allowed| host == allowed || host.ends_with(&format!(".{allowed}")))
        });
        if !allowed {
            self.error(
                FieldError::new(
                    format!("{path}.attrs.url"),
                    "host_not_allowed",
                    "must be an `https://` address on one of: {hosts}",
                )
                .with_param("hosts", self.embed_hosts.join(", ")),
            );
        }
    }
}

/// Link an toàn: `http`, `https`, `mailto`, hoặc đường dẫn trong site (`/…`, không `//`).
fn safe_link(href: &str) -> bool {
    let lower = href.trim().to_ascii_lowercase();
    // `\` thành `/` trong trình duyệt: `/\evil.com` là `//evil.com` — link ra ngoài trá hình
    // đường dẫn trong site.
    if href.chars().any(|c| c.is_control() || c == '\\') || href.len() > 2048 {
        return false;
    }
    lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("mailto:")
        || (lower.starts_with('/') && !lower.starts_with("//"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn hosts() -> Vec<String> {
        vec!["youtube.com".to_owned()]
    }

    fn fields(body: Value) -> Vec<(String, String)> {
        check(&body, 100_000, &hosts())
            .unwrap_err()
            .into_iter()
            .map(|error| (error.field, error.code))
            .collect()
    }

    /// `\` là `/` với trình duyệt: không lọt kiểm host của embed hay link "trong site".
    #[test]
    fn dau_gach_nguoc_khong_lot() {
        for url in [
            "https://evil.com\\.youtube.com/x",
            "https://youtube.com\\@evil.com/x",
            "https://www.youtube.com/watch?v=a b",
        ] {
            let body =
                json!({"type": "doc", "content": [{"type": "embed", "attrs": {"url": url}}]});
            assert_eq!(
                fields(body),
                [("body.content[0].attrs.url".to_owned(), "host_not_allowed".to_owned())],
                "{url}"
            );
        }
        let ok = json!({"type": "doc", "content": [{"type": "embed", "attrs": {"url": "https://www.youtube.com/watch?v=abc"}}]});
        assert!(check(&ok, 100_000, &hosts()).is_ok());
        assert!(!safe_link("/\\evil.com"));
        assert!(safe_link("/blog/hello"));
    }

    #[test]
    fn than_bai_hop_le_trich_van_ban() {
        let image = Uuid::now_v7();
        let body = json!({"type": "doc", "content": [
            {"type": "heading", "attrs": {"level": 2}, "content": [{"type": "text", "text": "Đà Lạt"}]},
            {"type": "paragraph", "content": [
                {"type": "text", "text": "Xin chào ", "marks": [{"type": "bold"}]},
                {"type": "text", "text": "bạn", "marks": [{"type": "link", "attrs": {"href": "https://example.com"}}]},
                {"type": "hardBreak"}
            ]},
            {"type": "image", "attrs": {"mediaId": image.to_string(), "alt": "Hồ", "caption": "Hồ Xuân Hương"}},
            {"type": "bulletList", "content": [{"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "một"}]}]}]},
            {"type": "codeBlock", "attrs": {"language": "rust"}, "content": [{"type": "text", "text": "fn main() {}"}]},
            {"type": "table", "content": [{"type": "tableRow", "content": [{"type": "tableCell", "attrs": {"colspan": 1}, "content": [{"type": "paragraph"}]}]}]},
            {"type": "embed", "attrs": {"url": "https://www.youtube.com/watch?v=x"}},
            {"type": "horizontalRule"}
        ]});
        let info = check(&body, 100_000, &hosts()).unwrap();
        assert_eq!(info.media_ids.into_iter().collect::<Vec<_>>(), [image]);
        assert_eq!(stored_image_ids(&body.to_string()), [image]);
        assert!(stored_image_ids("{\"type\": \"doc\", \"content\": []}").is_empty());
        assert!(stored_image_ids("\"image\" but not JSON").is_empty());
        assert!(info.plain_text.starts_with("Đà Lạt\nXin chào bạn"), "{}", info.plain_text);
        assert!(info.plain_text.contains("Hồ Xuân Hương"));
        assert_eq!(info.word_count, 12);
        assert_eq!(reading_minutes(info.word_count, 238), 1);
        assert_eq!(reading_minutes(477, 238), 3);
        assert_eq!(reading_minutes(0, 238), 0);
        assert!(check(&empty(), 100, &hosts()).is_ok());
    }

    #[test]
    fn bao_moi_loi_mot_lan_kem_vi_tri() {
        let body = json!({"type": "doc", "content": [
            {"type": "paragraph", "content": [
                {"type": "text", "text": "x", "marks": [{"type": "link", "attrs": {"href": "javascript:alert(1)"}}]},
                {"type": "text", "text": "y", "marks": [{"type": "link", "attrs": {"href": "//evil.example"}}]}
            ]},
            {"type": "script", "content": []},
            {"type": "image", "attrs": {"src": "https://evil.example/x.png"}},
            {"type": "heading", "attrs": {"level": 1, "onclick": "x"}, "content": [{"type": "text", "text": "h"}]},
            {"type": "embed", "attrs": {"url": "https://evil.example/video"}},
            {"type": "paragraph", "style": "color:red"},
            {"type": "text", "text": "loose"}
        ]});
        let found = fields(body);
        let expected = [
            ("body.content[0].content[0].marks[0].attrs.href", "unsafe_link"),
            ("body.content[0].content[1].marks[0].attrs.href", "unsafe_link"),
            ("body.content[1].type", "not_allowed"),
            ("body.content[2].attrs.src", "not_allowed"),
            ("body.content[2].attrs.mediaId", "required"),
            ("body.content[3].attrs.onclick", "not_allowed"),
            ("body.content[3].attrs.level", "range"),
            ("body.content[4].attrs.url", "host_not_allowed"),
            ("body.content[5].style", "not_allowed"),
            ("body.content[6].type", "misplaced"),
        ];
        let expected: Vec<(String, String)> = expected
            .iter()
            .map(|(field, code)| ((*field).to_owned(), (*code).to_owned()))
            .collect();
        assert_eq!(found, expected);
    }

    #[test]
    fn qua_sau_qua_lon() {
        let mut deep = json!({"type": "paragraph"});
        for _ in 0..40 {
            deep = json!({"type": "blockquote", "content": [deep]});
        }
        let found = fields(json!({"type": "doc", "content": [deep]}));
        assert!(found.iter().any(|(_, code)| code == "too_deep"), "{found:?}");
        let big = json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "x".repeat(2000)}]}]});
        let error = check(&big, 1024, &hosts()).unwrap_err();
        assert_eq!(error[0].code, "too_large");
        assert_eq!(fields(json!({"type": "paragraph"}))[0].0, "body");
    }

    // Property test (đặc tả 31 B3): JSON bất kỳ (giới hạn độ sâu) và cây node gần đúng không làm
    // bộ kiểm panic; thân đã nhận thì kiểm lại vẫn nhận; `stored_image_ids` chỉ trả UUID có trong
    // thân.
    fn any_json() -> impl proptest::strategy::Strategy<Value = Value> {
        use proptest::prelude::*;
        let leaf = prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::Bool),
            any::<i32>().prop_map(Value::from),
            "\\PC{0,12}".prop_map(Value::String),
        ];
        leaf.prop_recursive(5, 64, 6, |inner| {
            prop_oneof![
                proptest::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
                proptest::collection::btree_map(
                    "type|content|text|attrs|marks|level|mediaId|[a-z]{1,6}",
                    inner,
                    0..6
                )
                .prop_map(|map| Value::Object(map.into_iter().collect())),
            ]
        })
    }

    /// Cây node gần với thân bài thật: tên node, thuộc tính và cấp lồng đúng phần lớn thời gian.
    fn near_document() -> impl proptest::strategy::Strategy<Value = Value> {
        use proptest::prelude::*;
        let text = "\\PC{0,20}".prop_map(|text| json!({"type": "text", "text": text}));
        let inline = prop_oneof![text, Just(json!({"type": "hardBreak"}))];
        let paragraph = proptest::collection::vec(inline, 0..4)
            .prop_map(|content| json!({"type": "paragraph", "content": content}));
        let image = (any::<u128>(), "\\PC{0,10}").prop_map(|(id, alt)| {
            json!({"type": "image", "attrs": {"mediaId": Uuid::from_u128(id).to_string(), "alt": alt}})
        });
        let heading = (1u8..7, "\\PC{1,10}").prop_map(|(level, text)| {
            json!({"type": "heading", "attrs": {"level": level}, "content": [{"type": "text", "text": text}]})
        });
        let block = prop_oneof![paragraph, image, heading, Just(json!({"type": "horizontalRule"}))];
        proptest::collection::vec(block, 0..8)
            .prop_map(|content| json!({"type": "doc", "content": content}))
    }

    fn all_strings(value: &Value, found: &mut Vec<String>) {
        match value {
            Value::String(text) => found.push(text.clone()),
            Value::Array(items) => items.iter().for_each(|item| all_strings(item, found)),
            Value::Object(map) => map.values().for_each(|item| all_strings(item, found)),
            _ => {}
        }
    }

    fn check_invariants(body: &Value) -> Result<(), proptest::test_runner::TestCaseError> {
        if check(body, 1 << 20, &[]).is_ok() {
            let stored = body.to_string();
            let again: Value = serde_json::from_str(&stored).unwrap();
            proptest::prop_assert!(
                check(&again, 1 << 20, &[]).is_ok(),
                "accepted once, refused after a round trip"
            );
            let mut strings = Vec::new();
            all_strings(body, &mut strings);
            for id in stored_image_ids(&stored) {
                proptest::prop_assert!(
                    strings.contains(&id.to_string()),
                    "{id} is not in the body"
                );
            }
        }
        Ok(())
    }

    proptest::proptest! {
        #[test]
        fn json_bat_ky_khong_panic(body in any_json()) {
            check_invariants(&body)?;
        }

        #[test]
        fn cay_gan_dung_giu_bat_bien(body in near_document()) {
            check_invariants(&body)?;
        }
    }
}
