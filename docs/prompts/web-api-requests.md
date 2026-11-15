# Đề bài: API giao diện quản trị cần thêm hoặc sửa (đợt 1, sau core v0.14.0)

Đề bài này giao cho agent làm việc trong repo Autoxide. Đọc `CLAUDE.md` trước; mọi luật ở đó áp dụng
(cả SQLite và Postgres, module sở hữu bảng, OpenAPI đầy đủ, i18n, gate).

Bối cảnh: giao diện quản trị (`autoxide-web`) đã dựng đủ mọi màn của bản thiết kế. Màn nào core đã có
API thì gọi API thật; còn lại dùng dữ liệu mẫu có nhãn. Khi dựng, giao diện ghi lại từng chỗ core thiếu
vào bảng `docs/prompts/web-build-api-gaps.md` (mỗi dòng: màn, cần gì, bước nào, giao diện đang tạm làm
gì). Đề bài này rút từ bảng đó **những việc làm được ngay trên các module đã có** (`auth`, `kernel`,
`content`, `media`, `menus`, `forms`, `newsletter`, `comments`, `webhooks`, `audit`). Phần thuộc các bước
sau (33–49, 51) ghi ở mục E: chỉ cần đọc khi làm tới bước đó.

Đã kiểm trên `main` (sau bước 32) ngày 15/11/2026: mọi mục dưới đây vẫn còn thiếu.

## Việc phải làm

1. Ghi quyết định bằng skill `record-decision` (lấy số kế tiếp chưa dùng). Xếp các mục vào lộ trình:
   thêm một bước mới hay gộp vào bước sẵn có (vd 51, 52) — agent tự quyết, ghi lý do.
2. **Chỉ thêm, không phá vỡ** (D-059): thêm trường, tham số, endpoint mới; không đổi `code` lỗi đang có,
   không đổi hành vi endpoint cũ. Chỗ nào bắt buộc phải đổi thì làm endpoint mới và khai deprecation.
3. Mỗi mục có test, chạy trên cả hai database; sinh lại `docs/openapi.json`; thêm bản dịch
   `locales/vi.json` cho mọi i18n key mới. Gate đầy đủ (`./gate.sh`) xanh.
4. Làm xong một dòng nào thì đánh `[x]` ở dòng tương ứng trong `docs/prompts/web-build-api-gaps.md`, để
   agent giao diện biết mà bỏ phần tạm.
5. Thứ tự: A → B → C → D. Mục nào thấy không hợp với thiết kế của core thì không làm theo y nguyên: ghi lý
   do vào quyết định, đề xuất cách khác và đánh dấu dòng đó trong bảng.

## A. Sửa chỗ chưa phù hợp ở endpoint đang có

1. **`errors[].detail` của Problem dịch theo `Accept-Language`.** Câu `detail` chung đã dịch, nhưng câu
   của từng lỗi trong `errors[]` vẫn là tiếng Anh (vd lỗi `to_url` của chuyển hướng, lỗi mục menu). Giao
   diện hiện nguyên câu của core dưới ô, nên người dùng tiếng Việt thấy tiếng Anh. Áp cho mọi module, kèm
   một bài kiểm trong gate: mỗi `code` trong `errors[]` có bản dịch.
2. **Câu `detail` viết cho người dùng, không nhắc tên endpoint.** `auth.wrong_password` nhắc
   `POST /api/v1/auth/password-reset`; `auth.password_already_set`, `auth.invalid_verification_token`
   cũng vậy. Câu cho máy (gọi endpoint nào) chuyển vào mô tả response trong OpenAPI; `detail` nói người
   dùng phải làm gì.
