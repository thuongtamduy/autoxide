## D-20261119-merge-batches · Merge theo lô do phiên điều phối ghép; mỗi phiên một PR lớn cho một cụm việc; file sinh tự động không gộp tay — 2026-11-19 — agent quyết (chủ dự án đồng ý)

Chủ đề: Quy trình, gate, lộ trình

Quyết định:
- **Ghép lô.** Phiên làm việc chạy `./gate.sh --quick` trên máy rồi push PR, không chờ CI của PR (gate nhanh là đủ để
  merge vào `main`, D-300). Phiên điều phối cứ khoảng 1–2 giờ gom các PR sẵn sàng: gộp tất cả trên một bản ghép ở máy,
  sinh lại file tự động một lần, chạy `cargo xtask check`, `cargo xtask sdk --check` và các test hay vỡ khi ghép
  (`modules::tests`, `openapi`, `e2e::relocation`, `e2e::migrations`), rồi merge cả lô vào `main` (kiểu merge, có
  `expectedHeadSha`). CI trên `main` chạy sau làm lưới thứ hai; đỏ thì tìm trong lô vừa merge và sửa ngay.
- **Một PR cho một cụm việc.** Mỗi phiên gom các mục của bước mình vào một PR, push một lần khi xong cụm, thay vì một
  PR cho mỗi mục nhỏ: ít lượt ghép hơn.
- **File sinh tự động không gộp tay.** `docs/openapi.json`, `docs/mcp-tools.json`, `sdk/typescript/src/schema.ts`,
  `docs/decisions/INDEX.md`, `.env.example` khai `merge=generated` trong
  `.gitattributes`; máy ghép lô bật `git config merge.generated.driver true` rồi sinh lại (`autoxide openapi`,
  `mcp-manifest`, `config --env-example`, `cargo xtask sdk`, `cargo xtask decisions`). PR chỉ xung đột ở các file này
  thì phiên làm việc không phải gộp `main` lại: phiên điều phối lo khi ghép lô.
- Trước khi phát hành vẫn bắt buộc gate đầy đủ trên SQLite và Postgres (D-300), kèm đo hiệu năng.

Lý do: ngày 19/11/2026, với 9–10 phiên song song, mỗi PR phải gộp `main` rồi chờ CI ~35 phút trước khi merge; phần
lớn lượt gộp lại chỉ vì xung đột ở file sinh tự động, và hàng đợi có lúc 10–11 PR (vài giờ). Lần ghép thử 6 PR trên máy
(không xung đột, file sinh lại khớp, 14 test ghép xanh) rồi merge liền một lúc cho thấy cách này nhanh mà vẫn an toàn.

Đã cân nhắc: một nhánh chung cho mọi phiên push thẳng (đụng nhau khi push, lỗi khó quy về phiên nào); bỏ file sinh
tự động khỏi git (test giữ tài liệu khớp code và người dùng SDK cần chúng trong repo); máy CI lớn hơn (tốn phí, để chủ
dự án quyết).
