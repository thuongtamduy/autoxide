# Đề bài: tính năng AI — tầng kết nối AI, quyền cho agent, gợi ý, dịch, tìm theo ý nghĩa

File này là đề bài giao cho một phiên Claude Code. Cách dùng: mở phiên mới ở gốc repo, trên
nhánh dựa trên `main` mới nhất, và gửi:

> Làm bước 18 theo `docs/prompts/ai.md`.

Quyết định nền: **D-109** (chủ dự án quyết mở tính năng AI và chọn hướng; agent quyết cách làm).
Khảo sát, số đo, nguồn và phân tích: [`docs/research/ai.md`](../research/ai.md) — đọc mục
"Khuyến nghị" trước khi làm. Đề bài mô tả **cái cần có và các ràng buộc**; tên hàm, cách chia
file, chi tiết migration do agent quyết. Chỗ nào đề bài mâu thuẫn với code thật hoặc với
`CLAUDE.md` thì `CLAUDE.md` thắng — dừng lại, ghi quyết định mới, không lặng lẽ làm khác.

Đề bài viết khi `main` ở commit `27ad7c5` (quyết định mới nhất lúc đó là D-108, rồi D-109 của
chính đề bài này). Nếu đề bài `forms-newsletter-comments.md` được làm trước, số bước và số quyết
định có thể đã đổi — dùng số kế tiếp lúc làm và đọc code thật trước.

---

## 0. Điều kiện trước

1. Bước 17 (`content`) đã `[x]` trong `docs/ROADMAP.md`.
2. Đọc trước: `CLAUDE.md`; `docs/DECISIONS.md` từ D-080 (nhất là D-083, D-084, D-085, D-090,
   D-093, D-095, D-102, D-104, D-105, D-106, D-109); `docs/CONTENT.vi.md`; `docs/MEDIA.vi.md`;
   `docs/MODULE_GUIDE.vi.md`.
3. Đọc code thật trước khi thiết kế: `crates/kernel/src/mail.rs` + `modules/mail` (khuôn mẫu
   "trait ở kernel, module công bố cài đặt"); `crates/kernel/src/storage.rs`; `job/` (`enqueue`
   nhận `ConnectionTrait` — xếp job được trong transaction); `http/rate_limit.rs` (GCRA);
   `access.rs` (`Principal::api_key_id`); `mcp.rs`; `modules/content/src/body.rs` (bộ kiểm thân
   bài), `entries.rs`, `permissions.rs`, `tools.rs`; `modules/media/src/api.rs`, `variants.rs`.
4. API thư viện: skill `check-library-api` trước khi dùng thứ chưa dùng trong repo (phần
   `reqwest` cho JSON và multipart không cần; phần mới của SeaORM cho cột nhị phân).
5. **Không đoán API của nhà cung cấp AI** — cùng tinh thần với "không đoán API thư viện". Dạng
   request và response ghi trong `docs/research/ai.md` (có nguồn); chỗ nào chưa chắc thì đọc tài
   liệu chính thức của nhà cung cấp trước khi viết, và để test (máy chủ giả) giữ đúng dạng đó.

## 1. Phạm vi

**Có** (mỗi mục một phần ở dưới, làm theo thứ tự này):

- A. Giới hạn quyền cho AI agent (không cần nhà cung cấp AI nào).
- B. Tầng kết nối AI: trait trong kernel, module `ai`, hai adapter sinh chữ (`anthropic`,
  `openai`), một client embedding, hạn mức, metrics, lệnh `autoxide ai check`.
- C. Kiểm SEO không dùng AI, và gợi ý SEO bằng AI (`content`).
- D. Gợi ý alt text bằng AI (`media`).
- E. Dịch bài bằng AI vi ↔ en, mọi cặp trong `AUTOXIDE_CONTENT_LOCALES` (`content`).
- F. Tìm theo ý nghĩa, kết hợp với tìm theo chữ (`content`).

**Không:** sinh bài viết từ đầu ở máy chủ (agent bên ngoài tự viết rồi gọi API); chatbot hay
hỏi-đáp trên nội dung (RAG); sinh ảnh; dịch term, menu, tác giả; bảng thuật ngữ không dịch;
quy trình duyệt nhiều bước; hạn mức theo từng người hay từng API key (chỉ có hạn mức chung mỗi
ngày và rate limiting theo API key); streaming; Batch API; prompt caching; chạy mô hình ngay
trong binary (ONNX, candle); `pgvector`, `sqlite-vec` hay bất cứ thứ gì chỉ một database có;
adapter riêng cho Gemini hay cho Responses API của OpenAI (chỉ thêm khi đo được Chat Completions
thiếu thứ cần — D-109). Khái niệm của `content` (bài, bản dịch, SEO) không vào kernel; khái niệm
của nhà cung cấp (tên tham số, `stop_reason`…) không ra khỏi `modules/ai`.

