# Đối chiếu hai chiều: thiết kế 2 ↔ lộ trình — THƯƠNG MẠI và WEBSITE BÁN HÀNG (agent B)

Ngày 13/11/2026. Chỉ đọc, không sửa repo. Phạm vi: bước 36, 41–49 (+ "Sau core" phần thương mại).
Phần nền tảng / quản trị chung (15–35, 37–40, 50–52) chỉ ghi tham chiếu khi giao nhau.

## Ký hiệu đường dẫn

Thiết kế (artifact "Autoxide Admin" bản 37 và "Autoxide Web" bản 27):

| Viết tắt | File |
|---|---|
| `AO` | `admin/Orders.dc.html` (quản trị: đơn hàng, POS) |
| `AM` | `admin/Main.dc.html` (quản trị: menu, trang tổng quan phân hệ) |
| `MAP` | `admin/Map.dc.html` (bản đồ phân hệ) |
| `SM`, `SC`, `SP`, `SK`, `SS`, `SA`, `SL`, `SR` | `web/` `Main`, `Category`, `Product`, `Checkout`, `Seller`, `Account`, `Lookup`, `Article` `.dc.html` (mặt tiền) |
| `R` | `docs/ROADMAP.md`; `S36`…`S49` = `docs/steps/<số>-*.md` |

Trạng thái trong bảng: **API** = có mục lộ trình + có API trong đặc tả · **MỤC** = có mục nhưng đặc tả chưa nêu
API · **KHÔNG** = không có trong lộ trình/đặc tả · **UI** = chỉ là việc của giao diện · **ĐỀ BÀI** = đã có trong
`docs/prompts/ui-api-gaps.md` / `ui-help-api.md` (không tính là thiếu mới).

Nhận xét chung: **mọi bước 36, 41–49 đều chưa làm** — đó là bình thường. Bảng dưới chỉ nói đặc tả/lộ trình có
hay không. Phía quản trị, thiết kế chỉ vẽ thật **một** màn thương mại (Đơn hàng + popover POS, `AO`); mọi phân hệ
thương mại khác chỉ có trang tổng quan (`AM:675-760`) và mục menu trỏ `#`/rỗng (`AM:543-550`).

---

## 1. CHIỀU 1 — thiết kế → lộ trình

### 1.1 Mặt tiền: trang chủ, tìm kiếm, danh mục (`SM`, `SC`)

| Tính năng / khối (trích) | Bước · mục `R` | Endpoint/bảng trong đặc tả | Trạng thái |
|---|---|---|---|
| Gợi ý tìm gom từ khoá / "trong danh mục" / sản phẩm có giá / bài viết / cửa hàng; không dấu, đúng dấu xếp trước (`SM:40-41`, `SM:363-376`) | 41 · `R:912` | `GET /public/search/suggest`, trait `SuggestSource` (`S41:452`) | API |
| Chọn cửa hàng → giá và hàng có sẵn theo cửa hàng (`SM:147-158`, `SM:319`) | 41 · `R:910`; 42 · `R:945`; 34 (cửa hàng công khai) | `GET /public/products` + `store_id` (`S41:246`); `GET /public/availability` (`S42:393`); `GET /organizations/public/stores` (S34:564) | API |
| Thẻ sản phẩm: "Còn N ở CN…", "Hết · giao từ kho" (`SM:298-305`, `SC:375`) | 42 · `R:945` | `GET /public/availability` | API |
| Thẻ: "621.000 ₫ với mã BEPMOC10", không gạch giá (`SM:308`) | 43 · `R:966` | `GET /public/promotions/for-product/{id}` (`S43:355`) | API |
| Thẻ: điểm sao + số đánh giá, "3 cỡ" (`SC:144`, `SC:367`) | 41 · `R:912` | storefront entry thêm tóm tắt biến thể, điểm (`S41:460`) | API |
| Khối mã giảm công khai "còn 732 lượt" (`SM:110-114`) | 43 · `R:964` | `GET /public/promotions` (`S43:189`); `remaining_uses` chỉ khi số thật (`S43:367`) | API |
| Hero "Bộ sưu tập mùa đông", khối "Bán chạy tuần này" (`SM:68-73`, `SM:120`) | 41 · `R:912` (sort `best_selling`) ; bộ sưu tập thủ công: — | `sort=best_selling` (`S41:457`) | API (bán chạy) / **KHÔNG** (bộ sưu tập; từ khoá: `bộ sưu tập`, `collection`, `banner`, `merchandis`) |
| Khối "Chợ Bếp Mộc": 123 người bán đã xác thực, thẻ người bán có sao + số sản phẩm (`SM:167-177`, `SM:326-330`) | 46 · `R:1034` | chỉ có `GET /public/sellers/{slug}` (`S46:177`) | **KHÔNG** có danh sách người bán công khai (từ khoá `/public/sellers`) |
| Lọc "Nơi bán: Bếp Mộc bán / Người bán trên Chợ" có số đếm (`SC:102-104`, `SC:337`) | 41 · `R:910` | tham số `/public/products` không có người bán; đặc tả 41 không có khái niệm người bán (grep `owner_org_id`, `seller` trong S41: 0 kết quả) | **KHÔNG** |
| Lọc chất liệu, "dùng được bếp từ", khoảng giá, ≥ 4,7★, "chỉ hàng có sẵn ở CN…" (`SC:81-111`) | 41 · `R:910`, `R:912`; 42 · `R:945` | `attr.<code>`, `facets=true`, lọc số sao (`S41:457-459`); hàm lọc còn hàng qua `inventory::api` (`S42:396`) | API |
| Sắp xếp Bán chạy / Đánh giá cao / Giá / Mới nhất (`SC:125`) | 41 · `R:912` | `sort=best_selling|rating|price_*|newest` | API |
| Mục menu "Khuyến mại" → trang sản phẩm đang khuyến mại (`SM:345`, `SC` nav) | 43/41 | không có lọc `on_sale`/theo chương trình (từ khoá `on_sale`, `đang giảm`, `promotion_id=`) | **KHÔNG** |
| Đăng ký bản tin ở chân trang (`SM:203-211`) | 21 (agent kia) | — | tham chiếu |
| Logo Bộ Công Thương, thông tin chủ sở hữu ở chân trang (`SM:221-222`) | 37 (agent kia) | — | tham chiếu |
| "Cần giúp?": gọi, **nhắn tin với Bếp Mộc**, tra đơn, chính sách (`SM:54`) | chat: "Sau core" `R:1189` | — | UI (nếu là liên kết Zalo/điện thoại); chat thật = để sau |

### 1.2 Trang sản phẩm (`SP`)

| Tính năng | Bước · mục | Đặc tả | Trạng thái |
|---|---|---|---|
| Giá niêm yết gồm GTGT 8% + dòng "Còn X với mã…", mức giảm tính trên giá ngay trước KM, hạn, lượt còn, không cộng dồn (`SP:92-94`) | 41 `R:906`, 43 `R:966` | `S43:355-360`; giá trước KM = lịch sử giá `S41:206` | API |
| Chọn biến thể (cỡ) có giá từng cỡ (`SP:98-101`) | 41 `R:900` | `by-slug` (`S41:249`) | API |
| Giao tận nơi / Nhận tại cửa hàng; "có sau 2 giờ, giữ 3 ngày, người nhận hộ" (`SP:107-128`) | 44 `R:985`; 45 `R:1019` | `ready_at`, `hold_days` (`S44:431-433`); người nhận hộ (`S45:534`) | API |
| Bảng tồn theo từng cửa hàng, "Xem cửa hàng khác" (`SP:120-128`) | 42 `R:945` | `/public/availability` bỏ `store_id` | API |
| Báo khi có hàng (theo cửa hàng × cỡ) (`SP:119`, `SP:486`) | 42 `R:943` | `POST /public/restock-alerts` (`S42:402`) | API |
| Lưu sản phẩm không cần đăng nhập, đồng bộ khi đăng nhập (`SP:86`, `SP:483`) | 47 `R:1061` | `GET/POST /me/wishlist` (`S47:203`) | API (lưu cục bộ = UI; gộp khi đăng nhập: gọi lần lượt, không có endpoint gộp — nhỏ) |
| Ngày giao tuyệt đối + giờ chốt "đặt trước 16:00, còn 1 giờ 55 phút" (`SP:147`) | 44 `R:985` | `eta {earliest_date, latest_date, order_by}` (`S44:429`) | API |
| Ước tính phí theo tỉnh, ngưỡng miễn phí + thanh tiến độ (`SP:148`, `SP:159-163`) | 44 `R:988`; 45 `R:1019` | `GET /public/shipping/estimate` (`S44:422`) | API |
| Ưu điểm: Đổi trả 7 ngày, **Bảo hành 24 tháng** (`SP:500`) | 45 `R:1025` | 45 giả định `VariantSnapshot.warranty_months` (`S45:186`) nhưng **S41 không có cột/trường bảo hành** (grep `warranty`, `bảo hành` trong S41: 0) | MỤC (thiếu trường ở 41) |
| Thông số + "Thông tin nhãn hàng hoá theo thuộc tính bắt buộc" (`SP:193-195`, `SP:508`) | 41 `R:903` | `catalog_category_attributes.required` (`S41:181`) | API |
| Đánh giá: phân bố sao, lọc theo sao, "Có ảnh (24)", bộ sưu tập ảnh, trả lời của shop (`SP:202-231`) | 45 `R:1012`, `R:1019` | `GET /public/products/{id}/reviews` + `has_media`, `rating`, `…/reviews/media` (`S45:270`, `S45:537`) | API |
| Tóm tắt AI "nhân viên đã duyệt", không lược ý chê (`SP:215-217`) | 45 `R:1019`; 18 | `S45:537-539` | MỤC (đặc tả nói "bản nháp nhân viên duyệt" nhưng chưa nêu endpoint/bảng lưu bản tóm tắt) |
| Hỏi đáp sản phẩm, chỉ tài khoản đã xác thực SĐT (`SP:239-255`) | 22/41 | — | ĐỀ BÀI (A) |
| "Thường mua cùng" (`SP:262-268`) | 41 `R:921` chỉ có "tương tự theo ý nghĩa" | `GET /public/products/{id}/similar` (`S41:255`) — khác nghĩa "mua cùng" | **KHÔNG** (từ khoá `mua cùng`, `bought together`, `cross-sell`, `upsell`, `frequently`) |
| "Mua ngay" (`SP:142`) | 45 | giỏ + checkout | UI |

