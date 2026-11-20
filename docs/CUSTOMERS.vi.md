# Khách hàng

Module `customers` giữ hồ sơ bán hàng của từng người mua của cửa hàng, dựng lịch sử mua từ event
của đơn hàng, và chia khách thành nhóm cho bảng giá và khuyến mại. Module đăng ký sau `orders` và
chạy được khi không có `orders` (không có lịch sử, giỏ hàng không có nhóm).

## Hồ sơ

- Mỗi người mua trong một cửa hàng có một hồ sơ, nhận theo **tài khoản** người mua và / hoặc **số
  điện thoại**. Số được lưu cùng một mã băm có khoá (HMAC bằng khoá riêng của từng cửa hàng, đúng
  `buyer_key` mà `orders` ghi), nên cùng một số khớp trong một cửa hàng và không bao giờ khớp giữa
  hai cửa hàng.
- Hồ sơ được tạo khi người mua đặt đơn đầu tiên (`orders.order_placed`), khi nhân viên đăng ký khách
  tại quầy (`POST /api/v1/customers`), hay khi người mua gọi `GET /api/v1/customers/me` lần đầu.
- Hồ sơ theo số điện thoại chỉ gắn vào tài khoản **khi số của tài khoản đã xác thực** (mã một lần,
  `compliance`). Đăng ký một số ở quầy không trao lịch sử cho người nào gõ số đó sau này.
- `org_id` là nút nơi khách đăng ký hay mua lần đầu. Nhân viên có `customers.read` ở độ sâu `unit` /
  `subtree` chỉ thấy khách của nút mình; độ sâu `tenant` thấy hết.

## Lịch sử mua và tổng chi

`customers` chép tóm tắt mỗi đơn từ event của `orders` (không bao giờ đọc bảng của đơn hàng).
`GET /api/v1/customers/{id}/orders` liệt kê chúng.

- **Tổng chi** = tiền người mua trả cho hàng hoá của đơn **đã hoàn tất**, trừ tiền hoàn: sau mọi
  khoản giảm (kể cả coupon, voucher), gồm thuế, không gồm phí giao, không gồm phần trả bằng điểm hay
  thẻ quà tặng. Định nghĩa cố định (quyết định D-226).
- **Số đơn** = đơn đã đặt và không bị huỷ.
- Event giao hai lần không bao giờ bị tính hai lần.

## Che liên hệ

Mọi danh sách và chi tiết cho nhân viên (REST, tool MCP, xuất danh sách) chỉ có `phone_masked`
(`09•• ••• 412`) và `email_masked` (`n•••@gmail.com`). Muốn xem đủ, gọi
`GET /api/v1/customers/{id}/contact?reason=…` với permission `customers.contact.reveal`; mỗi lần gọi
phát `customers.contact_revealed` và audit log giữ lại (ai xem, khách nào, lý do). Người mua xem hồ sơ
của mình không bị che ở `GET /api/v1/customers/me`.

## Nhóm khách

- Nhóm **tay**: nhân viên thêm, bớt thành viên (`POST` / `DELETE
  /api/v1/customers/groups/{id}/members/{profile_id}`).