**Không thêm thư viện.** HTTP bằng `reqwest` sẵn có, JSON bằng `serde_json`, base64 bằng
`base64`, kiểm schema tự viết trên phần JSON Schema mình dùng (vài chục dòng), cosine tự viết.
Thấy buộc phải thêm thì ghi quyết định và `STACK.md`.

---

## A. Giới hạn quyền cho AI agent

Hôm nay đã có: API key mang một phần permission và có hạn; audit log ghi `actor_api_key_id`;
`content.write` tách khỏi `content.publish`; `If-Match` bắt buộc. Thêm:

1. **`content.delete`** — tách quyền xoá mục ra khỏi `content.publish`. `content.publish` giữ
   đăng, gỡ, lưu trữ, chuyển trang, hẹn giờ. Role `admin` có đủ như cũ; role tự tạo có
   `content.publish` mất quyền xoá — chấp nhận vì module mới, chưa có client cũ (như lý lẽ ở
   D-102); ghi trong `CHANGELOG.md` mục "Changed".
2. **`content.write_own`** — tạo nháp, sửa, tạo bản dịch, khôi phục revision, xin duyệt, xin dịch,
   xin gợi ý, **chỉ trên mục do chính bên gọi tạo**. "Chính bên gọi" = cùng `created_by` **và**
   cùng `created_by_api_key_id` (cột mới, `NULL` khi tạo bằng phiên đăng nhập). Hai agent dùng
   hai key của cùng một người không sửa được bài của nhau. Mục không phải của mình: lỗi 403
   `content.not_owner`. Đọc nháp vẫn cần `content.read`.
3. **`content.publish_own`** — đăng và gỡ **chỉ mục của chính mình** (định nghĩa như trên); không
   lưu trữ, không xoá, không chuyển trang, không hẹn giờ. Đăng trang con vẫn chịu mọi luật cây
   trang sẵn có.
4. **Xin duyệt:** `POST /entries/{id}/request-review` (`content.write` hoặc `write_own` trên mục
   của mình; `If-Match` bắt buộc) đặt `review_requested_at` + `review_requested_by` trên nháp và
   phát event `content.review_requested` (website hay hệ thống thông báo nhận qua webhook).
   `DELETE /entries/{id}/request-review` rút lại. Đăng mục thì tự xoá cờ. Không có trạng thái
   "đã duyệt / từ chối" — quy trình duyệt nhiều bước nằm ngoài phạm vi (bước 17).
5. **Đánh dấu nội dung do AI tạo và việc người duyệt:** cột `ai_generated` (bool) trên nháp, bản
   công khai và revision. Máy chủ tự đặt `true` khi bản dịch do máy chủ dịch (phần E); bên gọi đặt
   được qua `POST`/`PATCH` (tài liệu khuyên agent luôn đặt). Khi **một người** (request bằng phiên
   đăng nhập, không phải API key) đăng một mục `ai_generated` và gửi `"ai_reviewed": true`, bản
   công khai và revision `published` ghi `ai_reviewed_by` + `ai_reviewed_at`; API key gửi trường
   này thì bị từ chối (`content.ai_review_requires_human`). Endpoint công khai trả `ai_generated`
   và `ai_reviewed` (bool, không lộ ai duyệt); website tự quyết cách hiện nhãn. Thêm `created_via`
   (`session` / `api_key`) ở API quản trị. Tài liệu nói rõ: các trường này giúp người vận hành làm
   đúng nghĩa vụ minh bạch (EU AI Act Điều 50, áp dụng từ 2/8/2026 — ngoại lệ "đã qua người duyệt
   và có người chịu trách nhiệm biên tập" được hiểu hẹp, bấm duyệt cho có không tính; xem
   `docs/research/ai.md`); Autoxide chỉ ghi lại sự thật, không đưa ra lời khuyên pháp lý.
6. **Rate limiting theo API key:** `AUTOXIDE_HTTP_RATE_LIMIT_API_KEY_PER_MINUTE` (mặc định
   `120`) và burst tương ứng, dùng lại GCRA của kernel, đếm theo `api_key_id` sau khi xác thực.
   Áp cho mọi request đến bằng API key (REST và MCP), cộng thêm vào giới hạn theo địa chỉ sẵn có.
   Vượt: 429 `rate_limited` kèm `Retry-After`.
7. Tool MCP: `content.request_review`. Tool sửa và đăng hiện có kiểm `write_own` / `publish_own`
   như REST.
8. Tài liệu (`docs/CONTENT*.md`, mục "AI agent"): công thức cấp key cho agent — "agent viết,
   người duyệt" (`content.read` + `content.write_own`), "agent tự đăng bài của mình"
   (thêm `content.publish_own`); khuyên đặt `expires_at` ngắn.

Test: mọi tổ hợp permission trên mục của mình / của người khác / của key khác cùng người; xoá
cần `content.delete`; rate limiting theo key (hai key tách biệt, một key vượt thì key kia vẫn
chạy); `ai_generated` đi qua đăng, revision, khôi phục.

