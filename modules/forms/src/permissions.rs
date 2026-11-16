//! Permission của module `forms`. Bài gửi chứa dữ liệu cá nhân: đọc cần `forms.read`, xuất
//! và xoá theo email (quyền của chủ dữ liệu) tách riêng thành `forms.export`. Đổi địa chỉ nhận
//! mail báo (mail chứa câu trả lời) cần thêm `forms.read`. Gửi bài mà không qua lớp chống spam
//! cho người lạ cần `forms.submit` — chỉ đăng nhập thôi là chưa đủ.

// Theo nút (bước 34, D-208): đọc, quản lý, duyệt, xuất áp theo độ sâu — người của một cửa hàng chỉ
// thấy form của cửa hàng mình. Xuất / xoá theo email chạm mọi form nên cần độ sâu `tenant`.
use autoxide_kernel::access::{Permission, Risk};

pub(crate) const FORMS_READ: Permission =
    Permission::new("forms.read", "Read forms and their submissions (personal data).")
        .with_risk(Risk::SensitiveData)
        .scoped();
pub(crate) const FORMS_MANAGE: Permission = Permission::new(
    "forms.manage",
    "Create, change, open, close and delete forms. Setting `notify_emails` (the mails carry \
     the answers) also needs `forms.read`.",
)
.scoped();
pub(crate) const FORMS_REVIEW: Permission = Permission::new(
    "forms.review",
    "Mark submissions read, archived, spam or not spam, and delete single submissions.",
)
.scoped();
pub(crate) const FORMS_EXPORT: Permission = Permission::new(
    "forms.export",
    "Export submissions as CSV, and export or delete every submission of an email address.",
)
.with_risk(Risk::SensitiveData)
.scoped();

pub(crate) const FORMS_SUBMIT: Permission = Permission::new(
    "forms.submit",
    "Send form submissions without the anti-spam checks for anonymous visitors — for a website \
     server's own API key; still limited per account.",
);

pub(crate) fn all() -> Vec<Permission> {
    vec![FORMS_READ, FORMS_MANAGE, FORMS_REVIEW, FORMS_EXPORT, FORMS_SUBMIT]
}
