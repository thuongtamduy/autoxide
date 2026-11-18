//! Chính sách tách / gộp khách của các bảng của `inventory` (bước 38, D-20261118-step38-relocation):
//! tồn, sổ biến động và chứng từ kho đi theo nút kho (`org_id`); bảng con đi theo dòng cha.

use autoxide_kernel::tenancy::Relocation;

pub(crate) fn relocation() -> Relocation {
    Relocation::new().by_org(&["inventory_levels", "inventory_movements", "inventory_adjustments"])
}