---

## B. Tầng kết nối AI

### B.1 Trait trong kernel — `crates/kernel/src/ai.rs`

Trung lập với nhà cung cấp và với nghiệp vụ. Phác thảo (tên cuối cùng do agent chọn):

- `Prompt { purpose: &'static str, instructions: String, input: Vec<Part>, output_schema:
  serde_json::Value, effort: Effort, max_output_tokens: u32 }`;
  `Part::Text(String) | Part::Image { media_type, bytes }`; `Effort::Low | Medium | High`.
  `purpose` là `<module>.<việc>` (vd `content.translate`) — dùng cho hạn mức, log, metrics.
- `Generation { output: serde_json::Value, usage: Usage, model: String }`.
- `trait TextModel: Send + Sync { async fn generate(&self, prompt: Prompt) -> Result<Generation, Error>; fn capabilities(&self) -> Capabilities; }`
  — `output` **luôn** đã khớp `output_schema` (adapter tự kiểm lại, không tin nhà cung cấp);
  `Capabilities` cho biết có nhận ảnh không, có đang bật không.
- `trait Embedder: Send + Sync { fn model_id(&self) -> &str; fn dimensions(&self) -> usize; async fn embed(&self, texts: &[String], kind: EmbedKind) -> Result<Vec<Vec<f32>>, Error>; }`
  — `EmbedKind::Document | Query` (một số mô hình cần biết, adapter nào không cần thì bỏ qua).
- Hàm dùng chung cho vector: mã hoá `Vec<f32>` ↔ bytes (little-endian, cố định), cosine, top-k.
  Trung lập, đặt trong kernel (vd `ai::vector`), có test.

Comment giải thích vì sao trait cần `Send + Sync` và vì sao dùng `async_trait` (đi qua `dyn`),
như các trait sẵn có.

### B.2 Module `ai` — `modules/ai`

Như `modules/mail`: crate riêng, đăng ký trong `crates/app/src/modules.rs` **trước** `media` và
`content`, công bố `dyn TextModel` và `dyn Embedder` bằng `context.provide` trong `start`.
`content` và `media` lấy bằng `context.require`. Module `ai` luôn được đăng ký; khi tắt, nó công
bố bản "đã tắt" — mọi lời gọi trả `ai.disabled` (503) — nên module dùng nó không phải rẽ nhánh.

Bên trong, một lớp bọc chung (không phải mỗi adapter tự làm) lo: kiểm hạn mức trước khi gọi,
gọi adapter, kiểm `output` theo schema, cộng token thật vào hạn mức, ghi log và metrics.

**Bảng `ai_usage`** (một dòng mỗi `day` UTC × `purpose`): `input_tokens`, `output_tokens`,
`requests`. Cộng bằng upsert chạy được trên cả hai database (thử cả hai). Trước mỗi lần gọi:
tổng token hôm nay ≥ `AUTOXIDE_AI_DAILY_TOKEN_LIMIT` thì trả `ai.quota_exceeded` (429). Nhiều
lệnh cùng lúc có thể vượt một chút — tài liệu ghi rõ; không khoá toàn cục. Job dọn dòng cũ hơn
`AUTOXIDE_AI_USAGE_RETENTION_DAYS` (mặc định 90).

**Endpoint:** `GET /api/v1/ai/status` (permission mới `ai.view`): đang bật không, nhà cung cấp,
mô hình, có nhận ảnh không, embedding bật không, token hôm nay / hạn mức. Không bao giờ trả key
hay URL có chứa key. Giao diện dùng để ẩn hay hiện nút AI. Tool MCP `ai.status` tương ứng.

**Lệnh CLI:** `autoxide ai check` gửi một request rất nhỏ (cả sinh chữ có schema lẫn embedding
nếu bật) và in kết quả bằng lời thường: được / sai key / sai URL / mô hình không nhận schema / mô
hình không nhận ảnh. Không chạy trong gate.

### B.3 Adapter `anthropic`

Messages API gốc (`POST {base}/v1/messages`). Chi tiết đã kiểm và nguồn ở `docs/research/ai.md`:

- Header `x-api-key`, `anthropic-version` ghim làm hằng số (`2026-10-21` lúc viết đề bài), nâng
  có chủ đích kèm test.
- Kết quả theo schema bằng `output_config.format = {"type": "json_schema", "schema": …}`;
  `output_config.effort` lấy từ `Effort`. Không gửi `temperature`/`top_p`/`top_k` (phiên bản
  API trên từ chối).
- Ảnh: content block `image` nguồn base64.
- Fallback phía máy chủ khi bị bộ lọc an toàn từ chối: beta `server-side-fallback-2026-07-01`,
  `fallbacks: "default"`, bật/tắt bằng `AUTOXIDE_AI_ANTHROPIC_FALLBACK` (mặc định bật).
- Đọc `stop_reason` **trước** nội dung: `end_turn` → đọc; `refusal` → `ai.refused`;
  `max_tokens` → `ai.output_truncated`. Lỗi HTTP: bảng ở B.5.

