## D-20261118-public-ci-openapi-dup · Repo công khai: CI tự chạy; pull request chỉ gate trên PostgreSQL 16; bắt tham số OpenAPI khai trùng bằng `lint` thay vì thêm `tsc` vào gate (D-197, D-243, D-300) — 2026-11-18 — chủ dự án quyết phần CI, agent quyết cách làm

Chủ đề: Quy trình, gate, lộ trình

Bối cảnh: chủ dự án chuyển repo sang công khai. Workflow chạy theo D-197 (chỉ chạy khi repo công khai), miễn phí, không tính
phút. Lần chạy CI đầu tiên đỏ ở job `sdk`: `POST /api/v1/kernel/bulk/{resource}/{action}` khai `Idempotency-Key` (bắt buộc),
rồi `http::openapi::finish` thêm bản chung (không bắt buộc) lần nữa, nên `schema.ts` có hai thuộc tính cùng tên và `tsc` đỏ.
Lỗi có từ bước 51 P8 (thao tác hàng loạt, commit b264b6a) và không ai thấy, vì gate trên máy chỉ kiểm SDK khớp `openapi.json`, không
biên dịch nó (D-243).

Quyết định:

1. `finish` chỉ thêm `Accept-Language` / `Idempotency-Key` khi operation chưa tự khai header cùng tên (không phân biệt hoa
   thường); bản endpoint tự khai được giữ.
2. `http::lint` (gate chạy trên tài liệu của cả chương trình) báo tham số cùng vị trí và cùng tên khai hai lần. Gate vẫn
   không cần Node (giữ D-243); `tsc` vẫn chạy trong job `sdk` của CI.
3. CI: pull request chỉ chạy `gate` trên PostgreSQL 16 (bản thấp nhất được hỗ trợ, dễ hỏng nhất), push lên `main` và chạy tay
   chạy cả 16 và 18. Mỗi job `gate` chạy trọn `./gate.sh`, nên ma trận hai bản làm lại mọi bước tĩnh và test SQLite; với pull
   request chỉ còn một lần.

Đã cân nhắc: thêm `npx tsc --noEmit` vào `gate.sh` — buộc người chạy gate cài Node, trái D-243; tách CI thành một job kiểm
tĩnh + test SQLite và một ma trận chỉ test Postgres — phải tách `gate.sh` theo bước, chưa đáng khi CI miễn phí.

Hệ quả: lỗi chỉ lộ trên PostgreSQL 18 được bắt sau khi merge (lần push lên `main`), không chặn pull request. Status check bắt
buộc trong ruleset của `main` chỉ đặt `gate (PostgreSQL 16)`. D-300 không đổi: gate đầy đủ trên máy vẫn bắt buộc trước khi
phát hành.
