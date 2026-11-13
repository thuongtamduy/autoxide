# Đề bài: API còn thiếu cho thiết kế giao diện 2 (D-202)

Đề bài này giao cho một agent làm việc trong repo Autoxide. Đọc `CLAUDE.md` trước; mọi luật ở đó áp
dụng (cả SQLite và Postgres, pháp luật Việt Nam, module sở hữu bảng, gate). Bối cảnh: thiết kế giao diện 2
(D-200, D-201) đã chốt khung quản trị đa phân hệ và mặt tiền bán hàng; các hợp đồng API cho thiết kế đó
nằm ở bước 51, 52 và mục "Bổ sung theo D-200 / D-201" trong đặc tả bước 41–45 (`docs/steps/`).
Rà thiết kế lần cuối còn ra năm chỗ thiết kế dùng mà lộ trình chưa có API. Đề bài này liệt kê năm chỗ đó.

## Việc phải làm

1. Ghi quyết định **D-202** bằng skill `record-decision`: năm khoảng trống dưới đây, chọn cách nào, vì sao.
2. Thêm mục vào `docs/ROADMAP.md` và đặc tả `docs/steps/` đúng bước của từng phần (cột "Bước" bên dưới).
   Đặc tả viết đủ để agent làm bước đó không phải hỏi lại: bảng, endpoint, permission, mã lỗi, event,
   tool MCP, tên test.
3. **Làm code ngay** cho phần A và B, vì module của hai phần đó đã có (`comments`, `media`). Phần C, D, E
   chỉ ghi vào đặc tả; làm khi lộ trình tới bước đó, không nhảy cóc.
4. Phần làm code phải qua `./gate.sh` đầy đủ (cả hai database), sinh lại `docs/openapi.json`, thêm bản
   dịch `locales/vi.json` cho mọi `code` lỗi mới. Thay đổi API chỉ được **thêm**: không đổi hành vi hay
   hình dạng response của endpoint đang có (D-059).

| Phần | Nội dung | Bước | Làm code bây giờ? |
| ---- | -------- | ---- | ----------------- |
| A | Hỏi đáp sản phẩm (dựa trên `comments`) | 22 (`comments`) và 41 (`catalog`) | Có, phần hạ tầng trong `comments` |
| B | Mô tả ảnh (`alt`) theo ngôn ngữ | `media` (bước 16) | Có |
| C | Trả hàng kèm ảnh, xem trước tiền hoàn | 45 | Chỉ đặc tả |
| D | Thao tác trên đơn vãng lai sau khi tra cứu | 45 | Chỉ đặc tả |
| E | Nhật ký request ghi tên tool MCP | 52 | Chỉ đặc tả |

## A. Hỏi đáp sản phẩm

Thiết kế: trang sản phẩm có tab "Hỏi đáp (12)". Người mua đặt câu hỏi; câu hỏi chờ duyệt rồi mới hiện;
người bán trả lời với nhãn "Bếp Mộc" (tên cửa hàng), người đã mua trả lời với nhãn "Đã mua"; có ô tìm
trong các câu hỏi.

Hướng làm: **dùng lại module `comments`, không tạo module mới.** Câu hỏi là bình luận gốc, câu trả lời là
trả lời trong luồng. `comments` đã có đích chung (`target_type`, `target_id`), kiểm duyệt, đếm theo lô và
luồng.

- **Đích `catalog.product`.** Bước 41 đăng ký `catalog.product` vào sổ đối tượng `ResourceDirectory`
  (D-123), như `content` đã làm với `content.entry`, để `comments` kiểm sản phẩm có tồn tại và đang công
  khai không. Event "lưu trữ / xoá sản phẩm" phải đóng và ẩn hỏi đáp. Hiện `targets::OWNER_EVENTS` là
  hằng số viết cứng cho `content`. Đổi thành dữ liệu do module sở hữu đích khai báo (một hàm mới của
  `Module`, có cài đặt mặc định, D-038), để thêm loại đích mới không phải sửa `comments`.
