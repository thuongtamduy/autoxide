## D-20261118-step53-escalation · `support`: chuyển khiếu nại qua `ComplaintIntake` (đích là phiếu), liên kết qua sổ đối tượng, câu trả lời mẫu — 2026-11-18 — agent quyết

Chủ đề: Thương mại

Bối cảnh: bước 53 mục 4 (gắn phiếu với bản ghi, câu trả lời mẫu có biến, chuyển phiếu thành khiếu nại người tiêu dùng hay
lên sàn, mức hài lòng). Đặc tả cho phép bước này thêm `ComplaintIntake` vào `compliance::api` nếu chưa có, và nói
`compliance` nhận `source_ref = "support.ticket:<id>"` duy nhất.

Quyết định:

1. **`compliance::api::ComplaintIntake::open(tenant, ComplaintFromTicket) -> CaseRef`.** Vụ việc `consumer_complaint` lấy
   **đích** là chính phiếu (`support.ticket:<id>`, tra qua sổ đối tượng — `support` đăng ký loại `support.ticket`), bên bị
   khiếu nại là nút của phiếu, kênh `support`. Gọi lại cùng phiếu trả vụ việc cũ (`created = false`) — không thêm cột
   `source_ref` vào bảng của `compliance`: đích đã là khoá tự nhiên. Dùng chung hàm tạo vụ việc và mail báo của
   `compliance` (đổi sang `pub(crate)`, không viết lại).
2. **`support` gọi `ComplaintIntake` NGOÀI transaction**, rồi ghi `case_id` và đóng phiếu `escalated` trong transaction của
   mình; lần gọi sau thấy `case_id` thì trả phiếu như đang có. Không cài `compliance` → 503 `support.complaints_unavailable`.
3. **Form công khai không có loại "khiếu nại"** trong `support`: khiếu nại của người mua gửi thẳng
   `POST /api/v1/compliance/public/complaints` (kênh có sẵn của `compliance`), nên không có đường nào tạo khiếu nại ở hai nơi.
4. **Lên sàn** (`to: platform`) chỉ bật cờ `escalated_to_platform`, phiếu vẫn mở; nhân sự của sàn (độ sâu `tenant`) vốn thấy
   mọi phiếu. Chuyển lần hai → 409 `support.already_escalated`.
5. **Liên kết** kiểm loại có trong sổ đối tượng và bản ghi tồn tại; sổ chưa có khái niệm "người gọi được xem bản ghi
   này", nên kiểm quyền dừng ở quyền trả lời phiếu của nhân viên.
6. **Câu trả lời mẫu** (`/macros`, bảng `support_macros`) thuộc nút và dùng được ở mọi nút bên dưới; nội dung theo ngôn
   ngữ, biến `{customer_name}`, `{ticket_number}`, `{subject}`; chọn ngôn ngữ của người mua, không có thì `en`, rồi ngôn
   ngữ đầu tiên. Sửa nội dung thì mất dấu duyệt trả lời tự động (mục 6 dùng).
7. **Mức hài lòng** 1–5 chỉ khi phiếu `solved` / `closed`; chấm lại thì thay.

Hệ quả: đặc tả bước 53 giữ nguyên tên `ComplaintIntake`; `compliance` có thêm một trait công bố.
