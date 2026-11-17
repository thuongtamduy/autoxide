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

**Việc làm được ngay** trên các module đã có (sửa chỗ chưa phù hợp, thêm trường và bộ lọc, cấu hình cho giao diện)
đã gom thành đề bài [`web-api-requests.md`](web-api-requests.md). Bảng dưới vẫn là danh sách đầy đủ.

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
| [ ] | Mọi màn: số đếm trên menu | Số việc đang chờ cho từng mục menu, cùng nguồn với hộp việc | 51 (hộp việc) | Tự đếm từ 16 nguồn việc (`src/work/`, W-020), giữ 10 giây; xem mục Tổng quan và Việc ở cuối file |
| [ ] | Ctrl K | Tìm dữ liệu xuyên phân hệ (đơn, bài viết, người dùng…), tìm không dấu, theo quyền | 51 (`FinderSource`) | Chỉ tìm tên màn để "Đi tới" |
| [ ] | `/` Tổng quan | Hộp việc, số liệu tóm tắt, hoạt động gần đây | 51 | Hộp việc, số liệu, các bước thiết lập, doanh thu hôm nay, hoạt động (audit thật); phần chưa có API là dữ liệu mẫu (W-020) |
| [ ] | Góc trên bên trái | Khách hàng và nút tổ chức đang chọn, đổi phạm vi | 33, 34 | Hiện "Autoxide Quản trị", chưa đổi phạm vi được |
| [x] | Menu tài khoản: giao diện sáng / tối | Lưu lựa chọn theo tài khoản để theo người dùng sang thiết bị khác | v0.14.0 (`/me/flags`) | Cờ `ui.theme`, đặt lại cookie khi đăng nhập (W-024 của web) |
| [ ] | Nút "?" (trợ giúp theo trang) | Bài hướng dẫn theo trang (tiêu đề, tóm tắt, link) do core hay module khai, theo `Accept-Language` | mới | Danh sách viết cứng ở `src/help/pages.ts`, 8 trang |
| [ ] | Nút "?" › Có gì mới | Nguồn tin "Có gì mới" theo phiên bản core và module đang chạy | mới | Danh sách viết cứng ở `src/help/whats-new.ts`; mốc đã xem lưu ở `/me/flags` (`whatsnew.seen`) |
| [ ] | Nút ⓘ của số liệu | Trường `definition` của `GET /kernel/summary` (D-207) | 51 (P4) | Chưa có nút ⓘ cho số liệu; câu cách tính hiện dưới khối |
| [x] | Bộ chọn permission (role, API key) | `risk` trong `GET /auth/permissions` (hiện chỉ có ở `GET /kernel/modules`, cần thêm `kernel.modules.read`) | mới | Gọi thêm sổ module; thiếu quyền thì không có nhãn rủi ro |

## Tài khoản của tôi (`/account`)

| | Màn | Cần | Bước | Giao diện đang làm gì |
|---|---|---|---|---|
| [ ] | Hồ sơ | `PATCH /me` đổi tên hiển thị | 34 (đã ghi ở `ui-phase2-api.md`) | Chỉ hiện tên |
| [ ] | Hồ sơ | Ngôn ngữ ưa thích (thư và thông báo gửi chậm dùng ngôn ngữ này) | 37 (`/me/preferences`) | Ngôn ngữ giao diện lưu ở cookie |
| [ ] | Thông báo | Tuỳ chọn nhận thông báo theo loại và kênh | 37 (`/me/preferences`), 52 | Chưa dựng tab này |
| [ ] | Hồ sơ | "Nơi bạn làm việc": khách hàng và nhánh tổ chức bạn vào được, kèm cách vào (trực tiếp, uỷ quyền, vào để hỗ trợ) | 34 | Chưa dựng |
| [ ] | Đăng nhập và bảo mật | Tài khoản Google / OIDC đã liên kết: xem và gỡ | 40 | Chưa dựng |
| [x] | Đăng nhập và bảo mật | `/me` trả `has_password`; endpoint để tài khoản chưa có mật khẩu (vd đăng nhập bằng passkey hay OIDC) tự đặt mật khẩu. `DELETE /me` bảo "đặt mật khẩu trước" nhưng chưa có cách đặt | mới | Luôn hiện "Đổi mật khẩu" |
| [x] | Hồ sơ | Email đang chờ xác minh sau khi yêu cầu đổi (`pending_email`, hạn), và endpoint huỷ yêu cầu đổi | mới | Chỉ báo "đã gửi thư tới …" |
| [x] | Phiên đăng nhập | Mỗi phiên trả cách đăng nhập (mật khẩu, mật khẩu + mã, passkey) và IP hay vị trí gần đúng (lưu theo luật dữ liệu cá nhân, có hạn) | mới | Chỉ hiện trình duyệt và hệ điều hành (giao diện đã chuyển `User-Agent` thật cho core) |
| [x] | Đăng nhập và bảo mật | Passkey trả loại authenticator (AAGUID → tên như Touch ID, Windows Hello) | mới | Hiện tên người dùng tự đặt |
| [ ] | Đăng nhập và bảo mật | Thêm passkey: options của `POST /auth/me/passkeys/registration` đòi ép `credProtect` (`extensions.enforceCredentialProtectionPolicy: true`, `credentialProtectionPolicy: userVerificationRequired`). Authenticator ảo của Chromium từ chối (`NotAllowedError`); bỏ phần mở rộng thì tạo được. Cần thử trên thiết bị thật (Windows Hello, iCloud Keychain, Android): thiết bị không hỗ trợ `credProtect` sẽ không thêm được passkey. Đề xuất: `enforceCredentialProtectionPolicy: false` (vẫn xin, không ép), hoặc ghi rõ lý do ép trong quyết định | mới (gặp khi chạy thử 16/11) | Hiện "đã huỷ" của trình duyệt; đăng nhập bằng passkey (W-042 của web) đã nối và chạy với core thật |
| [x] | Mọi biểu mẫu có mật khẩu | `detail` của `auth.wrong_password` đang nhắc `POST /api/v1/auth/password-reset`: câu cho người gọi API, người dùng thường không hiểu. Viết câu cho người dùng; đưa hướng dẫn cho máy vào tài liệu OpenAPI | mới | Hiện nguyên câu của core |

## Tổ chức › Người dùng và role (`/org/access`)

| | Màn | Cần | Bước | Giao diện đang làm gì |
|---|---|---|---|---|
| [x] | Người dùng | `GET /auth/users` trả kèm role của từng người (hoặc `include=roles`) | mới | Gọi `GET /users/{id}/roles` cho từng dòng, mỗi trang 25 người |
| [ ] | Người dùng | Lọc theo trạng thái (đang dùng / đã khoá) và theo role | trạng thái: xong (`status=active|disabled`); theo role: 34 (`GET /memberships?role_id=`) | Chỉ tìm theo email hoặc tên |
| [x] | Chi tiết người dùng | `UserResponse` có `has_password`, `mfa_enabled`, lần đăng nhập cuối | mới | Luôn hiện "Đặt mật khẩu đầu tiên" và "Tắt xác thực hai bước"; đặt lần hai thì hiện lỗi `auth.password_already_set` |
| [ ] | Role | Số người giữ mỗi role, "xem người giữ role" | 34 (`GET /memberships?role_id=`); số đếm: mới | Chưa có cột này. Nhân bản role đã có (16/11, `GET` rồi `POST /auth/roles`) |
| [x] | Chi tiết role | Mô tả của permission theo `Accept-Language` | mới | Hiện mô tả tiếng Anh cả khi chọn tiếng Việt |
| [ ] | Chi tiết role | Độ sâu theo nút (`grants: [{permission, scope}]`, lưới permission × độ sâu), membership theo nút | 34 | Chưa dựng; chỉ chọn permission ở phạm vi toàn khách |
| [x] | `/verify-email`, `/reset-password` | Mã lỗi tách "hết hạn" và "đã dùng" (hiện chung `auth.invalid_*_token`) | mới | Hiện `detail` của core kèm cách xin link mới |
| [x] | Mọi form quản trị người dùng | `detail` của `auth.password_already_set`, `auth.invalid_verification_token` đang nhắc tên endpoint (câu cho máy) | mới (cùng loại với `auth.wrong_password`) | Hiện nguyên câu của core |
| [ ] | Lời mời | `GET/POST /auth/invitations`, `DELETE /{id}`, `POST /{id}/resend`; trả kèm tên role và người mời | 34 | Dữ liệu mẫu (`src/sample/org.ts`) |
| [ ] | Uỷ quyền | `GET/POST/DELETE /auth/delegations`; danh sách khách con | 34 (khách con: 33) | Dữ liệu mẫu (`src/sample/org.ts`) |
| [x] | Hồ sơ công khai | Hồ sơ công khai của nút (`PATCH /organizations/units/{id}`), giờ mở cửa theo ngày và ngày nghỉ, danh mục tỉnh / xã | 34 (D-200, `address`) | **Đã nối** (17/11, W-045 của web): `GET/PATCH …/units/{id}/public-profile`, trạng thái công khai theo `GET /public/stores`, tỉnh / xã từ `address`; form sửa một khung giờ chung cho mọi ngày |
| [ ] | Quyền vào hỗ trợ | `GET /auth/support-access`, cài đặt `auth.support_access` | 34 | Dữ liệu mẫu (`src/sample/org.ts`) |
| [ ] | Quyền vào hỗ trợ | Khách tự thu hồi một lần cấp quyền vào hỗ trợ (bước 34 chỉ cho nhà vận hành thu hồi) | mới | Dữ liệu mẫu (`src/sample/org.ts`), ghi "theo API đề xuất" |

## Nội dung › Bài viết và trình soạn bài (`/content`)