### B.4 Adapter `openai` (Chat Completions)

`POST {base}/chat/completions` — một adapter cho OpenAI, Gemini (endpoint tương thích OpenAI),
Ollama, vLLM, LM Studio, OpenRouter… đổi bằng `AUTOXIDE_AI_BASE_URL`.

- `Authorization: Bearer <key>`; key để trống thì không gửi header (Ollama tại máy).
- Kết quả theo schema bằng `response_format = {"type": "json_schema", "json_schema": {"name",
  "schema", "strict": true}}`. Schema của mọi `purpose` viết trong **phần chung** mà cả hai
  adapter nhận: chỉ `type` (`object`, `array`, `string`, `integer`, `boolean`), `properties`,
  `required` (liệt kê **mọi** trường), `additionalProperties: false` trên mọi object, `items`,
  `enum`, `description`; không `allOf`/`anyOf`/`not`/`if`/`$ref`/`default`. Độ dài và khoảng số
  (`maxLength`, `maxItems`…) nhà cung cấp **không** bảo đảm (Anthropic ghi rõ không ép) — bộ kiểm
  của mình ép. Một hàm kiểm schema tự viết dùng chung cho cả hai adapter, đúng phần con này. `AUTOXIDE_AI_OPENAI_SCHEMA_MODE`: `strict` (mặc định) /
  `json_object` (chỉ yêu cầu JSON, cho máy chủ không nhận schema) — cả hai đều tự kiểm kết quả.
- Ảnh: phần `image_url` với data URL base64.
- Giới hạn token ra: `max_completion_tokens` (OpenAI coi `max_tokens` là deprecated và mô hình
  suy luận không nhận); máy chủ tương thích cũ chỉ hiểu `max_tokens` thì đặt
  `AUTOXIDE_AI_OPENAI_MAX_TOKENS_PARAM=max_tokens`.
- `reasoning_effort` chỉ gửi khi `AUTOXIDE_AI_OPENAI_SEND_EFFORT=true`: mô hình không suy luận trả
  400 `unsupported_parameter` chứ không bỏ qua; Gemini tự đổi sang mức thinking của nó.
- Không dùng tool calling (chỉ cần kết quả theo schema), nên việc OpenAI đẩy tool calling của mô
  hình mới sang Responses API không ảnh hưởng.
- Gemini và Ollama được báo là có lúc **bỏ qua schema hoặc tham số lạ mà không báo lỗi** — vì vậy
  bước tự kiểm và thử lại một lần là bắt buộc, không phải phòng xa.
- `finish_reason`: `stop` → đọc; `length` → `ai.output_truncated`; `content_filter` hoặc
  `message.refusal` có giá trị → `ai.refused`.
- `AUTOXIDE_AI_MODEL` **bắt buộc** khi dùng adapter này (không điền sẵn tên — mau cũ, mỗi dịch vụ
  một kiểu): thiếu thì không khởi động được, báo đúng biến cần đặt.

### B.5 Lỗi chung

| Tình huống | Mã | HTTP | Retry |
|---|---|---|---|
| Chưa bật | `ai.disabled` | 503 | không |
| Sai key, không có quyền (401, 403) | `ai.misconfigured` | 503 | không; log `warn` cho người vận hành |
| Hết hạn mức ngày | `ai.quota_exceeded` | 429 | không (job: lùi tới ngày sau) |
| 408, 409, 429, 5xx, 529, lỗi mạng, timeout | `ai.provider_unavailable` | 503 | có — job queue có backoff; tôn trọng `retry-after` |
| Bị từ chối nội dung | `ai.refused` | 422 | không |
| Kết quả bị cắt | `ai.output_truncated` | 422 | không — bên gọi chia nhỏ hơn |
| Kết quả sai schema (sau một lần thử lại) | `ai.output_invalid` | 503 | không |
| Mô hình không nhận ảnh | `ai.unsupported_input` | 422 | không |
| 400 khác từ nhà cung cấp | `internal` | 500 | không; log `error` kèm `purpose` |

Dùng `ErrorKind` sẵn có (`Unavailable`, `TooManyRequests`, `Validation`…), không thêm kind trừ
khi thật cần. Mọi mã có bản dịch trong `modules/ai/locales/vi.json`.

### B.6 Embedding

