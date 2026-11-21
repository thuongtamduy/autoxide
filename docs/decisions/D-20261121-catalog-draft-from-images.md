## D-20261121-catalog-draft-from-images · Tạo sản phẩm nháp từ ảnh: ảnh qua `Media::ai_image`, job, kết quả luôn là nháp — 2026-11-21 — agent quyết

Chủ đề: Thương mại

Quyết định: (1) `media` công bố thêm `Media::ai_image(tenant, id)` (có cài mặc định trả `None` — bản giả
ở module khác không phải sửa, D-038): ảnh thu nhỏ do máy chủ làm, cạnh dài ≤ 1280 px, đã bỏ metadata,
không bao giờ là ảnh gốc; cùng luật với gợi ý alt text (ảnh riêng tư chỉ khi
`AUTOXIDE_MEDIA_AI_SEND_PRIVATE`). (2) `POST /products/from-images` kiểm ngay quyền, tính năng gói,
mô hình đọc được ảnh (`Capabilities.images`, không thì 503 `catalog.ai_images_unavailable`), ảnh có trong
`media`, ngôn ngữ, danh mục, nhóm thuế — gom lỗi một lần (D-083); rồi xếp job (`catalog_image_drafts`,
`queued` → `running` → `succeeded` / `failed`, chạy một lần). Job đọc ảnh và gọi mô hình ngoài
transaction ghi, rồi tạo sản phẩm qua đúng service của REST với quyền của người yêu cầu. (3) Kết quả luôn
là sản phẩm `draft`, `ai_assisted = true`, loại `physical`; danh mục chỉ nhận ID trong cây của khách;
thuộc tính chỉ giữ giá trị đúng kiểu, thuộc danh mục đã chọn (kể cả tổ tiên) — còn lại để trống,
`readiness` báo thiếu; ảnh công khai thành ảnh sản phẩm. (4) Khác đặc tả: nhóm thuế là trường bắt buộc
của yêu cầu (AI không chọn — thuộc về thuế, D-186); SKU là mã tạm `AI-…` vì cột SKU bắt buộc và duy nhất;
tính năng gói dùng chung `catalog.ai` thay vì thêm `catalog.ai_from_images` (một công tắc AI của
`catalog`). (5) Khối `catalog.product_grid` / `product_card` / `collection` chờ sổ khối của bước 67.

Lý do: dùng lại đường chuẩn bị ảnh cho AI sẵn có của `media` (bỏ EXIF, thu nhỏ) thay vì để `catalog` đọc
nơi lưu của module khác; job vì gọi mô hình với nhiều ảnh lâu hơn một request. Nháp + `ai_assisted` theo
D-109 và Luật 134/2025/QH15 (cần đối chiếu văn bản gốc).

Đã cân nhắc: nhận `file_ids` của `files` (sản phẩm không tự có ảnh); để AI chọn nhóm thuế (sai thuế là rủi
ro pháp lý); thêm cột SKU cho phép rỗng (đổi hợp đồng `VariantSnapshot`, ảnh hưởng mọi module thương mại).