| | Màn | Cần | Bước | Giao diện đang làm gì |
|---|---|---|---|---|
| [x] | Danh sách | Lọc "chờ duyệt" (`review_requested=true`) và "đã lên lịch" trong `GET /content/entries` | mới | Tab chỉ có Tất cả / Bản nháp / Đã đăng / Đã lưu trữ; nhãn "Chờ duyệt" hiện trên từng dòng |
| [ ] | Danh sách | Số bài của từng tab trạng thái | 51 (số đếm) | Chưa hiện số |
| [x] | Danh sách | Lọc "Bài của tôi" (`created_by=me`) và "thiếu bản dịch ngôn ngữ X" | mới | Chưa dựng |
| [x] | Danh sách | "Sắp đăng": lịch đăng của mọi bài (`GET /content/schedules?status=scheduled`) | mới | Chỉ xem lịch trong từng bài |
| [x] | Danh sách, trình soạn | Cấu hình cho giao diện: danh sách ngôn ngữ (`AUTOXIDE_CONTENT_LOCALES`), đầu đường dẫn bài viết, host được nhúng | mới | Dùng giá trị mặc định của core: en/vi, `/blog/`, YouTube/Vimeo |
| [x] | Danh sách, trình soạn | `published_at` trong `EntryResponse` của trang quản trị | mới | Cột ngày dùng `updated_at` |
| [ ] | Danh sách | Website theo tên miền | 37 (`GET /compliance/websites`) | Dữ liệu mẫu (`src/sample/content.ts`) |
| [ ] | Danh sách | Trang bắt buộc của website, tạo từ mẫu | 37 (`…/websites/{id}/checklist`, `POST /content/templates/{key}/drafts`) | Dữ liệu mẫu (`src/sample/content.ts`) |
| [ ] | Danh sách, trình soạn | Bài thuộc website / nút sở hữu nào, lọc theo nút | mới (bước 34 giữ `content` ở cấp khách) | Dữ liệu mẫu (`src/sample/content.ts`) |
| [ ] | Xem trước | Địa chỉ website của khách, để link xem trước trỏ về website thật | 33, 37 | Trang `/preview` của chính trang quản trị |
| [x] | Xem trước | Bản xem trước trả kèm địa chỉ ảnh (hiện chỉ có id media) | mới | Ảnh hiện thành ô giữ chỗ |
| [x] | Trình soạn | Lịch đăng mang theo `ai_reviewed` | mới | Hẹn giờ không gửi được dấu "đã duyệt" |
| [x] | Trình soạn | Kết quả kiểm SEO trả `message` theo `Accept-Language` | mới | Giao diện tự dịch theo `code`; mã lạ thì hiện câu tiếng Anh |
| [x] | Trình soạn | Tìm tác giả và danh mục theo chữ (hiện chỉ lấy một trang 200) | mới | Lấy 200 mục đầu |

## Cài đặt (`/settings`, `/settings/api-keys`, `/settings/developers`)

| | Màn | Cần | Bước | Giao diện đang làm gì |
|---|---|---|---|---|
| [ ] | Gói và thuê bao | `GET /plans/subscription`, `/entitlements` (hạn mức kèm `used`, `over_limit`), `/catalog`, `/billing-periods`, `GET/PUT /billing-profile`, `POST /subscription/change`, `/cancel`, `/resume`; trả kèm tên gói và giá | 35 | Dữ liệu mẫu (`src/sample/settings.ts`) |
| [ ] | Gói và thuê bao | `POST /billing-periods/{id}/pay` (trả tiền qua cổng của nhà vận hành) | 35 | Chưa có nút "Trả tiền" |
| [x] | Tên miền | `GET/POST/DELETE /kernel/domains`, `POST /{id}/verify` | 33 | **Đã nối** (16/11, W-032): danh sách, thêm, bản ghi TXT, kiểm tra ngay, gỡ; chế độ một khách hiện lời giải thích |
| [ ] | Tên miền | Gắn tên miền với nút: `PATCH /kernel/domains/{id} {org_id}` | 33, 34 | Core đã có API (D-297) nhưng chỉ nhận `null` tới khi có cây tổ chức; giao diện hiện `org_id` khi có, form gắn chờ danh sách nút của bước 34 (W-037 của web) |
| [ ] | Kết nối đối tác | `GET /kernel/settings`, `PUT/DELETE /settings/{name}`; tên setting theo nhà cung cấp (SMS, cổng thanh toán, hoá đơn điện tử) | 33 (tên setting: 36, 39) | Đã nối `GET/PUT/DELETE /kernel/settings` (W-027 của web); tên setting theo nhà cung cấp chờ bước 36, 39. Cùng component nay dùng cho `GET/PUT/DELETE /kernel/tenants/{id}/settings` ở trang chi tiết khách của nền tảng (D-296, W-035 của web). Nhóm `connection` (D-297) hiện thành khối riêng "Kết nối đối tác"; bản mẫu Email đã bỏ vì core có `mail.connection.smtp.…` (W-037 của web) |
| [ ] | Kết nối đối tác | `POST /kernel/settings/{name}/test`, `last_used_at`, `last_error` | mới (đề xuất ở `ui-trace-roadmap.md`, chưa vào lộ trình) | Dữ liệu mẫu |
| [ ] | Kết nối đối tác | "Địa chỉ công khai" dùng trong link email, theo khách | mới | Chưa dựng |
| [ ] | Job | `GET /kernel/jobs?status=` (theo khách, tóm tắt, tiến độ, người yêu cầu), `POST /jobs/{id}/retry` có audit | 52 | Dữ liệu mẫu (`src/sample/settings.ts`); core hiện chỉ có lệnh CLI `autoxide jobs` |
| [x] | Webhook | `EndpointResponse` trả lần giao gần nhất (trạng thái, lúc, HTTP); lọc endpoint theo trạng thái | mới | Gọi `/deliveries?limit=1` cho từng endpoint; lọc trên trang đang xem |
| [x] | Webhook | Mô tả event theo `Accept-Language` | mới | Hiện tiếng Anh cả khi chọn tiếng Việt |
| [ ] | Nhà phát triển › Webhook | Danh sách lần giao xuyên endpoint có lọc; gửi lại một lần giao | 52 | Gộp `/webhooks/{id}/deliveries` (25 lần mỗi endpoint); chưa có "Gửi lại" |
| [ ] | Nhật ký audit | `actor_kind` (người / agent / dịch vụ / hỗ trợ nền tảng) và tham số lọc `actor_kind=` | 51, 52 | Chưa có chip lọc theo loại người làm |
| [x] | Nhật ký audit | Mục audit trả kèm tên người làm (hoặc `include=actor`) | mới | Gọi `GET /auth/users/{id}` cho mỗi người trên trang; không có quyền thì hiện mã rút gọn |
| [ ] | API key | `kind` của key (người, agent, dịch vụ) | 51 | Chưa hiện loại key |
| [x] | API key | Quản trị viên xem và thu hồi key của mọi người trong khách | mới | Chỉ key của chính mình |
| [x] | API key | Xác nhận lại bằng passkey hoặc TOTP thay cho mật khẩu khi tạo key | mới | Chỉ mật khẩu |
| [x] | Tạo API key | Sổ permission đọc được khi không có `auth.roles.read` (vd `GET /me/permissions` kèm mô tả) | mới | Chỉ thấy tên permission, không có mô tả |
| [ ] | Chờ duyệt | `GET /kernel/approvals`, `POST /approvals/{id}/approve`, `/reject` | 51 | Dữ liệu mẫu (`src/sample/agent.ts`) |
| [ ] | Nhật ký gọi tool MCP | Nhật ký các lần agent gọi tool | 51, 52 | Dữ liệu mẫu (`src/sample/agent.ts`) |
| [x] | Kết nối MCP | Core công bố địa chỉ công khai (`AUTOXIDE_HTTP_PUBLIC_URL`) | mới | Dùng `AUTOXIDE_API_URL` của giao diện |
| [ ] | Nhà phát triển › Request | `GET /kernel/requests?status_class=&api_key_id=` | 52 | Dữ liệu mẫu (`src/sample/devtools.ts`) |
| [ ] | Nhà phát triển › Event theo bản ghi | `GET /kernel/records/{kind}/{id}/events` | 52 | Dữ liệu mẫu (`src/sample/devtools.ts`) |

## Tiếp thị › Biểu mẫu, Bản tin, Bình luận (`/marketing/...`)

| | Màn | Cần | Bước | Giao diện đang làm gì |
|---|---|---|---|---|
| [x] | Biểu mẫu | `FormResponse` kèm số bài nộp theo trạng thái (cột "Bài mới", số trên chip lọc) | mới (số đếm menu: 51) | Gọi `GET /forms/{id}/submissions?status=new&limit=100` cho từng form, hiện "100+" |
| [x] | Bài nộp | Xuất CSV lớn bằng job, trả link tải có hạn | 32 | Xuất ngay qua route của giao diện |
| [x] | Bài nộp | `AnswerResponse` kèm nhãn lựa chọn của đúng phiên bản form lúc gửi; kiểu `value` trong `openapi.json` đang là object | mới | Lấy nhãn từ form hiện tại; lựa chọn đã bỏ thì hiện giá trị thô |
| [x] | Bản tin | `GET /newsletter/settings`: nhãn quảng cáo, giới hạn thư / ngày, tốc độ gửi, đã đặt URL công khai / trang chọn danh sách / địa chỉ người gửi chưa | mới | Hiện mặc định `[QC]`, 3 thư / ngày; thiếu cấu hình thì hiện lỗi 503 của core khi gửi |
| [x] | Chiến dịch | Xem trước thư đúng như khi gửi (kèm chân thư): `GET /campaigns/{id}/preview` | mới | Xem trước HTML của người soạn; chân thư mô tả bằng chữ |
| [x] | Chiến dịch | Số huỷ đăng ký / bounce sau thư này | mới | Không hiện |
| [ ] | Danh sách nhận tin | Tổng số đăng ký và số địa chỉ không gửi của cả khách | 51 (số liệu tóm tắt) | Cộng các danh sách ở trang đầu (≤ 50) |
| [x] | Người đăng ký | Lọc theo trạng thái người đăng ký (`blocked`); tham số `status` hiện lọc theo trạng thái đăng ký | mới | Không có chip "Bị chặn"; nhãn hiện trên dòng |
| [x] | Bình luận | `CommentDetail` kèm tiêu đề và link của đích (sổ đối tượng D-123) | mới | `content.entry`: đọc bài qua API `content`; loại khác hiện id |
| [x] | Bình luận › Theo bài | `GET /comments/targets`: danh sách đích có số đếm, lọc theo trạng thái | mới | Lấy đích từ 200 bình luận mới nhất |
| [ ] | Menu Bình luận, Biểu mẫu | Số việc đang chờ (bình luận chờ duyệt, bài nộp mới) | 51 (hộp việc) | Chưa hiện số |
| [ ] | Bình luận › Cài đặt | Cài đặt bình luận theo khách (bật / tắt, chế độ duyệt) | 33 | Dữ liệu mẫu (`src/sample/engage.ts`) |
| [ ] | Bình luận › Cài đặt | Chính sách `posting.comments` (xác thực tài khoản, NĐ 147/2024) | 37 | Dữ liệu mẫu (`src/sample/engage.ts`) |
| [ ] | Ba tab "Dữ liệu theo email" | Một yêu cầu của chủ thể dữ liệu gom mọi module, gắn với hồ sơ ở Tuân thủ | 37 | Ba tab riêng, mỗi module tra / xoá một lần |