Client riêng theo dạng `POST {base}/embeddings` (`{"model", "input": [...]}` →
`data[].embedding`), cấu hình riêng (`AUTOXIDE_EMBEDDING_*`), độc lập với phần sinh chữ — sinh
chữ bằng Claude và embedding bằng Ollama là cấu hình hợp lệ. Dạng này dùng được với OpenAI,
Gemini (endpoint tương thích), Ollama, vLLM và Voyage AI (Anthropic không có API embedding, chỉ
giới thiệu Voyage). Tham số `dimensions` chỉ gửi khi `AUTOXIDE_EMBEDDING_SEND_DIMENSIONS=true`
(OpenAI `text-embedding-3-*` nhận; Voyage dùng tên khác nên không gửi). Tài liệu khuyên mô hình
đa ngôn ngữ khoảng 1.024 chiều (vd `bge-m3` chạy bằng Ollama) — mô hình 3.072 chiều tốn gấp ba bộ
nhớ cho cùng số đoạn. Gửi theo lô (tối đa
`AUTOXIDE_EMBEDDING_BATCH_SIZE`, mặc định 64). Kiểm số chiều trả về đúng
`AUTOXIDE_EMBEDDING_DIMENSIONS`; sai thì `ai.output_invalid`. Token embedding cộng vào hạn mức
chung với `purpose` riêng.

### B.7 Log, metrics, dữ liệu rời máy chủ

- Log: `purpose`, mô hình, token vào / ra, thời gian, kết quả. **Không bao giờ** ghi prompt, nội
  dung bài, ảnh, key.
- Metrics: `ai_requests_total{purpose, outcome}`, `ai_tokens_total{purpose, direction}`,
  `ai_request_duration_seconds{purpose}`. `purpose` là tập cố định do code khai.
- Gọi AI **không bao giờ** nằm trong transaction ghi. Việc có ghi (dịch, lập index) chạy trong job;
  việc chỉ trả gợi ý (SEO, alt) chạy ngay trong request với timeout.
- Prompt injection: luật cho mô hình chỉ nằm ở `instructions`; nội dung người dùng chỉ nằm ở
  `input`, bọc trong dấu phân cách rõ; kết quả bị ép theo schema và kiểm lại; mọi thân bài sinh
  ra chạy lại bộ kiểm của `content` và chỉ lưu thành nháp.
- Tài liệu ghi rõ dữ liệu nào gửi cho nhà cung cấp ở từng tính năng.

### B.8 Test

Gate không gọi mạng, không cần key:

- Bản giả `TextModel` / `Embedder` trong `autoxide-testkit` (trả kết quả định sẵn theo `purpose`,
  embedding tất định từ chữ — vd băm thành vector) để mọi test của `content`, `media` chạy trên
  cả hai database.
- Mỗi adapter test với một máy chủ HTTP giả chạy trong test (`axum` sẵn có): kiểm header, thân
  request (schema, ảnh, effort, fallback), và mọi nhánh: thành công, `refusal`, bị cắt, JSON
  sai schema rồi thử lại, 401, 429 có `retry-after`, 529, timeout.
- Hạn mức: đồng thời N lời gọi cộng token không mất (cả hai database); vượt hạn mức trả đúng mã.
- Cấu hình: `openai` thiếu `AUTOXIDE_AI_MODEL` không khởi động; `none` thì mọi tính năng trả
  `ai.disabled`.

---

## C. Kiểm SEO và gợi ý SEO (`content`)

1. **Kiểm không dùng AI** — `GET /entries/{id}/seo-check` (`content.read`), tool MCP
   `content.seo_check`: danh sách phát hiện, mỗi cái có `code` ổn định, mức (`error`/`warning`),
   vị trí và câu tiếng Anh nói phải sửa gì. Ít nhất: thiếu hoặc quá dài `meta_title` (> 60 ký
   tự) / `meta_description` (> 160); thiếu ảnh chia sẻ; ảnh trong bài hoặc ảnh đại diện thiếu alt;
   heading nhảy cấp; tiêu đề trùng với mục khác cùng ngôn ngữ; `noindex` trên bài đã đăng (cảnh
   báo); bản dịch thiếu so với các ngôn ngữ đã cấu hình. Chạy được khi AI tắt.
2. **Gợi ý bằng AI** — `POST /entries/{id}/seo-suggestions` (`content.write`, hoặc `write_own`
   trên mục của mình), tool MCP `content.suggest_seo`: trả `meta_title`, `meta_description`,
   `excerpt`, và tối đa 5 thẻ **chọn trong các term đang có** của ngôn ngữ đó (trả ID; mô hình
   không được bịa thẻ mới — kiểm lại). Không ghi gì; giao diện cho người xem rồi áp bằng `PATCH`.
   Gửi cho mô hình: tiêu đề, tóm tắt, chữ thuần của bài (cắt theo giới hạn có ghi rõ), danh sách
   term. Effort `Low`.

## D. Gợi ý alt text (`media`)

