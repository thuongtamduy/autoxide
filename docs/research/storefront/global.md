# R2 — Chuẩn quốc tế cho mặt tiền bán hàng (storefront) — áp cho Bếp Mộc

Ngày nghiên cứu: 14/11/2026. Người làm: agent nghiên cứu (R2).
Phạm vi: 8 khu vực storefront. Các điều **đã nghiên cứu ngày 13/11** không làm lại; ở đâu có số
liệu mới thì ghi "bổ sung cho 13/11".

## Cách đọc

- Mỗi điều có ba phần: **Khuyến nghị** (làm gì), **Bằng chứng** (số liệu, năm, URL), **Ví dụ**
  (trang nào làm tốt, chỉ ghi điều đã thấy tận mắt qua WebFetch).
- "x% site làm sai" là tỉ lệ trong benchmark của Baymard (khoảng 150–340 site bán lẻ lớn nhất
  Mỹ/châu Âu). Năm ghi là **năm của benchmark hoặc năm bài viết**, không phải năm đọc.
- Ghi chú **[thứ cấp]** = số liệu chỉ thấy qua trang khác trích lại, chưa đọc được trang gốc.
- Nguồn bị chặn hoặc trả phí, đã bỏ qua, không đoán: trang sản phẩm Williams Sonoma và Crate &
  Barrel (HTTP 403), Muji US (404), trang mua iPhone của Apple (WebFetch chỉ đọc được footer),
  Amazon (không thử, thường chặn bot), phần Premium của Baymard (chỉ đọc được tiêu đề guideline).

## Bức tranh chung (số liệu mới nhất đọc được)

| Khu vực (Baymard)                | Desktop "mediocre" trở xuống | Mobile | Năm benchmark | Nguồn |
| -------------------------------- | ---------------------------- | ------ | ------------- | ----- |
| Trang chủ + điều hướng danh mục  | 61%                          | 65%    | 2026          | https://baymard.com/blog/ecommerce-navigation-best-practice |
| Danh sách sản phẩm + bộ lọc      | 58%                          | 78%    | 2025          | https://baymard.com/blog/current-state-product-list-and-filtering |
| Trang sản phẩm                   | 52%                          | 62%    | 2025–2026     | https://baymard.com/blog/current-state-ecommerce-product-page-ux |
| Checkout                         | 59%                          | 59%    | 2026          | https://baymard.com/research-articles/current-state-of-checkout-ux |

- Tỉ lệ bỏ giỏ trung bình **70,22%** (trung bình 50 nghiên cứu 2006–2025, trang cập nhật 28/10/2026).
  Lý do (trừ "chỉ xem"): chi phí phụ cao 40%, giao chậm 20%, không tin site giữ thẻ 19%, bắt tạo tài
  khoản 18%, checkout dài/rối 17%, site lỗi 17%, chính sách đổi trả không ổn 13%, không thấy tổng
  tiền trước 12%. Sửa checkout có thể tăng chuyển đổi tới **35,26%**.
  https://baymard.com/blog/cart-abandonment-rate
- Mobile luôn kém desktop → Bếp Mộc nên thiết kế mobile trước.

---

## 1. Trang chủ

### 1.1 Khối nào có hiệu quả

**Khuyến nghị.** Trang chủ là "bản đồ" của danh mục: người mới phải đoán được shop bán những gì
trong vài giây. Thứ tự gợi ý: thanh tìm kiếm rõ → lối vào danh mục có ảnh → một khối ưu đãi tĩnh →
bán chạy → nội dung (hướng dẫn/công thức) → cam kết (giao, đổi trả, bảo hành) → đăng ký nhận tin.
Không để quảng cáo, pop-up hay hộp đăng ký che trang.

**Bằng chứng.**
- Trưng ít nhất **40% loại sản phẩm** trên trang chủ (Baymard guideline #237; 6% site trưng quá
  hẹp — review 2020, 60 site). https://baymard.com/learn/ecommerce-ux-best-practices ,
  https://baymard.com/learn/ux-statistics
- **59%** site dùng quảng cáo, banner pop-up hay hộp đăng ký gây xao nhãng (2020); **95%** site
  mobile đặt quảng cáo ở vùng chính của trang chủ (nghiên cứu mobile 2 năm). Cùng nguồn trên.
- **22%** site không đặt ô tìm kiếm nổi bật trên trang chủ (2020). Cùng nguồn.
- Ảnh trong cảnh dùng ("inspirational imagery") mà không link tới sản phẩm trong ảnh: **64%** site
  sai (2026). https://baymard.com/blog/ecommerce-navigation-best-practice

**Ví dụ.** Le Creuset: thanh trên cùng ghi rõ ưu đãi + ngưỡng miễn phí giao (≥99 USD); trang
sản phẩm có ô "Recipes" (đọc 14/11/2026). Our Place: trang chủ có khối cam kết ngắn "Free shipping
$135+ · 100 Day Trial · Free Returns" và ô danh mục có ảnh (đọc 14/11/2026).

### 1.2 Carousel tự chạy

**Khuyến nghị.** **Không dùng carousel tự chạy**, nhất là trên mobile. Thay bằng các khối tĩnh
xếp dọc (người dùng cuộn là thấy hết). Nếu buộc phải có carousel trên desktop: slide đầu là nội dung
hợp nhiều người nhất; tự chạy 5–7 giây/slide (tới ~10 giây nếu nhiều chữ); dừng khi rê chuột;
dừng hẳn sau khi người dùng bấm; nút điều khiển to, tương phản cao; chữ là HTML, không in vào ảnh;
không để carousel là lối duy nhất tới một nội dung.

**Bằng chứng.**
- Baymard "Homepage carousel" (2019, cập nhật 03/04/2025): ~33% site desktop có carousel; **46%**
  trong số đó có lỗi; mobile **không nên tự chạy** vì không có hover để dừng, người dùng hay bấm đúng
  lúc slide đổi và vào nhầm trang; khối tĩnh "tốt bằng hoặc hơn, dễ làm hơn".
  https://baymard.com/blog/homepage-carousel
