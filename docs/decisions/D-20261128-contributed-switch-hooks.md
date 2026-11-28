## D-20261128-contributed-switch-hooks · Hook đóng góp chạy sau hook cũ; module nghỉ không có hook đóng góp; `contribute` thiếu `dyn` bị chặn — 2026-11-28 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định: Bổ sung D-20261128-contribute ba điểm. (1) Hook của bước chuyển khi tách / gộp khách đóng góp bằng
`contribute::<dyn SwitchHook>` chạy **sau** mọi hook khai bằng `Relocation::on_switch`, theo thứ tự đăng ký module; module
vừa `on_switch` vừa đóng góp thì khởi động lỗi. (2) Module đang nghỉ (D-20261125-dormant-modules) không chạy `start` nên
**không** có hook đóng góp; bảng của nó vẫn đi theo chính sách tách / gộp như trước. Module chuyển hook từ `on_switch` sang
`contribute` (vd `auth` ở cụm 1) phải kiểm lại: hook cũ có chạy khi module nghỉ không — nếu có và cần giữ, phải ghi quyết
định mới trước khi chuyển. (3) `StartContext::contribute::<T>` với `T` là kiểu cụ thể (gọi thiếu `::<dyn Trait>`) trả lỗi
khởi động: con trỏ tới kiểu cụ thể là con trỏ mỏng, tới `dyn Trait` là con trỏ béo — so kích thước là đủ, không cần
specialization; trong repo không có đóng góp nào theo kiểu cụ thể.

Lý do: kiểm độc lập #623 nêu: thứ tự hook và hành vi với module nghỉ chưa ghi; gọi `contribute(Arc::new(x))` thì trình biên
dịch suy ra kiểu cụ thể và đăng ký mất im lặng — lỗi mà agent dễ mắc nhất.

Đã cân nhắc: chặn bằng kiểu lúc biên dịch (trait đánh dấu cho `dyn Trait`) — Rust chưa có cách viết "chỉ nhận kiểu không
kích thước"; dùng `type_name` bắt đầu bằng `dyn ` — định dạng không được Rust cam kết; chạy hook đóng góp trước hook cũ — hook
cũ có trước, đổi thứ tự là đổi hành vi của module chưa chuyển.
