# Đề bài: lấp chỗ trống API và đặc tả mà các màn hình đợt 2 lộ ra

Đề bài cho teammate làm phần backend (đặc tả trong `docs/steps/`, lộ trình, quyết định). Agent chính đã vẽ
xong đợt 2 của thiết kế quản trị và website (xem `docs/research/ui-traceability.md`, mục "Đợt 2"). Khi
vẽ, mỗi màn chỉ dùng thứ đặc tả đã có. Chỗ nào màn cần mà đặc tả chưa có thì màn vẫn vẽ theo hướng hợp lý
nhất, và chỗ đó được ghi vào đây.

Đề bài này **bổ sung** cho `docs/prompts/ui-trace-roadmap.md`, không lặp lại. Mục nào đã có ở đó (job
API, xuất danh sách, xác nhận lại danh tính, ngôn ngữ ưa thích, thống nhất tên tài khoản sổ cái, hạn
VietQR, mã số thuế 12 số, lọc bút toán theo đơn, nhận tại cửa hàng, COD trên sàn, sổ yêu cầu của chủ thể
dữ liệu, đếm người chấp nhận văn bản, lọc nhãn AI, cấu hình module theo khách, danh sách thuê bao tổng hợp,
đội database bằng job, khách tự thu hồi quyền hỗ trợ) thì làm theo đề bài đó.

## Cách làm

1. Với mỗi mục: đọc đặc tả của bước, quyết định thêm vào đặc tả, ghi vào phần "việc để sau" của lộ
   trình, hoặc bác bỏ (kèm lý do). Chọn cách đơn giản nhất mà giải quyết trọn vấn đề.
2. Quyết định có ảnh hưởng lâu dài thì ghi vào `docs/DECISIONS.md` bằng skill `record-decision`.
3. Giữ mọi luật trong `CLAUDE.md`, nhất là:
   - chạy trên cả SQLite và Postgres;
   - module sở hữu bảng của mình;
   - danh sách quy định hay đổi (lý do, ngưỡng, thời hạn) là dữ liệu có ngày hiệu lực;
   - căn cứ pháp luật chưa chắc thì ghi "cần đối chiếu văn bản gốc".
4. Không viết code ở đề bài này; chỉ đặc tả, lộ trình và quyết định.

Ký hiệu loại giống đề bài trước: **thiếu** là không có ở đâu; **hở** là đặc tả dùng mà chưa định nghĩa;
**lệch** là hai chỗ mâu thuẫn nhau.

## Việc theo bước

### Kernel, tổ chức, danh tính (33, 34, 40, 52)

| Mục | Bước | Loại | Nội dung | Màn |
|---|---|---|---|---|
| Gắn tên miền với nút | 33, 34 | thiếu | Bảng tên miền có `org_id` nhưng không có endpoint ghi. Thêm `PATCH /kernel/domains/{id} {org_id}` (quyền quản trị khách). | Cài đặt › Tên miền |
| Lọc audit theo loại người làm | 52 | hở | `actor_kind` hiện chỉ có `user` / `system`. Cần phân biệt người, AI agent (API key loại agent), dịch vụ (API key loại dịch vụ) và nhân sự hỗ trợ của nền tảng, kèm tham số lọc `actor_kind=`. | Cài đặt › Nhật ký audit |
| Ghi hồ sơ công khai của nút | 34 | thiếu | Đặc tả 34 có `GET /organizations/public/stores` nhưng chưa có endpoint sửa tên hiển thị, địa chỉ, giờ mở cửa, ảnh. | Tổ chức › Hồ sơ công khai |
| Tự đổi tên hiển thị | 34 | thiếu | Chưa có `PATCH /me {display_name}`. | Tài khoản của tôi › Hồ sơ |
| Gỡ danh tính OIDC đã liên kết | 40 | thiếu | `DELETE /auth/me/identities/{id}`; chặn khi đó là cách đăng nhập duy nhất còn lại. | Tài khoản của tôi › Đăng nhập |
| Tên setting của kết nối đối tác | 33, 36, 37, 44 | hở | Màn Kết nối liệt kê SMS, cổng thanh toán, hoá đơn điện tử, hãng vận chuyển. Mỗi bước tương ứng cần khai tên setting theo khách. Phần "kiểm tra kết nối" đã có ở đề bài trước. | Cài đặt › Kết nối đối tác |

### Tiền, sổ cái, thuế (36)

| Mục | Loại | Nội dung | Màn |
|---|---|---|---|
| Trạng thái niêm phong của bút toán | hở | API không cho biết bút toán đã vào chuỗi băm chưa. Thêm trường `sealed_at` (NULL là chưa) và số đếm theo kỳ (tổng, đã niêm phong). | Tài chính › Sổ cái |

### Sản phẩm, kho (41, 42)

