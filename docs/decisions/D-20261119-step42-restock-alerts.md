## D-20261119-step42-restock-alerts · Báo khi có hàng lại của `inventory`: tin giao dịch đã xác nhận hai bước, giành lượt bằng `armed`, ở lại khách gốc khi tách — 2026-11-19 — agent quyết

Chủ đề: Thương mại

Quyết định:

1. **Đăng ký** `POST /api/v1/inventory/public/restock-alerts {variant_id, store_id?, email | phone, consent, …}`
   (thay cho trường `contact` của bản phác thảo: tách `email` / `phone` như `support`, để kiểm từng loại rõ ràng).
   `consent` phải là `true`; chữ và phiên bản đồng ý giữ làm bằng chứng, mục đích `inventory.restock_alert` — đồng ý
   riêng, không gộp với đồng ý nhận quảng cáo. Người đã đăng nhập không nêu liên lạc thì báo vào tài khoản ngay
   (`user_id`, D-237). Khách vãng lai qua `SpamGuard` (phạm vi `inventory:restock_alerts`), giới hạn theo IP / email /
   số, rồi **xác nhận hai bước**: tin có link tới trang `AUTOXIDE_INVENTORY_RESTOCK_CONFIRM_URL` (thiếu thì 503
   `inventory.restock_alerts_unavailable`), trang gửi token về `/confirm`. Câu trả lời luôn 202 giống hệt; biến thể
   không có / đang ẩn cũng 202, không lưu.
2. **Tin giao dịch** (`Category::Transactional`) của `notifications`: người nhận tự xin, đã xác nhận, nội dung chỉ báo
   món có lại, không kèm khuyến mãi. Căn cứ: NĐ 91/2020/NĐ-CP (chống tin nhắn, thư điện tử rác) và Luật Bảo vệ dữ liệu
   cá nhân 91/2025/QH15 — cần đối chiếu văn bản gốc / người có chuyên môn pháp lý. Thiếu module `notifications` thì
   không nhận đăng ký (503) và không nghe event.
3. **Một lần mỗi đợt**: cột `armed`. Bên nghe `inventory.level_changed` của chính module: `available` từ ≤ 0 lên > 0 →
   với mỗi đăng ký `confirmed` + `armed` của biến thể mà cửa hàng của nó nay bán được (cùng hàm với tình trạng công
   khai; không `store_id` = kho bán online = nút gốc), giành lượt bằng `UPDATE … SET armed = false, sent_count + 1
   WHERE armed AND sent_count = <cũ>` rồi gửi (khoá chống trùng `id:sent_count`); gửi lỗi thì trả lượt và báo lỗi để
   event giao lại. `available` từ > 0 xuống ≤ 0 mà cửa hàng hết hàng → bật `armed` lại.
4. Mỗi tin có link **dừng** với token mới (database chỉ giữ SHA-256 phần thô; token mang khách theo `link_token`).
5. Tách khách (relocation): bảng đăng ký **ở lại khách gốc** (`stay`) — đăng ký là của người mua với cả khách, cửa hàng
   có thể là kho online; bảng throttle / nonce chống spam là tạm (`ephemeral`).

Lý do: event có thể đến lại và nhiều máy có thể nhận cùng lúc; giành lượt bằng câu `UPDATE` có điều kiện chạy giống
nhau trên SQLite và Postgres.

Đã cân nhắc: xoá đăng ký sau lần báo đầu — người mua phải đăng ký lại mỗi lần; khoá theo ID event — mỗi kho có hàng lại
sinh một event, một cửa hàng nhiều kho sẽ báo nhiều lần.

Thay: không.
