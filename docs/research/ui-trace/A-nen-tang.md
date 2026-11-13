# Báo cáo A: đối chiếu thiết kế 2 với lộ trình, phần nền tảng và quản trị chung

Ngày 13/11/2026. Agent chỉ đọc, nhánh `claude/intelligent-knuth-5lphbh`. Không sửa file nào trong repo.

## 0. Cách đọc báo cáo

- **Thiết kế** (bản thiết kế quản trị, artifact "Autoxide Admin" bản 37): trích `Tên.dc.html:dòng`. Dòng là
  dòng của file HTML gốc. Dữ liệu JS nằm ở cuối mỗi file, phần `renderVals`.
- **Repo**: trích `ROADMAP.md:dòng` (tức `docs/ROADMAP.md`), `51:dòng` (tức `docs/steps/51-ui-contracts.md`),
  tương tự cho `33:`, `34:`, `35:`, `37:`, `38:`, `39:`, `40:`, `52:`. `openapi` là `docs/openapi.json`
  (endpoint đang chạy thật).
- **Nhãn trạng thái** (chiều 1):
  - `API`: có mục trong lộ trình, đặc tả nêu endpoint hoặc trait.
  - `MỤC`: có mục trong lộ trình, nhưng đặc tả chưa nêu API.
  - `CÓ`: API đã chạy (bước 15–24 đã xong).
  - `THIẾU`: không có trong lộ trình. Đã grep, từ khoá ghi kèm.
  - `UI`: chỉ là việc của giao diện, máy chủ không cần làm gì.
  - `TM`: thuộc phần thương mại của agent kia, chỉ ghi chú chỗ giao nhau.
- Không báo lại hai đề bài đã có. `[ĐB-gaps]` là `docs/prompts/ui-api-gaps.md`. `[ĐB-help]` là
  `docs/prompts/ui-help-api.md`.

## 1. Chiều 1: từ thiết kế sang lộ trình

### 1.1 Map.dc.html (bản đồ phân hệ)

| Khối / tính năng | Bước, mục | API trong đặc tả | Trạng thái |
|---|---|---|---|
| Ba điều kiện hiện menu: module bật, permission ở nút, tính năng gói (`Map:35-44`) | 51 P1, 35 | `GET /kernel/navigation` (51:42), `Entitlements` (35) | API |
| Lọc phân hệ theo "mô hình" khách: chuỗi, một cửa hàng, SaaS, chỉ nội dung (`Map:141-143`) | 51 P10 có `models` (51:257) | không có nơi lưu "mô hình" của khách | THIẾU (xem 3.4) |
| 12 phân hệ, mục con kèm mô tả (`Map:145-211`) | 51 P1 | NavGroup, NavItem | API |
| Mặt điều khiển nền tảng, 6 mục (`Map:233-240`) | 33, 35, 34 | xem 1.6 | API, có chỗ thiếu |
| Luật của khung (`Map:242-252`) | 51 P7, P8, P10, P11, P12; 52 | như 1.2 | API |
| "Việc cần làm: mỗi module đẩy việc lên qua event" (`Map:148`) | 51 P2 | P2 tính việc từ dữ liệu, không qua event | Mâu thuẫn (3.1) |
| Thanh đáy 5 mục trên điện thoại (`Map:247`) | "Sau core" (ROADMAP.md:1185) | — | UI (lộ trình ghi để sau) |

### 1.2 Main.dc.html: khung chung

| Khối / tính năng | Bước, mục | API trong đặc tả | Trạng thái |
|---|---|---|---|
| Thanh phân hệ, hàng tab con, số đếm, nhãn khoá "Pro" kèm lời giải thích (`Main:71-76`, `:541-577`) | 51 P1, P2 counts, P11 | navigation, `work/counts`, `me/can` (51:262) | API |
| Ngữ cảnh "khách › nút": tìm nút, nút gần đây, đổi khách (`Main:359-394`, `:580-602`) | 34 | `GET /me/tenants`, `POST /me/tenant`, `GET /units?q` (34:292, 319) | API |
| Dòng "Dữ liệu: Postgres riêng, lưu tại Việt Nam" (`Main:363`) | 33, 37 | `GET /kernel/tenant` không trả placement; residency cần `compliance.privacy.read` | MỤC (thiếu một trường đọc được cho quản trị khách) |
| Ctrl+K: lệnh `>`, toán tử, phạm vi, mở thẳng theo mã (`Main:416-448`, `:921-944`) | 51 P3, P9 | `kernel/find`, `kernel/resolve` (51:242) | API |
| Đổi ngôn ngữ VI / EN (`Main:59-60`) | — | ngôn ngữ ưa thích của người dùng đã bị hoãn ở D-062 (DECISIONS.md:1280) | UI cho màn hình; THIẾU cho thông báo gửi chậm (3.9) |
| Sáng / tối, toast (`Main:463`) | — | — | UI |
| Chuông thông báo: Hộp thư / Đã lưu / Xong, lý do nhận, "Chọn kênh nhận" (`Main:330-349`, `:956-973`) | 37 notifications, 52 mục 4 | `/me/inbox` (37:544), `/me/preferences` | API |
| Tin "Bản phát hành mới có bản vá bảo mật" gửi quản trị mọi khách (`Main:960`) | — | — | THIẾU (từ khoá: broadcast, announcement, thông báo mọi khách, bản phát hành mới) |
| Bảng Trợ giúp: bài theo trang, phím tắt (`Main:62`) | — | — | UI |
| Trợ giúp: "Hỏi trợ lý AI về trang này" (`Content:48`) | 52 mục 3 | 52:33-38 không có endpoint hội thoại | MỤC |
| Trợ giúp: "Có gì mới" | [ĐB-help] A (đã xem tới đâu) | nội dung danh sách do repo giao diện giữ | đã có đề bài; nội dung là UI |
| Trợ giúp: "Liên hệ hỗ trợ, kèm trang đang xem và mã khách" | — | — | UI (cần một cấu hình liên hệ của nền tảng, xem (b)) |
| Thẻ "Mới: lưu view" có "Đã hiểu" (`Orders:104`) | [ĐB-help] A | `/me/flags` | đã có đề bài |
| Toast "Hoàn tác" cho hoãn và xử lý (`Main:471-476`, `:835`) | 52 mục 2; 51 P10 | `restore` (52:27); không có endpoint bỏ hoãn | MỤC (thiếu bỏ hoãn, xem (c)) |
| Thanh đáy điện thoại (`Main:456-460`) | Sau core | — | UI |

### 1.3 Main.dc.html: trang Tổng quan và trang tổng quan của các phân hệ trong phạm vi

