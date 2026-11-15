# Bước 54 — Phân quyền chuẩn hàng đầu (kernel, `auth`)

> Lộ trình: `docs/ROADMAP.md` bước 54 (và hai mục ở bước 31) · Quyết định: D-205, D-232 (khách duyệt từng lần
> vào hỗ trợ, xem như người dùng) · Nền: D-046 (RBAC), D-190, D-211 (quyền vào hỗ trợ), đặc tả 33 (khách),
> 34 (quyền theo độ sâu, quyền vào hỗ trợ), 35 (gói), 37 (dữ liệu cá nhân, KYC), 51 (P11 `/me/can`, P12 lệnh
> chờ duyệt), 52 (nhật ký request)

## Mục tiêu

Phân quyền của Autoxide ngang các nền tảng lớn và hơn họ ở chỗ: **thu hồi có hiệu lực ngay**, **lọc danh
sách luôn khớp với kiểm từng bản ghi** (chứng minh bằng test), **hợp đồng quyền đọc được bằng máy** (gate
chứng minh không endpoint nào hở), **giải thích và mô phỏng được mọi quyết định**, và **sinh ra cho AI
agent** (quyền tối thiểu, hành động thay người dùng có vết, lệnh rủi ro chờ người duyệt) — chạy như nhau trên
SQLite và Postgres, mã nguồn mở.

Khách kiểm soát ai vào dữ liệu của mình: nhân sự nền tảng chỉ vào khi khách **duyệt từng lần** (mặc định cho khách
mới), và để gỡ lỗi quyền, hiển thị thì có **xem như người dùng** — chỉ đọc, không bao giờ làm thay (D-232).

## Nền tảng lớn đang làm gì, ta lấy gì, ta hơn ở đâu

| Nền tảng | Họ làm | Ta lấy | Ta làm hơn |
|---|---|---|---|
| NIST RBAC (chuẩn) | Role ↔ permission ↔ người | Đã có (D-046) | Permission do module khai, kernel kiểm lúc khởi động |
| Microsoft Dynamics 365 | Quyền có độ sâu: người dùng / đơn vị / nhánh / tổ chức | Đặc tả 34 (`own/unit/subtree/tenant`) | Lọc danh sách là điều kiện SQL, có test so khớp với kiểm từng bản ghi (P2) |
| Google Zanzibar, Google Drive | Quan hệ người–bản ghi (chia sẻ), kiểm tra nhất quán, "ai có quyền" | Chia sẻ bản ghi có hạn (P3), `who-can` (P9) | Không đồ thị sâu tuỳ ý: quan hệ một bước + cây tổ chức → mọi câu hỏi là một câu SQL có index, không bài toán "list objects" chậm |
| Salesforce | Profile + permission set, chia sẻ theo luật, quyền theo trường (FLS) | Quyền theo trường dữ liệu (P8) | Phân loại trường in ngay trong OpenAPI; đọc dữ liệu nhạy cảm có audit (Luật 91/2025) |
| AWS IAM | Deny thắng allow, ranh giới quyền, SCP rào cả tài khoản con, Policy Simulator, Access Analyzer (quyền không dùng) | Deny, rào khách con, mô phỏng, quyền không dùng (P4, P9) | Không ngôn ngữ chính sách tự do: luật là dữ liệu có kiểu, kiểm được lúc lưu, người thường đọc hiểu |
| GitHub | Token chi tiết: một chủ, chọn tài nguyên, bắt buộc hạn, chủ tổ chức duyệt token; chế độ "sudo" hỏi lại mật khẩu | API key chi tiết, hết hạn bắt buộc, duyệt key (P4); xác thực lại cho việc nhạy cảm (P5) | Quyền của key = giao với quyền **hiện tại** của chủ (đã có), key của agent có loại riêng |
| Microsoft Entra PIM | Quyền "đủ điều kiện", kích hoạt khi cần, có hạn, MFA, lý do, người duyệt; rà soát quyền định kỳ | Nâng quyền đúng lúc (P6), rà soát quyền (P9) | Kích hoạt, duyệt, rà soát là API + tool MCP, vào hộp việc (bước 51) |
| Ngân hàng, kế toán | Hai người (maker–checker), tách nhiệm vụ | P7 | Ngưỡng là dữ liệu có ngày hiệu lực; dùng chung cơ chế duyệt của bước 51 |
| Shopify, Stripe | Quyền nhân viên theo cửa hàng; key hạn chế; ứng dụng bên thứ ba có phạm vi | Đã có ở 34 (theo nút) và P4 | — |
| OpenID AuthZEN Authorization API 1.0 (chuẩn cuối 01/2026) | API chuẩn để hỏi "chủ thể X được làm Y trên Z không" | Endpoint AuthZEN (P10) | Cổng API, dịch vụ khác, agent hỏi Autoxide bằng chuẩn mở thay vì API riêng |
| OAuth 2.0 Step-up (RFC 9470), Token Exchange (RFC 8693) | Đòi xác thực mạnh hơn; hành động thay người khác có chuỗi `act` | P5, P10 | Audit ghi "agent A thay người dùng B" |
| Shopify (tài khoản cộng tác), Salesforce (Grant Login Access, Login As) | Đối tác / hỗ trợ xin quyền vào, chủ cửa hàng duyệt; quản trị đăng nhập thay người dùng | Khách duyệt từng lần (P12), xem như người dùng (P13) | Xem như **chỉ đọc**, không bao giờ làm thay; người mua phải tự cấp phép; mọi request ghi cả hai danh tính (D-232) |

## Bắt đầu khi

- P1 (hợp đồng quyền đọc được bằng máy + test độ phủ) **làm ngay ở bước 31**, không chờ bước này.
- Phần còn lại SAU bước 34 (quyền theo độ sâu). P7, phần duyệt của P6 dùng cơ chế duyệt của P12 bước 51;
  P8 làm cùng mục dữ liệu cá nhân của bước 37 nếu bước đó chưa xong.
