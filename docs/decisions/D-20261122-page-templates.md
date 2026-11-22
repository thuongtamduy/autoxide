## D-20261122-page-templates · Trang mẫu theo ngành: bộ mẫu nhúng vào binary, khách cài vào bảng của mình, tạo trang bỏ khối không dùng được — 2026-11-22 — agent quyết

Chủ đề: Module nội dung, file, liên lạc

Quyết định (bước 67 mục "thư viện trang mẫu theo ngành", chi tiết hoá mục 4 của `docs/steps/67-page-builder.md`):

1. Bộ mẫu đi kèm bản phát hành là file `modules/content/templates/pages/<ngành>.json` (bán lẻ, ăn uống, dịch vụ,
   sự kiện, tuyển dụng), mỗi file một mẫu có tên, mô tả và bố cục cho `en` và `vi`, nhúng vào binary; test kiểm mọi
   file đọc được và mọi mẫu tạo được trang hợp lệ qua bộ kiểm thật.
2. **Không có dòng `tenant_id` NULL** cho mẫu dùng chung như bảng dữ liệu của đặc tả: khách cài bộ mẫu vào bảng
   `content_page_templates` của mình bằng `POST /page-templates/presets` (giống bộ vị trí banner — D-236), mỗi mẫu
   một dòng mỗi ngôn ngữ khách dùng, chạy lại không trùng (`preset_key` + `locale`). Nhờ vậy mọi dòng có
   `tenant_id`, lọc theo khách như mọi bảng (không cần ngoại lệ "mặt điều khiển"), danh sách là `Page` thường.
3. Khách lưu trang bố cục thành mẫu riêng (`POST /page-templates`, giữ nút và ngôn ngữ của trang).
   `POST /entries/from-template` tạo trang **nháp** chép bố cục, bỏ khối khách không dùng được (module tắt, gói
   thiếu tính năng) và trả tên các khối đã bỏ (`dropped_blocks`).

Lý do: một cách lưu cho mọi mẫu, không thêm đường đọc chéo khách; khách sửa hay xoá mẫu cài sẵn của mình được.

Đã cân nhắc: dòng `tenant_id` NULL (phải thêm ngoại lệ cho gate `tenant-scope` và mặt điều khiển, mẫu dùng chung
không xoá riêng được); đọc thẳng file nhúng mỗi lần liệt kê (hai nguồn trong một danh sách, khó phân trang).
