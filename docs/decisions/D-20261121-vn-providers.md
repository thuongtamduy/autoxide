## D-20261121-vn-providers · Bộ nối nhà cung cấp đầu tiên: SePay, MoMo, Giao Hàng Nhanh, eSMS, SePay eInvoice; tài khoản do từng khách tự cấu hình — 2026-11-21 — chủ dự án quyết

Chủ đề: Thương mại

Quyết định:

1. Bộ nối thật đầu tiên (bước 70 P7): **thanh toán** — SePay và MoMo; **vận chuyển** — Giao Hàng Nhanh; **SMS** —
   eSMS; **hoá đơn điện tử** — SePay eInvoice. Mỗi bộ nối là một module nhỏ sau trait sẵn có (`PaymentGateway`,
   `BankFeed`, `ShippingCarrier`, `SmsSender`, `EInvoiceProvider`); core không đổi hợp đồng vì một nhà cung cấp.
2. **Tài khoản nhà cung cấp là của từng khách, do khách tự cấu hình** (API key, mã cửa hàng, secret webhook…): lưu
   theo khách, mã hoá, đặt qua API có permission; không phải cấu hình chung của hệ thống. Hai khách trên cùng một hệ
   thống dùng hai tài khoản khác nhau của cùng một nhà cung cấp; webhook về phải tìm đúng khách rồi mới kiểm chữ ký
   bằng secret của khách đó. Secret không bao giờ trả lại qua API hay vào log.
3. **Viết theo tài liệu công khai của nhà cung cấp trước**, chưa có tài khoản thử. Vì chưa chạy với dịch vụ thật,
   các bộ nối thuộc hạng thử nghiệm và tài liệu ghi rõ "viết theo tài liệu, chưa kiểm với dịch vụ thật" cho tới khi
   có người chạy thử bằng tài khoản thật. Test dùng máy chủ giả dựng theo đúng tài liệu (request, chữ ký, webhook).
4. P5 (đăng npm, crates.io): **ghi nợ**, làm sau. P6 (kiểm toán bảo mật): chưa thuê bên ngoài; trước 1.0 một phiên
   AI độc lập rà lại toàn bộ, và `SECURITY.md` ghi đúng đó là rà soát nội bộ có AI hỗ trợ, không phải kiểm toán của
   bên thứ ba.

Lý do: người dùng Autoxide là nhiều khách độc lập, mỗi khách có hợp đồng riêng với nhà cung cấp; tiền của người mua
đi thẳng vào tài khoản của khách qua đối tác có giấy phép, hệ thống không đứng giữa (D-187). Viết theo tài liệu trước
để không chờ tài khoản thử; ghi rõ mức kiểm chứng để người cài không hiểu nhầm.

Đã cân nhắc: cấu hình nhà cung cấp bằng biến môi trường chung cho cả hệ thống — loại: mọi khách dùng chung một tài
khoản thì tiền và hoá đơn lẫn vào nhau. Chờ tài khoản thử rồi mới viết — loại: chậm, trong khi phần lớn công việc là
theo tài liệu.

Thay: phần "chờ chủ dự án" ở P5, P6, P7 của D-20261121-community-first
