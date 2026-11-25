## D-20261125-regen-and-gate-order · Một lệnh `cargo xtask regen`; gate kiểm rẻ trước, kiểm file sinh sớm; luật số thứ tự migration — 2026-11-25 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. **`cargo xtask regen [--check]`** sinh lại một lượt, theo thứ tự phụ thuộc: `docs/openapi.json` (`autoxide openapi`),
   `docs/mcp-tools.json` (`mcp-manifest`), `.env.example` (`config --env-example`), rồi SDK (`cargo xtask sdk`, đọc openapi.json vừa
   ghi) và mục lục quyết định. Chạy với cấu hình mặc định (bỏ biến `AUTOXIDE_*` của người chạy). `--check` chỉ so ba file đầu và in
   `cargo xtask regen` khi lệch; SDK vẫn có `sdk --check` riêng.
2. **Thứ tự bước của `gate.sh`**: fmt, `cargo xtask check`, `sdk --check` (vài giây) chạy ĐẦU TIÊN, trước AVIF, ví dụ module và clippy;
   sau clippy là `regen --check` (chỉ khi có Rust đổi: nó dựng binary `autoxide`, cùng bản dựng bước test dùng sau đó — đo 8 phút
   lần đầu, không tốn thêm khi test dựng lại), rồi api-diff, deny, gitleaks. Không đổi tập bước, không đổi `--only`.
3. **Số thứ tự migration** (`m0038_…` viết tay): trước đây chỉ bắt tên trùng hệt nhau (`Migrator::new`), không bắt hai nhánh chọn cùng số
   với tên khác. Thêm bài test `modules::tests::so_thu_tu_migration_khong_trung_va_tang_dan` (chạy trong gate, mọi module và kernel): trong
   một module, số không được trùng và không được nhỏ hơn số đứng trước; số cách quãng thì được. Thông báo nêu số và module. Không đổi
   cách đặt tên migration đã commit. Là test chứ không phải luật `xtask check`: kernel khai tên trong code Rust, theo từng module, nên
   chỉ danh sách thật của `Migrator` mới biết ranh giới của một chuỗi số.
4. **Đĩa / `CARGO_INCREMENTAL`**: đo `target/` 12 GB → 23 GB sau một lần `cargo test` mặc định (incremental bật). `cloud-setup.sh`
   ghi `export CARGO_INCREMENTAL=0` vào `CLAUDE_ENV_FILE` khi nền tảng đặt biến đó (chưa kiểm chứng; ở phiên viết nó không có), không thì
   nhắc; in cảnh báo khi đĩa trống dưới 20 GB. Không đặt `build.incremental = false` trong `.cargo/config.toml` (chậm vòng sửa-build ở
   máy thường).
5. **Khoá dịch `field:<câu>`**: kiểm tra, đã được chặn — đổi một chữ của câu tiếng Anh ở `FieldError::new` làm `cargo xtask check` đỏ
   hai lỗi (`i18n-field` thiếu bản dịch của câu mới, `i18n` bản dịch của câu cũ thành thừa). Không đổi gì.
6. **`shellcheck`**: không bước nào của gate gọi nó (chỉ có chú thích `# shellcheck disable=` trong script, và `actionlint` dùng nó nếu có trên
   máy). Không thêm; lệnh báo "not found" là lệnh mình gõ tay.

Lý do: lỗi rẻ (định dạng, ranh giới, file sinh lệch, số migration) chỉ lộ ra sau 10–30 phút của một vòng gate; hai phiên song song
chọn trùng số migration (phiên bảo mật phải đổi m0038 → m0040).

Đã cân nhắc: `regen --check` trong nhóm rẻ đầu tiên (nó cần dựng binary, không rẻ); luật migration trong `xtask check` bằng quét mã nguồn (kernel
không có ranh giới chuỗi số theo thư mục); `build.incremental = false` toàn repo (hại người dùng máy thường); cài `shellcheck` (không gate nào cần).
