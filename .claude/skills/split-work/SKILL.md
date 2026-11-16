---
name: split-work
description: Quyết định có chia việc cho subagent không, chia bao nhiêu và chia sao cho an toàn. Dùng trước khi gọi Agent / subagent / workflow, khi một mục lớn có nhiều phần độc lập, khi cần rà soát bằng cặp mắt khác, hoặc chủ dự án nói "chia agent", "chạy song song". Không dùng cho việc nhỏ.
---

# Chia việc cho subagent

Chủ dự án cho phép chia agent và subagent trong dự án này, **bao nhiêu cũng được**,
miễn là hợp với tài nguyên máy và độ khó của việc. Chia là để nhanh hơn hoặc chắc
hơn — không phải để trông bận rộn.

## 1. Có nên chia không

Chia khi:

- việc gồm **nhiều phần độc lập** (mỗi phần một crate hoặc một nhóm file riêng,
  không sửa chung file);
- cần **tra cứu rộng** (nhiều thư viện, nhiều vùng code) mà chỉ cần kết luận;
- cần **rà soát độc lập**: người viết và người rà là hai agent khác nhau.

Không chia khi:

- việc nhỏ, hoặc các bước phụ thuộc nối tiếp nhau;
- nhiều phần cùng sửa một file (`Cargo.toml` gốc, file đăng ký module, migration);
- chưa hiểu việc đủ rõ để viết lời giao việc cụ thể — hiểu trước đã.

## 2. Chia bao nhiêu — theo máy đang chạy

Không có con số cố định. Mỗi người dev một máy khác nhau; số agent phải co giãn
theo **máy đang chạy phiên này, tại thời điểm này**. Đo trước mỗi lần chia:

```bash
nproc                                   # số CPU logic
free -g | awk 'NR==2{print $7}'         # RAM còn dùng được (GB)
df -h .                                 # đĩa còn trống
```

(macOS: `sysctl -n hw.ncpu` và `memory_pressure`; Windows: dùng WSL.)

Biên dịch Rust là thứ ăn tài nguyên: một lượt `cargo build` / `cargo test` của
stack này dùng nhiều nhân và vài GB RAM. Vì vậy tách hai loại agent, với điểm xuất
phát gợi ý:

| Loại agent                                   | Điểm xuất phát                                      |
| -------------------------------------------- | --------------------------------------------------- |
| **Có biên dịch** (viết code, chạy test)      | `min(nproc / 4, RAM còn dùng được / 4)`, tối thiểu 1 |
| **Chỉ đọc** (tra cứu, đọc code, rà soát)     | khoảng `nproc / 2`                                  |

**Đĩa** cũng là trần: mỗi worktree biên dịch có `target/` riêng, khoảng 25–30 GB sau một lượt test đầy đủ (thư viện
ở bản tối ưu nhẹ cộng cache incremental). Số agent biên dịch không vượt `đĩa trống / 30 GB`; hết đĩa thì xoá
`target/` của worktree đã xong trước.

Đây là điểm xuất phát, không phải trần cứng. Máy mạnh thì chia nhiều hơn, máy yếu
thì ít hơn; máy đang bận việc khác thì tính theo phần còn rảnh. Rồi **nhìn máy mà
chỉnh**:

- Còn dư CPU và RAM, build không chậm đi → tăng thêm agent được.
- Máy bắt đầu dùng swap, build chậm hẳn, test timeout lạ → giảm số agent biên
  dịch. Đừng tăng timeout để che.

Độ khó của việc cũng quyết định: việc gồm nhiều phần độc lập thì chia rộng; việc
khó nhưng liền một khối thì thường cần **một** agent làm kỹ cộng một agent rà soát,
không phải nhiều agent cùng làm.

Không ghi số đo của một máy cụ thể vào repo: nó sai với người dev kế tiếp.

Đặc tả bước (`docs/steps/`) đã xếp thứ tự làm: mục nào chỉ sửa một crate mới hay crate không ai khác sửa trong
bước thì là ứng viên chạy song song (vd bước 34: module `address` song song với nền kernel). Các mục cùng sửa một crate
(vd nhiều mục của `auth`) thì một agent làm nối tiếp.

## 3. Chia thế nào cho không giẫm nhau

- **Mỗi agent biên dịch một git worktree riêng** (tham số `isolation: "worktree"`
  khi gọi Agent). Hai agent build chung một thư mục `target/` sẽ khoá nhau và có
  thể sửa đè file của nhau.
- **Ranh giới chia = ranh giới crate.** Mỗi agent một crate hoặc một module; không
  agent nào sửa file của agent khác.
- **File dùng chung do agent chính giữ:** `Cargo.toml` gốc, `Cargo.lock`, chỗ đăng
  ký module, `docs/ROADMAP.md`, `docs/DECISIONS.md`, `STACK.md`, `CLAUDE.md`.
  Subagent cần đổi những file này thì báo lại, agent chính sửa.
- **Migration không chia song song** trong cùng một module: thứ tự là một phần của
  nội dung.
- Subagent **không** commit vào nhánh chính, không push. Agent chính gộp kết quả,
  chạy `./gate.sh` trên bản đã gộp rồi mới commit.

## 4. Giao việc

Subagent không biết gì ngoài lời giao việc. Mỗi lời giao phải có:

1. **Mục tiêu** của phần việc, và nó nằm ở đâu trong mục tiêu chung.
2. **Phạm vi**: được sửa những file / crate nào; cấm sửa gì.
3. **Luật phải đọc**: `CLAUDE.md` (nhất là luật số một về SQLite và Postgres), và
   skill `check-library-api` trước khi dùng API thư viện.
4. **Điều đã biết và đã loại**, để nó không làm lại.
5. **Kết quả phải trả về**: đã sửa file nào, test nào đã chạy và kết quả, điều gì
   chưa chắc.

Subagent cũng phải tuân mọi luật của `CLAUDE.md`. Giao việc không làm luật nhẹ đi.

## 5. Nhận kết quả

- Báo cáo của subagent là **lời khai, chưa phải bằng chứng.** Tự chạy lại test và
  đọc phần thay đổi chính trước khi tin.
- Hai agent cho kết luận trái nhau thì kiểm chứng bằng code hoặc lệnh, không chọn
  theo bên nói tự tin hơn.
- Gate phải xanh trên **bản đã gộp**, trên cả SQLite và Postgres. Từng phần xanh
  riêng không đảm bảo gộp lại vẫn xanh.
- Quyết định có ảnh hưởng lâu dài mà subagent đã tự đưa ra: agent chính xem lại
  rồi ghi bằng skill `record-decision`.
