## D-20261121-catalog-digital-downloads · Hàng số của `catalog`: file chép từ `files`, quyền tải phát theo event của `orders`, địa chỉ ký theo khách — 2026-11-21 — agent quyết

Chủ đề: Thương mại

Quyết định: (1) Module `files` công bố thêm trait `Files` (`autoxide_files::api`): `find` / `open` một
file đã tải lên, theo đúng luật xem của `files` (chủ file hay `files.manage`). Người bán tải file lên qua
`/api/v1/files` (có giới hạn cỡ / loại, tải thẳng lên S3), rồi gắn `file_id` vào biến thể hàng số;
`catalog` **chép** nội dung vào `catalog/<asset_id>/file` trong nơi lưu của khách — xoá file gốc hay xoá
tài khoản người tải lên (event `privacy.user_deleted` của `files` xoá file của người đó) không làm người
mua mất hàng đã trả tiền; kernel chép theo `<asset_id>` khi tách khách. (2) Quyền tải phát khi `catalog`
nhận `orders.order_confirmed` có `payment_status = paid` (`Subscription::optional`, không sửa `orders`):
mỗi (dòng đơn, file) một quyền, khoá chống trùng là unique (`tenant_id`, `order_line_id`, `asset_id`) với
`INSERT … ON CONFLICT DO NOTHING` — nhận lại event không phát thêm. Thu hồi khi nhận `orders.order_cancelled`
hay `orders.refund_completed` **không gắn trả hàng** (`return_id = null`; hàng số không trả được nên hoàn
tiền của lần trả hàng chỉ là hàng vật lý). (3) Token ngẫu nhiên 32 byte, chỉ lưu SHA-256; người mua có tài
khoản dùng `GET /downloads` / `POST /downloads/{id}` (không cần token); người mua vãng lai nhận token qua
`POST /orders/{id}/downloads/reissue` (người quản lý cấp token mới, lần duy nhất token hiện ra) — trait
`Catalog` không thêm `issue_downloads` / `revoke_downloads` của phác thảo đặc tả, vì phát theo event đã đủ
và thêm hàm vào trait buộc mọi bản giả ở module khác phải sửa. (4) Trừ lượt bằng một câu `UPDATE` có điều
kiện (`rows::adjust`, `NON_NEGATIVE`, kèm điều kiện chưa thu hồi / chưa hết hạn) — D-181, không đọc trước.
(5) Địa chỉ file ký HMAC-SHA256 trên (ID quyền, hạn) bằng `SecretKey::derive_for(khách,
"autoxide/catalog/download-link/v1")`, sống `AUTOXIDE_CATALOG_DOWNLOAD_LINK_SECONDS` (300 giây): địa chỉ của
khách này sai chữ ký ở khách khác; thu hồi có hiệu lực ngay vì mỗi lần đọc file kiểm lại `revoked_at`.
Địa chỉ phục vụ một đoạn `Range` (tự đọc header, một đoạn; nhiều đoạn thì gửi cả file như RFC 9110 cho
phép). (6) Không gỡ được file khi còn quyền tải dùng được (`catalog.digital_asset_in_use`) hay khi là file
cuối của sản phẩm đang bán.

Lý do: Dùng lại đường tải file sẵn có của `files` (giới hạn, S3, tải thẳng) thay vì mở thêm một endpoint
nhận file lớn trong `catalog`; chép để hàng đã bán không phụ thuộc vòng đời file của người tải lên. Phát
theo event giữ ranh giới module (không sửa `orders`) và chịu được nhận lại. Địa chỉ ký ngắn hạn thay vì
token dài hạn trong URL file: lộ địa chỉ chỉ lộ vài phút, không trừ thêm lượt khi tải tiếp. Khoá ký dẫn
theo khách thì không cần lưu khoá riêng và đổi khoá gốc chỉ làm hỏng địa chỉ sống vài phút (D-113).

Đã cân nhắc: tham chiếu thẳng `file_id` của `files` (mất hàng khi file gốc bị xoá); endpoint nhận file
trong `catalog` (lặp lại giới hạn / S3 của `files`); token tải cố định suy ra từ khoá gốc (phụ thuộc khoá
sống lâu, trái D-113); thu hồi khi mọi lần hoàn tiền (hoàn tiền trả hàng vật lý làm mất hàng số).
