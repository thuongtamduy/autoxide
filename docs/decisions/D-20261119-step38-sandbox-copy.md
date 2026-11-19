## D-20261119-step38-sandbox-copy · Sandbox: sao cấu hình bằng `Module::sandbox_copy`, đẩy về thành bản nháp bằng `Module::promotable`, bảng nối của kernel — 2026-11-19 — agent quyết

Chủ đề: Đa khách, tổ chức, gói

Quyết định: Bước 38 mục sandbox (D-232), phần sao / đẩy (thứ tự làm 14), làm theo đặc tả 38 Luồng 4 với các điểm chốt sau.

1. **Hook của module** (cuối trait `Module`, có mặc định — D-038): `sandbox_copy() -> Option<Arc<dyn SandboxCopy>>` và
   `promotable() -> Vec<Promotable>`, gọi sau `start`. Kernel gom theo thứ tự đăng ký; loại đẩy được phải tên
   `<module>.<loại>` và khai một lần (sai → không khởi động).
2. **Sao không đi qua `Distributor`** (khác đặc tả): chưa module nào có bảng phân phối, và sao sandbox chỉ chạy một lần
   khi tạo. `SandboxCopy::copy(&CopyContext)` gọi thẳng service của module ở sandbox, với bên làm có mọi permission
   (chỉ ở sandbox — bản sao của chính khách, job do chủ khách tạo). Job `kernel.sandbox_copy` chạy nó sau khi gieo dữ
   liệu khách mới và TRƯỚC khi ghi `kernel.sandbox_created` (uỷ quyền vào sandbox chỉ có sau đó, nên nhân viên không
   thấy bản sao dở dang). Lỗi kiểm tra của một bản ghi (dữ liệu khách không hợp ở sandbox) thì bỏ qua bản ghi đó, ghi
   log; lỗi khác làm job thử lại.
3. **Nối bản ghi bằng bảng của kernel** thay cho cột `source_record_id` ở từng module: `kernel_sandbox_links` (tạo
   trong `m0038_create_sandbox_tables`, ở database của sandbox) giữ `(kind, bản ghi sandbox, bản gốc, bản đã đẩy, dấu
   vân tay lần đẩy)`. Module dùng `link_copy` / `copied` (chạy lại không tạo trùng, đổi tham chiếu cha – con theo bản
   gốc) và `promoted_target`. Không phải thêm migration vào module nào.
4. **Sao gì ở đợt này**: kernel — cài đặt khách KHÔNG bí mật (secret không bao giờ sang); `auth` — role (trừ `admin` đã
   gieo; không người dùng, membership, API key); `content` — trang (không bài viết, không trang lưu trữ; cây giữ nguyên,
   trang đang đăng thì đăng ở sandbox; không ảnh, không trường tự định nghĩa, không nối bản dịch); `notifications` —
   mẫu khách tự soạn đang dùng (sao rồi xuất bản ở sandbox). Module khác (cây tổ chức, catalog, promotions,
   automations, theme…) khai `sandbox_copy` / `promotable` của mình khi cần — không phải sửa kernel. Bản ghi của
   sandbox nằm ở nút gốc của sandbox (cây tổ chức chưa sao).
5. **Đẩy**: `POST /api/v1/kernel/sandboxes/{id}/promote {items: [{kind, id}]}` (tool MCP `kernel.promote_sandbox`, mục
   dạng `<kind>:<id>` vì tool MCP không nhận kiểu lồng), gọi ở khách gốc, `kernel.sandboxes.manage`, 1–100 mục; loại lạ
   → 422 (`tenancy.promote_kind_unknown` trong `errors`). Module ghi bằng service của chính nó với **quyền của người
   gọi** (không ai đẩy được thứ họ không tự ghi được bằng REST). Bên khách thật luôn nhận bản nháp: bản ghi sao từ
   khách thật sửa bản nháp của bản gốc (trang đang đăng giữ nguyên, mang `has_unpublished_changes`), bản ghi tạo mới
   thành bản nháp mới; nội dung bằng lần đẩy trước → `unchanged`; trùng slug → `conflict` cho mục đó. Event
   `kernel.sandbox_promoted` ở khách thật. `GET /api/v1/kernel/sandboxes/promotable` (tool `kernel.list_promotable`).
6. **Loại đẩy được ở đợt này**: `content.page`. Mẫu thông báo chỉ sao, chưa đẩy: mẫu định danh bằng (loại, kênh, ngôn
   ngữ), không có ID bản ghi ổn định để trả `target_id`; module `notifications` khai `promotable` khi có.

Lý do: dùng service của module (có event, audit, kiểm dữ liệu) thay vì chép bảng; bảng nối chung giữ module gọn và
không đổi schema của module nào; quyền của người gọi giữ luật "MCP / REST không làm được gì hơn permission".

Đã cân nhắc: sao qua `Distributor` với `Targets::Sandbox` như đặc tả (mỗi module phải thêm bảng phân phối và bộ áp chỉ
để chạy một lần); cột `source_record_id` ở từng bảng của module (migration ở nhiều module, nhánh song song dễ đụng số);
đẩy bằng bên làm có mọi permission (người chỉ có `kernel.sandboxes.manage` sẽ ghi được trang mà không có
`content.write`).