| Mục | Bước | Loại | Nội dung | Màn |
|---|---|---|---|---|
| Trạng thái của lần nhập CSV | 41 | hở | `catalog_transfers.status` chưa có danh sách giá trị. Màn dùng: chờ chạy, đang chạy, xong, xong có dòng lỗi, bị từ chối. | Sản phẩm › Nhập CSV |
| Sản phẩm nháp chưa đủ điều kiện mở bán | 41 | thiếu | Bộ lọc hoặc số đếm sản phẩm thiếu thuộc tính bắt buộc hay giấy tờ ngành hàng. | Sản phẩm (số liệu đầu trang) |
| Lý do điều chỉnh tồn | 42 | hở | `reason_code` chưa có danh sách. Màn dùng: hư hỏng, mất, tìm thấy, sửa nhập sai, dùng làm mẫu. Nên là dữ liệu khách tự thêm được. | Kho › Sổ biến động |
| Ngưỡng "sắp hết hạn" của lô | 42 | hở | Trạng thái lô suy ra từ `expires_on`, nhưng số ngày coi là "sắp hết hạn" chưa có. Nên là cấu hình theo khách. | Kho › Lô và serial |
| Trạng thái ghi sổ của phiếu nhập | 42 | hở | Màn hiện "chờ ghi sổ" / "đã ghi sổ cái" cho phiếu nhập; đặc tả chưa có trường này. | Kho › Nhập hàng |
| Công nợ nhà cung cấp | 42, 36 | thiếu | Cột "đang nợ" cần số dư theo nhà cung cấp. Kho không đọc bảng sổ cái: cần trait hoặc endpoint của `ledger` trả số dư theo đối tượng. | Kho › Nhà cung cấp |

### Khuyến mại (43)

| Mục | Loại | Nội dung | Màn |
|---|---|---|---|
| Một chương trình, nhiều nơi thông báo | hở | `notice_status` là một cột. Chương trình chạy ở nhiều tỉnh có thể phải thông báo nhiều nơi: cần bảng thông báo theo nơi nhận. Cần đối chiếu NĐ 81/2018 sửa bởi NĐ 128/2024. | Khuyến mại › Thông báo Sở Công Thương |
| Kiểm trước khi bật | thiếu | Lỗi khi bật bị mất. Thêm `last_activation_error`, hoặc `POST /promotions/{id}/activate?dry_run=true` trả danh sách điều kiện chưa đạt (mức giảm tối đa, đã thông báo chưa…). | Khuyến mại › Chương trình |
| Chương trình không thời hạn | lệch | `ends_at` bắt buộc, nhưng miễn phí giao thường không có ngày kết thúc. Quyết định cho `ends_at` NULL với loại không cần thông báo, hoặc giữ bắt buộc. Màn hiện vẽ "tới 31/12/2026". | Tổng quan, Khuyến mại |
| Người mua lưu mã | thiếu | Website có nút "Lưu mã" để mã hiện sẵn ở bước thanh toán. Cần `GET/POST/DELETE /me/saved-promotions` (theo khách, tối đa vài chục mã). | Website › Khuyến mại |
| Nhắc khi flash sale bắt đầu | thiếu (thấp) | Nút "Nhắc tôi" gửi thông báo qua `notifications` trước giờ mở. Cần đồng ý nhận thông báo theo bước 37. | Website › Khuyến mại |

### Vận chuyển (44)

| Mục | Loại | Nội dung | Màn |
|---|---|---|---|
| Lịch giao | hở | `shipping_calendars` có ở D-201 nhưng chưa có cột và endpoint. Ngày nghỉ (ví dụ Tết) là dữ liệu nhập khi có thông báo chính thức. | Vận chuyển › Lịch giao |
| Giờ chốt của "giao trong ngày" | thiếu | Phương thức cần trường giờ chốt đơn (ví dụ 11:00) theo giờ Việt Nam. | Vận chuyển › Phí vận chuyển |
| Trạng thái tài khoản hãng | thiếu | "Đã kiểm tra" / "lỗi kết nối" và lần kiểm gần nhất. | Vận chuyển › Hãng vận chuyển |
| Khớp dòng lệch COD | thiếu | Lấy lại hành trình từ hãng rồi đánh dấu một dòng lệch là đã khớp, có lý do và audit. | Vận chuyển › Đối soát COD |
| Thời hạn giữ hàng chờ nhận | hở | Bổ sung cho mục nhận tại cửa hàng của đề bài trước: `hold_days` và mốc hết hạn giữ. | Cửa hàng › Nhận tại cửa hàng |

### Đơn hàng, cửa hàng, đánh giá (45)

| Mục | Loại | Nội dung | Màn |
|---|---|---|---|
| Đóng ca có chênh lệch | hở | Thiếu trường lý do chênh lệch. Chưa rõ chênh lệch tiền két có ghi bút toán không, vào tài khoản nào. | Cửa hàng › Ca thu ngân |
| Danh sách lý do | hở | Lý do ẩn đánh giá (bước 37 mới nói có "gói vn"), lý do từ chối trả hàng, giá trị của `allowed_return_reasons`. Cả ba là dữ liệu theo khách có bộ mặc định. | Cửa hàng › Đổi trả, Đánh giá |
| Đánh giá mới, chưa xem | thiếu | Không có trạng thái đã xem, nên không đếm được "4 đánh giá mới". | Cửa hàng › Đánh giá |
| Bản nháp tóm tắt đánh giá bằng AI | thiếu | D-201 nhắc ý nhưng chưa có bảng, endpoint và bước duyệt. Theo D-109: kết quả chỉ là bản nháp, người duyệt mới hiện. | Cửa hàng › Đánh giá |

