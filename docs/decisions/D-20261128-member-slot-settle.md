## D-20261128-member-slot-settle · Hạn mức số nhân sự: đối chiếu với role thật tới khi ổn định — 2026-11-28 — agent quyết

Chủ đề: Đa khách, tổ chức, gói

Quyết định: sau mỗi lần đổi role CÓ thay đổi, `RoleService::settle_member` đọc trạng thái thật (qua kết nối ghi), giữ
+ xác nhận chỗ nếu người đó còn giữ role (giữ lại cùng mã `auth.member:<id>` không giữ thêm), trả chỗ nếu không, rồi
đọc lại; trạng thái đã đổi (lần đổi đồng thời khác vừa commit) thì làm lại, tối đa 4 lượt, còn lệch thì `warn` và job
đối soát của `plans` (`MemberCounter`) sửa. Hết chỗ ở bước giữ sau khi đã ghi role (người giữ role từ trước khi vào gói
chưa có dòng trong sổ, hay chỗ vừa bị lượt kia trả) chỉ `warn`, không làm hỏng lệnh đã commit. Giữ chỗ TRƯỚC khi đổi
cho người sắp vào (`will_hold`) và "không đổi gì thì không đụng chỗ" giữ nguyên (D-20261125-scim-sync-report).

Lý do: hai lần đổi role thật cho cùng một người chạy đồng thời (một gỡ hết, một gán) làm chỗ lệch với role thật — lượt
gán thấy người đó vốn có role nên không giữ, lượt gỡ đọc "không còn role" trước khi lượt gán commit rồi trả chỗ. Hành
động cuối cùng của mọi lượt nay luôn sau mọi commit và được một lần đọc sau nó xác nhận. Test
`hai_lan_doi_role_dong_thoi_khong_lech_cho` đỏ với bản cũ (Postgres), xanh với bản mới trên cả hai database.

Đã cân nhắc: khoá dòng người dùng quanh "giữ chỗ → đổi → xác nhận" — lần đổi mở transaction ghi riêng, giữ thêm một
transaction khoá thì SQLite (một bên ghi) tự chặn mình; chỉ trông vào job đối soát — lệch tới lượt job sau.
