//! Migration của các bảng phụ trợ kernel định nghĩa mà module sở hữu, gộp thành MỘT lời gọi
//! (D-20261122-module-helpers).
//!
//! Bảng event (outbox) và bảng job lớn lên qua nhiều bước của lộ trình, nên một module có sẵn liệt
//! kê từng bước: `outbox_migration`, `outbox_actor_migration`, `jobs_migration`,
//! `tenancy::migration::adopt(…)`, `outbox_access_migration`. Module MỚI không có lịch sử đó; nó
//! gọi [`helper_tables`] và nhận đúng chuỗi ấy với tên cố định. Module có sẵn giữ nguyên danh sách
//! của mình — migration đã áp dụng thì không đổi tên.

use super::MigrationTrait;

/// Một bảng phụ trợ của kernel trong module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum HelperTable {
    /// `<module>_events`: transactional outbox sau `Outbox::record` (`StartContext::outbox`).
    Events,
    /// `<module>_jobs`: hàng đợi job sau `StartContext::jobs`, `Module::jobs` và
    /// `Module::schedules`.
    Jobs,
}

/// The migrations that create the kernel's helper tables `tables` for `module`, in their current
/// shape. Put the result in the module's migration list, after its own first migrations:
///
/// ```
/// use autoxide_kernel::migration::{HelperTable, helper_tables};
///
/// let migrations = helper_tables("bookmarks", &[HelperTable::Events, HelperTable::Jobs]);
/// assert_eq!(migrations.len(), 6);
/// ```
///
/// Each table has its own chain of migrations with fixed names (`k_events_…`, `k_jobs_…`), so a
/// module that needs another helper table later adds a second call at the end of its list with
/// only the new table. The order of `tables` does not matter; a table named twice counts once.
pub fn helper_tables(module: &'static str, tables: &[HelperTable]) -> Vec<Box<dyn MigrationTrait>> {
    use crate::tenancy::migration::adopt;
    let mut migrations: Vec<Box<dyn MigrationTrait>> = Vec::new();
    // Thứ tự cố định, không theo thứ tự của `tables`: cùng một module luôn ra cùng một danh sách.
    if tables.contains(&HelperTable::Events) {
        migrations.push(crate::event::outbox_migration("k_events_1_create", module));
        migrations.push(crate::event::outbox_actor_migration("k_events_2_actor", module));
        migrations.push(adopt("k_events_3_tenancy", vec![crate::event::tenant_table(module)]));
        migrations.push(crate::event::outbox_access_migration("k_events_4_access", module));
    }
    if tables.contains(&HelperTable::Jobs) {
        migrations.push(crate::job::jobs_migration("k_jobs_1_create", module));
        migrations.push(adopt("k_jobs_2_tenancy", vec![crate::job::tenant_table(module)]));
    }
    migrations
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(tables: &[HelperTable]) -> Vec<String> {
        helper_tables("demo", tables).iter().map(|migration| migration.name().to_owned()).collect()
    }

    #[test]
    fn ten_co_dinh_va_khong_phu_thuoc_thu_tu() {
        let both = names(&[HelperTable::Jobs, HelperTable::Events, HelperTable::Jobs]);
        assert_eq!(
            both,
            [
                "k_events_1_create",
                "k_events_2_actor",
                "k_events_3_tenancy",
                "k_events_4_access",
                "k_jobs_1_create",
                "k_jobs_2_tenancy"
            ]
        );
        // Thêm bảng job về sau = một lời gọi thứ hai: tên không trùng với lời gọi đầu.
        let later = [names(&[HelperTable::Events]), names(&[HelperTable::Jobs])].concat();
        assert_eq!(later, both);
        assert!(names(&[]).is_empty());
    }
}
