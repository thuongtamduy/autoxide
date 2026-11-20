## D-20261120-step47-customers · Module `customers` (bước 47 mục 1–2): nhận diện theo số đã xác thực, người mua không cần permission, tổng chi từ event, che liên hệ, nhóm theo quy tắc là điều kiện SQL — 2026-11-20 — agent quyết

Chủ đề: Thương mại

Bối cảnh: đặc tả `docs/steps/47-customers.md` (mục "Hồ sơ và nhận diện", "Nhóm và phân khúc") phác thảo bảng, API,
permission; code thật của bước 45 lệch phác thảo ở vài chỗ (khoá liên hệ ở `SecretKey`, event của `orders` chưa có
`goods_paid`, người mua không mang permission). Mục này ghi lựa chọn của hai mục đầu; các mục sau của bước 47 ghi
file `D-<ngày>-step47-<tên>.md` riêng.

Quyết định:

1. **Nhận diện theo số bằng `SecretKey::phone_key`** (D-20261120-step45-guest-orders, không có hàm
   `tenancy::contact_key` riêng): hồ sơ lưu `phone_key` (HMAC theo khách) và số E.164 để nhân viên có quyền xem đủ.
   Unique (`tenant_id`, `user_id`) và (`tenant_id`, `phone_key`); NULL không đụng nhau trên cả hai database.
2. **Gắn hồ sơ theo số vào tài khoản chỉ khi số đã xác thực**, đọc qua `compliance::api::AccountVerification::
   verified_phone` ngoài transaction ghi, gắn bằng câu cập nhật có điều kiện `WHERE user_id IS NULL` (hai lần gắn
   cùng lúc chỉ một bên thắng). Số đã thuộc hồ sơ của tài khoản khác → tạo hồ sơ mới không mang số. Thiếu
   `compliance` thì không gắn theo số.
3. **Người mua không cần permission** (như D-20261118-step45-orders-api): bỏ `customers.self` của đặc tả; `/me` chỉ
   cần đăng nhập và chỉ chạm hồ sơ của chính người gọi.
4. **`orders` thêm `buyer_key`, `placed_at`, `goods_paid` vào thân event của đơn** (`lifecycle::payload`).
   `goods_paid` = Σ `total_minor` của dòng (sau giảm, gồm thuế, không phí giao) — đúng cơ sở D-226. Khi bước 47 thêm
   tender điểm / thẻ, phần trả bằng tender trừ khỏi số này ở `orders`.
5. **Tổng chi và số đơn dựng từ event, chịu được nhận lại:** dòng `customers_order_facts` chèn kiểu "trùng thì bỏ
   qua" (lần đầu thấy đơn mới cộng số đơn), trạng thái đổi bằng câu có điều kiện (đơn `completed` / `cancelled`
   không đổi nữa), tổng chi cộng khi chuyển `completed`, trừ khi hoàn tiền (bảng `customers_refund_facts` khoá
   theo `refund_id`, không trừ quá phần đã cộng, không xuống dưới 0). Nghe `orders.order_status_changed` thay cho
   từng event trạng thái: một event cho mọi lần chuyển. Tổng chi giữ một tiền tệ (tiền tệ của đơn hoàn tất đầu
   tiên); đơn tiền tệ khác không cộng, ghi log.
6. **Che liên hệ (D-226):** `09•• ••• 412` (số Việt Nam ở dạng trong nước, giữ 2 ký tự đầu và 3 số cuối; số nước
   khác giữ mã nước), `n•••@gmail.com`. Audit qua event `customers.contact_revealed` (người xem, hồ sơ, lý do ≤ 200
   ký tự) — module `audit` ghi mọi event, không gọi trực tiếp.
7. **Quy tắc nhóm dịch thành điều kiện SQL trên bảng hồ sơ** (`total_spent_min`, `orders_count_min`,
   `last_order_within_days`, `tier_in`, `tags_any` qua `TagIndex::condition`, `org_in` qua
   `OrgTree::subtree_condition`): xem trước, đánh giá một hồ sơ và đánh giá cả nhóm dùng cùng một điều kiện, chạy
   như nhau trên hai database, không đổ hồ sơ ra bộ nhớ. Nhóm của một nút chỉ khớp khách trong nhánh đó. Kiểu nhóm
   (`manual` / `rule`) và nút của nhóm không đổi sau khi tạo.
8. **Đánh giá nhóm:** theo hồ sơ — NGAY trong transaction làm đổi hồ sơ (vài câu đọc theo index, không gọi mạng),
   không qua job: job mang `unique_key` theo khe phút làm mất lần đánh giá thứ hai trong cùng phút (khoá còn sống tới
   khi job đã xong bị dọn — test bắt được). Theo nhóm — job `customers.evaluate_group` khi tạo / đổi quy tắc, không
   khoá trùng (chạy hai lần vô hại). Toàn bộ — `customers.evaluate_groups` hằng ngày (quy tắc theo thời gian đổi kết
   quả dù không có event).
9. **`BuyerGroups::profile_of`** trả ID nhóm, hạng (`tier_code`, điền ở mục 3) và `price_group` = mã của nhóm đầu
   tiên theo tên có mã — cố định, không phụ thuộc thứ tự chèn.
10. **Xoá tài khoản:** hồ sơ mất mọi trường cá nhân và liên kết tài khoản, giữ con số tổng và tóm tắt đơn ẩn danh.
    Ghi chú nội bộ trên hồ sơ (bảng `kernel_record_notes`) chưa xoá được vì kernel chưa có hàm xoá ghi chú theo
    bản ghi — ghi tồn đọng, làm khi tách kernel (sửa kernel bây giờ buộc build lại gần hết workspace).

Căn cứ: Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15 và NĐ 356/2025/NĐ-CP (tối thiểu hoá, kiểm soát truy cập, quyền xoá)
— **cần đối chiếu văn bản gốc / người có chuyên môn pháp lý**.

Hệ quả: `orders` gửi đủ dữ liệu cho `customers`, `affiliate`, `channels` tính theo người mua mà không đọc bảng của nó;
hồ sơ có tổng chi, số đơn đúng dù event giao lại; bảng giá và khuyến mại theo nhóm dùng được ngay khi có nhóm.