- Nhóm **theo quy tắc**: thành viên theo quy tắc, mọi điều kiện phải đúng:

  ```json
  {"total_spent_min": 5000000, "orders_count_min": 3, "last_order_within_days": 180,
   "tier_in": ["gold"], "tags_any": ["<id nhãn>"], "org_in": ["<id nút>"]}
  ```

  Thành viên được cập nhật ngay sau khi hồ sơ đổi và mỗi đêm (quy tắc theo thời gian, như "mua trong
  180 ngày", đổi kết quả dù không có event nào). `GET /groups/{id}/preview` đếm số khách khớp lúc này.
- Nhóm của một nút chỉ khớp khách của nút đó và các nút dưới nó.
- `price_group` (vd `vip`) là mã nhóm khách mà bảng giá của `catalog` dùng. Người mua ở nhiều nhóm có
  mã thì nhóm đứng đầu theo tên được chọn.
- Giỏ hàng của `orders` nhận ID nhóm của người mua (cho khuyến mại giới hạn theo nhóm), hạng thành
  viên và mã nhóm giá.

## Điểm thưởng và hạng

- **Quy tắc** (`POST /api/v1/customers/loyalty/rules`) là dữ liệu có ngày hiệu lực: đơn vị tích
  (`earn_per_minor`, vd 10.000 đ → `points_per_unit` điểm), giá trị một điểm, số tháng hiệu lực, số điểm
  tối thiểu để đổi, phần tối đa của đơn được trả bằng điểm. Bản mới bắt đầu ở `effective_from` (lúc này
  hay sau) và không bao giờ sửa quá khứ.
- Điểm được **tích khi đơn hoàn tất**, trên tiền người mua trả cho hàng hoá (định nghĩa ở trên), nhân hệ
  số hạng. Mỗi lần tích là một **lô** hết hạn vào cuối ngày (giờ Việt Nam) sau `points_valid_months`
  tháng. Đổi điểm dùng lô hết hạn sớm nhất trước.
- **Trả bằng điểm**: gửi `"tenders": [{"kind": "points", "input": {"points": 500}}]` tới
  `POST /api/v1/orders/checkout`. Điểm được giữ tới khi đơn xác nhận (thì dùng) hay bị huỷ / quá hạn trả
  (thì trả lại). Đối tác thanh toán chỉ thu phần còn lại.
- **Hạng** (`POST /loyalty/tiers`): hạng cao nhất có ngưỡng chi tiêu 12 tháng đạt được, xét khi đơn hoàn
  tất và mỗi ngày. Mã hạng vào giỏ hàng (khuyến mại theo hạng) và quy tắc nhóm (`tier_in`).
- **Điều chỉnh tay**: `POST /api/v1/customers/{id}/points/adjustments` với `customers.loyalty.adjust`, mã
  tham chiếu (không bao giờ áp hai lần) và lý do.
- **Phần thưởng voucher** (`POST /loyalty/voucher-rewards`, loại `birthday`, `purchase`, `points`) cấp qua
  module khuyến mại; sinh nhật một voucher mỗi năm, mỗi đơn một voucher, đổi điểm thì trừ điểm trước và
  trả lại nếu không cấp được voucher. Người mua xem điểm, quy tắc hiện hành và phần thưởng ở
  `GET /me/points`, `GET /me/points/rewards`, đổi bằng `POST /me/points/vouchers`.
- **Sổ cái**: điểm chưa dùng là nghĩa vụ (`loyalty.liability`) theo giá trị lúc tích; đổi điểm ghi Có
  `buyer.prepayments`, hết hạn ghi Có `loyalty.breakage`.

## Yêu thích, vừa xem, nhắc giỏ, giới thiệu bạn

- **Yêu thích** (`/me/wishlist`): sản phẩm hay một biến thể, tối đa `AUTOXIDE_CUSTOMERS_WISHLIST_MAX`. Ngay
  sau khi đăng nhập, gửi danh sách lưu ở trình duyệt tới `POST /me/wishlist/merge`; dòng trùng được bỏ qua.
- **Vừa xem** (`/me/recently-viewed`): chỉ người đã đăng nhập (vãng lai lưu ở trình duyệt), mới nhất trước,
  dòng cũ được xoá mỗi ngày.
- **Báo yêu thích** (`GET`/`PUT /me/wishlist/alerts`): tắt cho tới khi người mua tự bật, kèm đồng ý
  của họ (`consent_text`, `consent_version`, ghi trong sổ đồng ý với mục đích
  `customers.wishlist_alerts`). Hai loại: có hàng lại (cần `inventory`; bỏ qua sản phẩm người mua đã
  đăng ký báo ở `inventory`) và giảm giá: thấp hơn ít nhất `AUTOXIDE_CUSTOMERS_WISHLIST_PRICE_DROP_MIN_BP`
  (mặc định 5%) so với giá lúc thêm vào yêu thích, không bao giờ so với giá gạch; sau khi báo, giá đã báo
  thành mốc mới. Báo được gom mỗi giờ, chỉ trong khung giờ quảng cáo: một tin tối đa 10 sản phẩm, tối đa
  `AUTOXIDE_CUSTOMERS_WISHLIST_ALERTS_PER_DAY` tin mỗi ngày (mặc định 1), mỗi sản phẩm mỗi loại tối đa một
  lần trong 7 ngày. Rút đồng ý thì dừng ngay.
- **Nhắc giỏ bỏ dở** (`POST /reminder-rules`, `customers.engagement.manage`): nhắc giỏ là tin quảng cáo. Chỉ
  gửi cho người mua đã đồng ý nhận tin quảng cáo (sổ đồng ý của `compliance`), kiểm lại đồng ý ngay trước khi
  gửi, dừng khi người mua đặt đơn hay rút đồng ý. `notifications` áp khung giờ và link huỷ nhận.
- **Giới thiệu bạn**: mỗi người mua có một mã (`GET /me/referral`); người mới nhập mã trong
  `AUTOXIDE_CUSTOMERS_REFERRAL_APPLY_DAYS` ngày và trước đơn đầu (`POST /me/referral/apply`). Khi đơn đầu đó
  hoàn tất đủ tiền hàng, cả hai được thưởng một lần bằng điểm và voucher tuỳ chọn (`POST /referral-rules`).
  Cùng số điện thoại hay email với người giới thiệu bị từ chối.

## Thẻ quà tặng và số dư cửa hàng (tắt mặc định)

Giá trị trả trước có thể thuộc quy định về thanh toán (NĐ 52/2024/NĐ-CP). Phải bật cả hai lớp mới dùng
được: `AUTOXIDE_CUSTOMERS_STORED_VALUE_ENABLED=true` (chỉ sau khi người có chuyên môn đã đối chiếu quy định
cho cửa hàng của bạn) **và** tính năng của gói `customers.gift_cards` / `customers.store_credit`. Nếu không,
mọi endpoint dưới đây trả 403 `customers.stored_value_disabled` và lúc thanh toán không có hai hình thức
này. `autoxide doctor` cảnh báo khi đang bật.

- Giá trị chỉ dùng trả đơn của chính cửa hàng phát hành. **Không** có cách rút ra, đổi tiền mặt, nạp bằng
  tiền hay chuyển cho người khác.
- **Thẻ quà tặng** (`customers.stored_value.manage`): `POST /gift-cards` phát hành thẻ tối đa
  `AUTOXIDE_CUSTOMERS_GIFT_CARD_MAX_MINOR`; mã 16 ký tự chỉ có trong câu trả lời đó (cửa hàng chỉ giữ bản băm
  và 4 ký tự cuối). Cùng `reference` không bao giờ ra thẻ thứ hai. `POST /gift-cards/lookup` tìm thẻ theo
  mã; `POST /gift-cards/{id}/void` huỷ thẻ bị mất.
- Mã sai được đếm theo người: sai quá `AUTOXIDE_CUSTOMERS_GIFT_CARD_LOOKUPS_PER_HOUR` lần trong một giờ thì
  tra cứu trả 429 `customers.lookup_throttled` tới hết giờ. Mã đúng không bị đếm.
- **Số dư cửa hàng** chỉ đến từ bồi thường của nhân viên: `POST /{id}/store-credit/adjustments` kèm
  `reference` và lý do (cùng `reference` chỉ áp một lần; không xuống dưới 0).
- **Người mua**: `GET /me/gift-cards`, `POST /me/gift-cards/attach` (kèm mã), `GET /me/store-credit`. Lúc
  thanh toán, trả một phần hay toàn bộ đơn bằng `gift_card` (`{"code": "…"}`) hay `store_credit`
  (`{"amount": <tối đa>}`, tuỳ chọn); cần đăng nhập. Giá trị được giữ tới khi đơn được xác nhận, và trả lại
  khi đơn bị huỷ hay hết hạn giữ.
- Sổ cái cho thấy phần còn nợ trên thẻ và số dư (`gift_card.liability`, `store_credit.liability`).

## Nhãn, ghi chú, trường tự định nghĩa

Hồ sơ là bản ghi loại `customers.customer`: nhãn nội bộ, ghi chú và trường tự định nghĩa dùng
endpoint chung của kernel (`/api/v1/kernel/tags`, ghi chú trên bản ghi, định nghĩa trường với đích
`customers.customer`). Lọc danh sách theo nhãn bằng `?tags=<id>,<id>`.

## Permission

| Permission | Cho phép |
|---|---|
| `customers.read` | Xem hồ sơ trong phạm vi (đã che), lịch sử mua, nhóm |
| `customers.manage` | Đăng ký tại quầy, sửa hồ sơ, gắn nhãn |
| `customers.contact.reveal` | Xem đủ số điện thoại, email (có audit) |
| `customers.segments.manage` | Tạo, sửa, xoá nhóm và thành viên |
| `customers.loyalty.manage` | Quy tắc điểm, hạng, phần thưởng voucher (cả cửa hàng) |
| `customers.loyalty.adjust` | Cộng / trừ điểm bằng tay |
| `customers.engagement.manage` | Nhắc giỏ và thưởng giới thiệu |
| `customers.stored_value.manage` | Phát hành, tra, huỷ thẻ quà tặng; đổi số dư cửa hàng |

Người mua không cần permission: `/me` chỉ cần tài khoản đã đăng nhập.

## Dữ liệu cá nhân

Hồ sơ, lịch sử mua và nhóm có trong bản xuất dữ liệu của người dùng. Khi tài khoản bị xoá, hồ sơ mất
mọi trường cá nhân (tên, số điện thoại, email, ngày sinh, trường tự định nghĩa) và liên kết với tài
khoản; con số tổng và tóm tắt đơn còn lại, không mang gì nhận ra người đó. Căn cứ cần đối chiếu với
người có chuyên môn pháp lý: Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15 và NĐ 356/2025.
