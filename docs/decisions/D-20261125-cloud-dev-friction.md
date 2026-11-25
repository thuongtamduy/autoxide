## D-20261125-cloud-dev-friction · Máy cloud: Postgres cục bộ, build test trước bằng `--prebuild`, test-affected chỉ dựng xtask; báo workflow đêm đỏ bằng issue — 2026-11-25 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. **Postgres cục bộ.** `scripts/cloud-setup.sh` (chỉ khi `CLAUDE_CODE_REMOTE=true` và có `pg_ctlcluster`) bật cụm PostgreSQL 16 có
   sẵn, đặt mật khẩu cho role `postgres` (loopback, container dùng xong bỏ) và in `TEST_DATABASE_URL`. Không cài gói, không
   `initdb`; thiếu binary hay quyền thì bỏ qua. Cách dùng ở `docs/CLOUD-SESSIONS.md`.
2. **`cargo-semver-checks`** cài như các công cụ khác: bản ghim 0.51.0 (khớp `ci.yml`, STACK.md), tarball musl dựng sẵn, SHA-256
   đo lúc ghim (nhà phát hành không công bố checksum).
3. **Build lạnh.** Đo (máy 4 CPU, `CARGO_INCREMENTAL=0` như gate): `cargo test --workspace --no-run` 13 phút 28 giây, ~12 GB;
   dựng lại không đổi gì 4,6 giây; clippy thêm 5 phút 41 giây. Vì lần gate nhanh đầu gần như toàn là build này nên
   thêm cờ riêng `--prebuild` cho bước đó (bỏ qua khi đĩa trống dưới 25 GB, lỗi thì không làm hỏng script). Không gộp vào `--warm`: setup script của môi trường gọi `--warm` và có giới hạn thời gian (con số ~5 phút chưa kiểm chứng), mà bước dựng ~13 phút; `--prebuild` chạy nền sau khi phiên mở. Không build clippy trước: gate nhanh bỏ clippy.
   Build với `CARGO_INCREMENTAL` bật thêm ~11 GB incremental và không dùng chung cache với gate — tài liệu dặn tránh.
4. **`test-affected` chỉ dựng cái cần khi chỉ crate công cụ bị ảnh hưởng.** Nguyên nhân 13 phút cho một chương trình test chạy
   0,5 giây: `test-affected` luôn dựng test của cả workspace (D-299, để dùng chung cache với gate đầy đủ), kể cả khi chỉ `xtask`
   đổi, mà `xtask` không phụ thuộc kernel hay module nào. Nay nếu mọi crate bị ảnh hưởng đều là `Role::Tool` thì dựng
   `-p <crate>` (~20 giây trên máy lạnh); có crate khác thì giữ cách cũ.
5. **Test của kernel** `module_khong_khai_gi_thi_khong_co_migration_va_cau_hinh` không còn danh sách tên biến viết tay: so với
   bộ module không có module người dùng. Thêm `Setting` mới ở kernel không làm đỏ nó nữa.
6. **Mẫu PR** khớp luật: tick `./gate.sh --quick`; gate đầy đủ trên Postgres là việc của CI và của lúc phát hành (D-300,
   D-20261121-ci-batch-release-faster).
7. **Workflow theo lịch đỏ thì mở issue** (`property.yml`, `audit.yml`): job `report` chạy `scripts/nightly-issue.sh` — đỏ thì
   mở issue (đã có thì bình luận), xanh lại thì đóng. Quyền `issues: write` chỉ ở job đó; dùng `gh` có sẵn trên runner nên
   không thêm action; zizmor và actionlint sạch. Thử logic bằng `gh` giả (cả năm trường hợp); chưa thử trên GitHub thật.
8. **Lời nhắc "uncommitted / unpushed" lúc dừng lượt** không đến từ repo (`.claude/settings.json` không có hook `Stop`): là của nền
   tảng, không sửa được. `CLAUDE.md` ghi rõ nó không vượt luật gate.

Lý do: bốn phiên song song ngày 25/11/2026 mỗi phiên tự mò cách bật Postgres, mất 7–13 phút cho lần build đầu, và hai đêm
`property.yml` đỏ mà chỉ lộ ra khi chủ dự án tình cờ mở trang Actions.

Đã cân nhắc: build trước cả clippy (thêm 5,7 phút và đĩa, gate nhanh không dùng); dùng `-p` cho mọi lần `test-affected` (đo được
87 giây build lại sqlx/sea-orm chỉ cho `content` vì hợp feature khác; D-299 đã bỏ); action bên thứ ba để mở issue (thêm một thứ
phải ghim và duyệt); xoá danh sách biến bằng cách bỏ hẳn kiểm "module rỗng không thêm biến" (mất ý định gốc của test).
