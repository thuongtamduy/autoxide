# Thiết kế giao diện 2: khung quản trị đa phân hệ và mặt tiền bán hàng

Thiết kế ngày 13/11/2026 (D-200). Giao diện là repo riêng (D-096, D-097); core chỉ có API (luật kiến
trúc 9). File này ghi lại **giao diện cần gì từ core**, để agent làm các bước sau không phải đoán. Bản vẽ
chi tiết: bản chụp ở [`docs/design/`](../design/README.md) (D-301), bản gốc ở canvas thiết kế của chủ dự án.

Đây là tài liệu nội bộ (tiếng Việt) cho chủ dự án và agent.

## 1. Vì sao làm lại

Thiết kế 1 (12/11/2026) coi trang quản trị là một CMS: menu chỉ có nội dung, media, bình luận, form.
Sau khi lộ trình mở rộng sang nền đa khách (bước 33–40) và module thương mại (bước 41–49), quản trị nội
dung chỉ còn là **một phân hệ trong mười hai**. Website công khai cũng đổi từ blog sang **mặt tiền bán
hàng** của một khách, blog thành một mục "góc bài viết".

## 2. Khung quản trị

### 2.1 Mười hai phân hệ

Thanh dọc bên trái, mỗi mục là một phân hệ gom các module cùng nghiệp vụ. Thứ tự cố định cho mọi khách;
cấu hình chỉ ẩn bớt, không đảo chỗ.

| # | Phân hệ | Module | Bước | Hiện với mô hình |
|---|---|---|---|---|
| 1 | Tổng quan | kernel | 51 | mọi mô hình |
| 2 | Bán hàng | `orders`, `shipping` | 44–45 | có bán hàng |
| 3 | Sản phẩm | `catalog`, `inventory` | 41–42 | có bán hàng |
| 4 | Tiếp thị | `promotions`, `newsletter`, `forms` | 20–21, 43 | mọi mô hình |
| 5 | Nội dung | `content`, `media`, `comments` | 16–17, 22 | mọi mô hình |
| 6 | Khách hàng | tài khoản người mua, `customers` | 34, 47 | mọi mô hình |
| 7 | Sàn | `marketplace` | 46 | sàn |
| 8 | Tài chính | `ledger`, `tax`, `invoices` | 36 | có bán hàng |
| 9 | Báo cáo | `reporting` | 38, 45 | có bán hàng |
| 10 | Tổ chức | `organizations`, `auth` | 34 | mọi mô hình |
| 11 | Tuân thủ | `compliance` | 37 | mọi mô hình |
| 12 | Cài đặt | kernel, `plans`, module kết nối | 33, 35 | mọi mô hình |

Phần của bước 47–49 nằm trong các phân hệ trên, không thêm phân hệ thứ mười ba (đối chiếu thiết kế 2, B-b12):

| Bước | Phần | Phân hệ | Hiện với mô hình |
|---|---|---|---|
| 47 | Hồ sơ khách (CRM), nhóm và phân khúc, điểm thưởng, hạng thành viên, thẻ quà tặng | 6 Khách hàng | mọi mô hình có bán hàng |
| 48 | Kênh bán (Shopee, Lazada, TikTok Shop…), liên kết sản phẩm, phân bổ tồn, nguồn cấp sản phẩm Google / Meta | 2 Bán hàng (đơn từ kênh), 3 Sản phẩm (liên kết, nguồn cấp) | có bán hàng |
| 49 | Tiếp thị liên kết (người tiếp thị, link, hoa hồng, chi trả), phiên livestream | 4 Tiếp thị | có bán hàng |

Hai vai trò có khung riêng ngoài nhân viên của khách (B-b12):

| Vai trò | Thấy gì | Bước |
|---|---|---|
| Người bán trên sàn | Khung quản trị thu gọn trong phạm vi gian hàng của mình (nút người bán): sản phẩm, đơn, trả hàng, tranh chấp, đối soát, phiếu hỗ trợ, sức khoẻ gian hàng; không thấy dữ liệu người bán khác | 46, 52, 53 |
| Người tiếp thị | Trang riêng: link và mã của mình, đơn ghi nhận, hoa hồng chờ / được chi, chứng từ khấu trừ thuế; không thấy dữ liệu người mua | 49 |

