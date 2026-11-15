# Chỗ thiếu API khi dựng giao diện thật (`autoxide-web`)

File sống, cho team API. Mỗi lần dựng xong một màn của repo giao diện `autoxide-web`, agent giao diện thêm vào đây những
gì màn đó cần mà `docs/openapi.json` chưa có.

Luật của repo giao diện: gọi REST API công khai của core cho mọi thứ core đã có. Phần core chưa có API thì giao diện
dựng bằng **dữ liệu mẫu** (chủ dự án quyết ngày 15/11/2026, W-006 của `autoxide-web`):
- dữ liệu và hàm mẫu nằm gọn trong `apps/admin/src/sample/`, có kiểu theo đúng hình dạng dự kiến của API trong đặc tả bước;
- trên màn có nhãn "Dữ liệu mẫu · chờ API bước N";
- tắt mặc định ở production.

Khi team API làm xong một dòng, giao diện thay hàm mẫu bằng lời gọi thật. Nếu API thật khác hình dạng đã đoán, sửa
giao diện theo API, không sửa API theo dữ liệu mẫu.

File này khác các đề bài trước:
- `ui-api-gaps.md`, `ui-phase2-api.md`, `ui-phase3-api.md` đối chiếu **bản thiết kế** với lộ trình.
- File này ghi chỗ thiếu gặp khi **dựng thật** với core đang chạy.

Chỗ nào đã nằm trong lộ trình thì chỉ ghi số bước để biết giao diện đang chờ gì, không phải việc mới.

## Cách đọc

| Cột | Nghĩa |
|---|---|
| Màn | Màn của `autoxide-web` (đường dẫn) |
| Cần | Endpoint hay trường còn thiếu, và giao diện dùng nó vào việc gì |
| Bước | Bước của `docs/ROADMAP.md` đã có mục này; "mới" là chưa có ở đâu |
| Giao diện đang làm gì | Cách tạm thời trong lúc chờ |

Team API làm xong một dòng thì đánh dấu `[x]` ở đầu dòng; agent giao diện sẽ bỏ phần tạm và nối API thật.

## Khung quản trị (đã dựng 15/11/2026)

| | Màn | Cần | Bước | Giao diện đang làm gì |
|---|---|---|---|---|
| [ ] | Mọi màn: thanh bên, menu con, bảng lệnh | Sổ điều hướng: nhóm phân hệ, màn, permission, tính năng của gói, thứ tự; đã lọc theo người gọi | 51 (`Module::navigation`) | Danh sách viết ở `apps/admin/src/nav/nav.ts`, lọc theo `GET /kernel/modules` và `permissions` của `GET /auth/me`. Có API thì bỏ file này |
| [ ] | Mọi màn: số đếm trên menu | Số việc đang chờ cho từng mục menu, cùng nguồn với hộp việc | 51 (hộp việc) | Chưa hiện số |
| [ ] | Ctrl K | Tìm dữ liệu xuyên phân hệ (đơn, bài viết, người dùng…), tìm không dấu, theo quyền | 51 (`FinderSource`) | Chỉ tìm tên màn để "Đi tới" |
| [ ] | `/` Tổng quan | Hộp việc, số liệu tóm tắt, hoạt động gần đây | 51 | Chỉ lời chào và lối vào các phân hệ |
| [ ] | Góc trên bên trái | Khách hàng và nút tổ chức đang chọn, đổi phạm vi | 33, 34 | Hiện "Autoxide Quản trị", chưa đổi phạm vi được |
| [ ] | Menu tài khoản: giao diện sáng / tối | Lưu lựa chọn theo tài khoản để theo người dùng sang thiết bị khác. Nên gộp vào `/me/preferences` của bước 37 | mới (gộp vào 37) | Lưu cookie `ax_theme` trên từng trình duyệt |

## Tài khoản của tôi (`/account`)

| | Màn | Cần | Bước | Giao diện đang làm gì |
|---|---|---|---|---|
| [ ] | Hồ sơ | `PATCH /me` đổi tên hiển thị | 34 (đã ghi ở `ui-phase2-api.md`) | Chỉ hiện tên |
| [ ] | Hồ sơ | Ngôn ngữ ưa thích (thư và thông báo gửi chậm dùng ngôn ngữ này) | 37 (`/me/preferences`) | Ngôn ngữ giao diện lưu ở cookie |
| [ ] | Thông báo | Tuỳ chọn nhận thông báo theo loại và kênh | 37 (`/me/preferences`), 52 | Chưa dựng tab này |
| [ ] | Hồ sơ | "Nơi bạn làm việc": khách hàng và nhánh tổ chức bạn vào được, kèm cách vào (trực tiếp, uỷ quyền, vào để hỗ trợ) | 34 | Chưa dựng |
| [ ] | Đăng nhập và bảo mật | Tài khoản Google / OIDC đã liên kết: xem và gỡ | 40 | Chưa dựng |
| [ ] | Đăng nhập và bảo mật | `/me` trả `has_password`; endpoint để tài khoản chưa có mật khẩu (vd đăng nhập bằng passkey hay OIDC) tự đặt mật khẩu. `DELETE /me` bảo "đặt mật khẩu trước" nhưng chưa có cách đặt | mới | Luôn hiện "Đổi mật khẩu" |
| [ ] | Hồ sơ | Email đang chờ xác minh sau khi yêu cầu đổi (`pending_email`, hạn), và endpoint huỷ yêu cầu đổi | mới | Chỉ báo "đã gửi thư tới …" |
| [ ] | Phiên đăng nhập | Mỗi phiên trả cách đăng nhập (mật khẩu, mật khẩu + mã, passkey) và IP hay vị trí gần đúng (lưu theo luật dữ liệu cá nhân, có hạn) | mới | Chỉ hiện trình duyệt và hệ điều hành (giao diện đã chuyển `User-Agent` thật cho core) |
| [ ] | Đăng nhập và bảo mật | Passkey trả loại authenticator (AAGUID → tên như Touch ID, Windows Hello) | mới | Hiện tên người dùng tự đặt |
| [ ] | Mọi biểu mẫu có mật khẩu | `detail` của `auth.wrong_password` đang nhắc `POST /api/v1/auth/password-reset`: câu cho người gọi API, người dùng thường không hiểu. Viết câu cho người dùng; đưa hướng dẫn cho máy vào tài liệu OpenAPI | mới | Hiện nguyên câu của core |
