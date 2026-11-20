## D-20261120-catalog-csv-ai-drafts · Nhập / xuất CSV và bản nháp AI của `catalog` — 2026-11-20 — agent quyết

Chủ đề: Thương mại

Quyết định: (1) `POST /imports` nhận thân là chính file (`Content-Type: text/csv`), không multipart; file
lưu vào `dyn Storage` của khách, job chạy với `PrincipalSnapshot` của người yêu cầu và đi qua đúng
service của REST (cùng kiểm tra, hạn mức, event). (2) Một dòng một biến thể, khớp theo SKU: SKU đã có thì
sửa nội dung ở `locale` (giữ slug), mã vạch, cân nặng, nhóm thuế; SKU mới thì tạo sản phẩm **nháp** một
biến thể. Nhập không đổi trạng thái bán; sản phẩm nhiều biến thể / tuỳ chọn hoàn thiện qua API (hay
`imports`, bước 56). (3) Lỗi theo dòng lưu `{row, field, code, message}`, tối đa 1 000. (4) Giới hạn 50 000
dòng / 20 MiB và hạn giữ file 7 ngày là hằng số, không phải biến cấu hình. (5) Lần nhập / xuất chỉ người
bắt đầu xem và tải. (6) Đọc lại file đã xuất bỏ dấu `'` mà bộ ghi CSV thêm trước ô dạng công thức. (7)
Gợi ý mô tả / SEO bằng AI lưu thành bản nháp `catalog_suggestions`; áp mới ghi vào bản dịch và bật
`ai_assisted` (D-109); kiểm quyền ghi trước khi gọi mô hình, gọi mô hình ngoài transaction.

Lý do: Thân là file thì agent gửi được bằng một lệnh, không cần dựng multipart; đi qua service của REST
giữ một cách kiểm cho mỗi việc. Khớp theo SKU làm nhập lại an toàn khi agent retry. Ít biến cấu hình hơn
mà mặc định đã hợp lý cho người tự cài. Xuất chứa dữ liệu theo phạm vi của người tạo nên không chia cho
người khác.

Đã cân nhắc: multipart (khó cho máy, không thêm gì); dòng theo sản phẩm có cột `options` (định dạng tự chế
khó kiểm, để cho `imports` theo định dạng của từng nền tảng); `MAX_IMPORT_ROWS` / `MAX_IMPORT_BYTES` là biến
(chưa ai cần đổi).
