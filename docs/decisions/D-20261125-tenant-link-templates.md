## D-20261125-tenant-link-templates · Link trong thư theo từng khách: mẫu URL đầy đủ, host ràng vào khách — 2026-11-25 — agent quyết

Chủ đề: Đa khách, tổ chức, gói

Quyết định: Mỗi link giao diện trong thư (đặt lại mật khẩu, xác minh email, lời mời, đăng nhập bằng link, báo đổi
email, trang phiếu hỗ trợ, theo dõi khiếu nại, xác nhận báo có hàng lại, huỷ nhận quảng cáo, xác minh / ngừng nhận
bình luận, xác nhận / lựa chọn bản tin) là MỘT tenant setting nhóm `link`, khai bằng `TenantSetting::link(tên,
mô tả, placeholder)` (tên `<module>.link.<việc>`), giá trị là mẫu URL đầy đủ có đường dẫn
(`https://shop.example/dat-lai-mat-khau?t={token}`). Module đọc bằng `SiteLinks::link(tenant, &SETTING,
mẫu_máy_chủ)`: mẫu của khách nếu hợp lệ, không thì cách cũ (mẫu của máy chủ, đổi scheme + host theo
`kernel.public_url`); không có cả hai thì tính năng cần link tắt với riêng khách đó. Kernel cung cấp `SiteLinks`
qua `context.require::<SiteLinks>()` (có tập tên miền đã xác minh); `SiteLinks::new(settings)` vẫn giữ.

Kiểm (setting là đầu vào không tin cậy): chỉ `https`, không user/mật khẩu, không khoảng trắng / ký tự điều khiển /
`\`, không placeholder trong phần host, đủ mọi placeholder module điền, tối đa 2000 ký tự, host đọc như trình duyệt
(crate `url`, WHATWG) phải là host của `kernel.public_url` của khách, một tên miền đã xác minh của khách, host
`AUTOXIDE_HTTP_PUBLIC_URL` hay host của mẫu máy chủ. Kiểm lúc ĐẶT (422 `kernel.link_template_invalid`,
`kernel.link_placeholder_missing`, `kernel.link_host_not_allowed`) và lại lúc DÙNG: tên miền bị thu hồi hay
`kernel.public_url` đổi thì link quay về mẫu máy chủ (ghi warn), không gửi token tới host khách không còn giữ.
Luật gate `site-links` (`cargo xtask check`): mỗi biến `AUTOXIDE_…_URL` hoặc được nêu trong mô tả của một
`TenantSetting::link` cùng crate, hoặc nằm trong danh sách ngoại lệ có lý do (API đối tác, trang của nền tảng).

Lý do: chủ dự án yêu cầu cấu hình theo từng khách cho SaaS; `kernel.public_url` chỉ đổi host nên giao diện của khách
đặt đường dẫn khác (`/dat-lai-mat-khau`) thì link hỏng, và bốn module (support, compliance, inventory,
notifications) còn đọc thẳng biến chung nên thư của khách B trỏ về tên miền chung. Ràng host vào khách: không có
thì admin của khách đặt link sang trang ngoài để lấy token của người dùng mình hay giả trang của khách khác
(phishing / open redirect). Mỗi link một setting thay vì một JSON "bản đồ link": dùng lại sổ setting sẵn có (tên
lạ bị từ chối, mô tả hiện trong `GET /kernel/settings?group=link`, đặt / gỡ từng cái), kiểm placeholder đúng của
từng link ngay lúc đặt.

Rủi ro còn lại: `kernel.public_url` hiện nhận host BẤT KỲ (không đòi tên miền đã xác minh), nên một link mẫu có host
chỉ khớp nhờ `kernel.public_url` do chính admin của khách tự đặt cũng không được ràng chặt hơn `kernel.public_url`:
admin của khách A trỏ được thư của người dùng khách A tới host tuỳ ý (không chạm được người dùng khách khác, vì
mỗi khách chỉ đọc cấu hình của mình). Đóng lỗ này là việc `[ ]` P16 ở lộ trình: ràng `kernel.public_url` vào tên
miền đã xác minh ở chế độ nhiều khách, hoặc cảnh báo.

Đã cân nhắc: một setting JSON `kernel.links` (tên link → mẫu) — phải tự kiểm tên, mô tả không hiện cho admin, một
lỗi làm hỏng mọi link. Chỉ kiểm lúc dùng — admin không biết link mình bị bỏ. Ràng `kernel.public_url` vào tên miền
đã xác minh — phá khách đang đặt giá trị không xác minh được (chế độ một khách không có tên miền); để lại, ghi
`[ ]` ở lộ trình.
