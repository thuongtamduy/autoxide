//! Tách bài thành "mẩu" để dịch bằng AI, rồi ráp bản dịch về đúng cấu trúc gốc (D-109).
//!
//! Mô hình **không bao giờ** thấy JSON của thân bài, link hay ID ảnh: nó chỉ thấy chữ, với định
//! dạng trong đoạn đổi thành thẻ giữ chỗ có số — `<m1>chữ đậm</m1>`, `<br2/>`. Mỗi đoạn văn hay
//! tiêu đề là MỘT mẩu (không dịch rời từng mảnh chữ: thứ tự từ tiếng Việt và tiếng Anh khác
//! nhau — "Xin **chào** bạn" phải dịch như một câu, thẻ được dời theo). Lúc ráp lại, mark và
//! thuộc tính (URL của link…) lấy từ bản gốc theo số của thẻ. Bản dịch thiếu, thừa hay sai thẻ
//! thì bị từ chối — cấu trúc bài không thể hỏng vì mô hình.
//!
//! Không dịch: `codeBlock`, `embed`, ID ảnh. Có dịch: `alt` và `caption` của ảnh.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

/// Một mẩu cần dịch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Segment {
    /// `f.title`, `b3` (khối thứ 3 có chữ), `b5.alt`…
    pub id: String,
    pub text: String,
}

/// Chỗ đặt bản dịch của một mẩu trong thân bài.
#[derive(Debug, Clone)]
enum Slot {
    /// Nội dung inline của khối ở `pointer` (JSON Pointer), với mark / node của từng thẻ.
    Inline { pointer: String, tags: BTreeMap<u32, Tag> },
    /// Thuộc tính `attr` của node ở `pointer`.
    Attr { pointer: String, attr: &'static str },
}

/// Thứ một thẻ giữ chỗ đại diện.
#[derive(Debug, Clone, PartialEq)]
enum Tag {
    /// Đoạn chữ mang các mark này (`<mN>…</mN>`).
    Marks(Vec<Value>),
    /// Một node inline không phải chữ (`<brN/>`: `hardBreak`).
    Atom(Value),
}

/// Bài đã tách: các mẩu, và cách ráp lại.
#[derive(Debug, Clone)]
pub(crate) struct Split {
    pub segments: Vec<Segment>,
    slots: BTreeMap<String, Slot>,
}

/// Tách các trường chữ `fields` (`(tên, giá trị)`) và thân bài `body`.
pub(crate) fn split(fields: &[(&str, &str)], body: &Value) -> Split {
    let mut split = Split { segments: Vec::new(), slots: BTreeMap::new() };
    for (name, value) in fields {
        if !value.trim().is_empty() {
            split.segments.push(Segment { id: format!("f.{name}"), text: escape(value) });
        }
    }
    let mut counter = 0;
    walk(body, "", &mut counter, &mut split);
    split
}

fn walk(node: &Value, pointer: &str, counter: &mut u32, split: &mut Split) {
    let kind = node.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "codeBlock" | "embed" => return,
        "image" => {
            for attr in ["alt", "caption"] {
                let text = node.pointer(&format!("/attrs/{attr}")).and_then(Value::as_str);
                if let Some(text) = text.filter(|text| !text.trim().is_empty()) {
                    *counter += 1;
                    let id = format!("b{counter}.{attr}");
                    split.segments.push(Segment { id: id.clone(), text: escape(text) });
                    split.slots.insert(id, Slot::Attr { pointer: pointer.to_owned(), attr });
                }
            }
            return;
        }
        _ => {}
    }
    let children = node.get("content").and_then(Value::as_array);
    let inline = children.is_some_and(|children| {
        children.iter().any(|child| child.get("type") == Some(&json!("text")))
    });
    if inline {
        let (text, tags) = encode_inline(children.map(Vec::as_slice).unwrap_or_default());
        if !text.trim().is_empty() {
            *counter += 1;
            let id = format!("b{counter}");
            split.segments.push(Segment { id: id.clone(), text });
            split.slots.insert(id, Slot::Inline { pointer: pointer.to_owned(), tags });
        }
        return;
    }
    for (index, child) in children.into_iter().flatten().enumerate() {
        walk(child, &format!("{pointer}/content/{index}"), counter, split);
    }
}

