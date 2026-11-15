# Chỗ thiếu API khi dựng giao diện thật (`autoxide-web`)

File sống, cho team API. Mỗi lần dựng xong một màn của repo giao diện `autoxide-web`, agent giao diện thêm vào đây những
gì màn đó cần mà `docs/openapi.json` chưa có.

Luật của repo giao diện: gọi REST API công khai của core cho mọi thứ core đã có. Phần core chưa có API thì giao diện
dựng bằng **dữ liệu mẫu** (chủ dự án quyết ngày 15/11/2026, W-006 của `autoxide-web`):
- dữ liệu và hàm mẫu nằm gọn trong `apps/admin/src/sample/`, có kiểu theo đúng hình dạng dự kiến của API trong đặc tả bước;
- trên màn có nhãn "Dữ liệu mẫu · chờ API bước N";
- tắt mặc định ở production.

Khi team API làm xong một dòng, giao diện thay hàm mẫu bằng lời gọi thật. Nếu API thật khác hình dạng đã đoán, sửa
giao diện theo API, không sửa API theo dữ liệu mẫu.

File này khác các đề bài trước:
- `ui-api-gaps.md`, `ui-phase2-api.md`, `ui-phase3-api.md` đối chiếu **bản thiết kế** với lộ trình.
- File này ghi chỗ thiếu gặp khi **dựng thật** với core đang chạy.

Chỗ nào đã nằm trong lộ trình thì chỉ ghi số bước để biết giao diện đang chờ gì, không phải việc mới.

## Cách đọc

| Cột | Nghĩa |
|---|---|
| Màn | Màn của `autoxide-web` (đường dẫn) |
| Cần | Endpoint hay trường còn thiếu, và giao diện dùng nó vào việc gì |
| Bước | Bước của `docs/ROADMAP.md` đã có mục này; "mới" là chưa có ở đâu |
| Giao diện đang làm gì | Cách tạm thời trong lúc chờ |

Team API làm xong một dòng thì đánh dấu `[x]` ở đầu dòng; agent giao diện sẽ bỏ phần tạm và nối API thật.

## Khung quản trị (đã dựng 15/11/2026)

| | Màn | Cần | Bước | Giao diện đang làm gì |
|---|---|---|---|---|
| [ ] | Mọi màn: thanh bên, menu con, bảng lệnh | Sổ điều hướng: nhóm phân hệ, màn, permission, tính năng của gói, thứ tự; đã lọc theo người gọi | 51 (`Module::navigation`) | Danh sách viết ở `apps/admin/src/nav/nav.ts`, lọc theo `GET /kernel/modules` và `permissions` của `GET /auth/me`. Có API thì bỏ file này |
| [ ] | Mọi màn: số đếm trên menu | Số việc đang chờ cho từng mục menu, cùng nguồn với hộp việc | 51 (hộp việc) | Chưa hiện số |
| [ ] | Ctrl K | Tìm dữ liệu xuyên phân hệ (đơn, bài viết, người dùng…), tìm không dấu, theo quyền | 51 (`FinderSource`) | Chỉ tìm tên màn để "Đi tới" |
| [ ] | `/` Tổng quan | Hộp việc, số liệu tóm tắt, hoạt động gần đây | 51 | Chỉ lời chào và lối vào các phân hệ |
| [ ] | Góc trên bên trái | Khách hàng và nút tổ chức đang chọn, đổi phạm vi | 33, 34 | Hiện "Autoxide Quản trị", chưa đổi phạm vi được |
| [ ] | Menu tài khoản: giao diện sáng / tối | Lưu lựa chọn theo tài khoản để theo người dùng sang thiết bị khác. Nên gộp vào `/me/preferences` của bước 37 | mới (gộp vào 37) | Lưu cookie `ax_theme` trên từng trình duyệt |

## Tài khoản của tôi (`/account`)

