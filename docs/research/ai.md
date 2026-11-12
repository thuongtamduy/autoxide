# Tính năng AI cho Autoxide: khảo sát, số đo và phân tích

Bản khảo sát làm nền cho D-109 và đề bài [`docs/prompts/ai.md`](../prompts/ai.md). Ngày khảo sát:
12/11/2026.

**Độ tin của nguồn.** Lúc khảo sát, môi trường không mở được trực tiếp nhiều trang tài liệu chính
thức (OpenAI, Google AI, Ollama, Sanity, Drupal, WordPress, OWASP, DeepL). Phần lớn dữ kiện dưới
đây lấy từ đoạn trích mà công cụ tìm kiếm trả về của chính các trang đó, kèm đường dẫn. Đoạn trích
đáng tin khi trả lời "có hay không có", kém tin hơn khi là danh sách chi tiết (từ khoá schema, giới
hạn số). Chỗ đánh **(chưa kiểm)** là chưa xác nhận được. Agent làm bước 18 phải đọc lại tài liệu
chính thức trước khi viết phần gọi API (đề bài, mục 0.5).

Riêng thông tin về thư viện Rust thì chắc hơn: agent khảo sát đã tải mã nguồn của crate về đọc.

---

## Kết luận chính

1. **Mọi CMS đã xem đều đưa kết quả AI về nháp hoặc gợi ý; người bấm đăng.** Sanity ghi kết quả
   Agent Actions vào bản nháp, không bao giờ đăng. Plugin AI của WordPress đưa gợi ý để người bấm
   nhận. Contentful đặt bước người duyệt trước khi đăng. Autoxide làm giống vậy.
2. **Mã nguồn mở thì để người cài tự mang key, chọn nhà cung cấp.** Drupal AI có khoảng 48 nhà
   cung cấp sau một lớp trừu tượng. WordPress 7.0 có AI Client trong core nhưng không kèm nhà
   cung cấp nào; Anthropic, Google, OpenAI là plugin riêng. Strapi bán AI theo credit, không cho
   tự mang key — mô hình đó không hợp với Autoxide.
3. **Hai dạng API là đủ.** Chat Completions của OpenAI vẫn được hỗ trợ (OpenAI chỉ khuyên dùng
   Responses API), và Gemini, Ollama, vLLM, LM Studio đều có endpoint theo dạng này. Anthropic
   giữ adapter gốc. Không crate Rust nào gọn hơn tự viết.
4. **Dịch phải tách chữ khỏi cấu trúc.** Module AI Translate của Drupal gửi nguyên HTML cho mô
   hình và có hàng loạt lỗi mở: thẻ bị bỏ hay sửa, thêm `<p>`, tham chiếu bị xoá, bản người dịch
   bị ghi đè. Ngành dịch máy đã có lời giải từ lâu: DeepL `tag_handling`, mã inline của XLIFF.
5. **Tìm theo ý nghĩa trên cả hai database chỉ có một đường an toàn:** vector trong bảng thường,
   tính trong Rust. `sqlite-vec` là extension C, và với sqlx 0.9 nạp extension là API `unsafe`.
   `pgvector` chỉ Postgres có. Duyệt hết 100.000 vector mất cỡ vài chục mili giây trên một nhân.
6. **Quyền là lớp phòng thủ chính, không phải prompt.** OWASP nói chưa rõ có cách nào chặn hẳn
   prompt injection. Cách giảm thiệt hại là cấp quyền tối thiểu, bắt người duyệt việc có tác động
   lớn ("đăng"), và ghi log.
7. **Điều 50 của EU AI Act áp dụng từ 2/8/2026.** Văn bản do AI tạo, đăng để thông tin cho công
   chúng về việc công ích, phải được công bố là do AI tạo. Được miễn nếu đã qua người duyệt **và**
   có người chịu trách nhiệm biên tập. Ủy ban châu Âu hiểu ngoại lệ này hẹp: bấm duyệt cho có
   không tính. Autoxide nên ghi lại sự thật (AI có tham gia không, ai duyệt) để người vận hành tự
   làm đúng luật.

---

## 1. Các CMS làm AI thế nào

