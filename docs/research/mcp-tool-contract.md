# Hợp đồng tool MCP đầy đủ (bước 71, G4 mục 4 + G13 phần tool + UT-5)

Thiết kế trước khi code; rà độc lập rồi mới làm PR (1). Ngày 27/11/2026. Căn cứ: đặc tả MCP 2026-07-28 (tools, pagination,
progress, cancellation, extension `io.modelcontextprotocol/tasks`, elicitation theo nhiều lượt request — "MRTR"), bản
2025-11-25 cho client cũ; hợp đồng kernel `docs/research/kernel-contract-1-0.md` (§5 đăng ký trong `start`, §13 đánh phiên
bản, §14.1 bước egress); báo cáo dùng thử `docs/trials/rc-trial-r1/` (UT-5, nhánh `claude/user-trial-r1`, #564).

## 0. Hiện trạng (đo trên `main`, `docs/mcp-tools.json`)

- 312 tool, 347 chỗ `Tool::new` trong mã. Tool có `name`, `description`, `inputSchema`, `annotations` chỉ gồm
  `readOnlyHint` / `destructiveHint`; **không** có `title`, `outputSchema`, `idempotentHint`, `openWorldHint`.
- Kết quả: `structuredContent` = `{"data": …}` (cùng envelope REST) kèm bản text, nhưng kiểu ra là `O: Serialize` →
  `serde_json::Value`: không có schema, không qua bước egress của kernel.
- 1.248 tham số đầu vào, **143 tham số ở 63 tool không có mô tả** (UT-5 điểm 9).
- `tools/list` trả một lần cả danh sách (không `nextCursor`); không progress, không huỷ, không task, không elicitation.
- Lỗi tool: `isError: true`, `structuredContent` = `{code, message, reason?, errors?}`.

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
    // .open_world()                  // tool chạm thế giới ngoài: gửi mail/SMS, gọi nhà cung cấp, gọi mô hình AI
    // .long_running()                // được chạy thành task (§3)

async fn list_price_lists(call: ToolCall, input: ListPriceLists) -> Result<Page<PriceListView>, Error>
```

- `Tool::typed::<I, O>`: `I: Deserialize + Validate + Schema`, **`O: Schema + Serialize`**. `ToolCall` là struct
  `#[non_exhaustive]` của kernel: `tenant()`, `principal()`, `progress()`, `cancelled()`, `elicit()` (§3, §4). Gom vào một
  tham số để thêm khả năng sau này không đổi chữ ký của 312 handler lần nữa.
- `Tool::new` giữ nguyên tới khi chuyển hết, rồi `#[deprecated]` (gỡ ở PR 16 của G1 cùng các đường cũ).
- **Annotations** sinh từ khai báo: `readOnlyHint`, `destructiveHint`, `idempotentHint` (mặc định `true` khi `read_only`,
  `false` với lệnh ghi trừ khi gọi `.idempotent()`), `openWorldHint` (mặc định `false`). Kiểm lúc khởi động
  (`Tool::problems`): `read_only` + `destructive` là lỗi; `title` rỗng là lỗi; tool `typed` thiếu `title` là lỗi.
- **`outputSchema`** = schema của đúng thứ nằm trong `structuredContent` lúc thành công: `Data<O>` → `{data: O}`, `Page<O>`
  → `{data: [O], pagination}`. Schema tự đứng được: kiểu lồng vào `$defs` cục bộ với `$ref: "#/$defs/…"` (bộ kiểm JSON
  Schema nào cũng giải được; khác với đầu vào, nơi giữ luật "không `$ref`" vì client dựng form từ nó). Gốc luôn là
  `type: object`.
- **Lỗi** (`isError: true`) giữ hình dạng hiện có (`{code, message, reason?, errors?}`, không phá client đang đọc). Đã kiểm
  mã của SDK TypeScript 1.32.2 (`client/index.js`, `callTool`): nó kiểm `structuredContent` theo `outputSchema` **cả khi**
  `isError` — nên `outputSchema` khai cả hai dạng: `{"type": "object", "oneOf": [<thành công>, {"$ref":
  "#/$defs/ToolError"}]}` (thành công đòi `data`, lỗi đòi `code` + `message`, hai nhánh loại trừ nhau). Job CI `mcp-client`
  (PR 3 của G4) thêm một ca gọi tool thành công và một ca lỗi qua SDK để giữ điều này.
- **Egress** (hợp đồng kernel dòng 755, §14.1): kernel không gọi `serde_json::to_value(output)` trần nữa mà đưa `O` qua bước
  egress chung với REST (`Data`/`Page`): `egress(&principal, &output) -> Value`, che trường đã phân loại theo quyền của
  người gọi. Tool và REST dùng **một** hàm, nên không lối nào lộ hơn lối kia. Kết quả của task (§3) đi qua egress lúc trả
  (`tasks/get`), không lúc lưu.
- Không kiểm `structuredContent` với `outputSchema` lúc chạy (tốn mỗi lần gọi); kiểm trong test: bộ kiểm hợp đồng của
  testkit (đã kiểm response REST theo OpenAPI) thêm kiểm kết quả tool theo `outputSchema` cho mọi lời gọi tool trong test.

**Phụ thuộc G1** (báo phiên "Hợp đồng kernel"):

| Phần | Cần gì của G1 | Làm trước được không |
| --- | --- | --- |
| Ràng buộc `O: Schema` | PR 4 (`#[derive(Schema)]`) | Được: trước PR 4 ràng buộc là `utoipa::ToSchema`; `Schema` của PR 4 cài `ToSchema` nên đổi ràng buộc không làm module sửa gì |
| Egress che theo lớp | PR 4 (`data_class`, trait `Redact`) + bước 54 (hành vi che) | Được: PR (1) đưa tool qua **cùng hàm** egress mà `Data`/`Page` dùng (hôm nay là `to_value`); khi G1/54 thêm che, tool có ngay |
| Đăng ký `context.tools(…)` trong `start` | PR 5 (`Manifest` + `StartContext`) | Được: `Tool::typed` dùng với `Module::tools` hôm nay; PR 5 chỉ dời chỗ gọi |
| Bảng `kernel_mcp_tasks`, nonce của `requestState` | PR 3 (migration `Plan`) chỉ đổi cách khai | Được: migration kernel kiểu hiện có; PR 3 chuyển cùng các migration khác, giữ tên |

## 2. `tools/list` chia trang (phần B)

- Sắp theo tên đầy đủ; trang 100 tool; `nextCursor` = base64url của `{after: "<tên tool cuối>", v: 1}`. Cursor chỉ chứa
  tên tool người gọi đã thấy, nên không cần ký; cursor hỏng → JSON-RPC `-32602` kèm câu "gọi lại không có `cursor`".
- Danh sách lọc theo quyền **trước** khi chia trang, nên đổi quyền giữa hai trang chỉ làm trang sau ngắn đi, không lộ tool.
- Áp cho mọi bản giao thức (cursor có từ 2024-11-05). Endpoint theo module `/mcp/<module>` vẫn là cách khuyên cho client
  chỉ đọc một trang (đa số module dưới 100 tool). `ttlMs: 0`, `cacheScope: private` giữ như hiện có.
- `resources/list`, `prompts/list` dùng cùng cách.

## 3. Việc dài: progress, huỷ, task (phần C)

**Progress.** Request có `_meta.progressToken` và client nhận SSE: `call.progress().report(done, total, message)` gửi
`notifications/progress` trên chính stream trả lời (đặc tả 2026-07-28). Không có token hay client chỉ nhận JSON: hàm không
làm gì. Tần suất giới hạn trong kernel (tối đa 1 thông báo / 250 ms) để handler gọi thoải mái.

**Huỷ khi client đóng stream** (2026-07-28; bản cũ: `notifications/cancelled` — kernel ánh xạ về cùng một tín hiệu):

- Lời gọi thường (không phải task): hyper bỏ future của request khi kết nối đóng. Transaction ghi đang mở bị **rollback**
  (transaction của lớp `db` rollback khi drop — có test cho cả hai database), nên không có ghi nửa chừng **trong** transaction
  đó. Transaction đã commit trước đó vẫn còn (giống REST khi client ngắt). Job đã enqueue (ghi cùng transaction đã commit)
  **vẫn chạy**, không bị huỷ theo: job là việc đã cam kết, chịu được chạy lại (D-054). Bản ghi idempotency "đang chạy" của
  request bị bỏ được nhả để lần gọi lại chạy được (test ở PR (3)).
- `call.cancelled()` trả `true` sau khi stream đóng, cho handler dài tự dừng giữa hai bước (không bao giờ giữa transaction).
- Tool `long_running` chạy thành task (dưới) không bị huỷ khi stream đóng: task sống tách khỏi stream; huỷ bằng
  `tasks/cancel`.

**Task** (extension `io.modelcontextprotocol/tasks`, khai trong `capabilities.extensions` và trong tool:
`execution.taskSupport: "optional"` cho tool `.long_running()`):

- Client xin chạy thành task → kernel kiểm quyền, gói, khách chỉ đọc **như lời gọi thường** rồi ghi một dòng
  `kernel_mcp_tasks` (UUIDv7, `tenant_id`, `created_by` = user + credential, `tool`, `arguments_hash`, `status`
  `working|input_required|completed|failed|cancelled`, `result`, `error`, `created_at`, `expires_at`, `cancel_requested_at`)
  và enqueue job `kernel.mcp_task` **trong cùng transaction**; trả `resultType: "task"` kèm `taskId`, `pollIntervalMs`.
- Job chạy handler với `principal` dựng lại từ credential lúc chạy (grant OAuth bị thu hồi hay key bị khoá → task `failed`
  với `unauthenticated`, không chạy bằng quyền cũ). Kết quả lưu dạng chưa egress, chỉ người tạo đọc được.
- `tasks/get`, `tasks/update`, `tasks/cancel`: tra theo `(tenant_id, id)` **và** `created_by` của người gọi; không khớp thì
  trả **đúng lỗi như task không tồn tại** (JSON-RPC `-32602` "Unknown task", HTTP 404 ở bản mới) — không 403, để không lộ
  task của người khác có tồn tại. Kết quả trả qua egress với `principal` hiện tại.
- `tasks/cancel`: đặt `cancel_requested_at`; `call.cancelled()` của job thấy `true`; job dừng ở điểm an toàn kế tiếp, trạng
  thái `cancelled`. Phần đã commit giữ nguyên, job con đã enqueue vẫn chạy (như trên). Task đã xong thì `cancel` không đổi gì.
- Hạn: kết quả giữ 24 giờ rồi bị dọn bằng job dọn sẵn có; `ttlMs` trong trả lời nêu hạn.
- Chạy trên cả hai database: chỉ bảng thường + job queue sẵn có, không cú pháp riêng. Test đồng thời: hai `tasks/cancel` cùng
  lúc, `cancel` đua với lúc job xong — trên SQLite và PostgreSQL.

## 4. Elicitation (phần D)

Dùng khi tool cần người chọn một điều mà tham số không đủ để quyết (vd nhiều bảng giá khớp, "áp cho bảng nào?"). Theo
2026-07-28 không giữ kết nối: tool trả `resultType: "input_required"` kèm `inputRequests` và `requestState`; client gọi lại
`tools/call` cùng `inputResponses` và `requestState`.

```rust
let list = call.elicit().choose("price_list_id", "Which price list?", options).await?; // trả Answer hoặc dừng lượt
```

Luật cứng (kernel ép, không để module tự giữ):

- **Không bao giờ thay `x-approval`.** Tool `requires_approval_for_agent` vẫn trả `approval_required` và trỏ về REST; câu trả
  lời elicitation không được tính là người duyệt (agent có thể tự trả lời thay người).
- **Không bao giờ hỏi mật khẩu, token, mã OTP hay số thẻ.** API chỉ có `choose` (enum), `confirm` (boolean), `text` ngắn;
  không có `format: password`, không có elicitation dạng URL; tên trường khớp mẫu bí mật (`password`, `secret`, `token`,
  `otp`, `card`, `cvv`, `pin`) bị `Tool::problems` từ chối lúc khởi động.
- **`requestState`**: `axs1.` + base64url(payload) + HMAC-SHA256 bằng khoá dẫn xuất riêng của kernel
  `SecretKey::derive_for(tenant, "autoxide/mcp/request-state/v1")`. Payload: tenant, người gọi (user + credential), tên
  tool, SHA-256 của `arguments` dạng chuẩn hoá, các câu trả lời trước, `nonce`, `expires_at` = phát hành + **10 phút**. Kiểm:
  chữ ký, hạn, tenant, người gọi, tool, hash tham số — sai bất kỳ thì `-32602` "requestState is invalid or expired, call the
  tool again without it". **Dùng một lần**: `nonce` ghi vào bảng token một lần sẵn có của kernel (`OneTimeTokens`, chạy trên
  cả hai database) khi được nhận; gửi lại cùng `requestState` bị từ chối.
