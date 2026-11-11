---
name: next-step
description: Nhận và hoàn thành bước kế tiếp trong lộ trình Autoxide (docs/ROADMAP.md) theo đúng quy trình — đọc lộ trình, làm, chạy gate, đánh dấu, commit. Dùng khi chủ dự án nói "bắt đầu", "làm tiếp", "tiếp tục", "bước tiếp theo", "continue", "next step", khi mở một phiên mới mà chưa rõ đang ở đâu, hoặc ngay sau khi vừa xong một mục và cần biết làm gì kế. Không dùng cho việc lẻ chủ dự án giao trực tiếp ngoài lộ trình.
---

# Làm bước kế tiếp của lộ trình

Lộ trình là nguồn duy nhất cho câu hỏi "làm gì tiếp". Đừng tự chọn việc theo
hứng, đừng nhảy cóc.

## 1. Xác định đang ở đâu

1. Đọc `docs/ROADMAP.md`. Bước hiện tại là bước có số nhỏ nhất còn mục `[ ]`;
   việc cần làm là mục `[ ]` đầu tiên trong bước đó.
2. Chạy `git status` và `git log --oneline -10`. Nếu cây làm việc có thay đổi
   chưa commit thì đó là việc dở của phiên trước: đọc diff, hoàn thành nó trước
   khi mở mục mới. Không vứt bỏ thay đổi dở.
3. Dòng "Trạng thái" đầu file ghi "chưa bắt đầu" mà chủ dự án chưa nói "bắt đầu"
   thì dừng lại hỏi — đó là quyết định của họ.

## 2. Chuẩn bị

- Đọc lại mục "Mục tiêu — không được quên" ở đầu `CLAUDE.md` và trả lời ba câu
  hỏi ở đó cho việc sắp làm. Không trả lời được "có" cho câu 1 thì đừng làm.

- Đọc lại phần liên quan trong `CLAUDE.md` (luật kiến trúc, dữ liệu, code) và
  `STACK.md` (thư viện dành cho việc này).
- Tìm trong `docs/DECISIONS.md` quyết định nào đã chốt cho mảng này.
- Mục cần một thư viện chưa từng dùng trong repo: chạy skill `check-library-api`
  **trước** khi viết code dùng nó.
- Repo đã có code: dùng CodeGraph (`codegraph explore "<tên hoặc câu hỏi>"`) để
  xem phần liên quan thay vì đọc mò từng file.

## 3. Làm

- Làm trọn một mục `[ ]`, kèm test của nó. Mục quá lớn thì chia thành nhiều
  commit, nhưng chỉ đánh dấu `[x]` khi cả mục xong.
- Gặp lựa chọn có ảnh hưởng lâu dài (đổi thư viện, thêm quy ước, cần ngoại lệ cho
  một luật): tự quyết rồi ghi lại bằng skill `record-decision`.
- Không dựng thứ nằm ngoài mục đang làm. Thấy việc khác cần làm thì thêm một dòng
  `[ ]` vào đúng bước trong lộ trình, không làm ngay.

## 4. Chạy gate

```bash
./gate.sh
```

- Đỏ thì sửa code. Không nới gate, không tắt lint, không đánh dấu test bỏ qua.
- `gate.sh` chưa tồn tại (đang ở bước 0): tạo nó là một mục của bước 0; trước đó
  chạy tay `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`.
- Gate phải xanh trên **cả SQLite và Postgres** mới tính là xong.

## 5. Đánh dấu và commit

1. Trong `docs/ROADMAP.md`: đổi `[ ]` thành `[x]` cho mục vừa xong. Cập nhật dòng
   "Trạng thái" đầu file thành bước và mục đang làm.
2. Thêm thư viện mới thì cập nhật `STACK.md`.
3. Commit cục bộ, chỉ `git add` đúng file đã sửa (không `git add -A`):

   ```
   step <số bước>: <việc đã xong, viết ngắn>

   <vì sao làm như vậy, nếu không hiển nhiên>
   ```

   Kết thúc thông điệp bằng dòng ghi công mà phiên làm việc yêu cầu.
4. **Không push** khi chủ dự án chưa yêu cầu.

## 6. Đi tiếp hay dừng

Đi tiếp sang mục kế mà không hỏi, trừ khi rơi vào một trong các trường hợp phải
dừng ghi ở `CLAUDE.md`: cần secret thật, cần trỏ tên miền, sắp làm việc không
hoàn tác được ở bên ngoài máy, hoặc muốn đảo một quyết định của chủ dự án.

Khi dừng (hết phiên hoặc phải hỏi), báo cho chủ dự án bằng lời thường: đã xong
mục nào, gate có xanh không, đang dở gì, cần họ làm gì. Họ mới học Rust — đừng
giả định họ đọc diff.
