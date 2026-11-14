# Đối chiếu hai chiều: thiết kế 2 ↔ lộ trình (13/11/2026)

Chủ dự án yêu cầu: thiết kế và lộ trình phải khớp **theo hai chiều**:
- thứ có trên màn hình phải có trong lộ trình, kèm API;
- mục trong lộ trình mà người dùng thấy được phải có màn hình.

API thiếu thì viết đề bài cho teammate.

## Cách làm

- Hai agent chỉ đọc, chia theo ranh giới nghiệp vụ:
  - **A, nền tảng và quản trị chung** (bước 15–35, 37–40, 50–52): [`ui-trace/A-nen-tang.md`](ui-trace/A-nen-tang.md).
  - **B, thương mại và website bán hàng** (bước 36, 41–49): [`ui-trace/B-thuong-mai.md`](ui-trace/B-thuong-mai.md).
- Mỗi khẳng định "không có trong lộ trình" đều ghi kèm từ khoá đã tìm.
- Agent chính kiểm lại các phát hiện chính bằng tài liệu trong repo. Có một phát hiện sai và đã loại: báo
  cáo B ghi đặc tả 46 "có nhắc COD", nhưng 5 chỗ khớp chỉ là chữ `code`.
- Số dòng của thiết kế lấy theo bản quản trị 37 và bản website 27. Thiết kế sửa sau đó thì dòng sẽ lệch.

## Kết quả

| | Nền tảng (A) | Thương mại (B) |
|---|---|---|
| Thiết kế cần vẽ thêm hay sửa | 22 | 19 |
| Lộ trình cần bổ sung | 10 | 14 |
| API thiếu | 12 | 14 |
| Thiết kế lệch đặc tả | 18 | 6 nhóm |

Nhận xét chung:

- Phần lớn là **chưa tới bước**: bước 33–49, 51, 52 đều chưa làm. Đây là tình trạng bình thường.
- Có ba loại cần sửa ngay:
  - **thiết kế nói sai so với đặc tả**, ví dụ trạng thái đơn, bút toán, quyền, tên permission;
  - **đặc tả tự mâu thuẫn**, ví dụ bút toán giữa 36, 45 và 46, hạn thanh toán VietQR;
  - **thiếu hẳn**, ví dụ nội dung theo website, job API, COD trên sàn.
- Có sáu chỗ phải **đối chiếu văn bản luật gốc** (D-186):
  - bán trước khi thông báo website: bị cấm hay chỉ bị phạt;
  - hạn trả lời yêu cầu dữ liệu cá nhân;
  - thời điểm lập hoá đơn;
  - GTGT trên phí giao;
  - khấu trừ thay theo loại người bán;
  - số định danh 12 số dùng làm mã số thuế.

## Ai làm gì

- **Lộ trình, đặc tả, API:** đề bài cho teammate [`docs/prompts/ui-trace-roadmap.md`](../prompts/ui-trace-roadmap.md),
  kèm 10 hướng đã chọn (H1–H10). Hai đề bài trước vẫn giữ: `ui-api-gaps.md` và `ui-help-api.md`.
- **Thiết kế:** agent chính làm. Theo dõi bằng danh sách dưới đây.

## Việc thiết kế

### Đợt 1: sửa màn hình đang có cho khớp đặc tả (xong 14/11/2026: quản trị bản 39, website bản 28)

- [x] Hộp việc: bỏ "Đã xử lý", thay bằng hành động thật, Hoãn và **Giao cho…**; sửa chữ ở bản đồ (A-a1)
- [x] Đơn hàng: trạng thái theo bảng của 45, gồm "Chờ xác nhận" theo H2; "Đổi trả" thành view; thêm Đã
      giao, Giao thất bại, Đã huỷ; đơn tại quầy thành "Đã giao". Sổ cái theo 45 và 46. Hoá đơn lập khi
      giao. Khấu trừ thay chỉ cho hộ và cá nhân (B-a1)
- [x] Tổ chức: mời thành viên không chọn độ sâu, độ sâu đi theo role; "Tách thành khách riêng" thành
      "Gửi yêu cầu tách" (A-a2, A-a7)
- [x] Nền tảng: tách "Tạm khoá hẳn" khỏi "Chỉ đọc"; thời hạn hỗ trợ 1 / 4 / 8 giờ; nút đội database
      hiện dry-run (A-a6)
- [x] Tạo API key: thêm xác nhận lại danh tính và hạn dùng (A-a9)
- [x] Tuân thủ: yêu cầu tự phục vụ chỉ để theo dõi; KYC dùng trạng thái của đặc tả (A-a10)
- [x] Bỏ "tỉ lệ mở / bấm" của bản tin (A-a8)
- [x] Gian hàng và tài khoản: bỏ tiêu chí "trả lời tin nhắn" và nút "Nhắn người bán" (B-a19)
- [x] Checkout: tạo tài khoản sau khi đặt hàng phải qua OTP số điện thoại (B-a16)
- [x] Dữ liệu mẫu: điểm sao, thứ trong tuần, giá thớt, ngày xác thực người bán, thứ tự hoàn tiền, tên
      permission, nội dung chuyển khoản, ký hiệu hoá đơn máy tính tiền, mã số thuế 12 số (A-a22, B-a17)
