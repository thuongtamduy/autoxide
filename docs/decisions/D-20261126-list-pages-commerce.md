## D-20261126-list-pages-commerce · Danh sách cụm 3b: catalog, compliance, orders, promotions — 2026-11-26 — agent quyết

Chủ đề: HTTP, REST, OpenAPI

Quyết định: ghi lại các lựa chọn của PR đại trà cụm 3b (bước 70 P9 mục 3), bổ sung D-20261125-list-pages:

1. **Keyset trong database** cho mọi danh sách nằm trong bảng của khách: bảo hành, hoàn tiền, chứng từ của đơn, luật
   và danh sách chặn COD (`orders`); tham gia, số liệu theo ngày, phiên bản (`promotions`); văn bản, đồng ý của tôi, số
   chấp nhận theo phiên bản — nay `GROUP BY` trong SQL thay vì đếm trong bộ nhớ (`compliance`); thuộc tính, tài liệu
   sản phẩm, tài sản số, luật làm tròn, cây danh mục quản trị và công khai — keyset ba cột (depth, position, id), điều
   kiện "mọi tổ tiên hiện và có tên ở ngôn ngữ này" thành subquery trên `catalog_category_paths` (`catalog`). Báo cáo
   bán hàng gom theo cửa hàng / kênh / sản phẩm: keyset trên tổng bằng `HAVING` + `text_key_cursor("sales:<nhóm>")`.
   Migration index mới: catalog m0021, compliance m0022, orders m0017, promotions m0009; index cũ thành tiền tố thì xoá,
   `down` tạo lại.
2. **`page_of`, có lý do:** gói luật dùng chung trên control plane, như bảng thuế (`promotions` legal-limits và
   notice-rules, `compliance` rules, `catalog` restriction-rules); tập cố định trong code (`compliance` policies,
   retention, tracking-providers; `catalog` products/counts); mặc định luật gộp mục đích của khách (`compliance`
   purposes, public/purposes); kiểm "còn phải chấp nhận" trong code, tối đa một văn bản mỗi loại
   (`me/pending-acceptances`); luật xung đột chỉ viết được bằng code (`promotions/{id}/conflicts`); giá tính bởi bộ máy
   khuyến mãi mỗi lần gọi, tối đa 3 mã mỗi chương trình (`public/promotions/for-product`); quyền tải của một đơn (số dòng
   × số file, kiểm từng quyền trong code — `catalog/orders/{id}/downloads`); xếp hạng có trần theo thiết kế (similar ≤ 12,
   frequently-bought-together ≤ 20, search/suggest ≤ 10); báo cáo bán hàng theo ngày / tháng (khoảng tối đa 366 ngày).
   Giới hạn đã biết: báo cáo theo ngày / tháng vẫn đọc mọi đơn của khoảng để gom trong Rust (như trước), chỉ câu trả lời
   chia trang.
3. **Ngoại lệ chờ duyệt** (giữ dòng `api-unpaged-list` kèm lý do): `GET /api/v1/catalog/variants?code=` — tra theo mã
   client gửi, tối đa một biến thể theo SKU và một theo mã vạch.
4. Tool MCP: `compliance` thêm `NaturalPageInput` (cùng tham số với `NaturalOrderPageQuery`) cho các tool danh sách theo
   thứ tự tự nhiên; mọi tool danh sách của bốn module nhận `count`.
5. Thứ tự đổi có chủ đích (changelog `p9-list-pages-cluster-3b.changed.md`): nhiều danh sách mới nhất trước; thuộc tính
   theo thứ tự của lần `PUT` cuối (trước: theo ngày hiệu lực — cột `Date` không làm khoá keyset được); `limit` cũ của
   gợi ý / tìm nhanh thay bằng tham số trang chung.

Lý do: giữ một hình dạng danh sách cho mọi endpoint mà không mất dòng nào trong im lặng; chỗ nào thứ tự hay bộ lọc chỉ
tính được bằng code thì dùng `page_of` trên tập có trần theo cấu trúc, ghi rõ trần.

Đã cân nhắc: chuyển báo cáo theo ngày / tháng sang bảng tổng hợp có index — hoãn: thay đổi lớn ngoài phạm vi chia
trang; giữ `limit` riêng 1–12 của gợi ý — loại: hai nghĩa của `limit` trên cùng một API.