### 1.3 Giỏ hàng và đặt hàng 4 bước (`SK`)

| Tính năng | Bước · mục | Đặc tả | Trạng thái |
|---|---|---|---|
| Giỏ nhóm theo nơi bán, nhóm người bán có "Đã xác thực danh tính" (`SK:89-102`) | 45 `R:998`; 46 `R:1034` | `SellerSplit` (`S45:161`); `ensure_can_sell` (`S46:54`) | API |
| Bộ chọn "Mã của bạn" kèm lý do chưa dùng được ("Thêm 550.000 ₫ hàng Bếp Mộc"), ô nhập sau liên kết (`SK:112-124`, `SK:475-482`) | 43 `R:964` | `GET /public/promotions/eligible?cart_id` (`S43:364`) | API |
| Thanh tiến độ miễn phí giao **theo từng nơi bán** (`SK:230`) | 45 `R:1019` | `free_shipping_remaining` theo nơi bán (`S45:527`) | API |
| Mua không cần tài khoản là lựa chọn chính (`SK:132`) | 45 `R:1019` | `Cart-Token`, checkout công khai (`S45:240`) | API |
| Địa chỉ hai cấp tỉnh → phường/xã (`SK:139-144`) | 44 `R:976`; `address` (34) | `Address`, `AdministrativeUnits` | API |
| Nhận tại cửa hàng miễn phí, có sau 2 giờ (`SK:148`) | 44 `R:985` | `Pickup` (`S44:198`) | API |
| Phí giao riêng của người bán 30.000 ₫, 2–4 ngày (`SK:150`, `SK:400`) | 44 `R:978`; 46 `R:1038` | phương thức theo nút (seller là nút) `S44:188` | API |
| Thanh toán: QR, thẻ, ví, COD (`SK:435`) | 36 `R:682`, `R:701`; 45 `R:1000` | `orders_payments.method` (`S45:219`) | API |
| **Một lần thanh toán cho đơn của Bếp Mộc + đơn người bán** qua QR "của đối tác thu hộ" (`SK:183`, `SK:194`) | 45/46 | "chung một `checkout_id` và một khoản thanh toán" (`S46:69`); **không nói gì khi hai nút khác chế độ dòng tiền** (từ khoá `khác chế độ`, `nhiều chế độ`, `mixed`) | **KHÔNG** (xem 3.2) |
| Xuất hoá đơn cho công ty: MST 10/13 số, "tên công ty lấy theo mã số thuế" (`SK:164-168`, `SK:449`) | 45 `R:1008`; 36 | `invoice_request` (`S45:211`); MST 10 hoặc 10-3 số (S36 `tax_profiles`) | API (nhập MST) / **KHÔNG** (tra tên theo MST; từ khoá `tra cứu mã số thuế`, `tax code lookup`, `lấy theo mã số thuế`) |
| Đồng ý điều khoản có phiên bản + đồng ý nhận tin tách riêng (`SK:172-174`) | 37 (agent kia); 45 `accept_terms_version` | `S45:249` | API / tham chiếu |
| Màn xong: 2 đơn, QR VietQR: mở app ngân hàng, lưu ảnh QR, sao chép STK + nội dung, trạng thái "Đang chờ tiền về" (`SK:182-199`) | 36 `R:701`; 45 `R:1019` | VietQR (`S36` mục VietQR); `GET /public/payments/{token}` (`S45:528`) | API |
| Màn xong: "Tạo tài khoản + passkey; hai đơn vừa đặt đã gắn vào tài khoản" (`SK:204-207`, `SK:494`) | 45 `R:1019`; 12/34 passkey | `POST /me/orders/claim` sau khi **xác thực SĐT** (`S45:532`) | ĐỀ BÀI (D, gần) — nhưng thiết kế không có bước OTP ở màn này |
| Ô nhập mã người tiếp thị / phiên livestream | 49 | `PUT /carts/{id}/attribution` (`S45:248`) | API — **CHƯA VẼ** |
| Dùng điểm thưởng / thẻ quà tặng khi thanh toán | 47 | `Tender` (`S45:168`) | API — **CHƯA VẼ** |
| Trả góp | 45 `R:1025` | `method = installment` | API — **CHƯA VẼ** |

### 1.4 Gian hàng người bán (`SS`)

| Tính năng | Bước · mục | Đặc tả | Trạng thái |
|---|---|---|---|
| Hồ sơ gian hàng, "Đã xác thực danh tính", thông tin người bán bắt buộc theo Luật TMĐT (`SS:72-77`, `SS:135-139`, `SS:353`) | 46 `R:1034` | `GET /public/sellers/{slug}` (`S46:177`) | API |
| Huy hiệu "Người bán tin cậy" có tiêu chí công khai (`SS:77`) | 52 `R:1178-1179` | sức khoẻ người bán (S52) | MỤC — tiêu chí "trả lời tin nhắn trong 24 giờ ≥ 90%" và "Thường trả lời trong 2 giờ" cần **chat** → "Sau core" `R:1189` (mâu thuẫn, xem 3.4) |
| "★ 4,8 · 1.204 đánh giá", tab đánh giá của cả gian hàng (`SS:77`, `SS:116-120`, `SS:335`) | 45/46 | `marketplace_sellers.rating_*` (`S46:145`) có cột; **không có endpoint công khai đánh giá theo người bán** (reviews chỉ theo sản phẩm `S45:270`) | MỤC (thiếu API) |
| Danh sách sản phẩm của gian hàng, nhóm theo loại của gian hàng, "đã bán 640" (`SS:91-106`) | 41/46 | không có lọc theo người bán ở `/public/products` | **KHÔNG** |
| Theo dõi gian hàng, báo khi có sản phẩm mới (`SS:81`, `SS:356`) | — | — | **KHÔNG** (từ khoá `theo dõi gian hàng`, `follow`, `người theo dõi`, `followers`) |
| Tab "Chính sách của gian hàng" (đổi trả, giao, hoá đơn, tranh chấp) (`SS:347-352`) | 45 `orders_policies`; 44 phương thức | chính sách là số (hạn đổi trả…), không có văn bản chính sách riêng của người bán | MỤC (một phần) |
| Báo vi phạm về gian hàng (`SS:82`, `SS:155-165`) | 37 (agent kia) | `POST /public/reports` (S37:400) | tham chiếu |
| "Mua trên sàn được bảo vệ thế nào": tiền giữ ở đối tác tới hết hạn đổi trả (`SS:64`, `SS:142-144`) | 36 `R:685`; 46 `R:1040` | D-187 | API |

### 1.5 Tài khoản người mua (`SA`) và tra cứu đơn vãng lai (`SL`)

| Tính năng | Bước · mục | Đặc tả | Trạng thái |
|---|---|---|---|
| Danh sách đơn, dòng thời gian 6 mốc, tách đơn theo nơi bán (`SA:82-101`, `SA:354-358`) | 45 `R:1003` | `GET /me/orders` (`S45:251`) | API |
| Huỷ đơn khi "Đang chuẩn bị" (`SA:355`, `SL:120`, `SL:330`) | 45 | `buyer_cancel_until` chỉ `pending_payment/confirmed` (`S45:230`); bảng chuyển: người mua chỉ huỷ ở `confirmed` (`S45:102`) | **mâu thuẫn** (xem 3.1) |
| "Nhắn người bán" (`SA:355`) | chat "Sau core" `R:1189` | — | **KHÔNG** (để sau) |
| Đổi trả 3 bước, ảnh bắt buộc theo lý do, cách gửi trả, số tiền + thời gian hoàn, hoá đơn điều chỉnh (`SA:169-197`, `SA:380-390`) | 45 `R:1005`, `R:1019` | `POST /me/orders/{id}/returns`, `return_method` (`S45:255`, `S45:535`) | ĐỀ BÀI (C: ảnh, xem trước) |
| Viết đánh giá chỉ khi đã mua + SĐT đã xác thực | 45 `R:1012`; 37 | `POST /me/orders/{id}/lines/{line}/review` | API |
| **Tải hoá đơn (PDF và XML)** (`SA:357`) | 36 `R:691`; 45 `R:1008` | chỉ có `GET /invoices/documents/{id}/render|einvoice` cho nhân viên (`invoices.read`, S36 API); **không có đường cho người mua** (từ khoá `tải hoá đơn`, `lookup_code`, `me/orders/{id}/invoice`) | **KHÔNG** (API) |
| "Mua lại" (`SA:357`) | — | — | UI |
| Sổ địa chỉ, mặc định, nhắc cập nhật địa chỉ cũ ba cấp (`SA:112-116`) | 44 `R:976` | `GET/POST /me/addresses` (`S44:231`) | API |
| Bảo mật: SĐT đã xác thực, passkey, mật khẩu, thiết bị (`SA:413-418`) | 12, 34, 37 (agent kia) | — | tham chiếu |
| Đồng ý theo mục đích, văn bản đã chấp nhận, tải/xoá dữ liệu (`SA:134-157`) | 37, 13 (agent kia) | — | tham chiếu |
| Tra đơn vãng lai: mã + SĐT → OTP → xem đơn, che SĐT, địa chỉ tới phường, phiên 30 phút (`SL:67-137`) | 45 `R:1019` | `POST /public/orders/lookup` (`S45:530`) | API + ĐỀ BÀI (D) |
| Huỷ / đổi địa chỉ / hỗ trợ / tạo tài khoản từ trang tra (`SL:120-135`) | 45 | — | ĐỀ BÀI (D) |
| Trong tài khoản **không có**: đã lưu (wishlist), điểm & hạng, bảo hành của tôi, tải hàng số, giới thiệu bạn, tranh chấp với người bán, gian hàng đang theo dõi | 47 `R:1056`, `R:1061`; 45 `R:1025`; 41 `R:921`; 46 `R:1042` | `GET /me/wishlist`, `/me/points`, `/me/referral` (S47), `GET /me/warranties` (`S45:274`), `POST /marketplace/disputes` (`S46:186`) | API — **CHƯA VẼ** |