- [x] Bản đồ phân hệ: thêm Kênh bán (48) và Tiếp thị liên kết (49); Khách hàng thêm điểm và hạng; Tài
      chính thêm hàng chờ sai lệch; Sản phẩm thêm Nhập hàng (B-a18)

### Đợt 2: vẽ màn hình còn thiếu, xếp theo vai trò (xong 14/11/2026: quản trị bản 42, website bản 32)

- [x] Người bán trên sàn: trung tâm người bán (B-a2); vận hành sàn (B-a3)
- [x] Kế toán: sổ cái, hàng chờ sai lệch, hoá đơn điện tử, thuế, khoá sổ (B-a4)
- [x] Cửa hàng: nhận tại quầy (B-a5); ca thu ngân (B-a11)
- [x] Đổi trả và đánh giá phía nhân viên (B-a6)
- [x] Kho và nhập hàng (B-a7); vận chuyển (B-a8)
- [x] Khuyến mại (B-a9); sản phẩm và giá (B-a10)
- [x] Khách hàng và CRM (B-a12); kênh bán (B-a13); tiếp thị liên kết và livestream (B-a14)
- [x] Người mua (B-a15), xong ở đợt 3. Đã có: Đã lưu, Điểm và hạng, Bảo hành, Giới thiệu bạn, Báo có hàng, trang
      Khuyến mại (flash sale, mã), trang Bán trên Chợ Bếp Mộc. Còn: tải hàng số, tranh chấp với người bán;
      checkout thêm trả góp, dùng điểm, mã người tiếp thị, giỏ chỉ có hàng số hay dịch vụ.
- [x] Đăng nhập và tài khoản của tôi (A-a3); trạng thái toàn khung: tạm khoá, chỉ đọc, bảo trì, module
      chưa bật (A-a4)
- [x] Cài đặt: Job (A-a11), Tên miền và Kết nối (A-a12), Gói và thuê bao phía khách (A-a16)
- [x] Tổ chức: lời mời, uỷ quyền, cài đặt nút, hồ sơ công khai (A-a13); quyền vào hỗ trợ phía khách (A-a14)
- [x] Nền tảng: gói và giá, thuê bao, cấp phát, sức khoẻ, đội database (A-a15)
- [x] Biểu mẫu, bản tin, bình luận: các module đã chạy mà chưa có màn hình (A-a17)
- [x] Nội dung (A-a18), xong ở đợt 3. Đã có: trang, danh mục, tác giả, menu, chuyển hướng, lịch sử phiên bản. Còn: vẽ
      lại trình soạn bài và thư viện media theo khung 2 (đang dùng bản của thiết kế 1).
- [x] Tuân thủ còn thiếu (A-a19); nhân sự, webhook, audit (A-a20)
- [x] Việc, hoạt động, ghim, xem trước `dry_run` (A-a21); ô lọc bằng lời thường xong ở đợt 3.

Chỗ trống API và đặc tả mà các màn đợt 2 lộ ra: `docs/prompts/ui-phase2-api.md`.

### Đợt 3: phần còn lại (xong 14/11/2026)

- [x] Phần còn thiếu của B-a15 (tải hàng số, tranh chấp phía người mua, dùng điểm, mã người tiếp thị, trả góp
      qua đối tác, giỏ chỉ có hàng số), A-a18 (trình soạn bài và thư viện media theo khung 2) và A-a21 (ô lọc
      bằng lời thường)
- [x] Báo cáo theo kênh bán (bước 48); trang công khai của phiên livestream (bước 49)
- [x] Thêm ngoài danh sách:
      - thanh danh mục có bảng xổ;
      - đánh giá và hỏi đáp có lọc, sắp xếp, nút hữu ích;
      - tóm tắt đơn thu gọn khi đặt hàng trên điện thoại;
      - tab hiện đủ trên điện thoại;
      - ba trang chiến dịch có phong cách riêng: dày khuyến mại, tối giản, sang trọng.

Chỗ trống API và đặc tả của đợt 3: `docs/prompts/ui-phase3-api.md`. Menu động lấy từ backend: chờ chủ dự án
chọn cách.

Mọi màn hình mới theo luật đã đặt: dùng bộ kiểu dáng có sẵn của hai thiết kế, có trợ giúp ⓘ ở chỗ khó
hiểu, và qua bộ kiểm tra tự động gồm:
- mọi tab và mục;
- khổ 360, 390, 430px, máy tính bảng và máy tính;
- chế độ tối;
- nhiều khách khác nhau.

Không còn chữ đè, tràn ngang hay chữ bị cắt thì mới đăng. Từ đợt 2 bộ kiểm tra đo thêm:
- **lấn lề:** nhãn hay nút vượt quá phần bên trong của khung chứa, làm lề hai bên không đều;
- **thẻ lẻ:** lưới nhiều cột mà hàng cuối chỉ còn một thẻ.

Nhóm có số thẻ cố định thì dùng số cột chia hết số thẻ ở mọi khổ, hoặc xếp kiểu giãn đều. Khổ máy tính
bảng (768–1100) được kiểm như khổ điện thoại.
