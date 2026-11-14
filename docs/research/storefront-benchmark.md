# Mặt tiền bán hàng: đối chiếu với các trang hàng đầu và sửa theo bằng chứng (14/11/2026)

Tài liệu nội bộ, cho chủ dự án và agent. Ghi lại vòng nghiên cứu và rà soát website mẫu "Bếp Mộc": đã học gì,
từ đâu, đã sửa gì, việc gì để sau và vì sao. Ghi chép gốc có nguồn đầy đủ (URL, năm, mức chắc chắn):

- [`storefront/vietnam.md`](storefront/vietnam.md): các sàn và bán lẻ Việt Nam (Shopee qua blog chính thức, Tiki,
  Lazada, Thế Giới Di Động / Điện Máy Xanh, Bách Hoá Xanh, Elmich, Lock&Lock, Sunhouse).
- [`storefront/global.md`](storefront/global.md): Baymard Institute, Nielsen Norman Group, web.dev, WebAIM; các
  trang mẫu IKEA, Le Creuset, Made In, Our Place.

Nghiên cứu trước (13/11) ở `ui-design.md` mục 5 không lặp lại ở đây.

## Cách làm

Ba hướng chạy song song:

1. **Việt Nam:** người mua Việt quen với gì trên đầu trang, trang chủ, danh mục, trang sản phẩm, giỏ, thanh toán, sau
   mua, niềm tin và pháp lý.
2. **Quốc tế:** khuyến nghị kèm số liệu "bao nhiêu phần trăm site làm sai" và năm của số liệu.
3. **Rà soát chính site:** 14 trang ở 360, 390, 768 và 1440px, theo 10 tiêu chí: màn đầu, thứ bậc chữ, khoảng cách,
   thẳng hàng, nhất quán, vùng chạm, thanh dính, trạng thái, chế độ tối, chi tiết nhỏ. Kết quả: 7 lỗi nặng, khoảng
   30 lỗi vừa.

Trang nào chặn hay phải đăng nhập thì bỏ qua, không đoán. Một đề xuất bị loại vì trái quyết định của chủ dự án
(xem "Không làm").

## Điều quan trọng nhất học được

- **Điện thoại là chính và luôn kém hơn máy tính.**
  - Việt Nam: khoảng 92% người mua dùng điện thoại.
  - Baymard xếp trải nghiệm danh sách sản phẩm trên điện thoại "trung bình trở xuống" ở 78% site (2025), và
    checkout ở 59% site (2026).
  - Tỉ lệ bỏ giỏ trung bình 70%; lý do lớn nhất là chi phí phát sinh (40%).
- **Tab ngang làm người dùng bỏ sót nội dung:** 27% so với 8% khi dùng các mục mở/gập (accordion).
- **Thẻ sản phẩm ở Việt Nam nhiều thông tin nhưng không rối,** nhờ mỗi loại thông tin nằm cố định một dòng.
- **Mega menu:** mở sau 300–500 ms khi rê chuột (62% site không có độ trễ), tiêu đề nhóm phải bấm được, phải mở
  được bằng bàn phím.
- **Checkout:**
  - số ô phải nhập quan trọng hơn số bước; 7–8 ô là vừa;
  - bước đã xong thu gọn thành tóm tắt (72% site không làm);
  - kiểm lỗi khi rời ô;
  - giải thích vì sao cần số điện thoại.
- **Đồ bếp:**
  - mục "Dùng & chăm sóc" là câu hỏi số một sau mua;
  - chọn cỡ bằng nút có ghi số người ăn (57% site dùng dropdown);
  - có mục "Trong hộp có gì";
  - "Dùng kèm" đặt sát nút mua.
- **Niềm tin:**
  - cam kết lấy từ dữ liệu của từng sản phẩm. Một trang mẫu dán chung "bảo hành chính hãng" trong khi thông số
    của chính sản phẩm đó ghi "không bảo hành";
  - giá gạch phải là giá thật ngay trước khuyến mại;
  - chân trang đủ thông tin pháp lý.
- **Tương phản:** 83,9% trang chủ có chữ tương phản thấp (WebAIM 2026). Bảng màu tông gỗ nhạt cần đo.

## Đã sửa (website, đăng ngày 14/11/2026)

