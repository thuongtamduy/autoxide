## D-20261128-access-revoked-retry · Event `auth.access_revoked` không bao giờ vào dead-letter — 2026-11-28 — agent quyết

Chủ đề: Database, dữ liệu, job, event

Quyết định: bộ phát event coi `auth.access_revoked` như `privacy::USER_DELETED`: quá `MAX_ATTEMPTS` lần giao hỏng vẫn
không đánh dấu `failed_at`, thử lại tối đa mỗi giờ một lần (backoff có trần sẵn có) và ghi log lỗi mỗi lần. Danh sách
event "không bỏ cuộc" là hằng `NEVER_GIVE_UP` trong `event/dispatcher.rs`.

Lý do: P16 — `automations` tạm dừng luật của tài khoản bị cắt cửa vào khi nhận event này
(D-20261126-automation-owner-revoked). Event vào dead-letter thì luật mà người lạ dựng khi giữ tài khoản chạy tiếp mãi,
không còn đường nào khác dừng. Bên nhận đã chịu được việc nhận lại (D-054), nên thử lại mãi là an toàn; giống lý do
của D-149 với xoá tài khoản.

Đã cân nhắc: runner hỏi `auth` "tài khoản có vừa bị cắt cửa vào không" trước mỗi lần chạy (cần trait mới, đổi hợp đồng
`auth`, và vẫn hở giữa lúc khoá và lúc hỏi — D-20261126-automation-owner-revoked đã loại); job quét dead-letter của
`automations` (bảng outbox là của `auth`, module không đọc bảng của nhau).
