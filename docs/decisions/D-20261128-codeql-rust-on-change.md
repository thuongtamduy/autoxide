## D-20261128-codeql-rust-on-change · CodeQL Rust trên pull request chỉ chạy khi PR chạm Rust — 2026-11-28 — chủ dự án quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: trên `pull_request`, workflow CodeQL (`.github/workflows/codeql.yml`) có job `plan` gọi
`scripts/rust-changed.sh <base.sha>` — cùng script và cùng danh sách `scripts/rust-free-paths.txt` mà gate và job `plan`
của `ci.yml` dùng (D-20261125-ci-skip-rust), không có danh sách đường dẫn thứ hai. Chỉ khi script in đúng `false` (mọi file
đã đổi nằm trong danh sách cho phép: tài liệu, SDK TypeScript, changelog, quyết định…) thì matrix bỏ `rust`; mọi trường hợp
khác (`plan` đỏ, bị bỏ qua, không tìm được mốc so sánh, có file không khớp) đều quét Rust. `javascript-typescript` và
`actions` luôn chạy. Push `main`, lịch hằng tuần và chạy tay quét đủ ba ngôn ngữ như cũ: `plan` chỉ chạy trên pull
request, output rỗng thì matrix đủ.

Lý do: `analyze (rust)` mất 21–27 phút và là job dài nhất của mọi PR kể cả PR chỉ sửa tài liệu (PR #605 chỉ sửa workflow:
mọi job khác xong trong 8 phút, CodeQL Rust 27 phút; PR #608: Rust 21 phút, JS/TS 70 giây, actions 50 giây). PR không đổi
file `.rs` nào và không đổi cấu hình CodeQL thì kết quả quét Rust giống hệt ở nhánh gốc — đã được quét khi nhánh gốc vào
`main`. Không mất lớp bảo mật nào: mọi commit vào `main` vẫn quét đủ, và mỗi tuần quét lại với bộ truy vấn mới.

Ngôn ngữ bị bỏ không sinh check (matrix không có phần tử đó), nên PR không có check đỏ hay "pending" treo; `main` không
có required status check (D-20261127-ci-runner-budget). Code scanning có thể ghi chú trên check "CodeQL" của PR rằng thiếu
cấu hình `/language:rust` so với `main` — chỉ là ghi chú, không chặn merge.

Đã cân nhắc:
- `paths-ignore` ở mức trigger `pull_request`: bỏ cả workflow (mất JS/TS và actions), và là danh sách đường dẫn thứ hai
  phải giữ khớp với `rust-free-paths.txt`.
- Bỏ quét ở mức step (job vẫn xin máy, chỉ bỏ bước analyze): tốn runner vô ích, và nếu bước quyết định hỏng thì dễ thành
  bỏ quét.
- Lọc JS/TS theo đường dẫn: bộ trích JS của CodeQL đọc nhiều loại file (`.html`, `.json`, `.yml`…), khó lập danh sách
  chắc chắn; mỗi lần chỉ chừng một phút nên không đáng.
- Job khác của `ci.yml` (image, MCP OAuth, upgrade, MSRV, luật cảnh báo) đã lọc qua job `plan` từ
  D-20261127-ci-runner-budget; `external module example` của gate.yml đã bỏ phần nặng khi PR không chạm Rust. Không đổi gì
  thêm, không đụng D-20261121-ci-batch-release-faster.
