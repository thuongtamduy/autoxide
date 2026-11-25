## D-20261125-dormant-modules · Chế độ "nghỉ": module tắt vẫn xuất / xoá dữ liệu cá nhân và đi theo vòng đời khách — 2026-11-25 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định: (bước 70 P9 mục 2, chủ dự án mở lại ở D-20261125-p9-reopened.)

1. **Hook mới `Module::dormant(&self, &mut DormantContext)`**, cài đặt mặc định không làm gì (D-038). Kernel gọi nó
   **thay cho `start`** cho mọi module có trong chương trình nhưng bị `AUTOXIDE_MODULES` tắt, sau khi mọi module bật đã
   khởi động. Module đăng ký ở đó đúng hai thứ: phần bản xuất dữ liệu cá nhân (`context.personal_data`) và bên nghe
   `privacy::USER_DELETED` (`context.on_user_deleted`) — cùng logic như khi bật, dựng không cần service.
   `DormantContext` cho `tenancy()`, `translations()`, `sync_log()` và `api::<T>()` (chỉ thứ kernel hay module ĐANG BẬT
   công bố). Module tắt vẫn không route, không OpenAPI lúc chạy, không tool MCP, không job, không nhận event nào khác,
   không công bố trait cho module khác.
2. **Chỉ chạy nơi module từng chạy**: kernel gọi phần xuất / bên nghe của module tắt chỉ khi database của khách có
   `<module>_migrations`; không có thì bỏ qua (bản xuất không có mục của nó, event tính là đã giao) — database mới
   không trả giá gì, tập module chạy không phụ thuộc trạng thái database.
3. **Vòng đời khách theo khai báo**: xoá hẳn khách, bản xuất dữ liệu của khách (D-328), dời / tách / gộp khách lấy bảng
   của module bật CỘNG module tắt (`retained_tables`, `control_plane_tables`, `relocation` của module tắt được gom như
   module bật). Trước khi chép (dời, tách, gộp — cả dry run), migration của **chính module tắt** được áp ở database
   nguồn và đích nếu nguồn đã có bảng của nó; không chạm database chưa từng có bảng đó. Bỏ chốt chặn
   `409 tenant_move_modules_off` (mã không còn phát ra).
4. **Gate**: test `module_tat_van_xuat_va_xoa_du_lieu_ca_nhan_nhu_khi_bat` (crate `app`, cả hai database) so tập module
   góp phần xuất / nghe `USER_DELETED` khi mọi module bật với khi chỉ module core bật — module quên cài `dormant` là
   đỏ. `Modules::user_deleted_receivers()` và `PersonalDataExport::modules()` công khai cho việc đó.
5. **Event của module tắt không được ghi**: phần xoá dùng chung nhận `Option<&Outbox>`, khi tắt là `None` — không ai
   giao event của module tắt, ghi vào thì về sau bật lại sẽ bắn webhook / audit cũ. Ba event bị bỏ:
   `content.schedule_canceled`, `content.author_changed`, `comments.comments_erased`; `conversations.recording_deleted`
   cũng không ghi. Bỏ là đúng vì bên nhận không lệch trạng thái: các event này là thông báo phụ (audit, webhook, tin
   nhắn) chứ không phải nguồn dữ liệu của module khác — trạng thái thật (lịch đã huỷ, tác giả đã gỡ, bình luận đã xoá,
   bản ghi đã xoá) nằm trong bảng của chính module, đúng như khi bật; audit vẫn có event `auth.user_deleted` gốc.
6. **Không xoá dở**: `files` tắt vẫn đọc cấu hình của nó (module tắt đọc cấu hình ở reader riêng, lỗi chỉ cảnh báo),
   tự mở kho và chia cho kernel (`DormantContext::share_storage`; kernel dùng khi không module bật nào công bố kho —
   xoá khách, bản xuất của khách cũng xoá / ghi được file) nên xoá tài khoản xoá cả nội dung file; `conversations` tắt
   xoá bản ghi âm qua kho đó và bộ nối thoại của module đang bật (`DormantContext::contributions`). Không mở được kho
   hay thiếu bộ nối thì bên nghe trả lỗi — event `USER_DELETED` được thử lại mãi (dispatcher không bỏ event này), dòng dữ liệu còn nguyên để
   lần sau xoá trọn; không bao giờ báo xong khi nội dung còn nằm lại.
7. **Còn chưa làm khi tắt** (mục con `[ ]` dưới P9 ở `docs/ROADMAP.md`, ghi ở `docs/MODULES.md`): dọn theo thời hạn
   lưu (là job của module), gieo dữ liệu cho khách mới, giao bù event khác; `copy` / `backup` vẫn từ chối khi còn dữ
   liệu của module tắt.

Căn cứ: Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15 (hiệu lực 01/01/2026) — quyền của chủ thể được truy cập, nhận bản sao
và yêu cầu xoá dữ liệu; Nghị định 13/2023/NĐ-CP (Điều 9 quyền của chủ thể dữ liệu, Điều 14–16 rút lại sự đồng ý, xoá,
cung cấp dữ liệu). Bên kiểm soát dữ liệu phải đáp ứng với mọi dữ liệu nó còn giữ, kể cả dữ liệu của tính năng đã tắt.
Số điều và ngày hiệu lực cần đối chiếu văn bản gốc / người có chuyên môn pháp lý.

Lý do: tắt module (bước 70 P1) bỏ module khỏi sổ, nên dữ liệu cá nhân nó còn giữ lọt khỏi bản xuất và lệnh xoá — vi
phạm nghĩa vụ trên — và dời khách bị chặn. Hook riêng nhỏ hơn và an toàn hơn việc chạy `start` của module tắt: `start`
`require` module khác (có thể cũng đang tắt) và công bố trait, khiến module bật thấy "module tắt" như đang có
(`.ok()` trả `Some`) — phá nghĩa của P1. Gom bảng theo khai báo thì xoá / dời khách không cần module viết thêm gì.

Đã cân nhắc: (a) chạy `start` đầy đủ cho module "nghỉ" (phác thảo `docs/research/dormant-modules.md`) — loại vì lý do
trên và vì tập module phải tính theo database đã kết nối; (b) lệnh riêng quét xuất / xoá mọi module — loại: yêu cầu xoá
qua API vẫn bỏ sót tới khi có người chạy lệnh; (c) chạy migration của module tắt ở mọi database lúc `migrate up` — loại:
database mới sẽ có bảng của module không dùng; chỉ áp khi dời khách và nguồn đã có bảng.

Thay: phần "Giới hạn đã biết" (xuất / xoá dữ liệu cá nhân, xoá khách, dời / tách / gộp khách) của D-20261121-module-selection