## Nội dung › Thư viện media, Trang và cấu trúc, Menu

| | Màn | Cần | Bước | Giao diện đang làm gì |
|---|---|---|---|---|
| [x] | Thư viện media | Lọc "thiếu alt" (`alt_missing=true`) và số ảnh thiếu alt | mới | Lọc trên trang đang xem (48 tệp) |
| [x] | Thư viện media | Tóm tắt cả thư viện: tổng tệp, dung lượng, số ảnh thiếu alt | mới | Tính trên trang đang xem |
| [x] | Thư viện media | Số tệp trong mỗi thư mục (`item_count` trong `FolderResponse`) | mới | Không hiện số |
| [x] | Thư viện media | Alt theo từng ngôn ngữ (`PUT/DELETE /media/{id}/alts/{locale}`, `alts` trong tệp) | v0.14.0 | Đã nối API thật (W-021 của web) |
| [x] | Thư viện media | Cờ "ảnh chỉ để trang trí" (alt để trống có chủ ý, khác "chưa có alt") | mới | Dữ liệu mẫu (`src/sample/media.ts`) |
| [x] | Thư viện media | Danh sách ngôn ngữ alt máy chủ mở (`AUTOXIDE_MEDIA_ALT_LOCALES`) qua API, để giao diện chỉ hiện ô của ngôn ngữ được nhận | mới | Hiện ô theo ngôn ngữ của nội dung; ngôn ngữ không mở thì hiện lỗi `media.unsupported_locale` của core |
| [ ] | Thư viện media | "Đang dùng ở": bài, trang, sản phẩm nào dùng tệp (`GET /media/{id}/usages`) | 51 (D-267) | Ghi "máy chủ chưa liệt kê được" |
| [x] | Thư viện media | Tên người tải lên (hiện chỉ có id) | mới | Không hiện |
| [x] | Thư viện media | Giới hạn tải lên cho giao diện: dung lượng tối đa, loại cho phép | mới (cùng loại "cấu hình cho giao diện") | Để core báo lỗi theo `code` sau khi gửi |
| [x] | Mọi biểu mẫu | `detail` trong `errors[]` của Problem dịch theo `Accept-Language` (vd lỗi `to_url` của chuyển hướng ra tiếng Anh) | mới | Hiện nguyên câu của core |
| [x] | Trang và cấu trúc | Số bài theo danh mục, thẻ, tác giả (`entry_count`) | mới | Đã nối (W-028 của web): cột số bài, ô tìm `q` |
| [x] | Trang và cấu trúc | Đường dẫn đầy đủ của trang (`path`) trong `EntryResponse` | mới | Đã nối (W-028 của web) |
| [x] | Trang và cấu trúc | ETag (hoặc `version`) của từng dòng trong danh sách | mới | Đã nối cho bài và trang (W-028 của web); danh mục, thẻ, tác giả, chuyển hướng chưa có `version` trong danh sách |
| [ ] | Trang và cấu trúc | Lấy cả cây trang khi hơn 200 trang | mới | Lấy 200 trang đầu, có ghi chú |
| [ ] | Lịch sử phiên bản | Tên người sửa của phiên bản (hiện chỉ có id) | mới | Hiện lý do giữ phiên bản và thời điểm |
| [x] | Menu | Module `menus`: `GET/POST /menus`, `GET/PUT/DELETE /menus/{id}`, `GET /menus/public/{key}` | v0.14.0 | API thật (W-022 của web) |
| [ ] | Menu | Sổ đối tượng: danh sách loại kèm cờ "có cây"; tìm đối tượng | 51 (P6) + đề bài menus, điểm 1–2 | Hai loại của `content` ghi cứng; bài và danh mục đọc qua API content (200 đầu); loại thương mại là dữ liệu mẫu, core từ chối khi lưu (`menus.ref_unknown_kind`) |
| [ ] | Menu | Đọc menu kèm tiêu đề và trạng thái của từng đích, cho người có `menus.read` | mới (đề bài menus, điểm 3) | Tự tra từ bài và danh mục đã đọc; đích ngoài danh sách chỉ hiện id |
| [ ] | Menu | Lệnh nhập menu cũ có chạy thử; ghi lần nhập gần nhất | mới (đề bài menus, điểm 9) | Tự tính bản xem trước từ dữ liệu thật, hiện câu lệnh `autoxide task menus.import_content_menus`; chưa có nút chạy |
| [x] | Menu | Danh sách menu kèm số mục (`item_count`), người và lúc sửa cuối | mới | Bảng không có cột "Mục" |
| [x] | Menu | `POST /menus` trả `ETag` như `GET` và `PUT` | mới | Đọc lại menu sau khi tạo |
| [x] | Menu | Giữ id của mục giữa các lần lưu (nhận `id` trong `SiteMenuItemInput`); hiện `PUT` cấp id mới cho mọi mục | mới | Giữ mục đang chọn theo vị trí trong cây |
| [x] | Menu | Ngôn ngữ được phép của menu (`AUTOXIDE_MENUS_LOCALES`) qua API | mới | Dùng danh sách ngôn ngữ của content; core vẫn kiểm |
| [x] | Menu | Header `Deprecation` / `Sunset` trên `/content/menus…` | v0.14.0 | Lấy ngày từ header `Sunset` thật |

## Tổ chức › Cây tổ chức (`/org`) và Tuân thủ (`/compliance`)

| | Màn | Cần | Bước | Giao diện đang làm gì |
|---|---|---|---|---|
| [x] | Cây tổ chức | `GET /organizations/units` (`unit_type`, `code`, `status`), `GET /unit-types` | 34 | **Đã nối** (17/11, W-045 của web); cây cũng dùng cho bộ chọn nút góc trên trái (lọc `owner_org`, tạo mới với `org_id`) |
| [x] | Cây tổ chức | `POST /organizations/units` (nút `pending` khi thiếu `units.manage`), `POST /units/{id}/status` | 34 | **Đã nối** (17/11): thêm nút con, Hoàn tác = lưu trữ, duyệt nút chờ |
| [x] | Cây tổ chức | `GET /auth/memberships?org_id=` kèm tên, email người dùng, tên role, độ sâu rộng nhất của role | 34 (phần kèm tên và độ sâu: mới) | **Đã nối** (17/11). Phần "mới" chưa có, giao diện tự ghép: đọc từng người bằng `GET /auth/users/{id}` (N lời gọi mỗi màn), độ sâu tính từ `grants` của role. Có `include=user` thì bớt được N lời gọi — không gấp |
| [x] | Cây tổ chức | `include_inherited=true` cho memberships: thêm người ở nút cha có độ sâu `subtree` / `tenant` | mới | **Đã có** dưới tên `inherited=true`; đã nối (17/11). Tham số chưa có mô tả trong OpenAPI — xem mục "Core trả khác mô tả OpenAPI" |
| [x] | Cây tổ chức | Số người làm được ở mỗi nút | mới | **Đã có** (17/11, D-332): `GET /api/v1/auth/memberships/counts` — mỗi nút một dòng (`org_id`, `direct`, `total`; `total` gồm người ở nút trên, cùng nghĩa `inherited=true`), một lời gọi cho cả cây |
| [x] | Cây tổ chức | `GET /organizations/settings`, `GET/PUT/DELETE /units/{id}/settings/{key}` | 34 | **Đã nối** (17/11). Sổ cài đặt chưa nói dạng giá trị (chuỗi, danh sách, đúng sai): giao diện đoán theo giá trị đang có và mặc định; có trường `type` / JSON Schema thì làm được ô nhập đúng loại. **Đã có** (17/11, D-332): mỗi cài đặt trong `GET /organizations/settings` có `schema` (JSON Schema: `type`, `enum`, `pattern`, `items`…) |
| [ ] | Cây tổ chức | Yêu cầu tách nhánh từ phía khách: `GET/POST /organizations/units/{id}/split-requests` | mới (bước 38 chỉ có CLI) | Đã bỏ nút khỏi màn (17/11) tới khi có API |
| [ ] | Cây tổ chức | `POST /auth/invitations` (mời vào một nút) | 34 | Dữ liệu mẫu |
| [ ] | Tuân thủ | `GET /compliance/rules?as_of=` kèm lịch sử phiên bản | 37 | Dữ liệu mẫu (`src/sample/compliance-rules.ts`) |
| [ ] | Tuân thủ › Website | `GET /compliance/websites`, `GET …/{id}/checklist`, `GET …/{id}/dossier`, `POST …/{id}/status` (số hồ sơ, ngày nộp; kết quả: link và ảnh logo) | 37 (ngày nộp: mới) | Dữ liệu mẫu (`src/sample/compliance.ts`) |
| [ ] | Tuân thủ | `GET /compliance/overview`: số việc chờ theo khu vực | mới | Tính từ hàm mẫu |
| [ ] | Tuân thủ › KYC | `GET /kyc/profiles?status=` kèm tên chủ thể và cờ "thiếu giấy tờ"; `POST …/approve`, `POST …/reject {reason_code, note}` | 37 (tên và cờ: mới) | Dữ liệu mẫu |
| [ ] | Tuân thủ › Dữ liệu cá nhân | `GET/POST /incidents`, `PATCH /incidents/{id}`, `POST …/entries` | 37 | Dữ liệu mẫu |
| [ ] | Hồ sơ | `GET/PATCH /legal-profiles`, `GET/PUT /policies` | 37 | Dữ liệu mẫu |
| [ ] | Hồ sơ › Mục đích | `GET/POST/PATCH /purposes` kèm số người đồng ý / đã rút | 37 (số đếm: mới) | Dữ liệu mẫu (`src/sample/compliance-records.ts`) |
| [ ] | Hồ sơ › Hạn lưu | `GET /retention`, `GET /processing-register` | 37 | Dữ liệu mẫu; route của giao diện xuất JSON |
| [ ] | Hồ sơ › Đăng nội dung | Thống kê người mua đã xác thực số điện thoại / định danh | mới | Dữ liệu mẫu |
| [ ] | Hồ sơ › Văn bản | `GET /documents`, `POST /documents/{id}/versions`, `GET …/acceptances`; số người đã chấp nhận theo phiên bản | 37 (số đếm: mới) | Dữ liệu mẫu; trang nguồn đọc thật |
| [ ] | Hồ sơ › Vụ việc | `GET /cases`, `POST /cases/{id}/restrict`, `…/restore`, `…/decision`, `POST …/entries` | 37 | Dữ liệu mẫu |
| [x] | Hồ sơ › Nhãn AI | Bộ lọc `ai_generated=true` trên `GET /content/entries` | mới | Đọc 100 bài sửa gần nhất rồi lọc (API thật) |
| [ ] | Hồ sơ › Nhãn AI | Lọc và bỏ nhãn AI cho sản phẩm và alt media | mới | Dữ liệu mẫu |
| [ ] | Hồ sơ › Yêu cầu của chủ thể | `GET/POST /compliance/subject-requests`, `POST …/{id}/status`; số lần tự xuất / tự xoá trong 30 ngày | mới | Dữ liệu mẫu |
| [ ] | Hồ sơ › Yêu cầu của chủ thể | Một API tra / xoá dữ liệu cá nhân theo email chung cho mọi module | mới (bước 37 chỉ có `/consents?subject=`) | Dẫn tới 3 tab "dữ liệu theo email" thật |