| Khối / tính năng | Bước, mục | API trong đặc tả | Trạng thái |
|---|---|---|---|
| Lời chào kèm số việc (`Main:87`) | 51 P2 | `kernel/work` | API |
| Hộp việc nhóm Quá hạn / Hôm nay / Sắp tới, "vì sao bạn thấy", hoãn (`Main:110-137`, `:807-854`) | 51 P2, P10 | `kernel/work`, `me/work/{key}/snooze` (51:254) | API |
| Nút "Đã xử lý" trên từng việc (`Main:131`, `:268`, `:840`) | 51 P2: không có trạng thái "bỏ qua" (51:98) | — | Mâu thuẫn (3.2) |
| Giao việc cho người khác | 51 P10 `assign` | có API | API, nhưng thiết kế CHƯA VẼ |
| Danh sách việc thiết lập, nhãn "Chặn bán" (`Main:146-157`, `:857-879`) | 51 P10 `SetupStep` | `GET /kernel/setup` (51:258) | API, có hai chỗ lệch (3.4, 3.5) |
| Bốn ô số liệu theo phân hệ đã bật (`Main:99-101`, `:627-637`) | 51 P4 | `kernel/summary` | API; định nghĩa số liệu thuộc [ĐB-help] B |
| Doanh thu theo nút (`Main:167-181`) | 51 P4 | — | TM |
| Gói và hạn mức (`Main:189-198`, `:887-901`) | 35 | `GET /entitlements` (35:300) | API |
| Hoạt động gần đây (`Main:207-214`) | 51 P5 | `audit/activity` (51:147) | API |
| Nút tạo nhanh (`Main:91`, `:914-919`) | 51 P1 | navigation | API |
| Landing **Tiếp thị**, phần newsletter và form: ô số, việc, bảng chiến dịch (`Main:694-702`) | 20, 21 (đã xong); 51 P2, P4 | `CÓ` cho danh sách; nguồn việc và số liệu ở 51 | API |
| Landing Tiếp thị: đồng hồ "Tỉ lệ mở 42%, bấm liên kết 7%" (`Main:700`) | — | `newsletter` không đo lượt mở hay bấm | Mâu thuẫn (3.8) |
| Landing **Nội dung** (`Main:703-711`) | 16, 17, 22, 37; 51 | `CÓ` | API; "website · nút" xem 3.3 |
| Landing **Khách hàng**, phần tài khoản và đồng ý (`Main:712-720`) | 34 (`/customers`), 37 (consent) | 34:323, 37:385 | API; "gộp tài khoản trùng" THIẾU (TM, bước 47) |
| Landing **Báo cáo**: báo cáo đã lưu, lịch gửi email, "Báo cáo mới" (`Main:739-748`) | 38 | `reporting/summary` có; báo cáo đã lưu và lịch gửi không có | THIẾU (từ khoá: lịch gửi, gửi báo cáo, scheduled report, báo cáo đã lưu, saved report) |
| Landing **Tổ chức** (`Main:749-757`) | 34 | — | API, có thêm hai chỗ thiếu: "4 thành viên chưa bật 2 lớp, role tài chính thì bắt buộc" là THIẾU (từ khoá: MFA bắt buộc, require_mfa, enforce); "3 tài khoản không đăng nhập 90 ngày" là THIẾU (từ khoá: last_login, không đăng nhập, inactive) |
| Landing Tổ chức: "Nhân sự nền tảng đang có quyền vào… Thu hồi" (`Main:752`) | 34 | chỉ khách vận hành thu hồi được (34:331) | Mâu thuẫn (3.6) |
| Landing **Tuân thủ** (`Main:758-766`) | 37, 52 mục 5 | `compliance/health` (52:52) | API |
| Landing **Cài đặt**: kết nối đối tác, kèm "lần dùng cuối" và trạng thái (`Main:767-775`) | 33 | `GET /kernel/settings` chỉ có `is_set` và `updated_at` (33:421) | MỤC (thiếu "kiểm tra kết nối", xem (c)) |
| Cài đặt › Module "bật, tắt" (`Map:208`) và "Module đang bật và phiên bản" (`Main:668`) | 24, 35 | chỉ nhà vận hành ghi đè (35:305); `ModuleInfo` không có phiên bản | Mâu thuẫn (3.10) |
| Cài đặt › Job "trạng thái, lỗi, chạy lại" (`Main:671`) | 52 mục 1 | không có endpoint job nào (xem 1.7) | MỤC, API thiếu |
| Phân hệ Bán hàng, Sản phẩm, Sàn, Tài chính | 36, 41–49 | — | TM |

### 1.4 Content.dc.html (Bài viết, thiết kế 2)

| Khối / tính năng | Bước, mục | API | Trạng thái |
|---|---|---|---|
| Chọn website theo tên miền; mỗi website gắn một nút (`Content:65-72`, `:234-243`) | — | `content` giữ dữ liệu ở cấp khách, không có `org_id` (34:60) | THIẾU (3.3) |
| Cột "Nút sở hữu"; cửa hàng viết bài cho trang của mình, trụ sở sửa cả nhánh (`Content:96`, `:118`, `:131`) | — | như trên | THIẾU (3.3) |
| Tab Bản nháp / Chờ duyệt / Đã lên lịch / Đã đăng, có số đếm (`Content:88`, `:266-270`) | 17 | `status` chỉ có draft/published/archived; "chờ duyệt" là cờ `review_requested_at`; "lên lịch" nằm ở `schedules` riêng từng bài; danh sách không lọc được theo hai thứ này (openapi) | MỤC: thiếu tham số lọc (c) |
| Lọc "Có AI hỗ trợ"; nhãn "AI gợi ý SEO / AI viết nháp / AI dịch" (`Content:98`, `:248-255`) | 18, 37 (`ai_assisted`, ROADMAP.md:747) | trường `ai_generated` có, tham số lọc không có | MỤC: thiếu tham số lọc |
| Cột VI EN, view "Thiếu bản tiếng Anh" (`Content:117`, `:273`) | 17 (bản dịch), 51 P7 (view) | có `translation_group_id`, không lọc được "thiếu bản dịch ở `locale`" | MỤC: thiếu tham số lọc |
| View đã lưu "Bài của tôi" (`Content:273`) | 51 P7 | `/me/views` | API |
| "Duyệt và đăng" (`Content:120`, `:283`) | 17 | `POST /entries/{id}/publish` | CÓ |
| Trang bắt buộc theo website, nút "Tạo từ mẫu" (`Content:138-155`, `:289-305`) | 37 (ROADMAP.md:728) | `content/templates`, `websites/{id}/pages` (37:379) | API |
| Khối "Sắp đăng" của cả website (`Content:159-166`) | 17 | chỉ có `GET /entries/{id}/schedules` cho từng bài | MỤC: thiếu cách liệt kê lịch của nhiều bài |
| Ô tìm bài (`Content:46`) | 15, 17 | `q` | CÓ |

### 1.5 Org.dc.html (Cây tổ chức)

| Khối / tính năng | Bước, mục | API | Trạng thái |
|---|---|---|---|
| Cây nút với loại do khách đặt (`Org:61-71`, `:229-254`) | 34 | `GET /units`, `unit-types` (34:292-299) | API |
| Thêm nút con, kèm hạn mức "4 / 10 cửa hàng" (`Org:159-163`, `:334`) | 34, 35 | `POST /units`, `reserve("organizations.units")` | API; hạn mức đếm theo loại nút là chỗ lệch (3.12) |
| Ai làm ở nút này, gồm người được gán ở nút cha (`Org:88-109`, `:272-282`) | 34 | `GET /memberships?org_id` (34:318); chưa nêu có trả người thừa hưởng từ nút cha | MỤC |
| Mời thành viên: email, role, **độ sâu**, link 7 ngày, mật khẩu hoặc passkey (`Org:146-155`, `:332`) | 34 (ROADMAP.md:638) | `POST /invitations` (34:326) | API; độ sâu chọn khi mời là chỗ lệch (3.7) |
| Bảng độ sâu: của mình / nút / nút và con / toàn khách (`Org:127-132`, `:302-307`) | 34 | `Scope` (34:73) | API |
| Cài đặt hiệu lực, ghi rõ kế thừa từ nút nào (`Org:116-121`, `:285-300`) | 34 | `GET /units/{id}/settings` (34:301) | API; màn hình sửa CHƯA VẼ |
| "Tách thành khách riêng", người quản trị khách tự bấm (`Org:81`, `:167-170`, `:336`) | 38 (ROADMAP.md:769) | CLI và API của nhân sự nền tảng (38:286) | Mâu thuẫn (3.6) |
| Nút tab Lời mời (2), Uỷ quyền (`Org:53-54`) | 34 | `invitations`, `delegations` (34:326-329) | API, CHƯA VẼ |

### 1.6 Platform.dc.html (mặt điều khiển nền tảng)