- **Nhãn tác giả.** `comments` không được phụ thuộc `orders` hay `catalog`. Thêm vào kernel một cơ chế
  trung lập, theo mẫu `ResourceDirectory`: module đăng ký bộ tính nhãn cho một loại đích. Nhãn gồm:
  - "người bán": tác giả có permission quản lý sản phẩm ở nút tổ chức sở hữu sản phẩm;
  - "đã mua": `orders` trả lời qua hàm có sẵn trong đặc tả 45 ("người mua đã nhận dòng hàng chưa").

  Nhãn tính lúc ghi rồi lưu cùng bình luận (đường đọc không gọi chéo module). Nhãn hiện trong response
  công khai dưới dạng mã ổn định (`seller`, `verified_buyer`), không phải câu chữ.
- **Chỉ tài khoản đã xác thực mới được hỏi và trả lời.** Đây là yêu cầu của NĐ 147/2024 về xác thực tài
  khoản đăng nội dung, ở mục bước 37 của `docs/ROADMAP.md`. Nếu bước 37 chưa có, khai điểm nối dạng trait,
  mặc định giữ hành vi hiện tại, và ghi rõ trong D-202.
- **Tìm trong hỏi đáp của một đích.** Thêm tham số `q` cho danh sách công khai theo đích: không phân biệt
  dấu, khớp đúng dấu xếp trước, như quy ước tìm kiếm của bước 51. Phân trang bằng `fetch_page`.
- **Số đếm cho tab.** Dùng số đếm theo lô sẵn có. Tách số câu hỏi đã công khai khỏi số chờ duyệt.
- **Tool MCP.** Thêm tool liệt kê câu hỏi chưa trả lời và tool trả lời câu hỏi. Mỗi tool gắn đúng
  permission như REST và gọi cùng service.
- **Test.** Chạy trên cả hai database. Các test tối thiểu:
  - `cau_hoi_chi_hien_sau_khi_duyet`
  - `tra_loi_cua_nguoi_ban_co_nhan_seller`
  - `nguoi_chua_mua_khong_co_nhan_verified_buyer`
  - `tim_cau_hoi_khong_phan_biet_dau`
  - `luu_tru_dich_thi_dong_va_an_hoi_dap`
  - `tai_khoan_chua_xac_thuc_khong_hoi_duoc`

  Thêm test đồng thời: hai câu trả lời cùng lúc không làm lệch số đếm.

## B. Mô tả ảnh (`alt`) theo ngôn ngữ

Thiết kế: mặt tiền có hai ngôn ngữ (Tiếng Việt / English). Hiện `media` chỉ có một chuỗi `alt`
(`modules/media/src/api.rs`), và `catalog_product_media.alt` (đặc tả 41) cũng chỉ có một chuỗi.

- **Cách lưu.** Thêm bảng `media_alt_translations` (`tenant_id` nếu bảng media đã theo khách, `media_id`,
  `locale`, `alt`, `updated_at`; khoá (`media_id`, `locale`)) bằng **migration mới**, không sửa
  migration cũ. `alt` hiện có giữ nguyên nghĩa là bản của ngôn ngữ mặc định.
- **API (chỉ thêm).**
  - Đọc: response thêm trường `alts` (bản đồ `locale → alt`).
  - Ghi: endpoint đặt hoặc xoá alt của một ngôn ngữ.
  - Phục vụ công khai: chọn theo ngôn ngữ của request, không có thì lùi về `alt` mặc định.
  - Mã ngôn ngữ kiểm theo danh sách ngôn ngữ cấu hình; sai thì báo lỗi có `code` riêng.
- **Gợi ý bằng AI.** Gợi ý alt theo từng ngôn ngữ chỉ là bản nháp người duyệt (D-109). Gọi qua `TextModel`
  và không gọi trong transaction ghi.
- **Bước 41.** Ghi vào đặc tả: `catalog_product_media` cho ghi đè alt theo ngôn ngữ theo cùng cách.
- **Test.** Lùi về mặc định khi thiếu bản dịch; mã ngôn ngữ sai bị từ chối; xoá ảnh thì xoá bản dịch.
  Chạy trên cả hai database.

## C. Trả hàng kèm ảnh, xem trước tiền hoàn (đặc tả bước 45)

Thiết kế: luồng ba bước.

1. Chọn sản phẩm.
2. Chọn lý do và mô tả. Ảnh **bắt buộc** khi lý do là "lỗi / vỡ" hoặc "giao sai".
3. Chọn cách gửi trả và xem số tiền hoàn cùng thời gian hoàn.