`POST /api/v1/media/items/{id}/alt-suggestions` (`media.write`), tool MCP `media.suggest_alt`,
nhận `locale` và `context` tuỳ chọn (vd tiêu đề bài, tối đa 500 ký tự). Gửi **ảnh thu nhỏ** lớn
nhất không quá 1280 px đã có (D-093); ảnh gốc đã nhỏ hơn thì gửi ảnh gốc; chưa có bản thu nhỏ phù
hợp (đang làm, hay `AUTOXIDE_MEDIA_IMAGE_WIDTHS=off`) thì đổi cỡ tạm trong bộ nhớ bằng `image`
trong `spawn_blocking`, cùng giới hạn pixel của D-093, không lưu. Ảnh `private` chỉ gửi khi
`AUTOXIDE_AI_SEND_PRIVATE_MEDIA=true`, không thì `media.ai_private_not_allowed` (403). Chỉ ảnh
raster mà thư viện `image` đọc được. Trả `alt` (≤ 250 ký tự) và `decorative` (bool — mô hình cho
rằng ảnh chỉ để trang trí, nên để alt trống). Không ghi gì. Mô hình không nhận ảnh:
`ai.unsupported_input`. Effort `Low`.

Ghi chú cho tài liệu: `media.alt` chỉ có một ngôn ngữ; alt theo từng bản dịch nằm ở thuộc tính
`alt` của node ảnh trong thân bài.

## E. Dịch bài bằng AI (`content`)

`POST /entries/{id}/translate` (`content.write`, hoặc `write_own`; `Idempotency-Key` được nhận)
với `{"locale": "vi"}` → `202 Accepted` + `Location` tới một **yêu cầu dịch**. Bảng
`content_translation_requests`: `source_entry_id`, `source_revision_id` (revision ghim lúc yêu
cầu — tạo một revision nếu nháp đã đổi so với revision gần nhất), `target_locale`,
`target_entry_id` (có sẵn hoặc `NULL`), `target_updated_at_at_request`, `status`
(`queued`/`running`/`succeeded`/`failed`), `error_code`, `created_by`, `created_by_api_key_id`,
thời gian. `GET /translation-requests/{id}` và `GET /entries/{id}/translation-requests` để theo
dõi. Mỗi (`source`, `locale`) tối đa một yêu cầu đang chờ (`content.translation_in_progress`).
Event `content.translation_succeeded` / `content.translation_failed`. Job xếp trong cùng
transaction với dòng yêu cầu.

**Job** — ba bước, không gọi AI trong transaction:

1. Đọc revision nguồn. **Tách** thân bài thành "mẩu": mỗi khối chứa chữ (đoạn, heading, ô bảng,
   mục danh sách…) là một mẩu; mark trong khối thành thẻ giữ chỗ có số (`<b1>…</b1>`,
   `<a2>…</a2>` — link chỉ giữ số, không giữ URL); `alt`, `caption` của ảnh là mẩu riêng;
   `codeBlock` và `embed` không dịch. Thêm `title`, `excerpt`, `meta_title`, `meta_description`,
   `featured_media_alt`.
2. Gọi `TextModel` theo lô mẩu sao cho mỗi lần kết quả dưới khoảng 8.000 token
   (`AUTOXIDE_AI_TRANSLATE_BATCH_TOKENS`); schema bắt trả đúng danh sách `id`. **Kiểm:** đủ mọi
   `id`, không thừa; mỗi mẩu có đúng tập thẻ như bản gốc, lồng đúng; không rỗng khi bản gốc không
   rỗng. Sai thì thử lại lô đó một lần, vẫn sai thì yêu cầu `failed` với `ai.output_invalid`.
3. **Ráp lại** vào cấu trúc gốc (link, ID ảnh, thuộc tính lấy từ bản gốc, không lấy từ mô hình),
   chạy bộ kiểm thân bài, rồi trong một transaction: nếu chưa có bản dịch thì tạo (đúng luật
   trang cha của bản dịch sẵn có, slug sinh từ tiêu đề đã dịch); nếu có rồi thì **chỉ ghi khi
   `updated_at` của bản dịch vẫn bằng lúc yêu cầu** — người đã sửa thì yêu cầu `failed` với
   `content.translation_conflict`, không ghi đè. Kết quả luôn là **nháp**, `ai_generated = true`,
   ghi `translated_from_revision_id`. Không bao giờ tự đăng.

Bản dịch có `translated_from_revision_id` và bài nguồn đã có revision mới hơn thì API quản trị
trả `translation_outdated: true`.

Prompt: dịch giữ nguyên thẻ, không thêm bớt nội dung, giữ tên riêng, giọng văn tự nhiên của ngôn
ngữ đích; tên ngôn ngữ nguồn / đích lấy từ mã BCP 47. Effort `Medium`.

## F. Tìm theo ý nghĩa (`content`)

1. **Bảng `content_embeddings`**: `entry_id`, `scope` (`draft` / `published`), `locale`,
   `chunk_index`, `content_hash` (SHA-256 của chữ của đoạn), `model`, `dimensions`, `vector`
   (nhị phân — `BLOB` / `BYTEA` qua SeaORM, mã hoá bằng hàm của kernel), `updated_at`. Bảng
   `content_embedding_state`: một dòng mỗi (`scope`, `locale`) với `version` tăng mỗi lần đổi.