| Khối / tính năng | Bước, mục | API | Trạng thái |
|---|---|---|---|
| Sổ khách: mô hình, gói, nơi lưu, migration, thuê bao, hạn mức gần nhất (`Platform:82-92`, `:249-279`) | 33, 35 | `GET /kernel/tenants` (33:412) chỉ có status và parent; gói, thuê bao, hạn mức nằm ở endpoint khác, gọi riêng cho từng khách | MỤC: thiếu một danh sách tổng hợp (c) |
| Lọc "Database riêng / Dùng chung / Chờ migration / Cần chú ý" (`Platform:259-264`) | 33, 35 | lọc theo placement, schema_state, thuê bao không có trên `/tenants` | MỤC |
| Bốn ô số liệu nền tảng (`Platform:281-286`) | 51 P4 (đặc tả chỉ nói phạm vi khách) | — | MỤC |
| Gói là dữ liệu: thẻ gói, số khách, giá (`Platform:100-107`, `:288-294`) | 35 | `GET /plans`, `subscriptions` (35:298-303) | API; trang sửa gói CHƯA VẼ |
| Ngăn chi tiết: bật module, "Ghi đè ngoài gói" (`Platform:135-139`, `:300-310`) | 35 | `tenants/{id}/overrides` (35:305) | API |
| "Chuyển sang database riêng / Gộp về dùng chung" (`Platform:128-131`, `:317-318`) | 33 (ROADMAP.md:580) | chỉ có CLI `autoxide tenant move` (33:377) | Mâu thuẫn (3.11) |
| "Migrate cả đội database" (`Platform:58`, `:340`) | 33 | chỉ có CLI `migrate up --all` (33:365) | Mâu thuẫn (3.11) |
| Vào dữ liệu khách: lý do, 1 / 4 / 24 giờ (`Platform:146-159`, `:324`) | 34 (ROADMAP.md:645) | `POST /support-grants` (34:331); tối đa mặc định 8 giờ (34:366) | API; "24 giờ" lệch (3.6) |
| Gia hạn, xuất dữ liệu cho khách, tạm khoá chỉ đọc (`Platform:163-170`, `:316-321`) | 33, 35 | gia hạn: `mark-paid` hoặc `PUT …/subscription`; tạm khoá: xem 3.11; xuất toàn bộ dữ liệu của một khách: không có API | MỤC; xuất: THIẾU (từ khoá: xuất dữ liệu, tenant export, data export) |
| Cấp phát: tên, tên miền, **mô hình**, gói, email quản trị; báo cần KYC nếu là sàn (`Platform:181-195`, `:341-362`) | 35 (ROADMAP.md:667) | `POST /provisionings` (35:312); không có trường "mô hình" | API; mô hình THIẾU (3.4) |
| Thanh bên: Thuê bao, Truy cập hỗ trợ, Sức khoẻ (`Platform:235`) | 35, 34, 39 | `subscriptions`, `support-grants`, `ops/backlog`, `ops/backups` (39:126) | API, cả ba trang CHƯA VẼ |

### 1.7 Devtools.dc.html (Nhà phát triển)

| Khối / tính năng | Bước, mục | API | Trạng thái |
|---|---|---|---|
| Nhật ký request: lọc theo status và key; xem chi tiết request ID, Idempotency-Key, trace; không lưu thân request; giữ 14 ngày (`Devtools:62-96`, `:204-216`) | 52 mục 1 | `kernel_request_log` (52:19); chưa nêu endpoint và cột trace ID | MỤC |
| Tên tool MCP trong nhật ký | — | — | [ĐB-gaps] E |
| Webhook: lần giao, "Gửi lại" (`Devtools:109-111`, `:217-225`) | 52 mục 1 | `GET /webhooks/{id}/deliveries` đã CÓ; gửi lại chưa có tên endpoint | MỤC |
| Job lỗi, dead-letter queue, "Chạy lại" (`Devtools:119-124`, `:226-233`) | 52 mục 1 | không có endpoint job nào trong openapi; 51:239 nhắc `GET /kernel/jobs/{id}` mà không định nghĩa | MỤC, API thiếu (c1) |
| Event theo bản ghi: loại và mã (`Devtools:132-139`) | 52 mục 1 | "đọc từ outbox theo kind + id" (52:24), nhưng event đã giao bị dọn theo lịch (ROADMAP.md:60-62) | MỤC, nguồn dữ liệu có rủi ro (c) |
| Permission `kernel.devtools.read` trên trang có nút ghi (`Devtools:62`) | 52 | chỉ có `.read` (52:25) | MỤC (thiếu permission ghi) |
| Tải OpenAPI (`Devtools:68`) | đã có | `/api/v1/openapi.json` | CÓ |

### 1.8 Agent.dc.html (API key và AI agent)

| Khối / tính năng | Bước, mục | API | Trạng thái |
|---|---|---|---|
| Tab Chờ duyệt: tóm tắt, tool cùng tham số, hạn, Duyệt / Từ chối (`Agent:79-85`, `:203-213`) | 51 P12 | chỉ có `POST /kernel/approvals/{id}/approve` (51:274); thiếu danh sách, chi tiết, từ chối kèm lý do | MỤC, API thiếu (c2) |
| Bảng key: loại Người / Agent / Dịch vụ, permission, lần dùng cuối, thu hồi (`Agent:100-107`, `:215-223`) | 51 P12 (`kind`); 2 | `GET/POST/DELETE /auth/api-keys` CÓ; `kind` thuộc 51 | API |
| Đánh dấu permission "agent: cần duyệt" khi tạo key (`Agent:233`, `:244`) | 51 P12 | `requires_approval_for` nằm trên lệnh (51:270), chưa có trong danh mục permission hay danh sách tool | MỤC (c) |
| Form tạo key: tên, loại, permission (`Agent:132-143`) | 2 | API bắt buộc `current_password` và có `expires_at` (openapi `CreateApiKeyRequest`) | Mâu thuẫn (3.13) |
| Key chỉ hiện một lần (`Agent:94`) | 2 | — | CÓ |
| Nhật ký gọi tool MCP (`Agent:118-120`) | — | — | [ĐB-gaps] E |

### 1.9 Compliance.dc.html (Tuân thủ)

| Khối / tính năng | Bước, mục | API | Trạng thái |
|---|---|---|---|
| Tab Tình trạng: ô theo nghĩa vụ (`Compliance:370-376`) | 52 mục 5 | `compliance/health` (52:52) | API |
| Website TMĐT theo tên miền: danh sách kiểm, xuất hồ sơ, logo (`Compliance:84-110`, `:317-342`) | 37 (ROADMAP.md:723) | `websites`, `checklist`, `dossier`, `status` (37:377-380) | API |
| KYC: bảng, Xác nhận, **"Yêu cầu bổ sung"**, trạng thái "Thiếu giấy tờ" (`Compliance:118-135`, `:287`) | 37 (ROADMAP.md:719) | approve, reject, revoke; trạng thái draft/pending/verified/rejected/expired/revoked (37:245) | API; lệch tên trạng thái (3.14) |
| Yêu cầu của chủ thể dữ liệu: xuất, xoá, rút đồng ý; Mới / Đang xử lý / Hoàn tất; "Duyệt và tạo gói" (`Compliance:141-147`, `:301-313`) | 37 (ROADMAP.md:737) | không có sổ yêu cầu; tự xuất `/auth/me/export`, tự xoá `DELETE /auth/me` chạy ngay (openapi) | THIẾU (từ khoá: chủ thể, subject_request, data subject, DSR, yêu cầu xuất) (3.15) |
| Sự cố 72 giờ: đếm giờ, các bước, "dựng sẵn bản thông báo" (`Compliance:152-168`, `:207-216`, `:390`) | 37 | `incidents`, `entries` (37:388); không có bản thông báo dựng sẵn | API; phần soạn sẵn thông báo THIẾU (từ khoá: thông báo cơ quan, mẫu, template trong mục incident) |
| Văn bản pháp lý: phiên bản, số người đã chấp nhận, số người bán chưa chấp nhận (`Compliance:178-184`, `:344-349`) | 37 (ROADMAP.md:740) | `documents/{id}/acceptances` là `Page`; không có số đếm theo phiên bản | MỤC |
| Báo vi phạm: luồng, hạn (`Compliance:192-198`) | 37 (ROADMAP.md:732) | `cases` (37:389) | API; chi tiết vụ việc CHƯA VẼ |
| "Bộ quy định vn · 2026.11", "Cập nhật bộ quy định" (`Compliance:55`, `Main:759`) | 37 | `GET/POST /rules`, chỉ nhân sự nền tảng ghi | API; nút ở phía khách lệch quyền (3.16) |

