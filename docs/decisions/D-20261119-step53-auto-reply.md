## D-20261119-step53-auto-reply · `support`: trả lời tự động — mô hình chỉ phân loại, luật quyết trong Rust, duyệt bằng chính lệnh bật — 2026-11-19 — agent quyết

Chủ đề: MCP, AI, agent dùng API

Bối cảnh: bước 53 mục 6 làm theo D-230. Vài chỗ đặc tả để mở hay cần chọn cách làm.

Quyết định:

1. **Duyệt = lệnh bật.** `PUT /macros/{id}/auto-reply {enabled: true}` của người có `support.auto_reply.manage` ghi
   `approved_by` / `approved_at`; phải kèm ngày rà lại (hôm nay tới một năm sau) — thiếu hay sai thì
   `support.macro_not_approved`. Sửa nội dung câu trả lời mẫu thì mất dấu duyệt. Quá ngày rà lại thì không dùng nữa.
2. **Schema phân loại không có chỗ cho chữ trả lời** (`kind`, `macro_id`, `intent`, `params`, `confidence_bp`,
   `risk_flags` theo danh sách cố định): chữ do mô hình viết không có đường tới người mua. Mô hình lỗi hay trả sai schema
   → chuyển người.
3. **Luật Rust** (`autoreply::rules`): tắt → không; quá số lần liên tiếp, có từ khoá chặn (danh sách có sẵn bỏ dấu, so
   theo từ — vd "khieu nai", "hoan tien", "huy don", "gap nhan vien", "refund", "lawyer"… — cộng danh sách của nút) hay
   có cờ rủi ro → chuyển người; không phải `answer` / `fact` hay tin cậy thấp → bản nháp AI cho nhân viên.
4. **Đếm "liên tiếp"** là số câu trả lời tự động từ lần cuối nhân viên trả lời (nhân viên trả lời thì đếm lại); người mua
   viết thêm không đếm lại, nên một người mua hỏi mãi vẫn tới người sau `auto_reply_max_per_thread` câu.
5. **Gửi tự động** là tin công khai `author_kind = system`, `auto_reply = true`, không đổi trạng thái phiếu (phiếu vẫn
   trong hộp việc tới khi nhân viên xử lý hay tự đóng). Mỗi tin của người mua được trả lời nhiều nhất một lần: job có
   `unique_key` theo tin và nhật ký `support_auto_replies` có khoá duy nhất `(tenant_id, message_ref)`.
6. **Người hỏi chưa chứng minh liên lạc** (khách vãng lai chưa OTP) đưa cho `FactSource` một liên lạc rỗng: nguồn không
   khớp được chủ dữ liệu nên không trả lời.

Hệ quả: `AUTOXIDE_SUPPORT_AUTO_REPLY_FLAG_LIMIT` (mặc định 3); `docs/SUPPORT.md` có mục trả lời tự động. Căn cứ: Luật
Trí tuệ nhân tạo 134/2025/QH15 (cho người dùng biết đang nhận câu trả lời tự động) — cần đối chiếu văn bản gốc.