## Sản phẩm › Sản phẩm và giá (`/catalog`), Tồn kho (`/catalog/inventory`)

Cột "Giao diện đang làm gì": dữ liệu mẫu ở `src/sample/catalog.ts`, `inventory.ts`, `tax.ts`, trừ chỗ ghi khác.

| | Màn | Cần | Bước |
|---|---|---|---|
| [ ] | Sản phẩm | `GET/POST /catalog/products` (lọc `status`, `kind`, `category_id` gồm cả nhánh, `q` không dấu), `GET/PATCH/DELETE /products/{id}`, `POST /products/{id}/status` | 41 |
| [ ] | Sản phẩm | Danh sách trả kèm khoảng giá ở bảng giá của nút đang xem, ảnh đầu, tóm tắt SKU; đếm theo trạng thái | mới (gộp 41) |
| [ ] | Sản phẩm | Tình trạng "mở bán được chưa": `GET /products/{id}/readiness` hoặc trường `issues`, kèm lọc `blocked=true` | mới |
| [ ] | Chi tiết | `POST /products/{id}/variants`, `PATCH /variants/{id}`, `PUT /products/{id}/options`, `GET /variants?code=` | 41 |
| [ ] | Chi tiết | Thuộc tính đồ bếp có cấu trúc theo **từng biến thể** (số người ăn, trong hộp có gì / không kèm, dùng và chăm sóc theo ngôn ngữ, máy rửa bát, tệp PDF). Đề xuất `PUT /products/{id}/kitchen` | mới (gộp 41; đã ghi ở `ui-phase3-api.md`) |
| [ ] | Chi tiết | `PUT /products/{id}/media` (media công khai, gắn biến thể); alt theo ngôn ngữ | 41 (alt theo ngôn ngữ: mới) |
| [ ] | Chi tiết | `PUT /products/{id}/translations/{locale}`, `PUT /products/{id}/attributes` | 41 |
| [ ] | Chi tiết | Đọc tiền tố đường dẫn công khai (`AUTOXIDE_CATALOG_PRODUCT_PATH_PREFIX`) | mới (cấu hình cho giao diện) |
| [ ] | Chi tiết | Dòng thời gian thay đổi của một sản phẩm | 52 |
| [ ] | Danh mục | `GET/POST /categories`, `PATCH`, `POST /{id}/move`, `PUT /{id}/attributes` (ngày hiệu lực, căn cứ), `PUT /{id}/restrictions`; kèm số sản phẩm cả nhánh | 41 (số sản phẩm: mới) |
| [ ] | Thuộc tính | `GET/POST /attributes`, `PUT /{id}/options`; mã lỗi trùng `catalog.attribute_code_taken` | 41 (mã lỗi: mới) |
| [ ] | Hàng cấm | `GET /restriction-rules` (gói "vn", chỉ đọc) | 41 |
| [ ] | Giấy tờ | `GET/POST/DELETE /products/{id}/documents`; danh sách xuyên sản phẩm `GET /documents?status=expiring\|missing` | 41 (xuyên sản phẩm: mới) |
| [ ] | Bảng giá | `GET/POST /price-lists`, `PATCH`, `POST /{id}/prices` (`reference`), `GET /{id}/prices?variant_id=`; kèm tên nút và số dòng | 41 |
| [ ] | Bảng giá | Giá theo cửa hàng cho mọi biến thể một lần: `GET /stores/{org_id}/prices` | mới |
| [ ] | Bảng giá | Lịch sử giá của một biến thể gộp mọi bảng giá | mới |
| [ ] | Bảng giá | `GET /variants/{id}/price` trả mọi bảng ứng viên kèm lý do bị bỏ qua | 41 (hở) |
| [ ] | Nhóm thuế | `GET /tax/categories`, `GET /tax/rates?jurisdiction=VN&category=` | 36 |
| [ ] | Nhập CSV | `POST /imports`, `GET /imports/{id}`, `POST /exports`, `GET /exports/{id}/content` | 41 |
| [ ] | Nhập CSV | Chạy thử `POST /imports?dry_run=true`; danh sách việc nhập / xuất | mới |
| [ ] | Bản nháp AI | `POST /products/{id}/suggestions`, `POST /suggestions/{id}/apply\|discard`; danh sách chờ `GET /suggestions?status=pending` | 41 (danh sách: mới) |
| [ ] | Tồn | `GET /inventory/levels` (lọc `org_id`, `below_reorder`, `q`; kèm tên biến thể và kho), `GET /levels/{id}` kèm chỗ giữ, lô, serial, biến động | 42 (`q`, kèm tên, kèm chi tiết: mới) |
| [ ] | Tồn | `PATCH /levels/{id}` (`reorder_point`) | 42 |
| [ ] | Tồn | `GET /reports/valuation` theo kho, kèm đếm dưới mức / hết hàng / phiếu đang đi đường | 42 (phần đếm: mới) |
| [ ] | Sổ biến động | `GET /movements`, `POST /adjustments` (`reference`, serial cho hàng theo serial) | 42 (serial: hở) |
| [ ] | Chuyển kho | `GET/POST /transfers`, `POST /{id}/ship\|receive\|close\|cancel` (nhận một phần) | 42 |
| [ ] | Kiểm kê | `GET/POST /counts`, `PUT /{id}/lines`, `POST /{id}/apply\|cancel` | 42 |
| [ ] | Lô | `GET /lots?status=`; loại bỏ một lô sớm | mới |
| [ ] | Serial | `GET /serials?variant_id=&status=`; hàng khách trả: cho bán lại / loại bỏ | mới |
| [ ] | Nhà cung cấp | `GET/POST /suppliers`, `PATCH`; công nợ phải trả đọc từ sổ cái | 42 (công nợ: 36) |
| [ ] | Nhập hàng | `GET/POST /purchase-orders`, `POST /{id}/order\|close\|cancel`, `POST/GET /receipts` (`reference`), `POST/GET /supplier-returns`; kèm tình trạng job ghi sổ cái | 42 (tình trạng ghi sổ: mới) |
| [ ] | Nhập hàng | Phải trả và giá trị nhập trong tháng | mới (đọc sổ cái, 36) |

## Mặt điều khiển nền tảng (`/platform`)

Cột "Giao diện đang làm gì": dữ liệu mẫu ở `src/sample/platform.ts`, `platform-plans.ts`, `platform-data.ts`, trừ chỗ ghi khác.