/// Nội dung inline thành chữ có thẻ.
fn encode_inline(children: &[Value]) -> (String, BTreeMap<u32, Tag>) {
    let mut text = String::new();
    let mut tags = BTreeMap::new();
    let mut next = 1;
    for child in children {
        match child.get("type").and_then(Value::as_str) {
            Some("text") => {
                let value = child.get("text").and_then(Value::as_str).unwrap_or("");
                let marks =
                    child.get("marks").and_then(Value::as_array).cloned().unwrap_or_default();
                if marks.is_empty() {
                    text.push_str(&escape(value));
                } else {
                    text.push_str(&format!("<m{next}>{}</m{next}>", escape(value)));
                    tags.insert(next, Tag::Marks(marks));
                    next += 1;
                }
            }
            _ => {
                text.push_str(&format!("<br{next}/>"));
                tags.insert(next, Tag::Atom(child.clone()));
                next += 1;
            }
        }
    }
    (text, tags)
}

/// Vì sao một bản dịch bị từ chối.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rejected(pub String);

impl Split {
    /// Kiểm bản dịch của các mẩu `ids` (một lô): đủ mọi `id`, không thừa, đúng thẻ, không rỗng.
    pub(crate) fn check(
        &self,
        ids: &[String],
        translated: &BTreeMap<String, String>,
    ) -> Result<(), Rejected> {
        for id in translated.keys() {
            if !ids.contains(id) {
                return Err(Rejected(format!("unexpected segment `{id}`")));
            }
        }
        for id in ids {
            let Some(text) = translated.get(id) else {
                return Err(Rejected(format!("segment `{id}` is missing")));
            };
            if text.trim().is_empty() {
                return Err(Rejected(format!("segment `{id}` is empty")));
            }
            if let Some(Slot::Inline { tags, .. }) = self.slots.get(id) {
                decode_inline(text, tags)
                    .map_err(|problem| Rejected(format!("segment `{id}`: {problem}")))?;
            } else if text.contains('<') && !text.contains("&lt;") && looks_like_tag(text) {
                return Err(Rejected(format!("segment `{id}` has tags it should not have")));
            }
        }
        Ok(())
    }

    /// Ráp bản dịch (đã [`check`](Self::check)) vào bản sao của `body`; trả thân bài mới và các
    /// trường đã dịch (`tên → chữ`).
    pub(crate) fn assemble(
        &self,
        body: &Value,
        translated: &BTreeMap<String, String>,
    ) -> Result<(Value, BTreeMap<String, String>), Rejected> {
        let mut body = body.clone();
        let mut fields = BTreeMap::new();
        for (id, text) in translated {
            if let Some(name) = id.strip_prefix("f.") {
                fields.insert(name.to_owned(), unescape(text));
                continue;
            }
            match self.slots.get(id) {
                Some(Slot::Inline { pointer, tags }) => {
                    let content = decode_inline(text, tags).map_err(Rejected)?;
                    let node = body.pointer_mut(pointer).and_then(Value::as_object_mut);
                    let Some(node) = node else {
                        return Err(Rejected(format!("no block at `{pointer}`")));
                    };
                    node.insert("content".to_owned(), Value::Array(content));
                }
                Some(Slot::Attr { pointer, attr }) => {
                    let attrs = body
                        .pointer_mut(pointer)
                        .and_then(Value::as_object_mut)
                        .and_then(|node| node.get_mut("attrs"))
                        .and_then(Value::as_object_mut);
                    let Some(attrs) = attrs else {
                        return Err(Rejected(format!("no image at `{pointer}`")));
                    };
                    attrs.insert((*attr).to_owned(), Value::String(unescape(text)));
                }
                None => return Err(Rejected(format!("unknown segment `{id}`"))),
            }
        }
        Ok((body, fields))
    }
}