Ngoài ra có **mặt điều khiển** của nền tảng (khách hàng, gói, cấp phát, đội database, thuê bao, vào
dữ liệu của khách để hỗ trợ) — một khung riêng cho nhân sự vận hành, khách không thấy.

### 2.2 Ba điều quyết định một mục menu có hiện không

1. **Module đã bật cho khách** (bước 35). Chưa bật thì mục không hiện; API trả `module_not_enabled`.
2. **Permission ở nút tổ chức đang chọn** (bước 34). Không có quyền thì không hiện.
3. **Tính năng trong gói** (bước 35). Có module nhưng tính năng thuộc gói cao hơn thì mục **vẫn hiện**,
   kèm nhãn khoá, để người dùng biết nâng gói sẽ mở được gì; bấm vào chỉ giải thích.

Vì cả ba đều là dữ liệu của core, menu phải do core trả về (bước 51), không viết cứng ở giao diện.

### 2.3 Luật của khung

- Menu con của phân hệ là một hàng tab dưới thanh tiêu đề (thiết kế ban đầu dùng một cột bên trái —
  bỏ vì tốn chỗ).
- Thanh phân hệ (14/11/2026) theo kiểu của Shopify, Stripe, Linear bản mới:
  - nền sáng cùng tông trang, mục đang mở nổi lên như một thẻ, dính cạnh trái khi cuộn;
  - từ 1280px: rộng 232px, biểu tượng và tên cùng hàng; người dùng thu gọn được;
  - hẹp hơn: thanh biểu tượng có chữ nhỏ;
  - đáy thanh: Bản đồ, Trợ giúp, Thu gọn.
  Điện thoại dùng thanh dưới đáy nền sáng mờ. Hàng tab và hàng chip chỉ cuộn ngang.
- Ngữ cảnh "khách › nút tổ chức" luôn ở góc trên bên trái; đổi nút là đổi phạm vi mọi số liệu, danh
  sách và kết quả tìm.
- Ctrl + K tìm xuyên mọi phân hệ, trong phạm vi và permission của người dùng.
- Mỗi phân hệ có trang tổng quan: hành động chính, 4 số liệu, "cần xử lý", bảng dữ liệu gần đây, danh
  sách màn hình con có mô tả.
- Trang Tổng quan chung: số liệu từ các phân hệ đã bật, **hộp việc** gom việc chờ của mọi phân hệ (sắp
  theo mức gấp), hạn mức gói, hoạt động gần đây.
- Mọi màn hình dùng chung một kiểu bảng, bộ lọc, ngăn chi tiết, thông báo — như API dùng chung envelope.

**Bổ sung sau rà soát R4 (14/11/2026).** Gốc phần lớn lỗi là mỗi màn tự khai menu.

- **Điều hướng hai bậc, một nguồn.**
  - Thanh bên trỏ tới phân hệ.
  - Dải trên cùng là các **màn** của phân hệ, thứ tự cố định, giống nhau ở mọi màn trong phân hệ. Phân hệ chỉ
    có một màn thì không có dải này.
  - Tab bên trong một màn nằm ngay dưới tiêu đề.
  - Số đếm trên menu là việc đang chờ, lấy cùng nguồn với hộp việc.
  - Mục menu trỏ thẳng vào một tab được (`#tab`).
- **Điện thoại:**
  - thanh đáy gồm Tổng quan, Đơn, Việc, Tìm, Phân hệ;
  - "Tìm" mở bảng lệnh;
  - "Phân hệ" mở tấm từ đáy liệt kê mọi phân hệ, có nút sáng / tối;
  - giao diện theo cài đặt sáng / tối của máy khi người dùng chưa chọn.