| | Màn | Cần | Bước | Giao diện đang làm gì |
|---|---|---|---|---|
| [ ] | Hồ sơ | `PATCH /me` đổi tên hiển thị | 34 (đã ghi ở `ui-phase2-api.md`) | Chỉ hiện tên |
| [ ] | Hồ sơ | Ngôn ngữ ưa thích (thư và thông báo gửi chậm dùng ngôn ngữ này) | 37 (`/me/preferences`) | Ngôn ngữ giao diện lưu ở cookie |
| [ ] | Thông báo | Tuỳ chọn nhận thông báo theo loại và kênh | 37 (`/me/preferences`), 52 | Chưa dựng tab này |
| [ ] | Hồ sơ | "Nơi bạn làm việc": khách hàng và nhánh tổ chức bạn vào được, kèm cách vào (trực tiếp, uỷ quyền, vào để hỗ trợ) | 34 | Chưa dựng |
| [ ] | Đăng nhập và bảo mật | Tài khoản Google / OIDC đã liên kết: xem và gỡ | 40 | Chưa dựng |
| [ ] | Đăng nhập và bảo mật | `/me` trả `has_password`; endpoint để tài khoản chưa có mật khẩu (vd đăng nhập bằng passkey hay OIDC) tự đặt mật khẩu. `DELETE /me` bảo "đặt mật khẩu trước" nhưng chưa có cách đặt | mới | Luôn hiện "Đổi mật khẩu" |
| [ ] | Hồ sơ | Email đang chờ xác minh sau khi yêu cầu đổi (`pending_email`, hạn), và endpoint huỷ yêu cầu đổi | mới | Chỉ báo "đã gửi thư tới …" |
| [ ] | Phiên đăng nhập | Mỗi phiên trả cách đăng nhập (mật khẩu, mật khẩu + mã, passkey) và IP hay vị trí gần đúng (lưu theo luật dữ liệu cá nhân, có hạn) | mới | Chỉ hiện trình duyệt và hệ điều hành (giao diện đã chuyển `User-Agent` thật cho core) |
| [ ] | Đăng nhập và bảo mật | Passkey trả loại authenticator (AAGUID → tên như Touch ID, Windows Hello) | mới | Hiện tên người dùng tự đặt |
| [ ] | Mọi biểu mẫu có mật khẩu | `detail` của `auth.wrong_password` đang nhắc `POST /api/v1/auth/password-reset`: câu cho người gọi API, người dùng thường không hiểu. Viết câu cho người dùng; đưa hướng dẫn cho máy vào tài liệu OpenAPI | mới | Hiện nguyên câu của core |

## Tổ chức › Người dùng và role (`/org/access`)

| | Màn | Cần | Bước | Giao diện đang làm gì |
|---|---|---|---|---|
| [ ] | Người dùng | `GET /auth/users` trả kèm role của từng người (hoặc `include=roles`) | mới | Gọi `GET /users/{id}/roles` cho từng dòng, mỗi trang 25 người |
| [ ] | Người dùng | Lọc theo trạng thái (đang dùng / đã khoá) và theo role | mới (lọc theo role: 34, `GET /memberships?role_id=`) | Chỉ tìm theo email hoặc tên |
| [ ] | Chi tiết người dùng | `UserResponse` có `has_password`, `mfa_enabled`, lần đăng nhập cuối | mới | Luôn hiện "Đặt mật khẩu đầu tiên" và "Tắt xác thực hai bước"; đặt lần hai thì hiện lỗi `auth.password_already_set` |
| [ ] | Role | Số người giữ mỗi role, "xem người giữ role" | 34 (`GET /memberships?role_id=`); số đếm: mới | Chưa có cột này |
| [ ] | Chi tiết role | Mô tả của permission theo `Accept-Language` | mới | Hiện mô tả tiếng Anh cả khi chọn tiếng Việt |
| [ ] | Chi tiết role | Độ sâu theo nút (`grants: [{permission, scope}]`, lưới permission × độ sâu), membership theo nút | 34 | Chưa dựng; chỉ chọn permission ở phạm vi toàn khách |
| [ ] | `/verify-email`, `/reset-password` | Mã lỗi tách "hết hạn" và "đã dùng" (hiện chung `auth.invalid_*_token`) | mới | Hiện `detail` của core kèm cách xin link mới |
| [ ] | Mọi form quản trị người dùng | `detail` của `auth.password_already_set`, `auth.invalid_verification_token` đang nhắc tên endpoint (câu cho máy) | mới (cùng loại với `auth.wrong_password`) | Hiện nguyên câu của core |
| [ ] | Lời mời | `GET/POST /auth/invitations`, `DELETE /{id}`, `POST /{id}/resend`; trả kèm tên role và người mời | 34 | Dữ liệu mẫu (`src/sample/org.ts`) |
| [ ] | Uỷ quyền | `GET/POST/DELETE /auth/delegations`; danh sách khách con | 34 (khách con: 33) | Dữ liệu mẫu (`src/sample/org.ts`) |
| [ ] | Hồ sơ công khai | Hồ sơ công khai của nút (`PATCH /organizations/units/{id}`), giờ mở cửa theo ngày và ngày nghỉ, danh mục tỉnh / xã | 34 (D-200, `address`) | Dữ liệu mẫu (`src/sample/org.ts`); cùng một giờ cho mọi ngày |
| [ ] | Quyền vào hỗ trợ | `GET /auth/support-access`, cài đặt `auth.support_access` | 34 | Dữ liệu mẫu (`src/sample/org.ts`) |
| [ ] | Quyền vào hỗ trợ | Khách tự thu hồi một lần cấp quyền vào hỗ trợ (bước 34 chỉ cho nhà vận hành thu hồi) | mới | Dữ liệu mẫu (`src/sample/org.ts`), ghi "theo API đề xuất" |

