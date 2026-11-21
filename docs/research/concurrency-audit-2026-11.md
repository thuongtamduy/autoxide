# Rà lỗi đua chỉ lộ trên Postgres — tháng 11/2026

Ngày rà: 20/11/2026. Phạm vi: `crates/kernel` và mọi module trong `modules/`.

## Vì sao có bản rà này

CI trên Postgres bắt hai lỗi đồng thời mà test trên SQLite không thấy:

- #246 `catalog` storefront: hai lần dựng lại cùng một sản phẩm cùng đọc "chưa có dòng", rồi cùng
  ghi event `catalog.display_price_changed`. Sửa bằng `rows::lock` dòng sản phẩm trước khi đọc.
- #253 luồng sự kiện: event `Last-Event-ID` bị gửi lại khi nối lại.

Gốc chung: trên SQLite mọi transaction ghi (`Database::begin_write`) là `BEGIN IMMEDIATE`, tức mỗi
lúc chỉ một bên ghi, nên "đọc → xét → ghi" luôn đúng. Trên Postgres (mức cô lập *read committed*),
hai transaction cùng đọc trạng thái cũ rồi cùng ghi. Lỗi kiểu này chỉ lộ khi chạy trên Postgres có
tải đồng thời.

Mẫu lỗi đi tìm: **trong transaction ghi, đọc trạng thái → so sánh / quyết định → ghi** (dòng, event
outbox, số dư, bộ đếm, trạng thái máy) mà không khoá dòng, không cập nhật có điều kiện, và không có
unique index chặn bản trùng.

Công cụ chuẩn để sửa (D-181, `crates/kernel/src/data/rows.rs`):

- `rows::lock` / `lock_one` / `lock_one_shared`: khoá dòng (`FOR UPDATE` / `FOR SHARE` trên Postgres);
- `rows::adjust`: cộng trừ có điều kiện trong một câu;
- `update_many … filter(<trạng thái cũ>)` rồi kiểm `rows_affected`;
- unique index kèm bắt lỗi trùng;
- `Database::write_with_retry`.

Cách rà: ba agent chỉ đọc chia theo nhóm module (thương mại; tài chính / pháp lý / tài khoản; kernel
và nội dung), mỗi agent đối chiếu migration để biết chỗ nào đã có unique index. Agent chính kiểm lại
từng lỗi trước khi sửa: viết test đồng thời (`tokio::join!`), chạy trên Postgres để thấy đỏ, sửa,
rồi xác nhận xanh trên cả hai database.

## Trạng thái

- **Đã sửa**: có test đồng thời (`tokio::join!`), xanh trên cả hai database sau khi sửa (Postgres
  lặp 3 lần), và toàn bộ test của module xanh trên cả hai. Cột trạng thái ghi số lần test đỏ trên
  Postgres khi tạm gỡ phần sửa. Vài chỗ test không tái hiện được thứ tự đua (khoảng hở quá hẹp khi
  đi qua HTTP hay job); chỗ đó sửa theo phân tích, test giữ lại để chặn hồi quy.
- **Chưa sửa**: còn mở, xếp theo mức nghiêm trọng để phiên sau làm tiếp.
- **Cả hai DB**: lỗi không riêng Postgres. Bước kiểm nằm ngoài transaction, nên SQLite cũng sai.

## Kết quả

- 39 ứng viên chính (cộng một nhóm mức thấp), từ ba lượt rà chỉ đọc theo nhóm module.
- 20 ứng viên đã sửa, 2 sửa một phần, trong 13 PR theo module: #259 (`support`), #263 (`shipping`), #264
  (`invoices`), #265 (`bookings`), #266 (`compliance`), #267 (`promotions`), #268 (`notifications`),
  #269 (`comments`), #270 (`ledger`), #271 (`orders`), #272 (`automations`), #273 (`catalog`), #274
  (`auth`).
- Mỗi PR có test đồng thời. Ba chỗ test không tái hiện được thứ tự đua (mục 3, 29, 34): sửa theo
  phân tích, test giữ để chặn hồi quy.
- Ứng viên còn lại ghi "Chưa sửa", xếp theo mức nghiêm trọng, để phiên sau làm tiếp. Nên làm trước các
  mục 6, 8, 9, 10, 14, 16, 18, 31.