- Benchmark 2026: **31%** site có carousel làm sai. https://baymard.com/blog/ecommerce-navigation-best-practice
- **52%** site/app mobile có carousel tự chạy trên trang chủ (guideline #957, không ghi năm).
  https://baymard.com/learn/ecommerce-ux-best-practices
- WCAG 2.2 SC 2.2.2 Pause, Stop, Hide (mức A): nội dung tự chuyển động > 5 giây phải có cách dừng.
  https://www.w3.org/WAI/WCAG22/Understanding/pause-stop-hide.html
- Hiệu năng: ảnh slide ẩn lúc đầu nên `fetchpriority="low"`, không tranh băng thông với ảnh LCP
  (web.dev, cập nhật 31/03/2025). https://web.dev/articles/optimize-lcp

**Ví dụ.** Our Place vẫn dùng hero carousel tự chạy (14/11/2026) — **ví dụ không nên theo**.
Hướng ngược lại (khối tĩnh) là khuyến nghị chính thức của Baymard.

### 1.3 Lối vào danh mục

**Khuyến nghị.** Một hàng/lưới ô danh mục có **ảnh thumbnail rõ** + nhãn ngắn ngay dưới hero.
Trang danh mục trung gian (vd "Nồi & chảo") đặt các danh mục con **lên đầu**, không để khuyến mại
chen lên trên. Chia nhóm khi vượt ~10 lựa chọn; tầng sâu nhất nên có ≥ 10 sản phẩm.

**Bằng chứng (Baymard 2026).** Thumbnail danh mục con thiếu hoặc khó hiểu: **39%**; danh mục con
không phải nội dung chính ở trang trung gian: **38%**; không chia nhóm hợp lý: **51%**; người dùng
thấy quá tải từ khoảng 10 danh mục con. **13%** site không có trang danh mục trung gian (2025).
https://baymard.com/blog/ecommerce-navigation-best-practice

**Ví dụ.** IKEA trang "Products": mỗi khối danh mục có ảnh, tiêu đề, link "Shop all" đứng đầu rồi
mới tới danh mục con (đọc 14/11/2026).

### 1.4 "Đang khuyến mại" đặt ở đâu

**Khuyến nghị.** Ba chỗ cùng lúc: (1) một mục **"Ưu đãi"** ở thanh điều hướng chính; (2) một khối
tĩnh trên trang chủ dẫn tới danh sách đã lọc sẵn; (3) bộ lọc "Đang giảm giá" trong mọi danh sách.
Làm "Ưu đãi" là **danh mục dựa trên bộ lọc** (mở ra danh sách đã bật filter giảm giá), không phải
danh mục thật — để người dùng gỡ filter là thấy toàn bộ, có link "Xem tất cả".

**Bằng chứng.** Baymard 07/12/2022: 16% site không có danh mục Sale/Deals dạng filter, 16% có nhưng
làm sai; có một nhóm người dùng đi săn hàng giảm giá ngay khi vào site.
https://baymard.com/blog/sales-filter-based-category . Guideline #1191 "có filter Sale/Deal dễ thấy".
https://baymard.com/learn/ecommerce-ux-best-practices

**Ví dụ.** IKEA có mục "Offers & deals" ngay ở menu chính (14/11/2026). Le Creuset có "Specials" ở
menu phụ và nhãn "Sale" ngay trên swatch màu đang giảm (14/11/2026).

---

## 2. Điều hướng

### 2.1 Mega menu

**Khuyến nghị.**
- **Cột:** không có con số chuẩn. Chia theo nhóm có nghĩa (vd theo loại: nồi, chảo, dao; theo chất
  liệu; theo nhu cầu), nhóm quan trọng nhất ở góc trên-trái, mỗi mục chỉ xuất hiện một lần. Thực tế
  3–4 cột chữ + tối đa 1 ô ảnh là đủ cho một shop đồ bếp cỡ vừa.
- **Ảnh:** dùng khi ảnh giúp chọn (vd ảnh nhóm sản phẩm hay một ô "Ưu đãi"); icon là tuỳ chọn.
- **Tiêu đề nhóm phải bấm được** (dẫn tới trang danh mục cha).
- **Hover hay click:** desktop mở bằng hover **có độ trễ 300–500 ms**, giữ mở thêm ~0,5 giây khi
  chuột rời đi, xử lý đường chéo (menu-aim); đồng thời mở được bằng click/Enter cho bàn phím và
  cảm ứng. Mỗi mục đầu bảng phải là một trang thật liệt kê đủ mục con (để không phụ thuộc menu).
- Ô tìm kiếm luôn hiện, không giấu trong menu.

**Bằng chứng.**
- Baymard 2026: **62%** site không có hover delay; **31%** tiêu đề nhóm không bấm được.
  https://baymard.com/blog/ecommerce-navigation-best-practice
- Baymard 31/01/2023: 74% site desktop dùng menu mở bằng hover; delay 300–500 ms bỏ được phần lớn
  lần mở nhầm; 43% menu bị "nhấp nháy" [thứ cấp cho con số 43%].
  https://baymard.com/blog/dropdown-menu-flickering-issue
- NN/g "Mega Menus Work Well" (26/03/2017): chờ chuột đứng yên 0,5 giây rồi mới mở, hiện trong
  0,1 giây; đóng sau khi chuột rời 0,5 giây; ảnh dùng khi giúp chọn; viền/bóng rõ cho người thị lực
  kém. https://www.nngroup.com/articles/mega-menus-work-well/
- NN/g "Timing Exposing Content" (11/01/2015): phản hồi hover trong 0,1 giây, chờ 0,3–0,5 giây xác
  nhận ý định. https://www.nngroup.com/articles/timing-exposing-content/

**Ví dụ.** Our Place: mỗi menu có nhóm "Featured/Best Sellers" + nhóm theo chất liệu + nhóm theo
loại + "Shop All" (14/11/2026). Le Creuset: mega menu 5 mục chính + menu phụ có "Recipes",
"Materials", "Colors" (14/11/2026).

### 2.2 Điện thoại: hamburger hay bottom nav

**Khuyến nghị.** Dùng **dạng kết hợp**: thanh trên cố định có logo, ô tìm kiếm luôn hiện, giỏ;
menu danh mục mở bằng nút có **chữ "Danh mục"** (không chỉ icon ba gạch). Cấp đầu của menu mobile
phải là **danh mục sản phẩm** (không phải "Về chúng tôi", "Blog"). Bottom nav (Trang chủ · Danh
mục · Tìm · Giỏ · Tài khoản) hợp với app hoặc PWA; trên web mobile là tuỳ chọn — nếu dùng thì tối đa
5 mục, có nhãn chữ, và phải chừa chỗ cho thanh mua dính ở trang sản phẩm.

**Bằng chứng.**
- NN/g (2016, 179 người, 6 site): menu ẩn bị dùng ít hơn và chậm hơn; menu ẩn có độ nổi thấp tệ
  nhất (44% dùng, lần đầu sau 33 giây) so với dạng kết hợp (89%, 24 giây). Khuyến nghị: hiện thẳng
  nếu ≤ 4–5 mục, nhiều hơn thì dùng dạng kết hợp; nếu buộc phải ẩn thì nút phải nổi, có nhãn.
  https://www.nngroup.com/articles/find-navigation-mobile-even-hamburger/ ,
  https://www.nngroup.com/articles/hamburger-menus/
- Baymard 2026: **22%** site không đặt danh mục sản phẩm ở cấp đầu menu mobile.
  https://baymard.com/blog/ecommerce-navigation-best-practice
- Bottom nav: chưa tìm được nghiên cứu độc lập (NN/g/Baymard) chứng minh bottom nav hơn hamburger
  trên **web** mobile; các nguồn ủng hộ đều là blog/vendor → coi là tuỳ chọn, không phải chuẩn.

### 2.3 Breadcrumb

**Khuyến nghị.** Breadcrumb theo **cây danh mục** (không theo lịch sử duyệt), bắt đầu bằng "Trang
chủ", mục cuối là trang hiện tại không phải link. Trang sản phẩm thêm link **"← Quay lại kết quả"**
giữ nguyên bộ lọc/sắp xếp. Mobile: một dòng, không xuống dòng — chỉ hiện 1–2 cấp cuối; link đủ to
để chạm. Đánh dấu danh mục đang đứng ở menu chính.

**Bằng chứng.**
- NN/g (2018, rà lại 01/09/2026): 11 guideline, gồm không xuống dòng trên mobile, chỉ hiện 1–2 cấp
  cuối trên mobile. https://www.nngroup.com/articles/breadcrumbs/
- Baymard 2026: **96%** site không đánh dấu vị trí hiện tại ở menu chính; trên mobile breadcrumb
  thường thiếu hoặc bị cắt. https://baymard.com/blog/ecommerce-navigation-best-practice
- Baymard (benchmark 2013, 50 site): 68% làm sai breadcrumb — cần cả hai loại (cây + "quay lại kết
  quả") [thứ cấp; số liệu cũ]. https://baymard.com/blog/ecommerce-breadcrumbs

**Ví dụ.** IKEA: Products > Kitchenware & tableware > Cookware > Pots > tên sản phẩm. Le Creuset:
Home > Cookware > Dutch Ovens (14/11/2026).

---

## 3. Danh sách sản phẩm

### 3.1 Thông tin trên thẻ

**Khuyến nghị.** Mỗi thẻ: ảnh (tỉ lệ cố định), tên, giá (giá gốc gạch nếu giảm), **điểm trung bình
+ số lượt đánh giá**, swatch màu/chất liệu, nhãn ngắn (Mới / Bán chạy / Giảm x%), tình trạng hàng
nếu sắp hết. Với hàng bán theo dung tích/bộ: thêm **giá theo đơn vị** (vd "/lít", "/món"). Gộp các
biến thể của một sản phẩm thành **một thẻ** (không lặp 5 thẻ cho 5 màu). Toàn bộ thẻ là một vùng bấm
tới trang sản phẩm; hiệu ứng hover đồng bộ. Hiện tổng số sản phẩm ở đầu và cuối danh sách.

**Bằng chứng (Baymard 2025, 170+ site).** **42%** không gộp biến thể; **80%** không có ≥ 3 ảnh
cho mỗi thẻ; **67%** không hiện giá theo đơn vị.
https://baymard.com/blog/current-state-product-list-and-filtering . Hiện cả điểm trung bình **và**
số lượt (guideline #441); 70% người được hỏi chọn sản phẩm có nhiều lượt đánh giá hơn dù điểm thấp
hơn. https://baymard.com/blog/user-ratings-distribution-summary . 76% site (2016) không cho biết rõ
chỗ nào bấm được. https://baymard.com/blog/list-items-hover-and-hit-area

### 3.2 Ảnh thứ hai khi rê chuột

**Khuyến nghị.** Có, trên desktop: rê vào thì hiện ảnh thứ hai (nên là **ảnh trong bối cảnh dùng**
hoặc ảnh thể hiện kích thước), có **độ trễ nhỏ** để không nhấp nháy khi lướt chuột. Tốt hơn nữa:
cho xem ≥ 3 ảnh ngay ở thẻ (chấm/vuốt). Mobile không có hover → vuốt ngang trong ảnh thẻ hoặc chỉ
một ảnh.

**Bằng chứng.** NN/g (27/02/2022): ảnh phụ khi hover hữu ích, cần delay để không đổi ảnh khi chỉ
lướt qua. https://www.nngroup.com/articles/product-photos-listing-pages/ . Baymard 2025: 80% site
không cung cấp ≥ 3 ảnh trong thẻ (link 3.1).

### 3.3 Swatch màu

**Khuyến nghị.** Hiện swatch (ô màu/ô chất liệu) dưới ảnh; bấm/rê swatch thì đổi ảnh thẻ sang màu
đó. Mobile: **hiện đủ swatch** bằng hàng cuộn ngang có mũi tên rõ, swatch cuối bị cắt để gợi ý còn
nữa; vùng chạm lớn, cách nhau rộng; tránh chỉ ghi "+5" nhỏ.

**Bằng chứng.** Baymard 08/08/2023: **57%** site không cho xem đủ màu ở thẻ trên mobile; người dùng
bỏ sót chữ "+N" khi cuộn nhanh. https://baymard.com/blog/mobile-interactive-color-swatches

**Ví dụ.** Le Creuset có ~28 màu cho một nồi gang — loại catalog cần đúng cách hiện swatch này.

### 3.4 "Thêm nhanh" (quick add / quick view)

**Khuyến nghị.** **Không dùng Quick View** (lớp phủ xem nhanh) trên desktop. "Thêm vào giỏ" ngay
trên thẻ chỉ cho sản phẩm **không có biến thể** (vd khăn lau, muỗng); sản phẩm có cỡ/màu thì nút
dẫn tới trang sản phẩm. Nút thêm nhanh đặt **ngoài vùng ảnh**, luôn hiện (không chỉ khi hover), và
sau khi thêm phải có phản hồi rõ (toast/mini giỏ). Nếu sau này làm quick view trên mobile: danh sách
phải còn thấy mờ phía sau, nút Back đóng lớp phủ.

**Bằng chứng.** Baymard: Quick View trên desktop "thường hại nhiều hơn lợi", hay bị nhầm là trang
sản phẩm, nút đặt giữa ảnh gây bấm nhầm [phần lớn là trích đoạn; bài đầy đủ thuộc Premium].
https://baymard.com/blog/mobile-desktop-quick-views . Baymard 01/08/2023: quick view thường là để
bù cho thẻ thiếu thông tin — sửa thẻ trước. https://baymard.com/blog/ecommerce-quick-views

### 3.5 Bộ lọc trên điện thoại

**Khuyến nghị.**
- Nút **"Lọc & sắp xếp"** dính khi cuộn, ghi số filter đang bật (vd "Lọc (2)").
- Bấm vào mở **sheet toàn màn hình hoặc bottom sheet**; các nhóm filter dạng accordion, mỗi lựa chọn
  có **checkbox** và **số kết quả** bên cạnh; nhóm dài (thương hiệu) có ô tìm trong nhóm.
- **Bấm áp (batch), không áp ngay** trên mobile; nút áp dính đáy sheet, ghi **"Xem 24 sản phẩm"**
  và số đó cập nhật theo từng lựa chọn (gọi API đếm, không tải lại danh sách). Có "Xoá tất cả".
- Sau khi áp: hàng **chip filter đang bật** cuộn ngang ở đầu danh sách, mỗi chip có nút ×.
- Cho chọn nhiều giá trị trong cùng một nhóm. Desktop: sidebar, áp ngay.
- Bộ lọc tối thiểu cho đồ bếp: giá, điểm đánh giá, màu, kích cỡ/dung tích, thương hiệu, **chất
  liệu**, **loại bếp dùng được (từ, gas, hồng ngoại, lò nướng)**, rửa máy được, đang giảm giá.

**Bằng chứng.**
- Baymard "Ecommerce filter UI" (17/04/2026): sheet riêng; nút Filter dính; mobile **dùng nút áp có
  chủ đích**, desktop áp ngay; nút "Show X Results" cập nhật số khi chọn; chip filter đang bật; vùng
  chạm ≥ 44×44 pt; 40% site mobile có nhãn filter khó hiểu. https://baymard.com/blog/ecommerce-filter-ui
- Baymard 2025: **14%** không cho chọn nhiều giá trị cùng nhóm; **51%** thiếu 5 loại filter thiết
  yếu (giá, điểm, màu, cỡ, thương hiệu); **20%** không có tóm tắt filter đang bật.
  https://baymard.com/blog/current-state-product-list-and-filtering
- NN/g (07/02/2016): mobile nên lọc theo lô với nút Apply; nhược điểm là khó hiện số liên tục —
  khắc phục bằng số trên nút. https://www.nngroup.com/articles/applying-filters/
- 4 kiểu sắp xếp tối thiểu: đã nghiên cứu 13/11. **Bổ sung:** Baymard 2025 — **69%** site vẫn
  thiếu đủ 4 kiểu (giá, điểm đánh giá, bán chạy, mới nhất).

### 3.6 Phân trang, "Xem thêm" hay cuộn vô hạn

**Khuyến nghị.** Nút **"Xem thêm"** (có thể tự tải thêm 1 lượt khi cuộn gần cuối, rồi hiện nút),
kèm dòng **"Đang xem 24 / 133 sản phẩm"**. Mỗi lần tải thêm cập nhật URL (cursor) để **Back từ trang
sản phẩm quay về đúng vị trí**. Hết hàng thì bỏ nút, hiện tổng. Không dùng cuộn vô hạn thuần (footer
có chính sách đổi trả sẽ không với tới được). Hợp với `fetch_page` + `cursor` sẵn có của Autoxide.

**Bằng chứng.** Baymard guideline #501 "Load More thay cho phân trang hoặc cuộn vô hạn".
https://baymard.com/learn/ecommerce-ux-best-practices . NN/g (06/03/2022): Show More hợp catalog
nhỏ-vừa, giữ footer với tới được, hợp mobile; hiện "Viewing 40 of 333"; hỗ trợ quay về đúng chỗ khi
Back. https://www.nngroup.com/articles/alternatives-pagination-listing-pages/ . web.dev (CLS,
cập nhật 07/02/2025): nội dung mới do người dùng bấm "Load more" không bị tính là layout shift.
https://web.dev/articles/optimize-cls

---

## 4. Trang sản phẩm

### 4.1 Bộ ảnh (zoom, video, ảnh trong bối cảnh)

**Khuyến nghị.** Tối thiểu **5 ảnh** cho mỗi sản phẩm bán chạy, theo thứ tự: ảnh nền trắng → ảnh
**trong bếp, đang nấu** → ảnh **thể hiện kích thước** (trong tay, cạnh vật quen thuộc, đặt trên bếp
gia đình) → cận chất liệu/đáy nồi/tay cầm → ảnh **tất cả món trong hộp**. Có zoom (độ phân giải đủ
lớn), mobile vuốt + pinch. Video ngắn cho sản phẩm đắt hoặc khó hiểu (vd chảo thép carbon cần tôi
dầu). Ảnh do người mua gửi trong review cho lướt qua lại được.

**Bằng chứng.**
- Baymard: 3–5 ảnh/sản phẩm (#740), video cho hàng bán chạy/phức tạp/đắt (#759).
  https://baymard.com/learn/ecommerce-ux-best-practices
- Baymard 2025–2026: **37%** site không có ảnh "in scale"; 42% người dùng đoán kích thước qua ảnh;
  63% không cho lướt ảnh review. https://baymard.com/blog/current-state-ecommerce-product-page-ux
- Baymard (2017): 56% người thử xem ảnh trước khi đọc chữ; ví dụ đúng bối cảnh: "máy trộn trong
  bếp". https://baymard.com/blog/in-scale-product-images
- **44%** site không chụp phụ kiện đi kèm cạnh sản phẩm; 25% thiếu độ phân giải/zoom (2020).
  https://baymard.com/learn/ux-statistics ,
  https://baymard.com/research-articles/ensure-sufficient-image-resolution-and-zoom
- NN/g (24/11/2019, 49 site): nhiều góc, ảnh đang dùng, zoom; video là "nice-to-have".
  https://www.nngroup.com/articles/ecommerce-product-pages/

**Ví dụ.** Le Creuset: ~35 ảnh cho một nồi, gồm ảnh bối cảnh, ảnh màu và **infographic dung tích**.
IKEA: 8 ảnh, có zoom, có link "All media" và **"Skip images"** cho người dùng bàn phím (14/11/2026).

### 4.2 Chọn biến thể

**Khuyến nghị.** Màu = swatch có tên màu; cỡ = **nút bấm** (không dropdown), mỗi nút ghi kích thước
**kèm ý nghĩa dùng** (vd "24 cm · 3,8 L · 3–4 người"). Nút hết hàng vẫn hiện nhưng mờ + ghi "Hết
hàng" (bấm vào → "Báo khi có hàng", đã nghiên cứu 13/11). Đổi biến thể thì đổi ảnh, giá, tình trạng,
URL. Mỗi biến thể có cùng bộ thông số để so được.

**Bằng chứng.** Baymard 2025–2026: **57%** site không dùng nút cho chọn cỡ; **81%** không hiện giá
theo đơn vị. https://baymard.com/blog/current-state-ecommerce-product-page-ux . NN/g 2019: giải
thích nghĩa từng lựa chọn, hiện tình trạng từng biến thể. https://www.nngroup.com/articles/ecommerce-product-pages/

**Ví dụ.** Le Creuset: 6 dung tích, mỗi cỡ kèm **số người ăn** (2 qt cho 1–2 người … 13,25 qt cho
9+) và link "Size Guide". IKEA: mỗi cỡ hiện **chênh lệch giá** (−£3 / +£10) (14/11/2026).

### 4.3 Mô tả theo lợi ích

**Khuyến nghị.** Mở đầu 1–2 câu "nồi này giúp bạn làm gì" (lợi ích), sau đó 3–6 **điểm nổi bật**
mỗi điểm một ảnh/icon + 1 câu (vd "Đáy 5 lớp — chín đều, không cháy giữa"), rồi phần "Tính năng
khác". Không nhồi chữ quảng cáo; giải thích thuật ngữ (từ tính, 5-ply, PFAS…).

**Bằng chứng.** Baymard "structure descriptions by highlights": cấu trúc theo điểm nổi bật làm
người dùng xem kỹ hơn; 78% site không làm [con số 78% thấy ở tiêu đề bản mirror].
https://baymard.com/blog/structure-descriptions-by-highlights . NN/g 2019: đủ mà không độn, thông
tin quan trọng lên đầu. https://www.nngroup.com/articles/ecommerce-product-pages/

**Ví dụ.** Made In: mô tả xoay quanh lợi ích (áp chảo, nhiệt đều, tay cầm không nóng) (14/11/2026).
IKEA: khối mô tả tính năng kèm ảnh (tay cầm, đáy, nắp, xếp chồng) (14/11/2026).

### 4.4 Thông số dạng bảng

**Khuyến nghị.** Bảng hai cột cố định cho cả ngành hàng: chất liệu, lớp đáy, đường kính, dung tích,
cao, dài kể cả tay cầm, trọng lượng, **bếp dùng được**, nhiệt độ lò tối đa, rửa máy được, xuất xứ,
bảo hành, kích thước + trọng lượng kiện. **Cùng nhãn, cùng đơn vị** cho mọi sản phẩm cùng loại
(lấy từ thuộc tính có cấu trúc của module `catalog`, không gõ tay vào mô tả).

**Bằng chứng.** Baymard: ~30% site có thông số không đồng nhất giữa sản phẩm cùng loại; 50% bảng
thông số khó quét [thứ cấp qua trang tóm tắt Baymard].
https://baymard.com/guidelines/839-making-specs-consistent-and-uniform ,
https://baymard.com/blog/spec-sheet-scannability . NN/g 2019: bảng cho thông số nhiều dữ liệu,
cùng loại thông số trong một ngành hàng.

**Ví dụ.** IKEA: khối "Weight and measurements" + "Package details" (kích thước kiện 24×15×31 cm,
1,30 kg). Le Creuset: bảng chất liệu, nguồn nhiệt, số món, cách rửa, xuất xứ, bảo hành (14/11/2026).
Made In: thông số ở dạng gạch đầu dòng và **chỉ có kích thước cho một cỡ** — ví dụ chưa tốt.

### 4.5 "Trong hộp có gì"

**Khuyến nghị.** Một dòng riêng "Trong hộp: 1 nồi, 1 nắp kính, 1 sách hướng dẫn" + **một ảnh chụp
đủ các món**. Nói rõ cái gì **không** kèm (vd "không kèm nắp").

**Bằng chứng.** Baymard: **44%** site không thể hiện phụ kiện đi kèm (nghiên cứu trang sản phẩm 60
site). https://baymard.com/learn/ux-statistics

**Ví dụ.** Le Creuset ghi "piece count: 2" (nồi + nắp). Made In không ghi, và có review 4 sao phàn
nàn vì tưởng có nắp (14/11/2026) — đúng loại thắc mắc mà dòng này tránh được.

### 4.6 Hướng dẫn chăm sóc (rất hợp đồ bếp)

**Khuyến nghị.** Mục riêng **"Dùng & chăm sóc"**: lần dùng đầu (tôi dầu/rửa), nhiệt độ khuyên dùng,
dụng cụ nên/không nên dùng, cách rửa, cách cất, dấu hiệu cần thay, bảo hành áp khi nào. Thêm PDF hướng
dẫn tải được và link sang bài hướng dẫn dài (khu vực 7).

**Bằng chứng.** NN/g 2019: mô tả phải nói "dùng thế nào"; trả lời trước câu hỏi người mua.
https://www.nngroup.com/articles/ecommerce-product-pages/ . (Chưa có số liệu Baymard công khai riêng
cho "care" — đây là khuyến nghị theo ngành hàng, dựa trên việc các site mẫu đều có mục này.)

**Ví dụ.** Le Creuset có mục "Care & Use" + "Great For" + FAQ, có link nhảy ngay dưới nút mua.
IKEA có "Materials and care" + tài liệu PDF. Made In có mục "Care" + FAQ (bếp từ, máy rửa bát)
(đều đọc 14/11/2026).

### 4.7 So sánh cỡ

**Khuyến nghị.** Bảng nhỏ so các cỡ ngay dưới bộ chọn cỡ (đường kính · dung tích · số người ·
trọng lượng · giá) + một ảnh xếp các cỡ cạnh nhau. Với bộ nồi: bảng "bộ gồm những món nào" so các
bộ.

**Bằng chứng.** 42% người dùng cố đoán kích thước qua ảnh (Baymard, link 4.1). NN/g 2019: dùng bảng
so sánh cho dữ liệu nhiều. Baymard 2023: tính năng so sánh giúp trên desktop, mobile ít lợi do phải
cuộn nhiều → trên mobile giữ bảng ngắn. https://baymard.com/blog/ecommerce-quick-views

**Ví dụ.** Le Creuset "Size Guide" + số người ăn. Made In mô tả trang danh mục "Compare sizes from 8
to 12 inches" (14/11/2026).

### 4.8 Tab, accordion hay trang dài

**Khuyến nghị.** **Không dùng tab ngang.** Trang dài với các mục **accordion dọc**, tiêu đề rõ;
mục quan trọng nhất (Điểm nổi bật, Thông số) mở sẵn; có hàng **link nhảy** ngay dưới nút mua
("Thông số · Chăm sóc · Hỏi đáp · Đánh giá"). Hơn hết là cắt bớt nội dung phụ.

**Bằng chứng.** Baymard: **27%** người dùng bỏ sót nội dung trong tab ngang, so với **8%** với mục
gập dọc; tab ngang không hợp mobile (guideline #753 cập nhật 07/2026). https://baymard.com/blog/avoid-horizontal-tabs .
Guideline #751 "mục gập dọc có tiêu đề rõ". https://baymard.com/learn/ecommerce-ux-best-practices

**Ví dụ.** Le Creuset: jump links "Great For · Care & Use · FAQs · Recipes". IKEA: các mục "Show
more" dạng panel (14/11/2026).

### 4.9 Thanh mua dính

**Khuyến nghị.** Mobile: khi nút "Thêm vào giỏ" chính cuộn khỏi màn hình thì hiện **thanh mỏng dính
đáy**: giá + biến thể đang chọn + nút. Nếu chưa chọn cỡ, bấm nút thì cuộn lên bộ chọn, không thêm
mù. Thanh không được che phần tử đang focus (dùng `scroll-padding-bottom`), không che chat/cookie, và
không dùng chung với bottom nav cùng lúc (ẩn bottom nav ở trang sản phẩm). Desktop: không cần.

**Bằng chứng.** **Chưa có nghiên cứu công khai của Baymard/NN/g** về thanh mua dính; A/B test của
vendor cho kết quả trái chiều (+6,5% chuyển đổi ở một test; ở test khác add-to-cart +6,8% nhưng
doanh thu/khách −7,7%) [thứ cấp]. https://cleancommit.io/ab-tests/mobile-bottom-sticky-add-to-cart-button/ .
Baymard #791: nút "Thêm vào giỏ" phải nổi bật và duy nhất (không cạnh tranh với nút khác).
https://baymard.com/guidelines/791-add-to-cart-button-prominence . WCAG 2.2 SC 2.4.11 (AA): thanh
dính không được che hoàn toàn phần tử đang focus; kỹ thuật C43 `scroll-padding`.
https://www.w3.org/WAI/WCAG22/Understanding/focus-not-obscured-minimum.html
→ Kết luận: làm, nhưng **đo bằng A/B** trước khi coi là mặc định.

### 4.10 Cross-sell

**Khuyến nghị.** Hai khối tách biệt: (1) **"Dùng kèm"** (bổ sung: nắp, xẻng gỗ, miếng lót, dầu
dưỡng) đặt **gần nút mua**, thêm được bằng một chạm; (2) **"Có thể bạn thích"** (thay thế: chảo
cùng cỡ chất liệu khác) đặt dưới, mỗi thẻ đủ tên, giá, điểm, ảnh để so. Ở giỏ ưu tiên hàng bổ sung.
Ít mà trúng; không để gợi ý lấn thông tin sản phẩm.

**Bằng chứng.** Baymard: chỉ **42%** (50 site) có cả hai loại; **68%** site desktop thiếu thông tin
trong thẻ cross-sell; ở giỏ nên ưu tiên hàng bổ sung.
https://baymard.com/blog/product-page-suggestions , https://baymard.com/blog/product-recommendations-cart .
NN/g 2019: giới hạn gợi ý ở thứ thật liên quan.

**Ví dụ.** IKEA "Complete with" (vá múc canh 4 £, nút thêm ngay) cạnh khối mua + "Accessories"
riêng. Le Creuset: phụ kiện (túi đựng nồi, khoá nắp) ngay trên nút "Add to Bag" (14/11/2026).

### 4.11 Bổ sung cho các điều đã nghiên cứu 13/11

- **Phí giao/ngày giao cạnh nút mua:** Baymard 2025–2026 — **67%** site không ước tính tổng chi phí
  (gồm giao) ở trang sản phẩm; **44%** không hiện/link chính sách đổi trả; 60% người mua tìm chính
  sách đổi trả ở trang sản phẩm.
- **Lưu sản phẩm khi chưa đăng nhập:** **89%** site có tính năng lưu khó tìm/khó dùng; 21% người mua
  dựa vào tính năng lưu (2025–2026). Cùng nguồn https://baymard.com/blog/current-state-ecommerce-product-page-ux
- **Đánh giá:** **89%** site không trả lời đánh giá tiêu cực (2025–2026); **43%** không có biểu đồ
  phân bố sao, 39% có nhưng không bấm lọc được; 90% người muốn xem một mức sao đã bấm vào biểu đồ.
  https://baymard.com/blog/user-ratings-distribution-summary

---

## 5. Giỏ và checkout

### 5.1 Các bước hay một trang

**Khuyến nghị.** Không cần tranh cãi một trang hay nhiều bước: **số trường nhập quan trọng hơn số
bước**. Gợi ý: checkout dạng **accordion 3 bước** (Giao hàng → Vận chuyển → Thanh toán) trên một URL,
bước xong **thu gọn thành tóm tắt** sửa được, cuối có bước **xem lại đơn** trước nút "Đặt hàng". Mời
tạo tài khoản **sau khi đặt xong**.

**Bằng chứng.**
- Baymard: một trang không tự nhiên tốt hơn nhiều bước đã tối ưu; tổng số trường mới là thứ quyết
  định. https://baymard.com/blog/one-page-checkout
- Baymard 2026 (44.000+ điểm): **72%** không thu gọn bước đã xong thành tóm tắt; **56%** không dời
  việc tạo tài khoản ra sau xác nhận. https://baymard.com/research-articles/current-state-of-checkout-ux
- Mobile: bước xem lại có thể bị tưởng là trang xác nhận, gây tới 10% bỏ checkout ở ca nặng.
  https://baymard.com/blog/accordion-checkout-usability
- Mua không cần tài khoản: đã nghiên cứu 13/11. **Bổ sung:** 62% site chưa đặt "mua không cần tài
  khoản" là lựa chọn nổi bật nhất (benchmark 2025) [thứ cấp].
  https://baymard.com/blog/make-guest-checkout-prominent

### 5.2 Tóm tắt đơn trên điện thoại

**Khuyến nghị.** Đầu mỗi bước có thanh **"Tóm tắt đơn · 1.250.000 ₫ ▾"** gập/mở; mở ra thấy món,
phí giao, giảm giá, tổng. Tổng tiền cuối cùng hiện **trước** khi khách chọn phương thức thanh toán.
Ở giỏ: nút "Thanh toán" dính đáy mobile cùng tổng tiền.

**Bằng chứng.** NN/g (02/12/2018): hiện tóm tắt đơn nổi bật ở đầu trang checkout mobile để khách
không phải cuộn tìm phí. https://www.nngroup.com/articles/mobile-checkout-ux/ . Baymard (2015, 50
site mobile): 33% không cho thấy tổng tiền trước khi hỏi thông tin thanh toán [số liệu cũ].
https://baymard.com/blog/mobile-ecommerce-checkout-forms . 12% bỏ giỏ vì không thấy tổng trước
(link bỏ giỏ ở đầu file).

### 5.3 Trường nhập tối thiểu

**Khuyến nghị.** Mục tiêu **7–8 trường** mặc định: họ tên (1 ô), số điện thoại (giải thích "để
shipper gọi khi giao"), email (tuỳ, nếu có SĐT), địa chỉ (ô tìm địa chỉ tự gợi ý + chọn
tỉnh/phường từ module `address`), ghi chú (ẩn sau link). Một địa chỉ mặc định, checkbox "Địa chỉ
nhận hoá đơn giống địa chỉ giao". Đánh dấu **cả** "bắt buộc" lẫn "tuỳ chọn". Nhãn **trên** ô, không
dùng placeholder làm nhãn. Đúng bàn phím (`inputmode="tel"`, `type="email"`), đúng `autocomplete`
để trình duyệt điền sẵn. Ô ít dùng (công ty, MST xuất hoá đơn) ẩn sau link "Cần hoá đơn công ty?".

**Bằng chứng.**
- Baymard: checkout Mỹ trung bình **23,48** phần tử (14,88 trường); lý tưởng **12–14** (7–8 trường);
  giảm được 20–60%. https://baymard.com/blog/cart-abandonment-rate
- Baymard 2026: **50%** không đánh dấu cả bắt buộc lẫn tuỳ chọn (32% người thử bỏ sót trường bắt
  buộc khi chỉ đánh dấu trường tuỳ chọn); **47%** không giải thích vì sao cần SĐT; 68% người ngại cho
  SĐT. https://baymard.com/research-articles/current-state-of-checkout-ux
- Tra địa chỉ tự động: 55% site không có (2023); 9% đơn trong test có lỗi gõ địa chỉ.
  https://baymard.com/blog/automatic-address-lookup
- **61%** site mobile dùng sai bàn phím (nghiên cứu mobile 2 năm). https://baymard.com/learn/ux-statistics
- NN/g 2018: đúng bàn phím, hỗ trợ autofill, tự suy trường từ trường khác.
  https://www.nngroup.com/articles/mobile-checkout-ux/

### 5.4 Kiểm lỗi trực tiếp (inline validation)

**Khuyến nghị.** Kiểm khi **rời ô** (blur); ô độ dài cố định (SĐT, mã bưu chính, mã OTP) kiểm khi
gõ đủ độ dài; **không báo lỗi khi đang gõ dở**; lỗi biến mất ngay khi sửa đúng (kiểm theo từng phím
khi ô đang lỗi); dấu ✓ cho ô đúng. Thông báo lỗi nói **cụ thể phải sửa gì** ("Số điện thoại cần 10
số, bắt đầu bằng 0"), không "Không hợp lệ". Vẫn giữ kiểm lỗi khi bấm gửi, gom mọi lỗi một lần (khớp
D-083 của Autoxide).

**Bằng chứng.** Baymard 09/01/2024: **31%** site không có kiểm lỗi trực tiếp; 6 quy tắc trên.
https://baymard.com/blog/inline-form-validation . Baymard (2015): 92% site mobile dùng một câu lỗi
chung chung [số liệu cũ]. https://baymard.com/blog/mobile-ecommerce-checkout-forms

### 5.5 Tin cậy (bảo mật, đổi trả)

**Khuyến nghị.** Khối thanh toán **đóng khung riêng** (nền/viền khác phần địa chỉ) + 1–2 icon khoá/
logo đối tác thanh toán bên trong khung; câu ngắn "Bếp Mộc không lưu số thẻ — thanh toán qua [đối
tác có giấy phép]" (đúng D-187). Dưới nút đặt hàng: một dòng tóm tắt đổi trả ("Đổi trả 30 ngày ·
miễn phí lần đầu") có link chính sách viết lời thường. Hiện đủ phương thức nhận (giao tận nơi, nhận
tại cửa hàng) ngay ở bộ chọn vận chuyển, ghi **ngày giao** (đã nghiên cứu 13/11).