- Song song được với 41–49; module thương mại khai phần của mình (P3, P7, P8) ngay trong bước của nó khi
  bước 54 đã có, không thì bước 54 khai thay cho module đang có.
- P12, P13 (D-232) làm cuối bước, sau P5 (step-up): cần quyền vào hỗ trợ của bước 34 (mục 13), yêu cầu duyệt
  (P12 bước 51) và nhật ký request (`kernel_request_log`, mục 1 bước 52).

## Thiết kế

### P1 — Hợp đồng quyền đọc được bằng máy, gate chứng minh độ phủ (làm ở bước 31)

- `Permission` thêm mô tả máy đọc: `risk` (`read` / `write` / `destructive` / `sensitive_data` /
  `money`) — hàm tạo mới, mặc định suy từ tên (`.read` → `read`, còn lại `write`), không bắt module cũ sửa.
- Mỗi operation OpenAPI có `x-permissions: ["notes.write"]` (một trong số đó là đủ) hoặc `x-public: true`;
  handler khai bằng một thuộc tính duy nhất cạnh `#[utoipa::path]` (cách cụ thể: agent bước 31 chọn — vd
  macro của kernel bọc `utoipa::path`), kernel gom vào tài liệu. Câu "Requires `x`" trong mô tả sinh ra từ
  đó, không viết tay.
- **Test gate độ phủ** (`crates/app/tests/access_coverage.rs`, chạy trên app có mọi module): đọc
  `docs/openapi.json`; với mọi operation không `x-public`: gọi không token → 401; gọi bằng token không có
  permission nào → 403 `permission_denied` (hay 404 khi endpoint giấu tồn tại — phải khai `x-hides-existence`);
  operation nào thiếu cả `x-permissions` lẫn `x-public` → đỏ. Tool MCP: mọi tool có `requires`, tool
  `read_only` chỉ yêu cầu permission `risk = read`.
- Lợi ích: AI agent và giao diện biết trước quyền của từng endpoint; endpoint mới quên kiểm quyền làm gate đỏ.

### P2 — Lọc danh sách khớp kiểm từng bản ghi

Bước 34 đã định `scope_condition` (điều kiện SQL) cho danh sách và `require_on` cho một bản ghi. Bước này
thêm **test so khớp** cho mọi loại dữ liệu có `org_id`: sinh ngẫu nhiên cây tổ chức, membership, role,
chia sẻ (P3), deny (P4); với mỗi người, tập bản ghi danh sách trả về **bằng đúng** tập bản ghi mà kiểm từng
bản ghi cho qua — trên cả hai database. Một hàm `Access::check(principal, action, target)` duy nhất là
nguồn sự thật; điều kiện SQL được sinh từ cùng mô tả luật.

### P3 — Chia sẻ bản ghi (quan hệ một bước, kiểu Google Drive)

- Bảng kernel `kernel_shares` (cấp khách hàng): `tenant_id`, `object` (`<kind>:<id>`, loại có trong sổ đối
  tượng D-123), `subject_kind` (`user` / `team` / `org` / `role`), `subject_id`, `relation`, `expires_at`
  NULL, `granted_by`, `created_at`; khoá (`tenant_id`, `object`, `subject_kind`, `subject_id`, `relation`);
  index (`tenant_id`, `subject_kind`, `subject_id`, `object`).
- Module khai quan hệ của loại đối tượng: `Module::share_relations` → `ShareRelation { kind, relation:
  "viewer"|"editor"|…, permissions: &[Permission] }` (mặc định rỗng, D-038). Chia sẻ chỉ trao được quyền mà
  người chia sẻ đang có trên bản ghi đó (không leo thang).
- Điều kiện SQL: `scope_condition OR id IN (SELECT … FROM kernel_shares WHERE …)` — một subquery có index,
  chạy như nhau trên hai database. Không quan hệ lồng nhau (nhóm trong nhóm): nhóm là danh sách phẳng.
- Ví dụ: chia bản nháp cho người duyệt, giao một phiếu hỗ trợ cho nhân viên nhánh khác, cho kế toán ngoài
  xem một bộ chứng từ trong 7 ngày.

### P4 — Deny, rào, API key chi tiết

- **Deny thắng allow:** role có thêm `denies: [permission]` (vd role "Thực tập" = mọi quyền của nhân viên
  trừ `orders.refund`). Thứ tự quyết định cố định: rào khách → deny → allow → mặc định từ chối. Không có
  deny theo điều kiện tự do.
- **Rào khách con (như SCP của AWS):** khách cha / nền tảng đặt `max_permissions` cho khách con (bước 38 có
  cây khách): quyền nào ngoài rào thì không role nào của khách con cấp được, kể cả `admin` của khách con.
- **API key chi tiết (như GitHub):** hạn **bắt buộc** (tối đa là cấu hình, mặc định 365 ngày), chọn tài
  nguyên (nút tổ chức — đã có ở 34 — hoặc danh sách bản ghi cụ thể qua `kernel_shares` với `subject_kind =
  api_key`), danh sách IP / dải IP được phép (tuỳ chọn), lần dùng cuối, xoay vòng không gián đoạn (key cũ
  sống thêm N giờ). Khách bật "key cần quản trị duyệt" thì key mới ở trạng thái chờ tới khi được duyệt.

### P5 — Xác thực lại cho việc nhạy cảm (step-up)

- Permission có `risk` `destructive` / `money` / `sensitive_data`, hay do khách đánh dấu, đòi phiên đã xác
  thực mạnh (passkey hay MFA) trong `AUTOXIDE_ACCESS_STEP_UP_MINUTES` (mặc định 15). Thiếu → 401
  `step_up_required` kèm header `WWW-Authenticate` theo RFC 9470 (`acr_values`, `max_age`); giao diện hỏi
  passkey / mã MFA rồi gọi lại.
