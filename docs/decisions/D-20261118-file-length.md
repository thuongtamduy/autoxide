## D-20261118-file-length · Độ dài file `.rs`: cảnh báo trên 800 dòng, chặn trên 1200, danh sách miễn không được dài thêm — 2026-11-18 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. `cargo xtask check` (trong gate) đếm dòng mọi file `.rs` git theo dõi (cả test):
   - **trên 800 dòng: chỉ in cảnh báo** (số file và mười file dài nhất), không làm gate đỏ — tách khi có dịp sửa file đó;
   - **trên 1200 dòng: luật `file-length` chặn**, trừ file có trong `xtask/file-length-baseline.txt` (dài từ trước khi có
     luật). File trong danh sách không được dài hơn số dòng ghi kèm; khi đã còn ≤ 1200 dòng, hay không còn, gate đòi xoá
     dòng của nó. Không thêm file mới vào danh sách.
2. Một ngưỡng cho cả code và test: file test dài cũng tốn ngữ cảnh như code, và tách theo nhóm test (`tests/<nhóm>.rs`) dễ
   như tách code.
3. Đợt này tách sáu file dài nhất ít bị phiên song song (bước 51/52) đụng tới, chỉ di chuyển code — không đổi hành vi, không
   đổi API công khai: `auth` role / session / user service, `plans` service và http, `xtask` api_diff. Cách tách: file giữ
   phần lõi, phần theo trách nhiệm sang module con `<file>/<phần>.rs` (`use super::*;`, phương thức thành `impl` riêng ở
   module con, hàm riêng tư thành `pub(super)`), khối `#[cfg(test)] mod …` sang file riêng.
4. Chưa tách (trong danh sách miễn): `content/src/http/tests.rs` và `content/src/entries.rs` (phiên bước 51/52 đang sửa
   nhiều), `kernel` (`ops/custom_fields.rs`, `data/tenancy/moves.rs`, `data/tenancy/admin.rs` — sửa kernel làm gate chậm và
   dễ xung đột), `compliance` (`cases.rs`, `http_cases.rs` — mục "sức khoẻ tuân thủ" của bước 51 có thể đụng).

Lý do: 191 file trên 500 dòng, dài nhất gần 2000 dòng; agent phải đọc từng khúc, dễ sửa trùng chỗ với phiên khác. Chặn
cứng mọi file trên 800 ngay thì gate đỏ hàng trăm chỗ; danh sách miễn với số dòng tối đa giữ cho file cũ không tệ thêm
mà không bắt tách hết một lần.

Đã cân nhắc: chỉ cảnh báo (không có gì ngăn file mới dài ra); chặn ngay ở 800 với danh sách miễn hơn trăm file (danh sách
dài, ai sửa file trong đó cũng phải sửa danh sách); ngưỡng riêng cho test (thêm luật mà không thấy lợi).