### 1.10 Orders.dc.html: phần khung chung (bước 51, 52)

| Khối / tính năng | Bước, mục | API | Trạng thái |
|---|---|---|---|
| View hệ thống, view của nút, view cá nhân; "Tuỳ chỉnh"; lưu view; sao chép liên kết (`Orders:80`, `:453-462`, `:598-599`) | 51 P7 | `/me/views` (51:228) | API (sao chép liên kết là UI) |
| Chọn cột, mật độ (`Orders:99-100`, `:270-271`, `:600-613`) | 51 P7 | `columns`, `density` | API |
| "Chọn cả N đơn khớp bộ lọc", bỏ qua đơn khác trạng thái, in phiếu bằng job (`Orders:155-158`, `:585-593`, `:616`) | 51 P8 | `kernel/bulk` (51:235) | API |
| Xuất CSV theo bộ lọc bằng job, tiến độ qua SSE, file giữ 7 ngày (`Orders:327-329`, `:621-636`) | 51 P8, 40 SSE | không có cơ chế xuất chung; mỗi module tự làm (41:237) | THIẾU cơ chế chung (từ khoá: export, xuất CSV, xuất dữ liệu) |
| Hoạt động và ghi chú, nhắc @, nhãn Agent (`Orders:175-188`, `:538-559`) | 51 P13, P12 | `records/{kind}/{id}/activity`, `notes` (51:281-283) | API |
| Đề xuất hoàn tiền của agent, Duyệt / Từ chối (`Orders:202`, `:562-565`) | 51 P12 | như 1.8 | MỤC, API thiếu (c2) |

### 1.11 Thiết kế 1 (lướt)

`Entries`, `Editor`, `Media` dùng khung CMS cũ. Các tính năng đều có API ở bước 16–18: SEO, đường dẫn tự chuyển
hướng, hẹn giờ, xem trước, gửi duyệt, tác giả, thẻ, thư mục, thùng rác, alt kèm gợi ý AI, thay file. Thiết kế 2
chưa vẽ lại trình soạn bài và thư viện media theo khung mới. Một chỗ nhỏ: lọc "Thiếu alt" (`Media.dc.html:146`)
chưa có tham số trên `GET /media`.

## 2. Chiều 2: từ lộ trình sang thiết kế

### 2.1 Bước 15–24 (đã xong)

| Mục (ROADMAP.md) | Có mặt người dùng? | Màn hình |
|---|---|---|
| 15: lọc, tìm không dấu, endpoint công khai, ETag (174-179) | Không | không cần màn hình |
| 16: thư viện media (186-198) | Có | Thiết kế 1 `Media`. Thiết kế 2: CHƯA VẼ lại (mục menu chỉ trỏ sang thiết kế 1, `Content:231`) |
| 17: bài và trang, nháp và bản đăng, hẹn giờ (209-212) | Có | `Content` (danh sách), `Editor` thiết kế 1 |
| 17: lịch sử phiên bản, khôi phục (213) | Có | CHƯA VẼ |
| 17: cây trang, chuyển nhánh (214) | Có | CHƯA VẼ (chỉ có mục menu "Trang", `Main:546`) |
| 17: đường dẫn, chuyển hướng làm tay, 410 (215-216) | Có | CHƯA VẼ; menu Nội dung không có mục "Chuyển hướng" |
| 17: danh mục, thẻ, **tác giả** (217) | Có | CHƯA VẼ; menu không có mục "Tác giả" |
| 17: menu điều hướng nhiều tầng (218) | Có | CHƯA VẼ |
| 17: đa ngôn ngữ, SEO, xem trước (219-222) | Có | `Content` (cột VI EN), `Editor` thiết kế 1 |
| 18: quyền cho agent, `ai_generated` (237-238) | Có | `Agent`, nhãn AI ở `Content` |
| 18: trạng thái nhà cung cấp AI `/ai/status`, hạn mức token (239-240) | Có | CHƯA VẼ (Cài đặt › AI, `Main:670`) |
| 18: SEO, alt, dịch, tìm theo ý nghĩa (241-245) | Có | alt và SEO ở thiết kế 1; nút "Dịch bằng AI" và tìm theo ý nghĩa CHƯA VẼ |
| 19: nền cho dữ liệu người đọc gửi lên (255-268) | Không | không cần màn hình (cấu hình) |
| 20: `forms` (273-277) | Có | CHƯA VẼ (chỉ có ô số và việc ở landing, `Main:696-697`) |
| 21: `newsletter` (281-286) | Có | CHƯA VẼ (danh sách, người đăng ký, chiến dịch, danh sách chặn) |
| 22: `comments` (291-296) | Có | CHƯA VẼ (hàng chờ duyệt, cấm, báo cáo) |
| 23 (300-307) | Không | không cần màn hình |
| 24: sổ module (313-314) | Có | Cài đặt › Module: CHƯA VẼ; `Platform` có danh sách module theo khách |

### 2.2 Bước 30, 31, 32, 50

Gần như không có mặt người dùng: CI, phát hành, file mã nguồn mở, WebP, deploy, SDK, test, bố cục, công khai repo.
Không cần màn hình. Riêng:

| Mục | Màn hình |
|---|---|
| 31: hướng dẫn nối AI agent qua MCP (ROADMAP.md:508) | Liên kết "Tài liệu API" ở `Devtools:67` và trong Trợ giúp: đủ, nội dung nằm ở `docs/` |
| 32: quản trị viên gỡ passkey của người khác (ROADMAP.md:546) | CHƯA VẼ (không có trang quản lý tài khoản nhân sự) |
| 32: `forms` xuất CSV lớn bằng job (ROADMAP.md:552) | CHƯA VẼ (cùng trang `forms`) |

### 2.3 Bước 33

| Mục (ROADMAP.md) | Màn hình |
|---|---|
| 569 sổ khách, placement, tên miền → khách | `Platform` (danh sách, chi tiết) |
| 572, 575, 578, 580, 583, 595 (ngữ cảnh, phân loại bảng, lọc, job, đo, thứ ngoài bảng) | không cần màn hình |
| 585 mã hoá secret | chỉ là chữ "mã hoá khi lưu, không hiện lại" (`Main:774`) |
| 592 cấu hình và secret theo khách | Cài đặt › Kết nối (cổng thanh toán, Email và SMS, hoá đơn, AI): CHƯA VẼ (chỉ có bảng ở landing) |
| 599 tên miền riêng, xác minh DNS TXT | Cài đặt › Tên miền: CHƯA VẼ (chỉ có việc "chưa xác minh", `Main:770`) |
| 602 `tenant_maintenance` cho từng database | Trạng thái lỗi toàn khung (bảo trì, tạm khoá, chỉ đọc): CHƯA VẼ |

### 2.4 Bước 34

| Mục | Màn hình |
|---|---|
| 613 cây, vòng đời nút (chờ duyệt, tạm khoá, lưu trữ), dời nút | `Org` có cây và thêm nút; dời nút và đổi trạng thái CHƯA VẼ |
| 616 membership có độ sâu | `Org` (lệch mô hình, 3.7); trang quản lý role và grant CHƯA VẼ |
| 619 cài đặt kế thừa | `Org` (chỉ đọc); sửa và quay về kế thừa CHƯA VẼ |
| 621 danh tính hai lớp | bộ chọn khách ở `Main`; danh sách người mua `/customers` chỉ có landing |
| 624 địa chỉ, đơn vị hành chính | Ô chọn tỉnh và xã cho địa chỉ nút và hồ sơ pháp lý: CHƯA VẼ |
| 633 uỷ quyền khách cha → con | CHƯA VẼ (chỉ có mục menu) |
| 635 đăng nhập bằng link | CHƯA VẼ (không có màn đăng nhập nào) |
| 638 lời mời | Hộp thoại ở `Org`; danh sách lời mời (gửi lại, thu hồi) và trang nhận lời mời CHƯA VẼ |
| 641 hồ sơ công khai của nút (giờ mở cửa, nhận tại quầy, bật công khai) | CHƯA VẼ (`Org` chỉ hiện giờ mở cửa như một cài đặt) |
| 645 nhân sự nền tảng vào dữ liệu | `Platform` (cấp quyền); phía khách: nhật ký vào (`GET /support-access`) và công tắc `auth.support_access` CHƯA VẼ |