**Bằng chứng.** 19% bỏ giỏ vì không tin site giữ thẻ; 13% vì chính sách đổi trả (link bỏ giỏ). Baymard
"perceived security": người dùng đánh giá độ an toàn qua **vẻ ngoài** của khung thanh toán; đóng khung
+ 1–2 icon trong khung hiệu quả. https://baymard.com/blog/perceived-security-of-payment-form .
Baymard 2026: **53%** không gom mọi phương thức nhận hàng vào bộ chọn vận chuyển; **48%** chỉ ghi tốc
độ thay vì ngày giao; **83%** không hiện giờ chốt đơn dạng đếm ngược; **95%** không dùng nút +/− để
đổi số lượng. https://baymard.com/research-articles/current-state-of-checkout-ux . Baymard #1501:
chính sách đổi trả có tóm tắt, tiêu đề, lời thường. https://baymard.com/learn/ecommerce-ux-best-practices

**Ví dụ.** IKEA đặt panel "It's OK to change your mind — 365 ngày đổi trả" ngay ở trang sản phẩm.
Made In có mục "Shipping, Warranty, and Returns" (dùng thử 60 ngày, bảo hành trọn đời) (14/11/2026).

### 5.6 Giỏ hàng

**Khuyến nghị.** Bấm icon giỏ trên mobile đi thẳng tới trang giỏ; nút xoá cạnh từng món; số lượng
bằng nút +/−; thay đổi áp ngay (không nút "Cập nhật"); giữ giỏ khi đăng nhập trên thiết bị khác.

