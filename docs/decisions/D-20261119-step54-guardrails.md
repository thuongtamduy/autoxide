## D-20261119-step54-guardrails · Bước 54: rào quyền tối đa cho khách con — 2026-11-19 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: Khách cấp trên đặt cho một khách cấp dưới danh sách permission tối đa (`PUT /api/v1/auth/guardrails/
{tenant_id}`, permission `auth.guardrails.manage`, `risk = destructive`). Chỉ đặt được từ tài khoản của chính khách đó,
không qua uỷ quyền hay hỗ trợ. Rào áp cho khách đó và mọi khách dưới nó, cho mọi role kể cả `admin`, và cả người từ
khách khác vào bằng uỷ quyền hay hỗ trợ. Quyền thật là giao của quyền theo role với **mọi** rào trên đường từ gốc,
nên khách con không nới được rào cha đặt. Kernel thêm `Principal::within`. Permission bị rào cắt trả
`permission_denied` với `reason = outside_guardrail`; nếu một role cũng cấm permission đó thì lý do vẫn là
`denied_by_role`.

Rào lưu ở bảng `auth_tenant_guardrails` của database nền tảng (migration `m0041`), một dòng mỗi cặp khách bị rào /
khách đặt rào. Bảng nằm trong module `auth`, không phải kernel; phiên giữ vùng tenancy đã đồng ý. Đọc rào của cả
chuỗi tổ tiên bằng `DatabaseRegistry::tenant_ancestors` cộng một câu truy vấn. Kết quả cache theo khách 30 giây và
xoá ngay trên máy đổi rào. Chế độ một khách không đọc gì. Tách / gộp khách: bảng khai `ephemeral`, vì rào nằm
ở database nền tảng theo ID khách, không đi cùng dữ liệu của khách. Thay vào đó, `auth` nghe
`kernel.relocation_started`. Khi **tách**, khách mới nhận bản sao mọi rào đang áp cho khách nguồn (của chính nó và
của tổ tiên), giữ nguyên khách đặt rào. Nhờ vậy khách tách ra làm khách gốc (không giữ cha) không thoát rào, và
khách đặt rào vẫn sửa / gỡ được rào đã chép dù không còn ở trên khách mới. Khi **gộp**, khách nhận giữ rào của nó;
khách bị gộp mất cùng rào của nó. Dữ liệu của khách bị gộp từ nay theo rào của khách nhận, kể cả khi rào cũ của
khách bị gộp chặt hơn: người gộp là nhân sự nền tảng, quyết điều đó khi gộp.

Khe hở còn mở, đóng ở PR kế tiếp: event `kernel.relocation_started` chỉ được giao khi khách nguồn phục vụ lại, nên
khách mới có thể mở vài giây trước khi nhận rào. Đó là một lỗ leo quyền. Cách đóng: kernel thêm một hook đồng bộ vào
cuối trait `Module` (cài đặt mặc định không làm gì, D-038), chạy trong bước tách **trước** khi khách mới được mở.
Hook nhận loại lượt, khách nguồn, khách mới và database nền tảng. Hook lỗi thì lượt tách dừng, khách mới không mở.
`auth` chép rào trong hook này; event giữ lại làm lưới an toàn (chạy lại được). Test bắt buộc: hook lỗi thì khách mới
không mở, trên cả SQLite và Postgres. Khách bị xoá hẳn thì nghe `kernel.tenant_deleted` để xoá rào đặt cho nó và rào nó đặt.

Lý do: Đặc tả 54 P4, như SCP của AWS. Rào là trần, không phải quyền cấp thêm, nên giao các rào là đủ và dễ giải
thích. Cache 30 giây đánh đổi thu hồi tức thì trên máy chủ khác lấy việc không thêm truy vấn ở đường nóng. Rào hiếm
khi đổi, và trên máy chủ đổi rào thì có hiệu lực ngay.

Đã cân nhắc: (1) Bảng ở kernel tenancy (`kernel_tenant_guardrails` như đặc tả). Bị loại vì cần migration kernel
trong vùng của phiên khác, trong khi chỉ `auth` dựng `Principal`. (2) Lưu rào ở database của khách cha. Bị loại vì đọc
chuỗi tổ tiên sẽ tốn một câu mỗi database. (3) Không cache. Bị loại vì mỗi request thêm một tới mười mấy câu
(`tenant_ancestors` đọc từng tầng).

Lộ trình: mục 2 của bước 54 đánh dấu xong. Phần "API key chọn bản ghi cụ thể" tách thành mục `[ ]` riêng và làm cùng
mục chia sẻ bản ghi (P3), vì nó dùng chung bảng `kernel_shares` với `subject_kind = api_key`.