### 2.5 Bước 35

| Mục | Màn hình |
|---|---|
| 656 gói = module + tính năng + hạn mức + giá | `Platform` có thẻ gói; trang sửa gói và giá CHƯA VẼ |
| 659 chặn theo khách | nhãn khoá trên menu ở `Main` |
| 662 vòng đời thuê bao | cột Thuê bao ở `Platform`; băng báo quá hạn hay chỉ đọc phía khách CHƯA VẼ |
| 665 hoá đơn thuê bao | phía khách (kỳ thanh toán, trả tiền, hồ sơ xuất hoá đơn) và phía vận hành (đánh dấu đã thu, huỷ kỳ) CHƯA VẼ |
| 667 cấp phát, khách tự đăng ký | Hộp thoại cấp phát ở `Platform`; trang tự đăng ký và theo dõi tiến độ cấp phát (thử lại) CHƯA VẼ |

### 2.6 Bước 37

| Mục | Màn hình |
|---|---|
| 719 KYC | `Compliance` tab KYC |
| 723 hồ sơ pháp lý, website, logo | `Compliance` tab Website; form "Thông tin chủ sở hữu, mã số thuế" (bước thiết lập, `Main:858`) CHƯA VẼ |
| 728 trang bắt buộc có mẫu | `Content` và `Compliance` |
| 732 báo vi phạm | `Compliance` (danh sách); chi tiết vụ việc và phía bị phản ánh CHƯA VẼ |
| 735 xác thực tài khoản đăng nội dung | việc ở landing Khách hàng; trang chính sách (`posting.*`) CHƯA VẼ |
| 737 dữ liệu cá nhân: sổ mục đích, hạn lưu, nơi lưu, sự cố | `Compliance` có sự cố và yêu cầu (3.15); mục đích, sổ xử lý, hạn lưu, nơi lưu CHƯA VẼ |
| 740 văn bản có phiên bản | `Compliance` (danh sách); phát hành bản mới, bắt chấp nhận lại CHƯA VẼ |
| 744 bảo vệ người tiêu dùng (kênh khiếu nại, hạn xử lý) | Gộp vào tab Báo vi phạm; khiếu nại của người tiêu dùng chưa thấy riêng |
| 747 nhãn AI | `Content`; danh sách "Nội dung do AI tạo" (`Main:552`, `ailabel`) CHƯA VẼ |
| 750 SMS | Cài đặt › Email và SMS: CHƯA VẼ |
| 752 `notifications` | chuông ở `Main`; trang tuỳ chọn nhận, mẫu tin, nhật ký tin đã gửi CHƯA VẼ |

### 2.7 Bước 38, 39, 40

| Mục | Màn hình |
|---|---|
| 38 / 764 kho báo cáo gộp | landing Báo cáo (số mẫu là TM); trang báo cáo chi tiết và độ trễ `GET /lag` CHƯA VẼ |
| 38 / 767 cha thao tác trên con, phân phối | CHƯA VẼ |
| 38 / 769 tách, gộp nhánh | Hộp thoại ở `Org` (lệch quyền, 3.6); phía `Platform` CHƯA VẼ |
| 39 / 783-801 backup, số đo tồn đọng, xoay khoá, sổ tay | `GET /ops/backlog`, `/ops/backups` có mặt người dùng → `Platform` › Sức khoẻ CHƯA VẼ; phía khách (job của khách mình) gộp vào trang Job. Còn lại không cần màn hình |
| 39 / 809-830 hiệu năng | không cần màn hình |
| 40 / 844 OIDC | nút đăng nhập bằng Google và các nhà cung cấp khác, danh tính liên kết trong hồ sơ, trang quản trị nhà cung cấp: CHƯA VẼ |
| 40 / 848 tải thẳng lên S3 | không cần màn hình (hành vi của ô tải lên) |
| 40 / 852 header bảo mật, 866 rate limit | không cần màn hình |
| 40 / 856 SSE | dùng cho tiến độ job (`Orders:622`) và số đếm |
| 40 / 862 MCP resources, prompts, `/llms.txt` | trợ giúp của `Agent` nói "Kết nối qua MCP" (`Agent:45`); không cần màn hình riêng |

### 2.8 Bước 51, 52

| Mục (ROADMAP.md) | Màn hình |
|---|---|
| 1122 nhúng bản ghi trong nội dung | khối nhúng trong trình soạn bài CHƯA VẼ (mặt tiền thuộc agent kia) |
| 1125 sổ điều hướng | `Main` |
| 1128 hộp việc | `Main`; trang "Việc cần làm" đầy đủ (mục `inbox`) CHƯA VẼ |
| 1132 tìm | `Main` (Ctrl+K) |
| 1134 số liệu tóm tắt | `Main` |
| 1136 hoạt động gần đây | `Main` (khối); trang "Hoạt động" đầy đủ (mục `act`) CHƯA VẼ |
| 1138 khai cho mọi module | không cần màn hình |
| 1140 view đã lưu | `Orders`, `Content` |
| 1143 thao tác hàng loạt | `Orders` |
| 1146 tìm nâng cao | `Main` |
| 1149 hộp việc theo người | `Main` có hoãn và "vì sao"; **giao việc** CHƯA VẼ |
| 1152 giải thích quyền | toast khoá ở `Main:573`; nút bị vô hiệu kèm lý do ở các màn khác chưa thấy |
| 1155 agent, lệnh cần duyệt | `Agent`, `Orders`, hộp việc ở `Main` |
| 1159 dòng hoạt động và ghi chú | `Orders` |
| 1170 bàn kỹ thuật | `Devtools` |
| 1173 `restore`, `dry_run` | toast Hoàn tác ở `Main`; xem trước `dry_run` khi thao tác hàng loạt hay nhập CSV CHƯA VẼ |
| 1174 AI trong quản trị | điểm vào "Hỏi trợ lý AI" trong Trợ giúp; ô lọc bằng lời thường CHƯA VẼ |
| 1176 thông báo, ghim | chuông ở `Main`; ghim (`/me/pins`) CHƯA VẼ (khung chỉ có "Gần đây") |
| 1178 sức khoẻ tuân thủ / người bán | `Compliance` tab Tình trạng và đồng hồ ở `Main:764`; phần người bán là TM |

### 2.9 Màn hình chung có mặt người dùng mà lộ trình đã có API nhưng thiết kế 2 chưa vẽ (gom)

Đăng nhập, MFA, passkey, quên mật khẩu, link đăng nhập, nhận lời mời, OIDC (bước 2, 3, 12, 34, 40). Hồ sơ của tôi:
đổi email, phiên đăng nhập, xuất dữ liệu của tôi, xoá tài khoản. Quản lý tài khoản nhân sự (`/auth/users`): khoá,
đặt lại mật khẩu, gỡ MFA, đăng xuất mọi phiên. Role và permission. Webhook (tạo, chọn event, đổi secret). Nhật ký audit
đầy đủ, lọc theo loại người thực hiện.

## 3. Kiểm chéo: chỗ thiết kế mâu thuẫn với đặc tả

