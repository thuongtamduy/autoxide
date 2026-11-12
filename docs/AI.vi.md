# Tính năng AI

*English: [AI.md](AI.md)*

Autoxide có thể dùng một mô hình AI — do bạn chọn và trả tiền — để giúp người biên tập và AI
agent:

| Tính năng | Ở đâu | Cần |
| --------- | ----- | --- |
| Kiểm SEO (không AI) | `GET /api/v1/content/entries/{id}/seo-check` | không cần gì |
| Gợi ý SEO | `POST /api/v1/content/entries/{id}/seo-suggestions` | mô hình sinh chữ |
| Gợi ý alt text | `POST /api/v1/media/{id}/alt-suggestions` | mô hình sinh chữ đọc được ảnh |
| Dịch bài hay trang | `POST /api/v1/content/entries/{id}/translate` | mô hình sinh chữ |
| Tìm theo ý nghĩa | `GET /api/v1/content/search`, `/public/search` | embedding |
| Giới hạn cho AI agent | permission `content.write_own`, `content.publish_own` | không cần gì |

**Mọi thứ tắt cho tới khi bạn bật.** Không nội dung nào rời máy chủ trước khi bạn cấu hình
một nhà cung cấp. Gợi ý không bao giờ tự lưu: một người (hay một agent) xem rồi lưu bằng
`PATCH` như thường. Bản dịch được ghi thành **bản nháp** và không bao giờ tự đăng.

Mọi endpoint và mã lỗi có trong [`openapi.json`](openapi.json) (và ở `/api/docs` khi
`AUTOXIDE_HTTP_API_DOCS=true`). Các tính năng này cũng là tool MCP cho AI agent: `ai.status`,
`content.seo_check`, `content.suggest_seo`, `content.translate`,
`content.get_translation_request`, `content.search`, `media.suggest_alt`.

## Bật sinh chữ

Đặt nhà cung cấp, key và (với `openai`) mô hình, rồi kiểm cấu hình:

```bash
autoxide ai check
```

Lệnh gửi một request rất nhỏ và nói bằng lời thường có chạy hay không (tốn vài token).

**Claude (Anthropic):**

```bash
AUTOXIDE_AI_PROVIDER=anthropic
AUTOXIDE_AI_API_KEY=sk-ant-…            # key của bạn
# AUTOXIDE_AI_MODEL=claude-opus-5-5     # mặc định
```

Khi bộ lọc an toàn của Claude từ chối một request, Anthropic chạy lại trên mô hình khác ngay
trong cùng lần gọi (`AUTOXIDE_AI_ANTHROPIC_FALLBACK=true`, mặc định). Đặt `false` cho mô hình
không hỗ trợ, như Claude Haiku.

**OpenAI:**

```bash
AUTOXIDE_AI_PROVIDER=openai
AUTOXIDE_AI_API_KEY=sk-…
AUTOXIDE_AI_MODEL=<tên một mô hình OpenAI>
# AUTOXIDE_AI_OPENAI_SEND_EFFORT=true   # chỉ cho mô hình suy luận
```

**Google Gemini** (qua endpoint tương thích OpenAI):

```bash
AUTOXIDE_AI_PROVIDER=openai
AUTOXIDE_AI_BASE_URL=https://generativelanguage.googleapis.com/v1beta/openai
AUTOXIDE_AI_API_KEY=<API key Gemini của bạn>
AUTOXIDE_AI_MODEL=<tên một mô hình Gemini>
```

**Ollama, chạy ngay trên máy chủ của bạn** — không gì rời máy:

```bash
AUTOXIDE_AI_PROVIDER=openai
AUTOXIDE_AI_BASE_URL=http://localhost:11434/v1
AUTOXIDE_AI_MODEL=<mô hình bạn đã tải>
AUTOXIDE_AI_IMAGES=false                # trừ khi mô hình đọc được ảnh
# AUTOXIDE_AI_OPENAI_SCHEMA_MODE=json_object   # nếu máy chủ bỏ qua JSON Schema
# AUTOXIDE_AI_OPENAI_MAX_TOKENS_PARAM=max_tokens
```

vLLM, LM Studio và OpenRouter dùng cùng cách: `openai` cộng `AUTOXIDE_AI_BASE_URL` của chúng.

**Câu trả lời được kiểm thế nào.** Mỗi request yêu cầu mô hình trả JSON theo một schema, và
máy chủ kiểm lại mọi câu trả lời — có máy chủ bỏ qua schema mà không báo. Câu trả lời sai được
thử lại một lần, rồi báo lỗi (`ai.output_invalid`). Mô hình nhỏ chạy tại máy sai nhiều hơn;
hãy dùng mô hình đủ mạnh để dịch.

## Bật embedding (tìm theo ý nghĩa)

Embedding tách khỏi sinh chữ: bạn có thể sinh chữ bằng Claude và tính embedding bằng Ollama.

```bash
AUTOXIDE_EMBEDDING_PROVIDER=openai          # dạng /embeddings
AUTOXIDE_EMBEDDING_BASE_URL=http://localhost:11434/v1
AUTOXIDE_EMBEDDING_MODEL=bge-m3
AUTOXIDE_EMBEDDING_DIMENSIONS=1024
```