| | Màn | Cần | Bước |
|---|---|---|---|
| [ ] | Mọi màn | Biết người đăng nhập thuộc khách nền tảng (`GET /kernel/tenant`) để hiện mặt điều khiển; permission của nhân sự vận hành | 33, 34 — Đã nối `GET /kernel/tenant` để hiện mặt điều khiển (W-027 của web); permission của nhân sự nền tảng chờ bước 34 |
| [ ] | Mọi màn | Sổ điều hướng của core có khái niệm không gian (khách / nền tảng) | 51 |
| [x] | Khách hàng | `GET /kernel/tenants` (lọc `status`, `parent_id`), `GET /kernel/placements` | 33 — Đã nối (W-027 của web): màn Khách hàng và Đội database đọc API thật, ghép với dữ liệu mẫu cho trường core chưa có. Đã nối thêm lọc `read_only`, `placement_kind`, `schema_state` và `expand=placement` (D-296, W-035 của web) |
| [ ] | Khách hàng | Trường của khách `business_model`, `primary_domain`; của placement `pending_migrations`, `location` | mới |
| [ ] | Khách hàng | Số đếm tóm tắt và hạn mức gần chạm nhất của mọi khách trong một lần gọi | mới — hợp đồng đã có (`expand=usage`, `near_limit`, D-296) nhưng trả `null` / rỗng tới bước 35; giao diện nối khi có dữ liệu |
| [ ] | Chi tiết khách | `PATCH /kernel/tenants/{id}` `{status}` kèm `reason` để ghi audit | 33 (`reason`: mới) — `PATCH {status}` đã nối (W-027 của web); `reason` để ghi audit vẫn thiếu |
| [x] | Chi tiết khách | Dời khách chung ↔ riêng qua API: `POST /kernel/tenants/{id}/move` `{to, dry_run}` | mới (33 mới có CLI) — **đã nối** (16/11, W-032): chạy thử, dời ngay khi `ready`, theo dõi `GET …/moves/{id}` |
| [ ] | Chi tiết khách | Xuất toàn bộ dữ liệu khách: `POST/GET /kernel/tenants/{id}/exports` và link tải | mới (33 mới có CLI) |
| [ ] | Chi tiết khách | Quyền dùng của một khách bất kỳ: `GET /plans/tenants/{id}/entitlements` | mới |
| [ ] | Chi tiết khách, Thuê bao | Lịch sử thuê bao; đặt / bỏ chỉ đọc; kỳ thanh toán theo khách cho nhà vận hành | mới — **đặt / bỏ chỉ đọc đã nối** (cờ của nhà vận hành, D-296, W-035 của web); lịch sử thuê bao và kỳ thanh toán chờ bước 35 |
| [ ] | Thuê bao | `GET /plans/subscriptions`, `PUT` / `cancel` thuê bao của khách, `POST /billing-periods/{id}/mark-paid` | 35 |
| [ ] | Gói và giá | `GET/POST/PUT` gói, `retire`, `prices`, `deactivate`; `subscriber_count` | 35 (`subscriber_count`: mới) |
| [ ] | Ghi đè | Danh sách ghi đè của mọi khách (`GET /plans/overrides`) | mới |
| [ ] | Cấp phát | `POST/GET /plans/provisionings`, `…/retry` | 35 |
| [ ] | Cấp phát | Danh sách lượt cấp phát `?status=`; lịch sử từng bước; `slug`, `business_model`, tên placement trong body | mới |
| [ ] | Đội database | Migrate bằng job, chạy thử trước: `POST /kernel/placements/migrations` `{placement_id?, dry_run}` | mới (33 mới có CLI) |
| [ ] | Sức khoẻ | `GET /kernel/ops/backlog`, `/ops/backups` | 39 |
| [ ] | Sức khoẻ | Cảnh báo đang bật kèm câu "làm gì": `GET /kernel/ops/alerts` | mới |
| [ ] | Vào dữ liệu khách | `GET/POST/DELETE /auth/support-grants` | 34 |
| [ ] | Vào dữ liệu khách | Nhà vận hành đọc cài đặt `auth.support_access` của khách; danh sách nhân sự nền tảng | mới |

## Tiếp thị › Khuyến mại (`/marketing/promotions`), Bán hàng › Vận chuyển (`/sales/shipping`)

Cột "Giao diện đang làm gì": dữ liệu mẫu ở `src/sample/promotions.ts` và `src/sample/shipping.ts`.

| | Màn | Cần | Bước |
|---|---|---|---|
| [ ] | Khuyến mại | `GET/POST /promotions/promotions`, `GET/PATCH/DELETE /{id}`, `PUT /{id}/translations/{locale}`, `/targets`, `/flash-items` | 43 |
| [ ] | Khuyến mại | `POST /{id}/activate`, `/pause`, `/resume`, `/end`, `/cancel` | 43 |
| [ ] | Khuyến mại | Kiểm trước khi bật: `POST /{id}/activate?dry_run=true` trả danh sách điều kiện; `last_activation_error` | 43 (ui-phase2) |
| [ ] | Khuyến mại | Danh sách kèm tên nút, mục tiêu đọc thành chữ, mã đầu tiên, `held_count`; endpoint đếm cho số liệu đầu trang | mới |
| [ ] | Khuyến mại | `GET /legal-limits?at`, `GET /notice-rules?at`, cờ `needs_review` (căn cứ cần đối chiếu) | 43 (cờ: mới) |
| [ ] | Khuyến mại | `POST /evaluate` trả thêm `skipped` (chương trình không áp + lý do `promotions.skip.*`); `code_not_applicable` mang `condition` có tham số | 43 (`skipped`: mới) |
| [ ] | Khuyến mại | Giá ngay trước khuyến mại → giá sau khuyến mại cho từng biến thể mục tiêu | 43 + `reference_prices` của 41 |
| [ ] | Khuyến mại | Mã lỗi mới: `promotions.reference_price_missing`, `targets_missing`, `extend_only`, `raise_only`, `not_a_discount`, `not_code_based`, `per_customer_over_total`, `target_unknown`, `line_format`, `no_price`, `too_many_generated`, `notice_not_required` | mới |
| [ ] | Mã giảm giá | Danh sách mọi mã của khách; `remaining` là số thật, `held_count`, `source`; `POST /{id}/codes` trả kết quả job; `GET /{id}/codes/export` | 43 (danh sách chung: mới) |
| [ ] | Flash sale | Flash item kèm giá tham chiếu, % giảm, giá thấp nhất được phép theo trần | 43 |
| [ ] | Thông báo Sở Công Thương | `POST /{id}/notice-export?format=csv\|json`, `PATCH /{id}/notice`; địa bàn, hạn nộp; một chương trình nhiều nơi nhận | 43 (nhiều nơi: hở) |
| [ ] | Lịch sử phiên bản | `GET /{id}/versions`; danh sách chung; `change {code, params, reason}` cho mỗi phiên bản | 43 (chung, `change`: mới) |
| [ ] | Vận chuyển | `GET /shipping/shipments?status&tracking&q`, `GET /{id}` (kiện, món, hành trình), `POST /shipments` | 44 |
| [ ] | Vận chuyển | `POST /{id}/book`, `/cancel`, `/split`, `PUT /{id}/parcels`, `POST /{id}/status` | 44 |
| [ ] | Vận chuyển | Hỏi lại hành trình theo yêu cầu: `POST /shipments/{id}/track` | mới |
| [ ] | Vận chuyển | `eta_from` / `eta_to` chụp lúc tạo, `max_attempts`, cân tính phí / cân quy đổi; endpoint đếm theo trạng thái | 44 + mới |
| [ ] | Vận chuyển | `GET /shipments/{id}/label` (in nhãn, chưa dựng) | 44 |
| [ ] | Phí vận chuyển | `GET/POST /methods`, `PATCH /{id}`, `PUT /{id}/rates`, `/pickup-points`, `GET/POST /zones`; kèm "ghi đè nút cha", tên và tình trạng hãng | 44 (kèm: mới) |
| [ ] | Phí vận chuyển | `POST /quote` trả `eta_from` / `eta_to`, `order_by`, `ready_at`, `hold_days`, `free_shipping_missing`, `unavailable[].reason_code` | 44 (`free_shipping_missing`: mới) |
| [ ] | Phí vận chuyển | Giờ chốt riêng của phương thức (`cutoff`); `tax_category` của phí giao | 44, 45 |
| [ ] | Lịch giao | `GET/POST /calendars` (phiên bản có ngày hiệu lực), `POST /calendars/holidays`, ngày nghỉ "chờ thông báo" | 44 (hở) |
| [ ] | Đối soát COD | `GET/POST /cod/statements`, `GET /cod/discrepancies`, `POST /{id}/resolve`; `payee`; số liệu phải thu / lệch / quá hạn | 44 (`payee`: 46) |
| [ ] | Hãng vận chuyển | `GET/POST /carrier-accounts`, `PATCH /{id}`, `POST /{id}/test`; `check_status`, `checked_at`, `last_error`, `capabilities`, `secret_set_at` | 44 |
| [ ] | Vận chuyển | Mã lỗi mới: `shipping.discrepancy_not_found`, `discrepancy_resolved`, `calendar_exists`, `parcel_count`, `days_range`, `outside_area`, `weight_out_of_range`, `no_pickup_point`, `carrier_name_taken` | mới |
| [ ] | Vận chuyển | Danh mục tỉnh / xã cho ô địa chỉ | 34 (`address`) |

## Tài chính › Kế toán (`/finance`)

Cột "Giao diện đang làm gì": dữ liệu mẫu ở `src/sample/finance.ts`, `finance-invoices.ts`, `tax.ts`, trừ chỗ ghi khác.

| | Màn | Cần | Bước |
|---|---|---|---|
| [ ] | Sổ cái | `GET /ledger/books`, `/accounts`, `/entries` (kèm `reversed_by_entry_id`, seal, `channel`), `GET /entries/{id}` | 36 |
| [ ] | Sổ cái | `POST /ledger/entries`, `POST /entries/{id}/reverse` | 36 |
| [ ] | Sổ cái | `GET /ledger/trial-balance?book_id=&from=&to=`; số dư phát sinh trong kỳ cho ô số liệu | 36 |
| [ ] | Hàng chờ sai lệch | `GET /ledger/exceptions`, `POST /exceptions/{id}/resolve`, `GET/POST /reconciliations`, `GET /payments` | 36 |
| [ ] | Hàng chờ sai lệch | Lọc thanh toán chờ theo số tiền + nút (`GET /payments?amount=&org_id=&status=`); mã `ledger.exception_not_open` | mới |
| [ ] | Hàng chờ sai lệch | Gửi thông báo cho người mua xác nhận tài khoản nhận hoàn | mới (chưa làm) |
| [ ] | Tài khoản ngân hàng | `GET/POST /ledger/bank-accounts`, `PATCH /bank-accounts/{id}`; mã `ledger.bank_account_taken`, `ledger.feed_not_connected` | 36 |
| [ ] | Tài khoản ngân hàng | Thử chuỗi VietQR mà không tạo thanh toán (`POST /ledger/vietqr/preview`) | mới |
| [ ] | Hoá đơn điện tử | `GET /invoices/documents`, `PATCH /documents/{id}`, `POST …/issue`, `/adjust`, `/replace`, `/cancel`, `DELETE` | 36 |
| [ ] | Hoá đơn điện tử | Hỏi lại nhà cung cấp ngay (`POST /documents/{id}/refresh`); mã `invoices.document_state`, `invoices.correction_pending` | mới |
| [ ] | Hoá đơn điện tử | Tải `GET /documents/{id}/einvoice?format=xml\|pdf`, `GET …/render` | 36 (chưa làm) |
| [ ] | Ký hiệu và uỷ nhiệm | `GET/POST /invoices/series`, `PATCH /series/{id}`, `GET/POST /mandates`, `POST /mandates/{id}/revoke`; mã `invoices.series_taken`, `invoices.mandate_overlap` | 36 (mã lỗi: mới) |
| [ ] | Ký hiệu và uỷ nhiệm | Đính kèm văn bản uỷ nhiệm (`document_file_id` qua media) | 36 (giao diện chỉ có ô đánh dấu) |
| [ ] | Thuế | `GET /tax/rates`, `/packs`, `/profiles`, `PUT /profiles/{org_id}`, `POST /tax/quote` | 36 |
| [ ] | Thuế | `GET /tax/withholding-rates`, `GET /tax/rules` | mới (bảng đã có trong đặc tả, endpoint chưa) |
| [ ] | Thuế | GTGT đầu ra / đầu vào theo kỳ và thuế suất (`GET /tax/vat-summary?book_id=&from=&to=`) | mới |
| [ ] | Kỳ kế toán | `GET /ledger/periods` (kèm số bút toán / đã niêm phong), `POST /periods/{id}/close` | 36 |
| [ ] | Kỳ kế toán | Kiểm trước khi khoá; mã `ledger.period_not_ended`; kết quả kiểm chuỗi băm qua API (`GET /ledger/integrity`) | mới |
| [ ] | Xuất kế toán | `POST /invoices/exports`, `GET /exports/{id}`, `GET /exports/{id}/download` | 36 |
| [ ] | Xuất kế toán | Danh sách bản xuất (`GET /exports`), lập lại bản lỗi (`POST /exports/{id}/retry`) | mới |

