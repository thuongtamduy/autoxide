//! Mục menu: dữ liệu vào (cây), kiểm phần không cần hỏi module khác, dàn cây thành dòng để lưu
//! và dựng lại cây từ dòng đã lưu.
//!
//! Một mục có đúng một trong ba kiểu đích: địa chỉ (`url`), đối tượng của sổ đối tượng (`ref`),
//! hoặc không có đích (nhóm tiêu đề, vd một cột của bảng xổ). Kiểm đối tượng có thật nằm ở
//! `service.rs` (cần sổ đối tượng, gọi ngoài transaction).
use autoxide_kernel::time::Timestamp;
use uuid::Uuid;

use autoxide_kernel::error::FieldError;

use crate::entity::item;

/// Số mục tối đa của một menu (kể cả mục con).
pub(crate) const MAX_ITEMS: usize = 200;
/// Số tầng tối đa (kể cả nhóm con tự lấy).
pub(crate) const MAX_DEPTH: usize = 4;
/// Số mục con tự lấy tối đa của một mục, và số mặc định.
pub(crate) const MAX_AUTO_CHILDREN: i32 = 50;
pub(crate) const DEFAULT_AUTO_CHILDREN: i32 = 12;
/// Độ dài tối đa của nhãn mục và chữ của nhãn nổi bật (ký tự).
const MAX_LABEL: usize = 200;
const MAX_BADGE: usize = 20;

/// Bộ biểu tượng cố định: giao diện vẽ từng tên bằng bộ biểu tượng của nó.
pub(crate) const ICONS: &[&str] = &[
    "bag", "bolt", "book", "calendar", "cart", "category", "chat", "fire", "gift", "grid", "heart",
    "help", "home", "info", "mail", "map_pin", "news", "percent", "phone", "search", "shop",
    "sparkles", "star", "store", "tag", "truck", "user",
];
/// Cách làm nổi một mục (vd lối vào đặc biệt dồn phải, có màu riêng).
pub(crate) const HIGHLIGHTS: &[&str] = &["accent", "brand", "sale", "muted"];
/// Tông màu của nhãn nổi bật.
pub(crate) const BADGE_TONES: &[&str] = &["sale", "brand", "new", "neutral"];

/// Chỉ tới một đối tượng của sổ đối tượng.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RefInput {
    pub kind: String,
    pub id: String,
}

/// Nhãn nổi bật của mục; có hạn thì tự ẩn ngoài khoảng [`from`, `until`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BadgeInput {
    pub text: String,
    pub tone: String,
    pub from: Option<Timestamp>,
    pub until: Option<Timestamp>,
}

/// Một mục như người gọi gửi (cây).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ItemInput {
    /// Id của mục đã lưu, để giữ id qua lần lưu này; `None`: mục mới.
    pub id: Option<Uuid>,
    pub label: Option<String>,
    pub url: Option<String>,
    pub target: Option<RefInput>,
    pub auto_children: bool,
    pub auto_children_limit: Option<i32>,
    pub icon: Option<String>,
    pub highlight: Option<String>,
    pub badge: Option<BadgeInput>,
    pub feature: Option<RefInput>,
    pub children: Vec<ItemInput>,
}

/// Một dòng đã kiểm, kèm vị trí trong JSON (vd `items[0].children[1]`) để báo lỗi đúng chỗ.
pub(crate) struct Flat {
    pub row: item::Model,
    pub path: String,
    /// Id do người gọi gửi (giữ id của mục đã lưu).
    pub kept: bool,
}

/// Một mục kèm các mục con.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Node {
    pub item: item::Model,
    pub children: Vec<Node>,
}

/// Dàn cây thành danh sách (cha trước con, theo thứ tự), kiểm từng mục; lỗi ghi vào `problems`.
pub(crate) fn flatten(items: &[ItemInput], problems: &mut Vec<FieldError>) -> Vec<Flat> {
    let mut flat = Vec::new();
    walk(items, None, 1, "items", &mut flat, problems);
    let mut seen = std::collections::HashSet::new();
    for entry in flat.iter().filter(|entry| entry.kept) {
        if !seen.insert(entry.row.id) {
            problems.push(FieldError::new(
                format!("{}.id", entry.path),
                "duplicate",
                "is given to another item too",
            ));
        }
    }
    if flat.len() > MAX_ITEMS {
        problems.push(
            FieldError::new("items", "length", "at most {max_items} items")
                .with_param("max_items", MAX_ITEMS),
        );
    }
    for (position, entry) in flat.iter_mut().enumerate() {
        entry.row.position = i32::try_from(position).unwrap_or(i32::MAX);
    }
    flat
}

