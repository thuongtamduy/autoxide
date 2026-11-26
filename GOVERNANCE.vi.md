# Quản trị dự án

[English](GOVERNANCE.md) · **Tiếng Việt**

Autoxide là dự án mã nguồn mở do một nhóm nhỏ người duy trì (maintainer) điều hành. Tài liệu này nói ai quyết định, quyết
định thế nào, và bạn thay đổi dự án bằng cách nào.

## Vai trò

- **Người dùng** dùng Autoxide và báo vấn đề. Không cần xin quyền.
- **Người đóng góp** gửi pull request, tài liệu, review hay ý tưởng module. Xem [`CONTRIBUTING.vi.md`](CONTRIBUTING.vi.md).
- **Người duy trì** review và merge, ra bản phát hành, chạy quy trình bảo mật và giữ dự án đi đúng lộ trình. Họ được liệt
  kê ở [`.github/CODEOWNERS`](.github/CODEOWNERS). Chủ dự án (người duy trì đầu tiên) có tiếng nói cuối cùng về hướng đi và
  về các mục ghi "chủ dự án quyết".

Người đóng góp thành người duy trì sau một thời gian dài có thay đổi và review tốt, khi một người duy trì đang có đề cử và
không người duy trì nào phản đối trong bảy ngày. Người duy trì không hoạt động sáu tháng chuyển sang "danh dự" (emeritus)
và quay lại được khi xin. Danh sách người duy trì là file CODEOWNERS; khi nó đổi thì `.github/dco-maintainers.txt` đổi trong
cùng pull request.

## Quyết định được đưa ra thế nào

1. **Thay đổi hằng ngày** (sửa lỗi, tính năng nhỏ trong lộ trình hiện tại, tài liệu) cần một người duy trì duyệt và CI
   xanh. Tác giả không tự duyệt thay đổi của mình.
2. **Thay đổi hạng hợp đồng ổn định** (API công khai của kernel, testkit, `api` của module, hợp đồng REST/MCP, cài đặt,
   migration) hay mã bảo mật, tiền, thuế cần review độc lập của người thứ hai và theo
   [`docs/COMPATIBILITY.vi.md`](docs/COMPATIBILITY.vi.md).
3. **Thay đổi lớn** (khả năng mới của core, đổi quy ước, đổi thư viện, thứ đắt để hoàn tác) đi qua quy trình RFC dưới đây
   trước khi viết code.
4. Quyết định theo **đồng thuận ngầm** (lazy consensus): đề xuất được thông qua khi không người duy trì nào phản đối trong
   thời hạn review (bảy ngày cho RFC). Nếu người duy trì bất đồng và không dàn xếp được, chủ dự án quyết và ghi lý do.
5. Quyết định lâu dài của dự án ghi ở `docs/decisions/` (mỗi quyết định một file) và không bị đảo ngầm; đảo là một quyết định
   mới nêu rõ quyết định nó thay.

## Đề xuất một thay đổi: quy trình RFC

RFC (request for comments) là một tài liệu thiết kế ngắn, được thảo luận trước khi xây.

1. Bàn ý tưởng trước trong một issue hay GitHub Discussion, để khỏi viết RFC cho thứ đã có kế hoạch.
2. Chép [`docs/rfcs/0000-template.md`](docs/rfcs/0000-template.md) thành `docs/rfcs/NNNN-ten-ngan.md` (số kế tiếp chưa dùng)
   và điền. RFC viết bằng tiếng Anh.
3. Mở pull request tiêu đề `rfc: <tên ngắn>`. Pull request chính là nơi thảo luận.
4. Người duy trì và cộng đồng bình luận. Tác giả cập nhật văn bản. Sau bảy ngày không còn phản đối chưa giải quyết, một người
   duy trì merge với trạng thái `accepted`, hoặc đóng với trạng thái `rejected` và nêu lý do.
5. RFC đã chấp nhận được cài đặt ở các pull request sau, có liên kết tới RFC. RFC đổi một luật lâu dài thì thêm cả file quyết
   định ở `docs/decisions/`.

Xem [`docs/rfcs/README.md`](docs/rfcs/README.md) để biết cái gì cần RFC và cái gì không.

## Nhãn issue và pull request

| Nhãn                        | Nghĩa                                                                        |
| --------------------------- | ---------------------------------------------------------------------------- |
| `good first issue`          | Nhỏ, mô tả rõ, có người duy trì giúp; đóng góp đầu tiên tốt                 |
| `help wanted`               | Chúng tôi muốn có đóng góp; mọi cỡ, không nhất thiết dễ                      |
| `bug`                       | Thứ gì đó không chạy như tài liệu                                            |
| `enhancement`               | Tính năng hay cải tiến mới                                                   |
| `documentation`             | Chỉ tài liệu                                                                 |
| `question`                  | Câu hỏi cách dùng (ưu tiên GitHub Discussions)                               |
| `needs-reproduction`        | Chưa tái hiện được; người báo được hỏi các bước                              |
| `needs-rfc`                 | Quá lớn cho một pull request; viết RFC trước                                 |
| `breaking-change`           | Đổi hạng hợp đồng ổn định; cần đường deprecated                              |
| `security`                  | Xử lý riêng trước; xem [`SECURITY.vi.md`](SECURITY.vi.md)                    |
| `module`, `module:<name>`   | Liên quan một module (và module nào)                                         |
| `db:sqlite`, `db:postgres`  | Chỉ tái hiện trên một database                                               |
| `wontfix`, `duplicate`      | Đóng kèm lý do                                                               |

Người duy trì hay chủ dự án tạo các nhãn này một lần trong cài đặt repo (file trong repo không tạo được nhãn).

## Quy tắc ứng xử và xung đột

Mọi người theo [quy tắc ứng xử](CODE_OF_CONDUCT.vi.md). Báo cáo về ứng xử gửi tới liên hệ trong tài liệu đó, không gửi vào
issue công khai. Bất đồng về hướng kỹ thuật theo các luật quyết định ở trên.

## Sửa tài liệu này

Sửa tài liệu này là một pull request cần chủ dự án duyệt.