### 1.6 Bài viết có gắn sản phẩm (`SR`)

| Tính năng | Bước | Trạng thái |
|---|---|---|
| Thẻ sản phẩm nhúng trong thân bài, "Thêm vào giỏ", "Sản phẩm nhắc tới trong bài" (`SR:95-106`, `SR:148-152`) | 51 (nhúng `embed`, agent kia) + 41 đăng ký loại đối tượng (`R:1122`) | API (tham chiếu) |
| Bản tiếng Anh dịch bằng AI có người duyệt; bình luận cần SĐT đã xác thực (`SR:80`, `SR:127-130`) | 17–18, 22 (agent kia) | tham chiếu |

### 1.7 Quản trị: Đơn hàng + POS (`AO`) và trang tổng quan các phân hệ thương mại (`AM`)

(Bỏ qua phần khung chung: view đã lưu, chọn cột, hàng loạt, ghi chú/hoạt động, yêu cầu duyệt — bước 51/52, agent kia.)

| Tính năng / khối | Bước · mục | Đặc tả | Trạng thái |
|---|---|---|---|
| Danh sách đơn website + quầy + sàn, lọc nút / kênh / dòng tiền (`AO:66-146`, `AO:463`) | 45 `R:1003` | `GET /orders?status&payment_status&org_id&channel…` (`S45:256`) | API (lọc "dòng tiền" `funds_flow` chưa có trong tham số) |
| Tab trạng thái: Chờ thanh toán / **Chờ xác nhận** / Đang xử lý / Đang giao / Hoàn tất / **Đổi trả** (`AO:420-424`, `AO:446`) | 45 `R:1003` | máy trạng thái `S45:83-113` | **mâu thuẫn** (xem 3.1) |
| Nút "Xác nhận đơn", "Bắt đầu chuẩn bị", "Tạo vận đơn" (`AO:424`) | 45 | `POST /{id}/transitions`, `POST /{id}/shipments` (`S45:257-258`) | API (trừ "Xác nhận đơn") |
| "**Gửi lại link thanh toán**" cho đơn chưa trả (`AO:424`) | — | chỉ có `POST /me/orders/{id}/payments` cho người mua | **KHÔNG** (từ khoá `gửi lại link`, `resend`, `payment link`) |
| Ngăn chi tiết: hàng hoá, giá và thuế **chụp lúc đặt** (`AO:218-227`) | 45 `R:1000`; 36 `R:687` | `orders_lines`, `orders_line_taxes` | API |
| Khối "Dòng tiền" (tự thu / thu hộ qua đối tác / COD / tiền mặt) (`AO:233-234`, `AO:425-430`) | 36 `R:685` | `ledger_flow_settings`, `FundsFlow` | API |
| Khối "Bút toán sổ cái" của đơn (`AO:238-244`, `AO:523-534`) | 36 `R:679`; 45 | `GET /ledger/entries?…` (S36) — **không có lọc theo nguồn (`source_module`, `source_id`)** trong tham số liệt kê | MỤC (thiếu lọc) + **mâu thuẫn bút toán** (3.2) |
| Giao hàng & chứng từ: vận đơn, hoá đơn ký hiệu/số (`AO:250-252`, `AO:549-550`) | 44 `R:982`; 36 `R:691` | `GET /shipping/shipments?order_id`; `orders_invoice_links` | API |
| Đề xuất hoàn tiền của agent chờ người duyệt (`AO:200-202`, `AO:562-565`) | 51 (agent kia) + 45 `POST /{id}/refunds` | — | tham chiếu |
| In phiếu giao hàng loạt; in phiếu một đơn (`AO:157`, `AO:257`) | 36 `R:707` (mẫu in) + 51 hàng loạt | `GET /invoices/documents/{id}/render` (S36) | API |
| **Xuất CSV đơn theo bộ lọc bằng job** (`AO:69`, `AO:609-620`) | — | 45 không có export; 51 không có export chung (grep `export`, `xuất csv` trong S45/S51/S38: 0) | **KHÔNG** |
| POS: chọn cửa hàng, tìm hàng, tồn tại cửa hàng, tiền mặt/thẻ/QR, GTGT, "tạo đơn và in hoá đơn máy tính tiền" (`AO:287-318`, `AO:491-500`, `AO:642-648`) | 45 `R:1010`; 36 `R:698` | `POST /orders/pos/sales` (`S45:265`) | API |
| POS: ca bán (mở/đóng, tiền đầu ca, chênh lệch) | 45 `R:1010` | `POST /pos/shifts` (`S45:264`) | API — **CHƯA VẼ** |
| Menu Bán hàng: Đổi trả và hoàn tiền · Bán tại quầy · **Giỏ hàng bỏ dở** · Vận đơn · Phí vận chuyển · Đối soát COD · Đánh giá (`AM:543`) | 45, 44 | Giỏ bỏ dở: chỉ có job + event `orders.cart_abandoned` (`S45:310`), **không có API liệt kê cho nhân viên** | menu có, màn **CHƯA VẼ**; giỏ bỏ dở = **KHÔNG** (API) |
| Tổng quan Bán hàng: doanh thu hôm nay, đơn theo kênh, "12 đơn quá 24 giờ chưa xác nhận", "COD tuần 45 lệch 180.000 ₫", "3 vận đơn giao thất bại lần 2" (`AM:676-684`) | 45 `R:1014`; 44 `R:983`; 51 | `SummaryMetric`, `WorkSource` (51) | API (khung 51) |
| Tổng quan Sản phẩm: tồn thấp, lỗi CSV theo dòng, 236 SP dùng GTGT 8%, lô sắp hết hạn → tạo KM, phiếu chuyển đang đi (`AM:685-693`) | 41 `R:917`; 42 `R:936`, `R:938`; 36 `R:687` | `inventory.low_stock`, `catalog_transfers.errors` | API |
| Tổng quan Tiếp thị: "Flash sale vượt trần 55% > 50% → bị chặn", "chưa xuất hồ sơ thông báo KM tháng 12" (`AM:694-711`) | 43 `R:962` | `promotions_legal_limits`, `notice-export` (`S43:181`) | API |
| Tổng quan Khách hàng: tài khoản, mua lại 90 ngày, nhóm B2B, đồng ý nhận tin, "3 doanh nghiệp xin vào nhóm sỉ", "18 cặp tài khoản có thể trùng" (`AM:712-720`) | 47 `R:1054`; 34/37 | — | MỤC (B2B xin vào nhóm = "Sau core" B2B `R:1189`; gộp tài khoản trùng **KHÔNG** — từ khoá `gộp tài khoản`, `merge.*account`, `tài khoản trùng`; thuộc 34, agent kia) |
| Tổng quan Sàn: người bán, chờ chi trả, tranh chấp T-031, "5 người bán chưa chấp nhận hợp đồng bản 2 → tạm ngừng" , đợt chi trả với thực chi = doanh thu − 6% − 1,5% (`AM:721-729`) | 46 `R:1034-1042` | `S46` | API |
| Tổng quan Tài chính: phải thu cổng / COD, GTGT đầu ra tạm tính, HĐ bị trả lỗi, đối soát COD, khoá sổ tháng 10, % khớp đối soát (`AM:730-738`) | 36 `R:679-709`; 44 `R:983` | S36 API | API |
| Tổng quan Báo cáo: lãi gộp, lịch gửi báo cáo email, báo cáo đã lưu, hàng chậm luân chuyển (`AM:739-747`) | 45 `R:1014`; 38 (agent kia) | — | lịch gửi / báo cáo đã lưu / chậm luân chuyển = **KHÔNG** (từ khoá `lịch gửi báo cáo`, `scheduled report`, `chậm luân chuyển`, `báo cáo đã lưu`) — thuộc 38, báo agent kia |
| Cài đặt › Kết nối: cổng thanh toán, email/SMS, HĐĐT, AI — **không có hãng vận chuyển** (`AM:553`), dù bảng kết nối ghi "Hãng vận chuyển 3 hãng" (`AM:772`) | 44 `R:980` | `GET/POST /shipping/carrier-accounts` (`S44:236`) | API — mục menu thiếu |
| `MAP`: Khách hàng chỉ ghi "Bước 34" (`MAP:173`); không có phân hệ / mục nào cho kênh bán (48) và tiếp thị liên kết, livestream (49) | 47, 48, 49 | — | **CHƯA VẼ** (cả trong `docs/research/ui-design.md`: grep `channels|affiliate|livestream|điểm thưởng|loyalty` → 0) |

