## D-20261125-miniapp-platform · Autoxide là nền cho nhà phát triển dựng mini app và website; bước 65 là bước đầu tiên sau 1.0 — 2026-11-25 — chủ dự án quyết

Chủ đề: Phạm vi và mục tiêu

Quyết định:

1. **Đích của sản phẩm:** Autoxide là nền để nhà phát triển dựng mini app và website trên đó, theo kiểu Zalo Platform
   hay Mini App của MoMo: họ dùng tài khoản, phân quyền, thanh toán, thông báo… của core thay vì tự làm lại.
2. **Hai tầng mở rộng, không thay nhau:** tầng trong là module Rust biên dịch chung vào server (người được tin cậy
   viết, toàn quyền với dữ liệu của module — bước 70 P2); tầng ngoài là ứng dụng bên ngoài viết bằng ngôn ngữ bất kỳ,
   chỉ gọi được API trong các quyền khách đã đồng ý (bước 65). "Mini app" của nhà phát triển bên thứ ba là tầng ngoài.
3. **Thứ tự:** vẫn ổn định core và ra 1.0 trước (D-20261121-community-first không đổi). **Bước 65 là bước đầu tiên
   được mở sau 1.0**, trước các bước tạm dừng khác (46, 48, 49, 56, 59, 62, 63, 64).
4. **Bước 65 thêm phần mini app** (ba mục mới trong lộ trình, đặc tả ở `docs/steps/65-apps.md` mục "Mini app"): bản
   mô tả mini app máy đọc được; giao diện mini app hiển thị trong sản phẩm của người vận hành kèm SDK cho giao diện
   đó; hướng dẫn cho nhà phát triển mini app.
5. **Không tính năng nào giả định một kiểu vận hành.** Cùng một phần mềm phải chạy được cho một công ty tự vận hành
   nền của họ và cho một dịch vụ nhiều khách; một mini app viết một lần phải cài được trên mọi bản cài Autoxide cùng
   phiên bản lớn. Chợ ứng dụng thuộc về từng bản cài (người vận hành tự duyệt); core không phụ thuộc một danh bạ
   trung tâm nào.
6. **Core chỉ có API, giữ nguyên** (luật kiến trúc số 9, D-096). Nền cho mini app không mang giao diện: bước 65 chỉ
   đưa ra hợp đồng và SDK để giao diện ở repo riêng dùng.

Lý do: chủ dự án nêu đích này ngày 25/11/2026. Lộ trình đang dựng kỹ tầng trong, còn thứ làm nên một nền tảng cho
nhà phát triển bên ngoài (OAuth, cài / đồng ý / gỡ, webhook theo ứng dụng) nằm ở bước 65 đang tạm dừng và mới chỉ tả
phía server. Nền tảng cho bên thứ ba cần hợp đồng API không đổi, nên 1.0 vẫn phải đi trước; nhưng ngay sau 1.0 thì
bước 65 quan trọng hơn mọi module nghiệp vụ còn chờ.

Đã cân nhắc: mở bước 65 trước 1.0 — loại, ứng dụng bên ngoài viết trên API chưa cam kết sẽ vỡ ở mỗi bản. Coi module
Rust là "mini app" — loại cho bên thứ ba: phải viết Rust, phải build lại server, và chạy với toàn quyền. Cài mini app
vào server đang chạy bằng plugin nạp lúc chạy — đã loại từ trước (D-005, D-026). Module mang giao diện của chính nó
và đóng gói cả giao diện vào một binary — loại: chủ dự án khẳng định lại ngày 25/11/2026 rằng **core chỉ có API**
(luật kiến trúc số 9, D-096), không đổi và không để ngỏ; giao diện của mini app và của người vận hành luôn ở repo riêng.

Thay: phần thứ tự các bước tạm dừng sau 1.0 của D-20261121-community-first (bước 65 lên đầu)
