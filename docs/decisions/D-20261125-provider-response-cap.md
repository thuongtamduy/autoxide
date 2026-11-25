## D-20261125-provider-response-cap · Câu trả lời của nhà cung cấp ngoài đọc tối đa `AUTOXIDE_PROVIDER_MAX_RESPONSE_BYTES` (mặc định 16 MiB) — 2026-11-25 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: một biến cấu hình của kernel, `AUTOXIDE_PROVIDER_MAX_RESPONSE_BYTES` (64 KiB … 1 GiB, mặc định 16 MiB), áp
cho mọi câu trả lời của nhà cung cấp ngoài: `autoxide_kernel::provider::send` (mọi bộ nối thanh toán, vận chuyển, SMS,
hoá đơn điện tử) và module `ai` (sinh chữ, embedding) cùng đọc thân qua `provider::read_body`, dừng ngay khi vượt
giới hạn (và từ chối trước khi đọc nếu `Content-Length` đã vượt). Vượt thì cuộc gọi coi như nhà cung cấp không tới
được (`Unreachable` / `ai.provider_unavailable`), log ghi giới hạn. Giới hạn là một số toàn process (atomic), đặt một
lần lúc khởi động khi module nền của kernel đọc cấu hình. Sửa I-2 của `docs/security/review-2026-11.md`.

Lý do: trước đây thân câu trả lời đọc hết vào bộ nhớ không giới hạn; địa chỉ do người vận hành cấu hình nhưng nhà cung
cấp hỏng, bị chiếm hay proxy ở giữa vẫn làm process hết bộ nhớ. Một biến cho mọi nhà cung cấp vì người vận hành chỉ cần
một nút; 16 MiB đủ cho lô embedding lớn (vd 64 vector × 1536 chiều ≈ 2 MB JSON) mà vẫn chặn được câu trả lời vô hạn.
Biến toàn cục thay vì tham số vì `send` / `http_client` là hàm tự do bộ nối gọi thẳng, không cầm cấu hình — đổi chữ ký
là phá vỡ hợp đồng với bộ nối ngoài repo.

Đã cân nhắc: hai biến riêng cho AI và bộ nối — thêm nút mà không ai cần chỉnh khác nhau; truyền giới hạn qua
`http_client` — reqwest không có tuỳ chọn giới hạn thân câu trả lời.
