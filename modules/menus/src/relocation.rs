//! Chính sách tách / gộp khách của các bảng của `menus` (bước 38, D-20261118-step38-relocation):
//! menu theo nút, mục menu theo menu.

use autoxide_kernel::tenancy::Relocation;

pub(crate) fn relocation() -> Relocation {
    Relocation::new().by_org(&["menus_menus"]).follow("menus_items", "menu_id", "menus_menus")
}
