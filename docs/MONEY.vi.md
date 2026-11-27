# Tiền, sổ cái và chứng từ kế toán

[English](MONEY.md)

Ba module lo phần tiền trong Autoxide:

| Module | Làm gì | Module khác dùng |
|---|---|---|
| `tax` | nhóm thuế, thuế suất có ngày hiệu lực, hồ sơ thuế, gói dữ liệu Việt Nam ([TAX.vi.md](TAX.vi.md)) | `dyn TaxEngine` |
| `ledger` | sổ kép, thanh toán qua đối tác có giấy phép, chế độ dòng tiền, tạm giữ, chi trả, chuyển khoản VietQR, tỉ giá | `dyn Ledger` |
| `invoices` | ký hiệu chứng từ, đánh số không nhảy số, hoá đơn điện tử, mẫu in, xuất dữ liệu kế toán, bản lưu trữ | `dyn Invoices` |

**Autoxide không bao giờ tự giữ tiền.** Không có ví, không có số dư "của người mua" nằm ở nền tảng:
tiền đi từ người mua tới đối tác thanh toán có giấy phép hay thẳng vào tài khoản ngân hàng của người
bán. Sổ cái chỉ ghi **nghĩa vụ** (ai nợ ai bao nhiêu) và đối soát với sao kê của đối tác.

> Các ghi chú pháp lý trong tài liệu này (luật kế toán, hoá đơn điện tử, khấu trừ thay, bảo vệ dữ
> liệu) là điểm xuất phát, **không phải tư vấn pháp lý**: đối chiếu văn bản gốc cùng người có chuyên
> môn trước khi dùng cho việc kinh doanh thật.

Mọi route nằm dưới `/api/v1/<module>`, mô tả trong `docs/openapi.json` (phục vụ ở
`/api/v1/openapi.json`). Tiền là `{"amount": "1999", "currency": "USD"}` — số nguyên đơn vị nhỏ nhất
của loại tiền, dạng chuỗi. Tỉ giá, số lượng là chuỗi thập phân như `"25345.5"`, không bao giờ là số
JSON.

## Sổ cái

- **Mỗi nút tổ chức một sổ** (gốc của khách, công ty con, người bán tự hạch toán), theo một tiền hạch
  toán (mặc định: tiền của nút, không có thì VND).
- **Tài khoản đặt theo mục đích**: `cash.bank`, `receivable.customer`, `revenue.sales`,
  `tax.vat_output`, `payable.seller_held`… kèm đối tượng (`org`, `customer`, `carrier`, `partner`,
  `supplier`) khi cần. Module khác thêm mục đích có tiền tố riêng.
- **Bút toán kép, chỉ thêm**: tổng nợ bằng tổng có, mọi dòng dương, một bút toán một loại tiền. Sửa sai
  bằng bút toán đảo (`POST /ledger/entries/{id}/reverse`); mỗi bút toán đảo được một lần.
- **Mọi lệnh ghi nhận `reference`**: gửi lại cùng reference, cùng nội dung → trả kết quả lần đầu (200);
  nội dung khác → bị từ chối (`reference_reused`).
- **Kỳ tháng** khoá được (`POST /ledger/periods/{id}/close`); kỳ đã khoá không nhận bút toán — ghi bút
  toán điều chỉnh vào kỳ đang mở. Kỳ đã khoá không mở lại.
- **Chống sửa**: bút toán được niêm phong vào chuỗi băm trong vài giây; `autoxide doctor` báo bút toán
  đầu tiên bị sửa hay xoá ngoài module (`ledger.chain`) và số dư lệch với dòng bút toán
  (`ledger.balances`).

Đọc: `GET /ledger/entries` (lọc `source_module`, `source_id` để xem bút toán của một chứng từ),
`GET /ledger/accounts/{id}/balance?as_of=`, `GET /ledger/trial-balance`.

## Thanh toán, chế độ dòng tiền, tạm giữ, chi trả