fn walk(
    items: &[ItemInput],
    parent: Option<Uuid>,
    depth: usize,
    path: &str,
    flat: &mut Vec<Flat>,
    problems: &mut Vec<FieldError>,
) {
    for (index, input) in items.iter().enumerate() {
        let here = format!("{path}[{index}]");
        if depth > MAX_DEPTH {
            problems.push(
                FieldError::new(here, "too_deep", "menus nest at most {max_depth} levels")
                    .with_param("max_depth", MAX_DEPTH),
            );
            continue;
        }
        let mut row = check(input, depth, &here, problems);
        row.parent_id = parent;
        if let Some(kept) = input.id {
            row.id = kept;
        }
        let id = row.id;
        flat.push(Flat { row, path: here.clone(), kept: input.id.is_some() });
        walk(&input.children, Some(id), depth + 1, &format!("{here}.children"), flat, problems);
    }
}

/// Kiểm một mục (không hỏi module khác) và dựng dòng để lưu.
fn check(
    input: &ItemInput,
    depth: usize,
    here: &str,
    problems: &mut Vec<FieldError>,
) -> item::Model {
    let label = input.label.as_deref().map(str::trim).unwrap_or_default();
    let url = input.url.as_deref().map(str::trim).filter(|url| !url.is_empty());
    let target = input.target.as_ref();
    if url.is_some() && target.is_some() {
        problems.push(FieldError::new(
            format!("{here}.url"),
            "invalid",
            "give at most one of `url` and `ref`",
        ));
    }
    // Nhãn: bắt buộc trừ khi mục trỏ tới đối tượng (khi đó bỏ trống là lấy tiêu đề của nó).
    if label.chars().count() > MAX_LABEL || (label.is_empty() && target.is_none()) {
        problems.push(
            FieldError::new(
                format!("{here}.label"),
                "length",
                "must be 1 to {max_label} characters (it may be empty when `ref` is given)",
            )
            .with_param("max_label", MAX_LABEL),
        );
    }
    if let Some(url) = url
        && !is_link(url)
    {
        problems.push(FieldError::new(
            format!("{here}.url"),
            "invalid",
            "must be `https://`, `http://`, `mailto:` or a path starting with a single `/`",
        ));
    }
    if let Some(target) = target {
        check_ref(target, &format!("{here}.ref"), problems);
    }
    if let Some(feature) = &input.feature {
        check_ref(feature, &format!("{here}.feature"), problems);
    }
    let auto_children_limit = if input.auto_children {
        if target.is_none() {
            problems.push(FieldError::new(
                format!("{here}.auto_children"),
                "invalid",
                "needs `ref`: sub-items are taken from the object it points to",
            ));
        }
        if depth >= MAX_DEPTH {
            problems.push(
                FieldError::new(
                    format!("{here}.auto_children"),
                    "too_deep",
                    "sub-items would go past {max_depth} levels",
                )
                .with_param("max_depth", MAX_DEPTH),
            );
        }
        let limit = input.auto_children_limit.unwrap_or(DEFAULT_AUTO_CHILDREN);
        if !(1..=MAX_AUTO_CHILDREN).contains(&limit) {
            problems.push(
                FieldError::new(
                    format!("{here}.auto_children_limit"),
                    "range",
                    "must be 1 to {max_auto_children}",
                )
                .with_param("max_auto_children", MAX_AUTO_CHILDREN),
            );
        }
        Some(limit)
    } else {
        if input.auto_children_limit.is_some() {
            problems.push(FieldError::new(
                format!("{here}.auto_children_limit"),
                "invalid",
                "only with `auto_children: true`",
            ));
        }
        None
    };
    let icon = input.icon.as_deref().map(str::trim).filter(|icon| !icon.is_empty());
    if let Some(icon) = icon
        && !ICONS.contains(&icon)
    {
        problems.push(
            FieldError::new(format!("{here}.icon"), "invalid", "must be one of: {allowed}")
                .with_param("allowed", ICONS.join(", ")),
        );
    }
    let highlight = input.highlight.as_deref().map(str::trim).filter(|value| !value.is_empty());
    if let Some(highlight) = highlight
        && !HIGHLIGHTS.contains(&highlight)
    {
        problems.push(
            FieldError::new(format!("{here}.highlight"), "invalid", "must be one of: {allowed}")
                .with_param("allowed", HIGHLIGHTS.join(", ")),
        );
    }
    let badge = input.badge.as_ref();
    if let Some(badge) = badge {
        let text = badge.text.trim().chars().count();
        if text == 0 || text > MAX_BADGE {
            problems.push(
                FieldError::new(
                    format!("{here}.badge.text"),
                    "length",
                    "must be 1 to {max_badge} characters",
                )
                .with_param("max_badge", MAX_BADGE),
            );
        }
        if !BADGE_TONES.contains(&badge.tone.as_str()) {
            problems.push(
                FieldError::new(
                    format!("{here}.badge.tone"),
                    "invalid",
                    "must be one of: {allowed}",
                )
                .with_param("allowed", BADGE_TONES.join(", ")),
            );
        }
        if let (Some(from), Some(until)) = (badge.from, badge.until)
            && until <= from
        {
            problems.push(FieldError::new(
                format!("{here}.badge.until"),
                "range",
                "must be after `from`",
            ));
        }
    }
    item::Model {
        id: Uuid::now_v7(),
        // Khách và menu: service điền lúc ghi.
        tenant_id: Uuid::nil(),
        menu_id: Uuid::nil(),
        parent_id: None,
        position: 0,
        label: label.to_owned(),
        url: url.map(str::to_owned),
        ref_kind: target.map(|target| target.kind.clone()),
        ref_id: target.map(|target| target.id.trim().to_owned()),
        auto_children: input.auto_children,
        auto_children_limit,
        icon: icon.map(str::to_owned),
        highlight: highlight.map(str::to_owned),
        badge_text: badge.map(|badge| badge.text.trim().to_owned()),
        badge_tone: badge.map(|badge| badge.tone.clone()),
        badge_from: badge.and_then(|badge| badge.from).map(micros),
        badge_until: badge.and_then(|badge| badge.until).map(micros),
        feature_kind: input.feature.as_ref().map(|feature| feature.kind.clone()),
        feature_id: input.feature.as_ref().map(|feature| feature.id.trim().to_owned()),
    }
}