---

## 2. CHIỀU 2 — lộ trình → thiết kế

"Vẽ" = có màn hay khối dùng được; "menu" = chỉ có mục menu / số trên trang tổng quan; "CHƯA VẼ" = không có gì.

### Bước 36 (`R:677-711`)

| Mục `R` | Mặt người dùng | Thiết kế |
|---|---|---|
| 677 `rust_decimal` | — | không cần màn hình |
| 679 sổ cái kép | kế toán | khối bút toán trong đơn (`AO:238`), bảng "Bút toán gần đây" (`AM:735`); **màn Sổ cái (bút toán, số dư tài khoản, đảo bút toán, bảng cân đối thử) CHƯA VẼ** |
| 682 cổng thanh toán, webhook, đối soát | kế toán | menu "Thanh toán và đối soát" + % khớp (`AM:736`); **hàng chờ sai lệch `ledger_exceptions` (lệch tiền, tiền vào không mã, trả hai lần, trả trễ) + xử lý tay CHƯA VẼ** |
| 685 ba chế độ dòng tiền | chủ khách, kế toán | khối Dòng tiền trong đơn (`AO:233`); **màn cài chế độ theo nút (`/ledger/flow-settings`) CHƯA VẼ** |
| 687 engine thuế, gói "vn" | kế toán | menu Thuế + việc "8% hết hiệu lực" (`AM:812`); **màn biểu thuế, hồ sơ thuế theo nút, nhập gói CHƯA VẼ** |
| 691 chứng từ, HĐĐT, uỷ nhiệm | kế toán | dòng HĐ trong đơn (`AO:252`); **danh sách HĐ, phát hành/huỷ/điều chỉnh/thay thế, ký hiệu, uỷ nhiệm CHƯA VẼ** |
| 694 lưu trữ, khoá sổ | kế toán | việc "Kỳ tháng 10 chưa khoá sổ" (`AM:733`); **màn Kỳ kế toán CHƯA VẼ** |
| 698 HĐ máy tính tiền | thu ngân | POS (`AO:317-318`) — vẽ |
| 701 VietQR tự khớp | người mua, kế toán | màn QR (`SK:193-199`) — vẽ; **tài khoản ngân hàng nhận + biến động chưa khớp CHƯA VẼ** |
| 705 tỉ giá | kế toán | CHƯA VẼ (ưu tiên thấp: mẫu chỉ VND) |
| 707 mẫu in | quản trị | nút In (`AO:157`, `AO:257`); **sửa mẫu, xem trước CHƯA VẼ** |
| 709 xuất kế toán | kế toán | nút "Xuất sổ cái" (`AM:731`); hồ sơ xuất CHƯA VẼ |

### Bước 41 (`R:897-922`)

| Mục | Mặt người dùng | Thiết kế |
|---|---|---|
| 897 chuẩn bị chung | — | không cần màn hình |
| 900 sản phẩm, biến thể | nhân viên catalog; người mua | mặt tiền vẽ (`SP`); **trình sửa sản phẩm (biến thể, tuỳ chọn, ảnh, dịch, nhóm thuế, kích thước) CHƯA VẼ** |
| 903 danh mục, thuộc tính bắt buộc, hàng cấm/có điều kiện | nhân viên; người mua | tab nhãn hàng hoá (`SP:195`); **quản lý cây danh mục/thuộc tính, giấy tờ hàng có điều kiện CHƯA VẼ** |
| 906 bảng giá, lịch sử giá | nhân viên giá | **CHƯA VẼ** (chỉ menu `AM:544`) |
| 910, 912 đọc công khai, tìm và lọc | người mua | vẽ (`SC`, `SM:40`) |
| 917 CSV, gợi ý AI | nhân viên | việc "2 dòng lỗi" (`AM:688`); **trạng thái nhập + bản nháp AI CHƯA VẼ** |
| 919 `catalog::api`, MCP | — | không cần màn hình |
| 921 hàng số, dịch vụ, combo, tương tự | người mua; nhân viên | **tải hàng số (người mua), combo CHƯA VẼ**; "Thường mua cùng" (`SP:262`) ≠ "tương tự" |

### Bước 42 (`R:931-947`)

| Mục | Mặt người dùng | Thiết kế |
|---|---|---|
| 931 kho, sổ biến động | nhân viên kho | bảng "Tồn thấp nhất" (`AM:690`); **màn Tồn kho + sổ biến động + điều chỉnh có lý do CHƯA VẼ** |
| 934 giữ chỗ | — (gỡ kẹt tay) | không cần màn hình (gỡ kẹt: thấp) |
| 936 chuyển kho, kiểm kê, tồn thấp | nhân viên kho | menu + việc (`AM:544`, `AM:688`); **phiếu chuyển, phiếu kiểm kê CHƯA VẼ** |
| 938 giá vốn, lô, serial | kho, kế toán | menu "Lô và hạn dùng" (`AM:544`); CHƯA VẼ |
| 940 nhận event `orders` | — | không cần màn hình |
| 941 **nhập hàng: nhà cung cấp, PO, phiếu nhập, trả NCC, công nợ** | thu mua, kho, kế toán | **CHƯA VẼ, không có cả mục menu** |
| 943 báo khi có hàng | người mua | vẽ (`SP:119`) |
| 945 tình trạng công khai | người mua | vẽ (`SM`, `SC`, `SP`) |

### Bước 43 (`R:957-968`)

| Mục | Mặt người dùng | Thiết kế |
|---|---|---|
| 957 mã, giảm tự động, mua X tặng Y, miễn phí giao | quản lý khuyến mại | bảng chương trình (`AM:699`); **trình tạo chương trình, sinh mã CHƯA VẼ** |
| 959 giữ chỗ lượt, flash sale | người mua | **trang/khối flash sale công khai (giờ, suất còn) CHƯA VẼ** |
| 960 tính giảm cho giỏ kèm lý do | người mua | vẽ (`SK:96` "Mã BEPMOC10: −…") |
| 962 trần pháp luật, thông báo Sở Công Thương | quản lý KM | việc trên tổng quan (`AM:697`); **màn xuất hồ sơ, đánh dấu đã nộp CHƯA VẼ** |
| 964, 966 mã dùng được; giá khi áp mã | người mua | vẽ (`SK:112`, `SP:92`) |

### Bước 44 (`R:976-989`)

| Mục | Mặt người dùng | Thiết kế |
|---|---|---|
| 976 sổ địa chỉ | người mua; CSKH | vẽ (`SA:112`, `SK:139`) |
| 978 phí tự đặt, nhận tại cửa hàng | quản trị vận chuyển | **CHƯA VẼ** (menu "Phí vận chuyển") |
| 980 `ShippingCarrier`, webhook | quản trị | **CHƯA VẼ; mục menu "Hãng vận chuyển" thiếu** |
| 982 vận đơn (nhiều kiện, giao một phần, thất bại, hoàn) | nhân viên kho/giao | dòng vận đơn trong đơn (`AO:251`); **màn Vận đơn, đặt hãng, in nhãn, tách kiện CHƯA VẼ** |
| 983 COD đối soát | kế toán | việc "lệch 180.000 ₫" (`AM:679`); **nhập bảng kê, sai lệch CHƯA VẼ** |
| 985 ngày giao, giờ chốt, lịch ngày lễ | người mua; quản trị | người mua vẽ (`SP:147`); **lịch làm việc / giờ chốt / ngày lễ (`shipping_calendars`) CHƯA VẼ** |
| 988 ước tính phí công khai | người mua | vẽ (`SP:159`) |

### Bước 45 (`R:998-1026`)

| Mục | Mặt người dùng | Thiết kế |
|---|---|---|
| 998, 1000 giỏ, đặt hàng | người mua | vẽ (`SK`) |
| 1003 vòng đời | nhân viên; người mua | vẽ (`AO`, `SA`) — lệch trạng thái (3.1) |
| 1005 đổi trả, hoàn tiền | người mua; CSKH, kho | người mua vẽ (`SA:169`); **màn nhân viên duyệt / nhận hàng / hoàn tiền CHƯA VẼ** |
| 1008 HĐĐT, mail | người mua; kế toán | dòng HĐ (`AO:252`), "Tải hoá đơn" (`SA:357`) |
| 1010 POS | thu ngân | vẽ một phần (`AO:287`); **ca bán CHƯA VẼ** |
| 1012 đánh giá, kiểm duyệt | người mua; kiểm duyệt | người mua vẽ (`SP:202`); **hàng chờ kiểm duyệt + trả lời + duyệt tóm tắt AI CHƯA VẼ** |
| 1014 báo cáo bán hàng | quản lý | tổng quan (`AM:739`); màn báo cáo CHƯA VẼ |
| 1016 chống bom hàng COD | CSKH | **CHƯA VẼ** (không menu, không chỉ báo rủi ro trong đơn) |
| 1019 mua không tài khoản trọn vẹn | người mua; nhân viên cửa hàng | người mua vẽ (`SK`, `SL`); **nhân viên cửa hàng: hàng đợi đơn nhận tại quầy, "đã sẵn sàng", bàn giao cho người nhận hộ CHƯA VẼ** |
| 1025 trả góp, bảo hành | người mua; kỹ thuật bảo hành | **CHƯA VẼ** (chỉ ưu điểm "Bảo hành 24 tháng" `SP:500`) |

### Bước 46 (`R:1034-1046`)