| # | Chỗ lệch | Thiết kế | Đặc tả / quyết định | Đề xuất |
|---|---|---|---|---|
| 3.1 | Nguồn của hộp việc | "mỗi module đẩy việc lên qua event" (`Map:148`) | việc tính từ dữ liệu, không có bảng việc (D-200 điểm 5; 51:99) | Sửa chữ trong thiết kế |
| 3.2 | Nút "Đã xử lý" trên việc | đánh dấu xong tại chỗ, có Hoàn tác (`Main:131`, `:268`, `:840-843`) | không có trạng thái "bỏ qua"; xử lý ở nơi gốc thì việc tự hết (51:99-100); P10 chỉ có hoãn và giao | Thay bằng nút hành động thật (mở bản ghi, duyệt ngay) cộng Hoãn và Giao |
| 3.3 | Nội dung theo website và nút | mỗi tên miền một website gắn với một nút; cột "Nút sở hữu"; cửa hàng viết bài của mình (`Content:65`, `:131`; `Main:707-710`) | `content`, `media`, `forms`, `newsletter`, `comments` chỉ có dữ liệu cấp khách, quyền chỉ ở độ sâu `tenant` (34:60). D-200 mục 4 cũng không liệt kê chỗ thiếu này | Quyết định: thêm vào lộ trình (b1) hoặc thiết kế bỏ cột nút. Hiện là chỗ thiếu lớn nhất |
| 3.4 | "Mô hình" của khách | lọc phân hệ, việc thiết lập và cấp phát theo mô hình (`Map:141`, `Main:532-537`, `:867-872`, `Platform:185`) | `SetupStep.models` (51:257) nhưng không bảng nào lưu mô hình; 35 cấp phát không có trường này | Bỏ khái niệm "mô hình" khỏi core (trung lập): `SetupStep` áp theo module hoặc tính năng đã bật. Thiết kế suy "mô hình" từ gói |
| 3.5 | "Chặn bán" | "chưa xong thì website chưa nhận đơn" (`Main:146`) | `blocking` chỉ là một cờ (51:257); không bước nào chặn đặt hàng theo việc thiết lập | Hoặc đổi chữ thành "bắt buộc theo luật trước khi bán", hoặc thêm cổng chặn ở 45 / 37 (TM; cần đối chiếu NĐ 52/2013 sửa bởi NĐ 85/2021 xem bán trước khi thông báo có bị cấm không) |
| 3.6 | Quyền thao tác nhạy cảm | Khách tự bấm "Tách thành khách riêng" (`Org:81`); khách "Thu hồi" quyền vào hỗ trợ (`Main:752`); nền tảng cấp 24 giờ (`Platform:324`) | Tách và gộp chỉ do nhân sự nền tảng làm (38:286); thu hồi support grant chỉ phía vận hành (34:331); tối đa mặc định 8 giờ (34:366) | Tách: chuyển nút sang `Platform` hoặc đổi thành "Gửi yêu cầu tách". Thu hồi: thêm quyền cho khách (b5). 24 giờ: đổi thành 1 / 4 / 8 giờ |
| 3.7 | Độ sâu gắn ở đâu | chọn độ sâu khi mời; mỗi dòng thành viên có độ sâu riêng (`Org:153-155`, `:258-271`) | độ sâu nằm trên grant của role (`auth_role_permission.scope`, 34:176, 251, 316) | Sửa thiết kế: mời chỉ chọn role và nút; độ sâu hiện như thuộc tính của role (chỉ đọc), kèm liên kết sang trang role |
| 3.8 | Thống kê newsletter | "Tỉ lệ mở 42%, bấm liên kết 7%" (`Main:700`) | `newsletter` không có pixel hay đếm bấm; khảo sát ghi lượt mở không đáng tin (Apple MPP), Ghost và Listmonk mặc định tắt (`docs/research/forms-newsletter-comments.md:80`) | Bỏ khỏi thiết kế; thay bằng số đã gửi, lỗi, bounce, huỷ đăng ký (đều có dữ liệu) |
| 3.9 | Ngôn ngữ của người nhận | VI / EN theo người; thông báo @, duyệt, job xong gửi chậm (`Main:956-961`) | D-062 hoãn "ngôn ngữ ưa thích"; mail theo ngôn ngữ của request (DECISIONS.md:1272-1282) | Thêm ngôn ngữ ưa thích (b4); thông báo chậm không có request để lấy ngôn ngữ |
| 3.10 | Ai bật, tắt module | Cài đặt › Module "bật, tắt" phía khách (`Map:208`); "phiên bản" (`Main:668`) | chỉ nhà vận hành ghi đè (35:305); chế độ `single` không đổi gì; `ModuleInfo` không có phiên bản | Thiết kế: phía khách chỉ xem (đang bật, lý do khoá, gói mở được). Hoặc thêm "khách tự tắt module có trong gói" (b9, thấp) |
| 3.11 | Thao tác đội database và tạm khoá | nút "Migrate cả đội", "Chuyển sang database riêng"; "Tạm khoá (chỉ đọc), vẫn đăng nhập và xuất được" (`Platform:58`, `:131`, `:165-170`) | migrate `--all` và `tenant move` chỉ có CLI (33:365, 377); `PATCH /tenants/{id}` `suspended` chặn mọi route (33:338); chỉ đọc là trạng thái thuê bao (35:169, 275) | Hai nút đội database: hoặc hiện lệnh CLI cần chạy, hoặc thêm API job (c6). Tạm khoá: tách hai nút "Tạm khoá hẳn" (kernel) và "Chỉ đọc" (thuê bao), hoặc thêm `POST /tenants/{id}/subscription/suspend` |
| 3.12 | Hạn mức nút | "4 / 10 cửa hàng" chỉ đếm cửa hàng (`Org:163`) | `reserve("organizations.units")` đếm mọi nút (34:295) | Ghi rõ trong đặc tả 35: hạn mức theo loại nút, hoặc đổi chữ trong thiết kế thành "nút" |
| 3.13 | Form tạo API key | chỉ có tên, loại, permission (`Agent:132-143`) | bắt buộc `current_password`; có `expires_at` (openapi) | Thêm ô mật khẩu (hay xác nhận lại bằng passkey, xem c4) và ô hạn dùng |
| 3.14 | Trạng thái KYC | "Thiếu giấy tờ", "Đã yêu cầu bổ sung" (`Compliance:287`) | draft/pending/verified/rejected/expired/revoked (37:245); không có "cần bổ sung" | Hoặc thiết kế dùng `rejected` kèm `reason_code`, hoặc thêm trạng thái `needs_info` vào 37 (c9) |
| 3.15 | Yêu cầu của chủ thể dữ liệu | hàng chờ có duyệt rồi mới tạo gói xuất (`Compliance:302`, `Main:815`) | người dùng tự xuất, tự xoá ngay qua `/auth/me/export`, `DELETE /auth/me`; không có sổ yêu cầu | Tự phục vụ thì không cần duyệt (đừng làm chậm quyền của chủ thể). Sổ yêu cầu chỉ dành cho yêu cầu qua kênh khác (email, form, người không có tài khoản), có hạn trả lời (b3) |
| 3.16 | Cập nhật bộ quy định | nút ở landing Tuân thủ phía khách (`Main:759`) | `POST /rules` chỉ nhân sự nền tảng (37:394) | Phía khách chỉ hiện "đang ở bản X, mới nhất / có bản mới" |
| 3.17 | Tên permission trong dữ liệu mẫu | `compliance.manage`, `plans.read`, `tax.manage`, `privacy.erase` (`Main:824`; `Agent:206`, `:217`) | không có các tên này; tên thật dạng `compliance.legal.manage`, `plans.billing.read`… | Sửa dữ liệu mẫu cho khớp, vì "vì sao bạn thấy" hiện đúng tên permission |
| 3.18 | Nhãn nhóm đổi theo mô hình | "Khách hàng" thành "Người đọc" khi chỉ có nội dung (`Main:547`) | `NavGroup.label` là một i18n key cố định (51:47) | Giữ một nhãn, hoặc để giao diện tự đổi nhãn |

## 4. Tổng hợp hành động

Quy ước: **[chưa tới]** = bước chưa làm, chỉ cần vẽ hay đặc tả thêm khi tới bước. **[thiếu hẳn]** = không có
ở đâu trong lộ trình hay đặc tả.

### (a) Thiết kế cần vẽ thêm hoặc sửa: 22 dòng