## Bán hàng › Đơn hàng (`/sales/orders`), Cửa hàng (`/sales/stores`)

Cột "Giao diện đang làm gì": dữ liệu mẫu ở `src/sample/orders.ts`, `stores.ts`, trừ chỗ ghi khác.

| | Màn | Cần | Bước |
|---|---|---|---|
| [ ] | Đơn hàng | `GET /orders` (lọc `status`, `payment_status`, `org_id`, `channel`, `from`, `number`) và `GET /{id}` | 45 |
| [ ] | Đơn hàng | Lọc `flow` (dòng tiền), `q` (mã hoặc tên người mua), số đếm theo bộ lọc (`count=true`) | mới |
| [ ] | Đơn hàng | Số liệu đầu màn: đơn trong 30 ngày, đơn chờ xử lý, số đếm trên menu | 51 |
| [ ] | Đơn hàng | View đã lưu `GET/POST /me/views` (hệ thống / nút / của tôi) | 51 |
| [ ] | Đơn hàng | Hỏi bằng lời: `POST /ai/filters/parse` | 52 (mẫu chỉ bật khi `/ai/status` thật bật) |
| [ ] | Đơn hàng | Thao tác hàng loạt kèm hoàn tác | 51 |
| [ ] | Đơn hàng | Xuất CSV theo bộ lọc bằng job; in phiếu giao | 51 (route của giao diện đọc dữ liệu mẫu) |
| [ ] | Chi tiết đơn | `POST /{id}/transitions` kèm `If-Match`; trạng thái `awaiting_acceptance` | 45 |
| [ ] | Chi tiết đơn | Hoàn tác một lần chuyển trạng thái của nhân viên trong N phút | mới |
| [ ] | Chi tiết đơn | `POST /{id}/refunds`, `POST /{id}/payments/manual`; nơi tiền về của từng khoản hoàn | 45 |
| [ ] | Chi tiết đơn | `POST /{id}/payment-link` (gửi lại link thanh toán) | 45 |
| [ ] | Chi tiết đơn | Ghi chú nội bộ: `POST /{id}/notes` | mới (51 có dòng hoạt động chung) |
| [ ] | Chi tiết đơn | Bút toán của một đơn: đọc sổ cái theo `reference` | 36 |
| [ ] | Chi tiết đơn | Duyệt / từ chối đề xuất của agent: `/kernel/approvals/{id}/approve\|reject` | 51 |
| [ ] | Đổi trả | `GET /returns`, `POST /returns/{id}/transitions`; `media_ids`, `return_deadline_at`, `respond_by` | 45 |
| [ ] | Đổi trả tại quầy | Nhân viên tạo yêu cầu đổi trả đã nhận tại quầy (`return_method=in_store`, tình trạng hàng) | mới |
| [ ] | Bán tại quầy | `POST /pos/sales`; danh mục bán tại quầy (giá của cửa hàng + tồn bán được) | 45 |
| [ ] | Bán tại quầy | Hoá đơn khởi tạo từ máy tính tiền (bản in) | 45, 36 |
| [ ] | Giỏ bỏ dở | `GET /orders/carts?status=abandoned`, `POST /carts/{id}/reminders` (kèm huỷ lần nhắc), chỉ khách đã đồng ý, tối đa 2 lần | 45 + mới |
| [ ] | Nhận tại cửa hàng | `POST /{id}/pickup/ready` (có hoàn tác), `/pickup/code`, `/pickup/handover`; lọc `pickup_org_id`, `pickup_status`; `hold_days` | 45 |
| [ ] | Ca thu ngân | `GET /pos/shifts`; `difference_reason` khi đóng ca lệch; tiền mặt dự kiến của ca | 45 + mới |
| [ ] | Đánh giá | `GET /reviews`, `/reply`, `/moderation`; trạng thái "đã xem"; danh sách lý do ẩn | 45 |
| [ ] | Đánh giá | Bản nháp tóm tắt đánh giá bằng AI: tạo, duyệt, bỏ | 45 |

## Sàn › Vận hành sàn (`/market`), Trung tâm người bán (`/seller`)

Cột "Giao diện đang làm gì": dữ liệu mẫu ở `src/sample/market.ts`, `market-data.ts`, trừ chỗ ghi khác.

| | Màn | Cần | Bước |
|---|---|---|---|
| [ ] | Người bán | `GET /marketplace/sellers?status=`, `GET /sellers/{org_id}` | 46 |
| [ ] | Người bán | Kèm "điều kiện bán" từng mục (KYC, quy chế, hợp đồng, giấy tờ, tài khoản nhận tiền, duyệt) và `can_sell`, tên ngành hàng | mới |
| [ ] | Người bán | `POST /sellers/{org_id}/transitions` (approve / reject / suspend / reinstate / terminate, `reason`, `If-Match`); `POST /documents/{id}/decision` | 46 |
| [ ] | Người bán | Tự gỡ tạm khoá khi KYC / văn bản đủ lại (đặc tả mới có chiều tự tạm khoá) | mới (bổ sung đặc tả 46) |
| [ ] | Người bán | Mời người bán (lời mời của bước 34 gắn nút sàn, hoặc `POST /marketplace/seller-invitations`) | mới |
| [ ] | Người bán | `POST /penalties`, `GET /sellers/{org_id}/performance` | 46 |
| [ ] | Duyệt sản phẩm | `GET /listing-reviews?status=&seller=`, `POST /listing-reviews/{id}/decision` | 46 |
| [ ] | Duyệt sản phẩm | Tóm tắt sản phẩm trong lượt duyệt; chuyển về `pending` để Hoàn tác "Duyệt" | mới |
| [ ] | Phí sàn | `GET /fee-rules`, `POST /fee-rules`, `POST /fee-rules/{id}/close`, `GET /fee-rules/preview` | 46 |
| [ ] | Chi trả | `GET /payouts`, `GET /settlements`, `POST /payouts/{id}/retry` | 46 |
| [ ] | Chi trả | Gửi ngay một đợt chi đang lập (`POST /payouts/{id}/send`); `seller_can_receive` | mới |
| [ ] | Chi trả, Tiền của người bán | Số liệu tổng: đang giữ ở đối tác, chờ chi, đã chi trong tháng | mới |
| [ ] | Tranh chấp | `GET /disputes?status=&seller=`, `POST /disputes/{id}/transitions` (`resolution`, `amount`, `reason`), tin nhắn | 46 |
| [ ] | Trung tâm người bán | `GET /sellers/me` kèm điều kiện bán | 46 |
| [ ] | Trung tâm người bán | `POST /sellers/me/submit` (422 kèm `errors[]`); `POST /sellers/me/documents` | 46 |
| [ ] | Trung tâm người bán | Người bán tự chấp nhận văn bản: `POST /compliance/documents/{id}/acceptances` | 37 |
| [ ] | Gian hàng | `PUT /sellers/me` (cách giao, COD, tài khoản nhận tiền vào kho secret, chỉ trả 4 số cuối) | 46 |
| [ ] | Sản phẩm của người bán | `POST /catalog/products` ở nút người bán; `GET /listing-reviews?seller=me` | 41, 46 |
| [ ] | Đơn của người bán | `GET /orders?seller=me` | 45 |
| [ ] | Tranh chấp phía người bán | `POST /disputes/{id}/transitions` (trả lời / đồng ý hoàn), `GET /disputes?seller=me` | 46 |
| [ ] | Điểm vi phạm | `GET /sellers/me/performance`, `POST /penalties/{id}/appeal` | 46 |
| [ ] | Tiền của người bán | `GET /settlements?seller=me`, `GET /payouts?seller=me`, `GET /statements` (CSV) | 46 (CSV qua route của giao diện) |
| [ ] | "Xem như người bán" | Đọc gian hàng như người bán thấy, chỉ đọc, cho `marketplace.sellers.read` / `marketplace.finance.read` | 46 (cookie `ax_seller_view` trên dữ liệu mẫu) |
| [ ] | Mọi màn Sàn | Mã lỗi chưa có trong đặc tả: `marketplace.listing_not_found`, `fee_rule_not_found`, `fee_rule_in_effect`, `payout_not_found`, `document_not_found`, `penalty_not_found`, `penalty_already_appealed`, `not_ready` | mới |

## Khách hàng (`/customers`), Kênh bán (`/sales/channels`), Tiếp thị liên kết (`/marketing/affiliate`)

Cột "Giao diện đang làm gì": dữ liệu mẫu ở `src/sample/customers.ts`, `channels.ts`, `channel-accounts.ts`, `affiliate.ts`.