| Mục | Mặt người dùng | Thiết kế |
|---|---|---|
| 1034 người bán đăng ký, KYC, văn bản, gian hàng | **người bán**; vận hành sàn | gian hàng công khai vẽ (`SS`); hàng chờ KYC ở Tuân thủ (agent kia); **luồng đăng ký người bán + cài gian hàng CHƯA VẼ** |
| 1037 duyệt sản phẩm | vận hành sàn | menu + "18 sản phẩm chờ duyệt" (`AM:724`); CHƯA VẼ |
| 1038 phí sàn, tách đơn, tự giao/sàn giao | vận hành sàn; người mua | tách đơn vẽ (`SK:183`); **biểu phí theo ngành có ngày hiệu lực CHƯA VẼ** |
| 1040 chi trả, đối soát cho người bán | vận hành sàn; **người bán** | bảng đợt chi (`AM:726`); **sao kê / quyết toán / chứng từ khấu trừ cho người bán CHƯA VẼ** |
| 1042 tranh chấp, điểm, chế tài, cơ quan có thẩm quyền | người mua; người bán; sàn | việc T-031 (`AM:724`); **mở tranh chấp (người mua), trả lời (người bán), phân xử (sàn), điểm & chế tài CHƯA VẼ** |
| 1044 báo cáo thuế, kho của sàn | sàn; người bán | **CHƯA VẼ** |

### Bước 47 (`R:1054-1062`)

| Mục | Mặt người dùng | Thiết kế |
|---|---|---|
| 1054 hồ sơ khách, nhóm, phân khúc | CSKH, marketing | menu Tài khoản / Nhóm khách (`AM:547`); **hồ sơ khách (lịch sử mua, tổng chi, nhãn, ghi chú), nhóm theo quy tắc + xem trước CHƯA VẼ** |
| 1056 điểm, hạng | người mua; thu ngân; marketing | **CHƯA VẼ** (không menu, không trong tài khoản, không trong POS/checkout) |
| 1058 thẻ quà tặng, số dư (tắt mặc định) | — | CHƯA VẼ (chấp nhận được khi tắt; nên có trạng thái "khoá/tắt") |
| 1061 yêu thích, vừa xem, nhắc giỏ, giới thiệu bạn | người mua; marketing | nút lưu (`SP:86`); **tab Đã lưu, giới thiệu bạn, quy tắc nhắc giỏ CHƯA VẼ** |

### Bước 48 (`R:1071-1076`) và 49 (`R:1085-1091`)

| Mục | Mặt người dùng | Thiết kế |
|---|---|---|
| 1071, 1073, 1075 kênh bán: tài khoản kênh, liên kết SP, giá riêng, phân bổ tồn, đơn từ kênh, đối soát | chủ shop đa kênh, kho, kế toán | **CHƯA VẼ toàn bộ**; bộ lọc kênh trong đơn chỉ có Website/Quầy/Sàn (`AO:463`) |
| 1085 người tiếp thị KYC, link, mã, quy tắc hoa hồng | người tiếp thị; marketing | **CHƯA VẼ** (chỉ chữ "tiếp thị liên kết" ở mô tả KYC `AM:665`) |
| 1087 hoa hồng, chi trả có khấu trừ TNCN | kế toán; người tiếp thị | **CHƯA VẼ** |
| 1090 phiên livestream | người phát; quản lý; người mua | **CHƯA VẼ** (cả trang công khai của phiên `GET /public/live-sessions/{id}`) |

Mục thuần nền, không cần màn hình: 36 `R:677`; 41 `R:897`, `R:919`; 42 `R:934`, `R:940`; 43 `R:959` (phần giữ chỗ);
các trait (`SalesChannel`, `ShippingCarrier`, `PaymentGateway`) ngoài màn cài tài khoản đối tác.

---

## 3. Kiểm chéo nhất quán

### 3.1 Trạng thái đơn và quyền huỷ

| # | Thiết kế | Đặc tả | Ghi chú |
|---|---|---|---|
| a | Có trạng thái **"Chờ xác nhận"** cho đơn đã trả (QR) và COD, nút "Xác nhận đơn" (`AO:409`, `AO:414`, `AO:421`, `AO:424`); việc "12 đơn quá 24 giờ chưa xác nhận" (`AM:679`, `AM:813`) | Không có trạng thái này: trả xong → `confirmed`; COD đủ điều kiện → `confirmed` ngay (`S45:96-99`) | Hoặc thiết kế đổi ("Chờ xác nhận" = `confirmed`, nút đầu tiên là "Bắt đầu chuẩn bị"), hoặc 45 thêm chính sách "người bán phải nhận đơn trong N giờ" (sàn Việt Nam hay có; liên quan chỉ số `cancelled_by_seller` của 46). Đề xuất: thêm vào 45 (mục 4b) |
| b | "Đổi trả" là một trạng thái/tab (`AO:422`, `AO:446`) | Đổi trả **không đổi `status`**, chỉ đổi `payment_status`/`fulfillment_status` (`S45:110-112`) | Thiết kế đổi: tab "Đổi trả" = view lọc `fulfillment_status=returned*` hoặc trỏ màn Đổi trả |
| c | Không có tab/nhãn Đã giao, Giao thất bại, Đã huỷ ở quản trị (`AO:446`) | Có `delivered`, `delivery_failed`, `cancelled` | Thiết kế thêm |
| d | Đơn POS tạo xong là "Hoàn tất" (`AO:646`) | POS → `delivered`, job `complete_delivered` → `completed` khi hết hạn đổi trả (`S45:97`, `S45:265`) | Thiết kế đổi nhãn "Đã giao tại quầy" |
| e | Người mua huỷ được khi "Đang chuẩn bị" ("chưa giao cho vận chuyển nên huỷ được") (`SA:355`, `SL:120`, `SL:330`) | Người mua chỉ huỷ ở `confirmed` nếu chính sách cho; `buyer_cancel_until ∈ {pending_payment, confirmed}` (`S45:102`, `S45:230`) | Chọn một: thiết kế ẩn nút khi `preparing`, hoặc 45 thêm giá trị `preparing` cho `buyer_cancel_until` (huỷ khi chưa có vận đơn). Đề bài D đã nói "cùng chính sách huỷ" nhưng không xử lý chỗ lệch này |
| f | Trang sản phẩm: đổi trả chỉ "lỗi NSX hoặc giao sai" (`SP:500`); luồng đổi trả có lý do "Đổi ý" (`SA:380`) | Lý do được trả là chính sách — `orders_policies` chưa có danh sách lý do cho phép | Thiết kế thống nhất; 45 thêm `allowed_return_reasons` vào chính sách (gần đề bài C) |

### 3.2 Dòng tiền, sổ cái, D-187

| # | Chỗ lệch | Trích |
|---|---|---|
| a | **Một QR cho cả đơn của Bếp Mộc (tự thu) và đơn người bán (thu hộ)** — QR "của đối tác thu hộ" (`SK:183`, `SK:194`). Đặc tả: chế độ dòng tiền theo nút (`ledger_flow_settings` unique theo `org_id`, S36 bảng `ledger_flow_settings`), nhưng 45/46 chỉ nói "một khoản thanh toán" (`S46:69`), không nói khi hai nút khác chế độ. Không chốt thì hoặc tiền tự thu của chuỗi chạy qua tài khoản thu hộ (đúng luật nếu đối tác có giấy phép, nhưng đổi chế độ), hoặc không trả một lần được | thiếu hẳn |
| b | **COD trên sàn** (`AO:413`: đơn sàn COD; bút toán "Phải thu hãng vận chuyển (COD)" rồi "Phải trả người bán", phí sàn, khấu trừ — `AO:523-530`). Đặc tả 46 không nhắc COD (grep `cod` trong S46: chỉ khớp chữ "code"); 44 không nhắc sàn. Nếu hãng chuyển tiền COD về tài khoản của **nhà vận hành sàn** thì sàn đang thu hộ – chi hộ tiền người mua → trái D-187 (NĐ 52/2024 — cần đối chiếu); nếu về người bán thì sàn không trừ phí / khấu trừ thay được | thiếu hẳn |
| c | Bút toán trong thiết kế ghi **doanh thu ngay khi thu tiền** (`AO:532`) và ghi **phải trả người bán, phí sàn, khấu trừ ngay khi thanh toán** (`AO:528`; `AM:734` ghi "Phải trả người bán 382.950 ₫" cho đơn BM-2611-0417 mới "Đã xác nhận"). Đặc tả 45: thu tiền → Có `buyer_prepayments`, doanh thu khi `delivered` (`S45:453-460`); 46: chia cho người bán khi `orders.order_completed` (`S46:95`) | thiết kế sai so với đặc tả |
| d | **Ba đặc tả tự lệch nhau**: 36 ghi tiền thu hộ vào `payable.seller_held[S]` ngay lúc tiền vào và phí/khấu trừ là các sự kiện riêng, tạm giữ bằng `ledger_holds` + job `ledger.release_holds` (`S36:355-358`, `S36:292`, `S36:506`); 45 ghi `buyer_prepayments` (`S45:453`); 46 ghi phải trả người bán khi hoàn tất và có cơ chế giữ riêng `marketplace_settlements.hold_until` + `marketplace.release_settlements` (`S46:88`, `S46:95`, `S46:222`). Tên tài khoản cũng khác (`clearing.gateway` / `payment_clearing` / `partner_collection`) | mâu thuẫn trong đặc tả |
| e | Phí sàn ghi một dòng "Doanh thu phí sàn (6%)" (`AO:528`) — đặc tả tách phí thành doanh thu + GTGT đầu ra của phí (`S36:356`) | thiết kế thiếu |
| f | Khấu trừ thay ghi một dòng 1,5% cho **mọi** người bán (`AO:527-528`, `AM:728`), gian hàng "Loại: Doanh nghiệp" vẫn ghi "Sàn khai và nộp thuế thay người bán" (`SS:350`, `SS:353`). NĐ 117/2025 khấu trừ thay áp cho hộ / cá nhân kinh doanh, doanh nghiệp tự kê khai (`S46:86`); đặc tả tách hai tài khoản `withheld_vat` / `withheld_pit` | thiết kế sai; cần đối chiếu văn bản gốc |
| g | "Hoá đơn: người bán phát hành; sàn khai và nộp thuế thay **theo uỷ quyền**" (`AO:550`) — khấu trừ thay là nghĩa vụ luật định, không phải uỷ quyền; uỷ quyền (`invoices_mandates`) là chuyện **lập hoá đơn** thay | sửa chữ |

