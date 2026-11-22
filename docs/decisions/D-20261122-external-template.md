## D-20261122-external-template · Khung module ngoài: README, `AGENTS.md`, CI mẫu, `--core-path`, gate build và test khung sinh ra — 2026-11-22 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định (bước 70 P10, nối tiếp D-20261122-module-helpers):

- Khung `cargo xtask new-module --external <dir> <module> <entity>` vẫn sinh từ `examples/external-module/` (một
  nguồn, CI kiểm mỗi lần). Khuôn có thêm: `README.md` + `README.vi.md` cho repo mới; `AGENTS.md` — luật cho AI agent
  làm việc trong repo module (cả hai database, chỉ dùng `autoxide_kernel` và `api` của module khác, bảng của mình,
  logic ở service, mã lỗi có bản dịch, test bằng testkit; lệnh `check.sh`, `check-module`; liên kết tới hướng dẫn)
  và `CLAUDE.md` là liên kết tới nó như ở repo chính (scaffold giữ liên kết là liên kết); `.github/workflows/ci.yml`
  chạy `check.sh` trên SQLite và PostgreSQL (trong repo này nó chỉ là khuôn: GitHub chỉ chạy workflow ở `.github/`
  gốc).
- Sinh xong, lệnh chạy `cargo fmt --all` trong thư mục mới: tên mới dài / ngắn hơn tên khuôn làm lệch thứ tự import
  và độ dài dòng, mà `check.sh` chạy `fmt --check` đầu tiên.
- `--core-path <checkout>`: workspace sinh ra phụ thuộc core bằng đường dẫn tới bản checkout thay cho git tag — để
  thử khung với code core đang sửa (tag của phiên bản đang đứng có thể chưa có thay đổi đó).
- `./gate.sh --only example` (job CI `external module example`, và gate nhanh khi `examples/` hay
  `xtask/src/scaffold.rs` đổi), sau `check.sh` của ví dụ: sinh khung vào thư mục tạm với `--core-path .`, rồi
  `fmt --check`, `clippy -D warnings`, `cargo test` trên SQLite, dùng chung thư mục build với ví dụ (chỉ biên dịch lại
  module đổi tên và binary). Postgres không chạy lại ở khung: cùng mã nguồn với ví dụ vừa chạy trên Postgres ngay trên.

Lý do: người lạ (và AI agent của họ) cần một lệnh ra repo chạy được ngay, có luật và CI sẵn; trước đây khung chỉ được
kiểm là "sinh đủ file, đổi hết tên" — không ai biết nó còn build được không.

Đã cân nhắc: `cargo generate` từ thư mục template — thêm một công cụ ngoài người dùng phải cài, và template có cú pháp
riêng không chạy được như một workspace thật để CI kiểm; repo template trên GitHub — việc ngoài repo, lệch khỏi ví dụ
CI đang kiểm (D-20261122-module-helpers); chạy cả Postgres cho khung sinh ra — gấp đôi thời gian của job dài nhất CI
mà không phủ thêm mã nào.