## Nội dung › Bài viết và trình soạn bài (`/content`)

| | Màn | Cần | Bước | Giao diện đang làm gì |
|---|---|---|---|---|
| [ ] | Danh sách | Lọc "chờ duyệt" (`review_requested=true`) và "đã lên lịch" trong `GET /content/entries` | mới | Tab chỉ có Tất cả / Bản nháp / Đã đăng / Đã lưu trữ; nhãn "Chờ duyệt" hiện trên từng dòng |
| [ ] | Danh sách | Số bài của từng tab trạng thái | 51 (số đếm) | Chưa hiện số |
| [ ] | Danh sách | Lọc "Bài của tôi" (`created_by=me`) và "thiếu bản dịch ngôn ngữ X" | mới | Chưa dựng |
| [ ] | Danh sách | "Sắp đăng": lịch đăng của mọi bài (`GET /content/schedules?status=scheduled`) | mới | Chỉ xem lịch trong từng bài |
| [ ] | Danh sách, trình soạn | Cấu hình cho giao diện: danh sách ngôn ngữ (`AUTOXIDE_CONTENT_LOCALES`), đầu đường dẫn bài viết, host được nhúng | mới | Dùng giá trị mặc định của core: en/vi, `/blog/`, YouTube/Vimeo |
| [ ] | Danh sách, trình soạn | `published_at` trong `EntryResponse` của trang quản trị | mới | Cột ngày dùng `updated_at` |
| [ ] | Danh sách | Website theo tên miền | 37 (`GET /compliance/websites`) | Dữ liệu mẫu (`src/sample/content.ts`) |
| [ ] | Danh sách | Trang bắt buộc của website, tạo từ mẫu | 37 (`…/websites/{id}/checklist`, `POST /content/templates/{key}/drafts`) | Dữ liệu mẫu (`src/sample/content.ts`) |
| [ ] | Danh sách, trình soạn | Bài thuộc website / nút sở hữu nào, lọc theo nút | mới (bước 34 giữ `content` ở cấp khách) | Dữ liệu mẫu (`src/sample/content.ts`) |
| [ ] | Xem trước | Địa chỉ website của khách, để link xem trước trỏ về website thật | 33, 37 | Trang `/preview` của chính trang quản trị |
| [ ] | Xem trước | Bản xem trước trả kèm địa chỉ ảnh (hiện chỉ có id media) | mới | Ảnh hiện thành ô giữ chỗ |
| [ ] | Trình soạn | Lịch đăng mang theo `ai_reviewed` | mới | Hẹn giờ không gửi được dấu "đã duyệt" |
| [ ] | Trình soạn | Kết quả kiểm SEO trả `message` theo `Accept-Language` | mới | Giao diện tự dịch theo `code`; mã lạ thì hiện câu tiếng Anh |
| [ ] | Trình soạn | Tìm tác giả và danh mục theo chữ (hiện chỉ lấy một trang 200) | mới | Lấy 200 mục đầu |

