# Bước 61 — Chat chăm sóc khách (`conversations`)

> Lộ trình: `docs/ROADMAP.md` bước 61 · Quyết định: D-221, D-230 (trả lời tự động), D-232 (gọi điện)
> Đặc tả liên quan: 40 P5 (SSE), 53 (`support`), 37 (thông báo, xác thực tài khoản), 46 (sàn), 51 (hộp việc),
> 44 (webhook)

## Mục tiêu

Người mua nhắn tin với cửa hàng hay người bán trên sàn ngay trên website / app: hỏi về sản phẩm, đơn hàng; nhân
viên trả lời từ hộp thư chung, có thể chuyển cuộc trò chuyện thành phiếu hỗ trợ (bước 53). Tin mới đến gần như
tức thì (SSE), người không online nhận thông báo. Người mua gọi cửa hàng từ trình duyệt hay bấm "Gọi lại cho
tôi"; cuộc gọi đi qua nhà cung cấp thoại và hiện trong dòng thời gian của cuộc trò chuyện (D-232).

## Bắt đầu khi

Bước 40 P5 (SSE), 53 (`support`), 37 (thông báo). Sàn (46) thì thêm chat với người bán. Gọi điện (D-232) dùng
thêm hộp việc của bước 51 (P2) và `Storage` của `files` cho bản ghi âm.

Ngoài phạm vi: bộ nối nhà cung cấp thoại thật (vd Stringee, Twilio) — module riêng, cần tài khoản và secret
thật của chủ dự án → dừng hỏi, không tự làm (D-232). Bước này chỉ có trait và bộ giả.

## Thiết kế

- **Cuộc trò chuyện** (`conversations_threads`: `tenant_id`, `org_id` (cửa hàng / gian hàng nhận),
  `customer_user_id` hay khách vãng lai đã xác thực OTP, `subject` (`<kind>:<id>` — sản phẩm, đơn — qua sổ đối
  tượng D-123), `status` `open/closed`, `assignee_user_id`, `last_message_at`, `unread_*`).
- **Tin nhắn** chỉ thêm (`conversations_messages`: chữ thuần, `media_ids` ảnh qua `media`, `author_kind`
  `customer/agent/system`, `client_id` của máy gửi để gửi lại không trùng, `call_id` NULL — tin hệ thống tóm
  tắt cuộc gọi, D-232). Sửa không được; thu hồi trong 2 phút
  (ẩn nội dung, giữ dấu).
- **Thời gian thực:** stream SSE của bước 40 (`/api/v1/conversations/stream`) phát `message_created`,
  `thread_updated`, `typing` (không lưu), lọc theo quyền; không có WebSocket riêng. Đã đọc: con trỏ đọc theo người.
- **Không online:** tin chưa đọc sau N phút → thông báo qua `notifications` (email / đẩy di động), gom lô.
- **Chuyển thành phiếu** (`support`, bước 53): `POST /threads/{id}/escalate` gọi `SupportIntake::open` với lịch
  sử rút gọn và liên kết.
- **Sàn:** người mua chat với gian hàng; nhân viên sàn chỉ xem khi có tranh chấp hay báo vi phạm (audit). Chặn
  dụ giao dịch ngoài sàn theo chính sách của sàn (dữ liệu: mẫu số điện thoại / link bị che, có cấu hình) — không
  đọc tin để quảng cáo.
- **Chống lạm dụng:** throttle theo người, chặn người dùng, báo vi phạm; xác thực tài khoản theo NĐ 147/2024 dùng
  `PostingVerification` của kernel (D-202) khi khách bật.
- **Trợ lý AI** (D-109, D-230): câu hỏi phức tạp hay mơ hồ → gợi ý câu trả lời cho nhân viên là bản nháp. Câu
  hỏi đơn giản đã có câu trả lời chính xác → trả lời tự động bằng `AutoResponder` của `support` (bước 53): cùng
  luật, cùng câu trả lời đã duyệt và `FactSource`, cùng cài đặt bật / tắt theo nút (tắt mặc định), không có bộ
  quyết riêng ở đây. Tin tự động `author_kind = system`, `auto_reply = true`, nhãn "Trả lời tự động" và nút
  "Gặp nhân viên" (dừng tự động trong cuộc trò chuyện, phân công người); chạy bằng job sau khi tin của khách đã
  lưu, không chặn việc gửi tin.