/// Loại và ID có đúng dạng không (có thật hay không thì service hỏi sổ đối tượng).
fn check_ref(target: &RefInput, here: &str, problems: &mut Vec<FieldError>) {
    let kind = target.kind.as_str();
    let valid_kind = kind.len() <= 64
        && kind.split_once('.').is_some_and(|(module, name)| {
            [module, name].iter().all(|part| {
                !part.is_empty()
                    && part
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            })
        });
    if !valid_kind {
        problems.push(FieldError::new(
            format!("{here}.kind"),
            "invalid",
            "must be a resource type like `content.term`",
        ));
    }
    let id = target.id.trim();
    if id.is_empty() || id.len() > 64 {
        problems.push(FieldError::new(
            format!("{here}.id"),
            "length",
            "must be 1 to 64 characters",
        ));
    }
}

/// Đích link dùng được trong menu: `https://`, `http://`, `mailto:` hay đường dẫn trong site bắt
/// đầu bằng đúng một `/` (`//evil.com` là site khác với trình duyệt; `\` thành `/`, D-149).
pub(crate) fn is_link(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let scheme_ok = lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("mailto:")
        || (url.starts_with('/') && !url.starts_with("//"));
    scheme_ok
        && url.len() <= 2048
        && !url.chars().any(|c| c.is_whitespace() || c.is_control() || c == '\\')
}

/// Thời điểm cắt tới micro giây: hai database lưu cùng một giá trị.
pub(crate) fn micros(at: Timestamp) -> Timestamp {
    at.truncate(jiff::Unit::Microsecond)
}

