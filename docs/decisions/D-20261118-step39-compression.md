## D-20261118-step39-compression · Nén response ngay trong app (gzip, brotli mức 1), tắt được khi proxy đã nén — 2026-11-19 — agent quyết

Chủ đề: Vận hành và hiệu năng

Quyết định: lớp HTTP của kernel nén response bằng `CompressionLayer` của `tower-http` (feature `compression-gzip`,
`compression-br`), mức 1 (`CompressionLevel::Precise(1)`), theo `Accept-Encoding` của client. Chỉ nén chữ: JSON
(`application/json`, `…+json`), `text/*` trừ `text/event-stream`, XML (`application/xml`, `…+xml`), JavaScript; thân
từ 1 KiB trở lên; response một khúc (`Content-Range`) không nén. File tải lên (ảnh, PDF, zip, nhị phân) không bao giờ
nén lại. Lớp đặt NGOÀI lớp idempotency (bản lưu để trả lại là bản chưa nén) và trong lớp rate limiting / CORS. `ETag`
giữ nguyên (không đổi sang `W/`): `If-None-Match` vẫn cho `304`, `If-Match` của lệnh sửa không đổi; `Vary:
Accept-Encoding` (do `tower-http` thêm) tách bản lưu của cache. Cấu hình `AUTOXIDE_HTTP_COMPRESSION` (mặc định `true`),
đặt `false` khi reverse proxy / CDN phía trước đã nén.

Lý do: đo trên JSON thật (`docs/BENCHMARKS.md`, mục "Nén response"): trang 20 bài công khai 136 KB → 26 KB (brotli 1,
0,66 ms) hay 32 KB (gzip 1, 0,46 ms); danh sách quản trị 306 KB → 58–72 KB (~1–1,4 ms); `openapi.json` 3 MB → 370 KB
(brotli 1, 9 ms) / 630 KB (gzip 1, 11 ms). Mức mặc định của các thư viện tốn hơn nhiều (gzip 6: 3,5 ms cho trang 20
bài; brotli 11: 173 ms) mà chỉ nhỏ thêm 10–25%. Người lạ tự cài (thường một binary sau Caddy, hay không có proxy) nhận
response nén mà không phải cấu hình gì; Caddy không nén nếu không khai `encode`. Không thêm thư viện mới: chỉ bật
feature của `tower-http` đã có (kéo theo `async-compression`, `flate2`, `brotli` — Rust thuần, không thư viện C).

Đã cân nhắc: để reverse proxy nén (người tự cài phải tự cấu hình, nhiều người sẽ quên; app vẫn trả JSON lớn qua
mạng nội bộ khi không có proxy); thêm zstd (nhanh và nhỏ hơn gzip, nhưng crate `zstd` bọc thư viện C và Safari chưa
nhận); mức nén mặc định (tốn 5–8 lần CPU cho 10–25% cỡ); đổi `ETag` sang `W/` khi nén như nginx (làm `If-Match` của
lệnh sửa — so sánh mạnh — thất bại).