- API key không có step-up: việc nhạy cảm qua key của agent đi đường duyệt của P12 bước 51.
- Điều kiện theo ngữ cảnh trên gán role (tuỳ chọn): dải IP, khung giờ làm việc theo múi giờ của nút.

### P6 — Nâng quyền đúng lúc (như Entra PIM)

- Gán role có thể là `eligible` (đủ điều kiện, chưa có hiệu lực). Người dùng kích hoạt: lý do bắt buộc,
  step-up, thời lượng ≤ tối đa của role; role đánh dấu "cần duyệt" thì tạo yêu cầu duyệt (P12 bước 51) và
  chỉ có hiệu lực khi được duyệt. Hết hạn tự thu hồi (job), có event và audit.
- Tài khoản khẩn cấp (break-glass): role `admin` cho tài khoản đánh dấu khẩn cấp, mọi lần đăng nhập báo cho
  mọi admin.

### P7 — Hai người, tách nhiệm vụ

- Module khai lệnh cần hai người: `Module::dual_control` → `DualControl { command: "orders.refund",
  threshold: Option<MoneyRule>, … }`. Người tạo lệnh không tự duyệt; dưới ngưỡng thì một người. Ngưỡng là dữ
  liệu theo khách, có ngày hiệu lực.
- Cặp quyền xung đột (tạo nhà cung cấp + duyệt thanh toán cho nhà cung cấp…) khai bởi module; lưu role vi
  phạm thì cảnh báo (không chặn), báo cáo trong rà soát quyền (P9).

### P8 — Quyền theo trường dữ liệu, dữ liệu cá nhân

- Trường của schema trả về được phân loại: `#[schema(extensions(("x-data-class", "personal")))]` —
  `personal` (email, số điện thoại, địa chỉ), `sensitive` (giấy tờ, sức khoẻ), `financial` (giá vốn, công nợ),
  `secret` (bí mật MFA, API key, token thanh toán, băm mật khẩu — không bao giờ trả trong phiên xem như, P13,
  D-232).
- Không có permission `<module>.read_personal` (hay `read_financial`) thì service trả trường đó đã che
  (`a•••@example.com`, `090•••567`) — che ở service, không phải ở handler, nên REST và MCP như nhau.
- Đọc dữ liệu `sensitive` ghi audit (ai, lúc nào, bản ghi nào) — căn cứ Luật 91/2025/QH15, NĐ 356/2025
  (cần đối chiếu văn bản gốc).
- Áp trước cho các chỗ đang lộ: email người bình luận (`comments`), dữ liệu biểu mẫu (`forms`), danh sách
  người đăng ký bản tin (`newsletter`), người dùng (`auth`).

### P9 — Giải thích, mô phỏng, rà soát

- `/me/can` (P11 bước 51) trả thêm **lý do có quyền**: role nào, ở nút nào, qua chia sẻ nào, hay bị deny /
  rào nào chặn.
- `POST /api/v1/kernel/access/simulate`: "người X (hay role nháp R chưa lưu) có làm được Y trên Z không, vì
  sao" — dùng trước khi lưu role (như IAM Policy Simulator). Quyền `kernel.access.simulate`.
- `GET /api/v1/kernel/access/who-can?permission=&object=`: ai có quyền trên một bản ghi (qua role theo cây,
  chia sẻ, key) — phân trang.
- **Quyền không dùng:** đếm lần dùng mỗi permission theo người / key (bộ đếm gộp theo ngày, không ghi từng
  request); báo cáo "quyền chưa dùng 90 ngày" để thu hẹp role (như IAM Access Analyzer).
- **Rà soát định kỳ** (như Entra Access Reviews): khách đặt lịch; người duyệt xác nhận hay thu hồi từng
  gán role / key / chia sẻ; hết hạn rà soát mà không ai xác nhận thì tuỳ cấu hình giữ hay thu hồi; vào hộp
  việc (bước 51).

### P10 — Chuẩn mở cho máy và agent

- **AuthZEN Authorization API 1.0:** `POST /access/v1/evaluation` và `/access/v1/evaluations` (nhiều câu
  một lần) theo đúng chuẩn (subject, action, resource, context → `decision`), gọi cùng `Access::check`.
  Dùng cho cổng API, dịch vụ khác của khách, agent. Quyền `kernel.access.evaluate`; chỉ hỏi được về khách
  của mình.
- **Hành động thay người dùng:** key loại `agent` (P12 bước 51) có thể gắn "thay mặt người dùng U"; quyền =
  giao của key và U; audit ghi chuỗi `actor = agent, on_behalf_of = U` (theo ý nghĩa `act` của RFC 8693).
- Ứng dụng bên thứ ba qua OAuth 2.0 với phạm vi là tên permission — bước 65 (`apps`, D-221).

### P11 — Thu hồi tức thì, nhanh, đo được

- Quyền **không** nằm trong access token (token chỉ mang danh tính): đổi role, rút key, hết hạn chia sẻ có
  hiệu lực ở request kế tiếp — khác các nền tảng nhét phạm vi vào JWT phải chờ token hết hạn.
- Nhanh: quyền của người gọi nạp một lần mỗi request (một câu truy vấn), cache trong bộ nhớ theo khoá
  (khách, người, `access_version` của khách); mọi thay đổi quyền tăng `kernel_access_version` cùng
  transaction → cache cũ tự hết. Đo p99 thời gian quyết định; mục tiêu < 1 ms khi trúng cache.
- Số đo `access_denied_total{module, reason}` (nhãn tập nhỏ cố định); cảnh báo khi một key bị từ chối dồn dập.

### P12 — Khách duyệt từng lần khi nhân sự nền tảng xin vào (D-232)

Nền là quyền vào hỗ trợ của bước 34 (`auth_support_grant`, `POST /support-grants`, khách tự thu hồi — D-190,
D-211). Bước này thêm chính sách theo khách và đường "xin → khách duyệt → cấp". Không có module mới.

