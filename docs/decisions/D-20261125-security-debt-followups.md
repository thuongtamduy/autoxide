## D-20261125-security-debt-followups · Nợ còn mở sau loạt trả nợ bảo mật mức thấp gom vào mục P16 của lộ trình — 2026-11-25 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: thêm mục P16 (bước 70) gom ba việc còn mở sau #505, #508, #512: bổ sung `org_id` cho event của các module
liệt kê trong D-20261125-event-org, L-11 (`return_url` của xác minh danh tính) và một test chắc chắn đi qua nhánh
"trùng unique index" khi xác minh tên miền. P6 giữ `[x]` vì việc rà soát và các mục nợ của nó đã xong.

Lý do: để mục con `[ ]` dưới một mục `[x]` làm lệch cách đánh "xong" của lộ trình; ba việc này phát sinh khi sửa, không
phải phần của rà soát ban đầu.

Đã cân nhắc: mở lại P6 (`[ ]`) — báo sai rằng việc rà soát chưa xong.
