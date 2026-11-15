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

## Nội dung › Thư viện media, Trang và cấu trúc, Menu

| | Màn | Cần | Bước | Giao diện đang làm gì |
|---|---|---|---|---|
| [ ] | Thư viện media | Lọc "thiếu alt" (`alt_missing=true`) và số ảnh thiếu alt | mới | Lọc trên trang đang xem (48 tệp) |
| [ ] | Thư viện media | Tóm tắt cả thư viện: tổng tệp, dung lượng, số ảnh thiếu alt | mới | Tính trên trang đang xem |
| [ ] | Thư viện media | Số tệp trong mỗi thư mục (`item_count` trong `FolderResponse`) | mới | Không hiện số |
| [ ] | Thư viện media | Alt theo từng ngôn ngữ và cờ "ảnh trang trí" (`GET/PUT /media/{id}/alts`) | mới | Alt ngôn ngữ mặc định là `alt` thật; ngôn ngữ khác và cờ trang trí là dữ liệu mẫu (`src/sample/media.ts`) |
| [ ] | Thư viện media | "Đang dùng ở": bài, trang, sản phẩm nào dùng tệp (`GET /media/{id}/usages`) | mới | Ghi "máy chủ chưa liệt kê được" |
| [ ] | Thư viện media | Tên người tải lên (hiện chỉ có id) | mới | Không hiện |
| [ ] | Thư viện media | Giới hạn tải lên cho giao diện: dung lượng tối đa, loại cho phép | mới (cùng loại "cấu hình cho giao diện") | Để core báo lỗi theo `code` sau khi gửi |
| [ ] | Mọi biểu mẫu | `detail` trong `errors[]` của Problem dịch theo `Accept-Language` (vd lỗi `to_url` của chuyển hướng ra tiếng Anh) | mới | Hiện nguyên câu của core |
| [ ] | Trang và cấu trúc | Số bài theo danh mục, thẻ, tác giả (`entry_count`) | mới | Ghi "chưa hiện số" |
| [ ] | Trang và cấu trúc | Đường dẫn đầy đủ của trang (`path`) trong `EntryResponse` | mới | Tự ghép slug theo cây, chưa có tiền tố ngôn ngữ |
| [ ] | Trang và cấu trúc | ETag (hoặc `version`) của từng dòng trong danh sách | mới | Đọc lại bản ghi trước khi sửa, so `updated_at` |
| [ ] | Trang và cấu trúc | Lấy cả cây trang khi hơn 200 trang | mới | Lấy 200 trang đầu, có ghi chú |
| [ ] | Lịch sử phiên bản | Tên người sửa của phiên bản (hiện chỉ có id) | mới | Hiện lý do giữ phiên bản và thời điểm |
| [ ] | Menu | Module `menus`: `GET/POST /menus`, `GET/PUT/DELETE /menus/{id}`, `GET /menus/public/{key}` | mới (đề bài `menus-module.md`) | Dữ liệu mẫu (`src/sample/menus.ts`) |
| [ ] | Menu | Sổ đối tượng: danh sách loại kèm cờ "có cây"; tìm đối tượng | 51 (P6) + đề bài menus, điểm 1–2 | Dữ liệu mẫu |
| [ ] | Menu | Đọc menu kèm tiêu đề và trạng thái của từng đích, cho người có `menus.read` | mới (đề bài menus, điểm 3) | Dữ liệu mẫu |
| [ ] | Menu | Lệnh nhập menu cũ có chạy thử; ghi lần nhập gần nhất | mới (đề bài menus, điểm 9) | Dữ liệu mẫu; nguồn là menu cũ thật |
| [ ] | Menu | Header `Deprecation` / `Sunset` trên `/content/menus…` (core chưa gửi) | mới (đề bài menus, mục 3) | Ghi ngày sunset theo bản thiết kế |

## Tổ chức › Cây tổ chức (`/org`) và Tuân thủ (`/compliance`)

| | Màn | Cần | Bước | Giao diện đang làm gì |
|---|---|---|---|---|
| [ ] | Cây tổ chức | `GET /organizations/units` (`unit_type`, `code`, `status`), `GET /unit-types` | 34 | Dữ liệu mẫu (`src/sample/org.ts`) |
| [ ] | Cây tổ chức | `POST /organizations/units` (nút `pending` khi thiếu `units.manage`), `POST /units/{id}/status` | 34 | Dữ liệu mẫu |
| [ ] | Cây tổ chức | `GET /auth/memberships?org_id=` kèm tên, email người dùng, tên role, độ sâu rộng nhất của role | 34 (phần kèm tên và độ sâu: mới) | Dữ liệu mẫu (`src/sample/org-tree.ts`) |
| [ ] | Cây tổ chức | `include_inherited=true` cho memberships: thêm người ở nút cha có độ sâu `subtree` / `tenant` | mới | Dữ liệu mẫu |
| [ ] | Cây tổ chức | Số người làm được ở mỗi nút | mới | Tính từ memberships mẫu |
| [ ] | Cây tổ chức | `GET /organizations/settings`, `GET/PUT/DELETE /units/{id}/settings/{key}` | 34 | Dữ liệu mẫu |
| [ ] | Cây tổ chức | Yêu cầu tách nhánh từ phía khách: `GET/POST /organizations/units/{id}/split-requests` | mới (bước 38 chỉ có CLI) | Dữ liệu mẫu |
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
| [ ] | Hồ sơ › Nhãn AI | Bộ lọc `ai_generated=true` trên `GET /content/entries` | mới | Đọc 100 bài sửa gần nhất rồi lọc (API thật) |
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
| [ ] | Mọi màn | Biết người đăng nhập thuộc khách nền tảng (`GET /kernel/tenant`) để hiện mặt điều khiển; permission của nhân sự vận hành | 33, 34 |
| [ ] | Mọi màn | Sổ điều hướng của core có khái niệm không gian (khách / nền tảng) | 51 |
| [ ] | Khách hàng | `GET /kernel/tenants` (lọc `status`, `parent_id`), `GET /kernel/placements` | 33 |
| [ ] | Khách hàng | Trường của khách `business_model`, `primary_domain`; của placement `pending_migrations`, `location` | mới |
| [ ] | Khách hàng | Số đếm tóm tắt và hạn mức gần chạm nhất của mọi khách trong một lần gọi | mới |
| [ ] | Chi tiết khách | `PATCH /kernel/tenants/{id}` `{status}` kèm `reason` để ghi audit | 33 (`reason`: mới) |
| [ ] | Chi tiết khách | Dời khách chung ↔ riêng qua API: `POST /kernel/tenants/{id}/move` `{to, dry_run}` | mới (33 mới có CLI) |
| [ ] | Chi tiết khách | Xuất toàn bộ dữ liệu khách: `POST/GET /kernel/tenants/{id}/exports` và link tải | mới (33 mới có CLI) |
| [ ] | Chi tiết khách | Quyền dùng của một khách bất kỳ: `GET /plans/tenants/{id}/entitlements` | mới |
| [ ] | Chi tiết khách, Thuê bao | Lịch sử thuê bao; đặt / bỏ chỉ đọc; kỳ thanh toán theo khách cho nhà vận hành | mới |
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