| # | Việc | Căn cứ |
|---|---|---|
| a1 | Sửa hộp việc: bỏ nút "Đã xử lý", thay bằng hành động thật, thêm **Giao cho…**; sửa chữ ở Map | `Main:131,268,840`; `Map:148`; 51:99, 254 |
| a2 | Sửa mời thành viên và bảng thành viên: bỏ chọn độ sâu khi mời, hiện độ sâu theo role. Vẽ **trang Role** (grant permission × độ sâu) | `Org:153-155`; 34:316 |
| a3 | Vẽ **màn đăng nhập và tài khoản**: đăng nhập (mật khẩu, passkey, link email, OIDC), MFA, quên mật khẩu, nhận lời mời; hồ sơ của tôi (email, phiên, passkey, danh tính liên kết, xuất dữ liệu, xoá tài khoản) | ROADMAP.md:635, 638, 844; openapi `/auth/me/*` |
| a4 | Vẽ **trạng thái toàn khung**: khách bị tạm khoá, chỉ đọc (thuê bao quá hạn), bảo trì 503, module chưa bật | 33:338; 35:169, 357 |
| a5 | Nội dung: quyết theo b1. Trong lúc chờ, ghi chú trên `Content` rằng cột "Nút sở hữu" và bộ chọn website cần mục b1 | `Content:65-131` |
| a6 | Sửa `Platform`: hai nút đội database (lệnh CLI hay job, theo c6); tách "Tạm khoá" và "Chỉ đọc"; thời hạn hỗ trợ 1 / 4 / 8 giờ; bỏ trường "Mô hình" khi cấp phát hoặc suy từ gói | `Platform:58,131,165-170,185,324` |
| a7 | Chuyển "Tách thành khách riêng" sang `Platform` (hoặc thành "Gửi yêu cầu") | `Org:81`; 38:286 |
| a8 | Bỏ "Tỉ lệ mở / bấm" của newsletter, thay bằng số liệu có thật | `Main:700` |
| a9 | Sửa form tạo key: thêm xác nhận lại danh tính và hạn dùng | `Agent:132-143`; openapi `CreateApiKeyRequest` |
| a10 | Sửa Tuân thủ › Dữ liệu cá nhân: tách yêu cầu tự phục vụ (chỉ hiện để theo dõi) khỏi yêu cầu qua kênh khác (cần xử lý, có hạn); KYC dùng trạng thái của đặc tả | `Compliance:141-147,287`; 37:245 |
| a11 | Vẽ **trang Job** (Cài đặt › Job): danh sách, tiến độ, lỗi, dead-letter, chạy lại; dùng chung cho tải file xuất | `Main:671`; `Devtools:119` |
| a12 | Vẽ **Cài đặt › Tên miền** (thêm, bản ghi TXT, xác minh lại, gắn nút) và **Cài đặt › Kết nối** (SMTP, SMS, AI, cổng, hoá đơn; không hiện lại secret; nút kiểm tra) | ROADMAP.md:592, 599 |
| a13 | Vẽ trang con của Tổ chức: Lời mời (gửi lại, thu hồi), Uỷ quyền, loại nút, sửa cài đặt nút (ghi đè, quay về kế thừa), dời nút và trạng thái nút, **hồ sơ công khai** (giờ, nhận tại quầy, địa chỉ có ô tỉnh và xã) | ROADMAP.md:613, 619, 633, 638, 641 |
| a14 | Vẽ phía khách của quyền vào hỗ trợ: nhật ký các lần vào, công tắc tắt, nút thu hồi (khi có b5) | ROADMAP.md:645; 34:330 |
| a15 | Vẽ `Platform`: sửa gói và giá, Thuê bao (kỳ thanh toán, đánh dấu đã thu), Cấp phát (tiến độ, thử lại), Sức khoẻ (`ops/backlog`, `ops/backups`), Đội database (placement, `schema_state`) | ROADMAP.md:656, 665, 667, 783, 791 |
| a16 | Vẽ phía khách của Gói và thuê bao: thuê bao, mức dùng, kỳ thanh toán, trả tiền, hồ sơ xuất hoá đơn, đổi hay huỷ gói | 35:301-308 |
| a17 | Vẽ màn của `forms`, `newsletter`, `comments`: các module đã chạy, chưa có màn nào | ROADMAP.md:273-296 |
| a18 | Vẽ lại trình soạn bài và thư viện media theo khung 2; thêm trang Trang (cây), Danh mục và thẻ, Tác giả, Menu, **Chuyển hướng**, Lịch sử phiên bản; thêm "Tác giả" và "Chuyển hướng" vào menu Nội dung | ROADMAP.md:213-218; `Main:546` |
| a19 | Vẽ Tuân thủ còn thiếu: hồ sơ pháp lý (chủ sở hữu, mã số thuế), mục đích và sổ xử lý, hạn lưu, chính sách đăng nội dung, phát hành văn bản mới, chi tiết vụ việc, danh sách nội dung có nhãn AI | ROADMAP.md:723, 735, 737, 740, 747 |
| a20 | Vẽ trang quản lý nhân sự (khoá, gỡ MFA và passkey, đăng xuất mọi phiên), Webhook (tạo, chọn event, đổi secret), Nhật ký audit đầy đủ (lọc theo loại người thực hiện) | openapi `/auth/users`, `/webhooks`, `/audit`; ROADMAP.md:546 |
| a21 | Vẽ trang "Việc cần làm" và "Hoạt động" đầy đủ; ghim (`/me/pins`); ô lọc bằng lời thường; màn xem trước `dry_run` cho thao tác hàng loạt và nhập CSV; tuỳ chọn nhận thông báo | ROADMAP.md:1128, 1136, 1173-1176 |
| a22 | Sửa nhỏ: tên permission mẫu theo đặc tả; nhãn nhóm cố định; Cập nhật bộ quy định chỉ đọc phía khách; Module phía khách chỉ đọc; hạn mức "nút" hay "cửa hàng" | 3.10, 3.12, 3.16, 3.17, 3.18 |

### (b) Lộ trình cần bổ sung: 10 dòng, chỉ phần core hay module trung lập

| # | Mục đề xuất | Bước | Loại | Căn cứ |
|---|---|---|---|---|
| b1 | **Nội dung theo nút và theo website**: `org_id` (nút sở hữu) cho `content`, `media` (và `forms`); quyền theo độ sâu; đọc công khai lọc theo website (tên miền → nút, `kernel_tenant_domains.org_id` đã có); menu và ngôn ngữ theo website. Seller của sàn cũng cần media riêng theo nút (giao với TM 46). Thay quyết định ở 34:60 bằng mục DECISIONS mới | 34 (mục mới) hoặc 51 | thiếu hẳn | `Content:65-131`; 34:60; D-200 mục 4 không có. Từ khoá: org_id, website, site_id, nhiều website, đa website, multi-site |
| b2 | **Job API chung**: xem trạng thái và tiến độ, liệt kê job theo khách, dead-letter, chạy lại có audit; file kết quả giữ có hạn kèm link ký. Dùng cho P8 và cho xuất | 51 (P8 đã dựa vào nó) hoặc 52 mục 1 | thiếu hẳn (51:239 dùng mà không định nghĩa) | `Devtools:119`; `Main:671`; `Orders:621-636` |
| b3 | **Sổ yêu cầu của chủ thể dữ liệu**: yêu cầu xuất, sửa, xoá, rút đồng ý đến từ kênh ngoài tự phục vụ; hạn trả lời là dữ liệu có ngày hiệu lực (Luật 91/2025, NĐ 356/2025: cần đối chiếu văn bản gốc); nối với cơ chế xuất, xoá đã có | 37 | thiếu hẳn | `Compliance:141-147` |
| b4 | **Ngôn ngữ (và múi giờ) ưa thích của người dùng** trong `auth`; `notifications` và mail gửi chậm dùng nó | 37 (cạnh `notifications`) hoặc 34 | thiếu hẳn (D-062 hoãn) | `Main:59-60`; DECISIONS.md:1280 |
| b5 | **Khách tự thu hồi một lần cấp quyền vào hỗ trợ** đang còn hạn (quyền của bên kiểm soát dữ liệu) | 34 | thiếu hẳn | `Main:752`; 34:331 |
| b6 | **Xác nhận lại danh tính cho thao tác nhạy cảm** bằng passkey, TOTP hay link, không chỉ mật khẩu. Tài khoản không mật khẩu (passkey, link, OIDC) hiện không tạo được API key | 34 hoặc 40 | thiếu hẳn (từ khoá: current_password, re-auth, reauth, step-up, sudo, xác nhận lại) | openapi `CreateApiKeyRequest`; `modules/auth/src/http/account.rs:84` |
| b7 | **Cấu hình hành vi của module theo khách**: áp `Module::tenant_settings` cho từ cấm và số link của `comments`, hạn lưu của `forms`, ngôn ngữ bật của `content`… (hiện chỉ là biến môi trường chung) | 33 (mục 592 mở rộng) | thiếu hẳn (33 chỉ áp cho `mail`, `ai`, 33:403) | `Main:653`; `modules/comments/src/config.rs:66-93` |
| b8 | **Xuất danh sách theo bộ lọc dùng chung**: dùng schema bộ lọc P7, chạy qua job (b2), file CSV qua `autoxide_kernel::csv` (bước 41 dời vào), báo xong qua `notifications`; module khai cột xuất | 51 (cạnh P8) | thiếu hẳn (mỗi module tự làm: forms ROADMAP.md:552, catalog 41:237) | `Orders:327`; `Main:713`; `Map:193` |
| b9 | **Xuất toàn bộ dữ liệu của một khách** khi huỷ thuê bao (gói tải về qua job; nhà vận hành và chính khách gọi được) | 35 | thiếu hẳn (35:282 chỉ nói "qua API") | `Platform:164`; `Map:206` |
| b10 | Thấp: chính sách bắt buộc MFA theo permission hay role; `last_login_at` và việc "tài khoản lâu không đăng nhập"; báo cáo đã lưu và lịch gửi qua email (`reporting`); thông báo của nền tảng tới quản trị mọi khách; cấu hình liên hệ hỗ trợ của nền tảng; khách tự tắt module có trong gói | 34, 38, 37, 35 | thiếu hẳn | `Main:752,740,960`; `Map:208` |

