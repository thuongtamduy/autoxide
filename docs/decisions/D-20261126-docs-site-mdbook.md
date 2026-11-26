## D-20261126-docs-site-mdbook · Website tài liệu có phiên bản: mdBook, dựng tĩnh, mỗi bản phát hành một bộ, đăng lên GitHub Pages — 2026-11-26 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: Website tài liệu chính thức (hướng dẫn, tham chiếu API, tài liệu cho người viết module; bước 71 G13) dùng
**mdBook** (công cụ của chính sách Rust Book; tìm kiếm có sẵn, ra HTML tĩnh), dựng bằng `scripts/build-docs-site.sh`
từ các file Markdown sẵn có (không viết lại tài liệu). Mỗi bản phát hành (tag `vX.Y.Z`) được dựng thành một thư mục
`/vX.Y.Z/<ngôn ngữ>/` kèm `/latest/` (chuyển hướng tới bản chính thức cao nhất) và `versions.json` cho bộ chọn phiên bản;
tiếng Anh và tiếng Việt là hai cuốn sách song song (`en`, `vi`) dựng từ cặp `X.md` / `X.vi.md` đã có. Workflow
`docs-site.yml` tự dựng khi tag được đẩy và đẩy vào nhánh `gh-pages` (GitHub Pages phục vụ từ nhánh đó); bản cũ giữ nguyên.
Tham chiếu API là Markdown sinh từ `docs/openapi.json` và `docs/mcp-tools.json` cùng lần dựng, tham chiếu Rust dẫn tới
docs.rs. Bật Pages (nguồn: nhánh `gh-pages`) là việc của chủ dự án, không phải của agent.

Lý do: chọn công cụ phổ biến, bền, dựng tĩnh, có tìm kiếm. mdBook cùng toolchain Rust với dự án (một cài đặt, không thêm
hệ sinh thái Node cho tài liệu), được cộng đồng Rust duy trì lâu năm, không phụ thuộc dịch vụ ngoài; tìm kiếm chạy ở
trình duyệt (không máy chủ). Nhiều phiên bản và hai ngôn ngữ làm bằng thư mục theo phiên bản, đơn giản và dễ kiểm.

Đã cân nhắc: Docusaurus (nhiều phiên bản và i18n có sẵn nhưng kéo cả hệ sinh thái Node/React, dựng chậm, nhiều thư viện để
nâng); MkDocs Material + mike (phổ biến nhưng Material đã chuyển sang chế độ bảo trì và đổi hướng, rủi ro bền vững);
VitePress (không có phiên bản sẵn); đăng bằng `actions/deploy-pages` (mỗi lần đăng thay cả site, phải dựng lại mọi bản
cũ — nhánh `gh-pages` giữ bản cũ nguyên vẹn).
