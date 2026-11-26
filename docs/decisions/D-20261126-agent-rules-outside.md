## D-20261126-agent-rules-outside · `AGENTS.md` là bản tiếng Anh ngắn cho người ngoài, không còn là liên kết tới `CLAUDE.md` — 2026-11-26 — agent quyết

Chủ đề: Phạm vi và mục tiêu

Quyết định (bước 71, G6, "rào chắn cho AI"):

1. `AGENTS.md` ở gốc repo là **file thật, tiếng Anh, ngắn**: luật quyết định phần lớn các lần rà soát, lệnh đầu tiên
   (`cargo xtask doctor`, `autoxide demo`, `./gate.sh --quick`), và con trỏ tới tài liệu. Codex, Cursor, Copilot đọc nó;
   người ngoài đọc được. Trước đây nó là liên kết tới `CLAUDE.md` (tiếng Việt, nội bộ) — người ngoài và agent của công
   cụ khác nhận luật bằng ngôn ngữ họ không đọc.
2. `CLAUDE.md` vẫn là luật đầy đủ và nội bộ (tiếng Việt). Hai file **không được mâu thuẫn**: luật mới ghi ở `CLAUDE.md`;
   luật nào người ngoài cần biết thì chép tóm tắt sang `AGENTS.md` (một lần, cùng commit).
3. `.cursor/rules/autoxide.mdc` và `.github/copilot-instructions.md` là bản rất ngắn, trỏ về `AGENTS.md` và nhắc bảy luật
   quan trọng nhất (công cụ này không theo liên kết đáng tin cậy nên cần có chữ ở đó).
4. Khung module viết ngoài repo (`cargo xtask new-module --external`) đã có `AGENTS.md` riêng; luật Cursor / Copilot cho
   khung đó làm cùng lúc với cụm "cách viết module mới" của G6 (phụ thuộc hợp đồng kernel 1.0).

Lý do: repo đã công khai và sẽ có người dùng agent không phải Claude Code; "mọi trình độ dùng được" gồm cả người không
đọc tiếng Việt.

Đã cân nhắc: giữ liên kết (đơn giản nhất nhưng ngôn ngữ sai cho người ngoài); dịch cả `CLAUDE.md` sang tiếng Anh (luật nội bộ
đã chốt tiếng Việt, D-20261126-road-to-1-0 mục 6); sinh `AGENTS.md` từ `CLAUDE.md` (hai ngôn ngữ, không sinh tự động
được).
