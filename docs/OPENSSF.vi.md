# Huy hiệu OpenSSF: Scorecard và Best Practices

[English](OPENSSF.md) · **Tiếng Việt**

Hai chương trình OpenSSF độc lập đo mức an toàn của một dự án mã nguồn mở. Autoxide dùng cả hai.

## OpenSSF Scorecard (tự động)

[Scorecard](https://scorecard.dev) chấm điểm repo theo các mục như phụ thuộc được ghim, quyền của token, review code,
branch protection, bản phát hành có chữ ký và fuzzing. Workflow
[`.github/workflows/scorecard.yml`](../.github/workflows/scorecard.yml) chạy mỗi tuần và mỗi lần push vào `main`, công
bố kết quả và đẩy lên tab Security. Mỗi bản phát hành không phải làm thêm gì.

Thêm huy hiệu vào README sau khi lần chạy đầu đã công bố:

```markdown
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/OWNER/REPO/badge)](https://scorecard.dev/viewer/?uri=github.com/OWNER/REPO)
```

Một số mục phụ thuộc cài đặt repo mà workflow không đổi được. Người duy trì phải đặt trong **Settings → Branches**
(branch protection hay ruleset cho `main`): bắt buộc pull request, bắt buộc các status check `gate (…)`, bắt buộc ít
nhất một người duyệt, chặn force push. Bật các mục này là cải thiện điểm lớn nhất.

## Huy hiệu OpenSSF Best Practices (trả lời trên web)

[Huy hiệu Best Practices](https://www.bestpractices.dev) là bảng câu hỏi do người duy trì trả lời, kèm liên kết bằng
chứng. Có ba mức: passing, silver, gold. Các bước:

1. Đăng nhập bestpractices.dev bằng tài khoản GitHub của một người duy trì và thêm repo.
2. Trả lời các câu hỏi. Phần lớn trả lời được bằng một liên kết tới repo này:

   | Tiêu chí                          | Bằng chứng trong repo                                    |
   | --------------------------------- | -------------------------------------------------------- |
   | Trang dự án, mô tả                | `README.md`                                              |
   | Cách đóng góp                     | `CONTRIBUTING.md`, `GOVERNANCE.md`                       |
   | Giấy phép FLOSS                   | `LICENSE-MIT`, `LICENSE-APACHE` (hai giấy phép)          |
   | Tài liệu                          | `docs/`, hợp đồng OpenAPI `docs/openapi.json`            |
   | HTTPS cho trang dự án             | GitHub Pages và github.com                               |
   | Mã nguồn công khai có version control | GitHub                                               |
   | Số phiên bản duy nhất, release notes | Tag SemVer, `CHANGELOG.md`                            |
   | Quy trình báo lỗi                 | `.github/ISSUE_TEMPLATE/`, `SUPPORT.md`                  |
   | Quy trình báo lỗ hổng             | `SECURITY.md` (báo riêng, thời hạn phản hồi)             |
   | Hệ thống build chạy được          | `cargo build`; build tái lập ở `docs/COMPATIBILITY.md`   |
   | Bộ test tự động                   | `./gate.sh`, `.github/workflows/ci.yml`                  |
   | Tính năng mới có test             | Luật trong `CONTRIBUTING.md`, kiểm khi review            |
   | Cờ cảnh báo / linter              | Clippy, `cargo xtask check`, zizmor, actionlint          |
   | Phân tích tĩnh                    | CodeQL (`codeql.yml`), Clippy                            |
   | Phân tích động, fuzzing           | `fuzz/` và `.github/workflows/fuzz.yml`                  |
   | Lỗ hổng đã biết được sửa          | `cargo deny` trong CI, `audit.yml` hằng đêm, Dependabot  |
   | Secret không nằm trong git        | gitleaks trong CI                                        |
   | Bản phát hành có chữ ký / provenance | Workflow phát hành (xem `docs/RELEASING.md`)          |
   | Build tái lập (gold)              | `docs/COMPATIBILITY.md`, mục "Reproducible builds"       |
   | Hai người review (gold)           | Branch protection; cần người duy trì thứ hai             |

3. Chép đoạn huy hiệu mà bestpractices.dev đưa (có số id của dự án) vào README.

Trả lời trung thực: tiêu chí đánh "đạt" phải đúng và liên kết bằng chứng phải chứng minh được. Tiêu chí chưa đúng (ví dụ
người review thứ hai cho gold) để "chưa đạt" cho tới khi đúng.

## Ai làm gì

| Bước                                          | Ai làm                       |
| --------------------------------------------- | ---------------------------- |
| Workflow Scorecard và CodeQL                  | Đã làm trong repo            |
| Branch protection cho `main`                  | Chủ dự án (cài đặt repo)     |
| Bật báo lỗ hổng riêng tư (private vulnerability reporting) | Chủ dự án (cài đặt repo) |
| Tạo mục Best Practices và trả lời             | Chủ dự án (web)              |
| Thêm hai huy hiệu vào README                  | Người đóng góp bất kỳ, sau các bước trên |
