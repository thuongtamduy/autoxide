## D-20261118-step39-key-rotation · Đổi khoá ký access token không đăng xuất ai — 2026-11-18 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: Bước 39 mục A5.

- `auth` thêm `AUTOXIDE_AUTH_TOKEN_SECRET_PREVIOUS` (secret, các khoá cách nhau bằng dấu phẩy, mỗi khoá ≥ 32 ký tự như
  khoá chính). `AccessTokens::issue` vẫn ký bằng khoá hiện tại; `verify` thử khoá hiện tại rồi từng khoá cũ, mỗi lần
  so sánh hằng thời gian (`verify_slice`); khoá dẫn theo khách (bước 33) cũng dẫn từ từng khoá cũ. Định dạng token
  không đổi (không thêm `kid`): token đang nằm ở client vẫn đọc được.
- Quy trình (DEPLOY, EN + VI): khoá mới vào `AUTOXIDE_AUTH_TOKEN_SECRET`, khoá cũ vào `…_PREVIOUS` trên mọi máy chủ,
  khởi động lại, bỏ `…_PREVIOUS` sau thời gian sống của access token. Khoá bị LỘ thì không đưa vào `…_PREVIOUS`.
- Khoá gốc của kernel đã xoay được từ trước (`AUTOXIDE_SECRET_KEY_PREVIOUS` + `autoxide secrets reseal`, mở secret đã
  mã hoá bằng mọi khoá). Token một lần (đặt lại mật khẩu, xác minh email…) là chuỗi ngẫu nhiên lưu bản băm, không phụ
  thuộc khoá nên không cần xoay. Thứ ký bằng khoá dẫn từ khoá gốc mà không kiểm bằng khoá cũ: token chống spam của
  form và link tải bản xuất (sống vài phút tới vài giờ) — đổi khoá gốc thì chúng hết hiệu lực sớm, người dùng lấy cái
  mới; dấu IP băm (D-153) bắt đầu lại — chấp nhận, ghi trong sổ tay.
- Chưa làm: secret của webhook giữ song song secret cũ một khoảng (`AUTOXIDE_WEBHOOKS_SECRET_OVERLAP_HOURS`, gửi hai
  chữ ký) của đặc tả — đó là secret chung với bên nhận, không làm ai bị đăng xuất; để lại cho lần sửa `webhooks` sau.

Lý do: đặc tả bước 39 — đổi khoá định kỳ hay khi bị lộ mà không buộc mọi client đăng nhập lại.

Đã cân nhắc: thêm `kid` vào header JWT (đổi định dạng token đang lưu ở client, phải hỗ trợ cả hai dạng); tự sinh khoá
mới định kỳ trong database (khoá phải giống nhau trên mọi máy chủ và nằm ngoài database).
