## D-20261125-p9-reopened · Mở lại P9 của bước 70: mẫu OTP theo khách, chế độ "nghỉ", danh sách có phân trang — 2026-11-25 — chủ dự án quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: ngày 25/11/2026 chủ dự án bảo mở lại và làm các mục ghi nợ của P9 (bước 70) đã hoãn ngày 22/11/2026,
theo thứ tự, mỗi mục một PR:

1. Mẫu câu SMS mang mã một lần khách tự sửa cho khớp template đã đăng ký với nhà mạng (D-20261125-sms-templates).
2. Chế độ "nghỉ": module tắt vẫn tham gia dữ liệu cá nhân và vòng đời khách (xuất, xoá, dời khách).
3. Endpoint GET trả `Data<Vec<T>>` không phân trang: endpoint mới trả `Page<T>`, endpoint cũ khai deprecation.

Mục "chạy thử năm bộ nối của P7 bằng tài khoản thử thật" vẫn chờ: cần tài khoản của chủ dự án. P9 không còn là "không
chặn 1.0 khi chưa mở lại": ba mục trên làm trước 1.0.

Lý do: chủ dự án quyết ngày 25/11/2026. Mục 2 là nghĩa vụ bảo vệ dữ liệu cá nhân (dữ liệu của module đang tắt không
được lọt khỏi bản xuất / lệnh xoá), mục 1 chặn khách dùng brandname thật, mục 3 là hợp đồng API phải chốt trước khi
cam kết 1.0 (D-20261123-compatibility-policy) — làm sau 1.0 thì endpoint cũ phải giữ suốt dòng 1.x.

Đã cân nhắc: giữ hoãn tới sau 1.0 — loại theo lời chủ dự án và vì ba lý do trên.
