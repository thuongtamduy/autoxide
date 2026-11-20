## D-20261120-batch-postgres · Ghép lô chạy test của crate bị sửa trên cả Postgres — 2026-11-20 — chủ dự án quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: bổ sung D-20261119-merge-batches. Khi phiên điều phối ghép một lô trên máy, ngoài sinh lại file tự động,
`cargo xtask check`, `cargo xtask sdk --check` và các test hay vỡ khi ghép, phải chạy test của mọi crate bị lô sửa
(và crate phụ thuộc chúng) **trên Postgres**:

```bash
TEST_DATABASE_URL=postgres://… cargo xtask test-affected --base origin/main --keep-env
```

Lô chỉ đổi tài liệu, workflow CI hay `xtask` thì không cần. Postgres dùng image `postgres:16-alpine` (Docker Hub giới hạn
lượt tải thì lấy qua `mirror.gcr.io/library/postgres:16-alpine`) với `max_connections` ≥ 300. Đỏ thì tìm trong lô, sửa
trước khi merge; lỗi do máy (Docker tắt, hết đĩa — "pool timed out") thì dựng lại rồi chạy lại.

Lý do: gate nhanh và bước ghép lô trước đây chỉ chạy SQLite. SQLite cho một bên ghi mỗi lúc (`begin_write` là
`BEGIN IMMEDIATE`) nên lỗi đua nhau giữa hai transaction không lộ; ngày 20/11/2026 hai lỗi như vậy (#246 event giá ghi
trùng, #253 luồng sự kiện gửi lại `Last-Event-ID`) chỉ bị CI trên `main` bắt sau khi đã merge. Chạy Postgres cho riêng
crate bị sửa rẻ hơn gate đầy đủ mà bắt được loại lỗi này trước khi vào `main`.

Đã cân nhắc: chạy gate đầy đủ mỗi lô (quá chậm cho nhịp ghép lô, D-300 giữ nó cho lúc phát hành); bắt phiên làm việc
chạy Postgres trong gate nhanh (máy của phiên thường không có Docker, gate nhanh giữ chỉ SQLite theo D-300).
