## D-20261120-step47-engagement · Yêu thích, vừa xem, nhắc giỏ bỏ dở, giới thiệu bạn của `customers` (bước 47 mục 5): nhắc chỉ khi có đồng ý quảng cáo, huỷ khi đặt đơn / rút đồng ý, thưởng giới thiệu bằng điểm — 2026-11-20 — agent quyết

Chủ đề: Thương mại

Bối cảnh: đặc tả `docs/steps/47-customers.md` (mục "Yêu thích, vừa xem, nhắc giỏ bỏ dở, giới thiệu bạn"). Code thật lệch
phác thảo ở hai chỗ: event `orders.order_placed` không mang `cart_id`; event `compliance.consent_withdrawn` không nói
tài khoản nào (`subject_value` rỗng với tài khoản).

Quyết định:

1. **Yêu thích theo sản phẩm hay biến thể** (khoá `variant_key` = ID biến thể hay `''`), tối đa
   `AUTOXIDE_CUSTOMERS_WISHLIST_MAX`. `POST /me/wishlist` vượt trần → 422 `customers.wishlist_full`; `POST
   /me/wishlist/merge` (gộp danh sách của trình duyệt khi đăng nhập) bỏ trùng và thêm tới trần rồi thôi, không lỗi.
   Không kiểm sản phẩm có thật (ID mờ, như nhóm khách với `catalog`); bảng có sẵn cột giá mốc / mốc đã báo cho báo yêu
   thích (D-233, làm ở PR sau).
2. **Vừa xem:** chỉ người đã đăng nhập (vãng lai: client tự giữ), xem lại đưa lên đầu, giữ tối đa
   `AUTOXIDE_CUSTOMERS_RECENT_VIEWS_MAX`, job hằng ngày xoá cũ hơn `AUTOXIDE_CUSTOMERS_RECENT_VIEWS_DAYS`.
3. **Nhắc giỏ bỏ dở là tin quảng cáo** (`Category::Marketing`, mục đích `marketing`): nghe `orders.cart_abandoned` →
   giỏ vãng lai không nhắc (không có đồng ý ghi trong sổ); không có đồng ý → ghi `skipped_no_consent`, dừng; có → mỗi
   mốc của quy tắc hiện hành (dữ liệu có ngày hiệu lực, 1–5 mốc 15–10080 phút) một dòng `scheduled` + job có
   `run_at`, `unique_key = cart:<id>:<bước>`. Job kiểm lại đồng ý ngay trước khi gửi, gửi qua `Notifier::notify` với
   `dedupe_key` theo giỏ + bước (chạy lại không gửi hai lần), rồi đặt `sent` có điều kiện. `notifications` vẫn áp khung
   giờ, nhãn quảng cáo, link huỷ nhận.
4. **Huỷ nhắc khi người đó đặt đơn bất kỳ** (nghe `orders.order_placed` theo `buyer_user_id`) thay vì theo giỏ, vì event
   đặt đơn không mang `cart_id`; và **khi rút đồng ý `marketing`**: không biết tài khoản nào rút, nên kiểm lại
   `ConsentLedger::is_granted` của mọi người đang có nhắc chờ, ai không còn đồng ý thì huỷ ngay.
5. **Giới thiệu bạn:** mỗi hồ sơ một mã 8 ký tự không nhập nhằng (bỏ 0/O, 1/I/L), tạo lúc dùng lần đầu. Nhập mã trong
   `AUTOXIDE_CUSTOMERS_REFERRAL_APPLY_DAYS` ngày sau khi có hồ sơ và trước đơn đầu; unique theo người được giới thiệu
   (nhập lần hai hay hai request cùng lúc → 409 `customers.referral_not_eligible`). Cùng số điện thoại (khoá) hay email
   với người giới thiệu → ghi `rejected` (`same_contact`) và 422 `customers.self_referral`. Đơn đầu hoàn tất đạt tiền
   hàng tối thiểu → `rewarded` bằng câu có điều kiện (event giao lại không thưởng hai lần): điểm hai bên là lô mới
   `referral:<id>:<bên>` (mặc định, cần quy tắc điểm), voucher tuỳ chọn qua `grant_voucher` (`referral:<id>`) sau commit.
6. **Xoá tài khoản** xoá yêu thích, vừa xem và nhắc giỏ của hồ sơ (dữ liệu hành vi), cùng các trường cá nhân như trước.

Căn cứ: NĐ 91/2020/NĐ-CP (tin quảng cáo chỉ gửi khi đã đồng ý, dừng ngay khi từ chối, khung giờ) — đang có dự thảo thay
thế; Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15 (rút đồng ý bất kỳ lúc nào, quyền xoá) — **cần đối chiếu văn bản gốc /
người có chuyên môn pháp lý**.

Hệ quả: website gộp yêu thích khi người mua đăng nhập; người bán bật nhắc giỏ mà không gửi tin cho người chưa đồng ý; giới
thiệu bạn có thưởng không tự giới thiệu được bằng tài khoản thứ hai cùng số / email.