- **Chế độ** `auth.support_access_mode` — chính sách của cả khách, không theo nút, lưu ở `kernel_tenant_settings`
  (bước 33); thay khoá cài đặt theo cây `auth.support_access` (bool) của bước 34:

  | Chế độ | Nhân sự nền tảng vào bằng `basis = support` thế nào |
  |---|---|
  | `disabled` | Không vào được (`auth.support_access_disabled`, như khoá cũ = `false`) |
  | `per_request` (**mặc định cho khách mới**) | Chỉ qua yêu cầu vào được quản trị của khách duyệt; cấp thẳng → 409 `auth.support_request_required` |
  | `standing` | Như cách cũ của D-190 / D-211: `auth.support.grant` cấp thẳng có thời hạn; yêu cầu vào vẫn dùng được |

  `legal_request` đi được ở **mọi** chế độ (dưới). Khách mới: `Module::seed_tenant` của `auth` ghi
  `per_request`; thiếu dòng cũng đọc là `per_request`. Khách đang có (`Module::upgrades`): đọc
  `auth.support_access` ở nút gốc qua API cài đặt theo cây của bước 34 — `true` → `standing`, `false` →
  `disabled` — để nâng cấp không đổi ngầm hành vi; khoá cũ thôi khai, CHANGELOG ghi.
- **Đổi chế độ:** chỉ chủ khách — người có `auth.support_access.manage` ở độ sâu `tenant` (mặc định chỉ role `admin`
  dựng sẵn); permission có `risk = sensitive_data` nên đòi xác nhận lại danh tính (P5, token `/me/reauth` của bước
  34); event `auth.support_access_mode_changed {from, to}` (audit). Không đổi qua `PUT /units/{id}/settings` của
  `organizations` (đường đó không đòi chủ khách, không step-up). Chuyển sang `disabled`: thu hồi mọi lần cấp
  `support` còn hạn và huỷ yêu cầu đang chờ của khách đó (job `auth.revoke_support_access`, vì lần cấp nằm ở
  database mặt điều khiển).
- **Yêu cầu vào** (`per_request`, dùng được cả ở `standing`): nhân sự nền tảng có `auth.support.request` ở khách
  vận hành gửi `POST /api/v1/auth/support-requests {tenant_id, role_id, scope, hours, reason, ticket_reference?}`:
  `reason` 10–500 ký tự (bắt buộc), `scope` `read` / `write`, `hours` 1–4 (trần cố định 4 giờ theo D-232, không
  phải cấu hình; vượt → `auth.support_duration_exceeded` với `{max} = 4`), `ticket_reference` (mã phiếu hỗ trợ,
  ≤ 64 ký tự, tuỳ chọn). `role_id` là role của khách vận hành, như lần cấp của bước 34.
  1. Transaction ở mặt điều khiển: dòng `auth_support_requests` (`pending`, `expires_at` = +24 giờ) + job
     `auth.open_support_approval` (unique_key `support_request:<id>`). Trả 201.
  2. Job mở ngữ cảnh khách được xin vào, trong một transaction ở database của khách: yêu cầu duyệt loại
     `support_access` (`kernel_approval_requests`, P12 bước 51, `expires_at` 24 giờ, tóm tắt gồm người xin, lý do,
     phạm vi, thời lượng, mã phiếu) + event `auth.support_access_requested`; rồi ghi `approval_id` vào dòng ở mặt
     điều khiển. Chạy lại thì thấy yêu cầu duyệt đã có (mã tham chiếu = ID yêu cầu vào), chỉ ghi lại `approval_id`.
  3. Quản trị của khách nhận thông báo (`notifications` nghe `auth.support_access_requested`, gửi cho người có
     `auth.support_access.approve` ở gốc — tìm bằng `who-can` của P9, loại tin `Security`, không tắt được) và thấy
     mục trong hộp việc (yêu cầu duyệt đang chờ của P12 bước 51).
  4. Duyệt / từ chối bằng `POST /api/v1/kernel/approvals/{id}/approve|reject` của bước 51. Người duyệt cần
     `auth.support_access.approve` ở độ sâu `tenant` (loại `support_access` khai permission duyệt riêng, vì người
     xin là nhân sự ở khách vận hành, không có permission ở khách này); permission này `risk = sensitive_data` nên
     đòi step-up — vì thế không duyệt được bằng API key hay tool MCP `decide_approval` (P5).
  5. `auth` nghe `kernel.approval_decided`: duyệt → trong một transaction ở mặt điều khiển tạo `auth_support_grant`
     (`basis = support`, `scope` và giờ đúng như đã duyệt, `starts_at` = lúc duyệt, `request_id` unique — nhận lại
     event không tạo lần cấp thứ hai) và đổi yêu cầu sang `approved`; event `auth.support_access_granted` ghi vào
     outbox của khách qua job `auth.mirror_event` sẵn có. Từ chối → `rejected` kèm lý do. Yêu cầu đã `cancelled` /
     `expired` thì bỏ qua, không cấp.
- **Hết hạn:** lần cấp hết `expires_at` thì mất quyền ở request kế (như bước 34). Job `auth.support_access_sweep`
  (5 phút một lần, unique_key có khe thời gian) đổi yêu cầu chưa duyệt quá 24 giờ sang `expired` và ghi event
  `auth.support_access_expired` cho lần cấp vừa hết hạn (vào outbox của khách qua `auth.mirror_event`), để nhật ký
  của khách có mốc kết thúc. Khách thu hồi sớm
  bằng `DELETE /support-access/{id}` của bước 34 (D-211); người xin huỷ yêu cầu chưa duyệt bằng
  `POST /support-requests/{id}/cancel`.
- **Phạm vi:** `read` → permission hiệu lực của lần cấp chỉ còn permission `risk = read` của role (P1), lệnh ghi
  trả 403 `auth.support_access_read_only`; `write` → đủ permission của role. Lần cấp cũ (trước bước này) là `write`.