| | Màn | Cần | Bước |
|---|---|---|---|
| [ ] | Khách hàng | `GET/POST /customers`, `GET/PUT /{id}`, `/{id}/notes`, `/{id}/orders`, `/{id}/points`, `POST /{id}/points/adjustments` | 47 |
| [ ] | Khách hàng | Trường hồ sơ trả sẵn bản che (SĐT / email / ngày sinh); xem đầy đủ cần permission riêng và ghi audit | mới |
| [ ] | Khách hàng | Số liệu tóm tắt: số hồ sơ, có tài khoản, chỉ SĐT, tỉ lệ mua lại 90 ngày | mới (51) |
| [ ] | Tài khoản người mua | `GET /auth/customers?status=`, `POST /{id}/lock`, `/{id}/unlock`, cài đặt `customer_registration` | 34 |
| [ ] | Nhóm khách | `GET/POST /groups`, `PUT/DELETE /groups/{id}`, `GET /groups/{id}/preview` (kèm số đã đồng ý nhận tin), thành viên; `used_in`; mã `customers.group_in_use`, `customers.rule_empty` | 47 (mã lỗi và `used_in`: mới) |
| [ ] | Điểm và hạng | `GET/POST /loyalty/rules`, `GET /loyalty/tiers`, `GET /loyalty/summary` | 47 (summary: mới) |
| [ ] | Điểm và hạng | Thưởng điểm khi đánh giá có ngày hiệu lực, không phụ thuộc số sao, đếm đã thưởng | 47 |
| [ ] | Giỏ bỏ dở | `GET/POST /reminder-rules` | 47 |
| [ ] | Giới thiệu bạn | `GET /customers/referrals` cho nhân viên, `GET/POST /referral-rules` | 47 (danh sách: mới) |
| [ ] | Rủi ro COD | `GET/POST /orders/risk/rules`, `POST /risk/lookup`, `GET/POST /risk/blocks`, `POST /risk/blocks/{id}/lift` | 45 |
| [ ] | Kênh bán | `GET /channels/connectors`, `/accounts`, `POST /accounts`, `/accounts/{id}/pause\|resume\|disconnect\|sync` | 48 |
| [ ] | Liên kết sản phẩm | Listings gộp mọi tài khoản kèm giá web / giá kênh; `PUT …/listings/{variant_id}`, `/publish`, `…/delist` | 48 (gộp, delist: mới) |
| [ ] | Phân bổ tồn | `GET/PUT /allocations`, `GET /stock-preview` gộp mọi tài khoản | 48 (gộp: mới) |
| [ ] | Đơn cần xử lý | `GET /inbound-orders`, `/{id}/mapping\|retry\|resolve`; số liệu; `resolution` | 48 (số liệu, `resolution`: mới) |
| [ ] | Đơn hàng | Đơn có `channel` = `shopee` / `tiktok_shop`, `channel_account_id` kèm tên, `external_ref`, `funds_flow = channel_collect` | 45, 48 |
| [ ] | Đối soát kênh | `GET /settlements`, `/{id}`, `POST /settlement-lines/{id}/resolve`, `GET /settlements/summary`, `POST /settlements/{id}/post` | 48 (post, hai tổng: mới) |
| [ ] | Báo cáo kênh | `GET /reports/channels?from&to` | 48 |
| [ ] | Tiếp thị liên kết | `GET /partners`, `/{id}`, `POST /{id}/transitions`; tóm tắt KYC theo mục đích, hạn, "còn thiếu gì" | 49 (tóm tắt KYC: mới) |
| [ ] | Quy tắc hoa hồng | `GET/POST /commission-rules`, `/{id}/close`, `/preview`, `GET/PUT /programs/{org_id}` | 49 |
| [ ] | Hoa hồng | `GET /commissions`, số liệu tóm tắt, điều chỉnh âm | 49 (số liệu, điều chỉnh: mới) |
| [ ] | Chi trả | `GET /payouts` (ảnh chụp khấu trừ), `POST /{id}/mark-paid`, `/{id}/retry`; thuế TNCN khấu trừ thay có ngày hiệu lực | 49, 36 |
| [ ] | Livestream | `GET/POST /live-sessions`, `/{id}/start\|end\|products\|log\|report` | 49 |
| [ ] | Góc người tiếp thị | `GET /partners/me/stats`, `/me/links`, `/me/tax-statement` | 49 |

## Tổng quan (`/`) và Việc và hoạt động (`/work`)

Giao diện dựng sổ nguồn việc ở `src/work/` theo đúng hình dạng đặc tả bước 51 (W-020 của web): có API
thì chỉ thay phần đọc. Ba nguồn đọc API thật (bình luận chờ duyệt, bài nộp form mới, bài chờ duyệt),
13 nguồn còn lại đọc dữ liệu mẫu của các phân hệ.

| [ ] | Màn | Cần | Bước | Giao diện đang làm gì |
| --- | --- | --- | --- | --- |
| [ ] | Hộp việc | `GET /kernel/work` gộp mọi nguồn theo permission, kèm `key`, `reason`, `incomplete` khi một nguồn không trả lời | 51 | 16 nguồn tự đọc từng module, hạn 2 giây mỗi nguồn |
| [ ] | Số đếm trên menu | `GET /kernel/work/counts` theo `NavItem.counter`, cache ngắn | 51 | Tự đếm, giữ 10 giây trong bộ nhớ theo người xem |
| [ ] | Hoãn, giao việc | `PUT/DELETE` trạng thái việc của từng người (bảng `kernel_work_states`), giao cho người có permission của nguồn | 51 | Giữ ở `src/sample/work.ts` |
| [ ] | Hạn của việc | Mỗi module tự khai hạn gợi ý cho việc không có hạn (đơn chờ nhận, bình luận, bài nộp, bài chờ duyệt) | 51 | Hạn viết ở `sources.ts` của web |
| [ ] | Ô số liệu Tổng quan | `GET /kernel/summary` (doanh thu hôm nay, đơn chờ, sắp hết hàng, bài chờ duyệt…) | 51 | 3 số thật, 3 số mẫu |
| [ ] | Các bước thiết lập | `GET /kernel/setup` theo mô hình kinh doanh, bước pháp lý chặn bán đánh dấu | 51 | 8 bước tính ở `setup.ts` |
| [ ] | Hoạt động | `GET /audit/activity` trả câu đã dịch (ai làm gì với gì), không chỉ bản ghi audit thô | 51 | Tự dựng câu từ `GET /audit` |
| [ ] | Bài chờ duyệt | Tham số lọc `review_requested=true` cho `GET /content/posts` và tổng số thật | 51 | Lọc `review_requested=true` đã nối (W-028 của web); tổng số thật chờ bước 51 |
| [ ] | Tổng số | `total` (hay `total_estimate`) trong pagination để ô số liệu không phải đếm từng trang | 51 | Đếm trên một trang, ghi "≥" khi còn trang |
| [ ] | Thao tác hàng loạt | `dry_run` (GET, không ghi) và `expected_count` → 409 `kernel.bulk_count_changed` | 51 | `src/sample/bulk.ts` |
| [ ] | Đã ghim | `GET/PUT/DELETE /me/pins` | chưa có trong lộ trình | Tab chỉ hiện khi bật dữ liệu mẫu |
| [ ] | Tuỳ chọn thông báo | `GET/PUT /me/notification-preferences` theo loại việc và kênh | 37, 52 | Tab chỉ hiện khi bật dữ liệu mẫu |
| [ ] | Ngôn ngữ ưa dùng | Lưu ngôn ngữ của người dùng ở core (`/me`) để email và thông báo theo đó | 52 | Chỉ nhớ bằng cookie của trình duyệt |

## Core trả khác mô tả OpenAPI (gặp khi nối, 16–17/11/2026)

Giao diện đã chịu được cả hai dạng; nên sửa mô tả hoặc giá trị cho khớp nhau.

| [x] | Chỗ | Mô tả OpenAPI | Core trả |
|---|---|---|---|
| [x] | `LastDelivery.status` (webhook) | `pending` / `delivered` / `failed` | `succeeded` thay cho `delivered` — **đã sửa mô tả** (16/11): giá trị đúng là `pending` / `succeeded` / `failed`, như `status` của lần gửi |
| [x] | `POST /kernel/tenants`, header `Location` | đường dẫn đầy đủ | `/kernel/tenants/{id}`, thiếu tiền tố `/api/v1` — **core trả `/api/v1/kernel/tenants/{id}`** (16/11, test `tenant_admin.rs` giữ hành vi này; mọi 201 dùng chung một chỗ ghép tiền tố). Thấy thiếu tiền tố thì kiểm lớp giữa (proxy của web) có cắt `/api/v1` không |
| [x] | `GET /kernel/client-config`, `content.post_path_prefix` | ví dụ `/blog/` | `blog` (không có dấu `/`) — **đã sửa mô tả** (16/11): giá trị là đoạn đầu của đường dẫn, không có `/`; `blog` cho `/blog/<slug>` |
| [x] | `CommentDetail.target_title` | — | chỉ có khi đích đang công khai; bình luận trên bài nháp chỉ có id (nên ghi rõ trong mô tả) — **đã ghi rõ** (16/11) |
| [ ] | `TenantMovePlan.files` | `0` khi hai placement dùng chung nơi lưu | `0` cả khi khách không có file nào — nên ghi cả hai trường hợp trong mô tả |
| [ ] | `Tenant.read_only`, `CurrentTenant.read_only` (D-296) | trường tuỳ chọn | core luôn trả — nên khai `required` để client khỏi phải coi thiếu là `false` |
| [ ] | `TenantExpanded` (D-296) | `placement`, `subscription`, `usage` tuỳ chọn | có `expand` thì cả ba luôn có mặt (có thể `null`) — nên khai `required` |
| [ ] | `PATCH /kernel/tenants/{id}` `read_only` cho khách nền tảng | chỉ nói bằng lời "cannot be made read-only" | 422 `validation_failed`, lỗi ô `read_only` — nên ghi vào mô tả response 422 |
| [ ] | `PATCH /kernel/tenants/{id}` | — | `id` chỉ nhận UUID, trong khi `…/tenants/{id}/settings` và `…/move` nhận cả slug — nên thống nhất (hoặc ghi rõ ở mô tả tham số) |
| [ ] | Mô tả của `TenantSettingState.description` (D-296) | — | vài mô tả có chuỗi nhiều dấu cách liền nhau giữa câu (vd `comments.banned_words`: "ignoring case      and accents") — do xuống dòng trong chuỗi nguồn |
| [ ] | Chọn khách của endpoint công khai (core nhiều khách), mục "Conventions" | chỉ nêu `tenant_suspended`, `tenant_read_only`, `tenant_maintenance` | tên miền chưa gắn khách nào: `400 tenant_required`; `Autoxide-Tenant` trỏ tới khách không có: `404 tenant_not_found`. Client (website bán hàng) phải rẽ nhánh theo hai mã này (web trả 404 cho cả trang, W-038 của web) — nên ghi vào "Conventions" cùng cách core chọn khách (credential, `Autoxide-Tenant`, tên miền / `X-Forwarded-Host` từ proxy tin cậy) |
| [ ] | Thiếu tham số query bắt buộc (vd `locale` của `GET /content/public/posts`) | `422` "A parameter is invalid; see `errors`" (và Conventions: lỗi kiểm dữ liệu liệt kê mọi trường một lần) | `400 bad_request`, `detail` là thông báo thô của thư viện bằng tiếng Anh ("Failed to deserialize query string: missing field `locale`"), không có `errors`, không theo `Accept-Language`. Giá trị sai (vd `limit=abc`) thì đúng `422 validation_failed` — nên trả cùng dạng cho tham số thiếu |
| [x] | `GET /auth/memberships`, tham số `inherited` (gặp 17/11) | không có mô tả | **Đã có mô tả** trong `docs/openapi.json` của `main` ("With `org_id`: also list memberships on the units above it, which reach it through `subtree` or `tenant` scopes. Default `false`."); bản phát hành v0.14.0 chưa có endpoint này — giao diện đang đọc tài liệu cũ, bản phát hành kế có |