### Gọi điện / tổng đài trong chat (D-232)

**Trait ở kernel** (`autoxide_kernel::voice`, như `PaymentGateway` của đặc tả 36): bộ nối nhà cung cấp là
module riêng chỉ phụ thuộc kernel, đăng ký TRƯỚC `conversations`, gọi `context.contribute::<dyn
VoiceProvider>(…)`; mọi hàm nhận `tenant: TenantId`, bộ nối tự đọc thông tin kết nối của khách từ kho secret
theo khách (bước 33). Testkit có `FakeVoiceProvider`: giữ mọi lệnh trong bộ nhớ, không có HTTP client, ký
webhook bằng HMAC, `emit(call_id, status)` để test đẩy trạng thái, bản ghi âm là vài byte cố định.

```rust
// crates/kernel/src/voice.rs — phác thảo, tra API thật khi làm
#[async_trait] pub trait VoiceProvider: Send + Sync {
    fn provider(&self) -> &'static str;                 // "stringee", "twilio", "fake"…
    fn capabilities(&self) -> VoiceCapabilities;         // browser_calls, outbound, recording, recording_notice
    /// Token ngắn hạn cho SDK WebRTC của nhà cung cấp: người mua chỉ gọi được tới `hotline`; nhân viên nhận /
    /// gọi với danh tính `identity` (= user id, để webhook báo đúng người nghe máy).
    async fn browser_session(&self, tenant: TenantId, request: &BrowserSessionRequest) -> Result<BrowserSession, Error>;
    /// Gọi ra: nối máy nhân viên với số người mua; số hiển thị là `from` (hotline của cửa hàng).
    async fn place_call(&self, tenant: TenantId, call: &OutboundCall) -> Result<ProviderCall, Error>;
    /// Kiểm chữ ký (so sánh thời gian hằng), cửa sổ chống phát lại; sai → `voice.signature_invalid`.
    fn verify_webhook(&self, tenant: TenantId, headers: &http::HeaderMap, body: &[u8]) -> Result<Vec<VoiceNotification>, Error>;
    async fn fetch_recording(&self, tenant: TenantId, provider_recording_id: &str) -> Result<ByteStream, Error>;
    async fn delete_recording(&self, tenant: TenantId, provider_recording_id: &str) -> Result<(), Error>;
}
pub struct OutboundCall { pub call_id: Uuid /* mã phía mình: nhà cung cấp chống trùng */, pub from: PhoneNumber,
    pub to: PhoneNumber, pub agent_identity: String, pub record: bool,
    pub recording_notice: Option<String> /* câu đã dịch; bắt buộc khi record */ }
pub struct VoiceNotification { pub provider_event_id: String, pub provider_call_id: String,
    pub call_id: Option<Uuid>, pub status: CallStatus, pub agent_identity: Option<String>, pub occurred_at: DateTime<Utc>,
    pub duration_seconds: Option<u32>, pub provider_recording_id: Option<String> }
pub enum CallStatus { Queued, Ringing, InProgress, Completed, NoAnswer, Busy, Failed, Cancelled }
```

Mã lỗi chung của nhóm ở kernel: `voice.signature_invalid` (401), `voice.provider_unavailable` (503).

- **Đường dây của cửa hàng** (`conversations_voice_lines`, một dòng mỗi nút): nhà cung cấp, `hotline` (số công
  khai của cửa hàng), giờ nhận gọi lại, ghi âm bật / tắt. Nút chưa có đường dây thì giao diện không hiện nút gọi
  và endpoint gọi trả 404 `conversations.voice_unavailable`.
