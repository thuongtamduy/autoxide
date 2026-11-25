## D-20261125-event-org · Event mang tổ chức theo một cách thống nhất (`Delivered::org_id`); luật tự động hoá của nút con chỉ nhận event của nhánh mình — 2026-11-25 — agent quyết

Chủ đề: Database, dữ liệu, job, event

Quyết định: tổ chức (nút trong cây) của một event đọc theo đúng một cách, `autoxide_kernel::event::Delivered::org_id()`:
`org_id` ở đầu payload, không có thì `after.org_id`, không có nữa thì `before.org_id`. Module phát event về dữ liệu của
một đơn vị ghi `org_id` ở một trong các chỗ đó (ảnh chụp dòng `{"before", "after"}` có cột `org_id` là đủ —
`docs/MODULE_GUIDE.md` mục 6). Module `automations`: luật ở nút gốc của khách nhận mọi event của khách như cũ; luật ở
một nút khác chỉ kích hoạt khi `org_id` của event là chính nút đó hoặc nằm trong nhánh của nó (hỏi `OrgDirectory::is_within`
của module `organizations`); event không nêu nút thì không tới luật của nút con. Sửa L-8 của
`docs/security/review-2026-11.md`.

Lý do: luật chạy hành động bằng quyền của người tạo trong phạm vi nút của luật, nhưng trước đây nhận event của mọi nhánh
— người quản lý chi nhánh A đọc được dữ liệu đơn của chi nhánh B qua lần chạy, hay kích hoạt hành động trên chúng. Phần
lớn event đã mang `org_id` trong ảnh chụp `after` / `before`; quy ước đọc ba chỗ đó tránh phải đổi bảng outbox của mọi
module (mỗi module một bảng, đổi schema hàng chục bảng chỉ để thêm một cột đã có trong payload). Event không rõ nhánh
coi như ngoài nhánh — cùng nguyên tắc luồng event (SSE) đang dùng (chỉ quyền cả khách thấy event không rõ nhánh).

Đã cân nhắc: thêm cột `org_id` vào bảng outbox và tham số vào `Outbox::record` — đổi chữ ký công khai, migration ở mọi
module, mà giá trị vẫn phải lấy từ cùng chỗ; cho luật nút con nhận cả event không rõ nhánh — giữ nguyên lỗ hổng cho mọi
event chưa ghi `org_id`; chặn hẳn luật theo event ở nút con — mất tính năng cho chuỗi nhiều chi nhánh.

Hệ quả (rà 2026-11-25, đổi hành vi có chủ đích): event về dữ liệu thuộc một đơn vị nhưng CHƯA ghi `org_id` ở ba chỗ đọc
— luật của nút con gắn vào chúng ngừng chạy cho tới khi module bổ sung `org_id` (thêm vào ảnh chụp, hay đổi tên khoá như
`selling_org_id` / `from_org_id` / `warehouse_org_id` / `seller_org_id` / `respondent_org_id` sang `org_id`): `notes`, `menus`, `forms` and `organizations` (every event); `promotions` (every event: `org_id` sits under `after.promotion`); `content` (`entry_*`, `review_*`, `type_changed`); `media` (`item_*`); `bookings` (`booking_*`, `schedule_replaced`, `exception_*`); `conversations` (`message_*`, `typing`, `recording_*`); `support` (`message_added`, `message_hidden`, `ticket_linked`, `satisfaction_recorded`, `auto_repl*`, `tickets_claimed`); `catalog` (`variant_*`, `preorder_date_changed`, `price_effective`, `downloads_*`); `customers` (`profile_updated`, `profile_linked`, `contact_revealed`, `group_*`); `inventory` (`reservation_*`, `transfer_*`, `preorder_allocated`: the unit is under another key); `orders` (`payment_recorded`); `invoices` (`document_accepted`, `_rejected`, `_cancelled`, `_corrected`); `ledger` (`payment_refunded`); `shipping` (`cod_discrepancy_resolved`); `compliance` (`website_updated`, `website_status_changed`); `automations` (`run_failed`); `auth` (`api_key_revoked`, `api_key_approved`, `memberships_changed`, `user_roles_changed`). Luật đặt ở nút gốc vẫn chạy như cũ. Việc bổ sung `org_id` cho từng module là việc riêng (không làm trong
PR này, nhiều module đang có phiên khác sửa).