### 3.3 Thuế, hoá đơn

| # | Chỗ lệch | Trích |
|---|---|---|
| a | "Hoá đơn điện tử phát hành **khi đơn hoàn tất**" (`AO:550`, `SK:164-168`, `SA:354`, `SL:117`). Hoàn tất = sau hạn đổi trả. NĐ 123/2020 Điều 9 (sửa bởi NĐ 70/2025): thời điểm lập hoá đơn bán hàng hoá là lúc **chuyển giao quyền sở hữu / quyền sử dụng**, không phụ thuộc đã thu tiền — cần đối chiếu. Đặc tả 45 có lựa chọn `invoice_timing = on_complete` (`S45:230`) mà không có `on_delivered` và không cảnh báo | rủi ro pháp lý |
| b | Thuế GTGT chỉ tính trên hàng; phí vận chuyển ghi "Thu phí vận chuyển" không thuế (`AO:432-442`, `AO:533`); đặc tả 45 ghi `shipping_revenue` không có dòng thuế (`S45:458-460`); 44 không có nhóm thuế cho phí giao. Luật Thuế GTGT 48/2024: giá tính thuế gồm khoản phụ thu, phí thu thêm mà người bán được hưởng — cần đối chiếu | thiếu ở đặc tả 44/45 |
| c | Ô MST nhận "10 hoặc 13 chữ số" (`SK:449`); đặc tả 36 kiểm "10 hoặc 10-3 số". Từ 01/7/2025 cá nhân, hộ kinh doanh dùng **số định danh cá nhân 12 số** làm MST (Thông tư 86/2024/TT-BTC — cần đối chiếu) | thiếu ở 36 + thiết kế |
| d | Ký hiệu HĐ "1C26TBM" dùng cả cho đơn POS "Hoá đơn khởi tạo từ máy tính tiền" (`AO:550` + `AO:429`). Theo TT 78/2021, chữ sau năm của ký hiệu phân biệt loại (vd `M` cho HĐ từ máy tính tiền) — cần đối chiếu; ít nhất mẫu nên dùng hai dãy ký hiệu | dữ liệu mẫu |
| e | Hạn thanh toán: 36 `AUTOXIDE_LEDGER_PAYMENT_EXPIRY_MINUTES = 30` "(kể cả VietQR)" (`S36:516`) vs 45 `VIETQR_TIMEOUT_MINUTES = 1440` (`S45:316`) | mâu thuẫn trong đặc tả |
| f | Nội dung chuyển khoản "BM26110419" theo số đơn (`SK:197`) cho một QR trả 2 đơn — đặc tả: `transfer_code` từ id thanh toán (`AX` + 8 ký tự, S36 mục VietQR) | dữ liệu mẫu nên theo đặc tả |

### 3.4 Người bán, KYC, huy hiệu

| # | Chỗ lệch | Trích |
|---|---|---|
| a | "Bán trên sàn từ 03/2026" nhưng "Xác thực danh tính ngày 12/11/2026" (`SS:77`, `SS:353`) — trái luật "chưa xác thực thì chưa bán" (46 `ensure_can_sell`) | dữ liệu mẫu tự mâu thuẫn (trừ khi là "xác thực lại" — nên ghi rõ) |
| b | Huy hiệu tin cậy dùng tiêu chí "trả lời tin nhắn trong 24 giờ ≥ 90%" và "Thường trả lời trong 2 giờ" (`SS:77`); tài khoản có "Nhắn người bán" (`SA:355`). Chat là "Sau core" (`R:1189`); 46 chỉ có tin nhắn trong tranh chấp | thiết kế vượt lộ trình |
| c | Quyền mẫu `orders.write`, `inventory.write`, `tax.manage`, `marketplace.manage` (`AM:824`) không có trong đặc tả (có `orders.manage`, `inventory.adjust`, `tax.profiles_manage`, `marketplace.sellers.manage`…) | dữ liệu mẫu |

### 3.5 Số liệu mẫu tự mâu thuẫn

| # | Chỗ | Lỗi |
|---|---|---|
| a | `SP:88` "4,8 · 212 đánh giá" với phân bố `SP:461` (168/31/8/3/2) | trung bình thật = 4,70 |
| b | `SP:432` "Th 7 15/11 – CN 16/11", "CN 16/11 – Th 2 17/11", "Th 2 17/11 – Th 4 19/11" | 13/11/2026 là thứ Sáu → 15/11 là CN, 16/11 Th 2, 17/11 Th 3, 19/11 Th 5. `SL:325` ("thứ Bảy 14/11 – Chủ nhật 15/11") thì đúng |
| c | `SL:326` thớt gỗ 290.000 ₫ | nơi khác 260.000 ₫ (`AO:403`, `SK:391`); tổng 980.000 ₫ ở `SA:354` = 950.000 + 30.000 phí giao, còn `SL:113` không có dòng phí giao |
| d | `AM:735` đã ghi bút toán đảo hoàn tiền BM-2611-0409 (638.889 ₫) trong khi `AO:562-565` đề xuất hoàn cho chính đơn đó còn chờ duyệt | thứ tự thời gian |
| e | Các con số còn lại khớp: tổng 5 đơn mẫu (`AM:681` ↔ `AO:409-416`), thực chi người bán 24,6 tr × 92,5% = 22,755 tr (`AM:726`), phải trả 382.950 ₫, 638.889 ₫ = 690.000/1,08, ngưỡng miễn phí 1.000.000 ₫, giảm 10% BEPMOC10, hạn đổi trả 7 ngày (11/11 → 18/11; 30/10 → 06/11), GTGT 8% tới 31/12/2026 khớp gói "vn" của S36 | — |

### 3.6 Khuyến mại

| # | Chỗ lệch | Trích |
|---|---|---|
| a | 43 nói "giảm tự động thì `catalog` hiện giá đã giảm" (`S43:357-358`) nhưng `catalog` đăng ký trước `promotions` và đặc tả 41 không có đường nhận giá giảm tự động (chỉ `compare_at` tay, `S41:109`) | mâu thuẫn trong đặc tả — trang danh mục không hiện được giá flash sale/giảm tự động |
| b | Lý do mẫu "chỉ cho đơn đầu tiên" (`S43:366`) — bảng `promotions_promotions` không có điều kiện "đơn đầu tiên" (`S43:152`) | thiếu cột |
| c | Thiết kế đúng luật hiển thị: mã phải nhập không gạch giá, ghi "còn X với mã" (`SM:308`, `SP:92`) — khớp `S43:357-360` | — |

---

## 4. Danh sách hành động

### (a) THIẾT KẾ CẦN VẼ THÊM / SỬA — xếp theo mức quan trọng