**Bằng chứng.** NN/g 2018 mobile checkout (link 5.2); Baymard 2026: 95% không dùng nút +/− (link 5.5).

---

## 6. Tài khoản và sau mua

### 6.1 Trang đơn hàng

**Khuyến nghị.** Trang "Đơn của tôi": đơn **đang giao lên đầu**; mỗi đơn hiện sẵn ảnh món, tên,
tổng tiền, trạng thái, **ngày giao dự kiến** (không giấu trong phần gập); nút "Mua lại". Trang chi
tiết: dòng thời gian các mốc (đã đặt → đóng gói → đang giao → đã giao), tên đơn vị vận chuyển, mã vận
đơn, bằng chứng giao — **theo dõi ngay trên site**, không đẩy sang site hãng vận chuyển. Huỷ/đổi địa
chỉ khi đơn chưa đóng gói. Menu tài khoản ở góc trên-phải.

**Bằng chứng.** Baymard Accounts & Self-Service (09/09/2026; 4.000+ giờ, 1.400+ lỗi quan sát, khảo
sát 1.000+ người Mỹ): theo dõi đơn là tính năng tự phục vụ quan trọng nhất; người dùng cần 3 thứ: ngày
giao dự kiến, tên hãng vận chuyển, các mốc giao; giấu thông tin đơn trong phần gập tệ ngang việc không
hiện; "mua lại" là ca dùng lớn; 81% lưu thẻ ở site hay mua; 60% thích có 2FA.
https://baymard.com/blog/accounts-and-self-service-ux-research-2026 . Baymard #873 (đơn đang mở lên
đầu), #922 (mốc trạng thái), #886 (menu góc trên-phải). https://baymard.com/learn/ecommerce-ux-best-practices .
56% site không tích hợp đủ thông tin theo dõi trong site [thứ cấp]. https://baymard.com/blog/integrate-tracking-info