2. **Lập index:** cắt chữ thuần của mục (tiêu đề + tóm tắt + thân) thành đoạn khoảng 300–500
   từ, chồng nhau một ít. Lưu nháp, đăng, gỡ, xoá, khôi phục → xếp job (trong cùng transaction)
   lập lại index của mục đó; đoạn có `content_hash` không đổi thì không gọi lại embedding. Job
   `content.reindex_embeddings` lập lại tất cả khi `model` / `dimensions` cấu hình khác dữ liệu
   đang có; lệnh CLI tương ứng. Embedding tắt thì không lập gì, không lỗi.
3. **Tìm:** mỗi tiến trình giữ vector của từng (`scope`, `locale`) trong bộ nhớ, mỗi lần tìm đọc
   `version` (một dòng) để biết có phải nạp lại không — chạy đúng khi nhiều máy chủ. Cosine
   duyệt hết (brute force), điểm của mục = điểm đoạn khớp nhất. Gộp với kết quả tìm theo chữ sẵn
   có (`search::contains`, D-104) bằng Reciprocal Rank Fusion (`k = 60`). Cache embedding của câu
   tìm (LRU nhỏ trong bộ nhớ).
4. **Endpoint:** `GET /search?q=&locale=&kind=&limit=` (`content.read`, cả nháp) và
   `GET /public/search?q=&locale=&kind=&limit=` (công khai, chỉ bản đã đăng) — **tắt mặc định**
   (`AUTOXIDE_CONTENT_PUBLIC_SEMANTIC_SEARCH=false`; tắt thì `404` mã
   `content.semantic_search_disabled`, để agent gọi biết rõ lý do). `q` tối đa 200 ký tự;
   `limit` tối đa 50; trả `Page` không có trang kế (top-k), mỗi kết quả kèm `score` và đoạn khớp.
   Embedding tắt: chỉ tìm theo chữ, trả `mode: "text"` để client biết. Endpoint công khai có rate
   limiting riêng chặt hơn theo địa chỉ client (`AUTOXIDE_CONTENT_PUBLIC_SEARCH_PER_MINUTE`, dùng
   lại GCRA), vì mỗi lần tìm tốn một lời gọi embedding. Tool MCP `content.search`.
5. **Đo:** thêm vào bộ đo hiệu năng (`docs/BENCHMARKS.md`) thời gian tìm với 10.000 và 100.000
   đoạn × 1.024 chiều, và bộ nhớ dùng. Ghi số đo. Vượt ngưỡng chấp nhận được (ghi trong quyết
   định) thì đó là lý do cho một bước sau (index ANN hoặc nhánh `pgvector` sau trait), **không**
   làm trong bước này.

## G. Cấu hình

Khai bằng `Setting`, sinh lại `.env.example`. Không cấu hình gì thì AI tắt và mọi thứ khác chạy
như cũ.

| Biến | Mặc định |
|---|---|
| `AUTOXIDE_AI_PROVIDER` | `none` (`none` / `anthropic` / `openai`) |
| `AUTOXIDE_AI_API_KEY` | trống (secret) |
| `AUTOXIDE_AI_BASE_URL` | `https://api.anthropic.com` khi `anthropic`, `https://api.openai.com/v1` khi `openai` |
| `AUTOXIDE_AI_MODEL` | `claude-opus-5-5` khi `anthropic`; **bắt buộc** khi `openai` |
| `AUTOXIDE_AI_TIMEOUT_SECONDS` | `600` |
| `AUTOXIDE_AI_DAILY_TOKEN_LIMIT` | `2000000` |
| `AUTOXIDE_AI_USAGE_RETENTION_DAYS` | `90` |
| `AUTOXIDE_AI_SEND_PRIVATE_MEDIA` | `false` |
| `AUTOXIDE_AI_ANTHROPIC_FALLBACK` | `true` |
| `AUTOXIDE_AI_OPENAI_SCHEMA_MODE` | `strict` |
| `AUTOXIDE_AI_OPENAI_SEND_EFFORT` | `false` |
| `AUTOXIDE_AI_OPENAI_MAX_TOKENS_PARAM` | `max_completion_tokens` (`max_completion_tokens` / `max_tokens`) |
| `AUTOXIDE_AI_TRANSLATE_BATCH_TOKENS` | `8000` |
| `AUTOXIDE_EMBEDDING_PROVIDER` | `none` (`none` / `openai`) |
| `AUTOXIDE_EMBEDDING_BASE_URL`, `…_API_KEY`, `…_MODEL`, `…_DIMENSIONS` | trống; `MODEL` và `DIMENSIONS` bắt buộc khi bật |
| `AUTOXIDE_EMBEDDING_BATCH_SIZE` | `64` |
| `AUTOXIDE_EMBEDDING_SEND_DIMENSIONS` | `false` |
| `AUTOXIDE_CONTENT_PUBLIC_SEMANTIC_SEARCH` | `false` |
| `AUTOXIDE_CONTENT_PUBLIC_SEARCH_PER_MINUTE` | `30` |
| `AUTOXIDE_HTTP_RATE_LIMIT_API_KEY_PER_MINUTE` | `120` |