- Client không khai khả năng elicitation: không hỏi, trả lỗi tool `<module>.choice_required` kèm danh sách lựa chọn trong
  `params` để agent gọi lại với tham số đủ — không bao giờ treo.
- Câu hỏi và nhãn lựa chọn là dữ liệu của khách (tên bảng giá…): đánh dấu như mọi dữ liệu người dùng (mục prompt injection
  của G4 mục 3), không trộn vào câu chỉ dẫn.

## 5. Gate mới (phần A)

- `mcp-output-schema`: tool trong `docs/mcp-tools.json` thiếu `output_schema` hay `title` → đỏ, trừ tên trong
  `xtask/mcp-output-baseline.txt` (312 dòng lúc đầu, **chỉ giảm**: gate đỏ khi baseline có tên không còn thiếu, để xoá dòng).
- `mcp-param-description`: mọi thuộc tính của `input_schema` có `description` (kiểu enum Rust tự có `enum`); thiếu → đỏ,
  trừ `tool.param` trong `xtask/mcp-param-baseline.txt` (143 dòng lúc đầu, chỉ giảm). PR (1) điền hết mô tả cho các tool UT-5
  nêu tên.
- Mỗi gate một dòng cách sửa trong `xtask/src/hints.rs` ("chuyển tool sang `Tool::typed` và thêm `.title(…)`"; "thêm `///`
  cho trường của kiểu đầu vào").

## 6. Đánh phiên bản tool (G13, phần E)

Tên và schema tool là hợp đồng như REST (quy tắc chung §13 hợp đồng kernel):

- **Không phá vỡ:** thêm tool; thêm tham số tuỳ chọn; thêm trường vào kết quả; thêm giá trị enum ở enum khai mở; nới quyền
  (thêm quyền thay thế); thêm annotation còn thiếu.
- **Phá vỡ:** bỏ/đổi tên tool hay tham số; đổi kiểu; thêm tham số bắt buộc; bỏ trường kết quả; đổi nghĩa; siết quyền;
  `readOnlyHint` từ `true` sang `false`; `idempotentHint` từ `true` sang `false`; thêm `destructiveHint` hay `openWorldHint`.
  Làm tool mới hậu tố `_v2` (tên tool chỉ cho `[a-z0-9_]`, nên không dùng `.v2` như event), chạy song song; tool cũ khai
  `Deprecation::tool(since, sunset, "<module>.<tên>_v2")` (đã có, D-240), gỡ ở bản lớn.
- `api-diff` (sau này `contract-diff`) so `docs/mcp-tools.json` với bản phát hành gần nhất theo luật trên; tool của module hạng
  thử nghiệm: báo, không chặn. Mỗi tool có `_meta["io.autoxide/stability"]` để client biết hạng.

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

1. `Tool::typed` + `ToolCall` + `title`/annotations + `outputSchema` qua egress + kiểm hợp đồng kết quả tool trong testkit + hai
   gate có baseline + `hints.rs` + `catalog.list_price_lists` + mô tả tham số; chuyển `notes` và các tool UT-5 nêu tên làm mẫu.
2. Chuyển 300 tool còn lại sang `Tool::typed` theo cụm module (baseline giảm dần) — **giao được cho phiên khác**.
3. `tools/list` chia trang; progress; huỷ; task (`kernel_mcp_tasks`, job, `tasks/*`), test cả hai database.
4. Elicitation (MRTR, `requestState`), luật G13 trong `api-diff`, phần còn lại của UT-5.

Tài liệu `docs/guides/mcp.md` + `.vi.md` cập nhật trong PR nào đổi hành vi người gọi thấy.