### 6.2 Đổi trả tự phục vụ

**Khuyến nghị.** Nút "Đổi/trả" ở từng món trong đơn, **ghi rõ hạn** ("Còn 12 ngày để đổi trả");
hết hạn thì vẫn hiện nút mờ + lý do, không ẩn mất. Luồng: chọn món → lý do → cách gửi (shipper tới
lấy / mang ra cửa hàng) → cách hoàn tiền (hiện đủ lựa chọn, không giấu sau link) → xác nhận. Gửi phiếu
gửi hàng qua email, cập nhật trạng thái đơn ngay khi khởi tạo trả.

**Bằng chứng.** Baymard 2026: đổi trả là phần thay đổi nhiều nhất từ 2018; người dùng khó biết món
nào còn được trả; ẩn nút "Start Return" khi hết hạn gây bối rối; lựa chọn hoàn tiền giấu sau link
gây nhầm (link 6.1). Benchmark 2024: 54% site có lỗi nặng trong luồng trả; 55% không cập nhật trạng
thái sau khi khởi tạo trả [thứ cấp]. https://baymard.com/blog/order-returns-ecommerce-ux

---

## 7. Nội dung

### 7.1 Hướng dẫn và công thức nấu ăn

**Khuyến nghị.** Hai loại nội dung, nối hai chiều với sản phẩm:
- **Hướng dẫn chọn & dùng** ("Chọn chảo cho bếp từ", "Tôi chảo gang lần đầu", "Vì sao đáy 5 lớp"):
  link từ trang danh mục và mục "Dùng & chăm sóc" của sản phẩm.
