## D-20261126-automation-owner-revoked · Luật tự động dừng khi tài khoản `run_as` bị cắt cửa vào, và không có hơn quyền lúc tạo — 2026-11-26 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định:

1. Kernel thêm hợp đồng event `autoxide_kernel::accounts::ACCESS_REVOKED` (`auth.access_revoked`, `after.user_id`,
   `after.reason`), như `privacy::USER_DELETED`. `auth` ghi nó trong `user::revoke_durable_grants_in` — chỗ chung của
   mọi đường cắt cửa vào tài khoản (D-20261125-durable-grants): khoá hết cửa vào (`locked_out`: SCIM đổi email, "không
   phải tôi", nhận lại tài khoản chưa xác minh bằng link), lấy lại bằng mail (`recovered`), đổi mật khẩu kèm thu hồi mọi
   thứ (`sessions_revoked`), khoá tài khoản qua REST hay SCIM (`disabled`), xoá (`deleted`) — cùng transaction.
2. `automations` nghe event đó (không phụ thuộc module `auth`, như `auth.api_key_revoked`): mọi luật đang bật có
   `run_as` là tài khoản đó tạm dừng, lý do `owner_access_revoked`. Bật lại là việc của một người có quyền (người bật
   thành `run_as`, luật được kiểm lại với quyền của họ) — mở khoá tài khoản không tự bật lại luật.
3. Luật tạo / sửa / bật qua phiên đăng nhập nay cũng chụp quyền của người làm vào `run_as_limit` (trước đây chỉ API key):
   luật chạy với quyền HIỆN TẠI của `run_as` nhưng không bao giờ hơn quyền lúc tạo / sửa / bật — như lời mời (sức mạnh ≤
   min(lúc tạo, lúc dùng)). Giới hạn theo TÊN permission (`Principal::only`): phạm vi là phạm vi hiện tại của `run_as`
   — chụp cả phạm vi là việc sau (ROADMAP P16). Luật cũ (cột NULL) giữ cách cũ tới lần sửa / bật kế tiếp (changelog
   ghi "Action needed").
4. Lần chạy dở không chạy tiếp bằng quyền của người khác, hai lớp:
   - Luật rời `active` (tạm dừng vì bất kỳ lý do nào, tắt, chuyển sang chờ duyệt): trong CÙNG transaction, mọi lần chạy
     `queued` / `waiting` / `awaiting_approval` của nó kết thúc với `automations.rule_not_active` (event
     `automations.run_failed` như lần chạy hỏng khác). `pause` nay luôn chạy trong transaction (trạng thái luật, lần chạy
     dở, event ghi cùng lúc).
   - Runner, trước khi chạy VÀ trước mỗi bước, đọc luật qua kết nối ghi: luật phải còn `active` và còn đúng
     `version` của lúc tạo lần chạy (`automations.rule_changed` nếu không). Bật lại luật nay tăng `version` như sửa luật;
     mọi lần đổi `run_as` đều đi qua sửa hay bật lại, nên "cùng phiên bản" kéo theo "cùng `run_as`" — không cần lưu
     `run_as` trên lần chạy (không thêm migration).

Lý do: D-20261125-durable-grants để lại mục này cho `automations` (P16): luật chạy bằng quyền hiện tại của `run_as`,
nên luật dựng trong lúc tài khoản bị người lạ dùng vẫn chạy sau khi chủ lấy lại tài khoản (khoá tài khoản thì luật đã
dừng ở lần chạy kế tiếp, nhưng lấy lại bằng mail hay khoá hết cửa vào thì tài khoản vẫn hoạt động). Một event hợp
đồng thay vì đọc bảng của `auth` giữ luật kiến trúc (module không đọc bảng của nhau); đặt ở `revoke_durable_grants_in`
thì đường khoá mới nào gọi hàm đó cũng tự có.

Đã cân nhắc: dừng luật lúc chạy bằng cách hỏi `auth` "tài khoản có vừa bị khoá không" (cần trait mới, và luật vẫn
chạy giữa lúc khoá và lúc kiểm); xoá luật (mất cấu hình của chủ thật); tự bật lại khi mở khoá (người lạ dựng luật thì
chủ phải xem lại trước).