- **Gọi cửa hàng từ trình duyệt.** `GET /me/conversations/threads/{id}/voice` trả thông tin cần hiện trước khi
  gọi (`available`, `recording`, câu thông báo ghi âm, giờ nhận gọi lại). `POST
  /me/conversations/threads/{id}/calls` ghi `conversations_calls` (`direction = inbound`, `queued`) rồi, ngoài
  transaction, gọi `browser_session` → trả token cho SDK WebRTC; cuộc gọi tới hotline, nhân viên nghe máy bằng
  phiên của mình (`GET /voice/agent-session`).
- **Yêu cầu gọi lại.** Người mua bấm "Gọi lại cho tôi": `POST /me/conversations/threads/{id}/callbacks
  {client_id, window_start, window_end, phone_source, recording_ack?}`.
  - Số gọi lại chỉ là số đã xác thực: `phone_source = account` (số của tài khoản qua
    `AccountVerification::verified_phone` của bước 37) hay `otp` (số khách vãng lai đã xác thực OTP trong cuộc
    trò chuyện); không có → 403 `conversations.phone_unverified`. Số lưu đã mã hoá theo khách (`seal`), kèm dạng
    che.
  - Khung giờ trong giờ nhận gọi lại của đường dây và trong `CALLBACK_DAYS_AHEAD` ngày; sai → 422
    `conversations.callback_window_invalid` (gom lỗi một lần, D-083).
  - Gửi lại cùng `client_id` → trả yêu cầu cũ (200); mỗi cuộc trò chuyện chỉ một yêu cầu đang mở (`open_key`),
    yêu cầu khác khi còn yêu cầu mở → 409 `conversations.callback_pending`. Throttle theo người mua
    `CALLS_PER_DAY` (chung cho gọi từ trình duyệt và gọi lại) → 429 `conversations.calls_too_many`.
  - Mỗi yêu cầu mở là **một** mục trong hộp việc: `WorkSource` `conversations.callbacks` (permission
    `conversations.call`, lọc theo phạm vi nút của cuộc trò chuyện, `due_at = window_end`), đếm thẳng từ bảng
    yêu cầu, không có bảng việc riêng. Event `conversations.callback_requested`.
- **Nhân viên gọi.** `POST /callbacks/{id}/call` (`Idempotency-Key`): transaction ngắn khoá dòng yêu cầu
  (`rows::lock_one`), `open → calling`, ghi cuộc gọi `outbound` `queued` → commit → ngoài transaction
  `place_call` với `from = hotline` (số người mua thấy là hotline của cửa hàng, không phải số nhân viên), `call_id`
  làm mã chống trùng ở nhà cung cấp. Hai nhân viên bấm cùng lúc → một cuộc gọi, người sau 409
  `conversations.callback_taken`; gọi lại cùng key → cùng cuộc gọi. Nhà cung cấp lỗi → cuộc gọi `failed`, yêu
  cầu về `open`. Cuộc gọi `completed` → yêu cầu `done`; `no_answer` / `busy` → về `open`, tăng `attempts`.
  Job `conversations.expire_callbacks`: quá `window_end` + 2 giờ còn mở → `expired`, báo người mua một lần
  (chọn giờ khác).
- **Số người mua không lộ.** Số đầy đủ chỉ đi tới `VoiceProvider`; API, MCP, event, log, SSE không bao giờ có số
  đầy đủ. Nhân viên của nhánh nhận cuộc trò chuyện thấy dạng che (`090•••567`, như đơn vãng lai của bước 45) và
  gọi bằng nút; nhân viên nhánh khác không thấy yêu cầu, cuộc gọi hay số (404, lọc theo nhánh như tin nhắn);
  nhân viên sàn xem khi có tranh chấp thấy cuộc gọi nhưng không thấy số. Log dùng `PhoneNumber::masked()`.
- **Dòng thời gian.** `conversations_calls` gắn `thread_id`; cuộc gọi tới trạng thái cuối thì thêm đúng một tin
  `author_kind = system` có `call_id` (mẫu i18n `conversations.call_summary`: hướng, kết quả, thời lượng),
  `client_id = call:<call_id>` để nhận lại webhook không thêm tin thứ hai; SSE phát `call_updated` theo quyền.
