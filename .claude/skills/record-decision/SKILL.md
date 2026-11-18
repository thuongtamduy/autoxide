---
name: record-decision
description: Ghi quyết định có ảnh hưởng lâu dài thành một file docs/decisions/D-<ngày>-<tên>.md đúng mẫu, chỉ thêm không sửa. Dùng khi đổi hay thêm thư viện, đổi quy ước, cần ngoại lệ cho luật trong CLAUDE.md, chọn giữa hai thiết kế, bỏ mục lộ trình, hoặc chủ dự án "chốt" một điều. Không dùng cho chi tiết cài đặt thường ngày.
---

# Ghi một quyết định

Nhật ký quyết định là trí nhớ của dự án giữa các phiên. Agent phiên sau không
biết phiên này đã cân nhắc gì — nếu không ghi, nó sẽ quyết lại, có khi ngược lại.

Nhật ký có hai phần (D-20261118-decision-ids):

- `docs/DECISIONS.md` — **nhật ký cũ, đã đóng**: D-001…D-381, số tuần tự. Đọc và trích được,
  **không thêm mục mới vào đó**.
- `docs/decisions/D-<YYYYMMDD>-<tên-ngắn>.md` — **mỗi quyết định mới một file**.
- `docs/decisions/INDEX.md` — mục lục theo chủ đề của cả hai, sinh bằng `cargo xtask decisions`
  (không sửa tay). Đọc mục lục trước khi tìm.

Vì sao không dùng số tuần tự nữa: nhiều phiên agent chạy song song trên các nhánh cùng gốc đều
thấy cùng "số cuối", nên luôn chọn trùng số kế tiếp (đã xảy ra với D-365, D-372, D-373, D-377).
ID theo ngày và tên do chính phiên chọn thì không cần hỏi ai, và mỗi mục một file nên git không
xung đột khi gộp.

## Khi nào ghi

Ghi khi quyết định đó **một phiên sau có thể vô tình đảo ngược**:

- thêm, bỏ hoặc đổi một thư viện; nâng phiên bản lớn; nâng toolchain Rust
- thêm hoặc đổi một quy ước (đặt tên, cấu trúc thư mục, cách viết test)
- cần ngoại lệ cho một luật trong `CLAUDE.md`
- chọn một cách thiết kế khi có cách khác cũng hợp lý
- thêm, bỏ hoặc đổi thứ tự một mục trong `docs/ROADMAP.md`

Không ghi: tên biến, cách chia hàm, sửa lỗi — những thứ đọc code và lịch sử git
là đủ.

## Ai được quyết gì

- Mục ghi **agent quyết**: agent sau được thay bằng một mục mới nếu có lý do.
- Mục ghi **chủ dự án quyết**: agent **không** tự đảo. Thấy cần đổi thì dừng lại,
  trình bày lý do bằng lời thường và chờ họ trả lời; họ đồng ý rồi mới ghi mục
  mới với nhãn "chủ dự án quyết".

## Cách ghi

1. Mở `docs/decisions/INDEX.md`, tìm mục cùng chủ đề. Có thì mục mới phải ghi rõ nó
   thay mục nào (dòng `Thay:`), và mục lục sẽ tự đánh dấu mục cũ "thay bởi …".
2. Chọn ID: `D-` + **ngày hôm nay** dạng `YYYYMMDD` + `-` + **tên ngắn** (2–4 từ tiếng Anh
   hay tiếng Việt không dấu, chữ thường `a-z0-9` nối bằng `-`, tối đa 40 ký tự), vd
   `D-20261118-decision-ids`. Không dùng số tuần tự. File `docs/decisions/<ID>.md` đã có (trùng
   tên trong cùng ngày) thì chọn tên khác.
3. Tạo file `docs/decisions/<ID>.md`, **một mục duy nhất**, đúng mẫu:

   ```markdown
   ## D-YYYYMMDD-ten-ngan · <tiêu đề ngắn> — <YYYY-MM-DD> — <chủ dự án quyết | agent quyết>

   Chủ đề: <một tên chủ đề có trong docs/decisions/INDEX.md, vd Quy trình, gate, lộ trình>

   Quyết định: <làm gì, một hai câu, cụ thể tới mức kiểm tra được>

   Lý do: <vì sao; nêu số đo hoặc ràng buộc nếu có>

   Đã cân nhắc: <phương án bị loại và vì sao loại>

   Thay: D-MMM   ← chỉ khi mục này thay một mục cũ; thay một phần: `Thay: phần "…" của D-MMM`
   ```

   Ngày trong ID phải trùng ngày trong tiêu đề. Dòng `Chủ đề:` không bắt buộc (thiếu thì mục
   lục đoán từ tiêu đề) nhưng nên có; tên lạ thì gate đỏ và in danh sách tên hợp lệ.
4. Chạy `cargo xtask decisions` để sinh lại mục lục, commit cả hai file.
5. Trích quyết định trong code và tài liệu bằng nguyên ID: `(D-20261118-decision-ids)`.
   `cargo xtask check` (trong gate) chặn ID trùng, ID sai dạng, file sai tên, mục lục lệch, và
   tham chiếu tới ID không có — trích nhầm số là gate đỏ.
6. **Không sửa, không xoá mục cũ**, kể cả khi nó đã bị thay. Lịch sử phải đọc lại
   được.
7. Dùng ngày thật của hôm nay, không ước chừng.

Nhánh cắt trước quy ước này có thể đã thêm mục số tuần tự (lớn hơn 381) vào `docs/DECISIONS.md`:
vẫn hợp lệ khi gộp nếu không trùng số; trùng thì gate đỏ, đổi mục của nhánh mình sang file
riêng theo quy ước trên và sửa các chỗ trích nó.

## Sau khi ghi

Quyết định làm thay đổi thứ đang được mô tả ở chỗ khác thì sửa luôn chỗ đó, trong
cùng commit:

| Quyết định về            | Sửa thêm                                   |
| ------------------------ | ------------------------------------------ |
| Thư viện, phiên bản      | `STACK.md`                                 |
| Luật, quy ước, ngoại lệ  | `CLAUDE.md`                                |
| Việc phải làm, thứ tự    | `docs/ROADMAP.md`                          |
| Toolchain Rust           | `rust-toolchain.toml`                      |

`CLAUDE.md` được nạp vào mọi lượt nên chỉ thêm vào đó **luật** ngắn gọn; lý do
dài để ở file của quyết định.

## Viết cho đúng

- Viết kết luận, không kể lại quá trình tìm ra nó.
- "Lý do" phải là thứ khiến người đọc sau hiểu vì sao không chọn cách hiển nhiên
  hơn. Nếu không viết ra được lý do, có thể đó chưa phải một quyết định đáng ghi.
- Số đo thì ghi kèm điều kiện đo (máy nào, bao nhiêu dòng, ngày nào).