## Ứng viên, xếp theo mức nghiêm trọng

### Tiền

| # | Chỗ | Mẫu | Hậu quả khi đua | Trạng thái |
|---|-----|-----|-----------------|------------|
| 1 | `modules/shipping/src/cod.rs` `import_statement`, `settle_shipment` | Đọc vận đơn không khoá, xét `cod_status != settled`, ghi lại cả dòng (`reset_all`), phát `shipping.cod_settled`, xếp job ghi sổ theo `line_id` | Hai bảng kê khác mã cùng có một tracking: vận đơn chốt hai lần, sổ cái ghi có tiền COD **hai lần** (hai `line_id` khác nhau), event trùng | Đã sửa (#263) — đỏ 3/3 trên Postgres |
| 2 | `modules/invoices/src/issue.rs` `issue_inner` (chứng từ thay thế / điều chỉnh) | Chỉ khoá dòng nháp; chứng từ gốc bị ghi `replaced` / `adjusted` bằng `update_many` không điều kiện trạng thái | Hai nháp thay thế cùng một hoá đơn đều phát hành: doanh thu nhân đôi; thay thế đè `cancelled` | Đã sửa (#264) — đỏ trên cả hai DB |
| 3 | `modules/bookings/src/jobs.rs` `deposit_paid` ↔ `booking.rs` `expire` | Đọc lịch `held` không khoá, ghi cả dòng; điều kiện chỉ có `deposit_status` | Lịch đã hết hạn và slot đã nhả cho người khác bị ghi lại thành `confirmed` | Đã sửa (#265) — test qua job/event không tái hiện được thứ tự đua; sửa theo phân tích |
| 4 | `modules/shipping/src/cod.rs` `resolve_discrepancy` | Đọc `status != resolved` không khoá, update không điều kiện | Hai người xử lý cùng lúc: job xoá nợ đã xếp nhưng hồ sơ ghi cách xử lý khác; event trùng | Đã sửa (#263) — đỏ 3/3 trên Postgres |
| 5 | `modules/ledger/src/posting.rs` `write` ↔ `service.rs` `update_account` | Ghi bút toán đọc `floor_minor` / `status` của tài khoản không khoá | Số dư xuống dưới sàn mới vừa đặt; ghi lọt vào tài khoản vừa đóng | Đã sửa (#270) — đỏ 2/3 trên Postgres |
| 6 | `modules/promotions/src/participations.rs` `review_participation`, `participate` | Đọc `status` không khoá rồi ghi | Duyệt và từ chối đua nhau: giá flash có hiệu lực nhưng hồ sơ `rejected` | Đã sửa (lượt 2, `promotions`): khoá dòng chương trình khi đăng ký và duyệt — đỏ 1/3 trên Postgres |
| 7 | `modules/plans/src/usage.rs` `reconcile_one` | Ghi `used = target` tuyệt đối | `reserve` / `release` chen giữa bị ghi đè, khách vượt hạn mức | Đã sửa (lượt 2, `plans`, D-20261121-plans-reconcile-snapshot): chụp sổ trước khi đếm, khoá dòng lượng dùng và chụp lại; sổ đổi thì bỏ lượt, thử lại — test chặn lần đếm giữa chừng, đỏ trên cả hai DB |
| 8 | `modules/plans/src/billing.rs` `invoice_period` ↔ `lifecycle.rs` `void_period` | Kiểm `status != void` ngoài transaction, gán `invoice_id` không điều kiện | Kỳ đã huỷ vẫn có hoá đơn GTGT không bị huỷ | Đã sửa (lượt 2, `plans`): gắn hoá đơn dưới khoá dòng kỳ (cùng khoá với `void_period`); kỳ đã huỷ thì huỷ luôn hoá đơn vừa phát hành — test chặn `issue` giữa chừng, đỏ trên cả hai DB |
| 9 | `modules/orders/src/checkout.rs` chuyển giỏ sang `converted` | Không điều kiện status | Hai lần đặt hàng khác idempotency key trên cùng giỏ ra hai đơn | Đã sửa (lượt 2, `orders`): đóng giỏ đầu transaction tạo đơn, có điều kiện giỏ còn mở, không đổi được thì huỷ — đỏ 3/3 trên cả hai DB |
| 10 | `modules/orders/src/pos.rs` `close_shift` ↔ `create_pos_order` | Tiền kỳ vọng tính ngoài transaction; lượt bán không kiểm ca còn mở | Lệch số dư két, bán vào ca đã đóng | Đã sửa (lượt 2, `orders`): lượt bán giữ khoá chia sẻ dòng ca (chỉ khi ca mở), đóng ca khoá độc quyền rồi mới cộng tiền kỳ vọng — đỏ 3/3 trên cả hai DB |

### Tồn kho / sức chứa

| # | Chỗ | Mẫu | Hậu quả khi đua | Trạng thái |
|---|-----|-----|-----------------|------------|
| 11 | `modules/bookings/src/actions.rs` `reschedule`, `cancel` | `save` chỉ điều kiện `status` không đổi; `reschedule` giữ nguyên status | Hai lần đổi giờ đua nhau: rò slot, `sessions.booked` bị trừ hai lần, lớp học nhận vượt sức chứa | Đã sửa (#265) — đỏ 2/9 trên Postgres |
| 12 | `modules/bookings/src/booking.rs` `reserve` (lớp sức chứa N) | Hai người đầu tiên cùng mở một buổi; bên sau vướng unique | Bên sau nhận `bookings.slot_taken` dù lớp còn chỗ | Chưa sửa (thấp) |

### Pháp lý

| # | Chỗ | Mẫu | Hậu quả khi đua | Trạng thái |
|---|-----|-----|-----------------|------------|
| 13 | `modules/compliance/src/kyc.rs` `apply_outcome` | Đọc lượt xác thực `pending` không khoá; không kiểm trạng thái hồ sơ | Hai callback cùng lượt: event `kyc_verified` trùng. Kết quả muộn của lượt cũ xác thực lại hồ sơ đã thu hồi (cả hai DB) | Đã sửa: callback trùng (#266, đỏ 7/9 trên Postgres); kết quả muộn (lượt 2, `compliance`): hồ sơ không còn chờ đúng lượt này (đã duyệt tay, thu hồi, mở lượt mới) thì lượt ghi `superseded`, không đụng hồ sơ — đỏ mọi lần trên cả hai DB |
| 14 | `modules/compliance/src/kyc.rs` kiểm giấy tờ trùng | Index `document_hash` không unique; đọc rồi quyết | Hai hồ sơ cùng một CCCD xác thực cùng lúc đều `verified` | Đã sửa (lượt 2, `compliance`, migration `m0020_kyc_verified_document`): cột `verified_document_hash` unique theo khách, vướng unique thì chạy lại và thấy hồ sơ kia — đỏ 1/3 trên Postgres |
| 15 | `modules/compliance/src/tracking.rs` `refresh_banner` | Đọc `cookie_setting` không khoá, ghi `policy_version + 1` tuyệt đối | Hai lần thêm tag chỉ tăng version một lần: người dùng không bị hỏi đồng ý lại cookie (NĐ 13/2023) | Đã sửa (#266) — đỏ 1/9 trên Postgres |
| 16 | `modules/compliance/src/cases.rs` `save_case` / `restricted_elsewhere` | Quyết phát `TargetRestored` theo "còn vụ khác đang ẩn" mà không khoá các vụ đó | Hai vụ cùng ẩn một đích khôi phục cùng lúc: nội dung bị ẩn vĩnh viễn | Đã sửa (lượt 2, `compliance`): khoá mọi vụ cùng đích (theo ID) trước khi xét "còn vụ khác đang ẩn", cả khi mở vụ có ẩn — đỏ 5/9 trên Postgres |
| 17 | `modules/catalog/src/products.rs` `set_status` (Active) ↔ `restrictions.rs` `delete_document` | `product_for_write` đọc không khoá | Sản phẩm ngành hạn chế mở bán khi giấy tờ bắt buộc vừa bị xoá | Một phần (#273): mọi lệnh sửa sản phẩm / giấy tờ khoá dòng sản phẩm. Đổi hạn chế của danh mục (nhiều sản phẩm) chưa khoá |
| 18 | `modules/promotions/src/compliance.rs` (kiểm khi kích hoạt) | Chạy ngoài transaction | Hai lần kích hoạt cùng lúc cùng vượt trần khuyến mại pháp lý | Đã sửa (lượt 2, `promotions`): khi ghi kích hoạt khoá dòng phiên bản của khách rồi kiểm lại tổng số ngày giảm giá; chương trình đổi giữa lúc kiểm và lúc ghi thì 409 — đỏ 3/3 trên Postgres và trên SQLite |

### Bảo mật

| # | Chỗ | Mẫu | Hậu quả khi đua | Trạng thái |
|---|-----|-----|-----------------|------------|
| 19 | `modules/support/src/guest.rs` `verify` (OTP phiếu vãng lai) | Đọc `otp_attempts` không khoá, ghi `current + 1` | Gửi loạt đoán song song vượt giới hạn 5 lần, dò được mã 6 số, rồi `claim` để đọc phiếu của người khác | Đã sửa (#259) — đỏ 3/3 trên Postgres |
| 20 | `modules/bookings/src/guests.rs` `verify_code` | Như trên | Dò được OTP của lịch hẹn vãng lai | Đã sửa (#265) — đỏ 2/2 trên Postgres |
| 21 | `modules/orders/src/guests.rs` `consume_code` | Không có transaction, không giới hạn lần thử lúc xác nhận | Dò mã | Đã sửa (#271) — đỏ trên cả hai DB |
| 22 | `modules/auth/src/invitation.rs` `accept` / `accept_roles` ↔ `revoke` | Ghi `accepted` và gán role không điều kiện | Lời mời vừa thu hồi vẫn cho người được mời nhận role | Đã sửa (#274) — đỏ 1/3 trên Postgres |
| 23 | `modules/automations/src/service/rules.rs` `enable_rule` / `set_status` | Kiểm luật ngoài transaction; không so version khi bật | Agent sửa luật chen giữa: luật do agent viết thành ACTIVE dưới quyền người, vượt bước duyệt | Đã sửa (#272): bật luật so phiên bản đã kiểm. Chưa có test tái hiện thứ tự chen giữa |
| 24 | `modules/auth/src/oidc/mod.rs` `unlink` | Kiểm "cách đăng nhập cuối" ngoài transaction | Gỡ hai liên kết cùng lúc: người dùng tự khoá mình ra ngoài | Đã sửa (bước 66): khoá dòng người dùng, đếm lại mật khẩu / passkey / liên kết trong transaction; test `go_hai_lien_ket_cung_luc_khong_mat_ca_hai` chạy cả hai DB |

### Trạng thái máy, event trùng, bộ đếm

| # | Chỗ | Mẫu | Hậu quả khi đua | Trạng thái |
|---|-----|-----|-----------------|------------|
| 25 | `modules/support/src/messages.rs` `add_message` | Đọc phiếu không khoá, kiểm `Closed`, ghi trạng thái mới | Phiếu đã đóng (job tự đóng, nhân viên) bị mở lại | Đã sửa (#259) — đỏ 3/3 trên Postgres |
| 26 | `modules/support/src/tickets.rs` `update` (không If-Match) | Như trên | Event `ticket_status_changed` trùng, bước chuyển trạng thái sai | Đã sửa (#259), cùng khoá với #25 |
| 27 | `modules/support/src/autoreply.rs` `send_auto_reply` | Đọc số lượt liên tiếp không khoá; không xét lại phiếu đã đóng trong transaction | Đếm mất lượt; chèn tin tự động vào phiếu vừa đóng (cả hai DB) | Đã sửa (#259), cùng khoá với #25 |
| 28 | `modules/promotions/src/programs/transitions.rs` `transition`, `write_activation` | Update lọc `status` cũ nhưng không kiểm `rows_affected` | Pause và End đua nhau: phát `promotion_ended` cho chương trình đang Paused, nhả suất hạn mức gói khi chương trình vẫn chiếm | Đã sửa (#267) — đỏ 3/3 trên Postgres |
| 29 | `modules/notifications/src/me.rs` `add_push_subscription`, `add_device_token` | Tìm theo key, không có thì sinh id; id là AAD của bản mã; upsert giữ id cũ | Đăng ký lại song song: bản mã gắn id sai, push hỏng vĩnh viễn; request nhận 500 | Đã sửa (#268) — test chưa tái hiện được đua (0/9); sửa theo phân tích |
| 30 | `modules/comments/src/posting/targets.rs` `change_target` | Đọc đích không khoá, ghi cả `state` lẫn `hidden` | Lệnh ẩn đích (gỡ vì pháp lý) bị ghi đè bởi handler event của module chủ; `TargetClosed` trùng | Đã sửa (#269) — đỏ 2/3 trên Postgres |
| 31 | `crates/kernel/src/data/tenancy/migration_runs.rs` `start` | "Đang có lượt chạy thì 409" nhưng không khoá, không unique | Hai lượt migration placement chạy song song | Đã sửa (lượt 2, kernel, migration `m0039_migration_run_active_key`): cột `active_key` unique, đặt khi xếp lượt, xoá khi xong; vướng unique → 409 — đỏ 2/3 trên Postgres |
| 32 | `modules/conversations/src/assist.rs` `auto_reply` | Đọc lại thread không khoá để xét `closed` / tạm dừng | Tin tự động chèn vào hội thoại vừa đóng hay vừa xin gặp người | Chưa sửa |
| 33 | `modules/catalog/src/variants.rs` `update_variant` (không If-Match) | Đọc biến thể không khoá | `PreorderDateChanged` phát trùng (cùng mẫu với #246) | Đã sửa (#273) — đỏ 3/3 trên Postgres |
| 34 | `modules/automations/src/service/rules.rs` `update_rule` | `version = before.version + 1` không khoá | Hai bản sửa cùng số version | Đã sửa (#272) — test chưa tái hiện được (0/9) |
| 35 | `modules/automations/src/runner/mod.rs` `start_run` | Đếm lần chạy trong giờ rồi chèn | Vượt `max_runs_per_hour` | Chưa sửa (thấp) |
| 36 | `modules/content/src/banners/items.rs` `create_banner` | Đếm < `max_items` và `max + 1` không khoá; index không unique | Vượt `max_items`, trùng `position` | Chưa sửa (thấp) |
| 37 | `modules/conversations/src/threads.rs` `mark_read` | Update con trỏ không điều kiện | Con trỏ đã đọc lùi | Chưa sửa (thấp) |
| 38 | `modules/newsletter/src/public.rs` `confirm` | Đọc `unconfirmed` rồi update không điều kiện | Người vừa huỷ đăng ký bị ghi lại thành đã xác nhận (đồng ý nhận thư, D-367) | Chưa sửa (thấp) |
| 39 | `modules/tax/src/service.rs` `set_profile` | So If-Match với `updated_at` mà không khoá | Mất cập nhật | Chưa sửa (thấp) |

Mức thấp khác (ghi để biết, hậu quả nhỏ hoặc dữ liệu vẫn đúng):

- `modules/notifications/src/custom.rs` `publish_template`: version `max + 1` có unique; bên thua nhận
  500 thay vì 409.
- `modules/menus/src/service.rs` `delete`: không kiểm `rows_affected`, `MenuDeleted` phát hai lần.
- `modules/support/src/teams.rs` `auto_assign`: khoá nhóm nhưng không khoá phiếu, có thể đè người
  được giao bằng tay.
- `crates/kernel/src/data/tenancy/exports.rs` `request`: hai lượt xuất trùng.
- `modules/plans/src/provisioning.rs` `retry_provisioning`, `signup.rs` `apply_signup`,
  `service/overrides.rs` `set_override`: xếp trùng job / 500 thay vì 200.
- `modules/reporting/src/forward.rs` `add_daily` ↔ `rehome.rs` `recompute_daily`: tổng theo ngày có
  thể thiếu một fact.
- `modules/organizations/src/service.rs` `delete_type`, `groups.rs` `delete_group`: dòng mồ côi.
- `modules/auth/src/delegation.rs` `revoke`, `modules/auth/src/user/service.rs` If-Match không khoá.
- `modules/tax/src/pack.rs` `import_pack`: kiểm version ngoài transaction.
- `modules/inventory/src/receipts.rs` `retry_receipt_ledger`: thừa job (sổ cái chống trùng nhờ
  reference).
- `modules/promotions/src/programs/store.rs` `replace_children`, `update_promotion`: xoá rồi chèn lại
  bộ đếm quà; sửa bộ phần con của chương trình vừa kích hoạt.
- `modules/promotions/src/redemption.rs` `confirm`: hai lần xác nhận cùng lúc, bên sau nhận sai mã
  lỗi. Dữ liệu vẫn đúng.
- `modules/shipping/src/cod.rs` `open_discrepancy`: ghi `mismatch` đè `settled` (cả hai DB).

## Đã an toàn, vì…

Những chỗ nhạy cảm đã kiểm và không cần sửa:

- **Kernel**:
  - Job queue claim dùng `FOR UPDATE SKIP LOCKED` (Postgres) hoặc `UPDATE … RETURNING`.
  - Lịch định kỳ chống trùng bằng `unique_key` cùng `ON CONFLICT DO NOTHING`.
  - Dispatcher nhận event bằng update có điều kiện.
  - Approvals `decide` lọc `status = pending`. Numbering là `INSERT … ON CONFLICT DO UPDATE … RETURNING`.
  - Token một lần `consume` có điều kiện.
- **orders**:
  - Mọi đổi `status` đi qua `lifecycle::lock` (FOR UPDATE). Thanh toán, hoàn tiền, giao hàng, đổi
    trả, đặt trước đều khoá đơn trước.
  - Số đơn sinh bằng `numbering::next`. Ca POS có unique `open_key`.
  - Thống kê đánh giá cộng nguyên tử; phiếu "hữu ích" dùng `on_conflict` và kiểm `rows_affected`.
- **inventory**:
  - Tồn (`on_hand`, `available`, `incoming`, lô) đổi bằng `rows::adjust`; giá vốn bình quân tính
    dưới `lock_one`.
  - Chỗ giữ khoá dòng và đổi trạng thái có điều kiện. Phiếu chuyển kho, kiểm kho, nhập hàng khoá
    dòng trước khi xét.
- **promotions**:
  - Mọi bộ đếm khi giữ chỗ dùng `adjust` có trần. Voucher `available → held → used` có điều kiện.
  - Redemption unique theo đơn. `release` / `expire_holds` / `tick` kiểm `rows_affected`.
- **shipping**: cập nhật vận đơn qua `lock_shipment`; `set_status` có điều kiện; tracking event,
  mã chênh lệch, mã bảng kê có unique.
- **bookings**: slot unique `(resource, slot_start)`; chỗ trong buổi tăng bằng update có điều kiện;
  hoàn cọc có điều kiện và reference cố định.
- **ledger**:
  - Ghi bút toán giữ khoá chia sẻ dòng sổ và `adjust` số dư có sàn; khoá kỳ khoá độc quyền dòng sổ.
  - Unique `reference` và `reverses_entry_id`. Thanh toán, hoàn tiền, tạm giữ, chi trả đều
    `lock_one`.
- **invoices**: cấp số qua `numbering::next` cùng unique `(series, number)`; phát hành, huỷ, sửa nháp
  khoá dòng.
- **plans**: `reserve` / `record_usage` dùng `adjust` có trần cùng unique reference; vòng đời thuê
  bao khoá dòng.
- **compliance**: OTP điện thoại khoá mọi mã của số; đồng ý, văn bản, vụ việc dùng khoá hoặc CAS
  theo `updated_at`.
- **auth**: refresh token đánh dấu `used_at` có điều kiện; TOTP `last_used_step` có điều kiện;
  "admin cuối cùng" khoá.
- **audit / idempotency**: khoá đầu chuỗi; unique khi chèn và nhận lại quyền có điều kiện.
- **comments, content, newsletter, forms, media, notes, webhooks, ai**:
  - Bộ đếm cộng bằng biểu thức. Đổi trạng thái lọc `status` / `updated_at`.
  - Cây nội dung và thư mục media khoá qua dòng khoá riêng.