- **Công thức**: mỗi công thức có khối "Dụng cụ dùng trong món này" (thẻ sản phẩm, thêm giỏ được);
  trang sản phẩm có 3 công thức liên quan + "Xem tất cả".
- Bài viết là nội dung của module `content`, liên kết sản phẩm bằng ID (không join bảng — luật kiến
  trúc số 1, 4).

**Bằng chứng.** Chưa tìm được nghiên cứu Baymard/NN/g công khai đo riêng hiệu quả công thức nấu ăn.
Baymard 2026: trang danh mục trung gian có thể dẫn người dùng bằng "lối đi cảm hứng, sản phẩm nổi
bật, nội dung tuyển chọn" (mô tả nghiên cứu Premium). https://baymard.com/homepage-and-category-usability .
NN/g 2019: mô tả phải cho biết sản phẩm "dùng thế nào"; how-to video là tính năng nâng cao.
→ Đây là khuyến nghị theo **thực tiễn ngành**, mức tin cậy thấp hơn các mục có số liệu.

**Ví dụ.** Le Creuset đặt 3 thẻ công thức + "See All Recipes" ngay trong trang sản phẩm nồi, và
"Recipes" ở menu phụ (14/11/2026). Our Place có bộ sưu tập theo đầu bếp ("Kenji López-Alt's Picks")
trong menu Quà tặng (14/11/2026).