| # | Vai trò | Màn / khối, nội dung | Căn cứ |
|---|---|---|---|
| a1 | Nhân viên bán hàng | **Sửa Đơn hàng**: trạng thái theo bảng 45 (bỏ/định nghĩa lại "Chờ xác nhận", bỏ tab "Đổi trả" thành view lọc, thêm Đã giao / Giao thất bại / Đã huỷ; POS → "Đã giao"); khối sổ cái theo 45/46 (thu tiền → `buyer_prepayments`; doanh thu khi giao; phần người bán khi hoàn tất; phí sàn tách GTGT; khấu trừ chỉ cho hộ/cá nhân, hai dòng GTGT/TNCN); sửa chữ "theo uỷ quyền" và "HĐ khi hoàn tất" | 3.1a–d, 3.2c–g, 3.3a; `AO:420-446`, `AO:523-550`, `AO:646` |
| a2 | Người bán trên sàn | **Seller center** (chưa có persona người bán trong `AM:533-536`): đăng ký + KYC + chấp nhận văn bản + giấy tờ ngành hàng + nộp duyệt; cài gian hàng (logo, mô tả, địa chỉ lấy hàng, tự giao/sàn giao, tài khoản nhận tiền); đơn của mình (Đơn hàng thu phạm vi nút seller); quyết toán, sao kê, đợt chi, chứng từ khấu trừ; tranh chấp (trả lời, đồng ý hoàn); điểm & chế tài + khiếu nại; gửi hàng vào kho sàn | 2 / bước 46; `S46:170-192` |
| a3 | Vận hành sàn | Người bán (danh sách + chi tiết + chuyển trạng thái), Duyệt sản phẩm (findings tự điền), Phí sàn (dòng có ngày hiệu lực + xem trước), Chi trả (đợt, chi lại), Tranh chấp (luồng tin, hạn, phân xử), Báo cáo thuế kỳ, Yêu cầu cơ quan có thẩm quyền | `AM:548`, `AM:721-729`; `S46:170-192` |
| a4 | Kế toán / đối soát | Sổ cái (bút toán, số dư, đảo, cân đối thử); **hàng chờ sai lệch thanh toán** (lệch tiền, tiền vào không mã, trả trùng, trả trễ — xử lý tay); tài khoản ngân hàng VietQR; Hoá đơn điện tử (danh sách, phát hành, bị trả lỗi → sửa gửi lại, huỷ/điều chỉnh/thay thế, ký hiệu, uỷ nhiệm); Thuế (biểu thuế, hồ sơ thuế theo nút, nhập gói); Kỳ kế toán (khoá sổ); Xuất kế toán; Mẫu in; Tỉ giá (thấp) | `AM:549`, `AM:730-738`; S36 API |
| a5 | Nhân viên cửa hàng | **Đơn nhận tại cửa hàng**: hàng đợi theo cửa hàng, "đã sẵn sàng" (báo người mua), bàn giao cho người nhận hộ (kiểm tên/SĐT/mã), quá hạn giữ → huỷ, trả chỗ | `SP:107`, `S45:534`, `S44:413` |
| a6 | CSKH, kho | **Đổi trả và hoàn tiền** (nhân viên): duyệt/từ chối có lý do, phương thức gửi trả, nhận hàng về (tình trạng từng dòng: nhập lại / hỏng), hoàn tiền, hoá đơn điều chỉnh; **Đánh giá**: hàng chờ, trả lời, ẩn chỉ với lý do trong danh sách, duyệt tóm tắt AI | `AM:543`; `S45:261`, `S45:271` |
| a7 | Nhân viên kho, thu mua | Tồn kho + sổ biến động + điều chỉnh có lý do; Chuyển kho; Kiểm kê; Lô/hạn dùng/serial; **Nhập hàng (nhà cung cấp, PO, phiếu nhập một phần, trả NCC) — thêm mục menu** | `AM:544`; `R:941`; `S42:199-212` |
| a8 | Quản trị vận chuyển | Vận đơn (đặt hãng, in nhãn, tách kiện, lịch sử trạng thái, giao lại); Phí vận chuyển (vùng theo tỉnh, bậc cân, miễn phí từ, điểm nhận); **Lịch giao: giờ chốt, ngày làm việc, ngày lễ**; Đối soát COD (nhập bảng kê, sai lệch); **thêm "Hãng vận chuyển" vào Cài đặt › Kết nối** | `AM:543`, `AM:553`, `AM:772`; `S44:229-246` |
| a9 | Quản lý khuyến mại | Trình tạo chương trình (loại, phạm vi, mục tiêu, cộng dồn, giới hạn lượt, cảnh báo trần pháp luật trước khi bật), sinh lô mã, flash sale (suất theo biến thể), xuất hồ sơ thông báo Sở Công Thương + đánh dấu đã nộp, lịch sử phiên bản | `AM:545`, `AM:694-711`; `S43:174-189` |
| a10 | Nhân viên catalog, giá | Trình sửa sản phẩm (biến thể, tuỳ chọn, ảnh, bản dịch, nhóm thuế, kích thước/cân, thuộc tính bắt buộc theo danh mục, giấy tờ hàng có điều kiện, hàng số, combo, bản nháp AI); Danh mục & thuộc tính; Bảng giá (lịch sử, "giá giải ra vì sao"); trạng thái nhập CSV theo dòng | `AM:544`; `S41:215-237` |
| a11 | Thu ngân | POS: mở/đóng ca (tiền đầu ca, đếm cuối ca, chênh lệch), quét mã vạch, khách theo SĐT (điểm thưởng), tiền khách đưa/tiền thối, trả hàng tại quầy | `AO:287-318`; `S45:264-265`; S47 đăng ký tại quầy |
| a12 | CSKH, marketing (CRM) | Hồ sơ khách (lịch sử mua, tổng chi, hạng, nhãn, ghi chú), nhóm theo quy tắc + xem trước số khớp, điểm & hạng (quy tắc, điều chỉnh tay có lý do), quy tắc nhắc giỏ & giới thiệu bạn, **Giỏ hàng bỏ dở**, rủi ro COD (thống kê người mua, quy tắc, danh sách chặn có hạn) + chỉ báo rủi ro trong ngăn đơn | `AM:547`, `AM:712-720`; S47 API; `S45:272` |
| a13 | Chủ shop đa kênh | **Phân hệ/mục Kênh bán** (hiện chưa có trong `MAP` và `ui-design.md`): tài khoản kênh, liên kết SP–mã kênh + giá riêng, phân bổ tồn + xem trước số đẩy, đơn từ kênh cần xử lý (thiếu ánh xạ, bán quá, xung đột), đối soát tiền kênh trả; bộ lọc kênh trong Đơn hàng thêm Shopee/TikTok… | `AO:463`; `S48:181-197` |
| a14 | Marketing; người tiếp thị; người phát | **Tiếp thị liên kết & livestream**: duyệt đối tác, quy tắc hoa hồng (có ngày hiệu lực, xem trước), hoa hồng & chi trả (khấu trừ TNCN, đánh dấu đã chi tay); phiên livestream (sản phẩm gắn, nhật ký, báo cáo); **cổng người tiếp thị** (link, mã, thống kê, thu nhập, bảng thuế năm); trang công khai của phiên | `S49:177-194` |
| a15 | Người mua | Tài khoản thêm: Đã lưu, Điểm & hạng, Bảo hành của tôi, Tải hàng số, Giới thiệu bạn, Tranh chấp với người bán, Báo có hàng đã đăng ký; Checkout thêm: trả góp, dùng điểm, mã người tiếp thị, giỏ chỉ hàng số/dịch vụ (không bước giao); trang Flash sale (giờ, suất còn); trang "Khuyến mại"; trang "Bán trên Chợ Bếp Mộc" (mời người bán đăng ký) | `SA:352`; `SK:435`; 2 / bước 41, 43, 45, 46, 47 |
| a16 | Người mua | Checkout màn xong: tạo tài khoản + passkey phải có **bước OTP xác thực SĐT** trước khi gắn đơn (hoặc chỉ gắn đơn của chính checkout này) | `SK:204-207`, `SK:494`; `S45:532` |
| a17 | (dữ liệu mẫu) | Sửa: điểm 4,8 → 4,7 hoặc đổi phân bố (`SP:88`, `SP:461`); thứ trong tuần (`SP:432`); giá thớt + dòng phí giao ở tra cứu (`SL:113`, `SL:326`); "Bán từ 03/2026" vs KYC 12/11/2026 (`SS:77`, `SS:353`); thứ tự hoàn tiền BM-2611-0409 (`AM:735`); tên permission mẫu (`AM:824`); nội dung CK theo mã thanh toán (`SK:197`); ký hiệu HĐ riêng cho máy tính tiền; MST nhận 12 số | 3.3, 3.4, 3.5 |
| a18 | (bản đồ) | `MAP`: Khách hàng ghi "Bước 34, 47" và liệt kê điểm/hạng, CRM; thêm chỗ cho Kênh bán (48) và Tiếp thị liên kết (49); Tài chính thêm "Hàng chờ sai lệch", "Xuất kế toán"; Sản phẩm thêm "Nhập hàng" | `MAP:150-195` |
| a19 | Người mua | Huy hiệu tin cậy: bỏ tiêu chí "trả lời tin nhắn" và "Thường trả lời trong 2 giờ", bỏ "Nhắn người bán" — trừ khi chủ dự án mở chat sớm | `SS:77`, `SA:355`; `R:1189` |

### (b) LỘ TRÌNH CẦN BỔ SUNG — xếp theo mức quan trọng

"Thiếu hẳn" = không có ở lộ trình lẫn đặc tả; "lệch" = có nhưng các đặc tả mâu thuẫn.