### Khách hàng, kênh bán, tiếp thị liên kết (47, 48, 49)

| Mục | Bước | Loại | Nội dung | Màn |
|---|---|---|---|---|
| Che số điện thoại và email trong danh sách | 47 | hở | Màn đang che (`09•• ••• 412`). Quyết định có che không; nếu có nút "xem đủ" thì có ghi audit không. Cần đối chiếu Luật 91/2025 về bảo vệ dữ liệu cá nhân. | Khách hàng › Hồ sơ |
| Cơ sở tính điểm | 47 | hở | "Tiền net" đã gồm thuế chưa, có trừ phí giao không. Màn ghi: sau giảm giá, không gồm phí giao. | Khách hàng › Điểm và hạng |
| Trạng thái đơn kênh sau khi xử lý | 48 | hở | Không có trạng thái sau `POST /inbound-orders/{id}/resolve`, sau khi giữ đơn bán quá, hay sau khi kênh huỷ. | Kênh bán › Đơn cần xử lý |
| Chế độ phân bổ `reserved` | 48 | hở | Chưa rõ chỉ đẩy phần giữ riêng, hay đẩy thêm phần chia theo tỉ lệ. | Kênh bán › Phân bổ tồn |
| Ngày chuyển khoản khi đánh dấu đã chi | 49 | thiếu | `mark-paid` cần `paid_on` ngoài `bank_reference`. | Tiếp thị liên kết › Chi trả |

### Bình luận (22) và xác thực tài khoản đăng nội dung

| Mục | Loại | Nội dung | Màn |
|---|---|---|---|
| Chỉ tài khoản đã xác thực số điện thoại được bình luận | thiếu | `comments` chỉ có `ALLOW_GUESTS` và `SIGNED_IN_ONLY_TYPES`. Theo NĐ 147/2024 tài khoản đăng nội dung phải được xác thực; phạm vi áp cho bình luận ngoài mạng xã hội cần đối chiếu văn bản gốc. Module nghiệp vụ không phụ thuộc `auth` (D-046), nên cần cờ "đã xác thực SĐT" trên `Principal` hoặc qua trait. Lưu theo khách (mục cấu hình module theo khách của đề bài trước). | Tương tác › Bình luận; Tổng quan |

## Căn cứ cần đối chiếu (agent đã ghi trên màn, chưa chắc)

Các mục dưới đây không chặn công việc. Khi đặc tả dùng tới thì ghi vào quyết định, kèm dòng "cần đối chiếu
văn bản gốc / người có chuyên môn pháp lý":

- Ngưỡng khấu trừ thuế thu nhập cá nhân 10% với hoa hồng: 2.000.000 ₫ mỗi lần chi (TT 111/2013 Điều 25).
  Có nguồn nói đã nâng lên 5.000.000 ₫.
- Người tiếp thị là hộ kinh doanh: đặc tả 49 không khấu trừ mà ghi số hoá đơn họ xuất. Cần so với quy
  định khấu trừ thay hộ kinh doanh trên sàn (NĐ 117/2025).
- Phí thuê phần mềm (gói của nền tảng) có chịu thuế GTGT không.
- Thời hạn trả lời yêu cầu của chủ thể dữ liệu, hạn lưu chứng từ kế toán 10 năm (Luật Kế toán 88/2015
  Điều 41), hạn xử lý khiếu nại (Luật Bảo vệ quyền lợi người tiêu dùng 19/2023).
- Ký hiệu hoá đơn và thủ tục uỷ nhiệm lập hoá đơn (TT 78/2021); điều chỉnh, thay thế hoá đơn (NĐ 123/2020
  sửa bởi NĐ 70/2025).
- Mức giảm tối đa của khuyến mại, trường hợp phải thông báo, số ngày báo trước, miễn phí giao có phải
  thông báo không (NĐ 81/2018 sửa bởi NĐ 128/2024).
- Hình thức nhãn nội dung do AI tạo (luật mới, còn chờ nghị định hướng dẫn).

## Việc thiết kế còn lại (agent chính làm, ghi để teammate biết)

- Báo cáo theo kênh bán (`GET /reports/channels`, bước 48) chưa có màn.
- Trang công khai của phiên livestream (`GET /public/live-sessions/{id}`, bước 49) chưa có màn phía người
  mua.

## Báo cáo khi xong

Báo cáo bằng tiếng Việt, lời thường, cho chủ dự án mới học Rust:
- quyết định đã ghi (số và tên);
- mục nào đã vào đặc tả hay lộ trình, ở bước mấy; mục nào bác bỏ và vì sao;
- chỗ nào còn "cần đối chiếu văn bản gốc";
- gate có xanh không.

Commit trên nhánh làm việc, không push khi chủ dự án chưa yêu cầu.
