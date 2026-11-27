## D-20261127-hsts-production · HSTS bật mặc định khi URL công khai là `https` — 2026-11-27 — agent quyết

Chủ đề: HTTP, REST, OpenAPI

Quyết định: bước 71 G7 ("HSTS bật mặc định ở môi trường production"). Repo không có biến "môi trường" riêng; dấu hiệu
production là **`AUTOXIDE_HTTP_PUBLIC_URL` bắt đầu bằng `https://`** (máy phát triển là `http://localhost…` hay không
đặt). `AUTOXIDE_HTTP_HSTS_SECONDS` không còn mặc định `0`: **không đặt** thì `31536000` (một năm,
`HSTS_DEFAULT_SECONDS`) khi URL công khai là `https://`, `0` khi không; đặt số thì theo số đó; **`0` tắt** cả ở
production. `includeSubDomains` vẫn chỉ khi `AUTOXIDE_HTTP_HSTS_INCLUDE_SUBDOMAINS=true`; `AUTOXIDE_HTTP_SECURITY_HEADERS=false`
vẫn tắt mọi header. `HttpConfig::read` chỉ đọc giá trị URL qua `ConfigReader::configured` (không ghi nhận lần đọc thứ hai;
`public_url::read_setting` vẫn là chỗ kiểm và ghi nhận). `autoxide doctor` (bước `hsts`) nay báo OK với URL `https` để
mặc định, chỉ cảnh báo khi người vận hành tắt.

Lý do: RFC 6797 — HSTS chỉ có tác dụng trên kết nối bảo mật (trình duyệt bỏ qua header qua HTTP thường, §8.1), nên gửi
khi API công khai qua HTTPS là đủ; máy chủ chạy sau proxy kết thúc TLS nên không tự biết request đến qua HTTPS, URL công
khai là thứ người vận hành đã khai sẵn cho đúng việc này. Một năm là giá trị thường dùng và là mức tối thiểu của danh
sách preload. Không bật `includeSubDomains` mặc định: có thể phá subdomain khác còn chạy HTTP.

Đã cân nhắc: thêm biến `AUTOXIDE_ENV=production` — thêm một khái niệm mới chỉ cho một header, và người cài dễ quên đặt;
dựa vào `X-Forwarded-Proto` từng request — giả mạo được khi không có proxy tin cậy, và không cần khi đã biết URL công khai;
bật HSTS ở mọi nơi — làm hỏng máy phát triển dùng HTTP trên `localhost` khi trình duyệt nhớ header từ lần chạy HTTPS khác.

Thay: phần "HSTS chỉ khi `AUTOXIDE_HTTP_HSTS_SECONDS > 0`" của D-320
