## D-20261118-step42-reservations · Giữ chỗ tồn của `inventory`: chọn kho bằng số đọc rồi `adjust` làm trọng tài, khoá dòng chỗ giữ cho mọi chuyển trạng thái — 2026-11-18 — agent quyết

Chủ đề: Thương mại

Quyết định:

1. **Thử kho kế: đọc rồi để `rows::adjust` làm trọng tài, không dùng savepoint.** Trong transaction giữ chỗ, đọc
   `available` của mọi kho ứng viên, chọn kho (một kho đủ mọi dòng trước; không có thì chia tham lam khi nút bán bật
   `allow_split`), rồi trừ từng phần bằng một câu `UPDATE … WHERE available >= q` theo thứ tự ID dòng tồn tăng dần.
   Câu nào trả 0 dòng (số đọc đã cũ: Postgres, bên khác vừa giữ) thì bỏ cả transaction và chọn lại với số đọc mới, tối
   đa 4 lần; vẫn thiếu thì `inventory.insufficient_stock`. SQLite chỉ một bên ghi nên số đọc không bao giờ cũ.
2. **Chọn kho**: ứng viên là `inventory_sources` đang bật của nút bán (theo `priority`, rồi `id`); không có thì chính
   nút bán nếu nó giữ được tồn; không có gì → `inventory.no_source`. `fixed_org_id` → chỉ kho đó. `prefer_nearby` bật
   và đơn có tỉnh giao → kho cùng tỉnh lên trước (sắp ổn định). `prefer_nearby`, `allow_split` mặc định **tắt** (một
   đơn một kho).
3. **Tỉnh và "nhận tại cửa hàng" của nút** lấy từ event `organizations.public_profile_updated`
   (`Subscription::optional`) và ghi vào `inventory_settings` — `OrgDirectory` không có địa chỉ, và module không đọc
   bảng của `organizations`.
4. **Mọi chuyển trạng thái chỗ giữ khoá dòng chỗ giữ trước** (`rows::lock_one`): xác nhận, trả, xuất, hết hạn của cùng
   một đơn nối đuôi nhau; trạng thái đổi bằng `UPDATE … WHERE status = <cũ>`. `confirm` nhận cả chỗ `held` đã quá hạn
   mà job chưa kịp trả (tiền về muộn vẫn giữ được hàng); job chỉ trả chỗ còn `held` VÀ đã quá hạn lúc nó khoá được dòng
   — bên khoá trước thắng, `available` không bao giờ cộng hai lần.
5. **Xuất kho** nhận chỗ `held` hay `confirmed` (bán tại quầy không cần bước xác nhận riêng) và xác nhận luôn phần còn
   lại; số của dòng xuất tính theo đơn vị của dòng đơn (combo: số combo × `per_unit` của từng thành phần). Trả phần còn
   lại sau khi đã xuất một phần → trạng thái `fulfilled`.
6. **Combo** tách thành thành phần lúc giữ (`bundle_components` của `catalog`, đọc trước transaction); biến thể không
   theo dõi tồn (hàng số, dịch vụ) không có dòng giữ. `sale_mode` `backorder` / `preorder` cho `available` âm
   (`backordered = true` khi giữ vượt tồn đọc được).
7. **Nhập lại hàng trả về**: dòng `sellable = false` chỉ ghi trong `inventory_restocks.lines`, không vào sổ hay tồn.

Lý do: savepoint làm transaction phức tạp và vẫn phải đọc để biết kho nào nên thử; với một câu `UPDATE` có điều kiện,
số đọc chỉ là gợi ý, tính đúng nằm ở câu ghi. Khoá dòng chỗ giữ đơn giản hơn so điều kiện trạng thái ở từng câu và loại
được cả đua giữa xuất kho với trả chỗ.

Đã cân nhắc: savepoint mỗi kho — được SeaORM hỗ trợ nhưng thêm nhánh lỗi riêng cho hai database mà kết quả như nhau;
bắt `confirm` từ chối chỗ đã quá hạn — làm mất đơn đã trả tiền chỉ vì job chạy chậm.