- **`legal_request`** (yêu cầu của cơ quan có thẩm quyền) vẫn qua `POST /support-grants` của bước 34, không cần
  khách duyệt, ở mọi chế độ kể cả `disabled` (D-211): bắt buộc `legal_reference` (số, ngày văn bản), audit, khách
  thấy trong `GET /support-access` và nhật ký phiên. Văn bản cấm thông báo cho khách thì người cấp đặt
  `withhold_notice = true`: lần cấp, nhật ký phiên và event không hiện cho khách (kể cả ở bàn kỹ thuật của bước 52)
  cho tới khi người có `auth.support.grant` gỡ cờ — **cần đối chiếu văn bản gốc / người có chuyên môn pháp lý**
  về trường hợp được giữ kín và thời hạn.
- **Mọi request trong phiên vào mang cả hai danh tính:** nhân sự nền tảng (`actor_id`, `actor_kind = support`) và
  lần cấp (`support_grant_id`, nối tới người duyệt của khách). Lớp ghi nhật ký request của bước 52 ghi **mọi**
  request (cả đọc) của phiên vào `kernel_request_log` với cột `support_grant_id`; các dòng này giữ theo hạn lưu
  audit, không theo hạn 14 ngày của nhật ký request. Khách xem bằng `GET /support-access/{id}/requests`.

| Method, path (`/api/v1/auth`) | Permission | Ghi chú |
|---|---|---|
| `GET /support-access/mode` | `auth.support_access.read` | `{mode, updated_by, updated_at}` |
| `PUT /support-access/mode` `{mode}` | `auth.support_access.manage` (độ sâu `tenant`, step-up) | audit; `disabled` thu hồi lần cấp `support` còn hạn |
| `GET /support-access` (bước 34) | `auth.support_access.read` | thêm yêu cầu vào và trạng thái, `scope`, `ticket_reference`; ẩn lần cấp `withhold_notice` |
| `GET /support-access/{id}/requests` | `auth.support_access.read` | nhật ký phiên: thời điểm, phương thức, đường dẫn mẫu, status, người bị xem như (P13); phân trang `fetch_page` |
| `POST /support-requests` | `auth.support.request` (ở khách vận hành) | 201 + `Location`; `disabled` → 403 `auth.support_access_disabled` |
| `GET /support-requests?tenant_id=&status=` · `GET /support-requests/{id}` | `auth.support.request` (ở khách vận hành) | người xin thấy trạng thái, lý do từ chối |
| `POST /support-requests/{id}/cancel` | người xin | chỉ khi `pending`, không thì 409 `auth.support_request_closed` |
| `POST /support-grants` (bước 34) | `auth.support.grant` | `basis = support`: chỉ ở `standing` (`per_request` → 409 `auth.support_request_required`); thêm `scope` (mặc định `write`), `withhold_notice` (chỉ với `legal_request`) |

Tool MCP: không có (như bước 34 — việc nhạy cảm chỉ qua REST có người). Event mới (ghi ở outbox của khách được
vào): `auth.support_access_mode_changed`, `auth.support_access_requested`, `auth.support_access_expired`; dùng lại
`auth.support_access_granted`, `support_access_started`, `support_access_revoked` của bước 34. Job mới:
`auth.open_support_approval`, `auth.support_access_sweep`, `auth.revoke_support_access` (đều chịu chạy lại).

### P13 — Xem như người dùng, chỉ đọc (D-232)

Thay cho "đăng nhập thay người dùng": **chỉ đọc, không bao giờ làm thay; không có chế độ act-as.** Dùng để gỡ lỗi
quyền và hiển thị: người có `auth.view_as` thấy giao diện / dữ liệu đúng như một người dùng khác thấy.

- **Ai được xem ai** (kiểm lúc mở phiên và **lại ở mỗi request**, hai `Principal` đều nằm trong cache của P11):

  | Người bị xem | Điều kiện |
  |---|---|
  | Nhân viên cùng khách (`realm = staff`, có membership ở khách) | Người xem có quyền bao trùm người bị xem (`require_covers` của bước 34 trên mọi membership của người bị xem); không bao giờ xem như người giữ role `admin` ở nút gốc (chủ khách). Người bị xem nhận thông báo (`notifications` nghe `auth.view_as_started`, loại `Security`) |
  | Người mua (`realm = customer`) | Chỉ khi người mua đã cấp phép từ tài khoản của họ ("Cho phép hỗ trợ xem tài khoản"): mã 6 chữ số, hiệu lực 30 phút, dùng một lần — dữ liệu cá nhân |
  | Bất kỳ ai, người xem là nhân sự nền tảng | Chỉ khi đang có lần cấp `basis = support` còn hạn ở khách đó (P12; ở `per_request` là lần đã được duyệt) và role của lần cấp có `auth.view_as`; rồi theo đúng hai dòng trên. Lần cấp `legal_request` không mở được phiên xem như |

  Không xem như chính mình (`auth.view_as_self`). `auth.view_as` có `risk = sensitive_data` nên mở phiên đòi step-up
  (P5); khi lọc lần cấp hỗ trợ phạm vi `read` (P12), `auth.view_as` vẫn giữ vì phiên luôn chỉ đọc.
- **Cấp phép của người mua:** `POST /api/v1/auth/me/view-as-consent` (đăng nhập, `realm = customer`) trả `{code,
  expires_at}`; mã lưu băm trong kho token dùng một lần `OneTimeTokens` của `auth` (D-117, mục đích
  `view_as_consent`, băm của `user_id` + mã); tạo mã mới thu hồi mã cũ. Mở phiên tiêu thụ mã (`consume` — đúng một
  bên thắng khi đua). Sai mã đếm bằng `Throttle` (D-116) theo người mua: sai 5 lần thì thu hồi mã.
  `DELETE /me/view-as-consent` rút phép: thu hồi mã chưa dùng và kết thúc phiên đang mở trên tài khoản đó.
