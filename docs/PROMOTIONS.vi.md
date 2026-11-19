# Khuyến mại

[English](PROMOTIONS.md)

Module `promotions` chạy giảm giá như **luật**. Chương trình đến người mua bằng một trong ba cách: tự động,
**coupon** (mã ai có cũng dùng được) hay **voucher** (quyền dùng một lần trong ví của một tài khoản). Đơn hàng
tính giỏ, giữ lượt dùng khi đặt, xác nhận khi đã trả tiền và trả lại khi huỷ, qua
`autoxide_promotions::api::Promotions`. Mọi endpoint ở dưới `/api/v1/promotions`, mô tả trong
`docs/openapi.json`; các việc chính có tool MCP (`promotions.*`).

## Chương trình

- `POST /promotions` tạo **bản nháp**. Chương trình nào cũng có giờ bắt đầu và giờ kết thúc bắt buộc
  (`ends_at`). `trigger` là `automatic`, `code` hay `voucher`; `scope` là `items`, `order` hay `shipping`, cho
  lớp cộng dồn `product`, `order` hay `shipping`.
- Loại: `percent_off`, `amount_off` (mỗi sản phẩm với `items`, mỗi đơn với `order`), `fixed_price`,
  `buy_x_get_y` (`reward_selection`: `cheapest`, `same_item`, `specified`; `reward_value_bp` 10000 = tặng),
  `free_shipping`, `flash_sale` (giá theo biến thể, suất còn là số thật), `tiered` (lấy bậc cao nhất đạt),
  `bundle_price` ("3 món 199.000"), `add_on` (giá ưu đãi cho món mua kèm khi có món điều kiện), `gift_choice`
  (người mua chọn quà; hàng tặng trả về thành dòng riêng giá 0).
- Điều kiện: mục tiêu (sản phẩm, biến thể, danh mục kèm danh mục con), nhóm khách, kênh, chỉ đơn đầu, đơn hay
  số lượng tối thiểu, và điều kiện mở rộng theo phương thức thanh toán, hãng giao, tỉnh, hạng thành viên, nơi
  mua (`web`, `app`, `pos`), người bán, khách mới / khách cũ, hàng đang giảm, mục tiêu loại trừ; khung giờ theo
  ngày trong tuần, theo múi giờ cửa hàng.
- Mục tiêu, giá flash sale và bản dịch có endpoint riêng (`PUT /promotions/{id}/targets`, `/flash-items`,
  `/translations/{locale}`).
- Sau khi bật chỉ được kéo dài, tăng lượt / ngân sách / số voucher và sửa chữ
  (`promotions.locked_after_start`); mọi lần đổi lưu ở `GET /promotions/{id}/versions`.

## Bật chương trình và pháp luật

`POST /promotions/{id}/activation-check` liệt kê các mục kiểm, không đổi gì; `POST /promotions/{id}/activate`
chạy cùng bộ kiểm và báo mọi mục không đạt trong `errors`:

- trần giảm đang hiệu lực (Việt Nam: 50% giá ngay trước khuyến mại, không giới hạn với chương trình khuyến mại
  tập trung quốc gia), dùng lịch sử giá của catalog;
- số ngày giảm giá mỗi chương trình và mỗi năm;
- thông báo khuyến mại đã ghi nhận cho từng nơi nhận (`PATCH /promotions/{id}/notices/{authority}`; tỉnh lấy từ
  cài đặt của nút `promotions.notice_province`);
- mục tiêu, và sàn giá của catalog (`block` chặn, `warn` cảnh báo); xung đột chỉ cảnh báo.

Hạn mức và quy tắc thông báo là **dữ liệu có ngày hiệu lực**, không viết trong code: nạp bằng
`autoxide task promotions.load_legal_data` (gói `vn` đi kèm máy chủ, ghi căn cứ và ghi chú "cần đối chiếu"),
đọc ở `GET /legal-limits?at=` và `GET /notice-rules?at=`. Không có dữ liệu hiệu lực thì không bật được
(`promotions.no_legal_data`). Gói được tổng hợp từ nguồn thứ cấp: cần người có chuyên môn pháp lý đối chiếu
trước khi chạy thật.

Để chuẩn bị thông báo, `GET /promotions/{id}/notice-export?format=json|csv` (permission `promotions.compliance`;
tool MCP `promotions.export_notice`) trả một mục mỗi nơi nhận: có cần thông báo không theo quy tắc hiệu lực ngày
bắt đầu, hạn nộp, căn cứ pháp lý và những gì đã ghi nhận. Con người nộp thông báo cho cơ quan; Autoxide chỉ chuẩn
bị dữ liệu và ghi nhận.

