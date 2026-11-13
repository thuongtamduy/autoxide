# Chính sách bảo mật

[English](SECURITY.md) · **Tiếng Việt**

## Phiên bản được vá

Autoxide đang ở loạt `0.x`. Bản vá bảo mật chỉ đi vào **bản phát hành minor mới nhất** (vd
`0.11.x` khi `0.11` là bản mới nhất); các bản minor cũ hơn không được vá. Hãy cập nhật lên
bản mới nhất trước khi báo, và ghi trong báo cáo bạn đã thử trên phiên bản nào.
`CHANGELOG.md` liệt kê mọi bản phát hành.

| Phiên bản               | Được vá bảo mật |
| ----------------------- | --------------- |
| Bản `0.x` mới nhất      | Có              |
| Các bản cũ hơn          | Không           |

## Báo lỗ hổng

**Đừng mở issue, pull request hay discussion công khai cho một vấn đề bảo mật.**

Báo riêng qua GitHub: mở tab **Security** của repo và chọn **Report a vulnerability**
(tính năng private vulnerability reporting của GitHub). Chỉ người duy trì thấy báo cáo, và
nó cho phép chúng tôi cùng bạn làm bản sửa và bản thông báo (advisory) trước khi công khai
bất cứ điều gì.

Một báo cáo hữu ích có:

- phiên bản (hoặc commit) và database (SQLite hay PostgreSQL) bạn đã thử;
- phần cấu hình liên quan, đã bỏ secret;
- các bước hay request tái hiện vấn đề, và kẻ tấn công được gì;
- bạn có định công bố không, và khi nào.

Chỉ thử trên bản cài của chính bạn. Đừng truy cập, sửa hay xoá dữ liệu không phải của bạn,
và đừng chạy thử tải vào máy chủ bạn không vận hành.

## Chờ đợi điều gì

Autoxide do một nhóm nhỏ duy trì, nên đây là mục tiêu, không phải cam kết:

| Bước                                              | Mục tiêu                                   |
| ------------------------------------------------- | ------------------------------------------ |
| Xác nhận đã nhận báo cáo                          | trong 7 ngày                               |
| Báo bạn biết có nhận báo cáo không, và vì sao     | trong 14 ngày                              |
| Bản phát hành có bản sửa cho báo cáo được nhận    | sớm nhất có thể, theo mức nghiêm trọng     |
| Thông báo công khai                               | cùng bản phát hành có bản sửa, chậm nhất 90 ngày sau báo cáo |

Chúng tôi ghi công người báo trong advisory và changelog, trừ khi bạn không muốn. Không có
chương trình thưởng (bug bounty).

## Những gì không tính là lỗ hổng

Đây là hành vi đã biết hoặc nằm ngoài dự án, không phải lỗi bảo mật:

- **Cấu hình mà tài liệu đã cảnh báo**: `/metrics` mở ra internet sau khi bạn bật
  `AUTOXIDE_HTTP_METRICS`, `AUTOXIDE_HTTP_CORS_ORIGINS=*`, tắt rate limiting bằng `0`, đặt
  `AUTOXIDE_HTTP_CLIENT_IP_HEADER` mà không có proxy phía trước, để trống
  `AUTOXIDE_SECRET_KEY` hay `AUTOXIDE_AUTH_TOKEN_SECRET` khi chạy thật (máy chủ ghi cảnh báo
  vào log cho từng trường hợp).
- **Từ chối dịch vụ bằng lưu lượng**: dội request vào máy chủ. Hãy báo nếu *một* request
  nhỏ làm máy chủ dùng bộ nhớ, CPU hay ổ đĩa nhiều hơn hẳn mức đáng có.
- **Báo cáo của công cụ quét tự động** mà không có cách tái hiện tác hại thật, vd thiếu
  header trên response JSON hay lộ chuỗi phiên bản.
- **Lỗ hổng trong một thư viện phụ thuộc** mà Autoxide không chạm tới. `cargo deny` kiểm mọi
  bản phát hành với cơ sở dữ liệu RustSec; nếu bạn tìm ra đường đi tới đúng đoạn code có lỗ
  hổng thì đó *là* một báo cáo.
- **Vấn đề ở giao diện người dùng** hay client khác: chúng nằm ở repo riêng. Autoxide chỉ
  phục vụ API.
- **Việc cần tài khoản quản trị** để làm điều quản trị viên vốn được làm (vd quản trị viên
  đọc dữ liệu người dùng).
