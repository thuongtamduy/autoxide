# Bất biến tiền, thuế, quyền — cái nào đã có property test

Rà ngày 26/11/2026 (71-G9, D-20261126-independent-review). Mọi property test (`proptest!`) nằm cạnh hàm được thử và chạy
hai nơi: trong gate (256 ca mỗi test) và hằng đêm ở `.github/workflows/property.yml` (`cargo test --workspace --lib`,
20 000 ca). Test mới thuộc `--lib` nên tự vào lượt chạy đêm, không cần khai thêm.

Phần chạm database (ghi bút toán thật, số dư, cô lập khách) là test tích hợp trên cả hai database, không phải property test.

## Tiền

| Bất biến | Nơi giữ |
| -------- | ------- |
| Chia theo tỉ lệ: tổng các phần đúng bằng số gốc, cùng dấu, lệch dưới một đơn vị nhỏ nhất | `crates/kernel/src/values/money.rs` |
| Chia đều: các phần lệch nhau tối đa một đơn vị, tổng đúng | `money.rs` |
| Cộng / trừ / nhân tràn thì báo lỗi, không panic; cộng rồi trừ trả lại số cũ; cấm cộng khác loại tiền | `money.rs` |
| Đọc số tiền không panic; đọc lại đúng số đã ghi (chuỗi và JSON) | `money.rs` |
| Số thập phân: làm tròn, đọc / ghi | `crates/kernel/src/values/decimal.rs` |
| Cửa kiểm bút toán (phần thuần, không database): nợ và có chia độc lập thành số dòng khác nhau, trộn thứ tự, vẫn qua; lệch một đơn vị là `ledger.unbalanced`; một dòng khác loại tiền là `ledger.currency_mixed`; dòng 0 hay âm bị chặn (`validation_failed`) dù bút toán vẫn cân; số cực lớn không panic, tràn thì báo `money.overflow`. Chưa phủ: kiểm khoá tài khoản, ngày, kỳ đóng, đảo bút toán (cần database) | `modules/ledger/src/posting_properties.rs` (mới) |
| Khuyến mãi không làm giá âm, không giảm quá số gốc | `modules/promotions/src/engine/tests.rs` |
| Vòng đời đơn hàng chỉ đi theo các cạnh hợp lệ | `modules/orders/src/lifecycle.rs` |

## Thuế

| Bất biến | Nơi giữ |
| -------- | ------- |
| Tổng dòng khớp tổng chứng từ; gross = net + thuế; không có thuế âm, net âm; 0% và không chịu thuế ra 0; tổng theo loại thuế khớp | `modules/tax/src/engine/properties.rs` |
| Số lượng quá lớn là lỗi đầu vào, không panic; không nhân phần dư làm tròn | `properties.rs` |

## Quyền

| Bất biến | Nơi giữ |
| -------- | ------- |
| `only` là phép giao: không bao giờ cấp thêm, grant giữ nguyên, quyền giữ lại thì `require` qua | `crates/kernel/src/security/access/properties.rs` (mới) |
| Cấm thắng cho phép (kể cả người có mọi quyền); quyền không bị cấm giữ nguyên cả grant và `require` qua; lỗi `permission_denied` | `access/properties.rs` (mới) |
| Rào của khách cha (`within`) không cấp thêm, không vượt tập cho phép | `access/properties.rs` (mới) |
| Cấm và rào áp theo thứ tự nào cũng ra cùng tập quyền | `access/properties.rs` (mới) |
| Bản chụp người gọi (job chạy sau) khôi phục đúng quyền, grant (nút, độ sâu), chủ, khách, API key, hàng rào, realm, qua JSON; người gọi được sinh với độ sâu Own/Unit/Subtree/Tenant ở nhiều nút, nhiều grant mỗi quyền, tên quyền ngoài danh mục | `access/properties.rs` (mới) |
| API key (token), phiên, JWT, ký webhook: không panic với đầu vào bậy, vòng đọc / ghi nguyên vẹn | `modules/auth/...`, `modules/webhooks/src/signature.rs` |

## Còn thiếu (để sau, ghi để không quên)

- **Quyết định theo nút (`require_at`, `scope_condition`, hàng rào `fence` khi kiểm một bản ghi)**: logic ở
  `access/decision.rs` cần cây tổ chức; property test ở trên chỉ giữ grant không đổi qua các phép thu hẹp, chưa kiểm
  "quyền ở nút `n` không bao giờ cho đọc nút ngoài cây của `n`".
- **Cô lập khách (`tenant_id`)**: do gate `tenant-scope` chặn ở mức mã nguồn và test tích hợp hai khách; chưa có
  property test sinh ngẫu nhiên chuỗi thao tác hai khách trên database (tốn: cần database, nên để workflow riêng).
- **Hoá đơn (`invoices`) và đơn hàng (`orders`)**: tổng chứng từ lấy từ engine thuế nên đã được phủ gián tiếp; chưa có
  property test cho làm tròn theo tờ hoá đơn có chiết khấu dòng.
- **Quy đổi ngoại tệ của sổ cái (`fx.rs`)**: bất biến "bút toán ngoại tệ quy đổi vẫn cân ở tiền sổ" chưa có property test.
