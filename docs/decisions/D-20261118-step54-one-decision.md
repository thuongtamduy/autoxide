## D-20261118-step54-one-decision · Bước 54 mục 1: một hàm quyết định quyền; cache theo phiên bản quyền tách thành mục riêng, chờ số đo — 2026-11-18 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: Luật quyền trên bản ghi viết **một lần** trong `Reach` (`crates/kernel/src/security/access/decision.rs`):
điều kiện SQL cho danh sách (`Principal::scope_condition`) và kiểm từng bản ghi (`Principal::check`, trả `Decision`
`Allow` / `Missing` / `OutOfScope`; `require_at` đổi thành lỗi) đều sinh từ nó. Test
`crates/kernel/tests/access_check.rs` sinh ngẫu nhiên cây tổ chức, bản ghi, grant và hàng rào API key (3 hạt cố định,
30 người gọi mỗi hạt, có và không có cột người sở hữu) và đòi hai đường cho cùng tập bản ghi trên cả hai database.
Thu hồi tức thì đã có từ trước (quyền không nằm trong token, nạp mỗi request bằng một câu — D-319), nay có test
`doi_role_co_hieu_luc_o_request_ke_tiep`. **Cache quyền theo `access_version` của khách (P11) tách thành một mục
`[ ]` riêng của bước 54**, chỉ làm khi benchmark của bước 39 cho thấy câu nạp quyền đáng kể.

Lý do: Hai đường viết tay riêng là chỗ dễ sai nhất mà đặc tả 54 nêu ("viết luật hai nơi rồi lệch nhau"). Test
ngẫu nhiên đã bắt được lỗi cố ý cài vào (bỏ hàng rào ở đường kiểm từng bản ghi) ngay ở vòng 1. Về cache: muốn
thu hồi tức thì khi chạy nhiều tiến trình thì mỗi request vẫn phải đọc `access_version` từ database, nên số vòng
mạng tới database không giảm (vẫn một câu). Cái được chỉ là đổi một câu JOIN bốn bảng có index sang một câu đọc
theo khoá chính, mà cái giá là phải tăng phiên bản ở MỌI chỗ đổi quyền (role, permission của role, gán role, khoá
tài khoản, MFA / passkey vì role bắt buộc MFA, API key, uỷ quyền, lần vào hỗ trợ). Sót một chỗ là một lỗ hổng:
quyền đã thu hồi vẫn còn hiệu lực. Luật "đo trước rồi mới tối ưu" của dự án nên chờ số đo.

Đã cân nhắc: (1) Làm cache ngay theo đặc tả. Bị loại vì số vòng mạng không giảm và rủi ro sót chỗ tăng phiên bản.
(2) Giữ phiên bản trong bộ nhớ của tiến trình, không đọc database. Bị loại vì chạy nhiều tiến trình thì mất thu hồi
tức thì. (3) Trigger của database tự tăng phiên bản. Bị loại vì cú pháp trigger khác nhau giữa SQLite và Postgres.