/// Chữ có thẻ thành nội dung inline, theo `tags` của bản gốc. Mỗi thẻ đúng một lần, không lồng
/// nhau, không có thẻ lạ.
fn decode_inline(text: &str, tags: &BTreeMap<u32, Tag>) -> Result<Vec<Value>, String> {
    let mut content = Vec::new();
    let mut seen = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let Some(start) = rest.find('<') else {
            push_text(&mut content, rest, None);
            break;
        };
        push_text(&mut content, &rest[..start], None);
        rest = &rest[start..];
        let end = rest.find('>').ok_or("a tag is not closed")?;
        let tag = &rest[1..end];
        rest = &rest[end + 1..];
        if let Some(number) = tag.strip_prefix("br").and_then(|tag| tag.strip_suffix('/')) {
            let number: u32 = number.parse().map_err(|_| format!("unknown tag `<{tag}>`"))?;
            match tags.get(&number) {
                Some(Tag::Atom(node)) if !seen.contains(&number) => {
                    seen.push(number);
                    content.push(node.clone());
                }
                _ => return Err(format!("unexpected tag `<{tag}>`")),
            }
        } else if let Some(number) = tag.strip_prefix('m') {
            let number: u32 = number.parse().map_err(|_| format!("unknown tag `<{tag}>`"))?;
            let close = format!("</m{number}>");
            let inner_end = rest.find(&close).ok_or(format!("tag `<m{number}>` is not closed"))?;
            let inner = &rest[..inner_end];
            if inner.contains('<') {
                return Err(format!("tag `<m{number}>` contains another tag"));
            }
            if inner.is_empty() {
                // Thẻ rỗng sẽ làm mất node (vd một link): từ chối.
                return Err(format!("tag `<m{number}>` is empty"));
            }
            match tags.get(&number) {
                Some(Tag::Marks(marks)) if !seen.contains(&number) => {
                    seen.push(number);
                    push_text(&mut content, inner, Some(marks));
                }
                _ => return Err(format!("unexpected tag `<m{number}>`")),
            }
            rest = &rest[inner_end + close.len()..];
        } else {
            return Err(format!("unknown tag `<{tag}>`"));
        }
    }
    if seen.len() != tags.len() {
        let missing: Vec<String> =
            tags.keys().filter(|n| !seen.contains(n)).map(u32::to_string).collect();
        return Err(format!("tags {} are missing", missing.join(", ")));
    }
    Ok(content)
}

fn push_text(content: &mut Vec<Value>, text: &str, marks: Option<&Vec<Value>>) {
    let text = unescape(text);
    if text.is_empty() {
        return;
    }
    let mut node = Map::new();
    node.insert("type".to_owned(), json!("text"));
    node.insert("text".to_owned(), Value::String(text));
    if let Some(marks) = marks {
        node.insert("marks".to_owned(), Value::Array(marks.clone()));
    }
    content.push(Value::Object(node));
}

fn looks_like_tag(text: &str) -> bool {
    text.split('<')
        .skip(1)
        .any(|part| part.starts_with('m') || part.starts_with("/m") || part.starts_with("br"))
}