- **Phiên `view_as`** là token riêng, không có refresh token: JWT ký như access token, `sub` = người bị xem, `act =
  {sub: người xem}` (nghĩa `act` của RFC 8693), `vas` = ID phiên, `exp` tối đa 30 phút kể từ lúc mở; không gia hạn
  ngầm — hết hạn thì mở phiên mới (người mua phải cấp mã mới). Mỗi request đọc dòng `auth_view_as_sessions` (một
  câu theo khoá chính) để kết thúc có hiệu lực ngay; phiên đã kết thúc / hết hạn → 401 `auth.view_as_session_ended`.
  Người xem kết thúc bất cứ lúc nào bằng `DELETE /api/v1/auth/view-as/current`.
- **Chỉ đọc ở kernel, không ở từng handler:** với token `view_as`, mọi request `POST` / `PUT` / `PATCH` / `DELETE`
  và mọi tool MCP không `read_only` trả 403 `auth.view_as_read_only` trước khi tới handler — ngoại lệ duy nhất là
  `DELETE /view-as/current`. Ghi phụ của request đọc (đánh dấu đã xem, `last_used_at`, bộ đếm quyền dùng của P9)
  không chạy trong phiên xem như.
- **Không bao giờ trả secret:** trong phiên, service bỏ mọi trường `x-data-class: secret` (P8) — bí mật MFA, API key,
  token thanh toán, mật khẩu — kể cả khi người bị xem tự xem được.
- **Audit:** mỗi request ghi `kernel_request_log` (bước 52) với người dùng = người xem (người thật), thêm
  `view_as_session_id` và `view_as_target_id`; lọc được theo hai cột này; giữ theo hạn lưu audit. Event
  `auth.view_as_started {session_id, actor_id, target_id, target_realm, support_grant_id?, expires_at}`,
  `auth.view_as_ended {session_id, ended_by: actor|target|revoked}`, `auth.view_as_consent_issued {user_id,
  expires_at}`. Trong lần vào hỗ trợ, dòng nhật ký có cả `support_grant_id` — hiện trong nhật ký phiên của P12.
- **Báo cho giao diện:** mọi response trong phiên có header `Autoxide-View-As: session="<id>", target="<user id>",
  expires="<RFC 3339>"` (Dictionary của Structured Field Values, RFC 9651) để giao diện hiện băng cảnh báo.

| Method, path (`/api/v1/auth`) | Permission | Ghi chú |
|---|---|---|
| `POST /view-as` `{target_user_id, consent_code?}` | `auth.view_as` (step-up) | 201 `{token, session_id, expires_at, target: {id, realm, name}}`; người mua thiếu mã → 403 `auth.view_as_consent_required` |
| `DELETE /view-as/current` | token `view_as` | kết thúc ngay; event `auth.view_as_ended` |
| `POST /me/view-as-consent` · `DELETE /me/view-as-consent` | đăng nhập, `realm = customer` | tài khoản nhân sự gọi → 403 `auth.view_as_consent_customer_only` |

Tool MCP: không có tool mở phiên (cần step-up, như P12). Tool `read_only` gọi được bằng token `view_as`; tool khác
bị chặn như trên. Job: `auth.cleanup` sẵn có xoá dòng `auth_view_as_sessions` quá hạn lưu audit (event giữ).

## Bảng dữ liệu (mọi bảng có `tenant_id` trừ khi ghi khác)

| Bảng | Cột chính |
|---|---|
| `kernel_shares` | xem P3 |
| `kernel_access_version` | `tenant_id`, `version` BIGINT — tăng cùng transaction với mọi thay đổi quyền |
| `auth_role_denies` | `tenant_id`, `role_id`, `permission` |
| `auth_role_assignments` (thêm cột) | `eligible` BOOL, `max_activation_minutes`, `requires_approval`, `conditions` JSON (IP, khung giờ) |
| `auth_role_activations` | `id`, `tenant_id`, `assignment_id`, `reason`, `approval_id` NULL, `starts_at`, `ends_at`, `revoked_at` |
| `auth_api_keys` (thêm cột) | `expires_at` NOT NULL (key cũ: đặt hạn 365 ngày từ lúc nâng cấp, báo trước), `allowed_ips` text, `status` (`pending/active/revoked`), `last_used_at`, `rotated_from` |
| `kernel_tenant_guardrails` (mặt điều khiển) | `tenant_id`, `max_permissions` text, `set_by_tenant_id`, `updated_at` |
| `kernel_permission_usage` | `tenant_id`, `day`, `subject_kind`, `subject_id`, `permission`, `count` |
| `auth_access_reviews` · `auth_access_review_items` | lịch, người duyệt, kết quả từng mục |
| `kernel_tenant_settings` (dòng mới, bước 33) | `auth.support_access_mode` = `disabled` / `per_request` / `standing` (P12, D-232) |
| `auth_support_requests` (mặt điều khiển, P12) | `id`, `tenant_id` (khách được xin vào), `staff_user_id`, `role_id`, `scope` (`read`/`write`), `hours` (1–4), `reason`, `ticket_reference` NULL, `status` (`pending`/`approved`/`rejected`/`expired`/`cancelled`), `approval_id` NULL, `decided_by` NULL, `decided_at` NULL, `decision_reason` NULL, `expires_at`, `created_at`; index (`tenant_id`, `status`, `expires_at`), (`tenant_id`, `staff_user_id`, `id`) |
| `auth_support_grant` (thêm cột, bước 34) | `scope` (`read`/`write`, mặc định `write`), `request_id` NULL unique, `withhold_notice` BOOL mặc định `false` (P12) |
| `auth_view_as_sessions` (P13) | `id`, `tenant_id`, `actor_user_id`, `actor_kind` (`staff`/`support`), `support_grant_id` NULL, `target_user_id`, `target_realm`, `started_at`, `expires_at`, `ended_at` NULL, `ended_by` NULL (`actor`/`target`/`revoked`); index (`tenant_id`, `target_user_id`, `id`), (`tenant_id`, `actor_user_id`, `id`) |
| `kernel_request_log` (thêm cột, bước 52) | `support_grant_id` NULL, `view_as_session_id` NULL, `view_as_target_id` NULL; index (`tenant_id`, `support_grant_id`, `id`), (`tenant_id`, `view_as_session_id`, `id`); dòng có một trong ba cột giữ theo hạn lưu audit (P12, P13) |