- **Ghi âm — tắt mặc định.** Khách bật theo nút (`recording_enabled`, `conversations.voice.manage`); bật chỉ
  được khi nhà cung cấp có `recording_notice`, không thì 422 `conversations.recording_unsupported`.
  - Khi bật: đầu mỗi cuộc gọi nhà cung cấp phát câu thông báo ghi âm (`OutboundCall.recording_notice`, cũng cài
    cho cuộc gọi tới hotline); trước khi gọi giao diện người mua hiện thông báo (`GET …/voice` trả `recording`
    và câu thông báo) và lệnh gọi / yêu cầu gọi lại phải có `recording_ack = true`, không thì 422
    `conversations.recording_notice_required`; thời điểm xác nhận lưu ở cuộc gọi / yêu cầu.
  - Bản ghi: webhook báo có bản ghi → job `conversations.fetch_recording` (ngoài transaction) tải từ nhà cung
    cấp, lưu vào `Storage` của khách (riêng tư, key `conversations/recordings/<call_id>`), ghi `recording_ref`,
    `recording_expires_at = ended_at + RECORDING_RETENTION_DAYS`, rồi `delete_recording` ở nhà cung cấp.
  - Nghe: `GET /calls/{id}/recording` (`conversations.recordings.read`, ghi audit mỗi lần mở). Xoá theo yêu
    cầu chủ thể: `DELETE /calls/{id}/recording` (`conversations.voice.manage`) và khi nhận
    `privacy::USER_DELETED` (xoá bản ghi các cuộc gọi của người đó, xoá số trong yêu cầu gọi lại); job
    `conversations.purge_recordings` mỗi ngày xoá bản quá hạn. Khai `Module::retention` cho bản ghi và yêu cầu
    gọi lại.
- **Webhook của nhà cung cấp** (như `shipping`, bước 44): `POST /voice/webhooks/{provider}/{line_token}` công
  khai, khách phân giải từ `line_token` (lưu băm) như webhook của `shipping`; giới hạn thân qua
  `Module::body_limits` (`VOICE_WEBHOOK_MAX_BYTES`). `verify_webhook` sai → 401 `voice.signature_invalid`, chỉ
  log và số đo `conversations_voice_webhook_rejected_total{provider}`, không lưu thân. Đúng → một transaction:
  INSERT `conversations_voice_events` ON CONFLICT DO NOTHING + enqueue `conversations.process_voice_event`
  (`unique_key` = id event) → 200. Job khoá dòng cuộc gọi, chỉ đi tới trạng thái sau (webhook tới sai thứ tự
  không lùi trạng thái; trạng thái cuối không đổi nữa), ghi thời lượng, nhân viên (`agent_identity`).

**Bảng của gọi điện** (mọi bảng có `tenant_id` đứng đầu khoá và index; thời gian UTC; trạng thái là chuỗi):

