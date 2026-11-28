## D-20261128-p17-design-left · Hai mục P17 cần thiết kế để lại: token thiết bị cho mã SMS, dọn bản ghi gắn nút không có — 2026-11-28 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: hai mục dưới đây của P17 không làm trong lượt dọn nợ bảo mật này; hướng thiết kế đã chọn ghi sẵn để phiên làm
không phải tìm lại.

1. **Token thiết bị cho đăng nhập bằng mã SMS** (rủi ro còn lại của D-20261126-otp-nist). Hướng: `PhoneCodes` thêm hàm
   có cài đặt mặc định (D-038) nhận `device: Option<&str>`; đăng nhập bằng số đúng thì `auth` trả `device_token` =
   `<id ngẫu nhiên>.<HMAC(SecretKey dẫn theo mục đích, tenant|số đã chuẩn hoá|id)>`, client gửi lại ở lần xin mã sau;
   `compliance` coi (số, id thiết bị hợp lệ) là "client quen" thay cho khối địa chỉ (CGNAT di động đổi địa chỉ), lưu id
   (không lưu token) ở bảng client quen hiện có bằng migration mới, thu hồi khi `auth.access_revoked`. Cần đổi body / response
   của endpoint đăng nhập bằng số (OpenAPI, SDK) — thay đổi thêm trường, không phá vỡ.
2. **Bản ghi đã gắn `org_id` không có trong cây từ trước PT-8** (D-20261125-org-must-exist). Hướng: `Module` thêm hàm
   mặc định rỗng khai các bảng có `org_id` của module (tên bảng, cột); kernel có lệnh `autoxide orgs orphans` (báo cáo
   theo khách, theo bảng) và `--reassign <nút>` (gắn về nút gốc hay nút chỉ định, trong transaction của từng module, kèm
   audit). Chạm mọi module có dữ liệu theo đơn vị nên làm một PR riêng, không trộn với sửa lỗi.

Lý do: cả hai đổi hợp đồng (trait công khai của `auth` / `Module`, endpoint) và chạm nhiều module đang có phiên khác sửa;
làm vội trong lượt dọn nợ thì dễ thiếu test hai database và kiểm độc lập. Không mục nào là lỗ hổng mở: (1) là chặn oan
người dùng thật sau CGNAT (đã có quản trị viên mở khoá), (2) là bản ghi cũ không mở được theo ID (không lộ dữ liệu).

Đã cân nhắc: dùng cookie thay token (API không dùng cookie); lệnh dọn đọc thẳng bảng của mọi module (phá luật module sở
hữu bảng).