- **Bảng lệnh:** Ctrl K / ⌘K mở ở mọi màn, con trỏ vào ô gõ, Esc đóng, tìm không cần dấu.
- **Thanh bên:** phần giữa tự cuộn; Bản đồ, Trợ giúp, Thu gọn luôn thấy, kể cả màn thấp.
- **Đầu trang:**
  - tab đầu là trang tổng của màn;
  - tab khác lấy tên tab làm tiêu đề;
  - nút đầu trang thuộc về tab;
  - nút chính ở phải cùng trên máy tính, đứng đầu trên điện thoại; nút phụ vào menu "⋯";
  - mô tả dài gập sau 2 dòng.
- **Bảng thành thẻ (≤900px):** mọi ô có nhãn cột; tiêu đề và nhãn trạng thái ở dòng đầu, các ô còn lại xếp lưới 2
  cột (điện thoại) hoặc 4 cột (máy tính bảng). Bảng rộng hơn khung thì mờ mép phải để báo còn cột.
- **Dải cuộn ngang** (menu, tab, chip): mờ mép phía còn mục; mục đang mở tự cuộn vào giữa.
- **Ngăn chi tiết:** nút thao tác dính đáy ngăn.
- **Biểu mẫu:**
  - gom mọi lỗi, đánh dấu từng ô sai, ghi lỗi dưới ô, đưa con trỏ về ô sai đầu tiên;
  - gợi ý trong ô viết "Ví dụ: …";
  - lựa chọn xuống dòng, không giấu trong dải cuộn.
- **Thông báo:** thao tác đảo được thì thông báo kèm "Hoàn tác" trong 6 giây.

### 2.4 Màn hình mẫu đã vẽ

Tổng quan; trang tổng quan của 11 phân hệ; Đơn hàng (bảng, thao tác hàng loạt, ngăn chi tiết có thuế
chụp lúc đặt, dòng tiền, bút toán sổ cái); Bài viết (theo từng website, nút sở hữu, nhãn AI, trang bắt
buộc); Cây tổ chức (thành viên theo độ sâu quyền, cài đặt kế thừa); Tuân thủ (hồ sơ Bộ Công Thương theo
tên miền, KYC, dữ liệu cá nhân, văn bản có phiên bản, báo vi phạm); Nền tảng.

## 3. Mặt tiền bán hàng

Bảy màn hình mẫu cho một chuỗi có sàn: trang chủ, danh mục, sản phẩm, giỏ hàng và đặt hàng (bốn bước),
gian hàng người bán, bài viết có gắn sản phẩm, tài khoản người mua. Website chỉ có nội dung (không bán)
dùng lại trang chủ, bài viết và tài khoản.

Điều giao diện cần mà các bước 34–47 đã có trong đặc tả: danh sách và lọc sản phẩm công khai (41), mã
giảm giá công khai kèm điều kiện (43), sổ địa chỉ và báo giá vận chuyển (44), giỏ hàng, đặt hàng có
`Idempotency-Key`, đơn của tôi, đổi trả, đánh giá chỉ từ người đã mua (45), tách đơn theo người bán và
thanh toán một lần, trang công khai của người bán (46), đồng ý theo mục đích, chấp nhận văn bản có
phiên bản, xác thực số điện thoại, logo Bộ Công Thương (37), xuất và xoá dữ liệu cá nhân (đã có).

## 4. Phần core còn thiếu (D-200)

| Giao diện cần | Đi vào |
|---|---|
| Menu theo khách, quyền, gói (mục khoá hiện kèm tính năng cần) | bước 51 |
| Hộp việc chung và số đếm trên menu | bước 51 |
| Tìm xuyên phân hệ (Ctrl + K) | bước 51 |
| Số liệu tóm tắt cho trang tổng quan của phân hệ | bước 51 |
| Hoạt động gần đây đọc được (từ audit, lọc theo quyền đọc) | bước 51 (module `audit`) |
| Nhúng bản ghi của module khác vào nội dung (sản phẩm trong bài) — `content` vẫn không biết sản phẩm | bước 51; `catalog` đăng ký loại đối tượng ở bước 41 |
| Danh sách cửa hàng công khai: tên, địa chỉ, giờ mở cửa, có nhận tại quầy | bước 34 |
| Tình trạng hàng công khai theo cửa hàng (còn / sắp hết / hết), lọc "có sẵn ở cửa hàng" | bước 42 |
| Ước tính phí giao theo tỉnh ngay trên trang sản phẩm, chưa cần giỏ | bước 44 |
| Giá khi áp mã cho từng sản phẩm, để trang sản phẩm hiện "còn X với mã …" | bước 43 |