### (c) API thiếu, cần đề bài cho teammate: 12 dòng, không tính phần đã có đề bài

| # | Endpoint / trait / bảng gợi ý | Bước | Lý do | Loại |
|---|---|---|---|---|
| c1 | `GET /api/v1/kernel/jobs?status=&kind=` · `GET /kernel/jobs/{id}` (`progress`, `result`, `file`) · `POST /kernel/jobs/{id}/retry`; cột tiến độ trong bảng job; permission đọc và ghi | 51 / 52 | P8 dẫn tới endpoint không có (51:239); Devtools, xuất file, cài đặt Job | thiếu hẳn |
| c2 | `GET /kernel/approvals?status=` · `GET /kernel/approvals/{id}` · `POST …/reject {reason}`; cách agent nhận kết quả (đọc lại hay event SSE `kernel.approval_decided`); tool MCP tương ứng | 51 P12 | đặc tả chỉ có `approve` (51:274) | chưa tới, đặc tả hở |
| c3 | `content`: `org_id`, bộ lọc `owner_org`, `site`; quyền theo độ sâu; công khai lọc theo website (chi tiết theo b1) | 34 hoặc 51 | b1 | thiếu hẳn |
| c4 | Xác nhận lại danh tính: `POST /auth/me/reauth` (passkey, TOTP, link) trả token ngắn hạn, các lệnh nhạy cảm nhận token này thay cho `current_password` | 34 / 40 | b6 | thiếu hẳn |
| c5 | Tham số lọc danh sách `content/entries`: `review=pending`, `scheduled=true` (sắp theo giờ hẹn), `ai_generated`, `missing_locale=en`; `GET /media?missing_alt=true`. Khai qua schema bộ lọc P7 | 51 P7 (chạy trên 17, 16) | tab và view của `Content`, `Media:146` | chưa tới |
| c6 | Hoặc giữ CLI và ghi rõ trong DECISIONS, hoặc thêm `POST /kernel/placements/migrations` (job, chạy lại được) và `POST /kernel/tenants/{id}/moves {placement}` (job, chỉ nhân sự nền tảng, dry-run mặc định như 38:287) | 33 | nút ở `Platform:58,131` | thiếu hẳn (cần quyết) |
| c7 | Danh sách khách tổng hợp cho mặt điều khiển: `GET /kernel/tenants?placement_kind=&schema_state=&subscription_status=&near_limit=` hoặc `expand=subscription,usage,placement`, tránh gọi riêng cho từng dòng | 33 / 35 | cột và bộ lọc của `Platform:82-92,259` | chưa tới, đặc tả hở |
| c8 | Sổ yêu cầu chủ thể dữ liệu: bảng `compliance_subject_requests` (`kind`, `channel`, `subject`, `received_at`, `due_at`, `status`, `job_id`), `GET/POST /compliance/subject-requests`, `POST …/{id}/fulfil`, việc trong hộp việc | 37 | b3 | thiếu hẳn |
| c9 | Nhỏ cho 37: trạng thái KYC `needs_info` hay `POST /kyc/profiles/{id}/request-info`; số người đã chấp nhận theo phiên bản (`GET /documents/{id}/versions` kèm `acceptance_count`); bản nháp thông báo sự cố cho cơ quan và cho người bị ảnh hưởng (`GET /incidents/{id}/notice?audience=`) | 37 | `Compliance:127-128,179,155` | chưa tới, đặc tả hở |
| c10 | Nhỏ cho 52 và 51: tên endpoint cho nhật ký request, gửi lại webhook, event theo bản ghi; permission `kernel.devtools.manage` cho lệnh ghi; lưu `request_id` và `trace_id`; event theo bản ghi đọc từ audit hay giữ outbox đủ lâu (event đã giao bị dọn, ROADMAP.md:60-62); `DELETE /me/work/{key}/snooze` để bỏ hoãn; `GET /memberships?org_id&inherited=true` | 52, 51 P10, 34 | `Devtools:85-139`; `Main:835`; `Org:91` | chưa tới, đặc tả hở |
| c11 | Nhỏ: `GET /kernel/settings` thêm `last_used_at`, `last_error`; `POST /kernel/settings/{name}/test` (gọi qua trait của module, không gọi trong transaction ghi); `requires_approval_for_agent` trong `ModuleTool` và danh mục permission | 33, 51 P12 | `Main:771-772`; `Agent:233` | chưa tới |
| c12 | Trợ lý AI trong quản trị: endpoint hội thoại (vd `POST /api/v1/ai/assistant/messages {page, text}`) chạy tool MCP với quyền của người gọi, chỉ đọc mặc định, lệnh ghi thành bản nháp hay yêu cầu duyệt (D-109) | 52 mục 3 | `Content:48` "Hỏi trợ lý AI"; 52:33-38 chưa nêu endpoint | chưa tới, đặc tả hở |

Đã có đề bài, không lặp lại: tên tool MCP trong nhật ký request ([ĐB-gaps] E); `/me/flags` và định nghĩa số
liệu ([ĐB-help]); hỏi đáp sản phẩm, alt theo ngôn ngữ, trả hàng, đơn vãng lai ([ĐB-gaps] A–D, phần của agent kia).

## 5. Điều chưa chắc

- 3.5 "Chặn bán": chưa rõ luật có cấm bán trước khi thông báo Bộ Công Thương hay chỉ phạt. Cần đối chiếu NĐ
  52/2013 sửa bởi NĐ 85/2021 và Luật TMĐT 2025 (người có chuyên môn pháp lý).
- b3, c8: hạn trả lời yêu cầu của chủ thể dữ liệu theo Luật 91/2025 và NĐ 356/2025 chưa kiểm văn bản gốc.
- b1: có thể chủ dự án chủ ý giữ nội dung ở cấp khách (34:60). Khi đó thiết kế phải sửa, không phải lộ trình.
  Cần chủ dự án hay agent bước 34 quyết và ghi DECISIONS.
- c6: để migrate và dời khách chỉ chạy bằng CLI có thể là chủ ý an toàn (33:416 ghi "tạo placement chỉ bằng
  CLI"). Chưa thấy quyết định nói rõ cho migrate và move.
- Chiều 2 dựa trên markup và JS. Những mục con chỉ có tên trên menu (vd `Main:546`) được tính là CHƯA VẼ.