## Cài đặt (`/settings`, `/settings/api-keys`, `/settings/developers`)

| | Màn | Cần | Bước | Giao diện đang làm gì |
|---|---|---|---|---|
| [ ] | Gói và thuê bao | `GET /plans/subscription`, `/entitlements` (hạn mức kèm `used`, `over_limit`), `/catalog`, `/billing-periods`, `GET/PUT /billing-profile`, `POST /subscription/change`, `/cancel`, `/resume`; trả kèm tên gói và giá | 35 | Dữ liệu mẫu (`src/sample/settings.ts`) |
| [ ] | Gói và thuê bao | `POST /billing-periods/{id}/pay` (trả tiền qua cổng của nhà vận hành) | 35 | Chưa có nút "Trả tiền" |
| [ ] | Tên miền | `GET/POST/DELETE /kernel/domains`, `POST /{id}/verify` | 33 | Dữ liệu mẫu (`src/sample/settings.ts`) |
| [ ] | Tên miền | Gắn tên miền với nút: `PATCH /kernel/domains/{id} {org_id}` | 33, 34 | Chỉ hiện nút, chưa đổi được |
| [ ] | Kết nối đối tác | `GET /kernel/settings`, `PUT/DELETE /settings/{name}`; tên setting theo nhà cung cấp (SMS, cổng thanh toán, hoá đơn điện tử) | 33 (tên setting: 36, 39) | Dữ liệu mẫu (`src/sample/settings.ts`) |
| [ ] | Kết nối đối tác | `POST /kernel/settings/{name}/test`, `last_used_at`, `last_error` | mới (đề xuất ở `ui-trace-roadmap.md`, chưa vào lộ trình) | Dữ liệu mẫu |
| [ ] | Kết nối đối tác | "Địa chỉ công khai" dùng trong link email, theo khách | mới | Chưa dựng |
| [ ] | Job | `GET /kernel/jobs?status=` (theo khách, tóm tắt, tiến độ, người yêu cầu), `POST /jobs/{id}/retry` có audit | 52 | Dữ liệu mẫu (`src/sample/settings.ts`); core hiện chỉ có lệnh CLI `autoxide jobs` |
| [ ] | Webhook | `EndpointResponse` trả lần giao gần nhất (trạng thái, lúc, HTTP); lọc endpoint theo trạng thái | mới | Gọi `/deliveries?limit=1` cho từng endpoint; lọc trên trang đang xem |
| [ ] | Webhook | Mô tả event theo `Accept-Language` | mới | Hiện tiếng Anh cả khi chọn tiếng Việt |
| [ ] | Nhà phát triển › Webhook | Danh sách lần giao xuyên endpoint có lọc; gửi lại một lần giao | 52 | Gộp `/webhooks/{id}/deliveries` (25 lần mỗi endpoint); chưa có "Gửi lại" |
| [ ] | Nhật ký audit | `actor_kind` (người / agent / dịch vụ / hỗ trợ nền tảng) và tham số lọc `actor_kind=` | 51, 52 | Chưa có chip lọc theo loại người làm |
| [ ] | Nhật ký audit | Mục audit trả kèm tên người làm (hoặc `include=actor`) | mới | Gọi `GET /auth/users/{id}` cho mỗi người trên trang; không có quyền thì hiện mã rút gọn |
| [ ] | API key | `kind` của key (người, agent, dịch vụ) | 51 | Chưa hiện loại key |
| [ ] | API key | Quản trị viên xem và thu hồi key của mọi người trong khách | mới | Chỉ key của chính mình |
| [ ] | API key | Xác nhận lại bằng passkey hoặc TOTP thay cho mật khẩu khi tạo key | mới | Chỉ mật khẩu |
| [ ] | Tạo API key | Sổ permission đọc được khi không có `auth.roles.read` (vd `GET /me/permissions` kèm mô tả) | mới | Chỉ thấy tên permission, không có mô tả |
| [ ] | Chờ duyệt | `GET /kernel/approvals`, `POST /approvals/{id}/approve`, `/reject` | 51 | Dữ liệu mẫu (`src/sample/agent.ts`) |
| [ ] | Nhật ký gọi tool MCP | Nhật ký các lần agent gọi tool | 51, 52 | Dữ liệu mẫu (`src/sample/agent.ts`) |
| [ ] | Kết nối MCP | Core công bố địa chỉ công khai (`AUTOXIDE_HTTP_PUBLIC_URL`) | mới | Dùng `AUTOXIDE_API_URL` của giao diện |
| [ ] | Nhà phát triển › Request | `GET /kernel/requests?status_class=&api_key_id=` | 52 | Dữ liệu mẫu (`src/sample/devtools.ts`) |
| [ ] | Nhà phát triển › Event theo bản ghi | `GET /kernel/records/{kind}/{id}/events` | 52 | Dữ liệu mẫu (`src/sample/devtools.ts`) |