| Thay đổi | Trang | Nguồn |
|---|---|---|
| Đầu trang điện thoại từ 205px còn 113px, dính khi cuộn; ô tìm rộng hết, gợi ý tìm đủ chữ | mọi trang | rà soát (N2, mục 8) |
| "₫" và đơn vị không bao giờ tách khỏi số (khoảng trắng không ngắt) | mọi trang | rà soát (N3, mục 2) |
| Thẻ sản phẩm: thứ tự dòng cố định (tên · giá · sao · tình trạng kho · người bán) | trang chủ, danh mục, gian hàng | Việt Nam; rà soát mục 4 |
| Trang danh mục ở máy tính bảng: bộ lọc thành ngăn kéo, lưới 3 cột | danh mục | rà soát (N6) |
| Trang chủ điện thoại: màn đầu thấy danh mục (lưới 3 × 2) | trang chủ | Việt Nam; quốc tế (lưới danh mục có ảnh) |
| Bảng xổ mở sau 300 ms, bàn phím mở ngay | mọi trang | quốc tế |
| Tiêu đề đủ khoảng cách dòng cho dấu tiếng Việt | mọi trang | rà soát mục 6 |
| Bảng màu: mọi cặp chữ / nền chính ≥ 4,5:1 (sửa 4 cặp) | mọi trang | quốc tế (WCAG AA) |
| Trang sản phẩm: màn đầu điện thoại thấy tên, sao, giá; ảnh 4:3 | sản phẩm | rà soát (N1) |
| Trang sản phẩm: điện thoại dùng mục mở/gập thay tab | sản phẩm | quốc tế (27% so với 8%) |
| Chọn cỡ ghi số người ăn; thêm "Dùng & chăm sóc", "Trong hộp có gì", "Dùng kèm" | sản phẩm | quốc tế |
| Bỏ lặp: mã giảm giá 3 lần còn 1; ba khối giao hàng gộp một | sản phẩm | rà soát |
| Điều kiện mã luôn hiện một dòng (phạm vi, hạn); chi tiết trong mục mở rộng | sản phẩm | pháp luật khuyến mại (cần đối chiếu văn bản gốc) |
| Thanh mua dính ở máy tính bảng; không che phần đang được chọn bằng bàn phím | sản phẩm | quốc tế (WCAG 2.4.11) |
| Tài khoản điện thoại: danh sách 9 mục kiểu trang "Tôi" | tài khoản | rà soát (N4); Việt Nam |
| Thẻ đơn thẳng hàng; danh sách có biểu tượng gọn | tài khoản | rà soát (N5, mục 5) |
| Đánh giá của gian hàng: điểm, lọc sao, sắp xếp, hữu ích, xem thêm | gian hàng | rà soát (N7) |
| Checkout: nút chính trong cột tóm tắt dính (máy tính) | thanh toán | rà soát |
| Checkout: bước đã xong thu gọn có "Sửa"; 6 ô; đánh dấu bắt buộc / tuỳ chọn; vì sao cần số điện thoại; kiểm khi rời ô | thanh toán | quốc tế |
| Màn "Xong" phẳng, khối chuyển khoản lên đầu, bỏ gợi ý mua thêm | thanh toán | rà soát |
| Bài viết điện thoại: bảng so sánh thành thẻ, mục lục lên đầu | bài viết | rà soát |
| Livestream điện thoại: sản phẩm ghim ngay dưới video, phần pháp lý thu gọn (dòng tóm tắt luôn hiện) | livestream | rà soát |
| Trang sale: chữ ≥ 12px, nút ≥ 44px | siêu sale 12.12 | rà soát; quốc tế |

Kiểm tra: quét 14 trang ở 360, 390, 768, 1024 và 1440px, cộng chế độ tối, không có lỗi. Các kịch bản bấm qua những
trạng thái bên trong cũng đạt: các bước mua, đánh giá, trạng thái trang sản phẩm, tài khoản, thanh toán.

## Không làm, và vì sao

- **Tự đổi địa chỉ cũ ba cấp sang mới:** trái D-193 (chủ dự án quyết). Người gõ tên cũ vẫn tìm được phường, xã theo
  tên mới nhờ tìm không dấu.
- **Thanh điều hướng dưới đáy cho web điện thoại:** chưa có bằng chứng độc lập cho thấy tốt hơn trên web, và nó dễ
  chồng với thanh mua dính. Để dành cho bản ứng dụng sau này.
- **Quick View trên máy tính:** thường hại hơn lợi (Baymard).
- **Banner tự chạy:** không dùng. Trang chủ đã là các khối tĩnh xếp dọc.

## Đợt sau (đăng ngày 14/11/2026)

| Thay đổi | Trang |
|---|---|
| "Có thể bạn thích" (hàng thay thế), tách khỏi "Dùng kèm"; mỗi món ghi khác ở điểm nào | sản phẩm |
| Hỏi đáp có sắp xếp, nút "Hữu ích" cùng kiểu đánh giá | sản phẩm |
| Ngày giao viết thống nhất mọi trang ("Thứ Bảy 14/11 – Chủ nhật 15/11"), điện thoại viết gọn | mọi trang |
| Khối "Hôm nay nấu gì?": chọn món, bỏ dụng cụ đã có, thêm cả bộ vào giỏ | trang chủ |
| Sắp xếp "% giảm nhiều nhất" | danh mục |
| Nút "Đã nhận được hàng"; đổi trả có lý do chọn sẵn, ảnh hoặc video mở hộp, ghi rõ còn mấy ngày (hết hạn thì nút tắt kèm lý do); "Viết đánh giá · +20 điểm" | tài khoản |

## Còn để sau

- **Giá theo đơn vị** cho hàng bán theo bộ.
- **Liên hệ nhanh** (gọi, Zalo); nhận thông báo đơn qua Zalo.
- **Đo Core Web Vitals thật** và gửi về backend; ảnh có kích thước cố định, ảnh chính không lazy-load. Việc này thuộc
  repo giao diện khi dựng thật, bản thiết kế không đo được.

## Việc của backend

Ghi ở `docs/prompts/ui-phase3-api.md`, mục "Từ đợt sửa theo nghiên cứu".
