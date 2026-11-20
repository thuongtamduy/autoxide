## D-20261120-step47-wishlist-alerts · Báo khi sản phẩm yêu thích có hàng lại hay giảm giá (bước 47, D-233): tự bật kèm đồng ý riêng, chỉ giảm thật so với mốc, gom lô theo khung giờ — 2026-11-20 — agent quyết

Chủ đề: Thương mại

Bối cảnh: đặc tả `docs/steps/47-customers.md` (mục "Báo khi sản phẩm yêu thích có hàng lại hay giảm giá (D-233)").
Bước 41 chưa có hàm đọc giá công khai cho nhiều ID; D-233 cho bước này thêm vào `catalog`.

Quyết định:

1. **`Catalog::display_prices(tenant, product_ids, variant_ids)`** mới ở `catalog::api`, có cài đặt mặc định trả rỗng (bản
   giả trong test của module khác không phải sửa). Giá theo biến thể = giá niêm yết của bảng giá chung, hay giá sau giảm
   tự động nếu thấp hơn; theo sản phẩm = giá thấp nhất của các biến thể đang bán. Không dùng giá gạch, giá nhóm khách,
   mã giảm giá. **Lệch đặc tả:** tính ở **nút sở hữu sản phẩm** (bảng giá chung gần nhất trong chuỗi của nó) thay vì cứng
   ở nút gốc — khách đặt bảng giá ở chuỗi thì nút gốc không có giá nào.
2. **Tự bật, mặc định tắt** (`PUT /me/wishlist/alerts`), hai loại riêng. Bật cần `consent_text` + `consent_version`
   (422 `customers.wishlist_alert_consent_required`), ghi `ConsentLedger::grant` mục đích `customers.wishlist_alerts`
   trước transaction; tắt cả hai thì `withdraw`. Không có `inventory` mà bật có hàng lại, hay không có sổ đồng ý / gửi
   tin → 422 `customers.wishlist_alert_unavailable`. Tính năng theo gói `customers.wishlist_alerts`. Rút đồng ý ở chỗ
   khác (`compliance.consent_withdrawn` của mục đích này, event không nói tài khoản) → kiểm lại mọi người đang bật, ai hết
   đồng ý thì tắt và xoá hàng đợi.
3. **Giá mốc** đặt sau khi thêm vào yêu thích (đọc `catalog` sau commit), chưa có giá thì ở lần đầu thấy giá. Giá tăng
   thì mốc giữ; mốc chỉ đổi khi đã báo (= giá đã báo) hay khác tiền tệ. Ngưỡng `giá × 10000 ≤ mốc × (10000 − bp)`
   (`AUTOXIDE_CUSTOMERS_WISHLIST_PRICE_DROP_MIN_BP`, mặc định 500 = 5%).
4. **Tín hiệu** (handler đọc lại giá / tồn, không tin số trong event): `catalog.display_price_changed`,
   `catalog.price_changed`, `catalog.price_effective` (biến thể → sản phẩm qua `Catalog::variants`) → giảm giá;
   `inventory.level_changed` với `available` từ ≤ 0 lên > 0 → có hàng lại, cho dòng theo biến thể và dòng theo sản phẩm
   của nó, bỏ người đã tự đăng ký ở `inventory` (`restock_alerts_of`, D-237). Chưa bắt: giá sau giảm tự động đổi ở biến
   thể không phải biến thể rẻ nhất (không có event riêng) — dòng theo biến thể đó nhận ở lần đổi giá kế.
5. **Gom lô** — job `customers.send_wishlist_alerts` mỗi giờ: chỉ trong giao các khung `notifications.send_windows` của
   kênh có khung (không có thì 07:00–22:00 giờ của khách, mặc định UTC+7); mỗi hồ sơ một tin tối đa 10 sản phẩm; tối đa
   `AUTOXIDE_CUSTOMERS_WISHLIST_ALERTS_PER_DAY` tin mỗi ngày (mặc định 1). Kiểm lại trước khi gửi (còn bật, còn đồng ý,
   còn hàng, giá vẫn đạt, còn bán). Transaction ngắn đặt `<loại>_alerted_at` **có điều kiện** (cũ hơn 7 ngày), đặt mốc
   mới, xoá hàng đợi, ghi lô `pending`; sau commit gọi `Notifier::notify` với `dedupe_key` theo lô; lô còn `pending` được
   gửi lại ở lượt sau. Hai lượt chạy đồng thời chỉ gửi một tin (test).
6. Tin là quảng cáo (`Category::Marketing`, `purpose = customers.wishlist_alerts`): `notifications` kiểm lại đồng ý, khung
   giờ, nhãn quảng cáo, link huỷ nhận. Xoá tài khoản xoá cài đặt, hàng đợi, lô; bản xuất dữ liệu có cài đặt.

Căn cứ: NĐ 91/2020/NĐ-CP (tin quảng cáo chỉ khi đồng ý, khung giờ); Luật 91/2025/QH15 (đồng ý theo mục đích, rút bất kỳ
lúc nào); Luật 19/2023/QH15 (không báo giảm giá ảo) — **cần đối chiếu văn bản gốc / người có chuyên môn pháp lý**.

Hệ quả: người mua nhận tin "có hàng lại / rẻ hơn" chỉ khi chính họ bật, không quá một tin mỗi ngày, không bị báo giảm giá
ảo; module khác đọc được giá công khai qua `Catalog::display_prices`.
