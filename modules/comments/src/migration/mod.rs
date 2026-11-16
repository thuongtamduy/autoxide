//! Migration của module `comments`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm file mới
//! và một dòng vào cuối [`all`].

mod m0001_create_comments;
mod m0008_name_key_lookalikes;
mod m0009_questions;
mod m0010_add_format;
mod m0012_target_ids;

use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::tenancy::migration::TenantTable;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_comments::Migration),
        // Bảng `comments_events` (transactional outbox) và cột người gây ra event (người lạ khác
        // hệ thống, D-122), định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "comments"),
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "comments"),
        // Bảng `comments_jobs` (job queue): job dọn dẹp hằng ngày.
        autoxide_kernel::job::jobs_migration("m0004_create_jobs", "comments"),
        // Bảng `comments_throttle`: giới hạn theo IP băm, email, người dùng; báo cáo; mail xác
        // minh (D-116).
        autoxide_kernel::throttle::throttle_migration("m0005_create_throttle", "comments"),
        // Bảng `comments_antispam_nonces`: nonce của token form — gửi lại là trả kết quả cũ (D-119).
        autoxide_kernel::antispam::antispam_migration("m0006_create_antispam_nonces", "comments"),
        // Bảng `comments_tokens`: xác minh email của khách, huỷ theo dõi trả lời (D-117).
        autoxide_kernel::tokens::tokens_migration("m0007_create_tokens", "comments"),
        // Khoá tên so theo dạng "khung": tên giả chữ Latin bằng chữ Kirin / Hy Lạp (D-151).
        Box::new(m0008_name_key_lookalikes::Migration),
        // Hỏi đáp: nhãn tác giả, khoá tìm đúng dấu, số câu hỏi theo đích (D-202).
        Box::new(m0009_questions::Migration),
        // Định dạng của bài: chữ thường hay Markdown giới hạn (D-252).
        Box::new(m0010_add_format::Migration),
        // Bảng phụ trợ của kernel (event, job, throttle, token, nonce) mang `tenant_id` (D-253).
        autoxide_kernel::tenancy::migration::adopt(
            "m0011_tenancy_helpers",
            vec![
                autoxide_kernel::event::tenant_table("comments"),
                autoxide_kernel::job::tenant_table("comments"),
                autoxide_kernel::throttle::tenant_table("comments"),
                autoxide_kernel::tokens::tenant_table("comments"),
                autoxide_kernel::antispam::tenant_table("comments"),
            ],
        ),
        // ID của đích, để liệt kê theo trang (D-270).
        Box::new(m0012_target_ids::Migration),
        // Bình luận, đích, hồ sơ, cấm, báo cáo, theo dõi trả lời thuộc một khách (bước 33). Cùng
        // đích, cùng tên hiển thị ở hai khách không đụng nhau. Chỉ việc xoá dấu vết IP quét cả
        // database (index `_sweep_idx`); bình luận hết hạn dọn theo từng khách.
        autoxide_kernel::tenancy::migration::adopt(
            "m0013_tenancy",
            vec![
                TenantTable::new("comments_comments")
                    .index(
                        "comments_comments_thread_idx",
                        ["tenant_id", "target_type", "target_id", "depth", "status", "id"],
                    )
                    .index(
                        "comments_comments_replies_idx",
                        ["tenant_id", "root_id", "status", "id"],
                    )
                    .index("comments_comments_queue_idx", ["tenant_id", "status", "id"])
                    .index("comments_comments_user_idx", ["tenant_id", "user_id", "status"])
                    .index("comments_comments_email_idx", ["tenant_id", "author_email"])
                    .index(
                        "comments_comments_hash_idx",
                        ["tenant_id", "target_type", "target_id", "content_hash", "created_at"],
                    )
                    .index("comments_comments_retention_idx", ["tenant_id", "status", "created_at"])
                    .drop_index("comments_comments_created_idx")
                    .sweep_index("comments_comments_created_sweep_idx", ["created_at"]),
                TenantTable::new("comments_targets")
                    .primary_key(["tenant_id", "target_type", "target_id"])
                    .unique("comments_targets_uid_key", ["tenant_id", "uid"]),
                TenantTable::new("comments_bans")
                    .index("comments_bans_value_idx", ["tenant_id", "kind", "value"])
                    .index("comments_bans_created_by_idx", ["tenant_id", "created_by"]),
                TenantTable::new("comments_reports")
                    .unique("comments_reports_unique", ["tenant_id", "comment_id", "reporter_key"])
                    .index("comments_reports_user_idx", ["tenant_id", "reporter_user_id"]),
                TenantTable::new("comments_reply_subscriptions")
                    .unique(
                        "comments_reply_subscriptions_unique",
                        ["tenant_id", "comment_id", "email"],
                    )
                    .index("comments_reply_subscriptions_email_idx", ["tenant_id", "email"]),
                TenantTable::new("comments_profiles")
                    .primary_key(["tenant_id", "user_id"])
                    .unique("comments_profiles_name_unique", ["tenant_id", "name_key"]),
            ],
        ),
        // Danh sách theo khách sắp theo `id` cần index `(tenant_id, id)` (đo ở bước 33 C, D-301).
        autoxide_kernel::tenancy::migration::list_index(
            "m0014_tenant_list_index",
            vec!["comments_comments", "comments_bans", "comments_reports"],
        ),
    ]
}