## Tiếp thị › Biểu mẫu, Bản tin, Bình luận (`/marketing/...`)

| | Màn | Cần | Bước | Giao diện đang làm gì |
|---|---|---|---|---|
| [ ] | Biểu mẫu | `FormResponse` kèm số bài nộp theo trạng thái (cột "Bài mới", số trên chip lọc) | mới (số đếm menu: 51) | Gọi `GET /forms/{id}/submissions?status=new&limit=100` cho từng form, hiện "100+" |
| [ ] | Bài nộp | Xuất CSV lớn bằng job, trả link tải có hạn | 32 | Xuất ngay qua route của giao diện |
| [ ] | Bài nộp | `AnswerResponse` kèm nhãn lựa chọn của đúng phiên bản form lúc gửi; kiểu `value` trong `openapi.json` đang là object | mới | Lấy nhãn từ form hiện tại; lựa chọn đã bỏ thì hiện giá trị thô |
| [ ] | Bản tin | `GET /newsletter/settings`: nhãn quảng cáo, giới hạn thư / ngày, tốc độ gửi, đã đặt URL công khai / trang chọn danh sách / địa chỉ người gửi chưa | mới | Hiện mặc định `[QC]`, 3 thư / ngày; thiếu cấu hình thì hiện lỗi 503 của core khi gửi |
| [ ] | Chiến dịch | Xem trước thư đúng như khi gửi (kèm chân thư): `GET /campaigns/{id}/preview` | mới | Xem trước HTML của người soạn; chân thư mô tả bằng chữ |
| [ ] | Chiến dịch | Số huỷ đăng ký / bounce sau thư này | mới | Không hiện |
| [ ] | Danh sách nhận tin | Tổng số đăng ký và số địa chỉ không gửi của cả khách | 51 (số liệu tóm tắt) | Cộng các danh sách ở trang đầu (≤ 50) |
| [ ] | Người đăng ký | Lọc theo trạng thái người đăng ký (`blocked`); tham số `status` hiện lọc theo trạng thái đăng ký | mới | Không có chip "Bị chặn"; nhãn hiện trên dòng |
| [ ] | Bình luận | `CommentDetail` kèm tiêu đề và link của đích (sổ đối tượng D-123) | mới | `content.entry`: đọc bài qua API `content`; loại khác hiện id |
| [ ] | Bình luận › Theo bài | `GET /comments/targets`: danh sách đích có số đếm, lọc theo trạng thái | mới | Lấy đích từ 200 bình luận mới nhất |
| [ ] | Menu Bình luận, Biểu mẫu | Số việc đang chờ (bình luận chờ duyệt, bài nộp mới) | 51 (hộp việc) | Chưa hiện số |
| [ ] | Bình luận › Cài đặt | Cài đặt bình luận theo khách (bật / tắt, chế độ duyệt) | 33 | Dữ liệu mẫu (`src/sample/engage.ts`) |
| [ ] | Bình luận › Cài đặt | Chính sách `posting.comments` (xác thực tài khoản, NĐ 147/2024) | 37 | Dữ liệu mẫu (`src/sample/engage.ts`) |
| [ ] | Ba tab "Dữ liệu theo email" | Một yêu cầu của chủ thể dữ liệu gom mọi module, gắn với hồ sơ ở Tuân thủ | 37 | Ba tab riêng, mỗi module tra / xoá một lần |
