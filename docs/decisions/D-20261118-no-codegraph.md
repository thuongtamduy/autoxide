## D-20261118-no-codegraph · Không dùng CodeGraph để agent tìm code; mục lục quyết định bỏ số đếm — 2026-11-18 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. Bỏ lời dặn "dùng CodeGraph (`codegraph explore`)" khỏi skill `next-step` (có từ commit đầu, nhưng công cụ chưa từng
   được cài hay khai MCP ở đâu, nên lời dặn không ai làm theo được). Agent tìm code bằng `grep` / `rg` theo tên kiểu, hàm,
   mã lỗi, tên bảng rồi đọc đúng đoạn đó. Không thêm `.mcp.json` hay `.codegraph/` vào repo.
2. Mục lục `docs/decisions/INDEX.md` không ghi số mục (tổng và theo chủ đề). Hai nhánh song song cùng thêm quyết định thì
   git gộp sạch nhưng số đếm sai, `decisions-index` báo lệch trên main dù không ai sửa tay (đã xảy ra sau #141 và #142,
   làm CI main đỏ). Không còn số đếm thì hai mục thêm ở hai chỗ gộp vẫn đúng; thêm sát nhau thì git báo xung đột rõ ràng,
   `cargo xtask decisions` sinh lại là xong. Mục lục vẫn được commit và vẫn được gate kiểm.

Lý do (mục 1), đo ngày 18/11/2026 trên repo này với CodeGraph 1.6.3 (1.228 file, 37.814 nút, 146.932 cạnh, chỉ mục
140 MB, lập trong 15 giây):

| Câu hỏi | `codegraph explore` | `grep -rn` |
|---|---|---|
| Chỗ báo bảng chưa khai chính sách tách / gộp | ~26 KB (~7 nghìn token), chỉ tới `tenancy/relocate.rs`, không tới chỗ báo | 1 dòng, đúng `crates/app/src/e2e/relocation.rs:193` |
| `Outbox::record` ghi event thế nào | ~26 KB, chỉ tới `event/mod.rs` | 1 dòng, đúng `crates/kernel/src/event/outbox.rs:39` |

Repo đã đặt tên rõ, file ngắn (D-20261118-file-length), luật ở một chỗ, OpenAPI và MCP theo module
(D-20261118-module-contracts), nên tìm theo tên đã trúng và rẻ hơn nhiều. CodeGraph còn mặc định gửi số liệu sử dụng ra
ngoài và thêm một bước cài cho mọi máy, mọi phiên. Đo lại khi repo lớn gấp vài lần hay khi tìm theo tên thường trượt.