Thanh toán tạo bằng `POST /ledger/payments` và trả qua **đối tác thanh toán** (một module bộ nối cài
trait `PaymentGateway` của kernel). Webhook của đối tác (`POST /ledger/webhooks/{provider}`, kiểm chữ
ký trên thân thô) ghi đã thu; đối soát hằng ngày so sổ cái với sao kê của đối tác và đưa mọi chênh lệch
vào hàng chờ xử lý tay (`GET /ledger/exceptions`).

Mỗi nút có một **chế độ dòng tiền** (`PUT /ledger/flow-settings/{org_id}`, kế thừa xuống cây):

| Chế độ | Ai nhận tiền | Sổ cái ghi gì |
|---|---|---|
| `self_collect` (mặc định) | người bán, trực tiếp | doanh thu và GTGT ở sổ người bán |
| `partner_collect` | đối tác có giấy phép thu hộ cho sàn | khoản phải trả người bán (`payable.seller_held`), tạm giữ tới hết hạn đổi trả rồi mới chi được |
| `split_at_gateway` | đối tác chia tiền ngay tại cổng | chỉ phần của sàn |

Tạm giữ được nhả bởi job (5 phút một lần) hay bằng tay (`POST /ledger/holds/{id}/release`). Chi trả
(`POST /ledger/payouts`) đi qua đối tác `PayoutGateway` và không bao giờ vượt số dư khả dụng, kể cả
khi gửi đồng thời. Phí sàn tách GTGT riêng; khấu trừ thay cho hộ kinh doanh (NĐ 117/2025/NĐ-CP — cần
đối chiếu) chỉ có ở chế độ thu hộ; core tính và ghi sổ, nhưng chưa tự khấu trừ ở mỗi lần chi trả, chưa lập chứng từ
khấu trừ, chưa chuẩn bị tờ khai (cùng module sàn, sau 1.0 — xem `TAX.vi.md`).

## Chuyển khoản VietQR

`POST /ledger/payments/vietqr` trả chuỗi VietQR (định dạng NAPAS / EMVCo) có nội dung chuyển khoản là
một mã ngắn. Tiền vào **thẳng tài khoản ngân hàng của nút**. Nguồn biến động số dư (`BankFeed`: ngân
hàng hay dịch vụ đọc sao kê, qua webhook hay hỏi định kỳ) báo tiền vào; đúng mã và đúng số tiền thì
thanh toán thành đã thu, còn lại (sai số tiền, không mã, trả hai lần, trả muộn) vào hàng chờ. Hiện chỉ
có nguồn giả: bộ nối thật cần hợp đồng với ngân hàng hay dịch vụ.

## Chứng từ kế toán

`invoices` giữ các loại chứng từ: `vat_invoice`, `sales_invoice`, `cash_register_invoice`,
`retail_receipt`, `delivery_note`, `goods_receipt`, `cash_receipt`, `cash_payment`,
`withholding_certificate`.

- **Nháp** chưa có số; thuế tính theo nhóm thuế của từng dòng.
- **Phát hành** (`POST /invoices/documents/{id}/issue`) cấp số tiếp theo của ký hiệu, **không nhảy số**,
  kể cả khi phát hành đồng thời; phát hành lại trả cùng số. Chứng từ bán hàng được ghi vào sổ người bán
  (phải thu, doanh thu, GTGT đầu ra).
- **Hoá đơn điện tử** gửi tới nhà cung cấp (`EInvoiceProvider`, kể cả hoá đơn từ máy tính tiền) trong
  job nền; bị từ chối thì sửa rồi gửi lại cùng số. Chứng từ đã phát hành chỉ sửa bằng chứng từ điều
  chỉnh, thay thế, hay huỷ.
- Một nút lập thay người bán khi có **uỷ nhiệm** (`/invoices/mandates`).

## Tỉ giá và chứng từ ngoại tệ

Tỉ giá nằm ở sổ cái, theo khách, theo cặp tiền, có **ngày hiệu lực**: một tỉ giá dùng từ ngày của nó
tới khi có tỉ giá muộn hơn của cùng cặp.

