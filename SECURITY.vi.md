# Chính sách bảo mật

[English](SECURITY.md) · **Tiếng Việt**

## Phiên bản được vá

Autoxide là dự án cộng đồng do một nhóm nhỏ duy trì. Các thời hạn dưới đây là mục tiêu, **không
phải cam kết dịch vụ (SLA)**: không có hỗ trợ trả phí, không có thời gian phản hồi bảo đảm.

**Từ 1.0** (hỗ trợ dài hạn cho mỗi bản major):

- **Bản minor mới nhất** của bản major hiện tại (vd `1.5.x` khi `1.5` là bản mới nhất) nhận mọi bản
  sửa lỗi và vá bảo mật.
- **Bản minor trước đó** nhận vá bảo mật trong **3 tháng** sau khi bản minor kế tiếp ra (vd `1.4.x`
  tới ba tháng sau `1.5.0`), để bạn có thời gian nâng cấp.
- Khi một bản major mới ra, bản minor cuối của bản major trước nhận vá bảo mật trong **6 tháng** sau
  bản major mới.
- Các bản cũ hơn không được vá.

| Phiên bản                                         | Được vá bảo mật                    |
| ------------------------------------------------- | ---------------------------------- |
| Bản minor `1.x` mới nhất                          | Có                                 |
| Bản minor `1.x` trước đó                          | 3 tháng sau khi bản minor kế ra    |
| Bản minor cuối của bản major trước (sau `2.0`)    | 6 tháng sau khi bản major mới ra   |
| Các bản cũ hơn (và `0.x` khi đã có 1.0)           | Không                              |

**Trước 1.0**, khi Autoxide còn ở loạt `0.x`, bản vá bảo mật chỉ đi vào bản minor mới nhất; các bản
`0.x` cũ hơn không được vá.

Bản vá ra dưới dạng bản patch (`1.5.2` → `1.5.3`), không đổi gì khác, nên cài ngay được
([`docs/COMPATIBILITY.vi.md`](docs/COMPATIBILITY.vi.md) ghi bản patch được chứa gì). Hãy cập nhật lên
bản mới nhất của dòng bạn dùng trước khi báo, và ghi trong báo cáo bạn đã thử trên phiên bản nào.
`CHANGELOG.md` liệt kê mọi bản phát hành.

## Rà soát bảo mật

Trước bản 1.0, nhóm duy trì đã rà soát bảo mật toàn bộ mã nguồn vào tháng 11/2026. Đây là **rà soát
nội bộ có AI hỗ trợ, không phải kiểm toán của một bên thứ ba độc lập**. Phạm vi, cách làm, từng phát
hiện kèm mức độ và cách sửa, cùng phần chưa rà được ghi ở
[`docs/security/review-2026-11.md`](docs/security/review-2026-11.md) (tiếng Anh).

Mô hình mối đe doạ của core — các ranh giới tin cậy, control ở từng ranh giới, việc còn mở và trọng tâm đề xuất cho
người kiểm thử bên ngoài — nằm ở [`docs/THREAT_MODEL.vi.md`](docs/THREAT_MODEL.vi.md).

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

## Chính sách công bố

Chúng tôi công bố có phối hợp:

- Bản sửa làm trong fork riêng tư của GitHub security advisory; bạn được mời vào nếu muốn.
- Báo cáo được nhận mà ảnh hưởng một bản đã phát hành thì chúng tôi xin GitHub cấp mã CVE.
- Advisory, CVE và bản phát hành có bản sửa được công bố cùng lúc. Dòng changelog của bản sửa nằm ở mục
  **Security** của `CHANGELOG.md`.
- Sau 90 ngày mà chưa có bản sửa thì chúng tôi vẫn công bố advisory kèm cách giảm thiểu đã biết, trừ khi đã thống
  nhất với bạn một ngày muộn hơn (vd bản sửa cần thay đổi ở thư viện phụ thuộc).
- Lỗ hổng đã bị khai thác hay đã công khai thì công bố ngay khi có bản sửa hay cách giảm thiểu, không chờ.
- Xin đừng công bố chi tiết trước advisory. Nếu thấy chúng tôi không phản hồi, hãy nói trong báo cáo riêng trước khi
  công khai.

Bản sửa cho các dòng cũ còn được hỗ trợ (bảng ở trên) ra cùng lúc khi có thể.

## Kiểm một bản phát hành

Bản phát hành được ký không cần khoá dài hạn (Sigstore, qua GitHub Actions) và kèm chứng thực nguồn gốc build:
[`docs/VERIFYING.vi.md`](docs/VERIFYING.vi.md) hướng dẫn kiểm image Docker và file đính kèm trước khi chạy. Kiểm thất
bại là một báo cáo bảo mật.

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