| Bảng | Cột chính | Khoá / index |
|---|---|---|
| `conversations_voice_lines` | `id`, `tenant_id`, `org_id`, `provider`, `hotline` (E.164), `line_token_hash`, `recording_enabled` BOOL default false, `callback_hours` text (JSON giờ nhận gọi lại theo thứ, mặc định 08:00–20:00 giờ của nút), `active`, `updated_at` | unique (`tenant_id`, `org_id`); index (`tenant_id`, `line_token_hash`) |
| `conversations_callback_requests` | `id`, `tenant_id`, `org_id`, `thread_id`, `requester_user_id` NULL, `client_id`, `phone_sealed`, `phone_masked`, `phone_source` (`account/otp`), `window_start`, `window_end`, `recording_ack_at` NULL, `status` (`open/calling/done/cancelled/expired`), `open_key` NULL (= `thread_id` khi `open` / `calling`), `assigned_user_id` NULL, `attempts` INT default 0, `last_call_id` NULL, `created_at`, `updated_at` | unique (`tenant_id`, `thread_id`, `client_id`); unique (`tenant_id`, `open_key`); index (`tenant_id`, `org_id`, `status`, `window_start`) |
| `conversations_calls` | `id`, `tenant_id`, `org_id`, `thread_id`, `callback_id` NULL, `direction` (`inbound/outbound`), `status` (`queued/ringing/in_progress/completed/no_answer/busy/failed/cancelled`), `provider`, `provider_call_id` NULL, `agent_user_id` NULL, `started_at` NULL, `answered_at` NULL, `ended_at` NULL, `duration_seconds` NULL, `recording_enabled` BOOL, `recording_ack_at` NULL, `provider_recording_id` NULL, `recording_ref` NULL (key trong `Storage`), `recording_expires_at` NULL, `created_at`, `updated_at` | unique (`tenant_id`, `provider`, `provider_call_id`); index (`tenant_id`, `thread_id`, `created_at`), (`tenant_id`, `org_id`, `created_at`, `id`), (`tenant_id`, `recording_expires_at`) |
| `conversations_voice_events` | `id`, `tenant_id`, `provider`, `provider_event_id`, `call_id` NULL, `body` (nguyên văn đã kiểm chữ ký), `status` (`received/processed/failed`), `received_at` | unique (`tenant_id`, `provider`, `provider_event_id`); index (`tenant_id`, `status`, `received_at`) |

`open_key`: cột mang giá trị khi yêu cầu còn mở, NULL khi không — unique trên cột NULL chạy được trên cả hai
database (như `active_buyer_key` của bước 45).

### API, MCP, quyền, lỗi

- REST `/api/v1/conversations`: `threads` (lọc trạng thái, người nhận, chưa đọc), `messages`, `read`, `assign`,
  `close`, `escalate`; `/me/conversations` cho người mua; `stream` (SSE).
- REST gọi điện (D-232):
  - người mua: `GET /me/conversations/threads/{id}/voice`, `POST /me/conversations/threads/{id}/calls`
    (`recording_ack?`; 201 kèm token phiên WebRTC), `POST /me/conversations/threads/{id}/callbacks` (201; gửi
    lại cùng `client_id` → 200), `DELETE /me/conversations/threads/{id}/callbacks/{callback_id}` (huỷ);
  - nhân viên: `GET /callbacks` (`Page`, lọc `status`, `org_id`, khung giờ; số đã che — `conversations.call`),
    `POST /callbacks/{id}/call` (`Idempotency-Key` bắt buộc), `POST /callbacks/{id}/close`, `GET
    /threads/{id}/calls`, `GET /voice/agent-session` (`conversations.call`); `GET /calls/{id}/recording`
    (`conversations.recordings.read`); `DELETE /calls/{id}/recording`, `GET/PUT /voice/lines/{org_id}`
    (`conversations.voice.manage`);
  - công khai: `POST /voice/webhooks/{provider}/{line_token}` (chữ ký của nhà cung cấp).
- MCP: `list_threads`, `get_thread` (`read_only`), `reply`, `suggest_reply` (bản nháp); `list_callbacks`,
  `list_calls` (`read_only`, số đã che, không bản ghi — D-232). Không có tool gọi điện hay nghe bản ghi (cuộc
  gọi cần người trên máy; bản ghi là dữ liệu nhạy cảm).
- Permission: `conversations.read`, `conversations.reply`, `conversations.manage`; người mua dùng `/me`. Gọi điện
  (D-232): `conversations.call` (hộp việc gọi lại, gọi, xem cuộc gọi), `conversations.recordings.read` (nghe bản
  ghi), `conversations.voice.manage` (đường dây, bật ghi âm, xoá bản ghi theo yêu cầu).
- Event: `conversations.thread_opened`, `…_closed`, `…_assigned`, `conversations.message_created` (không mang
  nội dung); `conversations.callback_requested`, `conversations.call_ended` (hướng, kết quả, thời lượng, có bản
  ghi hay không), `conversations.recording_deleted` — không mang số điện thoại hay `recording_ref` (D-232).