/// `<`, `>` và `&` trong chữ gốc thành thực thể, để không lẫn với thẻ.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body() -> Value {
        json!({"type": "doc", "content": [
            {"type": "heading", "attrs": {"level": 2}, "content": [{"type": "text", "text": "Đà Lạt"}]},
            {"type": "paragraph", "content": [
                {"type": "text", "text": "Xin "},
                {"type": "text", "text": "chào", "marks": [{"type": "bold"}]},
                {"type": "text", "text": " bạn, xem "},
                {"type": "text", "text": "bản đồ", "marks": [{"type": "link", "attrs": {"href": "https://example.com/map"}}]},
                {"type": "hardBreak"},
                {"type": "text", "text": "a < b & c"}
            ]},
            {"type": "codeBlock", "content": [{"type": "text", "text": "let x = 1;"}]},
            {"type": "image", "attrs": {"mediaId": "0190", "alt": "Hồ", "caption": ""}},
            {"type": "bulletList", "content": [{"type": "listItem", "content": [
                {"type": "paragraph", "content": [{"type": "text", "text": "Một"}]}
            ]}]}
        ]})
    }

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(id, text)| ((*id).to_owned(), (*text).to_owned())).collect()
    }

    #[test]
    fn tach_mau_co_the_giu_cho() {
        let split = split(&[("title", "Đà Lạt"), ("excerpt", "")], &body());
        let segments: Vec<(&str, &str)> =
            split.segments.iter().map(|s| (s.id.as_str(), s.text.as_str())).collect();
        assert_eq!(
            segments,
            [
                ("f.title", "Đà Lạt"),
                ("b1", "Đà Lạt"),
                ("b2", "Xin <m1>chào</m1> bạn, xem <m2>bản đồ</m2><br3/>a &lt; b &amp; c"),
                ("b3.alt", "Hồ"),
                ("b4", "Một"),
            ]
        );
    }

    #[test]
    fn rap_lai_giu_link_va_doi_cho_the() {
        let original = body();
        let split = split(&[("title", "Đà Lạt")], &original);
        let translated = map(&[
            ("f.title", "Da Lat"),
            ("b1", "Da Lat"),
            ("b2", "<m1>Hello</m1> friend, see the <m2>map</m2><br3/>a &lt; b &amp; c"),
            ("b3.alt", "Lake"),
            ("b4", "One"),
        ]);
        let ids: Vec<String> = split.segments.iter().map(|s| s.id.clone()).collect();
        split.check(&ids, &translated).unwrap();
        let (body, fields) = split.assemble(&original, &translated).unwrap();
        assert_eq!(fields["title"], "Da Lat");
        let paragraph = &body["content"][1]["content"];
        assert_eq!(
            paragraph[0],
            json!({"type": "text", "text": "Hello", "marks": [{"type": "bold"}]})
        );
        assert_eq!(paragraph[2]["marks"][0]["attrs"]["href"], "https://example.com/map");
        assert_eq!(paragraph[3], json!({"type": "hardBreak"}));
        assert_eq!(paragraph[4]["text"], "a < b & c");
        assert_eq!(body["content"][2], original["content"][2], "code is not translated");
        assert_eq!(
            body["content"][3]["attrs"],
            json!({"mediaId": "0190", "alt": "Lake", "caption": ""})
        );
        assert_eq!(body["content"][4]["content"][0]["content"][0]["content"][0]["text"], "One");
    }

    #[test]
    fn tu_choi_ban_dich_hong() {
        let split = split(&[], &body());
        let ids: Vec<String> = split.segments.iter().map(|s| s.id.clone()).collect();
        let good = |b2: &str| map(&[("b1", "x"), ("b2", b2), ("b3.alt", "x"), ("b4", "x")]);
        for (b2, problem) in [
            ("Hello friend <br3/>", "tags 1, 2 are missing"),
            ("<m1>a</m1><m1>b</m1><m2>c</m2><br3/>", "unexpected tag `<m1>`"),
            ("<m1>a <m2>b</m2></m1><br3/>", "contains another tag"),
            ("<m1>a</m1><m2>b</m2><br3/><img src=x>", "unknown tag"),
            ("<m1>a<m2>b</m2><br3/>", "not closed"),
            ("<m1></m1><m2>b</m2><br3/>", "is empty"),
        ] {
            let error = split.check(&ids, &good(b2)).unwrap_err();
            assert!(error.0.contains(problem), "{b2}: {error:?}");
        }
        let mut missing = good("<m1>a</m1><m2>b</m2><br3/>");
        missing.remove("b4");
        assert!(split.check(&ids, &missing).unwrap_err().0.contains("`b4` is missing"));
        let mut extra = good("<m1>a</m1><m2>b</m2><br3/>");
        extra.insert("b9".into(), "x".into());
        assert!(split.check(&ids, &extra).unwrap_err().0.contains("unexpected segment"));
        let mut empty = good("<m1>a</m1><m2>b</m2><br3/>");
        empty.insert("b1".into(), " ".into());
        assert!(split.check(&ids, &empty).unwrap_err().0.contains("empty"));
    }
}