- Nhập tay: `POST /ledger/exchange-rates` `{"base": "USD", "quote": "VND", "rate": "25345.5",
  "effective_date": "2026-11-17"}` (permission `ledger.settings_manage` cho cả khách). Tỉ giá đã ghi
  không sửa; tỉ giá khác cho cùng cặp và ngày bị từ chối.
- Lấy từ nguồn tỉ giá: `POST /ledger/exchange-rates/fetch` (bộ nối cài trait `ExchangeRateSource` của
  kernel; hiện chỉ có nguồn giả).
- Tra: `GET /ledger/exchange-rates/effective?base=USD&quote=VND&on=2026-11-17` — tỉ giá muộn nhất vào
  hay trước ngày đó; cùng ngày thì tỉ giá nhập tay thắng tỉ giá lấy từ nguồn. Tool MCP
  `ledger.exchange_rate`.

**Chứng từ ngoại tệ** lấy tỉ giá của ngày lập khi phát hành và giữ ảnh chụp (`exchange_rate`,
`exchange_rate_id`, `base_total`); tỉ giá sau đó không đổi chứng từ. Chưa có tỉ giá thì phát hành báo
`invoices.rate_missing`. Bút toán khác tiền của sổ lưu tỉ giá trên từng dòng và số tiền quy ra tiền
sổ; tổng quy đổi làm tròn một lần rồi chia về các dòng, nên bút toán cân ở cả hai loại tiền. Bút toán
đảo dùng tỉ giá của bút toán gốc. (Luật Kế toán 88/2015, Điều 11: số tiền ngoại tệ quy đổi ra đồng
Việt Nam — cần đối chiếu.)

## In chứng từ

`GET /invoices/documents/{id}/render?format=html|pdf&locale=vi|en` in một chứng từ.

- **HTML** dựng từ **mẫu in**: mẫu của nút cho loại chứng từ và ngôn ngữ đó, không có thì mẫu của cả
  khách, không có nữa thì mẫu dựng sẵn. Mẫu viết bằng cú pháp Jinja (`{{ buyer.name }}`,
  `{% for line in lines %}`, bộ lọc `money`, `number`, `date`) và chạy trong **hộp cát**: không đọc
  file, không gọi mạng, không nạp mẫu khác, giới hạn độ dài (`AUTOXIDE_TEMPLATE_MAX_BYTES`), số bước
  chạy và độ dài kết quả. Mọi giá trị đều được thoát HTML, không có bộ lọc `safe`.
- Quản lý mẫu ở `/invoices/templates` (permission `invoices.templates_manage`). Lưu mẫu thì kiểm và báo
  mọi lỗi một lần kèm dòng, cột: `template.syntax_error`, `template.unknown_variable`,
  `template.required_variable_missing`. `GET /invoices/template-defaults` trả mẫu dựng sẵn và danh sách
  biến; `POST /invoices/templates/preview` thử mẫu mà không lưu.
- **PDF** có bố cục A4 cố định dựng từ cùng dữ liệu (tiêu đề, hai bên, bảng dòng có xuống dòng và sang
  trang, tổng tiền, chỗ ký), nhúng font có đủ dấu tiếng Việt. Mẫu chỉ đổi bản HTML; muốn PDF theo bố
  cục riêng thì in bản HTML từ trình duyệt.
- Module khác (đơn hàng, kho) in phiếu giao hàng, phiếu nhập, phiếu thu theo cùng cách: lập chứng từ
  qua `dyn Invoices` rồi gọi `Invoices::render`.

## Xuất dữ liệu kế toán

`POST /invoices/exports` `{"profile": "generic_journal", "format": "csv", "from": "2026-01-01",
"to": "2026-01-31"}` (permission `invoices.export`) tạo file trong job nền; theo `Location` tới khi
`status` là `ready`, rồi `GET /invoices/exports/{id}/download`.