## Website bán hàng (`apps/storefront`, W-026 của web)

Website công khai chỉ gọi endpoint công khai, từ máy chủ của nó. Phần thương mại dùng dữ liệu mẫu (cùng sản phẩm,
giá, cửa hàng với dữ liệu mẫu của trang quản trị) tới khi các bước 41–46 có.

| [ ] | Màn | Cần | Bước | Website đang làm gì |
|---|---|---|---|---|
| [ ] | Chân trang, đầu trang | Hồ sơ công khai của khách: tên, thông tin pháp lý (mã số doanh nghiệp, địa chỉ), logo đã thông báo Bộ Công Thương, cấu hình giao diện (kiểu `market` / `minimal` / `luxe`) | 34 (+ 55) | **Tên đã nối** (17/11): hồ sơ công khai của nút gốc trong `GET /public/stores`. Còn thiếu: thông tin pháp lý và logo Bộ Công Thương (ô `[…]`), kiểu giao diện (biến `AUTOXIDE_STOREFRONT_THEME`) |
| [x] | Chân trang, đầu trang | Hồ sơ của **website đang được gọi**: nút mà tên miền hay `?site=` chọn (D-305) | mới | **Đã có** (17/11, D-331): `GET /api/v1/organizations/public/site` (tham số `site` tuỳ chọn) trả hồ sơ công khai của nút website; tên miền gắn cửa hàng → hồ sơ cửa hàng. Kèm sửa lỗi: trước đây `PATCH /kernel/domains/{id}` với `org_id` luôn bị từ chối (`organizations` chưa công bố cây cho kernel). Thông tin pháp lý và logo Bộ Công Thương: `GET /compliance/public/owner`, `/public/badge` của bước 37; logo thương hiệu: dòng "Thông tin cửa hàng công khai" bước 68 |
| [x] | `/blog` | Danh sách danh mục và thẻ công khai của `content` (kèm số bài) | mới | Gom chủ đề từ các bài trên trang đang xem — **đã có** (16/11): `GET /api/v1/content/public/terms?locale=&taxonomy=` trả danh mục / thẻ có bài đã đăng, kèm `posts`; bài của một thẻ: `GET /public/posts?term_id=` |
| [ ] | Bài viết | Nhúng bản ghi trong nội dung (khối sản phẩm trong bài) | 51 | Khối "sản phẩm nhắc tới trong bài" là dữ liệu mẫu |
| [x] | Menu | Bộ biểu tượng của menu có biểu tượng ngành hàng (chảo, nồi, dao…) | mới (`menus`) | Dùng biểu tượng chung — **không cần core đổi** (16/11): mục menu đã có `icon` (chuỗi tự do); bộ biểu tượng ngành hàng là tài nguyên của giao diện, website ánh xạ tên → biểu tượng |
| [ ] | Địa chỉ đã gỡ | `resolve` trả 410 cho địa chỉ đã gỡ; website (App Router) chưa trả được 410 nên đang trả 404 | — (ghi để biết) | 404 |
| [ ] | Trang chủ, danh mục, sản phẩm, giỏ, đặt hàng, gian hàng | Catalog, giá theo cửa hàng, tồn theo cửa hàng, giá khi áp mã, giỏ, đơn, người bán (công khai) | 41–46 | Dữ liệu mẫu |
| [ ] | Livestream | Danh sách phiên livestream công khai (lịch phát), không chỉ một phiên theo id | 49 (mới) | Dữ liệu mẫu (`src/sample/live.ts`) — đã thêm vào lộ trình bước 49 (16/11) |
| [ ] | Sản phẩm › Đánh giá | Số đếm đánh giá có ảnh và theo biến thể; "chủ đề người mua hay nhắc" kèm bản tóm tắt AI đã duyệt | 45 (mới) | Dữ liệu mẫu — đã thêm vào lộ trình bước 45 (16/11) |
| [ ] | Sản phẩm, đánh giá, livestream | Endpoint công khai để báo vi phạm (lý do theo danh sách cố định) | mới | Dữ liệu mẫu — đã thêm vào lộ trình bước 46 (16/11) |
| [ ] | Sản phẩm | Thông tin hiển thị theo biến thể trong `by-slug` (số người ăn, trong hộp có gì, cách dùng / chăm sóc theo ngôn ngữ, máy rửa bát) | 41 (mới) | Dữ liệu mẫu — đã thêm vào lộ trình bước 41 (16/11) |
| [ ] | Sản phẩm | Phí và ngày giao theo tỉnh khi chưa có giỏ, kèm giờ chốt đơn trong ngày | 44 (mới) | Dữ liệu mẫu, giờ chốt 16:00 — đã thêm vào lộ trình bước 44 (16/11) |
| [ ] | Khuyến mại | Số thông báo Sở Công Thương và trần giảm theo luật trong `GET /public/promotions` | 43 (mới) | Dữ liệu mẫu — đã thêm vào lộ trình bước 43 (16/11) |
| [ ] | Tài khoản người mua | Đăng nhập / tạo tài khoản công khai cho người mua bằng OTP qua SMS hay passkey | 47 (mới) | Mẫu: số điện thoại + mã cố định, không nhận mật khẩu — đã thêm vào lộ trình bước 47 (16/11) |
| [x] | Đặt hàng | Endpoint công khai cho đơn vị hành chính của module `address` (tỉnh, phường) | D-193 (mới) | **Đã có** (bước 34); trang quản trị đã dùng (hồ sơ công khai). Ô địa chỉ khi đặt hàng của website còn dùng mẫu: việc của web, đợt sau |
| [ ] | Giỏ, đặt hàng | Báo giá giỏ trả thêm: số điểm tối đa dùng được, cách trả dùng được cho giỏ (COD theo từng người bán), ngày giao dự kiến theo từng nơi bán | 45 (mới) | Dữ liệu mẫu — đã thêm vào lộ trình bước 45 (16/11) |
| [ ] | Đơn của tôi | `return_until`, `buyer_cancel_until` có sẵn trong đơn | 45 (mới) | Tự tính hạn 7 ngày — đã thêm vào lộ trình bước 45 (16/11) |
| [ ] | Gian hàng người bán | Catalog công khai lọc theo người bán và đường dẫn trang sản phẩm cho hàng người bán; hồ sơ người bán có số sản phẩm theo nhóm, tỉ lệ giao đúng hạn, trạng thái tạm ngừng | 41, 46 (mới) | Dữ liệu mẫu; hàng người bán mở ở gian hàng — đã thêm vào lộ trình bước 41, 46 (16/11) |
| [ ] | Mời người bán | Phí sàn công khai (hiện `/fee-rules/preview` cần `marketplace.sell`); đăng ký bán trước khi có tài khoản | 46 (mới) | Dữ liệu mẫu; form đăng ký dùng `forms/public/ban-tren-cho` nếu cửa hàng đã tạo — đã thêm vào lộ trình bước 46 (16/11) |
| [ ] | Quyền riêng tư của người mua | Đối chiếu đủ endpoint tải về / xoá dữ liệu cá nhân từ website | 37 | Dữ liệu mẫu |
| [ ] | Tài khoản › Tải hàng số | `GET /api/v1/orders/me/downloads` (file, lượt còn, hạn); `POST /me/downloads/{id}/link` (link riêng sống 5 phút, trừ một lượt; tải tiếp bằng cùng link không trừ thêm); `POST …/resend` (email mới, link cũ hết hiệu lực, lượt giữ nguyên) | 41, 45 (mới) | Dữ liệu mẫu (`src/sample/account-more.ts`, W-044 của web) |
| [ ] | Tài khoản › Bảo hành | `GET /api/v1/orders/me/warranties` (món, số tháng, hạn tính từ ngày giao, phạm vi); `POST …/{id}/claims` (mô tả lỗi, ảnh qua `media`; cửa hàng trả lời cách gửi) | 45 (mới) | Dữ liệu mẫu |
| [ ] | Tài khoản › Tranh chấp (phía người mua) | `GET /api/v1/marketplace/me/disputes/{id}` (trạng thái, mốc, đề nghị hai bên, nơi giữ tiền), `POST …/messages` (lời + ảnh, không sửa / xoá), `POST …/transitions` (`accept_offer`, `escalate`, `withdraw`) | 46 | Dữ liệu mẫu; ảnh chỉ giữ tên file |
| [ ] | Tài khoản › Đăng nhập và bảo mật | Phiên của người mua (xem, đăng xuất nơi khác), thêm passkey cho người mua | 47 | Dữ liệu mẫu; passkey ghi "sắp có" |

