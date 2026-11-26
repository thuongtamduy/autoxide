## D-20261126-review-hide-policy · Ẩn đánh giá của orders hỏi quy tắc luật; ModerationPolicy vào kernel — 2026-11-26 — agent quyết

Chủ đề: Pháp luật và tuân thủ

Quyết định: Trait `ModerationPolicy` chuyển từ `autoxide_compliance::api` sang `autoxide_kernel::moderation` (compliance
re-export, cùng một trait, không phá vỡ). `orders` lấy nó bằng `require().ok()`; khi có, ẩn đánh giá phải qua
`check_hide_reason(tenant, "reviews", mã)` sau khi mã có trong danh sách lý do của `orders`. Ánh xạ mã: `unlawful_content`
→ `illegal_content`, `abusive_language` → `offensive`, `spam_or_advertising` → `spam`, `not_about_product` → `unrelated`,
`personal_data` giữ nguyên; mã khách tự thêm giữ nguyên nên bị quy tắc luật từ chối. Chưa cài `compliance`: như cũ.

Căn cứ (rà pháp lý vòng 1, LG-12): Luật Bảo vệ quyền lợi người tiêu dùng 19/2023/QH15 và NĐ 55/2024/NĐ-CP (không ẩn đánh
giá của người tiêu dùng trái quy định) — cần đối chiếu điều cụ thể.

Lý do: chọn nối code thay vì sửa tài liệu, vì danh sách lý do của `orders` cho khách thêm mã — khách có thể thêm "đánh giá
xấu" và ẩn đánh giá tiêu cực, đúng điều luật cấm; quy tắc luật có ngày hiệu lực ở `compliance` mới là nơi đổi khi luật đổi.
Trait vào kernel để `orders` không phải phụ thuộc crate `compliance` (kéo theo `auth`, trái D-046).

Đã cân nhắc: chỉ sửa tài liệu — để ngỏ khe hở trên; khoá danh sách lý do của `orders` — viết cứng quy định trong code.
