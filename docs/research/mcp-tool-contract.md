# Hợp đồng tool MCP đầy đủ (bước 71, G4 mục 4 + G13 phần tool + UT-5)

Thiết kế trước khi code; rà độc lập rồi mới làm PR (1). Ngày 27/11/2026, bản 3 (sau hai lượt rà). Căn cứ: đặc tả MCP
2026-07-28 (tools, pagination, progress, cancellation, elicitation theo nhiều lượt request — "MRTR"), extension
`io.modelcontextprotocol/tasks` (repo ext-tasks), bản 2025-11-25 cho client cũ; hợp đồng kernel
`docs/research/kernel-contract-1-0.md` (§5 đăng ký trong `start`, §13 đánh phiên bản, §14.1 bước egress); D-157
(idempotency), D-213 (job theo dõi được, bước 51 P15); báo cáo dùng thử `docs/trials/rc-trial-r1/` (UT-5, #564).

## 0. Hiện trạng (đo trên `main`, `docs/mcp-tools.json`)

- 312 tool, 347 chỗ `Tool::new` trong mã. Tool có `name`, `description`, `inputSchema`, `annotations` chỉ gồm
  `readOnlyHint` / `destructiveHint`; **không** có `title`, `outputSchema`, `idempotentHint`, `openWorldHint`.
- Kết quả: `structuredContent` = `{"data": …}` (cùng envelope REST) kèm bản text, nhưng kiểu ra là `O: Serialize` →
  `serde_json::Value`: không có schema, không qua bước egress của kernel.
- 1.248 tham số đầu vào, **143 tham số ở 63 tool không có mô tả** (UT-5 điểm 9).
- `tools/list` trả một lần cả danh sách (không `nextCursor`); không progress, không huỷ, không task, không elicitation.
- Lỗi tool: `isError: true`, `structuredContent` = `{code, message, reason?, errors?}`.
- Job theo dõi được (`kernel_job_runs`, D-213): trạng thái `queued|running|succeeded|failed`, progress, kết quả;
  `GET /api/v1/kernel/jobs/{id}`; principal của job là `PrincipalSnapshot`. Không job nào dọn dòng `kernel_job_runs` (chỉ
  có hạn giữ file của job).

## 1. Khai tool: `Tool::typed` (phần A)

Tên hàm do phiên này quyết (hợp đồng kernel dòng 687):

```rust
// Trong `Module::start` (§5 hợp đồng kernel): context.tools([...]).
Tool::typed("list_price_lists", "List the price lists of the tenant, …", list_price_lists)
    .title("List price lists")        // bắt buộc với `typed`: tên cho người, tiếng Anh
    .requires(CATALOG_READ)
    .read_only()                      // readOnlyHint = true, idempotentHint = true
    // .idempotent()                  // lệnh ghi gọi lại cùng tham số không đổi thêm gì (vd đặt giá trị)
    // .destructive()                 // destructiveHint
    // .open_world()                  // tool chạm thế giới ngoài: gửi mail/SMS, gọi nhà cung cấp, gọi mô hình AI, webhook
    // .long_running()                // service của tool enqueue job có theo dõi (§3)
    // .ask_for("price_list_id", "Which price list?", price_list_choices)   // §4

async fn list_price_lists(call: ToolCall, input: ListPriceLists) -> Result<Page<PriceListView>, Error>
```

- `Tool::typed::<I, O>`: `I: Deserialize + Validate + Schema`, **`O: Schema + Serialize`**. `ToolCall` là struct
  `#[non_exhaustive]` của kernel: `tenant()`, `principal()`, `progress()`, `cancelled()`. Gom vào một tham số để thêm khả
  năng sau này không đổi chữ ký của 312 handler lần nữa.
- **`ToolCall` cho người gọi không phải MCP** (automations, bước 58, gọi qua `call_tool`): `progress()` không làm gì,
  `cancelled()` luôn `false`, tool `.long_running()` chạy tại chỗ như mọi tool (service vẫn enqueue job của nó), trường
  `.ask_for` thiếu thì trả `kernel.choice_required` (§4) — không bao giờ treo.
- `Tool::new` không gắn `#[deprecated]`: gỡ thẳng trước 1.0 khi chuyển hết, theo dõi bằng baseline của gate
  `mcp-output-schema` (D-20261127-transition-paths, #580).
- **Annotations** sinh từ khai báo và **luôn phát đủ bốn hint tường minh**: `readOnlyHint`, `destructiveHint`,
  `idempotentHint` (`true` khi `read_only`; lệnh ghi `false` trừ khi gọi `.idempotent()`), `openWorldHint` (`false` trừ khi
  `.open_world()`). Kiểm lúc khởi động (`Tool::problems`): `read_only` + `destructive` là lỗi; `title` rỗng hay thiếu (tool
  `typed`) là lỗi; `.long_running()` cùng `.returns_secret()` là lỗi (kết quả bí mật không nằm lại trong bảng job).
- **`outputSchema`** = schema của đúng thứ nằm trong `structuredContent`: `{"type": "object", "anyOf": [<thành công>,
  {"$ref": "#/$defs/ToolError"}]}`. Thành công: `Data<O>` → `{data: O}`, `Page<O>` → `{data: [O], pagination}` (đòi `data`);
  lỗi: hình dạng hiện có `{code, message, reason?, errors?}` (đòi `code` + `message`, không phá client đang đọc). Lý do khai cả
  lỗi: mã SDK TypeScript 1.32.2 (`client/index.js`, `callTool`) kiểm `structuredContent` theo `outputSchema` **cả khi**
  `isError`. Kiểu lồng nằm trong `$defs` cục bộ với `$ref: "#/$defs/…"` (bộ kiểm JSON Schema nào cũng giải được; đầu vào vẫn
  giữ luật "không `$ref`" vì client dựng form từ nó).
- **Egress** (hợp đồng kernel dòng 755, §14.1): kernel không gọi `serde_json::to_value(output)` trần nữa mà đưa `O` qua bước
  egress chung với REST (`Data`/`Page`): `egress(&principal, &output) -> Value`, che trường đã phân loại theo quyền của người
  gọi. **Thân lỗi và bản text** của kết quả cũng đi qua egress (giá trị trong `params` của lỗi, như `Problem` của REST). Tool
  và REST dùng **một** hàm, nên không lối nào lộ hơn lối kia. Kết quả của task (§3) đi qua egress lúc trả, không lúc lưu.
- Không kiểm `structuredContent` với `outputSchema` lúc chạy (tốn mỗi lần gọi); kiểm trong test: bộ kiểm hợp đồng của testkit
  (đã kiểm response REST theo OpenAPI) thêm kiểm kết quả tool theo `outputSchema` cho mọi lời gọi tool trong test. Job CI
  `mcp-client` (PR 3 của G4) thêm một ca thành công và một ca lỗi qua SDK, với output có `uuid`, số nguyên 64 bit và `$defs`.

**Phụ thuộc G1** (báo phiên "Hợp đồng kernel"):

| Phần | Cần gì của G1 | Làm trước được không |
| --- | --- | --- |
| Ràng buộc `O: Schema` | PR 4 (`#[derive(Schema)]`) | Được: trước PR 4 ràng buộc là `utoipa::ToSchema`; `Schema` của PR 4 cài `ToSchema` nên đổi ràng buộc không làm module sửa gì |
| Egress che theo lớp | PR 4 (`data_class`, trait `Redact`) + bước 54 (hành vi che) | Được: PR (1) đưa tool qua **cùng hàm** egress mà `Data`/`Page` dùng (hôm nay là `to_value`); khi G1/54 thêm che, tool có ngay |
| Đăng ký `context.tools(…)` trong `start` | PR 5 (`Manifest` + `StartContext`) | Được: `Tool::typed` dùng với `Module::tools` hôm nay; PR 5 chỉ dời chỗ gọi |
| Cột `tool`, `cancel_requested_at` của `kernel_job_runs` | PR 3 (migration `Plan`) chỉ đổi cách khai | Được: migration kernel kiểu hiện có; PR 3 chuyển cùng các migration khác, giữ tên |

## 2. `tools/list` chia trang (phần B)

- Sắp theo tên đầy đủ; lọc theo quyền **trước** khi chia trang, nên đổi quyền giữa hai trang chỉ làm trang sau ngắn đi, không
  lộ tool.
- **Bản mới (2026-07-28):** trang mặc định 500 tool (lớn hơn 312 hiện có), để client bỏ qua `nextCursor` không mất tool;
  `nextCursor` = base64url của `{after: "<tên tool cuối>", v: 1}`. Cursor chỉ chứa tên tool người gọi đã thấy, nên không cần
  ký; cursor hỏng → JSON-RPC `-32602` kèm câu "gọi lại không có `cursor`".
- **Bản cũ:** một trang duy nhất (không `nextCursor`), như hôm nay.
- `ttlMs: 0`, `cacheScope: private` giữ như hiện có. `resources/list`, `prompts/list` dùng cùng cách.

## 3. Việc dài: progress, huỷ, task (phần C)

### 3.1 Progress qua SSE (lời gọi thường)

- Chỉ trả SSE khi request có `_meta.progressToken` **và không có** `Idempotency-Key` (kết quả lưu lại cho idempotency là một
  response JSON; không trộn hai thứ). Còn lại trả JSON như hôm nay; `call.progress().report(done, total, message)` không làm
  gì.
- Có SSE: `notifications/progress` gửi trên chính stream trả lời, tối đa 1 thông báo / 250 ms; header `X-Accel-Buffering:
  no` để proxy không gom; timeout của request (`enforce_timeout`) áp **bên trong** stream (hết giờ thì gửi kết quả lỗi rồi
  đóng, như JSON).
- Việc chạy **bên trong** future của body stream, không `tokio::spawn` tách rời: client đóng thì future bị bỏ, không có việc
  mồ côi.

### 3.2 Huỷ khi client đóng kết nối

- Bản 2026-07-28: huỷ = client đóng stream. `notifications/cancelled` (bản cũ) **không** ánh xạ: vẫn trả 202 và bỏ qua (server
  stateless không biết request nào đang chạy ở đâu).
- Future của request bị bỏ: transaction ghi đang mở bị **rollback** khi drop (có test cho cả hai database). Transaction đã
  commit vẫn còn (như REST khi client ngắt). Job đã enqueue cùng transaction đã commit **vẫn chạy** (D-054).
- **Idempotency khi bị bỏ (theo D-157, không nhả bản ghi):** drop guard của lớp idempotency ghi `idempotency_result_unknown`
  như khi hết giờ — lần gửi lại cùng key nhận 409 và lời khuyên tra kết quả trước. Chỉ được xoá bản ghi "đang chạy" khi cờ
  "đã có tác động" của request **chưa bật**. Cờ theo hướng thận trọng:
  - Bật khi có **bất kỳ** câu lệnh nào đi qua pool ghi (kể cả lệnh sau đó rollback), qua `Database::observe_statements` —
    hôm nay chỉ có một observer (testkit dùng); tách thành danh sách observer dùng chung để cả testkit và cờ này cùng gắn.
  - Bật cả khi request gọi ra ngoài qua HTTP client hay lớp storage (file) của kernel.
  - Cờ giữ bằng `tokio::task_local!`, đặt ở middleware idempotency; chỉ phủ task của chính request — việc `spawn` ra không
    được tính (kernel không `spawn` trong đường request; job đi qua job queue, đã ghi trong transaction nên cờ đã bật).
  - `Drop` là đồng bộ nên guard `spawn` việc ghi `result_unknown` (hoặc xoá bản ghi). Tiến trình tắt hay crash trước khi việc
    đó xong: `locked_until` của bản ghi quyết (hết khoá thì lần gửi lại chạy lại, như hôm nay) — giới hạn đã biết, ghi trong
    tài liệu.
  - Middleware dùng chung nên **REST cũng đổi hành vi** (client ngắt giữa chừng → gửi lại nhận 409 `idempotency_result_unknown`
    thay vì chờ hết khoá): có changelog.

  Test trên cả hai database: bỏ trước mọi câu lệnh ghi (bản ghi bị xoá, gửi lại chạy được) và bỏ sau một câu lệnh ghi (409
  `idempotency_result_unknown`).
- `call.cancelled()` trả `true` sau khi kết nối đóng (nếu future còn chạy tới điểm kiểm), cho handler dài dừng giữa hai bước,
  không bao giờ giữa transaction.
- **Best-effort sau proxy:** proxy có thể giữ kết nối tới server sau khi client đóng; khi đó server không thấy huỷ và lời gọi
  chạy xong. Tài liệu `mcp.md` ghi rõ: huỷ không đảm bảo, dùng idempotency key cho lệnh ghi.

### 3.3 Task (extension `io.modelcontextprotocol/tasks`)

Task chỉ là **cách nhìn theo giao thức lên một dòng `kernel_job_runs`** (D-213) — không có bảng mới.

- `.long_running()` chỉ đánh dấu tool mà service **vốn đã** enqueue job có theo dõi (xuất file, dời, nhập — đã có sẵn).
  `O` của tool đó là **view của job run** (cùng `JobRunView` của `GET /api/v1/kernel/jobs/{id}`), đi qua cùng egress. Không có
  tool nào được "biến thành" job bởi kernel.
- Thêm cột `tool` (nullable) vào `kernel_job_runs`: tên đầy đủ của tool khi dòng được tạo qua `tools/call`. `tasks/*` chỉ phục
  vụ dòng có `tool`; dòng không có trả "Unknown task".
- Không có `taskSupport` trên tool. Server tự quyết: chỉ khi request **khai extension** (trong `_meta` của request theo bản
  2026-07-28) và tool là `.long_running()`, kết quả là `CreateTaskResult` phẳng mang `resultType: "task"`: `taskId` (= id của
  job run), `status`, `createdAt`, `lastUpdatedAt`, `ttlMs`, `pollIntervalMs`. `decorate` của transport (`transport.rs`, hôm
  nay luôn ghi `resultType: "complete"`) không được ghi đè `resultType` đã có; áp tương tự cho `input_required` (§4).
  Request không khai extension: trả kết quả thường như hôm nay (view của job run trong `data`).
- Ánh xạ trạng thái: `queued`/`running` → `working`; `succeeded` → `completed`; job lỗi (lỗi của tool) → `completed` kèm kết
  quả `isError: true`, thân `{code: "kernel.job_failed", message}` (có bản dịch i18n); `failed` của task chỉ dùng cho lỗi
  JSON-RPC; huỷ xong → `cancelled`. Task không có progress: progress của job đổ vào `statusMessage`.
- Các method `tasks/*` của extension (đọc trạng thái, lấy kết quả, huỷ — đúng tên và hình dạng của extension; **không** có
  `tasks/update` và `input_required` trong 1.0):
  - Client bản 2026-07-28 không khai extension mà gọi `tasks/*` → `-32021`, HTTP 400 (thêm vào `error_status` của transport,
    dẫn đặc tả, có test). Bản 2025-11-25 trở về trước: `tasks/*` → `-32601` (method không có).
  - Header `Mcp-Name` của `tasks/*` so với `params.taskId` (mở rộng bảng kiểm header của transport); lệch → `-32020`, HTTP
    400, như `tools/call`.
  - Tra theo `(tenant_id, id)` **và** người tạo (job run của chính người gọi, như `GET /api/v1/kernel/jobs/{id}` không có
    `kernel.jobs.read`); rồi chạy lại `authorize` của tool và kiểm gói với principal **hiện tại**. Không khớp hay không đạt →
    **đúng lỗi như task không tồn tại**: `-32602` "Unknown task", ánh xạ HTTP như "Unknown tool" — không 403, để không lộ task
    của người khác có tồn tại.
  - Kết quả trả qua egress với principal hiện tại; chịu trần kích thước như kết quả lưu của idempotency (256 KiB); quá trần
    thì kết quả trỏ tới REST `GET /api/v1/kernel/jobs/{id}` (file của job tải qua đó).
- **Huỷ:** thêm cột `cancel_requested_at` vào `kernel_job_runs` và `POST /api/v1/kernel/jobs/{id}/cancel` (người tạo, hoặc
  `kernel.jobs.manage`) để REST làm được như MCP (luật 8); `tasks/cancel` gọi cùng service. Worker đọc cờ qua
  `call.cancelled()` / hàm tương ứng của job, dừng ở điểm an toàn kế tiếp, không giữa transaction. Trạng thái cuối ghi bằng
  compare-and-set: `UPDATE … SET status = 'cancelled' WHERE id = ? AND status IN ('queued', 'running')` (và tương tự cho
  `succeeded`/`failed`), nên cancel đua với lúc job xong chỉ một bên thắng. Phần đã commit giữ nguyên; job con đã enqueue vẫn
  chạy.
- **Trạng thái cuối không đổi được nữa:**
  - `progress()` của job ghi có điều kiện trạng thái (`WHERE status IN ('queued', 'running')`), không kéo dòng đã xong hay đã
    huỷ về `running`.
  - Worker bỏ qua job lúc nhận việc nếu `cancel_requested_at` đã đặt hoặc trạng thái là `cancelled` (ghi `cancelled` bằng
    compare-and-set nếu còn `queued`).
  - `POST /api/v1/kernel/jobs/{id}/retry` với dòng thuộc một task (`tool` không null): từ chối bằng mã ổn định
    `kernel.job_task_not_retryable` (409) — client gọi lại tool thì tạo run mới. Dòng không phải task giữ hành vi retry hiện có.
  - `cancelled` thêm vào bộ lọc `status` của `GET /api/v1/kernel/jobs` và vào các giá trị ghi trong `JobRunView`.
- **Principal của job:** `PrincipalSnapshot` như hiện có (D-213: quyền tại lúc yêu cầu). Kiểm credential còn sống (OAuth grant
  bị thu hồi, key bị khoá) trước khi job chạy đi qua một trait của kernel có cài mặc định "không kiểm"; module `auth` cài nó.
  Ghi thành decision khi làm PR (3).
- **Hạn giữ và vòng đời dữ liệu:** `kernel_job_runs` hôm nay không có job dọn dòng (chỉ dọn file) — PR (3) thêm việc dọn của
  kernel xoá dòng đã xong quá hạn (`ttlMs` báo đúng hạn đó) và kiểm bảng nằm trong luồng xoá, dời (đã có trong
  `relocate/kernel_tables.rs`) và xuất dữ liệu của khách.
- Chạy trên cả hai database: chỉ cột mới + câu `UPDATE` có điều kiện. Test đồng thời: hai `cancel` cùng lúc, `cancel` đua với
  lúc job xong — SQLite và PostgreSQL.

## 4. Elicitation (phần D)

Dùng khi một trường đầu vào **tuỳ chọn** thiếu mà cần người chọn (vd nhiều bảng giá, "áp cho bảng nào?"). Chỉ theo danh sách
cho phép tĩnh:

```rust
Tool::typed("add_prices", "…", add_prices)
    .ask_for("price_list_id", "Which price list?", price_list_choices) // câu hỏi `&'static str`

/// Hàm đọc-chỉ: các lựa chọn hợp lệ cho người gọi (kernel chạy trước handler).
async fn price_list_choices(call: &ToolCall, input: &AddPrices) -> Result<Vec<Choice>, Error>
```

- Tool khai **những trường tuỳ chọn của chính nó** được hỏi lại (`ask_for`), kèm hàm đọc-chỉ trả danh sách lựa chọn.
  `Tool::problems` kiểm lúc khởi động: trường phải có trong schema đầu vào, phải tuỳ chọn, không khớp mẫu bí mật
  (`password`, `secret`, `token`, `otp`, `card`, `cvv`, `pin`).
- Kernel chạy hàm lựa chọn **trước** handler khi trường thiếu: không lựa chọn nào → chạy không có trường đó, như hôm nay; một
  lựa chọn → dùng luôn; nhiều → hỏi. Câu trả lời thành đối số và đi qua **cùng** `Validate` + kiểm của service như khi agent tự
  gửi.
- `ask_for` xuất ra `docs/mcp-tools.json` dưới `_meta["autoxide/ask_for"]` (tên trường + câu hỏi) để `api_diff` thấy (§6).
- Chỉ có dạng chọn (enum): câu hỏi là `&'static str` (tiếng Anh, của code); nhãn lựa chọn là dữ liệu của khách, chỉ xuất hiện
  làm `title` của giá trị enum, được đánh dấu dữ liệu chưa tin cậy (mục prompt injection của G4 mục 3). Không có `text` tự do,
  không `format: password`, không elicitation dạng URL trong 1.0.
- **Không bao giờ thay `x-approval`.** Tool `requires_approval_for_agent` vẫn trả `approval_required` và trỏ về REST; câu trả
  lời elicitation không được tính là người duyệt.
- **Chỉ bản 2026-07-28:** trả `resultType: "input_required"` kèm `inputRequests` và `requestState`; client gọi lại
  `tools/call` cùng `inputResponses` và `requestState`. Client bản cũ, request không khai elicitation, lời gọi từ automation,
  và tool chạy thành task: **không bao giờ** nhận `input_required` (server stateless không biết capability), mà nhận lỗi
  kernel `kernel.choice_required` với `params.field` và `params.options` (qua egress, nhãn đánh dấu chưa tin cậy) để gọi lại
  với đối số đủ.
- **`requestState`**: tách lớp bọc HMAC của antispam (`security/antispam/token.rs`: `b64(payload).b64(mac)`, so sánh thời
  gian hằng) thành helper `pub(crate)` dùng chung cho cả hai, không thêm định dạng token mới; khoá dẫn xuất riêng
  `SecretKey::derive_for(tenant, "autoxide/mcp/request-state/v1")`. Payload riêng có số phiên bản (`v: 1`) và `exp` rõ ràng,
  kiểm lúc mở; payload được **ký nhưng không mã hoá** nên chỉ chứa id và lựa chọn, không chứa dữ liệu nào khác. Nội dung:
  tenant, người gọi theo **định danh ổn định** (id của API key, id của OAuth grant, id của session — không bao giờ là access
  token đang xoay), tên tool, SHA-256 của `arguments` chuẩn hoá theo JCS (RFC 8785), các câu trả lời trước, hạn = lúc phát
  hành cộng **10 phút**. Kiểm: chữ ký, hạn, tenant, người gọi, tool, hash tham số — sai bất kỳ thì `-32602` "requestState is
  invalid or expired, call the tool again without it". Không ép dùng một lần: TTL ngắn + các ràng buộc trên đủ, vì
  `requestState` chỉ chở câu trả lời chọn trong danh sách mà hàm lựa chọn tính lại mỗi lượt, và lệnh ghi vẫn đi qua
  idempotency.

## 5. Gate mới (phần A)

- `mcp-output-schema`: tool trong `docs/mcp-tools.json` thiếu `output_schema` hay `title` → đỏ, trừ tên trong
  `xtask/mcp-output-baseline.txt` (312 dòng lúc đầu, **chỉ giảm**: gate đỏ khi baseline có tên không còn thiếu, để xoá dòng).
  Baseline rỗng là điều kiện gỡ `Tool::new`.
- `mcp-param-description`: mọi thuộc tính của `input_schema` có `description` (kiểu enum Rust tự có `enum`); thiếu → đỏ, trừ
  `tool.param` trong `xtask/mcp-param-baseline.txt` (143 dòng lúc đầu, chỉ giảm). PR (1) điền hết mô tả cho các tool UT-5 nêu
  tên.
- Mỗi gate một dòng cách sửa trong `xtask/src/hints.rs` ("chuyển tool sang `Tool::typed` và thêm `.title(…)`"; "thêm `///`
  cho trường của kiểu đầu vào").

## 6. Đánh phiên bản tool (G13, phần E)

Tên và schema tool là hợp đồng như REST (quy tắc chung §13 hợp đồng kernel). Mở rộng luật `tool.*` sẵn có trong
`xtask/src/api_diff.rs` (`tool.removed`, `tool.parameter_*`, `tool.constraint_tightened`, `tool.no_longer_read_only`,
`tool.approval_added`, `tool.result_*`):

- **Không phá vỡ:** thêm tool; thêm tham số tuỳ chọn; thêm trường vào kết quả; thêm giá trị enum ở enum khai mở; nới quyền
  (thêm quyền thay thế).
- **Phá vỡ** (luật mới thêm vào `api_diff.rs`): thêm `session_only`; trường kết quả thành nullable hay tuỳ chọn; siết quyền;
  thêm `openWorldHint`; `idempotentHint` từ `true` sang `false`; thêm `destructiveHint`; thêm `ask_for` vào tool đã có (lời
  gọi cũ thiếu trường đó trước chạy theo mặc định, nay nhận `kernel.choice_required`). Làm tool mới hậu tố `_v2` (tên tool
  chỉ cho `[a-z0-9_]`, nên không dùng `.v2` như event), chạy song song; tool cũ khai `Deprecation::tool(since, sunset,
  "<module>.<tên>_v2")` (D-240), gỡ ở bản lớn.
- "Không trả `input_required` hay task cho client không khai" không kiểm bằng diff mà bằng **test lúc chạy** (PR 3, PR 4):
  gọi mọi tool `long_running` và mọi tool có `ask_for` bằng client bản cũ và client bản mới không khai, khẳng định kết quả
  là `complete`.
- Tool của module hạng thử nghiệm: báo, không chặn. Mỗi tool có `_meta["autoxide/stability"]` để client biết hạng.

## 7. UT-5 và #564 (phần A + PR 4)

Đọc nguyên văn UT-5 (ROADMAP nhánh `claude/user-trial-r1`) và bảng vai 3 của báo cáo:

| Điểm | Làm ở |
| --- | --- |
| Mô tả cho mọi tham số, gate kiểm (điểm 9) | PR (1): gate `mcp-param-description` + điền mô tả cho tool đã nêu (`catalog.*`, `invoices.create_draft`…) |
| `catalog.list_price_lists` (điểm 6, "bảng giá") | PR (1): tool đọc trên service sẵn có, `Tool::typed`, keyset như REST; `mcp.md` ghi việc phải làm trước bằng REST khi cài mới |
| Kiểm `tax_category`; giá trị lọc sai (`readiness`, `module`, `subject`) báo lỗi | PR (4): kiểu enum ở đầu vào → schema có `enum`, sai thì `validation_failed` |
| `version` nhận cả hai dạng (điểm 4); `precondition_failed` câu riêng cho MCP (điểm 5) | PR (4) |
| Ẩn tool luôn lỗi với API key (điểm 7); tool ẩn trả "Unknown tool" ghi vào tài liệu (điểm 8) | PR (4): khai `Tool::session_only()` → ẩn khỏi `tools/list` với credential không phải phiên |
| Idempotency của MCP trong `mcp.md` (điểm 3); `Idempotency-Key` trùng thành lỗi tool (điểm 13) | PR (4) |
| Gom lỗi `invoices.create_draft` (điểm 11); tên key trong audit | PR (4) |

Khách và đơn nháp bằng tool (vai 3, việc thứ ba) là tính năng nghiệp vụ mới — ngoài phạm vi G4 (đóng băng tính năng, đường tới
1.0); ghi vào tài liệu là chưa có.

## 8. Chia PR (phần F)

Cùng nhánh, nối tiếp, mỗi PR qua kiểm độc lập:

1. `Tool::typed` + `ToolCall` (cả nhánh không phải MCP) + `title` / bốn hint + `outputSchema` (`anyOf`) qua egress (cả lỗi) +
   kiểm hợp đồng kết quả tool trong testkit + hai gate có baseline + `hints.rs` + `catalog.list_price_lists` + mô tả tham số;
   chuyển `notes` và các tool UT-5 nêu tên làm mẫu.
2. Chuyển 300 tool còn lại sang `Tool::typed` theo cụm module (baseline giảm dần) — **giao được cho phiên khác**. Checklist:
   rà `openWorldHint` từng module (mail/SMS, nhà cung cấp, AI, webhook, job gọi nhà cung cấp) trước 1.0.
3. `tools/list` chia trang; SSE + progress; huỷ khi đóng kết nối + drop guard idempotency (đổi cả REST, có changelog); task
   trên `kernel_job_runs` (cột `tool`, `cancel_requested_at`, trạng thái `cancelled`, `POST …/jobs/{id}/cancel`, retry từ
   chối dòng task, `tasks/*`, mã lỗi giao thức, dọn dòng quá hạn, trait kiểm credential + decision), test cả hai database.
4. Elicitation (`ask_for`, MRTR, `requestState`, `kernel.choice_required`), luật G13 mới trong `api_diff.rs`, phần còn lại của
   UT-5.

Tài liệu `docs/guides/mcp.md` + `.vi.md` cập nhật trong PR nào đổi hành vi người gọi thấy.