| Hệ thống | Có gì | Kết quả AI đi đâu | Nhà cung cấp | Dịch nội dung có định dạng |
|---|---|---|---|---|
| **Drupal AI** | Tìm theo vector (qua Search API, nhiều vector DB), AI Translate, alt text, gợi ý nội dung, tự điền trường | Alt text: người xác nhận trước khi lưu. Translate: tạo bản dịch thật | ~48, tự mang key, có Ollama và LM Studio | Gửi nguyên HTML từng trường → nhiều lỗi mở về thẻ |
| **WordPress** (AI Building Blocks, 7.0) | AI Client trong core, Abilities API, MCP Adapter; plugin "AI": gợi ý tiêu đề và tóm tắt, alt text (theo cây quyết định alt của W3C), ghi chú SEO | Gợi ý, người bấm nhận | Không kèm sẵn; Anthropic, Google, OpenAI là plugin; tự mang key | — |
| **Strapi 5** | Tự dịch mọi ngôn ngữ khi bản chính đổi; alt text và chú thích lúc tải lên; MCP (tắt mặc định) | Ghi thẳng | AI của Strapi, trả bằng credit | (chưa kiểm) |
| **Sanity** | Agent Actions: Generate, Transform, Translate, Prompt, Patch | **Luôn vào nháp**; `noWrite` chỉ trả kết quả. Translate có `styleGuide`, `protectedPhrases` | AI Assist gửi tới OpenAI | Generate không tạo được annotation hay mark tuỳ biến (chưa kiểm phần Translate) |
| **Contentful** | AI Actions: dịch theo trường hoặc cả mục, hàng loạt | Có bước người duyệt trước khi đăng | Tự mang mô hình: OpenAI, Bedrock, Azure, hay **endpoint tương thích OpenAI** bất kỳ | Theo cả trường |
| **Payload** | Plugin MCP chính thức (API key, quyền đọc / ghi theo collection); AI là plugin cộng đồng | — | — | — |
| **Directus** | MCP tắt mặc định, chạy với quyền của người dùng nối vào; khuyên tạo người dùng riêng cho AI | — | — | — |
| **Ghost** | Không thấy tính năng AI chính thức | — | — | — |

