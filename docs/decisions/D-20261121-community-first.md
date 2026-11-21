## D-20261121-community-first · Core cho cộng đồng trước: tạm dừng module mới, bật/tắt module, module viết ngoài repo, hai hạng module — 2026-11-21 — chủ dự án quyết

Chủ đề: Phạm vi và mục tiêu

Quyết định:

1. **Tạm dừng dựng module mới** cho tới sau bản 1.0. Các bước chưa bắt đầu của lộ trình — 46 (sàn), 48 (đa kênh),
   49 (tiếp thị liên kết), 56 (chuyển dữ liệu), 59 (bán sỉ), 62 (bán theo kỳ), 63 (đa tiền tệ), 64 (quảng cáo), 65 (chợ
   ứng dụng) — giữ nguyên đặc tả, không bỏ, nhưng không làm trước 1.0. Bước kế tiếp không bao giờ là một bước tạm dừng.
2. **Bước 70 "Core cho cộng đồng" làm trước mọi bước còn mở**, theo thứ tự:
   - P1: bật/tắt module bằng cấu hình (module tắt không route, không tool MCP, không job, không migration trên
     database mới; dữ liệu đã có thì giữ nguyên).
   - P2: người ngoài viết module trong repo riêng, không fork: crate `autoxide` dùng được như thư viện, ví dụ chạy
     thật trong CI, hướng dẫn; phụ thuộc bằng git tag trước, crates.io khi chủ dự án cấp tài khoản.
   - P3: hai hạng module. **Chính thức** — giữ ổn định, tài liệu đầy đủ. **Thử nghiệm** — ghi rõ "có thể đổi", không
     bật khi không đặt cấu hình. Module mà một module chính thức cần thì phải chính thức; danh sách cuối do agent
     chốt theo phụ thuộc thật và ghi ở `docs/MODULES.md`. Người nâng cấp không được mất chức năng trong im lặng.
   - P4: bản 1.0 có cam kết tương thích (API, mã lỗi, cấu hình, migration, phần kernel mở cho module ngoài) và dòng
     hỗ trợ dài hạn.
   - P5: đăng SDK TypeScript lên npm; kernel và testkit lên crates.io (chờ tài khoản của chủ dự án).
   - P6: kiểm toán bảo mật độc lập trước 1.0 (chờ chủ dự án chọn bên làm).
   - P7: bộ nối nhà cung cấp thật ở Việt Nam — thanh toán, vận chuyển, SMS, hoá đơn điện tử — mỗi bộ nối một module
     nhỏ sau trait của core (chờ chủ dự án chọn đối tác và cấp tài khoản thử). Đây là thứ duy nhất được dựng thêm
     trước 1.0.
3. **Sau P1–P3 của bước 70** mới làm tiếp phần còn dở của các module đã có (đuôi của 51, 68, 41, 43, 45, 55), rồi 54,
   52, 67.
4. **Cách giới thiệu sản phẩm** (README, mô tả repo): backend thương mại sẵn cho thị trường Việt Nam — thuế, hoá
   đơn, địa chỉ hành chính, tuân thủ luật — và AI agent dùng được. Core vẫn trung lập với nghiệp vụ như đã chốt; đây
   là cách giới thiệu, không phải đổi phạm vi của core.

Lý do: sản phẩm đã có hơn 30 module nhưng người thử cài bản 0.17.0 ở mức nâng cao chưa khuyên dùng cho sản phẩm
thật: thiếu cách chỉ chạy phần mình cần, thiếu cam kết ổn định, thiếu đường cho người ngoài mở rộng mà không fork.
Thứ cộng đồng cần là lý do để tin và đường để đóng góp, không phải thêm tính năng. Phần thuế, hoá đơn và địa chỉ
hành chính Việt Nam là điểm các nền tảng nước ngoài không có.

Đã cân nhắc: tiếp tục ba cụm việc thêm tính năng (phân quyền bước 54, sàn bước 46, trang dựng từ khối bước 67) như
kế hoạch trước — loại vì càng nhiều module chưa ổn định thì càng khó cam kết 1.0. Bỏ hẳn các bước tạm dừng — loại:
đặc tả đã viết, sau 1.0 vẫn dùng được, và người ngoài có thể nhận làm khi P2 xong. Mọi module bật mặc định — loại:
người chỉ cần một phần phải chạy và bảo trì cả phần không dùng.

Thay: phần thứ tự làm của D-228 (các bước 46, 48, 49, 56, 59, 62–65 tạm dừng; bước 70 lên đầu)
