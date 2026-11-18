## D-20261118-step41-kernel-prep · Ba API kernel cho module thương mại: `Subscription::optional`, `csv::read`, `slug` — 2026-11-18 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định: Commit chuẩn bị của bước 41 thêm ba thứ vào kernel để các bước 42–49 chạy song song không cùng sửa kernel:

1. `Subscription::optional(event, handler)` (`autoxide_kernel::event`): như `Subscription::new`, nhưng khi chưa module
   nào khai `event` thì `ModuleRegistry::dispatcher` chỉ ghi log `info` thay vì báo lỗi khởi động. Module đăng ký trước
   dùng nó để nghe module đăng ký sau có thể chưa cài (`catalog` nghe `inventory.cost_changed`, `inventory` nghe
   `orders.*`). Event của module đã cài vẫn giao như thường.
2. `autoxide_kernel::csv` thêm `write(header, rows)` và bộ đọc `read(text, max_rows) -> Result<_, CsvError>`: bỏ BOM,
   nhận CRLF lẫn LF, ô có hay không ngoặc kép, `""` trong ngoặc, xuống dòng trong ô; lỗi nêu số dòng của file; quá
   `max_rows` bản ghi thì dừng. Bộ đọc **tự viết** (khoảng 70 dòng) thay vì crate `csv`: dưới ngưỡng 150 dòng đặc tả 41
   đặt ra, và bộ ghi cùng file đã tự viết. Bản sao trong `forms` bị xoá, `forms` gọi bản của kernel.
3. `autoxide_kernel::slug` (`is_valid`, `from_title`, `with_suffix`, `MAX_LENGTH`) dời từ `content`; `content` và
   `catalog` dùng một quy tắc slug. Crate `slug` chuyển từ `content` sang `kernel`.

Lý do: ba bước 42, 43, 44 làm song song sau bước 41; nếu mỗi bước tự thêm thứ mình cần vào kernel thì ba nhánh cùng sửa
một file. `Subscription::optional` cần vì luật "module trước không `require` được module sau" (D-198) cũng áp cho event:
registry báo lỗi khi nhận event không ai khai, đúng cho lỗi gõ tên nhưng sai cho module chưa cài.

Đã cân nhắc: crate `csv` — thêm một thư viện cho việc vài chục dòng làm được; để mỗi module tự đọc CSV — ba bộ đọc khác
nhau cho cùng định dạng; cho `Subscription::new` bỏ qua mọi event lạ — mất lỗi khởi động khi gõ sai tên event.
