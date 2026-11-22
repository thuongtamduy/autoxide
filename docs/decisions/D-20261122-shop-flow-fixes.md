## D-20261122-shop-flow-fixes · Luồng bán hàng mẫu (bước 70 P8): người nhận và kích thước kiện khi báo phí, cổng thanh toán mặc định theo khách, "kênh chưa bật" khi khách chưa đặt tài khoản — 2026-11-22 — agent quyết

Chủ đề: Thương mại

Bối cảnh: đi trọn luồng sản phẩm → giỏ → đơn → MoMo → GHN → SePay eInvoice → eSMS (bước 70 P8,
D-20261121-shop-first-order) gặp ba chỗ vướng đã được duyệt sửa kèm.

Quyết định:

1. **Báo phí vận chuyển mang người nhận và kích thước kiện.** `shipping::api::QuoteRequest` và
   `CarrierQuoteRequest` thêm `recipient: Option<(tên, số điện thoại)>` (builder `recipient(…)`, trường mới của kiểu
   `#[non_exhaustive]` — không phá vỡ, D-038). `orders` điền khi giỏ có đủ tên và số (nhập tay hay từ sổ địa chỉ).
   `shipping` gửi hãng một kiện có kích thước: dài và rộng lớn nhất, cao cộng dồn theo số lượng (xếp chồng); một món
   cần giao thiếu kích thước → không gửi kích thước (hãng dùng mặc định của nó), không đoán. GHN dùng người nhận thật
   cho `preview`, giá trị giữ chỗ chỉ còn khi chưa có (trang sản phẩm).
2. **Cổng thanh toán mặc định theo khách:** cấu hình khách `ledger.payments.default_provider` (không thuộc nhóm
   `connection`: chỉ chọn trong các cổng người vận hành đã bật, không đổi nơi tiền về). Lượt thu không nêu `provider`
   mà máy chủ có nhiều cổng → dùng cổng này; không đặt → lỗi cũ (ô `provider` bắt buộc), câu lỗi chỉ tên cấu hình.
   Cổng một khách chọn mà máy chủ không có → `payment.gateway_not_configured` như khi nêu thẳng. Áp cho tạo thanh
   toán và đối soát tay; các chỗ đã biết đối tác (hoàn, webhook) không đổi.
3. **`notifications` coi `provider.not_configured` là "kênh chưa bật":** tin được ghi `skipped` với lý do
   `channel_disabled`, không `failed`, không thử lại. Khách bật kênh sau thì tin mới đi; tin cũ không gửi lại (như
   khi máy chủ chưa có bên gửi).

Hệ quả: hướng dẫn `docs/guides/shop.md` + `.vi.md` làm theo được từ quickstart; test đầu-cuối
`crates/app/src/e2e/shop.rs` (cả SQLite và Postgres) chạy trên chương trình thật với `AUTOXIDE_MODULES` như bản cài.