Kiểm lúc khởi động, báo mọi lỗi một lần: thiếu key khi nhà cung cấp cần; URL sai dạng; `openai`
thiếu model; embedding bật mà thiếu model / số chiều.

## H. Mã lỗi mới

- `ai.`: `disabled`, `misconfigured`, `quota_exceeded`, `provider_unavailable`, `refused`,
  `output_truncated`, `output_invalid`, `unsupported_input`.
- `content.`: `not_owner`, `ai_review_requires_human`, `translation_in_progress`, `translation_conflict`,
  `translation_same_locale`, `semantic_search_disabled`.
- `media.`: `ai_private_not_allowed`, `ai_unsupported_image`.

Mỗi mã có bản dịch tiếng Việt trong `locales/vi.json` của crate.

## I. Test bắt buộc (ngoài test đã nêu ở từng phần)

Mọi test chạm database chạy trên **cả SQLite và Postgres** qua `TestDatabase::run`.

- Đồng thời: hai yêu cầu dịch cùng (`source`, `locale`) — chỉ một được nhận; job dịch chạy lại
  hai lần — không tạo hai bản dịch; người sửa bản dịch trong lúc job chạy — job thất bại
  `translation_conflict`, bản người sửa còn nguyên; hai job lập index cùng một mục — không trùng
  dòng; N lời gọi AI cùng lúc — hạn mức cộng đúng.
- Dịch: bài có đủ mọi node và mark của danh sách cho phép, link, ảnh có alt/caption, bảng, code
  (không bị dịch); mô hình giả trả thiếu `id`, thừa `id`, sai thẻ, rỗng — mỗi trường hợp bị chặn,
  không lưu gì; trang con khi trang cha chưa có bản dịch — lỗi sẵn có.
- Gợi ý SEO: mô hình giả trả ID term không tồn tại — bị bỏ, không lỗi cả câu trả lời.
- Alt: ảnh riêng tư khi chưa bật; ảnh không phải raster; mô hình không nhận ảnh.
- Tìm theo ý nghĩa: mục nháp không lộ ra endpoint công khai; đổi `model` cấu hình làm job lập lại
  chạy; `version` đổi ở "máy chủ" khác (database chung) làm cache nạp lại; embedding tắt thì
  `mode: "text"`.
- Tool MCP mới qua một MCP client thật, kể cả kiểm permission `write_own` / `publish_own`.
- OpenAPI: mọi endpoint mới có `///` và `description` cho từng response (test sẵn có kiểm).

## J. Cách làm

- Chạy skill `split-work` trước khi chia việc. Phần A, B nên làm tuần tự bằng một agent (đụng
  kernel và đăng ký module). Sau khi B xong và gate xanh, C+D (`media` và phần SEO của `content`)
  và E+F (`content`) có thể chia, nhưng E và F cùng sửa `modules/content` và cùng thêm migration
  — **không** chạy song song trong cùng module (luật của `split-work`). Thứ tự gợi ý, mỗi phần
  gate xanh rồi commit: A → B → C → D → E → F → tài liệu.
- Ghi quyết định mới (skill `record-decision`) khi chọn khác đề bài hoặc gặp chỗ đề bài không
  nói; ngưỡng thời gian tìm chấp nhận được (F.5) phải ghi.
- `STACK.md`: thêm dòng "AI" vào bảng "Dịch vụ của core" (trait + module `ai`, hai adapter,
  không thêm thư viện).
- Tài liệu cho người dùng: `docs/AI.md` + `docs/AI.vi.md` (cùng nội dung) — bật AI với Claude,
  OpenAI, Gemini, Ollama (ví dụ cấu hình từng cái); embedding; hạn mức và chi phí ước tính; dữ
  liệu nào gửi đi ở tính năng nào; `ai_generated` và nghĩa vụ minh bạch; công thức cấp key cho
  agent. Cập nhật `docs/CONTENT*.md`, `docs/MEDIA*.md`, README (mục tính năng) cả hai bản.
- `docs/prompts/frontend-handoff.md`: thêm mục cho giao diện — gọi `/ai/status` để ẩn hiện nút,
  luồng gợi ý → người xem → `PATCH`, theo dõi yêu cầu dịch, hiện `translation_outdated`, nhãn
  `ai_generated`.

## K. Xong khi

- Mọi mục bước 18 trong `docs/ROADMAP.md` đánh `[x]`; `./gate.sh` xanh trên cả hai database.
- `docs/openapi.json`, `.env.example`, bản dịch đã sinh lại và khớp.
- Báo cáo cho chủ dự án bằng tiếng Việt, lời thường: đã làm gì, quyết định nào đã ghi, số đo
  tìm kiếm, việc họ cần làm — **thử thật bằng `autoxide ai check` với key của họ** (agent không
  có key; không bao giờ đưa key vào git hay vào log), việc để lại cho repo giao diện.