3. **Token hết hạn khác token đã dùng.** `auth.invalid_*_token` gộp hai trường hợp, nên trang
   `/verify-email`, `/reset-password` không nói được "link đã hết hạn, xin link mới" hay "link đã dùng
   rồi". Giữ nguyên `code` (là hợp đồng), thêm trường mở rộng của Problem `reason: "expired" | "used" |
   "unknown"`.
4. **Mô tả dịch theo `Accept-Language`**: mô tả permission (`GET /auth/permissions`, sổ module) và mô tả
   event (webhook). Hiện luôn là tiếng Anh.
5. **`risk` trong `GET /auth/permissions`.** Trường này chỉ có ở `GET /kernel/modules` (cần
   `kernel.modules.read`), nên bộ chọn permission phải gọi thêm sổ module. Thêm `risk` vào
   `PermissionResponse`. Kèm `GET /api/v1/me/permissions`: permission của chính người gọi, có mô tả và
   `risk`, để người tạo API key mà không có `auth.roles.read` vẫn thấy mô tả.
6. **`menus`:**
   - `POST /menus` trả `ETag` như `GET` và `PUT` (giao diện đang phải đọc lại menu ngay sau khi tạo);
   - `PUT /menus/{id}` giữ id của mục: nhận `id` tuỳ chọn trong `SiteMenuItemInput`, mục có `id` cũ thì
     giữ id đó. Hiện mỗi lần lưu mọi mục nhận id mới, nên giao diện và nhật ký không đối chiếu được từng
     mục;
   - danh sách kèm `item_count`, người và lúc sửa cuối.

## B. Trường và bộ lọc thiếu khiến giao diện phải gọi nhiều lần

Mỗi mục dưới đây đang buộc giao diện gọi một request cho từng dòng (N+1), hoặc lọc trên trang đang xem
thay vì cả danh sách. Đây là việc của tiêu chí "Nhanh".

1. **`auth`:**
   - `GET /auth/users`: `include=roles` (role của từng người), lọc `status` (đang dùng / đã khoá);
   - `UserResponse`: `has_password`, `mfa_enabled`, `last_sign_in_at`. Giao diện đang luôn hiện "Đặt mật
     khẩu đầu tiên" và "Tắt xác thực hai bước", bấm sai thì mới thấy lỗi;
   - `MeResponse`: `has_password`, `pending_email` (kèm hạn) và endpoint huỷ yêu cầu đổi email;
   - tài khoản chưa có mật khẩu (vào bằng passkey) tự đặt mật khẩu. `DELETE /me` bảo "đặt mật khẩu
     trước", nhưng chưa có cách nào để đặt.
2. **`audit`:** `include=actor` trả tên người làm. Hiện giao diện gọi `GET /auth/users/{id}` cho từng
   người trên trang.
3. **`webhooks`:** `EndpointResponse` kèm lần giao gần nhất (trạng thái, lúc, mã HTTP); lọc endpoint theo
   trạng thái.
4. **`content`:**
   - `GET /content/entries`: lọc `review_requested=true`, `scheduled=true`, `created_by=me`,
     `ai_generated=true`, `missing_locale=<ngôn ngữ>`. Dữ liệu đã có (`review_requested_at`, `created_by`,
     `ai_generated`), chỉ thiếu tham số;
   - `EntryResponse`: `published_at`, `path` (đường dẫn đầy đủ, kèm tiền tố ngôn ngữ), `version` (hay
     ETag) để sửa từ danh sách mà không đọc lại;
   - `entry_count` cho danh mục, thẻ, tác giả; tìm danh mục và tác giả theo chữ (`q`);
   - `GET /content/schedules?status=scheduled`: lịch đăng của mọi bài;
   - bản xem trước trả kèm địa chỉ ảnh; lịch đăng mang theo `ai_reviewed`; `message` của kết quả kiểm SEO
     theo `Accept-Language`.
5. **`media`:**
   - lọc `alt_missing=true`, kèm tóm tắt thư viện (tổng tệp, dung lượng, số ảnh thiếu alt);
   - `item_count` trong `FolderResponse`; tên người tải lên;
   - `GET /media/{id}/usages`: tệp đang được bài, trang, menu nào dùng (sổ đối tượng);
   - cờ `decorative` (ảnh trang trí: alt trống có chủ ý, khác "chưa có alt").
6. **`forms`:**
   - `FormResponse` kèm số bài nộp theo trạng thái (hiện gọi một lần cho mỗi form);
   - `AnswerResponse` kèm nhãn lựa chọn theo đúng phiên bản form lúc gửi;
   - sửa kiểu của `value` trong `openapi.json` (đang là object).
7. **`newsletter`:**
   - `GET /newsletter/settings`: nhãn quảng cáo, giới hạn thư mỗi ngày, đã đặt URL công khai và địa chỉ
     người gửi chưa;
   - `GET /campaigns/{id}/preview`: thư đúng như khi gửi, kèm chân thư;
   - số huỷ đăng ký và bounce của mỗi chiến dịch; lọc người đăng ký `blocked`.
8. **`comments`:** `CommentDetail` kèm tiêu đề và link của đích (qua sổ đối tượng D-123);
   `GET /comments/targets`: danh sách đích có số đếm, lọc theo trạng thái.

## C. Cấu hình cho giao diện

Giao diện đang phải đoán các giá trị mặc định của core, nên lệch ngay khi người vận hành đổi cấu hình.
Đề xuất **một** endpoint chỉ đọc, chỉ gồm giá trị không phải secret, vd `GET /api/v1/kernel/client-config`.
Mỗi module khai phần của mình qua một hàm mới của `Module`, có mặc định rỗng (D-038):

- `content`: ngôn ngữ, tiền tố đường dẫn bài viết, host được nhúng;
- `media`: ngôn ngữ alt (`AUTOXIDE_MEDIA_ALT_LOCALES`), dung lượng tối đa, loại tệp cho phép;
- `menus`: ngôn ngữ (`AUTOXIDE_MENUS_LOCALES`);
- `kernel`: địa chỉ công khai (`AUTOXIDE_HTTP_PUBLIC_URL`) cho trang kết nối MCP và link trong email.

Agent tự chọn tên và hình dạng; yêu cầu là có một chỗ đọc, mô tả đủ trong OpenAPI.

## D. Tài khoản và API key

- Mỗi phiên đăng nhập trả cách đăng nhập (mật khẩu, mật khẩu + mã, passkey) và vị trí gần đúng. Lưu theo
  luật bảo vệ dữ liệu cá nhân, có hạn lưu.
- Passkey trả tên loại authenticator (AAGUID → "Touch ID", "Windows Hello").
- Quản trị viên xem và thu hồi API key của mọi người trong khách hàng.
- Xác nhận lại bằng passkey hoặc TOTP (không chỉ mật khẩu) khi tạo API key.

## E. Thuộc các bước sau — đọc khi làm tới bước đó

Bảng `docs/prompts/web-build-api-gaps.md` có một mục cho mỗi phân hệ. Mỗi dòng ghi đúng thứ giao diện cần
và giao diện đang tạm làm gì. Khi làm một bước, đọc mục tương ứng và làm cùng bước đó:

| Bước | Mục trong bảng |
| --- | --- |
| 33, 35, 39 | Mặt điều khiển nền tảng |
| 34 | Tổ chức › Cây tổ chức; Người dùng và role (lời mời, uỷ quyền, quyền vào hỗ trợ) |
| 36, 38 | Tài chính › Kế toán |
| 37 | Tuân thủ; Hồ sơ và dữ liệu cá nhân |
| 41, 42 | Sản phẩm và giá; Tồn kho |
| 43, 44 | Khuyến mại; Vận chuyển |
| 45 | Đơn hàng; Cửa hàng |
| 46 | Sàn; Trung tâm người bán |
| 47, 48, 49 | Khách hàng; Kênh bán; Tiếp thị liên kết |
| 51 | Khung quản trị; Tổng quan và Việc (hộp việc, số đếm, hoãn / giao việc, số liệu, các bước thiết lập, hoạt động, sổ đối tượng) |

Nhiều dòng ở đó ghi "mới": đặc tả của bước chưa có thứ đó. Đọc trước khi viết đặc tả chi tiết của bước,
để thêm vào từ đầu thay vì sửa sau.

## Báo cáo khi xong

Báo cáo bằng tiếng Việt, lời thường, cho chủ dự án mới học Rust:

- đã làm những mục nào, mục nào chưa làm và vì sao;
- gate có xanh trên cả hai database không;
- các mục đã vào bước nào của lộ trình.

Commit trên nhánh làm việc. Không push khi chủ dự án chưa yêu cầu.
