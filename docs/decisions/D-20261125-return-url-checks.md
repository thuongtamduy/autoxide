## D-20261125-return-url-checks · `return_url` qua `orders`: hai lý do tách riêng; dấu vân tay idempotency tính trên URL đã chuẩn hoá — 2026-11-25 — agent quyết

Chủ đề: Tiền, thuế, chứng từ

Quyết định: bổ sung D-20261125-return-url-allowlist.

1. Checkout và `payment-link` của `orders` giữ đúng lý do ledger từ chối ô `return_url`: mã ô `format` (không phải URL
   http(s), vd `ftp://…`, `//host`) → `orders.return_url_invalid`; mã `ledger.return_url_not_allowed` →
   `orders.return_url_not_allowed`. Hai mã, hai câu đúng nghĩa.
2. Dấu vân tay idempotency của `create_payment` tính trên `return_url` đã chuẩn hoá (chuỗi đã kiểm, cũng là chuỗi lưu
   và gửi đối tác). Hệ quả chấp nhận: một request ghi TRƯỚC bản có kiểm `return_url`, gửi lại cùng `Idempotency-Key`
   với URL mà chuẩn hoá làm đổi chuỗi (chữ hoa trong tên máy chủ, cổng mặc định, `%2e`…), bị báo "key đã dùng với thân
   khác". Trường hợp hẹp (chỉ lượt gửi lại vắt qua lúc nâng cấp), client sửa bằng key mới.

Lý do: gộp "sai định dạng" vào "không thuộc cửa hàng" khiến người tích hợp đi khai origin cho một URL vốn sai; tính dấu
vân tay trên chuỗi gốc thì hai URL cùng trỏ một trang lại thành hai lượt thu khác nhau.

Đã cân nhắc: tính dấu vân tay trên chuỗi gốc chỉ để giữ tương thích cho lượt gửi lại vắt qua lúc nâng cấp — đổi lấy sai
lệch lâu dài giữa thứ được kiểm và thứ được so.