/// Dựng cây từ danh sách mục đã theo thứ tự.
pub(crate) fn tree(items: Vec<item::Model>) -> Vec<Node> {
    fn children(parent: Option<Uuid>, items: &[item::Model]) -> Vec<Node> {
        items
            .iter()
            .filter(|item| item.parent_id == parent)
            .map(|item| Node { item: item.clone(), children: children(Some(item.id), items) })
            .collect()
    }
    children(None, &items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::SignedDuration;

    fn link(label: &str, url: &str) -> ItemInput {
        ItemInput { label: Some(label.into()), url: Some(url.into()), ..ItemInput::default() }
    }

    #[test]
    fn link_hop_le() {
        for good in ["https://a.vn", "HTTP://a.vn", "mailto:a@b.vn", "/", "/sale?q=1#top"] {
            assert!(is_link(good), "{good}");
        }
        for bad in ["//evil.com", "/\\evil.com", "javascript:alert(1)", "sale", "/a b", ""] {
            assert!(!is_link(bad), "{bad}");
        }
    }

    #[test]
    fn dan_cay_va_gom_moi_loi() {
        let deep = |child: ItemInput| ItemInput {
            label: Some("Group".into()),
            children: vec![child],
            ..ItemInput::default()
        };
        let items = vec![
            deep(deep(deep(link("Ok", "/a")))),
            deep(deep(deep(deep(link("Too deep", "/a"))))),
            ItemInput {
                label: None,
                url: Some("//evil.com".into()),
                auto_children: true,
                icon: Some("rocket".into()),
                badge: Some(BadgeInput {
                    text: String::new(),
                    tone: "red".into(),
                    from: Some(Timestamp::now()),
                    until: Some(Timestamp::now() - SignedDuration::from_hours(1)),
                }),
                ..ItemInput::default()
            },
        ];
        let mut problems = Vec::new();
        let flat = flatten(&items, &mut problems);
        let fields: Vec<&str> = problems.iter().map(|problem| problem.field.as_str()).collect();
        assert_eq!(
            fields,
            [
                "items[1].children[0].children[0].children[0].children[0]",
                "items[2].label",
                "items[2].url",
                "items[2].auto_children",
                "items[2].icon",
                "items[2].badge.text",
                "items[2].badge.tone",
                "items[2].badge.until",
            ]
        );
        // Cha trước con, đúng thứ tự, đúng cha.
        assert_eq!(flat[0].row.parent_id, None);
        assert_eq!(flat[1].row.parent_id, Some(flat[0].row.id));
        assert_eq!(flat[3].row.parent_id, Some(flat[2].row.id));
        assert!(flat.iter().enumerate().all(|(index, entry)| entry.row.position == index as i32));
        let rebuilt = tree(flat.iter().map(|entry| entry.row.clone()).collect());
        assert_eq!(rebuilt.len(), 3);
        assert_eq!(rebuilt[0].children[0].children[0].children[0].item.url.as_deref(), Some("/a"));
    }

    #[test]
    fn nhom_con_tu_lay_chi_cho_doi_tuong() {
        let target = RefInput { kind: "content.term".into(), id: "1".into() };
        let ok =
            ItemInput { target: Some(target.clone()), auto_children: true, ..ItemInput::default() };
        let mut problems = Vec::new();
        let flat = flatten(&[ok], &mut problems);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(flat[0].row.auto_children_limit, Some(DEFAULT_AUTO_CHILDREN));
        assert_eq!(flat[0].row.label, "", "label may be empty with a ref");

        let bad = vec![
            ItemInput {
                target: Some(target.clone()),
                auto_children: true,
                auto_children_limit: Some(51),
                ..ItemInput::default()
            },
            ItemInput {
                label: Some("x".into()),
                url: Some("/x".into()),
                target: Some(RefInput { kind: "Content".into(), id: String::new() }),
                auto_children_limit: Some(3),
                ..ItemInput::default()
            },
        ];
        let mut problems = Vec::new();
        flatten(&bad, &mut problems);
        let fields: Vec<&str> = problems.iter().map(|problem| problem.field.as_str()).collect();
        assert_eq!(
            fields,
            [
                "items[0].auto_children_limit",
                "items[1].url",
                "items[1].ref.kind",
                "items[1].ref.id",
                "items[1].auto_children_limit",
            ]
        );

        let many: Vec<ItemInput> = (0..=MAX_ITEMS).map(|_| link("x", "/x")).collect();
        let mut problems = Vec::new();
        flatten(&many, &mut problems);
        assert_eq!(problems.last().unwrap().field, "items");
    }
}
