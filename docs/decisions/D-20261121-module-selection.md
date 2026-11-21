## D-20261121-module-selection · Bật / tắt module bằng `AUTOXIDE_MODULES`, ba hạng module — 2026-11-21 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định: (bước 70 P1, P3; hướng "core cho cộng đồng trước" là của chủ dự án,
D-20261121-community-first.) Một biến `AUTOXIDE_MODULES` chọn module nào chạy: danh sách cách
nhau bằng dấu phẩy gồm `core` (chỉ module core buộc phải có), `official` (mặc định), `all`, `tên`
(thêm), `-tên` (bỏ). Bộ lọc là `Modules::select(danh sách module, Selection)` của kernel — nhận
danh sách từ bên gọi, nên module viết ngoài repo đi qua đúng bộ lọc đó. Module tắt không được
đăng ký: không route, OpenAPI lúc chạy, `/llms.txt`, tool MCP, job, event, migration. Bảng đã có
thì không ai chạm.

Module tự khai ba điều, đều có mặc định (D-038): `Module::stability()` (`Official` mặc định /
`Experimental`), `Module::essential()` (core không chạy được nếu thiếu), `Module::depends_on()`
(module BUỘC phải có — mỗi `context.require::<…>()?`; thứ lấy bằng `.ok()` không khai).

- **Phụ thuộc: tự bật kèm**, không báo lỗi. `AUTOXIDE_MODULES=content` chạy luôn `media`, `files`,
  `ai`; sổ module và `doctor` ghi module nào được bật kèm vì ai. Lỗi chỉ khi người cài tự mâu
  thuẫn: `-catalog` trong khi `orders` còn bật (câu lỗi nêu đủ các module phải bỏ cùng), `-auth`
  (module core), tên gõ sai (câu lỗi liệt kê tên hợp lệ), danh sách chỉ có `-tên`.
- **Core** (không tắt được): `auth`, `idempotency`, và `mail` (vì `auth` buộc cần `dyn Mailer`).
- **Chính thức**: `ai`, `files`, `address`, `organizations`, `audit`, `webhooks`, `tax`, `ledger`,
  `invoices`, `compliance`, `notifications`, `plans`, `media`, `content`, `forms`, `menus`,
  `catalog`, `inventory`, `shipping`, `promotions`, `orders`, `customers`.
- **Thử nghiệm** (tắt mặc định): `bookings`, `newsletter`, `comments`, `support`, `conversations`,
  `analytics`, `reporting`, `automations`. Không module chính thức nào buộc cần chúng (`orders`,
  `shipping`, `catalog` chỉ dùng `support` / `comments` "nếu có"). Gate chặn module chính thức
  phụ thuộc module thử nghiệm.
- **Hợp đồng trong repo vẫn đầy đủ**: `autoxide openapi`, `autoxide mcp-manifest`, `.env.example`
  luôn dựng với mọi module, bỏ qua biến; chỉ bản phục vụ lúc chạy phản ánh module đang bật.
- **Người nâng cấp từ 0.17** (mọi module từng chạy): không tự bật theo dữ liệu, mà báo to. Khi
  database còn DÒNG trong bảng của một module đang tắt: `serve` ghi `warn` lúc khởi động và
  `doctor` báo `modules_turned_off`, kèm đúng dòng `AUTOXIDE_MODULES=…` cần thêm (người cài tự đặt
  biến thì chỉ là `info`). `copy`, `backup create`, `backup restore` từ chối chạy khi còn dữ liệu
  như vậy, vì chúng chỉ thấy bảng của module đang bật.
- Quickstart (`compose.quickstart.yml`) dùng mặc định `official`, ghi rõ trong file.
- Test của crate `app` đọc cấu hình bằng `settings::read` (không đặt biến thì MỌI module) để bài
  sẵn có không đổi; bản cài đọc bằng `settings::read_with` (không đặt thì `official`). Một bài
  test khởi động TỪNG module chỉ với phụ thuộc nó khai, nên quên khai là gate đỏ.

Lý do: Người cài một website không cần 33 module, và module chưa chín không nên là mặc định của
mọi bản cài. Lọc TRƯỚC khi đăng ký (thay vì đăng ký rồi chặn từng lớp) là cách duy nhất chắc chắn
không sót: mọi thứ của module — route, tool, job, event, migration, permission, bản dịch — đều đi
qua `Modules`. Tự bật phụ thuộc vì người cài nghĩ theo tính năng ("tôi cần `content`"), không theo
đồ thị phụ thuộc; bắt họ liệt kê `media,files,ai` chỉ thêm chỗ gõ sai, còn phụ thuộc của module
chính thức luôn là chính thức nên không ai bị bật ngầm thứ thử nghiệm. Với người nâng cấp, tự bật
module theo dữ liệu trong database nghe an toàn hơn nhưng buộc tập module phụ thuộc trạng thái
database (cấu hình phải đọc sau khi kết nối, hai máy chủ cùng cấu hình có thể chạy khác nhau, và
"có dữ liệu" không phân biệt được dữ liệu thật với dòng gieo sẵn); cảnh báo kèm đúng dòng cần thêm
thì tất định và người cài vẫn là người quyết.

Đã cân nhắc: (1) Feature flag của Cargo cho từng module — binary nhỏ hơn nhưng người cài phải tự
build, image phát hành không dùng được; để ngỏ cho sau. (2) Thiếu phụ thuộc thì báo lỗi "bật thêm
Y" — loại vì lý do trên. (3) Mặc định `all` để không đổi hành vi — trái mục tiêu của bước 70. (4)
Từ chối khởi động khi còn dữ liệu của module tắt — làm hỏng bản cài tự cập nhật vì một module có
thể họ không dùng. (5) Thêm hạng vào tài liệu OpenAPI của từng endpoint — để sau; hiện
`GET /api/v1/kernel/modules` trả `stability`.

Giới hạn đã biết: trong lúc một module tắt, xuất / xoá dữ liệu cá nhân, dọn theo thời hạn lưu, xoá
khách, dời / tách / gộp khách không chạm tới dữ liệu của nó; kiểm "còn dữ liệu" chỉ xem database
điều khiển, không xem từng placement. Ghi ở `docs/MODULES.md`.