Cùng dạng này có ở OpenAI (`https://api.openai.com/v1`; đặt
`AUTOXIDE_EMBEDDING_SEND_DIMENSIONS=true` để rút ngắn vector của `text-embedding-3-*`), Gemini
và Voyage AI (Anthropic không có API embedding, chỉ giới thiệu Voyage). Hãy chọn mô hình **đa
ngôn ngữ** khoảng 1024 chiều: `bge-m3` và `multilingual-e5-large` hiểu tốt tiếng Việt; mô hình
3072 chiều tốn gấp ba bộ nhớ.

Khi đã bật, mỗi thay đổi của một mục cập nhật index tìm kiếm của nó ở nền, và một job mỗi giờ
lập index cho phần còn thiếu — sau khi bạn bật embedding, hay đổi mô hình hoặc số chiều (khi đó
cả index được lập lại). Tìm kiếm giữ vector của một ngôn ngữ trong bộ nhớ: khoảng 41 MB cho
10.000 đoạn ở 1024 chiều (một bài 1.500 chữ là 5–6 đoạn); thời gian tìm đã đo ở
[`BENCHMARKS.md`](BENCHMARKS.md).

Tìm kiếm gộp ý nghĩa và chữ (không phân biệt dấu và hoa thường): mỗi kết quả nói khớp bằng
cách nào (`matched_by`) và kèm đoạn khớp nhất. Embedding tắt thì chỉ tìm theo chữ.
`/api/v1/content/public/search` (cho người đọc không đăng nhập) tắt, trừ khi
`AUTOXIDE_CONTENT_PUBLIC_SEMANTIC_SEARCH=true`; mỗi lần tìm công khai tốn một request
embedding, nên tìm theo ý nghĩa có ngân sách mỗi phút
(`AUTOXIDE_CONTENT_PUBLIC_SEARCH_PER_MINUTE`, mặc định 30) — vượt thì chỉ tìm theo chữ.

## Chi phí và giới hạn

- **Hạn mức token mỗi ngày:** `AUTOXIDE_AI_DAILY_TOKEN_LIMIT` (mặc định 2.000.000 token, gồm
  cả sinh chữ và embedding; `0` = không giới hạn), đếm trong database, chung mọi tiến trình
  máy chủ. Vượt thì tính năng AI trả `429 ai.quota_exceeded` tới nửa đêm UTC. `GET
  /api/v1/ai/status` (permission `ai.view`) cho biết đã dùng bao nhiêu hôm nay.
- **Metrics** (khi `AUTOXIDE_HTTP_METRICS=true`): `ai_requests_total{purpose,outcome}`,
  `ai_tokens_total{purpose,direction}`, `ai_request_duration_seconds{purpose}`.
- **Chi phí ước tính** dịch một bài 1.500 chữ (khoảng 5.000 token vào và 5.000 token ra), theo
  bảng giá Anthropic ngày 3/11/2026: Claude Opus 5.5 ≈ 0,12 USD, Claude Sonnet 5.5 ≈ 0,06 USD,
  Claude Haiku 5.5 ≈ 0,003 USD. Một lần gợi ý SEO tốn một phần nhỏ số đó. Mô hình chạy tại máy
  không tốn tiền theo request.

## Gửi gì cho nhà cung cấp

| Tính năng | Gửi |
| --------- | --- |
| Gợi ý SEO | tiêu đề, tóm tắt, chữ thuần của bản nháp (12.000 ký tự đầu), tên danh mục và thẻ của ngôn ngữ đó |
| Alt text | một bản nhỏ của ảnh (tối đa 1280 px, không bao giờ file gốc), ngôn ngữ, ngữ cảnh tuỳ chọn bạn gửi |
| Dịch | chữ của revision đã ghim, theo từng mẩu — tiêu đề, tóm tắt, trường SEO, alt text của ảnh, đoạn văn và tiêu đề với định dạng thành thẻ có số; không bao giờ gửi link, ID ảnh hay khối code |
| Tìm kiếm | chữ của mỗi mục theo đoạn (khi lập index), và mỗi câu tìm |

**Ảnh riêng tư** chỉ được gửi khi `AUTOXIDE_MEDIA_AI_SEND_PRIVATE=true`. Log không bao giờ chứa
prompt, nội dung, ảnh hay key — chỉ việc gì, mô hình, số token và thời gian.

## Dịch bài, chi tiết

`POST /api/v1/content/entries/{id}/translate` với `{"locale": "vi"}` trả `202` kèm một yêu
cầu; một job nền làm việc:

1. Ghim mục như hiện tại (một revision): sửa sau đó không đổi bản dịch này.
2. Chữ được cắt thành mẩu; định dạng trong đoạn thành thẻ có số (`<m1>chữ đậm</m1>`,
   `<br2/>`) để thứ tự từ đổi được giữa hai ngôn ngữ. Mô hình không bao giờ thấy JSON của thân
   bài, link hay ID ảnh.
