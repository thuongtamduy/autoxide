# Runbook vận hành

[English](RUNBOOK.md) · **Tiếng Việt**

Việc cần làm khi production gặp sự cố, theo thứ tự nên làm. Mọi lệnh và endpoint ở đây đều có thật ở phiên bản này (trợ
giúp của CLI, `docs/openapi.json` và tên metric đã đối chiếu với code). Các tình huống theo từng cảnh báo (lỗi server, request
chậm, connection, backup, administrator cuối cùng) nằm ở [runbook của `DEPLOY.vi.md`](DEPLOY.vi.md#sổ-tay-xử-lý-sự-cố); trang này
nói các sự cố cần quyết định và một chuỗi bước.

## Cách dùng trang này

1. **Ngăn thiệt hại trước, tìm nguyên nhân sau.** Chặn thiệt hại (suspend một tenant, thu hồi một key, đưa một server ra khỏi
   vòng phục vụ), rồi mới tìm nguyên nhân.
2. **Ghi lại bạn làm gì và lúc nào** (một nhật ký chữ thường trong kênh sự cố). Bạn cần nó cho buổi rà soát, và cho thông báo
   vi phạm dữ liệu nếu dữ liệu cá nhân bị lộ ([`DEPLOY.vi.md`](DEPLOY.vi.md#sự-cố-dữ-liệu-cá-nhân)).
3. **Không bao giờ dán secret, token hay dữ liệu cá nhân** vào ticket hay chat. Dẫn `request_id` thay vào đó; mọi response
   lỗi đều mang một cái.
4. Lệnh đầu tiên cho gần như mọi việc: `autoxide doctor` (thêm `--json` cho script và AI agent). Nó không đổi gì và nói cái gì
   sai và sửa thế nào.

Trong các lệnh dưới, `API` là địa chỉ của API, `TOKEN` là access token của người dùng có permission mà bước đó nêu, và đã cài `jq`.

## Trước khi có sự cố

- Prometheus lấy `/metrics` (`AUTOXIDE_HTTP_METRICS=true`, port riêng) và nạp
  [`deploy/prometheus/autoxide-alerts.yml`](../deploy/prometheus/autoxide-alerts.yml); dashboard Grafana
  [`deploy/grafana/autoxide-dashboard.json`](../deploy/grafana/autoxide-dashboard.json) đã được nhập.
- Backup theo lịch chạy (`autoxide doctor` cảnh báo khi không), được sao ra khỏi nơi lưu chúng, và đã diễn tập restore trên
  một máy dự phòng ([`DEPLOY.vi.md`](DEPLOY.vi.md#khôi-phục-từng-bước)).
- Có ít nhất hai người đăng nhập được với tư cách platform staff, và ít nhất một người chạy được `autoxide` trên server
  (`autoxide user create … --admin` tạo administrator mới nếu mọi người đều bị khoá).
- Các secret (`AUTOXIDE_SECRET_KEY`, `AUTOXIDE_AUTH_TOKEN_SECRET`) được lưu ngoài repo và ngoài các backup database, và bạn
  biết chúng ở đâu.

## Database đầy hoặc chậm

Đầy (ghi lỗi "no space left on device"): giải phóng hay thêm đĩa trước, rồi xem
[`DEPLOY.vi.md`, "Database đầy"](DEPLOY.vi.md#database-đầy). SQLite: xem cỡ file database và file `-wal` (một lần đọc dài
như backup làm `-wal` cứ lớn). PostgreSQL: các bảng lớn nhất bằng
`SELECT relname, pg_size_pretty(pg_total_relation_size(relid)) FROM pg_statio_user_tables ORDER BY pg_total_relation_size(relid) DESC LIMIT 10;`.
Job đã xong và event đã giao tự bị xoá sau 7 ngày.

Chậm (cảnh báo `AutoxideSlowP95`, request phải chờ): so `database_connections{state="in_use"}` với `database_connections_max`
(cảnh báo `AutoxideDbPoolExhausted`); trên PostgreSQL xem `pg_stat_activity` tìm query dài, trên SQLite nhớ rằng chỉ có một
connection ghi theo thiết kế. Query thiếu index thì thêm vào một migration mới của module sở hữu bảng (`EXPLAIN ANALYZE` trên
PostgreSQL, `EXPLAIN QUERY PLAN` trên SQLite). Chỉ tăng `AUTOXIDE_DATABASE_READ_CONNECTIONS` /
`AUTOXIDE_DATABASE_WRITE_CONNECTIONS` sau khi đã kết thúc thứ đang giữ connection, và giữ tổng của mọi process dưới
`max_connections` của PostgreSQL.

## Hàng đợi việc nền bị ứ

Dấu hiệu: cảnh báo `AutoxideJobsStuck`, `AutoxideEventsBacklog`, `AutoxideDeadLetterGrowing`; mail, webhook, export đến trễ.

1. Có worker đang chạy không? Ít nhất một process phải có `AUTOXIDE_PROCESS_ROLE` là `all` hoặc `worker`; kiểm
   `/health/ready` và log của nó.
2. Module nào đang trễ? `jobs_pending` và `jobs_oldest_pending_seconds` theo `module` và `database`, hoặc, với platform
   staff có `kernel.ops.read`: `curl -H "Authorization: Bearer $TOKEN" "$API/api/v1/kernel/ops/backlog"`.
3. Nhiều khách: database đang migrate trả `503 tenant_maintenance` và việc của nó chờ; để `autoxide migrate up --all` chạy xong.
4. Job đã bỏ cuộc: `autoxide jobs failed [--module <module>] [--limit <n>]` liệt kê chúng kèm lỗi cuối. Sửa nguyên nhân (mật
   khẩu SMTP sai, đối tác sập) **trước** `autoxide jobs retry <module> <id>`; job retry quá sớm chỉ lại thất bại.
5. Event đã bỏ cuộc: module cứ lỗi với một event thì được thử lại riêng, nên các module khác đã nhận event đó. Mỗi
   dòng của `kernel_event_deliveries` có `failed_at` là một module (`subscriber`) chưa bao giờ nhận một event (`module`,
   `event_id`), kèm `last_error`. Sửa nguyên nhân, rồi chạy
   `UPDATE kernel_event_deliveries SET failed_at = NULL, attempts = 0 WHERE id = '<id>';` để chỉ thử lại module đó.
   Dòng của `<module>_events` có `failed_at` là dòng không đọc được (payload hỏng, khách không còn).
6. Vẫn trễ dù worker khoẻ: tăng `AUTOXIDE_JOBS_CONCURRENCY` hay chạy thêm worker process.

## Lộ key

Xác định key nào, rồi xoay nó. Đừng chờ chắc chắn mới làm.

**Key ký access token (`AUTOXIDE_AUTH_TOKEN_SECRET`).** Kẻ có nó giả được access token.

1. Sinh giá trị mới: `openssl rand -base64 48`.
2. Đặt nó trên **mọi** server và khởi động lại. **Không** đưa giá trị đã lộ vào `AUTOXIDE_AUTH_TOKEN_SECRET_PREVIOUS` (biến đó
   giữ một key cũ còn được chấp nhận để đổi key định kỳ êm).
3. Mọi access token ngừng dùng được ngay; client lấy token mới bằng refresh token, vẫn còn hiệu lực. Nếu refresh token cũng có
   thể bị lấy, hãy đăng xuất cả người dùng: xem [thu hồi hàng loạt](#thu-hồi-hàng-loạt-api-key-và-phiên).

**Key mã hoá (`AUTOXIDE_SECRET_KEY`).** Nó mã hoá các secret đã lưu (secret đăng nhập hai bước, secret ký webhook, thông tin xác
thực nhà cung cấp) và là khoá của hash địa chỉ IP khách truy cập.

1. Đặt giá trị mới, và giá trị cũ vào `AUTOXIDE_SECRET_KEY_PREVIOUS`, trên mọi server; khởi động lại.
2. Chạy `autoxide migrate up` một lần nếu vừa nâng cấp, rồi `autoxide secrets reseal`: nó mã hoá lại mọi secret đã lưu bằng key mới.
3. Gỡ `AUTOXIDE_SECRET_KEY_PREVIOUS` khỏi mọi server và khởi động lại.
4. Backup làm trước lần xoay vẫn mở được bằng key cũ: xoá chúng hoặc giữ an toàn như key đã từng. Nếu **database** (hay một
   backup) lộ cùng key thì các secret bên trong phải coi là đã lộ: yêu cầu các tenant xoay thông tin xác thực nhà cung cấp và
   secret webhook của họ.

**Một API key hay thông tin xác thực nhà cung cấp.** Thu hồi key (`DELETE /api/v1/auth/all-api-keys/{id}` cho key của bất kỳ
người dùng nào, cần `auth.users.manage`; người dùng tự thu hồi key của mình bằng `DELETE /api/v1/auth/api-keys/{id}`);
`POST /api/v1/auth/api-keys/{id}/rotate` thay một key bằng key mới. Thông tin xác thực nhà cung cấp (thanh toán, SMS, vận chuyển)
đổi ở trang quản trị của nhà cung cấp trước, rồi ở cài đặt của tenant.

## Thu hồi hàng loạt API key và phiên

Dùng khi tài khoản hay thiết bị có thể đã bị chiếm mà bạn không biết cái nào. Ngăn chặn trước:

- **Dừng hẳn một tenant:** `autoxide tenant suspend <id-hoặc-slug>` (mọi request tới nó trả 403) hoặc
  `PATCH /api/v1/kernel/tenants/{id}` với `{"status": "suspended", "reason": "…"}` (permission `kernel.tenants.manage`, platform
  staff). Hoàn tác bằng `autoxide tenant resume <id-hoặc-slug>` hoặc `"status": "active"`. Muốn "đọc được nhưng không sửa được"
  dùng `autoxide tenant read-only <id-hoặc-slug>` hoặc `{"read_only": true}`, hoàn tác bằng `autoxide tenant writable`.
- **Đăng xuất một người dùng ở mọi nơi:** `DELETE /api/v1/auth/users/{id}/sessions` (permission `auth.users.manage`). Access token
  đã cấp vẫn dùng được tới khi hết hạn (mặc định 15 phút, `AUTOXIDE_AUTH_ACCESS_TOKEN_SECONDS`). Vô hiệu hoá tài khoản
  (`PATCH /api/v1/auth/users/{id}`) chặn đăng nhập mới.
- **Đăng xuất mọi người dùng của một tenant:** không có lệnh gọi một phát; lặp qua danh sách người dùng (danh sách chia trang:
  `limit` tới 200, `cursor` lấy từ `pagination.next_cursor`):

  ```bash
  cursor=""
  while :; do
    page="$(curl -sf -H "Authorization: Bearer $TOKEN" "$API/api/v1/auth/users?limit=200${cursor:+&cursor=$cursor}")"
    jq -r '.data[].id' <<<"$page" | while read -r id; do
      curl -sf -X DELETE -H "Authorization: Bearer $TOKEN" "$API/api/v1/auth/users/$id/sessions" >/dev/null
    done
    cursor="$(jq -r '.pagination.next_cursor // empty' <<<"$page")"
    [ -n "$cursor" ] || break
  done
  ```

  Sau đó đổi `AUTOXIDE_AUTH_TOKEN_SECRET` (xem "Lộ key") để access token chết ngay thay vì trong 15 phút.
- **Thu hồi mọi API key của một tenant:** liệt kê bằng `GET /api/v1/auth/all-api-keys` (chia trang như trên) và gọi
  `DELETE /api/v1/auth/all-api-keys/{id}` cho từng key; dựng vòng lặp như trên. Sau đó cấp lại các key hợp lệ
  (`POST /api/v1/auth/api-keys`).
- **Người dùng mất thiết bị hay yếu tố thứ hai:** `autoxide user reset-mfa` và `autoxide user remove-passkeys` trên server, hoặc
  `DELETE /api/v1/auth/users/{id}/mfa` và `DELETE /api/v1/auth/users/{id}/passkeys` qua API.

Sau đó đọc audit log xem các thông tin xác thực bị chiếm đã làm gì (xem
[khách báo mất dữ liệu](#khách-báo-mất-dữ-liệu)).

## Khôi phục một khách từ backup

Không có lệnh khôi phục một tenant vào database chung đang chạy: nó sẽ ghi đè dữ liệu mới của tenant bằng dữ liệu cũ và không
hoàn tác được. Chọn theo cách tenant được lưu.

- **Tenant có database riêng (placement riêng).** Khôi phục database của placement đó từ backup của nó như ở
  [`DEPLOY.vi.md`](DEPLOY.vi.md#khôi-phục-từng-bước), hoặc từ point-in-time recovery của PostgreSQL về giây trước khi sai.
  Suspend tenant trước (`autoxide tenant suspend <id-hoặc-slug>`), khôi phục, chạy `autoxide doctor` và
  `autoxide migrate up --all`, rồi `autoxide tenant resume <id-hoặc-slug>`.
- **Tenant dùng chung database chính.** Khôi phục backup vào một **bản cài tạm** (database và nơi lưu dự phòng, cùng
  `AUTOXIDE_SECRET_KEY`):

  ```bash
  # trên máy dự phòng, DATABASE_URL và cài đặt nơi lưu trỏ tới đích TRỐNG
  autoxide restore ./restore-dir        # áp migration, chép mọi bảng và file
  autoxide doctor
  autoxide serve
  ```

  Ở đó đọc dữ liệu của tenant (qua API, hoặc `POST /api/v1/kernel/tenants/{id}/exports`, permission `kernel.data.export`, rồi
  `GET …/exports/{export_id}/content`), và đưa thứ đã mất trở lại qua API đang chạy: từng bản ghi (`POST`/`PATCH` của module),
  một revision (`POST /api/v1/content/entries/{id}/revisions/{revision_id}/restore`), một mục trong thùng rác
  (`POST /api/v1/media/{id}/restore`), nhập CSV nơi module có (ví dụ `POST /api/v1/catalog/imports`). Mất lớn thì chuyển tenant
  sang placement riêng (`autoxide tenant move … --dry-run` trước) để lần sau khôi phục được riêng một database.
- Sau đó chạy các kiểm tra riêng của module nơi có (`autoxide task inventory.verify_levels`,
  `autoxide task catalog.rebuild_storefront tenant=<slug>`) và báo tenant đã khôi phục gì và mất gì (mọi thay đổi sau backup).

## Khách báo mất dữ liệu

1. **Chưa đổi gì vội.** Ghi tenant, bản ghi, lần cuối khách thấy nó, và ai thao tác được trong tenant.
2. **Tìm điều đã xảy ra.** Audit log (permission `audit.read`):
   `curl -H "Authorization: Bearer $TOKEN" "$API/api/v1/audit?subject=<id bản ghi>&limit=50"`; thu hẹp bằng `module`, `action`,
   `actor_user_id`, `actor_kind` (một người, một API key hay một AI agent). Request log của tenant
   (`GET /api/v1/kernel/devtools/requests`, permission `kernel.devtools.read`) cho thấy các lời gọi đã đăng nhập gần đây trong
   `AUTOXIDE_REQUEST_LOG_DAYS` ngày (mặc định 14). Tìm trong log server bằng `request_id` và `tenant_id` (xem "Theo dõi").
3. **Do người hợp lệ xoá hay sửa:** phần lớn việc xoá hoàn tác được từ module — một revision nội dung
   (`POST /api/v1/content/entries/{id}/revisions/{revision_id}/restore`), một media trong thùng rác
   (`POST /api/v1/media/{id}/restore`). Không thì lấy giá trị cũ từ mục audit và ghi lại qua API.
4. **Do một thông tin xác thực không được phép thao tác:** coi là bị chiếm: thu hồi
   ([thu hồi hàng loạt](#thu-hồi-hàng-loạt-api-key-và-phiên)), rồi khôi phục như trên.
5. **Không có dấu vết thay đổi mà dữ liệu biến mất:** nghi ngờ nền tảng: kiểm `autoxide doctor`, lịch sử migration
   (`autoxide migrate status`), các lần triển khai gần đây và truy cập database bằng tay. Khôi phục từ backup hay point-in-time
   recovery ([ở trên](#khôi-phục-một-khách-từ-backup)).
6. **Báo khách** bạn thấy gì, đã khôi phục gì và gì không được. Nếu dữ liệu cá nhân bị lộ hay mất, theo
   [`DEPLOY.vi.md`, "Sự cố dữ liệu cá nhân"](DEPLOY.vi.md#sự-cố-dữ-liệu-cá-nhân) (có thời hạn).

## Theo dõi: xem gì và khi nào cảnh báo

Bật `AUTOXIDE_HTTP_METRICS=true` và cho `/metrics` port riêng (`AUTOXIDE_HTTP_METRICS_ADDRESS`); không bao giờ để nó ra
internet. Có `OTEL_EXPORTER_OTLP_ENDPOINT` thì trace cũng được xuất, mang `tenant_id` (metric không có label theo tenant, theo
thiết kế; tra một tenant trong log và trace).

| Xem (metric)                                              | Khoẻ                             | Cảnh báo (file `autoxide-alerts.yml`)             | Rồi                          |
| --------------------------------------------------------- | -------------------------------- | ------------------------------------------------- | ---------------------------- |
| `http_requests_total{status=~"5.."}` trên mọi request     | dưới 1%                          | `AutoxideHigh5xxRate` trên 2% trong 10 phút       | `DEPLOY.vi.md` "Lỗi máy chủ" |
| `http_request_duration_seconds` p95 theo `route`          | dưới 0,5 s                       | `AutoxideSlowP95` trong 10 phút                   | "Database đầy hoặc chậm"     |
| `database_connections{state="in_use"}` / `…_max`          | dưới 70%                         | `AutoxideDbPoolExhausted` ở 90%                   | "Database đầy hoặc chậm"     |
| `jobs_oldest_pending_seconds`, `events_oldest_pending_seconds` | vài giây                    | `AutoxideJobsStuck`, `AutoxideEventsBacklog`      | "Hàng đợi việc nền bị ứ"     |
| `jobs_dead`, `events_dead`                                | 0                                | `AutoxideDeadLetterGrowing`                       | `autoxide jobs failed`       |
| `backup_last_success_timestamp_seconds`, `backup_runs_total{outcome="failed"}` | trong một chu kỳ | `AutoxideBackupTooOld`, `AutoxideBackupFailed` | `DEPLOY.vi.md` "Backup" |
| `http_rate_limited_total`, `http_api_key_rate_limited_total`, `http_tenant_rate_limited_total` | gần 0 | `AutoxideRateLimitedClients` trên 1/s trong 15 phút | "Thu hồi hàng loạt" |
| `webhooks_failed_total` so với `webhooks_delivered_total`  | dưới 5% lỗi                      | `AutoxideWebhooksFailing` trên 20% trong 30 phút  | chủ của các bên nhận         |
| `mail_bounces_total{kind="hard"}`                         | ít                               | `AutoxideMailHardBounces` trên 20 mỗi giờ         | `DEPLOY.vi.md` "Thư bị dội" |

Các series khác đáng có một panel (mặc định không cảnh báo): `database_write_retries_total` (tranh chấp ghi),
`kernel_request_log_dropped_total` (request log quá tải), `entitlement_denials_total` (chạm giới hạn gói),
`mcp_tool_calls_total` (lưu lượng agent), `jobs_retried_total`, `backup_size_bytes` và `backup_duration_seconds` (tăng trưởng).

Ngưỡng chỉ là điểm xuất phát: theo dõi vài tuần rồi siết lại theo mức bình thường của triển khai của bạn. Mỗi cảnh báo có chú
thích `action` nói phải làm gì; thêm định tuyến (mức `page` đánh thức người, `warn` đi vào một kênh) ở Alertmanager.

## Sau sự cố

Trong một tuần viết một bản rà soát ngắn: chuyện gì xảy ra và lúc nào, phát hiện thế nào, đã làm gì, tốn gì, và đổi gì để không
lặp lại (một cảnh báo, một kiểm tra, một bước thêm vào trang này). Sửa runbook ở chỗ thực tế khác nó. Lỗ hổng trong chính
Autoxide đi theo [`SECURITY.vi.md`](../SECURITY.vi.md), không qua issue công khai.
