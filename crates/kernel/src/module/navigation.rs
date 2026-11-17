//! Sổ điều hướng (bước 51 P1, D-200): module khai nhóm phân hệ và mục menu của mình; kernel gom,
//! kiểm xung đột lúc khởi động và lọc theo khách, gói, permission của người gọi
//! (`GET /api/v1/kernel/navigation`). Kernel không có danh sách phân hệ viết cứng: nhóm `home` và
//! `settings` cũng chỉ là nhóm kernel tự khai như mọi module.
//!
//! Nhãn viết tiếng Anh ở chỗ khai; bản dịch tiếng Việt nằm ở `locales/vi.json` của module với khoá
//! `text:<câu tiếng Anh>`, như mô tả permission (D-264).

use crate::access::Permission;

/// Không gian của một nhóm: khách (trang quản trị của khách) hay nền tảng (nhân sự nền tảng).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Space {
    #[default]
    Tenant,
    /// Chỉ hiện cho nhân sự khách nền tảng, đang làm việc trong chính khách nền tảng.
    Platform,
}

impl Space {
    pub fn as_str(self) -> &'static str {
        match self {
            Space::Tenant => "tenant",
            Space::Platform => "platform",
        }
    }
}

/// Một nhóm phân hệ trên thanh bên. Hai module khai cùng `id` phải khai giống hệt nhau.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct NavGroup {
    /// `sales`, `content`… — do module khai, ổn định (giao diện dùng làm khoá).
    pub id: &'static str,
    /// Nhãn tiếng Anh (dịch qua `text:` của module).
    pub label: &'static str,
    /// Tên icon trung lập (`cart`, `file`…); giao diện tự ánh xạ.
    pub icon: &'static str,
    /// Thứ tự trên thanh phân hệ, như nhau ở mọi khách.
    pub order: u16,
    pub space: Space,
}

impl NavGroup {
    pub const fn new(
        id: &'static str,
        label: &'static str,
        icon: &'static str,
        order: u16,
    ) -> Self {
        NavGroup { id, label, icon, order, space: Space::Tenant }
    }

    /// Nhóm của không gian nền tảng.
    pub const fn platform(mut self) -> Self {
        self.space = Space::Platform;
        self
    }
}

/// Một mục menu (một màn) trong một nhóm.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct NavItem {
    /// `content.entries` — cũng là khoá màn hình của giao diện; duy nhất trong cả chương trình.
    pub id: &'static str,
    pub group: &'static str,
    /// Nhãn tiếng Anh của nhóm con trong hàng tab, khi có.
    pub section: Option<&'static str>,
    pub label: &'static str,
    /// Có MỘT trong các permission này (ở nút đang chọn) thì mục hiện; rỗng: mọi người đã đăng nhập.
    pub permissions: Vec<Permission>,
    /// Tính năng của gói (bước 35) mục cần: thiếu thì mục hiện ở trạng thái khoá, không ẩn.
    pub feature: Option<&'static str>,
    /// Nguồn số đếm của hộp việc (P2) để hiện cạnh tên mục.
    pub counter: Option<&'static str>,
    pub order: u16,
}

impl NavItem {
    pub fn new(id: &'static str, group: &'static str, label: &'static str, order: u16) -> Self {
        NavItem {
            id,
            group,
            section: None,
            label,
            permissions: Vec::new(),
            feature: None,
            counter: None,
            order,
        }
    }

    /// Hiện khi người gọi có một trong `permissions`.
    pub fn permissions(mut self, permissions: &[Permission]) -> Self {
        self.permissions = permissions.to_vec();
        self
    }

    pub fn feature(mut self, feature: &'static str) -> Self {
        self.feature = Some(feature);
        self
    }

    pub fn counter(mut self, counter: &'static str) -> Self {
        self.counter = Some(counter);
        self
    }

    pub fn section(mut self, section: &'static str) -> Self {
        self.section = Some(section);
        self
    }
}

/// Phần điều hướng một module khai qua `Module::navigation`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Navigation {
    pub groups: Vec<NavGroup>,
    pub items: Vec<NavItem>,
}

impl Navigation {
    pub fn group(mut self, group: NavGroup) -> Self {
        self.groups.push(group);
        self
    }

    pub fn item(mut self, item: NavItem) -> Self {
        self.items.push(item);
        self
    }
}

/// Sổ đã gom của mọi module: nhóm (không trùng) và mục kèm tên module khai.
#[derive(Debug, Clone, Default)]
pub(crate) struct NavigationCatalog {
    pub groups: Vec<NavGroup>,
    pub items: Vec<(&'static str, NavItem)>,
}

/// Gom phần khai của từng module (theo thứ tự đăng ký); lỗi khi hai module khai cùng nhóm khác
/// nhau, mục trùng `id`, hay mục chỉ tới nhóm không ai khai.
pub(crate) fn collect(
    parts: Vec<(&'static str, Navigation)>,
) -> Result<NavigationCatalog, Vec<String>> {
    let mut catalog = NavigationCatalog::default();
    let mut problems = Vec::new();
    for (_, navigation) in &parts {
        for group in &navigation.groups {
            match catalog.groups.iter().find(|known| known.id == group.id) {
                Some(known) if known != group => problems.push(format!(
                    "navigation group `{}` is declared twice with different label, icon, order or \
                     space; declare it the same way in every module",
                    group.id
                )),
                Some(_) => {}
                None => catalog.groups.push(*group),
            }
        }
    }
    for (module, navigation) in parts {
        for item in navigation.items {
            if !catalog.groups.iter().any(|group| group.id == item.group) {
                problems.push(format!(
                    "navigation item `{}` of module `{module}` is in group `{}`, which no module \
                     declares",
                    item.id, item.group
                ));
            }
            if catalog.items.iter().any(|(_, known)| known.id == item.id) {
                problems.push(format!("navigation item `{}` is declared twice", item.id));
            }
            catalog.items.push((module, item));
        }
    }
    catalog.groups.sort_by_key(|group| (group.order, group.id));
    catalog.items.sort_by_key(|(_, item)| (item.order, item.id));
    if problems.is_empty() { Ok(catalog) } else { Err(problems) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nhom_trung_phai_giong_het() {
        let sales = NavGroup::new("sales", "Sales", "cart", 20);
        let a = Navigation::default().group(sales).item(NavItem::new("a.list", "sales", "A", 1));
        let b = Navigation::default().group(sales).item(NavItem::new("b.list", "sales", "B", 2));
        let catalog = collect(vec![("a", a), ("b", b)]).unwrap();
        assert_eq!(catalog.groups.len(), 1);
        assert_eq!(catalog.items.len(), 2);

        let other = Navigation::default().group(NavGroup::new("sales", "Selling", "cart", 20));
        let problems =
            collect(vec![("a", Navigation::default().group(sales)), ("c", other)]).unwrap_err();
        assert!(problems[0].contains("declared twice"), "{problems:?}");
    }

    #[test]
    fn muc_phai_co_nhom_va_khong_trung() {
        let lost = Navigation::default().item(NavItem::new("x.list", "nowhere", "X", 1));
        assert!(collect(vec![("x", lost)]).unwrap_err()[0].contains("no module declares"));
        let group = NavGroup::new("g", "G", "home", 1);
        let twice = Navigation::default()
            .group(group)
            .item(NavItem::new("g.one", "g", "One", 1))
            .item(NavItem::new("g.one", "g", "Again", 2));
        assert!(collect(vec![("g", twice)]).unwrap_err()[0].contains("declared twice"));
    }
}