| Hồ sơ | Mỗi hàng |
|---|---|
| `generic_journal` | một dòng bút toán: ngày hạch toán, bút toán, ngày và số chứng từ, diễn giải, tài khoản, nợ, có, loại tiền, tỉ giá, số tiền quy đổi, đối tượng |
| `vn_double_entry` | cặp nợ / có (`TK Nợ`, `TK Có`, `Số tiền`) — hình dạng phần mềm kế toán Việt Nam nhập làm chứng từ nghiệp vụ khác qua bước ghép cột |
| `generic_documents` | một dòng chứng từ: ngày, ký hiệu, số, người mua, mặt hàng, số lượng, đơn giá, thuế suất và số tiền |

- CSV là UTF-8 có BOM (mở bằng Excel đúng dấu tiếng Việt), ô bắt đầu bằng `= + - @` được thêm `'` (chặn
  chèn công thức). Có cả XLSX.
- Số hiệu tài khoản: `external_code` riêng của tài khoản, không có thì theo ánh xạ mục đích → số hiệu
  của chế độ kế toán Việt Nam (`modules/ledger/data/vn-accounts.json`, vd `revenue.sales` → 5111 — **cần
  kế toán đối chiếu** với TT 99/2025/TT-BTC và TT 133/2016/TT-BTC).
- Mẫu nhập của MISA, Fast và các phần mềm khác được tải bên trong từng phần mềm và đổi theo phiên bản,
  nên chưa có hồ sơ riêng cho hãng nào; hãy ghép cột của `vn_double_entry` ở màn nhập của phần mềm.

## Lưu chứng từ 10 năm

Chứng từ và sổ kế toán lưu ít nhất 10 năm kể từ khi kết thúc năm kế toán (Luật Kế toán 88/2015/QH13
Điều 41; NĐ 174/2016/NĐ-CP Điều 12 — cần đối chiếu).

- Chứng từ đã phát hành không xoá được (`invoices.retention_hold`); mỗi chứng từ mang `retention_until`.
- Xoá hẳn một khách vẫn giữ các bảng kế toán (`ledger_*`, `invoices_series`, `invoices_documents`,
  `invoices_lines`, `invoices_mandates`, `invoices_archives`); mọi thứ khác bị xoá.
- **Bản lưu trữ**: `POST /invoices/archives` (permission `invoices.archive`) ghi sổ nhật ký chung
  (`ledger/journal.csv`), mọi chứng từ (`documents.jsonl`) và `manifest.json` (SHA-256 của từng file) vào
  nơi lưu file chung dưới `archives/<khách>/<id>/` — ngoài thư mục của khách, nên còn lại sau khi xoá
  khách. Khách đã xoá mà chưa có bản lưu trữ thì được tạo tự động.
- **Hết hạn lưu**: job hằng ngày (`invoices.purge_expired`) xoá dữ liệu cá nhân của người mua cá nhân
  khỏi chứng từ đã hết hạn (tên, email, địa chỉ, số định danh); với khách đã xoá, job xoá chứng từ hết
  hạn và chứng từ nháp. Chứng từ còn trong hạn lưu không bao giờ bị chạm. Sổ cái của khách đã xoá giữ
  nguyên: chuỗi băm không cắt được từng phần.

## Viết bộ nối đối tác

Bộ nối là một module riêng, chỉ phụ thuộc kernel, đăng ký **trước** `ledger` / `invoices`: cài một
trait của kernel và góp nó trong `start` bằng `context.contribute::<dyn PaymentGateway>(Arc::new(…))`
(hay `PayoutGateway`, `BankFeed`, `ExchangeRateSource`, `EInvoiceProvider`). Mọi lời gọi nhận khách:
đọc thông tin kết nối của khách đó từ kho secret theo khách, không đọc biến môi trường. Kiểm chữ ký
webhook trên thân thô bằng phép so sánh thời gian hằng. Test bằng bản giả của `autoxide-testkit`
(`FakePaymentGateway`, `FakePayoutGateway`, `FakeBankFeed`, `FakeExchangeRateSource`,
`FakeEInvoiceProvider`).