| # | Bước | Mục đề xuất | Loại |
|---|---|---|---|
| b1 | 46 (+44, 36) | **COD trên sàn**: tiền COD do hãng chuyển về đâu (tài khoản thu hộ của đối tác có giấy phép / người bán), không bao giờ về tài khoản của nhà vận hành sàn (D-187); khi người bán tự giao COD thì sàn thu phí và khấu trừ thay bằng cách nào (hoá đơn phí định kỳ, công nợ); ghi quyết định có căn cứ NĐ 52/2024, NĐ 117/2025 — cần đối chiếu | thiếu hẳn |
| b2 | 45/46 (+36) | **Một checkout nhiều nút khác chế độ dòng tiền**: chọn một quy tắc — chế độ của nút website/sàn áp cho cả checkout, hoặc tách `payment_action[]` theo chế độ, hoặc từ chối `orders.mixed_funds_flow`; test trên cả hai database | thiếu hẳn |
| b3 | 36/45/46 | **Thống nhất bút toán và tạm giữ**: một bộ tên tài khoản; tiền thu hộ ghi vào đâu lúc thu (`buyer_prepayments` hay `payable.seller_held`); phí/khấu trừ ghi lúc nào; chỉ một cơ chế tạm giữ (`ledger_holds` hay `marketplace_settlements.hold_until`) | lệch (`S36:355-358`, `S45:453-460`, `S46:88-98`, `S46:222`) |
| b4 | 41 (+46) | **Sản phẩm của người bán trên website của khách**: quy tắc hiển thị sản phẩm của nút `seller` (dưới nút sàn) trên storefront của chuỗi; lọc `seller`/`sold_by` ở `/public/products`; thẻ sản phẩm mang người bán (tên, slug, đã xác thực); `warranty_months`; điểm cắm `CatalogExtensions::set_listing_gate` mà 46 giả định (`S46:70-74`) | thiếu hẳn ở 41 |
| b5 | 45/36 | **Thời điểm lập hoá đơn** đúng NĐ 123/2020 Điều 9: thêm `invoice_timing = on_delivered` làm mặc định, `on_complete` chỉ khi có căn cứ — cần đối chiếu; **GTGT trên phí vận chuyển** (nhóm thuế cho phí giao ở 44, dòng thuế ở 45) — cần đối chiếu Luật 48/2024 | thiếu hẳn (rủi ro pháp lý) |
| b6 | 45 | Chính sách **"người bán / cửa hàng phải nhận đơn trong N giờ"** (nếu giữ "Chờ xác nhận" của thiết kế) hoặc ghi rõ không có; `buyer_cancel_until` thêm `preparing` (huỷ khi chưa có vận đơn); `allowed_return_reasons` trong `orders_policies` | thiếu |
| b7 | 43 (+41) | Giá giảm tự động / flash sale hiện trên danh sách công khai (catalog nghe event `promotions.*` qua `Subscription::optional` để cập nhật storefront entry, hoặc endpoint giá hiển thị của `promotions`); lọc "đang khuyến mại"; flash sale công khai (giờ, suất còn); điều kiện "đơn đầu tiên" | lệch / thiếu (`S43:357`, `S43:366`) |
| b8 | 46 (+47) | Danh sách người bán công khai; đánh giá theo người bán công khai; **theo dõi gian hàng** + báo sản phẩm mới qua `notifications` (đồng ý riêng, bước 37) | thiếu hẳn |
| b9 | 45 | Người mua tải hoá đơn của đơn mình (và bằng token xem đơn vãng lai); nhân viên: danh sách giỏ bỏ dở, gửi lại link thanh toán, xuất CSV đơn theo bộ lọc (hoặc xuất chung theo view ở 51 — báo agent kia), bàn giao nhận tại cửa hàng có mã | thiếu hẳn |
| b10 | 36 | Tạm hoãn: chốt một hạn thanh toán VietQR (30 phút ở 36 vs 1440 ở 45); kiểm MST nhận số định danh 12 số (TT 86/2024 — cần đối chiếu); ký hiệu HĐ riêng cho máy tính tiền trong dữ liệu mẫu | lệch |
| b11 | 52/46 | Huy hiệu tin cậy: tiêu chí chỉ từ dữ liệu có thật (giao đúng hạn, huỷ do người bán, tranh chấp thua, điểm) — bỏ "phản hồi tin nhắn" khi chat còn ở "Sau core" | lệch với thiết kế |
| b12 | `docs/research/ui-design.md` | Bảng phân hệ thêm hàng cho 47 (điểm, hạng, CRM), 48 (kênh bán), 49 (tiếp thị liên kết, livestream); thêm persona "người bán trên sàn" và "người tiếp thị" | thiếu |
| b13 | 41 (thấp) | "Thường mua cùng" (đồng xuất hiện trong đơn, bảng dẫn xuất) — hoặc ghi rõ giao diện dùng `similar` | thiếu |
| b14 | 38 (báo agent kia) | Lịch gửi báo cáo qua email, báo cáo đã lưu, hàng chậm luân chuyển (`AM:739-747`) | thiếu, ngoài phạm vi B |

### (c) API THIẾU — đề bài cho teammate (loại trừ phần đã có đề bài)

Đều là **thêm** (D-059), chạy trên cả SQLite và Postgres, có test, mô tả OpenAPI, mã lỗi + `locales/vi.json`.
Mỗi mục làm khi lộ trình tới bước đó (không nhảy cóc); trước đó chỉ ghi vào đặc tả.

| # | Bước | Endpoint / trait / bảng gợi ý | Lý do |
|---|---|---|---|
| c1 | 46/44/36 | `ledger_flow_settings.cod_remittance` (`partner_collection` / `seller`); `marketplace_settlements.funds_flow` nhận `CarrierCollect`; lỗi `marketplace.cod_flow_unsupported` khi cấu hình đưa tiền COD về nhà vận hành sàn; `shipping.cod_settled` mang `seller_org_id` | b1 |
| c2 | 45 | Checkout trả `payment_actions[]` (một mục mỗi chế độ) hoặc lỗi `orders.mixed_funds_flow` 422; `GET /carts/{id}` báo trước `funds_flow` theo nhóm | b2 |
| c3 | 41 | `GET /public/products?seller=<slug>&sold_by=own|marketplace` (+ `facets` cho "Nơi bán"); `ProductCard.seller {slug, name, verified}`; `catalog_variants.warranty_months`; `CatalogExtensions::set_listing_gate(Arc<dyn ListingGate>)`; quy tắc storefront cho nút seller | b4 |
| c4 | 46 | `GET /public/sellers?sort=rating|newest&category=` (Page); `GET /public/sellers/{slug}/reviews` (đọc từ `orders` qua `api` lọc `seller_org_id`, có `rating`, `has_media`); `POST/DELETE /me/followed-sellers/{slug}` + bảng `marketplace_followers` (cấp khách) + event cho `notifications` | b8 |
| c5 | 45 | `GET /me/orders/{id}/documents` và `GET /me/orders/{id}/documents/{doc_id}?format=pdf|xml` (gọi `Invoices` qua `api`, chỉ chứng từ của đơn mình); bản cho token xem đơn vãng lai | `SA:357`; b9 |
| c6 | 45 | `GET /orders/carts?status=abandoned&org_id&from&to` (`orders.read`; che liên hệ khi chưa đồng ý); `POST /orders/{id}/payment-link` (`orders.manage`, `Idempotency-Key`, gửi qua `Notifier`, chỉ khi `pending_payment`); `POST /orders/exports` (bộ lọc như `GET /`) → job → file có hạn — hoặc đi qua xuất chung của 51 | `AM:543`, `AO:424`, `AO:69` |
| c7 | 45/44 | Nhận tại cửa hàng: `POST /orders/{id}/pickup/ready` (báo người mua, gửi mã nhận 6 số), `POST /orders/{id}/pickup/handover {code | proxy_name, proxy_phone}`; `GET /orders?pickup_org_id&pickup_status` | a5 |
| c8 | 45 | `orders_policies.invoice_timing` thêm `on_delivered` (mặc định); `orders_policies.buyer_cancel_until` thêm `preparing`; `orders_policies.allowed_return_reasons`; (tuỳ quyết b6) `orders_policies.acceptance_hours` + lỗi `orders.acceptance_overdue` | b5, b6 |
| c9 | 44/45 | Nhóm thuế cho phí giao (`shipping_methods.tax_category`), `orders_line_taxes` cho dòng phí giao | b5 |
| c10 | 43/41 | `GET /public/promotions/display-prices?variant_ids&store_id` (giá sau giảm tự động, không gồm mã) hoặc cột `auto_sale_min_minor` trên `catalog_storefront_entries` cập nhật từ event `promotions.*`; `GET /public/promotions/flash-sales?store_id` (biến thể, giá, suất còn thật, `ends_at`); `GET /public/products?promotion=<id>|on_sale=true`; điều kiện `first_order_only` trên chương trình | b7 |
| c11 | 36 | Kiểm MST: nhận 10, 10-3 số và số định danh 12 số (cần đối chiếu TT 86/2024); (tuỳ chọn) trait `TaxpayerDirectory::lookup(tax_code)` để điền tên công ty (bộ giả cho test, bộ nối thật cần tài khoản đối tác — dừng hỏi) | `SK:168`, `SK:449` |
| c12 | 36 | `GET /ledger/entries?source_module&source_id` (khối bút toán trong ngăn đơn); một biến hạn thanh toán VietQR duy nhất | `AO:238`; 3.3e |
| c13 | 47 (thấp) | `POST /me/wishlist/merge {items[]}` (gộp danh sách lưu trên máy khi đăng nhập, chặn ở `WISHLIST_MAX`) | `SP:483` |
| c14 | 41 (thấp) | `GET /public/products/{id}/bought-together?limit` từ bảng dẫn xuất đồng xuất hiện (nghe `orders.order_completed`) | `SP:262` |

Đã có đề bài, không tính ở trên: hỏi đáp sản phẩm (A); alt ảnh theo ngôn ngữ (B); trả hàng kèm ảnh + xem trước tiền
hoàn (C); thao tác trên đơn vãng lai, throttle, che dữ liệu, tạo tài khoản từ số đã xác thực (D); nhật ký request ghi
tên tool MCP (E); trạng thái gợi ý theo người dùng, định nghĩa số liệu (`ui-help-api.md`).

---

## 5. Chưa chắc / cần người kiểm

- Căn cứ pháp lý ở 3.2b (COD trên sàn và NĐ 52/2024), 3.2f (NĐ 117/2025 chỉ khấu trừ hộ/cá nhân), 3.3a (NĐ 123/2020
  Điều 9 — thời điểm lập HĐ), 3.3b (GTGT trên phí giao, Luật 48/2024), 3.3c (TT 86/2024 — số định danh làm MST), 3.3d
  (ký hiệu HĐ máy tính tiền theo TT 78/2021): viết theo hiểu biết chung, **cần đối chiếu văn bản gốc / người có chuyên
  môn pháp lý** (D-186). Không chặn công việc.
- "Chờ xác nhận" có thể là chủ ý (sàn Việt Nam hay buộc người bán nhận đơn) — cần chủ dự án/agent chính chọn: sửa thiết
  kế hay thêm chính sách vào 45.
- Thiết kế cũ (List/Post/Page/Search.dc.html) chỉ lướt, không đối chiếu.
- Không đọc toàn bộ "Mã lỗi", "Test bắt buộc" của từng đặc tả; các khẳng định "KHÔNG có" dựa trên grep `docs/ROADMAP.md`
  + `docs/steps/*.md` với từ khoá đã ghi cạnh từng dòng.