## Permission mới

`kernel.access.simulate`, `kernel.access.evaluate`, `kernel.access.review`, `auth.roles.activate` (tự kích
hoạt role đủ điều kiện), `auth.api_keys.approve`; `<module>.read_personal` cho module giữ dữ liệu cá nhân.
P12, P13 (D-232): `auth.support_access.manage` (đổi chế độ; `sensitive_data`, chỉ độ sâu `tenant`),
`auth.support_access.approve` (duyệt yêu cầu vào; `sensitive_data`), `auth.support.request` (ở khách vận hành),
`auth.view_as` (`sensitive_data`).

## Mã lỗi (kèm bản dịch `vi`)

`step_up_required` (401), `permission_denied` (403, đã có — thêm `reason`: `missing` / `denied_by_role` /
`outside_guardrail` / `outside_key_scope` / `ip_not_allowed` / `outside_hours`), `auth.activation_required`
(403, có role đủ điều kiện nhưng chưa kích hoạt), `auth.dual_control_self_approval` (409),
`auth.api_key_expiry_required` (422), `auth.api_key_pending` (401), `kernel.share_escalation` (403 — chia
quyền mình không có), `kernel.share_kind_unknown` (422).

P12, P13 (D-232; dùng lại `auth.support_access_disabled`, `auth.support_duration_exceeded` của bước 34):

| Mã | HTTP | Câu tiếng Việt |
|---|---|---|
| `auth.support_request_required` | 409 | Khách hàng này duyệt từng lần vào hỗ trợ; tạo yêu cầu bằng `POST /api/v1/auth/support-requests`. |
| `auth.support_request_not_found` | 404 | Không có yêu cầu vào hỗ trợ `{id}`. |
| `auth.support_request_closed` | 409 | Yêu cầu vào hỗ trợ `{id}` đã ở trạng thái `{status}`; tạo yêu cầu mới nếu vẫn cần. |
| `auth.support_access_read_only` | 403 | Lần vào hỗ trợ này chỉ được đọc; gửi yêu cầu phạm vi `write` nếu cần sửa. |
| `auth.view_as_read_only` | 403 | Đang xem như người dùng khác nên chỉ được đọc; kết thúc phiên xem để thao tác. |
| `auth.view_as_target_not_covered` | 403 | Không xem như được `{user}`: người đó có quyền bạn không có hoặc là chủ khách hàng. |
| `auth.view_as_consent_required` | 403 | Người mua `{user}` chưa cho phép hỗ trợ xem tài khoản; nhờ họ tạo mã trong tài khoản của mình. |
| `auth.view_as_consent_invalid` | 403 | Mã cho phép xem không đúng, đã dùng hoặc đã hết hạn. |
| `auth.view_as_consent_customer_only` | 403 | Chỉ tài khoản người mua mới tạo mã cho phép xem. |
| `auth.view_as_support_grant_required` | 403 | Nhân sự nền tảng chỉ xem như người dùng trong một lần vào hỗ trợ còn hạn. |
| `auth.view_as_self` | 422 | Không xem như chính mình. |
| `auth.view_as_session_ended` | 401 | Phiên xem như người dùng đã kết thúc hoặc hết hạn; dùng lại token của chính bạn. |

## Thứ tự làm

1. (Bước 31) P1: `risk`, `x-permissions` / `x-public`, test độ phủ trong gate.
2. P11 (version, cache) và `Access::check` duy nhất; P2 test so khớp.
3. P4 (deny, rào, key chi tiết) → P5 (step-up) → P3 (chia sẻ).
4. P8 (trường dữ liệu) cho `auth`, `comments`, `forms`, `newsletter`.
5. P6, P7 (cần P12 bước 51) → P9 → P10.
6. Tài liệu `docs/ACCESS.md` (EN + VI): mô hình, cách khai quyền cho module, ví dụ cho từng mô hình kinh doanh.
7. (D-232, cần P5, P12 bước 51, nhật ký request bước 52) P12: chế độ vào hỗ trợ + nâng cấp khách cũ → yêu cầu vào,
   duyệt, phạm vi, hết hạn → `legal_request` giữ kín. Rồi P13: trường `secret` (P8) → token và phiên `view_as`,
   chặn ghi ở kernel và MCP → cấp phép của người mua → audit, header. Thêm cả hai vào `docs/ACCESS.md`.

## Test bắt buộc (cả hai database)

`moi_endpoint_khong_cong_khai_tra_401_va_403` (P1), `loc_danh_sach_khop_kiem_tung_ban_ghi` (P2, sinh ngẫu
nhiên), `deny_thang_allow`, `rao_khach_con_chan_ca_admin`, `chia_se_khong_trao_quyen_minh_khong_co`,
`chia_se_het_han_mat_quyen_ngay`, `doi_role_co_hieu_luc_o_request_ke_tiep`,
`key_het_han_hay_ngoai_ip_bi_tu_choi`, `viec_nhay_cam_doi_xac_thuc_lai`, `kich_hoat_role_het_han_tu_thu_hoi`,
`nguoi_tao_lenh_khong_tu_duyet`, `truong_ca_nhan_bi_che_khi_thieu_quyen_ca_rest_va_mcp`,
`mo_phong_role_nhap_khong_luu_gi`, `authzen_evaluation_dung_chuan`, `quyet_dinh_dong_thoi_khi_dang_doi_role`.

