# Bước 54 — Phân quyền chuẩn hàng đầu (kernel, `auth`)

> Lộ trình: `docs/ROADMAP.md` bước 54 (và hai mục ở bước 31) · Quyết định: D-205 · Nền: D-046 (RBAC),
> đặc tả 33 (khách), 34 (quyền theo độ sâu), 35 (gói), 37 (dữ liệu cá nhân, KYC), 51 (P11 `/me/can`,
> P12 lệnh chờ duyệt)

## Mục tiêu

Phân quyền của Autoxide ngang các nền tảng lớn và hơn họ ở chỗ: **thu hồi có hiệu lực ngay**, **lọc danh
sách luôn khớp với kiểm từng bản ghi** (chứng minh bằng test), **hợp đồng quyền đọc được bằng máy** (gate
chứng minh không endpoint nào hở), **giải thích và mô phỏng được mọi quyết định**, và **sinh ra cho AI
agent** (quyền tối thiểu, hành động thay người dùng có vết, lệnh rủi ro chờ người duyệt) — chạy như nhau trên
SQLite và Postgres, mã nguồn mở.

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

## Bắt đầu khi

- P1 (hợp đồng quyền đọc được bằng máy + test độ phủ) **làm ngay ở bước 31**, không chờ bước này.
- Phần còn lại SAU bước 34 (quyền theo độ sâu). P7, phần duyệt của P6 dùng cơ chế duyệt của P12 bước 51;
  P8 làm cùng mục dữ liệu cá nhân của bước 37 nếu bước đó chưa xong.
- Song song được với 41–49; module thương mại khai phần của mình (P3, P7, P8) ngay trong bước của nó khi
  bước 54 đã có, không thì bước 54 khai thay cho module đang có.

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
  `personal` (email, số điện thoại, địa chỉ), `sensitive` (giấy tờ, sức khoẻ), `financial` (giá vốn, công nợ).
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
- Ứng dụng bên thứ ba qua OAuth 2.0 với phạm vi là tên permission — để sau (ghi ở "Sau core") tới khi có nhu
  cầu chợ ứng dụng.

### P11 — Thu hồi tức thì, nhanh, đo được

- Quyền **không** nằm trong access token (token chỉ mang danh tính): đổi role, rút key, hết hạn chia sẻ có
  hiệu lực ở request kế tiếp — khác các nền tảng nhét phạm vi vào JWT phải chờ token hết hạn.
- Nhanh: quyền của người gọi nạp một lần mỗi request (một câu truy vấn), cache trong bộ nhớ theo khoá
  (khách, người, `access_version` của khách); mọi thay đổi quyền tăng `kernel_access_version` cùng
  transaction → cache cũ tự hết. Đo p99 thời gian quyết định; mục tiêu < 1 ms khi trúng cache.
- Số đo `access_denied_total{module, reason}` (nhãn tập nhỏ cố định); cảnh báo khi một key bị từ chối dồn dập.

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

## Permission mới

`kernel.access.simulate`, `kernel.access.evaluate`, `kernel.access.review`, `auth.roles.activate` (tự kích
hoạt role đủ điều kiện), `auth.api_keys.approve`; `<module>.read_personal` cho module giữ dữ liệu cá nhân.

## Mã lỗi (kèm bản dịch `vi`)

`step_up_required` (401), `permission_denied` (403, đã có — thêm `reason`: `missing` / `denied_by_role` /
`outside_guardrail` / `outside_key_scope` / `ip_not_allowed` / `outside_hours`), `auth.activation_required`
(403, có role đủ điều kiện nhưng chưa kích hoạt), `auth.dual_control_self_approval` (409),
`auth.api_key_expiry_required` (422), `auth.api_key_pending` (401), `kernel.share_escalation` (403 — chia
quyền mình không có), `kernel.share_kind_unknown` (422).

## Thứ tự làm

1. (Bước 31) P1: `risk`, `x-permissions` / `x-public`, test độ phủ trong gate.
2. P11 (version, cache) và `Access::check` duy nhất; P2 test so khớp.
3. P4 (deny, rào, key chi tiết) → P5 (step-up) → P3 (chia sẻ).
4. P8 (trường dữ liệu) cho `auth`, `comments`, `forms`, `newsletter`.
5. P6, P7 (cần P12 bước 51) → P9 → P10.
6. Tài liệu `docs/ACCESS.md` (EN + VI): mô hình, cách khai quyền cho module, ví dụ cho từng mô hình kinh doanh.

## Test bắt buộc (cả hai database)

`moi_endpoint_khong_cong_khai_tra_401_va_403` (P1), `loc_danh_sach_khop_kiem_tung_ban_ghi` (P2, sinh ngẫu
nhiên), `deny_thang_allow`, `rao_khach_con_chan_ca_admin`, `chia_se_khong_trao_quyen_minh_khong_co`,
`chia_se_het_han_mat_quyen_ngay`, `doi_role_co_hieu_luc_o_request_ke_tiep`,
`key_het_han_hay_ngoai_ip_bi_tu_choi`, `viec_nhay_cam_doi_xac_thuc_lai`, `kich_hoat_role_het_han_tu_thu_hoi`,
`nguoi_tao_lenh_khong_tu_duyet`, `truong_ca_nhan_bi_che_khi_thieu_quyen_ca_rest_va_mcp`,
`mo_phong_role_nhap_khong_luu_gi`, `authzen_evaluation_dung_chuan`, `quyet_dinh_dong_thoi_khi_dang_doi_role`.

## Tiêu chí xong

Mọi mục bước 54 `[x]`; gate xanh trên cả hai database; test độ phủ P1 và test so khớp P2 chạy trong gate;
`docs/ACCESS.md` hai ngôn ngữ; benchmark quyết định quyền ghi vào `docs/BENCHMARKS.md`.

## Chỗ dễ sai

- Viết luật hai nơi (một cho danh sách, một cho từng bản ghi) rồi lệch nhau — chỉ một mô tả luật (P2).
- Nhét quyền vào JWT để "nhanh" — mất thu hồi tức thì (P11).
- Chia sẻ thành đồ thị lồng nhau — mất câu SQL một bước, chậm và khó giải thích.
- Ngôn ngữ chính sách tự do — người dùng không đọc được, khó kiểm, khó chạy trên hai database.
- Che dữ liệu ở handler — MCP lộ (luật 8).

## Agent làm bước này tự quyết

Cách khai `x-permissions` (macro hay hàm đăng ký); tối đa hạn API key; có tách bảng `kernel_teams` hay dùng
nhóm của `organizations`; danh sách `risk` mặc định của permission đang có. Ghi vào `docs/DECISIONS.md`.

## Căn cứ và nguồn

- OpenID AuthZEN Authorization API 1.0 (Final, 01/2026): https://openid.net/specs/authorization-api-1_0.html
- RFC 9470 OAuth 2.0 Step Up Authentication Challenge; RFC 8693 OAuth 2.0 Token Exchange.
- NIST RBAC (ANSI INCITS 359), NIST SP 800-162 (ABAC).
- Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15, NĐ 356/2025 (P8) — cần đối chiếu văn bản gốc.