Đặc tả 45 đã có `orders_returns` và `return_method`, nhưng còn thiếu:

- **Ảnh kèm yêu cầu.** Thêm cột `media_ids` vào `orders_returns`. Ảnh phải do chính người mua tải lên;
  kiểm qua trait của `media`, không đọc bảng của `media`.
- **Lý do nào bắt buộc ảnh.** Là chính sách theo nút tổ chức (`orders_policies`, kế thừa theo cây), không
  viết cứng trong code. Thiếu ảnh thì báo lỗi có `code` riêng, gom chung với các lỗi khác (D-083).
- **Xem trước.** Thêm `POST /me/orders/{id}/returns/preview`. Endpoint trả về:
  - số tiền hoàn của các dòng đã chọn;
  - cách gửi trả được phép và phí của từng cách;
  - thời gian hoàn dự kiến theo phương thức thanh toán;
  - hạn đổi trả.

  Endpoint không ghi gì. Tiền là số nguyên cộng mã tiền tệ.

## D. Thao tác trên đơn vãng lai sau khi tra cứu (đặc tả bước 45)

Đặc tả 45 đã có `POST /public/orders/lookup {number, phone}` → OTP → token xem đơn, và
`POST /me/orders/claim`. Thiết kế còn dùng những thứ sau:

- **Thao tác bằng token xem đơn.**
  - Huỷ đơn theo cùng chính sách huỷ của người mua có tài khoản.
  - Đổi địa chỉ giao trước khi hàng rời kho. Đổi địa chỉ cần OTP lần nữa.
  - Mở hỗ trợ kèm sẵn mã đơn.

  Mỗi thao tác đi qua cùng service như đường có tài khoản và nhận `Idempotency-Key`.
- **Giới hạn.**
  - Throttle số lần tra theo cặp (mã đơn, số điện thoại) và theo IP, qua công cụ throttle sẵn có.
  - Gửi lại OTP phải chờ 60 giây.
  - Token xem đơn sống 30 phút.

  Ba con số này là cấu hình, có mặc định hợp lý.
- **Bảo vệ dữ liệu cá nhân.** Response cho token xem đơn che số điện thoại (`090•••567`) và chỉ hiện địa
  chỉ tới cấp phường. Ghi căn cứ: Luật Bảo vệ dữ liệu cá nhân 2025, NĐ 13/2023; cần đối chiếu văn bản gốc.
- **Tạo tài khoản từ số đã xác thực.** Ngay trên trang xem đơn, tạo tài khoản bằng số điện thoại vừa xác
  thực (đăng nhập bằng passkey hay OTP, không bắt buộc mật khẩu), rồi gọi `claim` để gộp đơn. Ghi rõ phần
  này thuộc `auth` (bước 34) hay `orders` (bước 45).

## E. Nhật ký request ghi tên tool MCP (đặc tả bước 52)

Thiết kế: trang "API key và AI agent" có nhật ký lệnh của agent. Mỗi dòng gồm tên tool, key, kết quả, và
liên kết tới yêu cầu duyệt nếu có. Hiện nhật ký request của bước 52 chỉ ghi đường dẫn mẫu, nên mọi lệnh
MCP đều hiện là một đường `/mcp`.

- Lệnh gọi MCP ghi `mcp:<tên tool>` vào cột đường dẫn mẫu. Không ghi tham số.
- Lọc theo `actor_kind` (P12 bước 51) và theo key.
- Lệnh nhận 202 `approval_required` thì dòng nhật ký giữ `approval_id`, để giao diện nối tới yêu cầu
  duyệt.
- Test: lệnh MCP ghi đúng tên tool; tham số không lọt vào nhật ký.

## Báo cáo khi xong

Báo cáo bằng tiếng Việt, lời thường, cho chủ dự án mới học Rust. Nội dung báo cáo:

- đã thêm gì;
- gate xanh trên cả hai database chưa;
- mục nào đã vào lộ trình và nằm ở bước mấy;
- chỗ nào ghi "cần đối chiếu văn bản gốc".

Commit trên nhánh làm việc. Không push khi chủ dự án chưa yêu cầu, trừ khi `docs/RELEASING.md` bảo phải
phát hành.
