# Số đo chất lượng (71-G9)

Các file ở đây là hợp đồng giữa code và các workflow đo chất lượng. Số trong đó **chỉ được đổi theo hướng tốt hơn**, trừ khi
chủ dự án hay phiên điều phối chủ ý chấp nhận kém đi (ghi lý do ở PR).

| File | Dùng cho | Workflow |
| ---- | -------- | -------- |
| `invariants.md` | Bất biến tiền, thuế, quyền: cái nào đã có property test, cái nào còn thiếu | `property.yml` (hằng đêm) |
| `mutation.json` | Phạm vi, số shard và ngưỡng mutant sống của mutation testing | `mutants.yml` (thứ Bảy) |
| `load-baseline.json` | Số gốc p95 của các đường nóng, mỗi database một cột | `load.yml` (thứ Hai) |
| `first-module-timing.json` | Số đo "người mới xong module đầu tiên" (người thật và agent, hai số riêng) | `release-metrics.yml` |
| `agent-eval.schema.json` | Định dạng file kết quả eval agent (`agent-eval.json`) | `release-metrics.yml` |

## Đọc số mutation cho đúng

`cargo-mutants` sửa từng chỗ trong code rồi chạy test; **mutant sống** là chỗ sửa mà test không bắt được. Tỉ lệ sống hiện
cao (khoảng 30% số mutant biên dịch được ở nhiều nhóm) nhưng đừng hiểu đó là "70% code không được kiểm":

- Workflow chỉ chạy `--lib` (unit test) trên SQLite. Mutant mà chỉ **test tích hợp** (thư mục `tests/`) hay **đường riêng của
  PostgreSQL** mới bắt được sẽ hiện là sống dù có test thật. Chạy thêm hai thứ đó là việc để sau (ROADMAP, 71-G9).
- Một phần mutant sống là thay đổi không đổi hành vi (so sánh tương đương, thứ tự không quan trọng): cần người xem, không
  phải test nào cũng nên thêm.
- Mutant "quá giờ" được đếm riêng, không tính là sống.
- Ngưỡng là **số mutant sống tối đa** của từng nhóm, chỉ để ngăn tệ đi. Mỗi lần thêm test bắt được mutant, hạ ngưỡng
  xuống số mới đo.
- Một nhóm thiếu shard (runner mất, quá giờ) bị báo đỏ vì thiếu số đo, không phải vì code tệ đi.
