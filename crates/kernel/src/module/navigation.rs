//! The navigation registry (step 51 P1, D-200): a module declares its sub-area groups and menu items; the
//! kernel collects them, checks for conflicts at startup and filters by the caller's tenant, plan and
//! permissions (`GET /api/v1/kernel/navigation`). The kernel has no hard-coded list of sub-areas: the
//! `home` and `settings` groups are just groups the kernel declares itself like any module.
//!
//! Labels are written in English where declared; the Vietnamese translation lives in the module's
//! `locales/vi.json` under the key `text:<English sentence>`, like permission descriptions (D-264).

use crate::access::Permission;

/// The space a group belongs to: tenant (the tenant's admin area) or platform (platform staff).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Space {
    #[default]
    Tenant,
    /// Shown only to platform tenant staff, working inside the platform tenant itself.
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

/// A sub-area group in the sidebar. Two modules that declare the same `id` must declare it identically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct NavGroup {
    /// `sales`, `content`... - declared by the module, stable (the UI uses it as a key).
    pub id: &'static str,
    /// The English label (translated through the module's `text:`).
    pub label: &'static str,
    /// A neutral icon name (`cart`, `file`...); the UI maps it itself.
    pub icon: &'static str,
    /// The order in the sub-area bar, the same for every tenant.
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

    /// Groups of the platform space.
    pub const fn platform(mut self) -> Self {
        self.space = Space::Platform;
        self
    }
}

/// The place of an item on mobile (step 52, D-232): the UI draws the bottom bar from `Primary` items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MobilePlacement {
    /// The bottom bar; each answer keeps at most 5 items, and the remainder becomes `Secondary`.
    Primary,
    /// The "More" menu.
    #[default]
    Secondary,
    /// Not shown on mobile.
    Hidden,
}

impl MobilePlacement {
    pub fn as_str(self) -> &'static str {
        match self {
            MobilePlacement::Primary => "primary",
            MobilePlacement::Secondary => "secondary",
            MobilePlacement::Hidden => "hidden",
        }
    }
}

/// A mobile hint for an item: the place and the order within that place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MobileHint {
    pub placement: MobilePlacement,
    pub order: u16,
}

/// A menu item (one screen) within a group.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct NavItem {
    /// `content.entries` - also the UI's screen key; unique across the whole program.
    pub id: &'static str,
    pub group: &'static str,
    /// The English label of the sub-group in the tab row, when there is one.
    pub section: Option<&'static str>,
    pub label: &'static str,
    /// The item shows if the caller has ONE of these permissions (at the selected node); empty: anyone
    /// signed in.
    pub permissions: Vec<Permission>,
    /// The plan feature (step 35) the item needs: if missing, the item shows as locked, not hidden.
    pub feature: Option<&'static str>,
    /// The source of the work inbox count (P2) to show next to the item name.
    pub counter: Option<&'static str>,
    pub order: u16,
    /// The mobile hint; if not declared, `Secondary` with the item's `order`.
    pub mobile: MobileHint,
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
            mobile: MobileHint { placement: MobilePlacement::Secondary, order },
        }
    }

    /// Shown when the caller has one of `permissions`.
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

    /// The item's place on mobile and its order within that place.
    pub fn mobile(mut self, placement: MobilePlacement, order: u16) -> Self {
        self.mobile = MobileHint { placement, order };
        self
    }
}

/// The navigation part a module declares through `Module::navigation`.
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

/// The collected registry of every module: groups (without duplicates) and items with the declaring
/// module's name.
#[derive(Debug, Clone, Default)]
pub(crate) struct NavigationCatalog {
    pub groups: Vec<NavGroup>,
    pub items: Vec<(&'static str, NavItem)>,
}

/// Collects each module's declarations (in registration order); an error when two modules declare the same
/// group differently, an item has a duplicate `id`, or an item points to a group nobody declared.
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
