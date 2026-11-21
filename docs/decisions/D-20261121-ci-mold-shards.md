## D-20261121-ci-mold-shards · CI: mold làm linker cho job build tests; test Postgres chia 6 phần — 2026-11-21 — chủ dự án quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. Job `build tests` của `gate.yml` cài **mold** (`rui314/setup-mold@v1`) trước khi build. Action thay linker mặc
   định của máy CI bằng mold, không đổi `RUSTFLAGS`, nên khoá cache của sccache và rust-cache giữ nguyên. Chỉ job
   này: nó link hàng chục chương trình test lớn, phần link chiếm đáng kể thời gian. Các job khác chỉ tải archive về
   chạy, không link. Build trên máy người phát triển và build image phát hành không đổi.
2. Phần test **Postgres chia 6 phần** (`postgres 16 k/6`, và `postgres 18 k/6` ở push lên `main`), SQLite giữ 4.
   Mỗi test Postgres tạo một database thật nên chậm hơn SQLite: với 4 phần, Postgres xong sau 5–7 phút trong khi
   SQLite xong sau 3–4 phút, và CI phải chờ phần chậm nhất. Check bắt buộc là job gộp `gate (PostgreSQL …)`, không
   phụ thuộc tên từng phần, nên đổi tên `k/4` → `k/6` không ảnh hưởng branch protection.

Lý do: chủ dự án muốn CI nhanh hơn nữa mà không tốn tiền. Repo công khai nên thêm máy GitHub Actions miễn phí; mold
là công cụ mã nguồn mở. Ước tính bớt 2–5 phút mỗi lần CI (đường dài nhất trước đây khoảng 17–20 phút); đo thật sau
khi merge rồi ghi vào docs/BENCHMARKS.md.

Đã cân nhắc: máy CI lớn hơn (tốn tiền); giảm tối ưu bản release (không liên quan CI test, và làm sản phẩm chậm hơn);
chia SQLite thêm (đã xong sớm, không nằm trên đường dài nhất).