### 7.2 "Shop the look" (mua theo khung cảnh)

**Khuyến nghị.** Ảnh khung cảnh (căn bếp, bàn ăn đã bày) có **điểm chạm** trên từng món + **danh
sách đúng các món trong ảnh** ngay dưới (giá, chọn biến thể, "Thêm tất cả vào giỏ"). Báo rõ "Ảnh này
mua được". Không để ảnh cảm hứng là ngõ cụt.

**Bằng chứng.** Baymard 2026: **64%** site không link sản phẩm trong ảnh cảm hứng; nên dẫn tới trang/
lớp phủ chỉ liệt kê các món trong ảnh và báo rõ điều đó; trên mobile hậu quả nặng hơn.
https://baymard.com/blog/ecommerce-navigation-best-practice . Các con số tăng doanh thu "shop the
look" chỉ có từ vendor, không dùng.

**Ví dụ.** IKEA có khối "Get the look" ở cuối trang sản phẩm (14/11/2026).

---

## 8. Accessibility và hiệu năng

### 8.1 Vùng chạm

**Khuyến nghị.** Mọi nút/link chính ≥ **44×44 px** (swatch, nút +/−, chip filter, nút đóng sheet);
tối thiểu tuyệt đối 24×24 px có khoảng cách. Khoảng cách giữa các vùng chạm ≥ 8 px.

**Bằng chứng.** WCAG 2.2 SC 2.5.8 (AA): ≥ 24×24 CSS px hoặc đủ khoảng cách; SC 2.5.5 (AAA): 44×44,
W3C khuyên nhắm 44×44 cho điều khiển quan trọng.
https://www.w3.org/WAI/WCAG22/Understanding/target-size-minimum.html . Baymard (mobile 2 năm): **66%**
site đặt vùng chạm quá sát, **32%** quá nhỏ. https://baymard.com/learn/ux-statistics

### 8.2 Tương phản

**Khuyến nghị.** Chữ thường ≥ **4.5:1**, chữ lớn ≥ 3:1, viền ô nhập/icon/focus ≥ 3:1. Cẩn thận với
tông "gỗ mộc" nhạt: chữ xám nhạt trên nền kem thường rớt dưới 4.5:1. Giá gạch ngang, nhãn "Hết hàng"
cũng phải đạt. Không in chữ vào ảnh banner.

**Bằng chứng.** WebAIM Million **2026** (dữ liệu 02/2026): **95,9%** trang chủ có lỗi WCAG phát hiện
được; **83,9%** có chữ tương phản thấp (2025: 79,1%); 53,1% thiếu alt; 51% ô nhập thiếu nhãn.
https://webaim.org/projects/million/ . Baymard (2021, 33 site): 24% có chữ trong ảnh không đủ tương
phản; 63% có chữ trong ảnh mà không truyền đạt lại bằng markup.
https://baymard.com/blog/accessibility-benchmark-launch . WCAG 1.4.3 / 1.4.11:
https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html

### 8.3 Focus

**Khuyến nghị.** Viền focus rõ (vd 2–3 px, tương phản ≥ 3:1) dùng `:focus-visible`, **không bao giờ**
`outline: none` trần. Thứ tự focus theo thứ tự đọc. Sheet filter, mini giỏ, lớp phủ ảnh: giữ focus
trong khi mở, Esc đóng, trả focus về nút đã mở. Có link "Bỏ qua tới nội dung" và "Bỏ qua ảnh". Thanh
dính dùng `scroll-padding` để không che phần tử đang focus.

**Bằng chứng.** Baymard (2021): **48%** site không giữ viền focus luôn thấy; **64%** có lỗi điều
hướng bàn phím (45% có chức năng không dùng được bằng bàn phím; 9% bẫy focus). Link 8.2. WCAG 2.2 SC
2.4.11 (AA) Focus Not Obscured. Link 4.9. IKEA có "Skip to main content", "Skip images", "Skip to
product list" (14/11/2026).

### 8.4 Giảm chuyển động

**Khuyến nghị.** Tôn trọng `prefers-reduced-motion: reduce`: tắt parallax, zoom tự động, slide
trượt, hiệu ứng "bay vào giỏ"; thay bằng chuyển trạng thái tức thì hoặc fade ngắn. Không có gì tự
chuyển động > 5 giây mà không có nút dừng. Hiệu ứng chỉ dùng `transform`/`opacity`.

**Bằng chứng.** WCAG 2.2 SC 2.3.3 Animation from Interactions (AAA) và SC 2.2.2 (A).
https://www.w3.org/WAI/WCAG22/Understanding/animation-from-interactions.html . web.dev CLS: animation
dùng `transform` không gây layout shift (link 8.5).

### 8.5 Ảnh có kích thước cố định, không nhảy bố cục — và Core Web Vitals

**Khuyến nghị.**
- Mọi `<img>`/`<video>` có `width` + `height` (hoặc `aspect-ratio` trên khung); mọi ảnh thẻ cùng tỉ
  lệ (vd 1:1 hoặc 4:5); mọi ứng viên `srcset` cùng tỉ lệ; `<picture>` đặt kích thước trên từng
  `<source>`.
- Ảnh LCP (ảnh hero / ảnh chính trang sản phẩm): **không lazy-load**, `fetchpriority="high"` (chỉ
  1–2 ảnh), có sẵn trong HTML (không chèn bằng JS), không có hiệu ứng fade-in. Ảnh dưới màn hình đầu:
  `loading="lazy"`. Định dạng AVIF/WebP, đúng cỡ theo viewport, qua CDN cùng origin.
- Chừa chỗ trước cho banner, khối "có thể bạn thích" tải sau, thanh thông báo; font dùng
  `font-display: optional` hoặc fallback chỉnh `size-adjust`.
- Mục tiêu (đo ở percentile 75, tách mobile/desktop): **LCP ≤ 2,5 s · INP ≤ 200 ms · CLS ≤ 0,1**.
  Gắn thư viện `web-vitals` gửi số đo thật (RUM) về backend.

**Bằng chứng.**
- Ngưỡng Core Web Vitals (web.dev, cập nhật 31/10/2024). https://web.dev/articles/vitals
- Cách chống CLS (web.dev, cập nhật 07/02/2025). https://web.dev/articles/optimize-cls
- Cách tối ưu LCP (web.dev, cập nhật 31/03/2025). https://web.dev/articles/optimize-lcp
- **Nuvemshop/Tiendanube** (nền tảng thương mại, web.dev 24/06/2026): bỏ lazy-load ảnh đầu, thêm
  `fetchpriority="high"`, bỏ transition ở khối đầu → LCP "good" 57% → 96%, đạt CWV 48% → 72%, chuyển
  đổi mobile từ Google organic **+8,9%** (01/2025 → 01/2026). https://web.dev/case-studies/nuvemshop
- **Rakuten 24** (A/B test 1 tháng, web.dev, không ghi năm): dùng `aspect-ratio`/`min-height` giữ chỗ
  ảnh, CLS cải thiện 92,72%; doanh thu/khách **+53,37%**, chuyển đổi **+33,13%**.
  https://web.dev/case-studies/rakuten
- Farfetch: chuyển đổi tăng khi CLS/LCP tốt hơn (web.dev). https://web.dev/case-studies/farfetch

---

## Đề xuất cho Bếp Mộc

Mỗi dòng: đề xuất — lý do. Không chép thiết kế hay tên thương hiệu của site mẫu. Các điều đã có từ
13/11 không lặp lại ở đây.

### Ưu tiên Cao

