---
name: record-decision
description: Ghi một quyết định có ảnh hưởng lâu dài vào docs/DECISIONS.md của Autoxide đúng mẫu, chỉ thêm không sửa. Dùng mỗi khi agent sắp hoặc vừa tự quyết một việc như đổi hay thêm thư viện, nâng phiên bản Rust hoặc thư viện lớn, thêm hay đổi một quy ước, cần ngoại lệ cho một luật trong CLAUDE.md, chọn giữa hai cách thiết kế, hoặc bỏ một mục khỏi lộ trình; cũng dùng khi chủ dự án chốt một điều mới ("chốt", "quyết định", "đổi sang", "record this decision"). Không dùng cho chi tiết cài đặt thường ngày mà đọc code là thấy.
---

# Ghi một quyết định

`docs/DECISIONS.md` là trí nhớ của dự án giữa các phiên. Agent phiên sau không
biết phiên này đã cân nhắc gì — nếu không ghi, nó sẽ quyết lại, có khi ngược lại.

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

1. Đọc `docs/DECISIONS.md`, lấy số của mục cuối cùng, cộng một.
2. Tìm xem đã có mục nào về cùng chủ đề chưa. Có thì mục mới phải ghi rõ nó thay
   mục nào.
3. Thêm vào **cuối file**, đúng mẫu:

   ```markdown
   ## D-NNN · <tiêu đề ngắn> — <YYYY-MM-DD> — <chủ dự án quyết | agent quyết>

   Quyết định: <làm gì, một hai câu, cụ thể tới mức kiểm tra được>

   Lý do: <vì sao; nêu số đo hoặc ràng buộc nếu có>

   Đã cân nhắc: <phương án bị loại và vì sao loại>

   Thay: D-MMM   ← chỉ khi mục này thay một mục cũ
   ```

4. **Không sửa, không xoá mục cũ**, kể cả khi nó đã bị thay. Lịch sử phải đọc lại
   được.
5. Dùng ngày thật của hôm nay, không ước chừng.

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
dài để ở `docs/DECISIONS.md`.

## Viết cho đúng

- Viết kết luận, không kể lại quá trình tìm ra nó.
- "Lý do" phải là thứ khiến người đọc sau hiểu vì sao không chọn cách hiển nhiên
  hơn. Nếu không viết ra được lý do, có thể đó chưa phải một quyết định đáng ghi.
- Số đo thì ghi kèm điều kiện đo (máy nào, bao nhiêu dòng, ngày nào).
