# Phác thảo: module "nghỉ" — tắt chức năng nhưng vẫn giữ nghĩa vụ với dữ liệu

Trạng thái: **đề xuất, chờ chủ dự án duyệt** (bước 70, nối tiếp P1; D-20261121-module-selection
mục "Giới hạn đã biết"). Chưa có code.

## Vấn đề

Module tắt bằng `AUTOXIDE_MODULES` không được đăng ký, nên dữ liệu nó để lại trong database không
còn ai chăm:

| Việc                                   | Hiện nay với dữ liệu của module tắt            |
| -------------------------------------- | ---------------------------------------------- |
| Xuất dữ liệu cá nhân của một người     | bỏ sót                                         |
| Xoá dữ liệu cá nhân theo yêu cầu       | bỏ sót — **rủi ro pháp lý** (Luật số hai)      |
| Dọn theo thời hạn lưu (retention)      | không chạy: dữ liệu nằm lại quá hạn            |
| Xoá hẳn một khách                      | dòng của khách đó nằm lại                      |
| Dời / tách / gộp khách, `copy`, backup | bị từ chối (đã làm) — an toàn nhưng chặn việc  |
| Khách mới tạo trong lúc module tắt     | thiếu dữ liệu khởi đầu khi bật lại             |

Người cài tắt `newsletter` vì không gửi bản tin nữa vẫn đang giữ email của người đăng ký. Yêu cầu
xoá dữ liệu của họ phải được đáp ứng.

## Đề xuất

Thêm trạng thái thứ ba giữa "bật" và "tắt": **nghỉ** (dormant). Module nghỉ khi nó bị tắt bằng cấu
hình VÀ database có bảng của nó (`<module>_migrations` tồn tại). Module tắt mà database chưa từng
có bảng của nó thì vẫn "tắt hẳn" như hiện nay — bản cài mới không trả giá gì.

Module nghỉ **có**:

- migration (để schema theo kịp bản mới — điều kiện để mọi việc dưới đây chạy đúng);
- phần dữ liệu cá nhân (`StartContext::personal_data`), thời hạn lưu (`Module::retention`), bảng giữ
  lại và chính sách dời / tách / gộp (`retained_tables`, `relocation`, `control_plane_tables`);
- job dọn theo thời hạn lưu của chính nó, và chỉ job đó.

Module nghỉ **không có**: route, OpenAPI, `/llms.txt`, tool / resource / prompt MCP, lịch định kỳ
và job nghiệp vụ, subscription (không nhận event), permission mới cấp được, mục điều hướng, gieo dữ
liệu cho khách mới. Với người gọi API, nó vẫn là "tắt".

Cách làm: kernel đã gom từng thứ module khai qua `Modules`. Thêm vào mỗi module một cờ nội bộ
`Activity::{Active, Dormant}`; mỗi hàm gom của `Modules` (route, tool, job, subscription…) lọc theo
cờ thay vì thêm hàm mới vào trait `Module` — module không phải sửa gì. `Modules::select` trả cả
module tắt; binary hỏi database (đã kết nối) module nào có bảng rồi đặt cờ trước khi `start`.

Điểm cần quyết:

1. **`start` của module nghỉ.** Phần dữ liệu cá nhân được đăng ký trong `Module::start`, mà `start`
   `require` trait của module khác. Hai cách: (a) chạy `start` đầy đủ — khi đó module nó phụ thuộc
   cũng phải ít nhất là nghỉ, và module vẫn dựng service không ai gọi; (b) thêm
   `Module::start_dormant` mặc định gọi `start` — module nặng tự viết bản gọn. Đề xuất (a) trước,
   (b) khi đo được là cần.
2. **Tập module phụ thuộc trạng thái database.** Đây chính là điều D-20261121-module-selection đã
   tránh cho việc BẬT module. Ở đây chấp nhận được vì trạng thái nghỉ không đổi thứ người gọi API
   thấy; nhưng `settings` phải tách thành hai bước (đọc cấu hình → kết nối → đặt cờ), và lệnh không
   kết nối database (`config`, `openapi`) không biết module nào nghỉ.
3. **Migration của module nghỉ có chạy không.** Đề xuất: có (`migrate up` áp dụng cả migration của
   module nghỉ) — nếu không, xoá dữ liệu cá nhân chạy trên schema cũ và bật lại sau nhiều bản phải
   chạy dồn migration. Hệ quả: "tắt module" không còn nghĩa là "schema của nó đứng yên".
4. **Gỡ hẳn.** Thêm `autoxide modules purge <tên>` (xoá bảng, có xác nhận, ghi audit) để người cài
   thoát khỏi trạng thái nghỉ mà không phải bật module → `migrate down` → tắt.
5. **Khi nào hết nghỉ.** Không tự động: chỉ `purge`, hoặc bật lại module.

## Bỏ được gì sau khi có

Chốt chặn `tenant_move_modules_off` và việc `copy` / `backup` từ chối chạy: module nghỉ tham gia
đủ các việc đó. Cảnh báo ở log khởi động và `doctor` đổi thành thông tin ("module X đang nghỉ, giữ
N bảng có dữ liệu; gỡ hẳn bằng …").

## Ước lượng

Vừa–lớn: sửa `Modules` (khoảng hai chục hàm gom), `settings` / `runtime` (hai bước), `migrate`,
`doctor`, thêm lệnh `purge` (cần `main.rs`), test trên cả hai database cho từng việc trong bảng
đầu file (mỗi việc: module bật ghi dữ liệu → tắt → việc đó vẫn xử lý dữ liệu của nó). Không đổi
API công khai, không đổi trait `Module` nếu chọn 1(a).

## Phương án khác đã cân nhắc

- **Giữ như hiện nay, chỉ ghi tài liệu.** Rẻ nhất; để lại rủi ro pháp lý cho người cài.
- **Bắt người cài `purge` trước khi tắt.** An toàn nhưng biến "tắt thử" thành việc mất dữ liệu.
- **Lệnh riêng `autoxide privacy sweep --all-modules`** chạy xoá / xuất với mọi module một lần.
  Nhỏ hơn nhiều, nhưng yêu cầu xoá qua API vẫn bỏ sót cho tới khi ai đó chạy lệnh; có thể là bước
  đệm nếu phương án chính chưa được duyệt.