1. **Bỏ carousel tự chạy ở trang chủ, thay bằng các khối tĩnh xếp dọc** — Baymard: khối tĩnh tốt bằng
   hoặc hơn, mobile tự chạy gây bấm nhầm; WCAG 2.2.2.
2. **Lưới danh mục có ảnh ngay dưới hero + danh mục sản phẩm ở cấp đầu menu mobile** — 39% site thiếu
   thumbnail danh mục, 22% giấu danh mục ở mobile (Baymard 2026).
3. **Mục "Ưu đãi" ở menu chính, mở danh sách đã lọc sẵn, gỡ filter được** — có nhóm khách săn giảm
   giá ngay khi vào; 32% site làm thiếu/sai (Baymard 2022).
4. **Menu desktop mở bằng hover có delay 300–500 ms, tiêu đề nhóm bấm được, mở được bằng bàn phím** —
   62% site không có delay, 31% tiêu đề không bấm được (Baymard 2026).
5. **Bộ lọc mobile: sheet riêng, nút áp ghi "Xem N sản phẩm", chip filter đang bật, chọn nhiều giá
   trị** — mobile list kém nhất (78% mediocre); Baymard 2026 và NN/g đều khuyên áp theo lô.
6. **Bộ lọc riêng cho đồ bếp: chất liệu, loại bếp dùng được, dung tích/đường kính, rửa máy được** —
   đây là tiêu chí quyết định mua đồ bếp; 51% site thiếu filter thiết yếu.
7. **Gộp biến thể thành một thẻ, có swatch đủ màu (cuộn ngang trên mobile), điểm + số lượt đánh giá**
   — 42% không gộp, 57% thiếu swatch trên mobile.
8. **"Xem thêm" + "Đang xem 24/133" + Back về đúng vị trí bằng cursor trong URL** — Baymard #501,
   NN/g 2022; khớp sẵn `fetch_page`/`cursor` của core.
9. **Chọn cỡ bằng nút, mỗi cỡ ghi đường kính · dung tích · số người ăn; hết hàng vẫn hiện mờ** — 57%
   site dùng dropdown; 42% người dùng khó đoán kích thước.
10. **≥ 5 ảnh/sản phẩm gồm ảnh trong bếp, ảnh thể hiện kích thước, ảnh "trong hộp"** — 37% thiếu ảnh
    in-scale, 44% không chụp đồ đi kèm.
11. **Mục "Dùng & chăm sóc" bắt buộc cho mọi sản phẩm đồ bếp** — câu hỏi số một sau mua (rửa máy,
    bếp từ, tôi chảo); mọi site đồ bếp mẫu đều có.
12. **Bảng thông số có cấu trúc, cùng nhãn và đơn vị cho cả ngành hàng (lấy từ thuộc tính của
    `catalog`)** — thông số không đồng nhất làm người mua không so được; cũng giúp AI agent đọc qua API.
13. **Trang sản phẩm dạng trang dài + accordion + link nhảy, không dùng tab ngang** — 27% bỏ sót nội
    dung trong tab, so với 8% với accordion.
14. **Checkout accordion: bước xong thu gọn thành tóm tắt, có bước xem lại, mời tạo tài khoản sau khi
    đặt** — 72% và 56% site làm sai (Baymard 2026).
15. **Form checkout ~7–8 trường, đánh dấu cả bắt buộc lẫn tuỳ chọn, giải thích vì sao cần SĐT, đúng
    `autocomplete`/`inputmode`** — giảm được 20–60% trường; 68% người ngại cho SĐT.
16. **Kiểm lỗi khi rời ô, xoá lỗi ngay khi sửa đúng, câu lỗi nói cách sửa** — 31% site không có
    (Baymard 2024); `code` lỗi của core đã là i18n key nên làm được câu lỗi cụ thể.
17. **Tóm tắt đơn gập/mở ở đầu mỗi bước checkout mobile, tổng tiền thấy trước bước thanh toán** —
    NN/g 2018; 12% bỏ giỏ vì không thấy tổng.
18. **Vùng chạm ≥ 44 px, tương phản ≥ 4.5:1 (kiểm kỹ bảng màu "gỗ"), viền focus luôn thấy** — 83,9%
    trang chủ có chữ tương phản thấp (WebAIM 2026); 48% site ẩn focus.
19. **Mọi ảnh có width/height hoặc aspect-ratio; ảnh LCP không lazy, `fetchpriority="high"`** — rẻ
    nhất mà có số liệu chuyển đổi rõ (+8,9% ở một nền tảng thương mại, 2026).

### Ưu tiên Vừa

20. **Hai khối cross-sell tách biệt: "Dùng kèm" sát nút mua, "Có thể bạn thích" ở dưới** — chỉ 42%
    site có cả hai; hàng bổ sung (nắp, xẻng, dầu dưỡng) rất tự nhiên với đồ bếp.
21. **Thanh mua dính đáy trên mobile, có A/B test trước khi bật mặc định** — chưa có nghiên cứu
    độc lập, vendor test trái chiều; phải không che focus (WCAG 2.4.11).
22. **Ảnh thứ hai khi rê chuột (ưu tiên ảnh trong bếp), có delay nhỏ** — NN/g 2022; 80% site thiếu
    nhiều ảnh trong thẻ.
23. **Bảng so cỡ ngay dưới bộ chọn cỡ + ảnh các cỡ đặt cạnh nhau** — giải quyết lo ngại kích thước
    mà không bắt mở nhiều trang.
24. **Trang đơn hàng: đơn đang giao lên đầu, ngày giao dự kiến + hãng + mốc, theo dõi trong site,
    nút "Mua lại"** — theo dõi đơn là tính năng tự phục vụ số một (Baymard 2026).
25. **Đổi trả tự phục vụ ghi rõ hạn còn lại, không ẩn nút khi hết hạn, hiện đủ cách hoàn tiền** —
    đổi trả là luồng nhiều lỗi nhất (Baymard 2024–2026).
26. **Khung thanh toán đóng khung riêng + câu "không lưu số thẻ, thanh toán qua đối tác có giấy
    phép"** — 19% bỏ giỏ vì không tin site; khớp D-187.
27. **Giá theo đơn vị cho hàng bán theo bộ/dung tích (vd "/món", "/lít")** — 67–81% site không có,
    giúp so bộ nồi với nồi lẻ.
28. **Công thức nấu ăn có khối "dụng cụ dùng trong món", trang sản phẩm hiện 3 công thức liên quan**
    — hợp ngành hàng, dùng module `content` sẵn có; bằng chứng định lượng còn yếu.
29. **Biểu đồ phân bố sao bấm lọc được, trả lời công khai đánh giá tiêu cực** — 43% không có biểu đồ,
    89% không trả lời đánh giá xấu.
30. **Hỗ trợ `prefers-reduced-motion`, animation chỉ dùng transform/opacity** — WCAG 2.3.3; không
    tốn thêm công khi làm từ đầu.

### Ưu tiên Thấp

31. **"Mua theo khung cảnh": ảnh bếp/bàn ăn có điểm chạm + danh sách món + "thêm tất cả"** — 64%
    site để ảnh cảm hứng thành ngõ cụt; làm khi đã có ảnh khung cảnh tốt.
32. **Nút thêm nhanh trên thẻ chỉ cho sản phẩm không có biến thể; không làm Quick View trên desktop**
    — Quick View desktop thường hại hơn lợi (Baymard).
33. **Bottom nav cho bản PWA/app sau này, web mobile giữ dạng kết hợp có nhãn "Danh mục"** — chưa có
    bằng chứng độc lập bottom nav hơn trên web; tránh chồng với thanh mua dính.
34. **Breadcrumb mobile rút gọn còn 1–2 cấp cuối + link "Quay lại kết quả" giữ filter** — NN/g 2018/
    2026; mất filter khi quay lại là nguyên nhân bỏ site.
35. **Video ngắn cho sản phẩm đắt hoặc cần hướng dẫn (chảo gang/thép cần tôi dầu)** — Baymard #759;
    tốn công sản xuất nên làm sau ảnh.
36. **Đo Core Web Vitals thật (RUM) bằng `web-vitals` gửi về backend, có ngưỡng cảnh báo** — "đo
    trước rồi mới tối ưu" đúng tiêu chí "Tối ưu" của dự án.