## Tính giỏ

`POST /evaluate` (và `Promotions::evaluate`) tính giỏ, không giữ gì:

1. ứng viên: đang chạy, trong khung giờ, của cửa hàng hay cấp trên (chương trình của người bán chỉ áp hàng của
   người bán đó), đủ điều kiện; coupon và voucher mở chương trình của chúng — mã / voucher sai trả về trong
   `rejected_codes` kèm lý do, giỏ vẫn tính được;
2. lớp `product`, rồi `order` trên phần còn lại, rồi `shipping`. Hai chương trình chỉ cộng khi cả hai cho phép
   (`exclusivity`, `combines_with`); `stop_after` dừng các chương trình ưu tiên thấp hơn cùng lớp. Cài đặt
   `promotions.stacking_mode` của nút chọn `priority` (mặc định, dễ đoán) hay `best_for_buyer`;
3. trần pháp luật mỗi dòng cộng mọi khoản giảm (mọi lớp, cả voucher sàn và voucher người bán); khoản áp sau bị
   cắt trước (`capped_by_law`), rồi tới sàn giá (`capped_by_floor`). Sản phẩm chưa có giá trước khuyến mại thì
   không giảm (`withheld`);
4. mỗi khoản giảm chia cho các bên trả tiền (`funding`: sàn, người bán, nhãn hàng, đối tác thanh toán) khớp
   từng đồng; lý do dịch theo `locale` của giỏ.

Phần trăm làm tròn nửa lên trên tổng dòng; khoản giảm cấp đơn chia về các dòng theo tiền dòng. Cùng giỏ luôn
cho cùng kết quả và cùng `fingerprint`.

## Giữ lượt dùng

`Promotions::reserve` tính lại giỏ và giữ, trong một transaction, mọi bộ đếm: lượt của chương trình và của
mã, lượt theo khoá người mua (tài khoản, số điện thoại băm, địa chỉ băm — tài khoản mới cùng số điện thoại không
được thêm lượt), lượt theo ngày, ngân sách, suất flash sale, quà và voucher. Đủ hết hoặc không giữ gì; gọi lại
cùng đơn, cùng giỏ trả lại lượt giữ cũ. Kết quả khác lúc người mua xem báo `promotions.changed`. `confirm` chốt;
`release` trả lại (sau khi xác nhận chỉ với chương trình có `return_usage_on_cancel`; voucher còn hạn về ví).
Lượt giữ không xác nhận tự hết hạn (`promotions.expire_holds`, mỗi phút).

## Voucher

Người mua lưu voucher của chương trình `claimable` (`POST /public/promotions/{id}/claim`), nhập mã nhận
(`POST /me/vouchers/claim-code`, giới hạn số lần thử) hay được cấp qua `grant_voucher` (điểm thưởng, sinh nhật,
bù lỗi…; cùng `source` + `source_ref` không bao giờ cấp hai lần) hoặc cấp tay kèm lý do
(`POST /promotions/{id}/vouchers`). `GET /me/vouchers` liệt kê ví. Ví có trong bản xuất dữ liệu cá nhân; xoá tài
khoản thì voucher chưa dùng bị thu hồi.

## Trả hàng

`api::returns` có quy tắc cho `orders`: tiền hoàn của một dòng = tiền đã trả × số trả / số mua, đơn vị cuối nhận
phần dư; khoản giảm cấp đơn chỉ thu lại khi cài đặt `promotions.clawback_on_return` của nút là `proportional`;
quà theo `gift_return_policy` của chương trình.

## Cấu hình

| Biến | Mặc định | |
|---|---|---|
| `AUTOXIDE_PROMOTIONS_MAX_CODES_PER_CART` | 5 | số mã mỗi giỏ |
| `AUTOXIDE_PROMOTIONS_MAX_HOLD_HOURS` | 72 | thời gian giữ tối đa |
| `AUTOXIDE_PROMOTIONS_CODE_LENGTH` | 10 | độ dài mã sinh |
| `AUTOXIDE_PROMOTIONS_MAX_VOUCHERS_PER_CLASS` | 1 | số voucher mỗi lớp, mỗi bên trả tiền |
| `AUTOXIDE_PROMOTIONS_COMPILED_TTL_SECONDS` | 60 | máy chủ khác thấy thay đổi sau tối đa chừng này giây |
| `AUTOXIDE_PROMOTIONS_JURISDICTION` | `VN` | dữ liệu luật áp dụng |