Hộp việc (việc nhân sự phải xử lý, tính từ dữ liệu của module) khác **hộp thư thông báo** của module
`notifications` (bước 37, `/me/inbox`, tin đã gửi cho một người). Hai thứ không gộp.

## 5. Tham khảo các sản phẩm hàng đầu (D-201)

Nghiên cứu ngày 13/11/2026: tài liệu chính thức của Shopify (Polaris, setup guide, index table), Stripe
(Dashboard, Workbench, MCP, Organizations), Linear (views, inbox, triage, agents), GitHub (command
palette, notifications, Primer), Vercel (điều hướng 2026), Atlassian, Odoo 18 (chatter, activities),
Salesforce SLDS, Google Access Approval; Baymard Institute (tìm kiếm, trang sản phẩm, giỏ, checkout,
cửa hàng, tài khoản, đổi trả, accessibility), web.dev (Core Web Vitals, passkey), Google Search Central
(structured data), IKEA và Thế Giới Di Động (tồn theo cửa hàng), Etsy (tiêu chí người bán tin cậy), Shopee
(trả hàng). Số liệu trích từ trang đã mở được; trang trả phí hay không mở được thì không dùng làm căn cứ.

### Đưa vào thiết kế quản trị

| Điều | Học từ | Ở đâu |
|---|---|---|
| View đã lưu thay tab trạng thái cố định; lệch thì "Tuỳ chỉnh", lưu được; bộ lọc trên đường dẫn | Shopify, Linear, Zendesk, Stripe | mọi bảng (mẫu: Đơn hàng) |
| Chọn trang này khác "chọn cả N kết quả khớp bộ lọc"; chạy bằng job, kết quả từng bản ghi | Shopify | thanh thao tác hàng loạt |
| Ctrl + K có chế độ lệnh `>`, toán tử `is:` `status:`, chọn phạm vi, dán mã để mở thẳng | GitHub, Stripe | bảng lệnh |
| Hộp việc nhóm Quá hạn / Hôm nay / Sắp tới, "vì sao bạn thấy", hoãn, giao | Odoo, Linear, GitHub | Tổng quan, "Cần xử lý" |
| Danh sách việc thiết lập theo mô hình, bước pháp lý chặn bán được đánh dấu | Shopify setup guide | Tổng quan |
| Agent là người dùng thật: key riêng, audit ghi loại người thực hiện, lệnh rủi ro chờ người duyệt | Stripe MCP, Linear agents | Đơn hàng (đề xuất hoàn tiền), hộp việc |
| Thiếu quyền hay thiếu gói thì nói vì sao, ai cấp được, gói nào mở | Primer | mục khoá, nút bị vô hiệu |
| Dòng hoạt động và ghi chú nội bộ có @ trên mọi bản ghi | Odoo chatter, SLDS | ngăn chi tiết |
| Thông báo kèm Hoàn tác cho thao tác đảo được | Primer | toàn khung |
| Trang tổng quan phân hệ gọn: bỏ danh sách màn hình (trùng menu), ghi chú thu gọn | nhận định từ nghiên cứu | 11 phân hệ |

### Đưa vào thiết kế mặt tiền

