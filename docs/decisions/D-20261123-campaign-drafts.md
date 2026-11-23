## D-20261123-campaign-drafts · Bộ nháp chiến dịch gọi `promotions` / `newsletter` qua sổ tool MCP; hạn mức AI trang theo gói — 2026-11-23 — agent quyết

Chủ đề: MCP, AI, agent dùng API

Quyết định (bước 67 mục "bộ nháp chiến dịch", chi tiết hoá mục 7 của `docs/steps/67-page-builder.md` và phần để
sau của D-20261122-ai-pages):

1. `POST /api/v1/content/campaign-drafts` `{promotion_id, org_id?, locale?, channels: [page|banner|newsletter],
   instruction?, banner_placement?, newsletter_list_id?}` → 202, một dòng `content_campaign_drafts`, job
   `content.campaign_drafts`; `GET /campaign-drafts/{id}` trả kết quả từng kênh (`kind` + `id` của bản nháp hay
   `error_code`); tool MCP `content.create_campaign_drafts` / `content.get_campaign_drafts` cùng service. Chỉ từ
   chương trình khuyến mại; bộ sưu tập (`collection_id`) để khi có nhu cầu.
2. `content` không phụ thuộc crate `promotions` / `newsletter` (cả hai đăng ký sau `content`): đọc chương trình bằng
   tool `promotions.get_promotion`, tạo chiến dịch bằng tool `newsletter.create_campaign`, qua
   `AutomationDirectory::call` với quyền đã chụp của người yêu cầu — cùng kiểm quyền, gói, khách chỉ đọc như `/mcp`
   (luật kiến trúc 8; tên tool là hợp đồng công khai). Module chưa cài → 503 `content.campaign_module_unavailable`.
3. Mỗi kênh là bản nháp ở module của nó, không tự đăng / gửi (D-109): trang đích = trang nháp mới + một lượt AI tạo
   trang (đường của D-20261122-ai-pages, khối `promotions.flash_sale` / `voucher_claim`); banner = banner chữ nháp ở
   vị trí `banner_placement`, link tới `promotions.promotion` (tự lưu trữ khi chương trình kết thúc); chiến dịch =
   nháp `advertising: true` (thư quảng cáo, Nghị định 91/2020/NĐ-CP — cần đối chiếu văn bản gốc).
4. Chữ banner và thư: một lần gọi mô hình theo JSON Schema; mô hình chỉ nhận tên, điều kiện, quy tắc của chương
   trình (bỏ mọi ID, bộ đếm; tiền thành dạng đọc được, `*_bp` thêm phần trăm). Mọi số trong chữ phải có trong dữ liệu
   đó (bỏ dấu phân cách hàng nghìn), không thì gọi lại kèm lỗi, tối đa 2 lần; vẫn sai → kênh đó
   `content.campaign_copy_invalid` — không bịa ưu đãi (Luật 19/2023/QH15, Luật Quảng cáo 16/2012/QH13 — cần đối chiếu).
5. Hạn mức `content.ai_page_generations` (`Metered`, mỗi tháng, `record_usage` ngoài transaction): mỗi yêu cầu AI
   tạo / sửa trang và mỗi bộ nháp chiến dịch tính một lần (lượt trang bên trong bộ nháp không tính lại), kể cả khi kết
   quả bị từ chối — mô hình đã được gọi. Hết hạn mức → 429 `quota_exhausted`.

Lý do: dùng lại đúng đường mà agent ngoài đi (tool) thì không cần thêm `api` mới cho `newsletter` hay phụ thuộc
ngược thứ tự đăng ký, và không làm được gì mà người yêu cầu không tự làm được; kiểm số là chặn rẻ nhất cho lỗi nặng
nhất của chữ quảng cáo (hứa sai ưu đãi).

Đã cân nhắc: thêm trait `CampaignDrafts` vào `api` của `newsletter` + `Later<dyn …>` (thêm hợp đồng Rust phải giữ
từ 1.0 chỉ cho một bên gọi); để AI viết số tự do rồi người duyệt bắt lỗi (dễ lọt); tính hạn mức theo số lần gọi mô
hình (khó đoán trước cho người dùng).