- Job (D-232): `conversations.process_voice_event` (enqueue cùng transaction với webhook),
  `conversations.fetch_recording` (enqueue, `unique_key` = `call_id`), `conversations.expire_callbacks` (mỗi 15
  phút), `conversations.purge_recordings` (mỗi ngày).
- Lỗi: `conversations.thread_closed` (409), `conversations.blocked` (403), `conversations.recall_expired` (409),
  `conversations.verification_required` (403). Lỗi của gọi điện (D-232):

  | Code | HTTP | Tiếng Việt |
  |---|---|---|
  | `conversations.voice_unavailable` | 404 | Cửa hàng chưa bật gọi điện. |
  | `conversations.phone_unverified` | 403 | Cần số điện thoại đã xác thực để nhận cuộc gọi lại. |
  | `conversations.callback_window_invalid` | 422 | Khung giờ gọi lại phải nằm trong giờ nhận gọi của cửa hàng, trong {days} ngày tới. |
  | `conversations.callback_pending` | 409 | Bạn đang có một yêu cầu gọi lại chờ xử lý. |
  | `conversations.callback_taken` | 409 | Yêu cầu gọi lại này đang được nhân viên khác xử lý. |
  | `conversations.callback_not_found` | 404 | Không tìm thấy yêu cầu gọi lại. |
  | `conversations.call_not_found` | 404 | Không tìm thấy cuộc gọi. |
  | `conversations.calls_too_many` | 429 | Bạn đã gọi quá số lần cho phép trong ngày; hãy thử lại sau. |
  | `conversations.recording_notice_required` | 422 | Cần xác nhận đã biết cuộc gọi được ghi âm trước khi gọi. |
  | `conversations.recording_unsupported` | 422 | Nhà cung cấp thoại không phát được thông báo ghi âm; chưa bật ghi âm được. |
  | `voice.signature_invalid` (kernel) | 401 | Chữ ký webhook của nhà cung cấp thoại không hợp lệ. |
  | `voice.provider_unavailable` (kernel) | 503 | Nhà cung cấp thoại tạm thời không phản hồi; hãy thử lại sau. |

- Hạn lưu tin nhắn là cấu hình (`AUTOXIDE_CONVERSATIONS_RETENTION_DAYS`, mặc định 365); xoá tài khoản thì ẩn danh
  hoá phía người mua.
- Cấu hình gọi điện (D-232): `AUTOXIDE_CONVERSATIONS_RECORDING_RETENTION_DAYS` (mặc định 90),
  `AUTOXIDE_CONVERSATIONS_CALLS_PER_DAY` (5, theo người mua), `AUTOXIDE_CONVERSATIONS_CALLBACK_DAYS_AHEAD` (7),
  `AUTOXIDE_CONVERSATIONS_VOICE_WEBHOOK_MAX_BYTES` (65536). Sinh lại `.env.example`.

## Test bắt buộc (cả hai database)

`gui_lai_cung_client_id_khong_trung`, `stream_chi_phat_cho_nguoi_co_quyen`, `nhan_vien_nhanh_khac_khong_doc_duoc`,
`thu_hoi_qua_hai_phut_bi_tu_choi`, `tin_chua_doc_gui_thong_bao_mot_lan`, `chuyen_thanh_phieu_ho_tro`,
`het_han_luu_thi_xoa`, `tu_dong_tra_loi_qua_autoresponder_cua_support`, `gap_nhan_vien_dung_tu_dong_trong_cuoc`.