| Điều | Bằng chứng | Ở đâu |
|---|---|---|
| Gợi ý tìm: từ khoá, "trong danh mục", sản phẩm có giá, bài viết, cửa hàng; không dấu vẫn ra, đúng dấu xếp trước | Baymard: 56% site không đáp ứng đủ; 34% người dùng tìm nội dung không phải sản phẩm | đầu trang |
| Phí và ngưỡng miễn phí giao cạnh nút mua, thanh tiến độ ở giỏ | Baymard: 40% bỏ giỏ vì phí phát sinh | sản phẩm, giỏ |
| Ngày giao là ngày cụ thể kèm giờ chốt; nhận tại cửa hàng ghi giờ sẵn sàng | Baymard: 41% site không hiện ngày giao | sản phẩm, đặt hàng |
| Mua không cần tài khoản là lựa chọn chính; mời tạo tài khoản và passkey ở màn xong | Baymard: 18% bỏ giỏ khi bị buộc tạo tài khoản; adidas: 47% tạo passkey | đặt hàng |
| Bộ chọn "Mã của bạn" kèm lý do chưa dùng được, ô nhập mã sau một liên kết | Baymard | giỏ |
| Màn QR: mở app ngân hàng, lưu ảnh QR, sao chép số tài khoản và nội dung, trạng thái tiền về | thói quen thanh toán ở Việt Nam | xong |
| Đánh giá: bộ sưu tập ảnh, lọc có ảnh, tóm tắt AI có nhãn chỉ từ người đã mua | Baymard; Amazon | sản phẩm |
| Báo khi có hàng ở một cửa hàng; lưu sản phẩm không cần đăng nhập | IKEA; Baymard: 89% site không cho lưu khi chưa có tài khoản | sản phẩm |
| Sắp xếp "Đánh giá cao", lọc theo số sao, thẻ ghi số biến thể | Baymard: 69% thiếu bốn kiểu sắp xếp tối thiểu | danh mục |
| Huy hiệu người bán tin cậy tách khỏi huy hiệu xác thực, có tiêu chí công khai | Etsy Star Seller | gian hàng |

### Không làm

- **Đối chiếu địa chỉ cũ (ba cấp) sang mới:** trái D-193, do chủ dự án quyết; giữ chỉ danh mục sau sáp
  nhập. Người gõ tên cũ vẫn tìm được phường, xã theo tên mới nhờ tìm không dấu.
- **Đổi hàng tab ngang của menu con sang thanh bên** như Vercel 2026: giữ hàng tab (mục 2.3); xem lại khi
  có số đo trên giao diện thật.

## 6. Trợ giúp trong giao diện (13/11/2026)

Chủ dự án yêu cầu học cách các sản phẩm lớn đặt hướng dẫn ngay cạnh tính năng. Nguồn: Carbon (IBM) tách
"tooltip" khỏi "toggletip"; Shopify Polaris chỉ dùng tooltip cho thông tin phụ, không giấu thông tin bắt
buộc; Atlassian có bảng trợ giúp theo trang mở từ nút "?"; nghiên cứu của Nielsen Norman Group cho thấy
hướng dẫn dạo đầu (product tour) thường bị bỏ qua, trợ giúp đúng lúc người dùng cần thì hiệu quả hơn.

| Mẫu | Dùng khi | Thiết kế 2 |
|---|---|---|
| Nút ⓘ (toggletip): bấm hoặc chạm mới mở, có tiêu đề, giải thích, ví dụ, "Tìm hiểu thêm" | Thuật ngữ, cách tính số liệu, quy định pháp luật, lựa chọn có hệ quả | Cạnh tiêu đề mục; trên điện thoại thành tấm trượt từ đáy có nút "Đã hiểu" |
| Chú thích khi rê chuột (tooltip) | Chỉ cho nút chỉ có biểu tượng; chỉ trên máy có chuột | Nút sáng / tối, thông báo, trợ giúp |
| Bảng "Trợ giúp" theo trang | Bài hướng dẫn của đúng trang, phím tắt, hỏi trợ lý AI, có gì mới, liên hệ | Nút "?" ở đầu mọi trang quản trị |
| Thẻ giới thiệu tính năng mới | Một tính năng vừa ra, tắt được, không chặn thao tác | Đơn hàng: "Lưu view" |
| "Cần giúp?" ở trang đặt hàng | Người mua kẹt giữa chừng: gọi, nhắn tin, tra cứu đơn, chính sách | Mặt tiền, chỉ ở trang đặt hàng |

Luật viết nội dung trợ giúp:

- Không giấu thông tin bắt buộc, lỗi hay cảnh báo trong ⓘ — những thứ đó hiện thẳng trên trang.
- Mỗi hộp tối đa ba câu và một ví dụ; dài hơn là dấu hiệu giao diện cần sửa, không phải cần thêm chữ.
- Nêu căn cứ pháp luật khi có, kèm "cần đối chiếu văn bản gốc" nếu nguồn chưa thống nhất.
- Định nghĩa số liệu lấy từ máy chủ (cùng nơi tính số), để chữ giải thích không lệch với con số.
- Không làm hướng dẫn dạo đầu bắt buộc; gợi ý đã tắt thì không hiện lại (lưu theo người dùng).