3. Câu trả lời được kiểm: mỗi mẩu về đúng một lần, mỗi thẻ đúng một lần, không mẩu nào rỗng.
   Sai thì thử lại một lần, rồi yêu cầu thất bại với `content.translation_invalid`.
4. Các mẩu được ráp lại vào cấu trúc gốc (link và ảnh lấy từ bản gốc) và lưu thành **bản
   nháp** của bản dịch: bản dịch mới với slug sinh từ tiêu đề đã dịch, hoặc bản dịch có sẵn —
   **chỉ khi không ai sửa nó từ lúc yêu cầu**; nếu có thì yêu cầu thất bại với
   `content.translation_conflict` và không gì bị ghi đè.

Theo dõi bằng `GET /api/v1/content/translation-requests/{id}` hay webhook
(`content.translation_succeeded`, `content.translation_failed`). Bản dịch mang `ai_generated`;
khi bản gốc đổi sau đó, API quản trị trả `translation_outdated: true`. Khối code và nhúng
không được dịch; tên riêng, số và địa chỉ được giữ.

## AI agent: giới hạn và duyệt

Cấp cho mỗi agent một API key riêng, hạn ngắn, chỉ những gì nó cần:

| Agent được… | Permission |
| ----------- | ---------- |
| viết nháp, người đăng | `content.read`, `content.write_own` |
| viết và tự đăng bài của mình | `content.read`, `content.write_own`, `content.publish_own` |
| dùng cả công cụ AI | không thêm gì: gợi ý SEO và dịch theo `content.write_own`; alt text cần `media.write` |

- Permission `_own` chỉ chạm tới mục tạo bằng **chính key đó**; key khác của cùng người dùng
  nhận `403 content.not_owner`. Agent không bao giờ lưu trữ, chuyển trang, hẹn giờ hay xoá
  (xoá cần `content.delete`).
- Viết xong, agent xin duyệt: `POST /api/v1/content/entries/{id}/request-review` (event
  `content.review_requested` — chuyển cho người biên tập bằng webhook).
- Request bằng API key bị rate limiting theo từng key
  (`AUTOXIDE_HTTP_RATE_LIMIT_API_KEY_PER_MINUTE`, mặc định 120), và mọi thay đổi nằm trong
  audit log kèm key đã làm.

## Báo cho người đọc biết nội dung do AI tạo

Mục mang `ai_generated` (máy chủ tự đặt cho bản nó dịch; agent nên gửi `"ai_generated": true`
cho thứ nó viết). Khi **một người** (đăng nhập, không phải API key) đăng mục đó sau khi đã
duyệt, họ gửi `{"ai_reviewed": true}` tới `POST /entries/{id}/publish`; bản công khai ghi ai
duyệt và lúc nào. Endpoint công khai trả `ai_generated` và `ai_reviewed`, để website hiện thông
báo khi luật yêu cầu — ví dụ Điều 50 của EU AI Act, áp dụng từ 2/8/2026 cho văn bản do AI tạo,
đăng để thông tin cho công chúng về việc công ích; ngoại lệ cho văn bản đã qua người duyệt và
có người chịu trách nhiệm biên tập được hiểu hẹp. Autoxide chỉ ghi lại sự thật, không đưa ra
lời khuyên pháp lý.

## Khi có lỗi

| Mã | Nghĩa | Làm gì |
| -- | ----- | ------ |
| `ai.disabled`, `ai.embedding_disabled` | chưa cấu hình | đặt `AUTOXIDE_AI_*` / `AUTOXIDE_EMBEDDING_*` |
| `ai.misconfigured` | nhà cung cấp từ chối key hay cấu hình | chạy `autoxide ai check`; log máy chủ ghi mã HTTP |
| `ai.provider_unavailable` | bận, bị rate limit hay không kết nối được | thử lại sau `Retry-After`; dịch bài tự thử lại |
| `ai.quota_exceeded` | hết hạn mức token của ngày | đợi tới nửa đêm UTC hay nâng hạn mức |
| `ai.refused` | mô hình từ chối nội dung | sửa nội dung hay làm bằng tay |
| `ai.output_truncated` | câu trả lời quá dài | giảm `AUTOXIDE_CONTENT_TRANSLATE_BATCH_TOKENS` |
| `ai.output_invalid`, `content.translation_invalid` | sai dạng hai lần | thử lại, hay dùng mô hình mạnh hơn |
| `ai.unsupported_input` | mô hình không đọc ảnh | đặt đúng `AUTOXIDE_AI_IMAGES`, hay dùng mô hình đọc ảnh |

## Cấu hình

Mọi biến và giá trị mặc định có trong [`.env.example`](../.env.example): `AUTOXIDE_AI_*`,
`AUTOXIDE_EMBEDDING_*`, `AUTOXIDE_MEDIA_AI_SEND_PRIVATE`,
`AUTOXIDE_CONTENT_TRANSLATE_BATCH_TOKENS`, `AUTOXIDE_CONTENT_PUBLIC_SEMANTIC_SEARCH`,
`AUTOXIDE_CONTENT_PUBLIC_SEARCH_PER_MINUTE`, `AUTOXIDE_HTTP_RATE_LIMIT_API_KEY_*`.
