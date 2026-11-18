---
name: next-step
description: Làm bước kế tiếp trong docs/ROADMAP.md theo đúng quy trình: đọc lộ trình, làm, chạy gate, đánh dấu, commit. Dùng khi chủ dự án nói "làm tiếp", "tiếp tục", "bước tiếp theo", "next step", hay khi mở phiên mới chưa rõ đang ở đâu. Không dùng cho việc lẻ ngoài lộ trình.
---

# Làm bước kế tiếp của lộ trình

Lộ trình là nguồn duy nhất cho câu hỏi "làm gì tiếp". Đừng tự chọn việc theo
hứng, đừng nhảy cóc.

## 1. Xác định đang ở đâu

1. Đọc `docs/ROADMAP.md`. Các bước xếp theo **thứ tự làm**, không theo số (D-228). Bước hiện tại là
   bước đầu tiên từ trên xuống còn mục `[ ]` mà các bước nó phụ thuộc (bảng "Phụ thuộc giữa các bước")
   đã xong; việc cần làm là mục `[ ]` đầu tiên trong bước đó. Bước đã xong nằm ở `docs/roadmap/done.md`.
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
- Tìm trong mục lục `docs/decisions/INDEX.md` (theo chủ đề, có đánh dấu mục đã bị thay)
  quyết định nào đã chốt cho mảng này, rồi đọc đúng mục đó.
- Mục cần một thư viện chưa từng dùng trong repo: chạy skill `check-library-api`
  **trước** khi viết code dùng nó.
- Repo đã có code: dùng CodeGraph (`codegraph explore "<tên hoặc câu hỏi>"`) để
  xem phần liên quan thay vì đọc mò từng file.

## 3. Làm

- Đặc tả của bước có thứ tự làm và danh sách "Test bắt buộc": trước khi code, xem mục nào thuộc crate riêng và
  không phụ thuộc nhau — đủ CPU, RAM, đĩa thì giao song song theo skill `split-work` (D-299).
- Viết test của mục cùng lúc với code (lấy tên từ "Test bắt buộc"), chạy riêng bài đang làm
  (`cargo test -p <crate> <tên>`) trong lúc viết; chạm database thì chạy thêm bài đó trên Postgres.
- Làm trọn một mục `[ ]`, kèm test của nó. Mục quá lớn thì chia thành nhiều
  commit, nhưng chỉ đánh dấu `[x]` khi cả mục xong.
- Gặp lựa chọn có ảnh hưởng lâu dài (đổi thư viện, thêm quy ước, cần ngoại lệ cho
  một luật): tự quyết rồi ghi lại bằng skill `record-decision`.
- Không dựng thứ nằm ngoài mục đang làm. Thấy việc khác cần làm thì thêm một dòng
  `[ ]` vào đúng bước trong lộ trình, không làm ngay.

## 4. Chạy gate

```bash
./gate.sh --quick   # mỗi nhóm việc, trước khi commit và trước khi merge: crate bị sửa, chỉ SQLite
./gate.sh           # chỉ trước khi phát hành: đầy đủ, cả hai database (D-300)
```

- **Không ngồi chờ gate** (D-299): chạy nó ở chế độ nền. Trong lúc chờ chỉ đọc và chuẩn bị mục kế (đặc tả, code
  liên quan, `check-library-api`) — không sửa file, vì gate đang đọc chính thư mục làm việc.
- Gate nhanh chỉ test crate đổi kể từ lần nó xanh gần nhất (ref cục bộ `refs/autoxide/quick-green`), nên chạy nó
  một lần mỗi nhóm việc là đủ (D-263).
- Đỏ thì sửa code. Không nới gate, không tắt lint, không đánh dấu test bỏ qua.
- `gate.sh` chưa tồn tại (đang ở bước 0): tạo nó là một mục của bước 0; trước đó
  chạy tay `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`.
- Bước xong khi mọi mục `[x]` và gate nhanh xanh; gate **đầy đủ** (cả SQLite và Postgres) chỉ bắt buộc
  trước khi phát hành (D-300). Code chạm database vẫn phải đúng trên cả hai: chạy riêng bài test đó trên
  Postgres lúc viết (`TEST_DATABASE_URL=postgres://… cargo test -p <crate> <tên>`).

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
4. Mục vừa xong là mục cuối của bước (mọi mục của bước đã `[x]`): gate nhanh xanh thì
   **merge vào `main`**, không phát hành, không chờ gate đầy đủ (D-300). Chỉ phát hành (theo `docs/RELEASING.md`, không hỏi) khi
   bước đó là bước cuối của một giai đoạn của lộ trình, hay khi chủ dự án bảo (D-248).

## 6. Đi tiếp hay dừng

Đi tiếp sang mục kế mà không hỏi, trừ khi rơi vào một trong các trường hợp phải
dừng ghi ở `CLAUDE.md`: cần secret thật, cần trỏ tên miền, sắp làm việc không
hoàn tác được ở bên ngoài máy, hoặc muốn đảo một quyết định của chủ dự án.

Khi dừng (hết phiên hoặc phải hỏi), báo cho chủ dự án bằng lời thường: đã xong
mục nào, gate có xanh không, đang dở gì, cần họ làm gì. Họ mới học Rust — đừng
giả định họ đọc diff.
