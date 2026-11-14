# Bước 61 — Chat chăm sóc khách (`conversations`)

> Lộ trình: `docs/ROADMAP.md` bước 61 · Quyết định: D-221 · Đặc tả liên quan: 40 P5 (SSE), 53 (`support`),
> 37 (thông báo, xác thực tài khoản), 46 (sàn)

## Mục tiêu

Người mua nhắn tin với cửa hàng hay người bán trên sàn ngay trên website / app: hỏi về sản phẩm, đơn hàng; nhân
viên trả lời từ hộp thư chung, có thể chuyển cuộc trò chuyện thành phiếu hỗ trợ (bước 53). Tin mới đến gần như
tức thì (SSE), người không online nhận thông báo.

## Bắt đầu khi

Bước 40 P5 (SSE), 53 (`support`), 37 (thông báo). Sàn (46) thì thêm chat với người bán.

## Thiết kế

- **Cuộc trò chuyện** (`conversations_threads`: `tenant_id`, `org_id` (cửa hàng / gian hàng nhận),
  `customer_user_id` hay khách vãng lai đã xác thực OTP, `subject` (`<kind>:<id>` — sản phẩm, đơn — qua sổ đối
  tượng D-123), `status` `open/closed`, `assignee_user_id`, `last_message_at`, `unread_*`).
- **Tin nhắn** chỉ thêm (`conversations_messages`: chữ thuần, `media_ids` ảnh qua `media`, `author_kind`
  `customer/agent/system`, `client_id` của máy gửi để gửi lại không trùng). Sửa không được; thu hồi trong 2 phút
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
- **Trợ lý AI** (D-109): gợi ý câu trả lời cho nhân viên là bản nháp; không tự trả lời khách khi chưa bật rõ
  ràng và gắn nhãn "trả lời tự động".

### API, MCP, quyền, lỗi

- REST `/api/v1/conversations`: `threads` (lọc trạng thái, người nhận, chưa đọc), `messages`, `read`, `assign`,
  `close`, `escalate`; `/me/conversations` cho người mua; `stream` (SSE).
- MCP: `list_threads`, `get_thread` (`read_only`), `reply`, `suggest_reply` (bản nháp).
- Permission: `conversations.read`, `conversations.reply`, `conversations.manage`; người mua dùng `/me`.
- Event: `conversations.thread_opened`, `…_closed`, `…_assigned`, `conversations.message_created` (không mang
  nội dung).
- Lỗi: `conversations.thread_closed` (409), `conversations.blocked` (403), `conversations.recall_expired` (409),
  `conversations.verification_required` (403).
- Hạn lưu tin nhắn là cấu hình (`AUTOXIDE_CONVERSATIONS_RETENTION_DAYS`, mặc định 365); xoá tài khoản thì ẩn danh
  hoá phía người mua.

## Test bắt buộc (cả hai database)

`gui_lai_cung_client_id_khong_trung`, `stream_chi_phat_cho_nguoi_co_quyen`, `nhan_vien_nhanh_khac_khong_doc_duoc`,
`thu_hoi_qua_hai_phut_bi_tu_choi`, `tin_chua_doc_gui_thong_bao_mot_lan`, `chuyen_thanh_phieu_ho_tro`,
`het_han_luu_thi_xoa`.

## Căn cứ pháp lý

| Nội dung | Văn bản | Ghi chú |
|---|---|---|
| Dữ liệu cá nhân trong tin nhắn, hạn lưu, xoá | Luật 91/2025/QH15; NĐ 356/2025 | Cần đối chiếu |
| Xác thực tài khoản đăng nội dung, gỡ nội dung vi phạm theo yêu cầu | NĐ 147/2024/NĐ-CP | Cần đối chiếu phạm vi với chat riêng tư |

## Agent làm bước này tự quyết

Thời gian thu hồi; cách gom thông báo; mẫu che liên hệ ngoài sàn mặc định. Ghi vào DECISIONS.
