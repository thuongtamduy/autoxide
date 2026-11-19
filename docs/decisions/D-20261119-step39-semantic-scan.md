## D-20261119-step39-semantic-scan · Tìm theo ý nghĩa trên dữ liệu lớn: `dot` dùng SIMD, duyệt chia nhiều nhân ngoài thread async — 2026-11-19 — agent quyết

Chủ đề: Vận hành và hiệu năng

Quyết định: giữ cách duyệt hết vector của D-109 / D-110 (chạy như nhau trên SQLite và Postgres), làm nhanh hơn bằng hai
việc không đổi kết quả: (1) `autoxide_kernel::ai::vector::dot` cộng vào 8 tổng riêng rồi gộp, để trình biên dịch dùng
lệnh SIMD (không `unsafe`); (2) `content` chạy vòng duyệt trong `tokio::task::spawn_blocking` và chia các đoạn thành
khúc liền nhau, mỗi nhân một khúc (`std::thread::scope`, tối thiểu 10 000 đoạn mỗi khúc), gộp theo thứ tự khúc — bằng
điểm thì giữ đoạn đứng trước, như duyệt một lượt. Không dùng vector `f16` / `int8` hay index ANN.

Lý do: đo trên cỡ thật của bộ `full` (50 000 bài một ngôn ngữ ≈ 100 000 đoạn), 4 nhân Xeon 2,1 GHz, build release
(`docs/BENCHMARKS.md`, "Tìm theo ý nghĩa"): 1 024 chiều 137 ms → 58 ms (một nhân, SIMD) → 20 ms (4 nhân); 1 536 chiều
209 → 30 ms; dưới ngưỡng 50 ms của D-110. 50 kết quả đầu giống hệt cách cũ. Trước đây vòng duyệt còn chạy thẳng trên
thread async — chặn mọi request khác trên thread đó 100+ ms mỗi lần tìm. `int8` chỉ nhanh hơn SIMD f32 một nhân ~30%,
đổi lấy sai số thứ hạng và phải đổi định dạng lưu; ANN / `pgvector` là nhánh riêng của Postgres (Luật số một) — chỉ
cân nhắc khi dữ liệu vượt ~250 000 đoạn mỗi ngôn ngữ mỗi khách.

Giới hạn còn lại: bộ nhớ — vector giữ trong RAM theo (khách, bản, ngôn ngữ), ~410 MB cho 100 000 đoạn × 1 024 chiều.
Nhiều khách lớn trên một máy chủ thì đó là giới hạn trước tiên; giảm bằng `f16` trong bộ nhớ (một nửa) là bước kế khi
đo được cần.

Đã cân nhắc: `int8` (xem trên); index ANN sau một trait (chưa vượt ngưỡng, ngoài phạm vi bước 39); `rayon` (thêm thư
viện cho việc `std::thread::scope` làm được trong vài dòng).
