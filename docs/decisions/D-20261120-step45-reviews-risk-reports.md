## D-20261120-step45-reviews-risk-reports · Đánh giá sau mua đặt ở `orders`, phiếu "hữu ích", tóm tắt AI chờ duyệt, chống bom hàng COD theo khoá liên hệ, báo cáo bán hàng truy vấn thẳng — 2026-11-20 — agent quyết

Chủ đề: Thương mại

Bối cảnh: các mục "Đánh giá sau mua", "Nút Đánh giá này hữu ích", "Báo cáo bán hàng", "Chống bom hàng COD" của bước 45
(D-194, D-201, D-202, D-203, D-207, D-213, D-223). Đặc tả để agent quyết: đặt đánh giá ở `comments`, module `reviews` hay
`orders`; bảng tổng hợp báo cáo; ngưỡng mặc định của quy tắc COD.

Quyết định:

1. **Đánh giá ở `orders`** (`reviews.rs`, bảng `orders_reviews`, `orders_review_votes`, `orders_review_summaries`,
   `orders_review_stats`): cần dữ liệu "đã nhận hàng" của chính `orders` (dòng đơn của đơn `delivered` / `completed`),
   có số sao và thống kê, và không được qua bộ tự kiểm duyệt / chặn spam tự ẩn của `comments` (Luật 19/2023 cấm chặn /
   ẩn đánh giá trái quy định). Mỗi dòng đơn một đánh giá (unique), đăng ngay, sửa trong `AUTOXIDE_ORDERS_REVIEW_EDIT_DAYS`
   (30). Khách bật xác thực tài khoản đăng nội dung (NĐ 147/2024) thì kiểm qua `PostingVerification` của kernel.
2. **Ẩn chỉ qua `POST /reviews/{id}/moderation`** với lý do thuộc danh sách `orders.review_hidden` (mã `legal`), ghi người
   ẩn, lý do, giờ; event `orders.review_moderated`; báo người viết qua `notifications` (loại `orders.review_hidden`); mở
   lại được, phiếu giữ nguyên. Không có đường ẩn nào khác.
3. **Thống kê sao** ở `orders_review_stats`, đổi bằng `UPDATE … ± n` cùng transaction với đánh giá (chỉ đánh giá
   `published`); trung bình tính bằng số nguyên (một chữ số thập phân).
4. **Phiếu "hữu ích"** (D-203, chủ dự án quyết): `PUT` / `DELETE /public/reviews/{id}/helpful`; chèn phiếu `ON CONFLICT DO
   NOTHING` và tăng / giảm `helpful_count` cùng transaction chỉ khi chèn / xoá được dòng; `voted_by_me` của cả trang một
   câu truy vấn; `sort=helpful` bằng `fetch_page_by` (con trỏ mang cả hai khoá). Chặn tự bỏ phiếu (409) và người có
   `orders.reviews.moderate` ở nút bán (403) — đặc tả nói thêm "quyền quản lý sản phẩm", nhưng `orders` không biết
   permission của `catalog`; người quản lý sản phẩm thường có quyền kiểm duyệt đánh giá ở cùng nút. Throttle
   `AUTOXIDE_ORDERS_REVIEW_VOTES_PER_HOUR`. Xoá tài khoản (`privacy::USER_DELETED`): xoá phiếu (trừ đếm) và đánh giá của
   người đó (trừ thống kê) — nhận lại event không còn gì để trừ.
5. **Tóm tắt AI** (D-109): `POST /products/{id}/review-summaries` ghi bản `pending` + job `orders.summarize_reviews` gọi
   `TextModel` ngoài transaction (luật ở `instructions`, gồm "giữ cân bằng, không bỏ ý kiến tiêu cực"; đánh giá chỉ ở
   `input`), kết quả theo JSON Schema `{summary, themes}` thành `draft`; chỉ bản được duyệt (`approve`) ra công khai, kèm
   `ai_generated: true`; một bản công khai mỗi (sản phẩm, ngôn ngữ) qua cột khoá `approved_key` (unique có NULL).
6. **Nhãn `verified_buyer`** cho hỏi đáp / bình luận trên `catalog.product` qua `context.badge_source`: đọc thẳng
   `product_id` trên dòng đơn của chính `orders` (không cần hỏi `catalog` biến thể của sản phẩm).
7. **Chống bom hàng COD** (D-194): người mua nhận diện bằng khoá liên hệ theo khách (`SecretKey::phone_key`,
   D-20261120-step45-guest-orders) — không lưu số, hai khách hai khoá. Thay bảng đếm `orders_delivery_stats` của đặc tả
   bằng `orders_delivery_outcomes` (một dòng mỗi đơn: `delivered` / `failed` / `refused`, ghi bằng `ON CONFLICT DO UPDATE`
   theo đơn) — event nhận lại không đếm hai lần, đơn giao lại sau khi thất bại đổi kết quả; thống kê là một câu `GROUP BY`
   có index. Quy tắc `orders_cod_rules` có ngày hiệu lực (thêm mới, kết thúc bằng `DELETE`, không sửa), theo nút bán hay
   gốc của khách; **mặc định không bật quy tắc nào**. Danh sách chặn `orders_cod_blocks` có lý do (danh sách
   `orders.cod_block`) và luôn có hạn ≤ 365 ngày. Đặt hàng COD bị chặn / rơi vào quy tắc → 422 `orders.cod_not_allowed`.
8. **Báo cáo bán hàng** `GET /reports/sales` truy vấn thẳng bảng có index, không bảng tổng hợp: `org` / `channel` /
   `product` bằng `GROUP BY` (`CAST(SUM(…) AS BIGINT)` cho hai database), `day` / `month` gom theo ngày UTC trong Rust (hàm
   ngày tháng khác nhau giữa hai database), khoảng ≤ 366 ngày. Đo bằng bộ dữ liệu lớn của bước 39 trước khi thêm bảng tổng
   hợp. Phạm vi theo `orders.reports.read` (`scope_condition`). Xuất đơn qua xuất chung P16 (`orders.list_orders`).

Căn cứ: Luật Bảo vệ quyền lợi người tiêu dùng 19/2023/QH15 và NĐ 55/2024/NĐ-CP (từ 01/7/2024): không chặn / ẩn đánh giá
trái quy định — cần đối chiếu điều cụ thể; NĐ 147/2024/NĐ-CP: xác thực tài khoản đăng nội dung; Luật Trí tuệ nhân tạo
134/2025 và D-109: nội dung AI gắn nhãn, người duyệt chịu trách nhiệm — cần đối chiếu; Luật Bảo vệ dữ liệu cá nhân
91/2025/QH15 và NĐ 356/2025/NĐ-CP: thống kê giao hàng để chống gian lận cần căn cứ xử lý (thực hiện hợp đồng / lợi ích hợp
pháp) ghi trong sổ của `compliance`, chỉ dùng trong khách — **cần người có chuyên môn pháp lý xác nhận**.

Hệ quả: khối `orders.reviews` của trang (bước 67) đọc `GET /public/products/{id}/reviews[/stats]`; `customers` (bước 47)
dùng cùng khoá liên hệ.
