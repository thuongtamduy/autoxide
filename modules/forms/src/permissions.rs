//! Permission của module `forms`. Bài gửi chứa dữ liệu cá nhân: đọc cần `forms.read`, xuất
//! và xoá theo email (quyền của chủ dữ liệu) tách riêng thành `forms.export`.

use autoxide_kernel::access::Permission;

pub(crate) const FORMS_READ: Permission =
    Permission::new("forms.read", "Read forms and their submissions (personal data).");
pub(crate) const FORMS_MANAGE: Permission =
    Permission::new("forms.manage", "Create, change, open, close and delete forms.");
pub(crate) const FORMS_REVIEW: Permission = Permission::new(
    "forms.review",
    "Mark submissions read, archived, spam or not spam, and delete single submissions.",
);
pub(crate) const FORMS_EXPORT: Permission = Permission::new(
    "forms.export",
    "Export submissions as CSV, and export or delete every submission of an email address.",
);

pub(crate) fn all() -> Vec<Permission> {
    vec![FORMS_READ, FORMS_MANAGE, FORMS_REVIEW, FORMS_EXPORT]
}
