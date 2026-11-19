## D-20261119-step39-cdn-cache · Header cache cho CDN do một lớp của kernel chốt; đọc công khai `s-maxage` 60 giây — 2026-11-19 — agent quyết

Chủ đề: Vận hành và hiệu năng

Quyết định: lớp `crates/kernel/src/http/cache.rs` (trong lớp `304`, để ETag nó thêm cũng cho `304`) chốt ba luật,
handler vẫn quyết `Cache-Control` của mình:

1. Lệnh ghi (không phải `GET` / `HEAD`): không đổi gì — cache dùng chung vốn không giữ response của lệnh ghi (RFC 9111
   §9), và `no-store` trên lệnh ghi đã có nghĩa riêng ở lớp idempotency ("response chứa secret, không lưu"); tự đặt nó
   làm mọi lệnh gọi lại với `Idempotency-Key` bị từ chối (gate bắt được).
2. Request có `Authorization` hay `Cookie` → response không bao giờ `public`: chưa đặt gì → `private, no-store`;
   `public` → `private`, bỏ `s-maxage`, `stale-while-revalidate`, `proxy-revalidate`.
3. Đọc không đăng nhập mà handler đặt đúng `public, no-cache` (quy ước sẵn có của endpoint công khai: `content`,
   `catalog`) → `public, max-age=0, s-maxage=<AUTOXIDE_HTTP_PUBLIC_CACHE_SECONDS>, stale-while-revalidate=300`, thêm
   `Vary: Accept-Language, Accept-Encoding` (`Origin`, `autoxide-tenant` đã có), và ETag yếu là SHA-256 của thân
   (JSON ≤ 4 MiB) khi handler chưa đặt ETag.

`AUTOXIDE_HTTP_PUBLIC_CACHE_SECONDS` mặc định 60 (0–86 400); `0` giữ `public, no-cache`. Trình duyệt luôn hỏi lại
(`max-age=0`), rẻ nhờ `304`; CDN đỡ phần lớn lượt đọc. Giữ nguyên những gì handler đặt chặt hơn: `no-store` của bản xem
trước, `immutable` một năm của file media có `?v=` đúng phiên bản, `private` của file riêng tư, `max-age` riêng của
`menus`, `bookings`, `address`, `organizations`.

Lý do: đo ở bước 39 B2 — danh sách bài, sitemap, feed trên SQLite 120–160 req/s mỗi máy chủ 4 nhân; một CDN giữ 60 giây
làm máy chủ chỉ nhận một lượt mỗi phút mỗi địa chỉ (mỗi điểm biên). Đổi bài thì website thấy bản mới chậm tối đa 60 giây
(+ 300 giây khi máy chủ lỗi / chậm) — chấp nhận được cho nội dung công khai; người cần tức thì đặt `0`. Một lớp ở kernel
thay vì sửa từng module: luật "đăng nhập thì không bao giờ public" phải đúng cho MỌI endpoint, kể cả module viết sau.

Đã cân nhắc: đánh dấu public bằng extension `x-public` của OpenAPI (lớp HTTP không biết route nào khớp sau khi đã ra
khỏi router; quy ước `public, no-cache` đã có sẵn ở các handler công khai); `max-age` cho trình duyệt (trình duyệt
không có cách xoá cache khi bài đổi); purge CDN theo event (cần API riêng của từng CDN — để sau, khi có người dùng cần).
