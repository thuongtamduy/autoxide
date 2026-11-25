## D-20261125-return-url-allowlist · `return_url` của thanh toán chỉ nhận địa chỉ của chính khách: tên miền đã xác minh (https) hoặc origin khai ở `ledger.payments.return_origins` — 2026-11-25 — agent quyết

Chủ đề: Tiền, thuế, chứng từ

Quyết định: module `ledger` (nơi mọi lượt thu đi qua: đơn hàng, thanh toán thẳng, gói thuê bao, REST lẫn MCP) kiểm
`return_url` khi tạo lượt thu. Nhận khi origin (scheme + host + port, đọc bằng crate `url` như trình duyệt) là:

1. `https://<tên miền>` với tên miền ở trạng thái `verified` của khách sở hữu lượt thu (`TenantDomains::verified_hosts`,
   chế độ nhiều khách); hoặc
2. một origin khách khai ở cấu hình theo khách `ledger.payments.return_origins` (danh sách cách nhau bằng dấu phẩy,
   chỉ scheme + host + port, http hoặc https) — website ở tên miền chưa gắn vào Autoxide, chế độ một khách, tên miền
   universal link / app link của app di động, `http://localhost:3000` lúc phát triển.

Ngoài ra bị từ chối: 422 `validation_failed`, lỗi ô `return_url` mã `ledger.return_url_not_allowed`. Không nhận scheme
riêng của app (`myapp://`): đối tác thanh toán chỉ chuyển trình duyệt tới http(s), app dùng universal link. Lượt thu của
gói thuê bao (module `plans`) thuộc khách vận hành, nên trang quay về phải thuộc khách vận hành. Trang mặc định của bộ
nối (`momo.connection.account.return_url`) do quản trị khách đặt nên không kiểm thêm. Sửa P-5 của
`docs/security/review-2026-11.md`.

Lý do: đối tác chuyển TRÌNH DUYỆT người mua tới `return_url` sau khi trả; nhận mọi http(s) là open redirect — kẻ gian tạo
link thanh toán thật của cửa hàng mà người trả xong bị đưa sang trang giả (lừa nhập thông tin, "thanh toán lỗi, trả lại").
So theo origin chứ không theo tiền tố chuỗi để `https://shop.example.evil.example`, `https://shop.example@evil.example`
hay dấu `\` không lọt.

Thay đổi phá vỡ có chủ đích (lý do bảo mật, `docs/COMPATIBILITY.md` cho phép sửa lỗ hổng bảo mật trong bản thường và
ghi ở mục **Security** của changelog): client đang gửi `return_url` tới địa chỉ chưa khai sẽ nhận 422 cho tới khi khách
khai origin hay xác minh tên miền. Không làm endpoint mới vì endpoint cũ giữ hành vi cũ chính là giữ lỗ hổng.

Đã cân nhắc: chỉ tên miền đã xác minh — chế độ một khách không có tên miền, website chạy trên tên miền chưa gắn và máy
phát triển không dùng được; dùng `AUTOXIDE_HTTP_CORS_ORIGINS` của máy chủ — cấu hình của người vận hành, chung mọi khách,
không phải của từng cửa hàng; lặng lẽ bỏ `return_url` lạ và dùng trang mặc định — client không biết vì sao người mua
về sai trang; cho cả địa chỉ `localhost` tự động — không cần, khai một dòng là đủ và không mở thêm gì cho máy thật.