Phần máy chủ cần thêm: đề bài cho teammate ở `docs/prompts/ui-help-api.md`.

## 7. Bố cục cân đối (14/11/2026)

- Nhóm có số thẻ cố định thì số cột phải chia hết số thẻ ở mọi khổ (danh mục 6 ô, trang danh mục 12 sản
  phẩm), hoặc xếp kiểu giãn đều để thẻ cuối lấp chỗ trống. Không để một thẻ lẻ ở hàng cuối.
- Nhãn, nút không được lấn ra lề khung chứa; chỗ thiếu bề rộng thì đổi bố cục (vd người bán thành hàng
  ngang trên điện thoại) chứ không bóp chữ.
- Đầu trang website: ô tìm chiếm phần giữa; khổ 721–1100 giữ một hàng. Trang thanh toán khép kín ở mọi
  khổ: không ô tìm, không danh mục, không dòng khuyến mãi.
- Trang bài viết dùng một khung biên tập chung 1144px (cột chữ 720 + khoảng cách 64 + cột bên 360) cho
  tiêu đề, ảnh bìa, nội dung và "đọc tiếp": mọi khối thẳng một mép trái và một mép phải, không dư khoảng
  trống. Thân bài hẹp hơn thanh menu là cố ý.
- Các khối lớn trong một trang dùng cùng bề rộng; khối hẹp hơn (biểu mẫu) là ngoại lệ có chủ đích.
- Bộ kiểm tra tự động đo thêm: lấn lề, thẻ lẻ, dải cuộn ngang mà kéo dọc được; quét ở 360, 390, 430, 768,
  820, 1024, 1280, 1440px.

## 8. Mặt tiền: điều hướng, trang sản phẩm, đặt hàng (14/11/2026)

- **Thanh danh mục:**
  - nút "Danh mục" đầu thanh;
  - danh mục sản phẩm có biểu tượng;
  - lối vào đặc biệt (Khuyến mại, sàn, Góc bếp) dồn phải, có màu riêng.
  
  Máy có chuột, từ 1024px: rê chuột hoặc dùng bàn phím mở bảng xổ (nhóm con, bán chạy, gợi ý), làm bằng CSS
  thuần. Màn hẹp hơn: dải chip cuộn ngang. Trong sản phẩm thật, menu lấy từ backend (cách làm chờ chủ dự án
  chọn).
- **Hàng tab trên điện thoại:** thành lưới hiện đủ mọi mục (tên ngắn + dòng phụ), không giấu mục sau thanh
  cuộn ngang.
- **Đánh giá:**
  - lọc theo sao, có ảnh, cỡ, chủ đề;
  - sắp xếp;
  - nút hữu ích và báo vi phạm;
  - tên người mua rút gọn.
- **Hỏi đáp:** ô tìm cạnh nút "Đặt câu hỏi" mở tại chỗ; câu trả lời ghi vai trò ("Cửa hàng", "Đã mua").
- **Số tiền không bao giờ bị bẻ dòng:**
  - giữa số và "₫" dùng khoảng trắng không ngắt (NBSP);
  - ô chứa số và giá đặt `white-space: nowrap`;
  - thiếu chỗ thì cả con số xuống dòng và căn phải.

  Kiểm cả với phông hiển thị rộng hơn (iPhone), vì công cụ quét chạy Chromium.
- **Đặt hàng trên điện thoại:** tóm tắt đơn thu gọn thành một dải ở đầu trang; thanh tổng tiền và nút tiếp
  tục cố định ở đáy màn hình.
- **Trang chiến dịch có phong cách riêng:**
  - dùng chung đầu trang và chân trang của site;
  - chỉ phần thân đổi phong cách, gom CSS dưới một lớp gốc riêng;
  - thông tin pháp lý ở chân trang giữ nguyên.