P12 (D-232): `khach_moi_mac_dinh_duyet_tung_lan`, `nang_cap_giu_che_do_cu_cua_khach_dang_co`,
`doi_che_do_can_chu_khach_va_xac_nhan_lai`, `duyet_tung_lan_chan_cap_thang`,
`yeu_cau_duoc_duyet_cap_dung_pham_vi_va_thoi_luong`, `pham_vi_read_chan_lenh_ghi`, `yeu_cau_qua_4_gio_bi_tu_choi`,
`yeu_cau_chua_duyet_het_han_sau_24_gio`, `tu_choi_thi_khong_co_quyen_vao`, `duyet_khong_lam_duoc_bang_api_key`,
`tat_ho_tro_thu_hoi_lan_cap_con_han`, `yeu_cau_co_tham_quyen_khong_can_duyet_va_hien_trong_nhat_ky`,
`yeu_cau_co_tham_quyen_giu_kin_khong_hien_cho_khach`, `nhat_ky_phien_vao_ghi_moi_request_ca_hai_danh_tinh`;
đồng thời: `duyet_dong_thoi_va_nhan_lai_event_chi_mot_lan_cap` (hai người duyệt cùng lúc + event giao lại: đúng một
`auth_support_grant`), `duyet_va_huy_dong_thoi` (huỷ thắng thì không có lần cấp; duyệt thắng thì huỷ trả 409).

P13 (D-232): `view_as_chan_lenh_ghi_ca_rest_va_mcp` (đọc `docs/openapi.json` và danh sách tool: mọi operation ghi,
mọi tool không `read_only` trả 403 `auth.view_as_read_only`), `view_as_khong_xem_duoc_nguoi_quyen_cao_hon`,
`view_as_khong_xem_duoc_chu_khach`, `view_as_kiem_lai_khi_nguoi_bi_xem_duoc_nang_quyen`,
`view_as_nguoi_mua_chua_cap_phep_bi_tu_choi`, `ma_cap_phep_het_han_hay_da_dung_bi_tu_choi`,
`ma_cap_phep_sai_5_lan_bi_thu_hoi`, `view_as_khong_tra_secret`, `view_as_het_30_phut_khong_gia_han`,
`ket_thuc_view_as_co_hieu_luc_ngay`, `nhan_su_nen_tang_view_as_chi_trong_lan_vao_con_han`,
`view_as_audit_du_hai_id_va_co_header`, `nguoi_bi_xem_nhan_thong_bao`; đồng thời:
`ma_cap_phep_dung_dong_thoi_chi_mot_phien` (16 request cùng mã: đúng một phiên).

## Tiêu chí xong

Mọi mục bước 54 `[x]`; gate xanh trên cả hai database; test độ phủ P1 và test so khớp P2 chạy trong gate;
`docs/ACCESS.md` hai ngôn ngữ; benchmark quyết định quyền ghi vào `docs/BENCHMARKS.md`.

## Chỗ dễ sai

- Viết luật hai nơi (một cho danh sách, một cho từng bản ghi) rồi lệch nhau — chỉ một mô tả luật (P2).
- Nhét quyền vào JWT để "nhanh" — mất thu hồi tức thì (P11).
- Chia sẻ thành đồ thị lồng nhau — mất câu SQL một bước, chậm và khó giải thích.
- Ngôn ngữ chính sách tự do — người dùng không đọc được, khó kiểm, khó chạy trên hai database.
- Che dữ liệu ở handler — MCP lộ (luật 8).
- Chặn ghi của `view_as` ở từng handler — sót một endpoint là làm thay được; chặn ở lớp xác thực của kernel cho
  REST và MCP, test đọc từ `docs/openapi.json` (P13). Nhớ chừa `DELETE /view-as/current`, không thì không thoát
  được phiên.
- Cho token `view_as` refresh token hay gia hạn khi còn dùng — phiên kéo dài vô hạn (P13).
- Chỉ kiểm "quyền bao trùm" lúc mở phiên — người bị xem được nâng quyền giữa phiên thì người xem thấy quá phần mình
  (P13: kiểm lại mỗi request).
- Ghi lần cấp ở mặt điều khiển và yêu cầu duyệt ở database của khách trong hai transaction lồng nhau — có thể là
  hai database; đi qua job / event chịu chạy lại, khoá trùng bằng `request_id` (P12).
- Đổi chế độ qua API cài đặt theo cây của `organizations` — bỏ qua kiểm chủ khách và step-up (P12).
- Giữ kín `legal_request` ở `GET /support-access` mà quên bàn kỹ thuật (`/kernel/devtools/requests`) và audit của
  khách — lộ qua đường khác (P12).

## Agent làm bước này tự quyết

Cách khai `x-permissions` (macro hay hàm đăng ký); tối đa hạn API key; có tách bảng `kernel_teams` hay dùng
nhóm của `organizations`; danh sách `risk` mặc định của permission đang có. Ghi vào `docs/DECISIONS.md`.

## Căn cứ và nguồn

- OpenID AuthZEN Authorization API 1.0 (Final, 01/2026): https://openid.net/specs/authorization-api-1_0.html
- RFC 9470 OAuth 2.0 Step Up Authentication Challenge; RFC 8693 OAuth 2.0 Token Exchange.
- NIST RBAC (ANSI INCITS 359), NIST SP 800-162 (ABAC).
- Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15, NĐ 356/2025 (P8) — cần đối chiếu văn bản gốc.
- Khách duyệt từng lần, xem như người dùng (P12, P13, D-232): Luật 91/2025/QH15 (hiệu lực 01/01/2026), NĐ
  356/2025/NĐ-CP — xử lý dữ liệu cá nhân phải có căn cứ, mục đích, để lại vết; người mua tự cấp phép trước khi bị
  xem như. Trường hợp cơ quan có thẩm quyền yêu cầu và được giữ kín không báo cho khách (`withhold_notice`) — **cần
  đối chiếu văn bản gốc / người có chuyên môn pháp lý** (điều khoản cụ thể, thời hạn giữ kín).
