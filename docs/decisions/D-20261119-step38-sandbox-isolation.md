## D-20261119-step38-sandbox-isolation · Sandbox cách ly: kernel bọc hợp đồng đối tác lúc công bố, hộp thư giả `kernel_sandbox_messages` — 2026-11-19 — agent quyết

Chủ đề: Đa khách, tổ chức, gói

Quyết định: Bước 38 mục sandbox (D-232), phần cách ly (thứ tự làm 13), làm theo đặc tả 38 Luồng 4 với các điểm chốt sau.

1. **Cơ chế thay cho `Partners::get` của đặc tả**: không thêm sổ đối tác mới. `StartContext::provide` và `contribute`
   bọc mọi bộ cài đặt của hợp đồng đối tác của kernel ngay lúc module công bố (`PartnerGuard::wrap`, so `TypeId`): bên
   dùng `require` / `contributions` luôn nhận bản bọc, module nghiệp vụ không sửa và không rẽ nhánh. Bản bọc tra sổ khách
   theo ID (có cache TTL; chế độ một khách không tra): khách sandbox → bộ giả của kernel, khách thật → bộ gốc. Hợp đồng
   được bọc: `PaymentGateway`, `PayoutGateway`, `BankFeed`, `SmsSender`, `ZnsSender`, `WebPushSender`,
   `MobilePushSender`, `EInvoiceProvider`, `IdentityVerifier`, `Mailer`.
2. **Bộ giả trả kết quả xác định, không gọi mạng**: thanh toán thành công (mã `SANDBOX-<payment_id>-<số tiền>-<tiền tệ>`
   để `payment_status` trả đúng số đã thu mà không lưu gì), hoàn / chi trả thành công, biến động số dư rỗng, sao kê rỗng,
   hoá đơn điện tử số `SANDBOX-…` được chấp nhận (tải về là tệp ghi "không phải chứng từ pháp lý"), eKYC duyệt (mã lượt
   `sandbox:<attempt>`; `fetch` nhận ra bằng mã vì không mang khách). Kiểm chữ ký webhook / callback vẫn qua bộ gốc (hàm
   đồng bộ, không tra được khách; đối tác thật không biết giao dịch giả).
3. **Không bọc**: `TextModel` / `Embedder` (không có hệ quả tiền hay pháp lý; hạn mức AI của sandbox đếm riêng),
   `ExchangeRateSource` (chỉ đọc dữ liệu công khai), `Storage`.
4. **Hộp thư giả**: migration kernel `m0038_create_sandbox_messages`, bảng cấp khách hàng `kernel_sandbox_messages`
   (kênh `partner` / `sms` / `zns` / `push` / `mail` / `webhook`, lý do `simulated` / `recipient_not_allowed` /
   `url_not_allowed`), ghi bằng `hold_back` ngoài transaction của bên gọi; relocation `ephemeral` (sandbox không tách /
   gộp); xoá cùng sandbox theo luồng xoá khách. Đọc: `GET /api/v1/kernel/sandbox/messages` (lọc `channel`) và tool MCP
   `kernel.list_sandbox_messages`, cùng hàm service, `kernel.sandboxes.read`; gọi ở khách thật → `tenancy.sandbox_only`
   (409).
5. **Mail**: bọc `Mailer` thay cho sửa module `mail` (cùng hiệu quả, một chỗ). Gửi thật khi người nhận là email đã xác
   minh của nhân viên (realm `staff`, chưa khoá) khách gốc giữ một role đang được uỷ quyền vào sandbox (uỷ quyền chưa thu
   hồi, chưa hết hạn) — trait `SandboxRecipients` của kernel, module `auth` công bố; không ai công bố thì mọi mail của
   sandbox vào hộp thư giả. `Mail` có đúng một người nhận nên không cần tách.
6. **Webhook**: module `webhooks` hỏi `webhook_allowed` trước khi gửi; sandbox chỉ gửi tới URL bắt đầu bằng một tiền tố
   trong cài đặt khách `kernel.sandbox.webhook_urls` (đặt trong sandbox; tiền tố phải dừng ở ranh giới `/`, `?`, `#` hay
   hết chuỗi để `https://a.example` không mở cho `https://a.example.evil.com`). URL khác: lần giao thành trạng thái mới
   `sandboxed`, nội dung vào hộp thư giả.
7. **Vận chuyển**: `ShippingCarrier` là hợp đồng của module `shipping`, kernel không bọc được. Ngoại lệ nhỏ có chủ đích của
   "module nghiệp vụ không tự rẽ nhánh": sổ hãng của `shipping` trả hãng giả (`FakeCarrier`) cho mọi tài khoản hãng của
   sandbox, ở đúng một chỗ (`Registry::for_tenant`).
8. Thêm `Modules::contributions::<T>()` (đọc các bộ góp sau khi khởi động, như `Modules::api`).
9. **Quyền dùng**: `plans` trả quyền dùng (snapshot) của khách gốc cho sandbox — sandbox không có thuê bao nên trước đây
   thành "không giới hạn". Bộ đếm hạn mức vẫn theo từng khách (đếm riêng).
10. **Pháp lý hiển thị**: `GET /compliance/public/badge` của sandbox trả `kind = status = "sandbox"`, `color = "none"`,
    không link / ảnh của Bộ Công Thương, nhãn "bản thử"; `GET /compliance/public/owner` thêm `sandbox` (bool).
11. **Website công khai**: ngoài header `X-Robots-Tag: noindex, nofollow` (phần nền), sitemap công khai của `content` rỗng
    trong sandbox. `content` không có `robots.txt` riêng nên không cần sửa.

Căn cứ pháp lý: kết quả của bộ giả (hoá đơn `SANDBOX-…`, thanh toán giả, eKYC giả) không bao giờ là căn cứ pháp lý: không
đẩy sang khách thật được (Luật Kế toán 88/2015, NĐ 123/2020/NĐ-CP về hoá đơn — hoá đơn thật chỉ do nhà cung cấp được
cấp phép phát hành). Hộp thư giả có thể chứa dữ liệu nhân viên nhập thử; nó bị xoá cùng sandbox khi hết hạn (Luật Bảo vệ dữ
liệu cá nhân 91/2025/QH15, nguyên tắc tối thiểu hoá và giới hạn thời gian lưu).

Lý do: bọc ở chỗ công bố bảo đảm KHÔNG đường nào trong module nghiệp vụ gọi được đối tác thật cho sandbox (kể cả module viết
sau), không phải sửa từng chỗ gọi, và giữ đúng D-038 (module cũ không phải sửa).

Đã cân nhắc: sổ `Partners::get(tenant)` mà mọi bên dùng phải gọi theo khách (phải sửa mọi chỗ gọi ở nhiều module, module mới
dễ quên); biến task-local mang khách hiện tại (mất khi `spawn`, job và event không đi qua lớp HTTP); bộ giả đặt trong module
nối đối tác (mỗi bộ nối tự lo, không đồng nhất).