Nguồn:
- Drupal: [nhà cung cấp](https://new.drupal.org/ai/about-drupal-ai/providers),
  [AI Translate](https://project.pages.drupalcode.org/ai/1.2.x/modules/ai_translate/), các lỗi mở
  [3585525](https://git.drupalcode.org/project/ai_translate/-/work_items/3585525),
  [3555741](https://git.drupalcode.org/project/ai_translate/-/work_items/3555741),
  [3585548](https://git.drupalcode.org/project/ai_translate/-/work_items/3585548).
- WordPress: [AI Building Blocks](https://make.wordpress.org/ai/2025/07/17/ai-building-blocks/),
  [MCP Adapter](https://make.wordpress.org/ai/2025/07/17/mcp-adapter/),
  [plugin AI](https://wordpress.org/plugins/ai/).
- Strapi: [i18n](https://docs.strapi.io/cms/features/internationalization.md),
  [MCP](https://strapi.io/blog/the-strapi-mcp-server-is-now-ga).
- Sanity: [Agent Actions](https://www.sanity.io/docs/agent-actions/introduction),
  [Translate](https://reference.sanity.io/_sanity/client/TranslateRequestBase).
- Contentful: [tự mang mô hình](https://www.contentful.com/help/ai-automations/ai-actions/bring-your-own-model/).
- Directus: [MCP security](https://directus.io/docs/guides/ai/mcp/security).

**Rút ra cho Autoxide:**
- **Gợi ý mà không ghi** (`noWrite` của Sanity, plugin AI của WordPress) là mặc định đúng cho
  SEO và alt.
- **Dịch vào nháp**, không bao giờ tự đăng, và không ghi đè bản người đã sửa — đúng chỗ Drupal
  đang vá.
- **Agent dùng chung mô hình quyền với người**, với người dùng hay token riêng quyền hẹp
  (Directus, Strapi). Autoxide đã có API key giới hạn theo permission; chỉ thiếu quyền "chỉ bài
  của mình" và tách quyền xoá.
- **Không tự dịch mỗi khi bản chính đổi** (Strapi): tốn tiền ngầm, ghi đè công người dịch.
  Autoxide dịch khi được yêu cầu, và báo bản dịch đã cũ (`translation_outdated`).

## 2. Nhà cung cấp và dạng API

### Anthropic — adapter `anthropic`

- **Kết quả theo schema:** `output_config.format = {"type": "json_schema", "schema": …}`, chính
  thức, không cần header beta. Mọi object cần `additionalProperties: false`. Giới hạn độ dài chuỗi
  và khoảng số **không được ép**. Không hỗ trợ `allOf` kèm `$ref` hay `$ref` ra ngoài.
  [Structured outputs](https://platform.claude.com/docs/en/build-with-claude/structured-outputs)
- **`stop_reason`:** `end_turn`, `max_tokens`, `stop_sequence`, `tool_use`, `pause_turn`,
  `refusal` (kèm `stop_details`). Bị từ chối vẫn trả HTTP 200, và nội dung có thể không đúng
  schema; bị cắt ở `max_tokens` thì JSON có thể dở dang. Vì vậy phải đọc `stop_reason` trước.
  [Handling stop reasons](https://platform.claude.com/docs/en/api/handling-stop-reasons)
- **Lỗi:** 429 `rate_limit_error` kèm `retry-after`; 529 `overloaded_error` (máy chủ quá tải,
  lùi dần rồi thử lại). [Errors](https://platform.claude.com/docs/en/api/errors)
- **Không có API embedding**; tài liệu giới thiệu Voyage AI.
  [Embeddings](https://platform.claude.com/docs/en/build-with-claude/embeddings)
- **Không có SDK Rust chính thức** (chỉ có Python, TypeScript, Java, Go, Ruby, C#, PHP). Các crate
  `anthropic-*` trên crates.io đều tự nhận không chính thức.
  [Client SDKs](https://platform.claude.com/docs/en/api/client-sdks)
- **Đã kiểm qua tài liệu đi kèm Claude Code:** header `anthropic-version: 2026-10-21`. Phiên bản
  này từ chối `temperature` / `top_p` / `top_k`.

  Fallback phía máy chủ khi bị từ chối dùng beta `server-side-fallback-2026-07-01` với
  `fallbacks: "default"`. Với `claude-opus-5-5`, effort mặc định là `medium` và không tắt được
  thinking.

  Giá ngày 3/11/2026, mỗi triệu token vào / ra: Opus 5.5 là 4 / 20 USD, Sonnet 5.5 là 2 / 10 USD,
  Haiku 5.5 là 0,10 / 0,50 USD.

### OpenAI và các máy chủ tương thích — adapter `openai`

**OpenAI**
- **Chat Completions vẫn được hỗ trợ.** OpenAI viết "While the Chat Completions API is still
  supported, you'll get improved model intelligence and performance by using Responses". Thứ bị
  ngừng là Assistants API (26/8/2026).
  [Reasoning guide](https://developers.openai.com/api/docs/guides/reasoning)
- **Điểm cần biết:** trang của mô hình mới (GPT-5.6) khuyên dùng Responses cho tool calling.
  Autoxide **không dùng tool calling**, chỉ cần kết quả theo schema, nên không bị ảnh hưởng.
- **Kết quả theo schema:** `response_format: {"type": "json_schema", "json_schema": {"name",
  "schema", "strict": true}}`.
  - Chế độ chặt: mọi object `additionalProperties: false`, mọi thuộc tính nằm trong `required`;
    trường tuỳ chọn thì thành bắt buộc với kiểu cho phép `null`.
  - Được báo là không hỗ trợ: `allOf`, `not`, `if/then/else`, `default` (danh sách đầy đủ:
    chưa kiểm).
  - [Structured outputs](https://developers.openai.com/api/docs/guides/structured-outputs.md)
- **Ảnh:** phần `{"type": "image_url", "image_url": {"url": "data:image/jpeg;base64,…"}}`.
- **`reasoning_effort`:** mô hình không suy luận trả **400 `unsupported_parameter`**, không bỏ
  qua → adapter mặc định không gửi.
- **Giới hạn token ra:** `max_tokens` đã deprecated và mô hình o-series không nhận; dùng
  `max_completion_tokens`.
- **`finish_reason`:** `stop`, `length`, `tool_calls`, `content_filter`. Tin nhắn có trường
  `refusal`.
- **Lỗi:** 429 (kể cả mã `slow_down` khi tăng tải quá nhanh) thì tôn trọng `Retry-After`; 429 do
  hết quota không nên thử lại; 500 và 503 là lỗi tạm thời.
  [Error codes](https://developers.openai.com/api/docs/guides/error-codes)

**Gemini, endpoint tương thích OpenAI**
- Base URL `https://generativelanguage.googleapis.com/v1beta/openai/`, key Gemini gửi dạng
  Bearer.
- Có `chat/completions` và `embeddings`.
- `json_schema` chạy được khi gọi thường, nhưng có báo cáo bị bỏ qua. Tham số không hỗ trợ bị
  **bỏ qua mà không báo**. Ảnh: chưa kiểm.
- `reasoning_effort` được đổi sang thinking budget hay thinking level tuỳ mô hình.
- [Gemini OpenAI compatibility](https://ai.google.dev/gemini-api/docs/openai),
  [báo cáo schema bị bỏ qua](https://discuss.ai.google.dev/t/structured-output-not-working-via-the-openai-compatible-layer/108341)

**Ollama**
- `http://localhost:11434/v1/`; key bị bỏ qua.
- Có `/v1/chat/completions` (JSON, ảnh base64, tool) và `/v1/embeddings`; lớp tương thích còn ghi
  là thử nghiệm.
- `json_schema` có trong tài liệu, nhưng một bộ kiểm bên thứ ba báo là lùi về chế độ JSON thường.
- Ollama có cả endpoint tương thích Anthropic (`/v1/messages`) — ghi lại để biết, không dùng.
- [OpenAI compatibility](https://docs.ollama.com/api/openai-compatibility),
  [structured outputs](https://docs.ollama.com/capabilities/structured-outputs)

**vLLM** nhận `json_schema` mặc định
([docs](https://docs.vllm.ai/en/stable/features/structured_outputs/)). **LM Studio** cũng nhận,
nhưng mô hình dưới 7B không đáng tin
([docs](https://lmstudio.ai/docs/developer/openai-compat/structured-output)).

**Hệ quả cho thiết kế:**
- Không tin nhà cung cấp nào ép đúng schema. Tự kiểm mọi kết quả, sai thì thử lại một lần.
- Viết schema trong phần chung nhỏ nhất: `type`, `properties`, `required` đủ mọi trường,
  `additionalProperties: false`, `items`, `enum`, `description`.
- Độ dài và khoảng số do code mình ép.

### Embedding

- **Một dạng chung, nhiều dịch vụ:** `POST /embeddings` với `{model, input}` →
  `data[].embedding`, theo dạng OpenAI.
  - **OpenAI:** chỉ `text-embedding-3-*` nhận tham số `dimensions`. Mặc định 1.536 chiều (small)
    và 3.072 chiều (large).
  - **Voyage AI:** cùng dạng trả về, nhưng thêm `input_type` (`query` / `document`) và gọi số
    chiều là `output_dimension`; voyage-4 có 256 / 512 / 1.024 (mặc định) / 2.048 chiều.
    [Voyage embeddings](https://docs.voyageai.com/reference/embeddings-api)
  - **Gemini:** `gemini-embedding-001`, 3.072 chiều, khuyên rút còn 768 hoặc 1.536.
    [Vertex embeddings](https://docs.cloud.google.com/vertex-ai/generative-ai/docs/embeddings/get-text-embeddings)
- **Mô hình hiểu tiếng Việt tốt:**
  - `bge-m3` (1.024 chiều, mở, chạy được bằng Ollama hay vLLM, có bản tinh chỉnh cho tiếng Việt);
  - `multilingual-e5-large` (1.024 chiều, dẫn đầu bộ đo khu vực Đông Nam Á SEA-BED,
    [arXiv 2508.12243](https://arxiv.org/pdf/2508.12243));
  - `gemini-embedding-001`; `text-embedding-3-large`.
  - Có bộ đo riêng cho tiếng Việt (VN-MTEB); điểm từng mô hình: chưa kiểm.

### Thư viện Rust (đã đọc mã nguồn)

| Crate | Bản | License | Nhận xét |
|---|---|---|---|
| `genai` | 0.7.4 | MIT / Apache-2.0 | Dùng chung `reqwest` 0.13, có schema cho cả Anthropic và OpenAI. Nhưng kéo `tokio` có `test-util` và bật `serde_json/preserve_order` cho **cả workspace**; thêm `derive_more`, `strum`, `serde_with`, `regex` |
| `rig-core` | 0.46.0 | MIT | Cả một framework (agent, RAG); MSRV 1.97.1 rất mới; feature `reqwest` kéo `tokio` `full` |
| `async-openai` | 0.43.0 | MIT | Chỉ OpenAI, không có Anthropic |

Phần Autoxide cần chỉ là hai dạng request / response nhỏ và ổn định. Tự viết trên `reqwest` và
`serde_json` sẵn có gọn hơn, và không đổi feature của cả workspace.

## 3. Dịch nội dung có định dạng

**Tiền lệ:**
- **DeepL** (`tag_handling=xml`): tách chữ, dịch, ráp lại vào cấu trúc. `ignore_tags` giữ phần
  không dịch; `non_splitting_tags` giữ một câu đi qua nhiều thẻ.
  [DeepL XML handling](https://developers.deepl.com/docs/xml-and-html-handling/xml)
- **Google Translation** (`mimeType=text/html`): giữ thẻ, dịch cả `alt`; `translate="no"` để chừa
  ra. [Google Translate](https://docs.cloud.google.com/translate/troubleshooting)
- **XLIFF 2.1:** mã inline `ph` (đứng một mình) và `pc` (cặp mở đóng); định dạng gốc nằm ngoài
  phần chữ cần dịch.
  [XLIFF 2.1](https://docs.oasis-open.org/xliff/xliff-core/v2.1/xliff-core-v2.1.html)

**Mô hình ngôn ngữ và thẻ:**
- Mô hình hay làm mất, bịa hoặc hỏng thẻ, nhưng giữ được thẻ thì đặt chỗ khá đúng.
  [MT Summit 2023](https://aclanthology.org/2023.mtsummit-research.13)
- Cho ví dụ có thẻ trong prompt giúp giảm lỗi.
  [AMTA 2024](https://aclanthology.org/2024.amta-research.8.pdf)

**Cách áp cho thân bài ProseMirror của `content`** (đã đưa vào đề bài, mục E):
1. Mỗi khối chứa chữ là một mẩu. Đoạn văn, heading, mục danh sách, ô bảng, `alt` và `caption` của
   ảnh đều là mẩu.
2. Mỗi đoạn có mark thành **cặp thẻ có số** (`<b1>…</b1>`). Mẩu nguyên câu, không chia theo từng
   mảnh chữ, vì vị trí bổ ngữ trong tiếng Việt và tiếng Anh ngược nhau: "Xin **chào** bạn" phải
   dịch như một câu. Link chỉ giữ số, URL không bao giờ tới mô hình.
3. `codeBlock`, `embed` không dịch.
4. Kiểm kết quả: đủ `id`, đúng tập thẻ, lồng đúng. Sai thì thử lại một lần, rồi báo lỗi.
5. Ráp lại bằng thuộc tính gốc, chạy lại bộ kiểm thân bài.

Bảng thuật ngữ không dịch (như `protectedPhrases` của Sanity) và hướng dẫn giọng văn
(`styleGuide`) để bước sau.

## 4. Tìm theo ý nghĩa trên cả hai database

**Hai cách bị loại:**
- **`sqlite-vec`:** bản 0.1.7 (3/2026), còn trước v1 ("expect breaking changes"), là extension C.
  Với sqlx 0.9, `SqliteConnectOptions::extension()` đã thành `unsafe` và nằm sau feature
  `sqlite-load-extension`, mà repo cấm `unsafe`. Bảng `vec0` của nó cũng chỉ duyệt hết.
  [sqlx discussion 4271](https://github.com/launchbadge/sqlx/discussions/4271),
  [sqlite-vec](https://docs.rs/crate/sqlite-vec/0.1.7)
- **`pgvector`:** bản 0.8 có index HNSW / IVFFlat lọc tốt hơn, index tối đa 2.000 chiều (`vector`).
  Nhưng chỉ Postgres có, lại phải cài thêm vào máy chủ database.
  [pgvector 0.8.0](https://www.postgresql.org/about/news/pgvector-0.8.0-released-2952/)

**Duyệt hết (brute force)** là cách chuẩn khi không có index. Số đo người khác công bố:

| Phép đo | Kết quả | Nguồn |
|---|---|---|
| `sqlite-vec`, 10.000 vector, k = 10 | 3,55 ms | [bài khảo sát](https://marcobambini.substack.com/p/the-state-of-vector-search-in-sqlite) (nguồn thứ cấp) |
| `sqlite-vec`, 100.000 × 384, M1 Pro | ~68 ms | như trên |
| Crate Rust `bfes`, 100.000 × 512, một luồng, Ryzen 5950X | 20–30 ms | [bfes](https://lib.rs/crates/bfes) |
| Một phép cosine 768 chiều có SIMD | ~165 ns | [Neumann](https://shadylukin.github.io/Neumann/benchmarks/vector-engine.html) |

Ước tính cho Autoxide (chưa đo, bước 18 phải đo):

| Số đoạn × chiều | Bộ nhớ (f32) | Thời gian một nhân |
|---|---|---|
| 10.000 × 1.024 | ~41 MB | vài ms |
| 100.000 × 1.024 | ~410 MB | vài chục ms (giới hạn bởi băng thông bộ nhớ) |

Chuẩn hoá vector lúc ghi thì cosine thành tích vô hướng. Lưu `f16` hay `int8` giảm bộ nhớ 2–4
lần — để sau, khi đo thấy cần.

**Tìm kết hợp:** Reciprocal Rank Fusion, `RRF(d) = Σ 1 / (k + hạng_r(d))`, `k = 60`, hạng tính
từ 1 (Cormack, Clarke, Büttcher, SIGIR 2009 —
[PDF](https://plg.uwaterloo.ca/~gvcormac/cormacksigir09-rrf.pdf)). Gộp hạng từ tìm theo ý nghĩa
với hạng từ tìm theo chữ không dấu sẵn có (D-104), không cần chuẩn hoá điểm của hai bên.

**Nhiều máy chủ:** vector giữ trong bộ nhớ của từng tiến trình. Một dòng `version` trong database
cho biết khi nào phải nạp lại. Đọc một dòng mỗi lần tìm rất rẻ, và cách này chạy như nhau trên
hai database.

## 5. Bảo mật: prompt injection và quyền quá rộng

OWASP Top 10 cho ứng dụng LLM (2025):

- **LLM01 Prompt Injection.** Chưa rõ có cách chặn hẳn. Biện pháp: cấp quyền tối thiểu cho mô
  hình; người duyệt các thao tác có đặc quyền; tách và đánh dấu nội dung bên ngoài là không tin
  được; lọc đầu ra.
  [LLM01](https://genai.owasp.org/llmrisk/llm01-prompt-injection/)
- **LLM06 Excessive Agency** (quyền quá rộng). Chỉ cấp công cụ cần thiết với quyền hẹp; kiểm
  quyền ở hệ thống phía sau, không tin mô hình; người duyệt việc có tác động lớn. Ví dụ của chính
  OWASP là bước duyệt trong chức năng "đăng bài".
  [LLM06](https://owasp.org/www-project-top-10-for-large-language-model-applications/2_0_vulns/LLM06_ExcessiveAgency.html)

**Áp cho Autoxide:**
- Thân bài, tiêu đề và ảnh đều là đầu vào không tin được khi đưa vào prompt dịch, SEO hay alt.
  Một câu cài sẵn trong bài có thể khiến mô hình đổi link hay chèn nội dung. Phòng thủ:
  - mô hình không bao giờ thấy URL;
  - kết quả ép theo schema và tự kiểm;
  - thân bài chạy lại bộ kiểm;
  - kết quả chỉ thành nháp hay gợi ý.
- Agent bên ngoài gọi qua MCP thì chính nó có thể bị lừa. Phòng thủ: `content.write_own`
  (không đụng bài người khác), `content.publish_own` (không xoá, không chuyển trang), quyền xoá
  tách riêng, xin duyệt, rate limiting theo API key, audit log có `actor_api_key_id`.

## 6. EU AI Act Điều 50, IPTC / C2PA

**Điều 50(4) — nghĩa vụ của bên triển khai**
- **Phạm vi:** văn bản do AI tạo hay sửa, "đăng với mục đích thông tin cho công chúng về việc công
  ích", phải được công bố là do AI tạo.
  [Điều 50](https://artificialintelligenceact.eu/transparency-rules-article-50/)
- **Ngoại lệ:** cần **cả hai** điều kiện — nội dung đã qua người duyệt hay kiểm soát biên tập,
  **và** có người (thể nhân hay pháp nhân) chịu trách nhiệm biên tập.
- **Ủy ban châu Âu hiểu hẹp:** bấm duyệt cho có, ký qua loa hay chỉ soát chính tả thì không tính.
  [Greenberg Traurig](https://www.gtlaw.com/en/insights/2026/6/deepfakes-chatbots-ai-generated-text-european-commission-details-transparency-obligations-under-the-ai-act).
  Hướng dẫn này đã chính thức hay còn là bản nháp: chưa kiểm.
- **Thời hạn:** áp dụng từ 2/8/2026. Thoả thuận Digital Omnibus (7/5/2026) chỉ cho nghĩa vụ đánh
  dấu đọc được bằng máy (khoản 2) thêm thời gian tới 2/12/2026, và chỉ với hệ thống đã có mặt
  trước 2/8/2026.
  [William Fry](https://www.williamfry.com/knowledge/eu-ai-act-omnibus-deal-reached-postponed-deadlines-watermarking-compromise-and-the-nudificiation-prohibition/)

**Autoxide làm gì:** ghi `ai_generated`; khi một **người** đăng và xác nhận đã duyệt thì ghi
`ai_reviewed_by` / `ai_reviewed_at`; API công khai trả `ai_generated` và `ai_reviewed` để website
hiện nhãn. Autoxide không quyết thay người vận hành nội dung nào thuộc phạm vi, và không đưa ra
lời khuyên pháp lý.

**Ảnh do AI tạo:** IPTC Digital Source Type có `trainedAlgorithmicMedia` (tạo hoàn toàn) và
`compositeWithTrainedAlgorithmicMedia` (sửa một phần), ghi trong XMP hay C2PA. Bước 18 không sinh
ảnh nên chưa cần; ghi lại cho khi nào `media` nhận ảnh do AI tạo.
[IPTC](https://iptc.org/news/new-digital-source-type-term-added-to-support-inpainting-outpainting-in-generative-ai/)

## 7. Chi phí ước tính

Một bài 1.500 chữ, tính cả phần thẻ và JSON, khoảng 5.000 token vào và 5.000 token ra mỗi lần
dịch. Theo bảng giá Anthropic ngày 3/11/2026:

| Mô hình | Dịch một bài | Gợi ý SEO (~3.000 vào, ~300 ra) |
|---|---|---|
| `claude-opus-5-5` | ~0,12 USD | ~0,018 USD |
| `claude-sonnet-5-5` | ~0,06 USD | ~0,009 USD |
| `claude-haiku-5-5` | ~0,003 USD | ~0,0005 USD |

Opus 5.5 còn tốn thêm token suy nghĩ tuỳ effort. Bảng chỉ để hình dung cỡ chi phí; số thật đọc
ở metrics `ai_tokens_total`. Ollama chạy tại máy thì không tốn tiền API.

Hạn mức mặc định 2 triệu token mỗi ngày tương đương khoảng 200 lần dịch bài cỡ trên.

---

## Khuyến nghị cho Autoxide

1. **Thứ tự:** quyền cho agent (không cần AI) → tầng kết nối → kiểm SEO không AI và gợi ý SEO →
   alt text → dịch → tìm theo ý nghĩa. Mỗi phần dùng được ngay khi xong.
2. **Tầng kết nối:**
   - trait `TextModel` / `Embedder` ở kernel, module hạ tầng `ai` công bố cài đặt;
   - hai adapter sinh chữ (`anthropic` gốc; `openai` theo Chat Completions) và một client
     embedding theo dạng `/embeddings`;
   - mặc định tắt; không thêm thư viện;
   - kết quả luôn theo schema trong phần chung nhỏ nhất, luôn tự kiểm.
3. **Không gọi AI trong transaction ghi.** Việc có ghi chạy bằng job; gợi ý chạy ngay trong
   request. Kết quả chỉ thành nháp hay gợi ý.
4. **Dịch theo mẩu có thẻ giữ chỗ**, không gửi nguyên JSON; không ghi đè bản người đã sửa; báo
   bản dịch đã cũ.
5. **Tìm theo ý nghĩa:** vector nhị phân trong bảng thường, duyệt hết trong bộ nhớ, gộp với tìm
   theo chữ bằng RRF. Đo với 10.000 và 100.000 đoạn; index ANN hay `pgvector` chỉ là bước sau khi
   số đo đòi.
6. **Quyền:**
   - `content.delete` tách riêng; `content.write_own`, `content.publish_own`;
   - xin duyệt; `ai_generated` / `ai_reviewed`; rate limiting theo API key;
   - tài liệu có công thức cấp key cho agent.
7. **Kiểm thật trước khi viết:** đọc lại tài liệu chính thức của từng nhà cung cấp (khảo sát này
   có nhiều chỗ chỉ từ đoạn trích). Lệnh `autoxide ai check` để người cài thử với key và máy chủ
   của họ.