Gọi điện (D-232), với `FakeVoiceProvider`: `goi_lai_tao_dung_mot_muc_hop_viec` (gửi lại cùng `client_id` và 10
request song song → một yêu cầu, hộp việc đếm 1), `hai_nhan_vien_bam_goi_cung_luc_chi_mot_cuoc_goi` (đồng thời),
`webhook_trung_khong_ghi_hai_lan` (cùng event giao hai lần, song song → một lần đổi trạng thái, một tin hệ thống),
`webhook_sai_chu_ky_401_khong_luu`, `webhook_den_sai_thu_tu_khong_lui_trang_thai`, `ghi_am_tat_mac_dinh`,
`bat_ghi_am_phai_xac_nhan_truoc_khi_goi`, `nha_cung_cap_khong_phat_thong_bao_thi_khong_bat_ghi_am`,
`nhan_vien_nhanh_khac_khong_thay_so_nguoi_mua` (cả REST, MCP, event, SSE), `so_chua_xac_thuc_khong_yeu_cau_goi_lai`,
`so_hien_thi_la_hotline_cua_cua_hang`, `ban_ghi_luu_vao_storage_va_xoa_o_nha_cung_cap`,
`ban_ghi_het_han_thi_xoa`, `xoa_tai_khoan_xoa_ban_ghi_va_so`, `cuoc_goi_hien_trong_dong_thoi_gian_mot_lan`,
`bo_gia_khong_goi_mang` (toàn luồng gọi lại, webhook, tải bản ghi chạy xong mà không mở kết nối mạng — bộ giả
không giữ HTTP client).

## Chỗ dễ sai (gọi điện, D-232)

- **Không gọi mạng trong transaction ghi**: `browser_session`, `place_call`, `fetch_recording`,
  `delete_recording` luôn sau commit hay trong job; transaction chỉ ghi trạng thái.
- **Một yêu cầu, một mục việc**: chống trùng bằng unique (`thread_id`, `client_id`) và `open_key`, không đọc rồi
  ghi; hộp việc đếm từ bảng yêu cầu, không có bảng việc thứ hai để lệch.
- **Webhook**: trả 2xx chỉ sau khi đã ghi nguyên văn; chống trùng bằng `provider_event_id`; không lùi trạng thái;
  sai chữ ký không lưu thân. Dựa vào `call_id` mình gửi đi, không dựa vào số điện thoại để ghép cuộc gọi.
- **Số điện thoại**: chỉ lưu dạng mã hoá + dạng che; số đầy đủ chỉ mở ra ngay trước `place_call`, không vào log,
  event, lỗi, payload SSE.
- **Ghi âm**: mặc định tắt; không bật được khi nhà cung cấp không phát thông báo; cuộc gọi ghi âm thiếu
  `recording_ack` của người mua là lỗi, không phải cảnh báo; bản ghi ở nhà cung cấp phải xoá sau khi đã lưu về.

## Căn cứ pháp lý

| Nội dung | Văn bản | Ghi chú |
|---|---|---|
| Dữ liệu cá nhân trong tin nhắn, hạn lưu, xoá | Luật 91/2025/QH15; NĐ 356/2025 | Cần đối chiếu |
| Xác thực tài khoản đăng nội dung, gỡ nội dung vi phạm theo yêu cầu | NĐ 147/2024/NĐ-CP | Cần đối chiếu phạm vi với chat riêng tư |
| Ghi âm cuộc gọi: thông báo trước, mục đích, hạn lưu, xoá theo yêu cầu chủ thể; số điện thoại người mua (D-232) | Luật 91/2025/QH15; NĐ 356/2025 | Cần đối chiếu: ghi âm chỉ cần thông báo hay cần đồng ý; hạn lưu mặc định |
| Số hiển thị khi gọi ra là hotline của cửa hàng (D-232) | Luật Viễn thông 24/2023/QH15 và văn bản hướng dẫn (hiển thị số chủ gọi) | Cần đối chiếu văn bản gốc / người có chuyên môn pháp lý; nhà cung cấp chịu phần đăng ký số |

## Agent làm bước này tự quyết

Thời gian thu hồi; cách gom thông báo; mẫu che liên hệ ngoài sàn mặc định; mặc định của gọi điện (hạn lưu bản
ghi 90 ngày, 5 lần gọi / ngày, giờ nhận gọi lại 08:00–20:00, quá hạn yêu cầu gọi lại sau 2 giờ — D-232). Ghi vào
DECISIONS.
