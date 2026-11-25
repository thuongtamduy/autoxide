# Module: cái gì chạy, và chọn thế nào

[English](MODULES.md)

Binary của Autoxide chứa mọi module, nhưng một bản cài chỉ chạy những module nó cần. Một biến cấu
hình chọn chúng: `AUTOXIDE_MODULES`.

## Ba hạng

- **core** — máy chủ không chạy được nếu thiếu (đăng nhập, lệnh ghi idempotent, và những thứ
  chúng cần). Luôn bật; không tắt được.
- **official** (chính thức) — bật sẵn. API không đổi kiểu phá vỡ trong cùng một dòng phiên bản:
  thay đổi làm hỏng client thì làm endpoint mới, endpoint cũ khai deprecated kèm ngày gỡ. Bản mới
  luôn đọc được dữ liệu của bản cũ (migration tự chạy). Lỗi bảo mật được vá cho mọi dòng phiên bản
  còn hỗ trợ dài hạn.
- **experimental** (thử nghiệm) — tắt sẵn; bật bằng cách nêu tên. Chạy được và được test trên
  SQLite lẫn PostgreSQL như phần còn lại, nhưng API **có thể đổi giữa các bản, kể cả kiểu phá
  vỡ**; mỗi lần đổi được ghi trong changelog. Hệ thống không bao giờ tự xoá dữ liệu của nó, nhưng
  chuyển dữ liệu sang bản mới có thể cần một bước làm tay, ghi trong ghi chú nâng cấp. Module thử
  nghiệm có thể bị gỡ nếu không ai dùng, hoặc lên chính thức.

Một module thuộc trọn một hạng; không chia hạng bên trong một module. Module chính thức không bao
giờ phụ thuộc module thử nghiệm.

**Điều kiện lên chính thức:** không còn mục dở trong lộ trình; đã có người dùng thật; API không
phải đổi qua vài bản liên tiếp; riêng bộ nối tới nhà cung cấp bên ngoài: đã chạy thử với tài khoản
thật.

## Các module

Cột `Cần có` liệt kê những module mà thiếu thì module không khởi động được; chúng được bật kèm.
Một module còn *dùng* module khác khi module đó đang bật (vd `orders` lấy phí ship từ `shipping`
và giữ kho qua `inventory`), và chỉ làm được ít hơn khi module đó tắt.

<!-- modules:begin -->
| Module | Hạng | Cần có | Làm gì |
| --- | --- | --- | --- |
| `mail` | core | — | Gửi mail cho mọi module, có hàng đợi và retry, qua SMTP hoặc ghi ra file khi phát triển. |
| `auth` | core | `mail` | Tài khoản, đăng nhập bằng mật khẩu, passkey và TOTP, access và refresh token, API key, role và permission. |
| `idempotency` | core | — | Lưu kết quả theo `Idempotency-Key`, để lệnh ghi gọi lại chỉ chạy một lần. |
| `ai` | official | — | Kết nối tới mô hình AI (sinh văn bản và embedding) cho mọi module; mặc định tắt. |
| `files` | official | — | Lưu file cho mọi module, trên đĩa hoặc nơi lưu tương thích S3. |
| `address` | official | — | Đơn vị hành chính Việt Nam (tỉnh và xã sau sáp nhập 2025) và kiểm địa chỉ, dùng chung cho mọi khách. |
| `organizations` | official | — | Cây tổ chức trong mỗi khách (trụ sở, vùng, cửa hàng, người bán…), loại nút, vòng đời và cài đặt. |
| `audit` | official | — | Audit log chỉ ghi thêm: ai đổi gì, trên bản ghi nào, khi nào, trước và sau. |
| `webhooks` | official | — | Webhook có chữ ký (Standard Webhooks) gửi event của module ra hệ thống khác, có retry và lịch sử gửi. |
| `tax` | official | — | Bộ máy thuế: nhóm thuế và thuế suất là dữ liệu có ngày hiệu lực, hồ sơ thuế theo nút, khấu trừ và yêu cầu hoá đơn. |
| `ledger` | official | — | Sổ cái kép theo nút tổ chức: tài khoản, bút toán chỉ ghi thêm, số dư, tạm giữ, chi trả và thanh toán qua đối tác có giấy phép. |
| `invoices` | official | `tax`, `ledger` | Chứng từ kế toán và hoá đơn điện tử: nháp, phát hành có số, điều chỉnh, thay thế và huỷ. |
| `compliance` | official | — | Nền tuân thủ pháp luật Việt Nam: định danh, văn bản pháp lý và chấp thuận, đăng ký website, yêu cầu về dữ liệu cá nhân và sự cố. |
| `notifications` | official | — | Thông báo tới người dùng qua email, SMS, Zalo ZNS, web push, mobile push và hộp thư trong ứng dụng, có mẫu và tuỳ chọn nhận. |
| `plans` | official | — | Gói bán module, tính năng và hạn mức theo khách, có add-on, ngoại lệ, thuê bao và mức dùng. |
| `media` | official | `ai`, `files` | Thư viện media: ảnh, video, âm thanh và tài liệu trong thư mục lồng nhau, có thẻ, alt text và bản thu nhỏ. |
| `content` | official | `ai`, `media` | Bài viết và trang của website bằng nhiều ngôn ngữ: nháp, phiên bản, hẹn giờ, chuyên mục, thẻ, tác giả và chuyển hướng. |
| `forms` | official | `mail`, `files` | Biểu mẫu có trường định kiểu, nhận bài gửi công khai sau lớp chống spam, hộp thư, xuất CSV và thời hạn lưu. |
| `menus` | official | `media` | Menu của website (thanh trên, mega menu, chân trang), mục trỏ tới địa chỉ hoặc bản ghi của bất kỳ module nào. |
| `catalog` | official | — | Sản phẩm, biến thể, cây danh mục, bảng giá có lịch sử và giá sàn: danh mục sản phẩm mọi module thương mại đọc. |
| `inventory` | official | `catalog` | Tồn kho theo biến thể và kho với sổ biến động chỉ ghi thêm, giữ chỗ không bán quá, chuyển kho, kiểm kê, lô, serial và mua hàng. |
| `shipping` | official | — | Vận chuyển: sổ địa chỉ người mua, phí ship do từng shop đặt, hãng vận chuyển cắm vào dạng bộ nối, vận đơn có kiện và theo dõi, đối soát COD. |
| `promotions` | official | `catalog` | Khuyến mại dạng luật: giảm tự động, mã giảm giá và voucher, flash sale, bậc, combo và quà tặng, cộng dồn theo ba lớp, giữ lượt dùng cho đơn và hạn mức pháp lý là dữ liệu có ngày. |
| `orders` | official | `catalog` | Đơn hàng: giỏ cho người mua và khách vãng lai, đặt hàng có giữ kho và khuyến mại theo thời hạn, đơn có số tuần tự và vòng đời, thanh toán qua đối tác có giấy phép, trả hàng và hoàn tiền, hoá đơn điện tử, bán tại quầy, đánh giá sản phẩm. |
| `customers` | official | — | Hồ sơ khách hàng theo tài khoản hoặc số điện thoại, lịch sử mua từ đơn hàng, thông tin liên hệ được che và xem có audit, nhóm khách thủ công hoặc theo luật cho bảng giá và khuyến mại. |
| `sepay` | experimental | — | Xác nhận chuyển khoản VietQR từ thông báo biến động số dư của SePay, mỗi khách một tài khoản SePay (thử nghiệm: viết theo tài liệu công khai của nhà cung cấp, chưa kiểm với dịch vụ thật). |
| `momo` | experimental | — | Cổng thanh toán MoMo, mỗi khách một tài khoản MoMo riêng: link thanh toán, thông báo kết quả, truy vấn trạng thái và hoàn tiền. Viết theo tài liệu công khai của nhà cung cấp, chưa kiểm với dịch vụ thật. Xem [hướng dẫn MoMo](guides/providers/momo.vi.md). |
| `einvoice_sepay` | experimental | — | Gửi hoá đơn điện tử tới cơ quan thuế Việt Nam qua SePay eInvoice, mỗi khách một tài khoản SePay eInvoice (thử nghiệm: viết theo tài liệu công khai của nhà cung cấp, chưa kiểm với dịch vụ thật). |
| `esms` | experimental | — | SMS qua eSMS (Việt Nam): mã một lần và tin chăm sóc khách hàng bằng tài khoản eSMS và brandname riêng của từng khách. Viết theo tài liệu công khai của nhà cung cấp; chưa kiểm với dịch vụ thật. |
| `ghn` | experimental | `shipping` | Bộ nối hãng Giao Hàng Nhanh (GHN) cho vận chuyển: báo giá, vận đơn, in vận đơn, huỷ, tra trạng thái và webhook trạng thái, bằng tài khoản GHN của từng khách. Viết theo tài liệu công khai của nhà cung cấp; chưa kiểm với dịch vụ thật. |
| `bookings` | experimental | — | Dịch vụ đặt lịch với nhân viên, phòng và thiết bị, lịch tuần, lịch trống công khai, lịch hẹn không chồng nhau, đặt cọc qua đối tác có giấy phép, chính sách huỷ và nhắc lịch. |
| `newsletter` | experimental | `mail` | Danh sách bản tin, người đăng ký double opt-in, huỷ đăng ký một chạm và chiến dịch gửi theo lô có giới hạn tốc độ. |
| `comments` | experimental | `mail` | Bình luận và câu hỏi trên mọi bản ghi (bài viết, sản phẩm…), có luồng trả lời, kiểm duyệt, báo cáo, cấm và trả lời. |
| `support` | experimental | — | Phiếu hỗ trợ khách hàng: tin nhắn, ghi chú nội bộ, khách vãng lai vào bằng token của phiếu, phạm vi nhân viên theo nút, chuyển sang khiếu nại người tiêu dùng. |
| `conversations` | experimental | — | Chat giữa khách và cửa hàng hay gian hàng trên sàn: hộp thư chung, phân công, con trỏ đã đọc, event thời gian thực, nhắc người offline, chuyển sang phiếu hỗ trợ và trả lời tự động từ câu đã duyệt. |
| `analytics` | experimental | — | Phân tích website không cookie, lưu trong chính database của bạn: trang, nguồn, chiến dịch UTM, đối tượng xem nhiều và phễu; theo dõi từng người chỉ khi có đồng ý. |
| `reporting` | experimental | — | Báo cáo gộp của cây khách hàng: số liệu từ mọi khách (cả trên database khác), cộng theo ngày, khách, nút và tiền tệ. |
| `automations` | experimental | — | Luật tự động hoá: khi có event, lịch, lời gọi tay hay số liệu bất thường và điều kiện thoả, chạy tool MCP với quyền của người viết luật. |
<!-- modules:end -->

`GET /api/v1/kernel/modules` liệt kê các module đang chạy trên một máy chủ, kèm `stability` của
từng cái. Trong tài liệu OpenAPI, mọi operation của module thử nghiệm mang
`"x-stability": "experimental"` (operation không có dấu này thuộc module chính thức), và
`docs/mcp-tools.json` có `stability` cho từng tool; cả hai cũng ghi điều đó trong phần mô tả.

## Chọn: `AUTOXIDE_MODULES`

Danh sách cách nhau bằng dấu phẩy. Mỗi mục thêm vào tập; module core luôn có trong tập.

| Mục        | Nghĩa                                                       |
| ---------- | ----------------------------------------------------------- |
| `core`     | Chỉ module core.                                            |
| `official` | Mọi module core và chính thức. **Mặc định.**                |
| `all`      | Mọi module, kể cả thử nghiệm.                               |
| `tên`      | Module đó, cùng những module nó cần.                        |
| `-tên`     | Bỏ module đó khỏi tập mà các mục khác đã chọn.              |

Ví dụ:

```sh
# Mặc định, viết rõ ra.
AUTOXIDE_MODULES=official

# Module chính thức cộng hai module thử nghiệm.
AUTOXIDE_MODULES=official,bookings,support

# Backend cho một website: đăng nhập, trang và bài viết, media, biểu mẫu, menu. `media`, `files`,
# `ai` và `mail` đi kèm vì `content`, `forms` và `menus` cần chúng.
AUTOXIDE_MODULES=content,forms,menus

# Mọi module chính thức trừ gói và các module thương mại không dùng ở đây.
AUTOXIDE_MODULES=official,-plans,-customers,-orders
```

Viết sai thì mọi lệnh báo ngay lúc khởi động, kèm cách viết đúng:

- tên không phải module: liệt kê các tên hợp lệ;
- `-tên` với một module mà module đang bật cần: nêu các module phải bỏ cùng
  (`official,-catalog` trả lời: bỏ thêm `-inventory,-orders,-promotions`, hoặc xoá `-catalog`);
- `-auth` hay module core khác: bị từ chối.

`autoxide doctor` in module nào đang chạy và module nào tắt (dòng đầu của log `autoxide serve`
cũng nói vậy), và `autoxide config` cho thấy giá trị đang dùng.
`autoxide modules` liệt kê mọi module của chương trình — kể cả module mà binary của bạn thêm —
kèm hạng, đang bật hay tắt, phụ thuộc và, với module đang tắt, database còn dữ liệu của nó không;
`autoxide modules --json` in đúng những thứ đó cho chương trình và agent.

Cấu hình của module đang tắt không được đọc: dòng `AUTOXIDE_NEWSLETTER_…` còn sót trong `.env` bị
bỏ qua. Task trong `AUTOXIDE_STARTUP_TASKS` mà module của nó đang tắt thì được bỏ qua kèm một cảnh
báo trong log; tên task không tồn tại ở một module đang bật vẫn là lỗi.

## "Tắt" nghĩa là gì

Module đang tắt:

- không phục vụ route nào: đường dẫn của nó trả `404`;
- không có trong tài liệu OpenAPI phục vụ ở `/api/v1/openapi.json`, trong `/llms.txt` và trong
  `GET /api/v1/kernel/modules`;
- không có tool, resource hay prompt MCP;
- không chạy job, không chạy lịch định kỳ, không nhận event;
- không có migration nào được áp dụng: trên database mới, bảng của nó không bao giờ được tạo.

File `docs/openapi.json` và `docs/mcp-tools.json` trong repo, và SDK sinh từ chúng, luôn mô tả
**mọi** module. `autoxide openapi` và `autoxide mcp-manifest` in hợp đồng đầy đủ đó bất kể
`AUTOXIDE_MODULES` đang là gì. Client hỏi một máy chủ đang chạy module nào qua
`GET /api/v1/kernel/modules`.

### Dữ liệu của module đang tắt

Tắt một module **không bao giờ xoá gì**. Database đã có bảng của nó thì bảng giữ nguyên, và bật
lại module là dùng tiếp từ đó (migration thêm trong lúc tắt được `autoxide migrate up` áp dụng như
thường).

Trong lúc tắt, module vẫn tham gia dữ liệu cá nhân và vòng đời khách ("chế độ nghỉ") — không route,
không tool MCP, không job, không nhận event khác, nhưng:

- bản xuất dữ liệu của một người (`GET /api/v1/auth/me/export`, yêu cầu truy cập của chủ thể) có phần
  của nó, và xoá tài khoản thì xoá / ẩn danh dữ liệu của người đó trong nó, như khi bật;
- xoá hẳn một khách thì xoá dòng của khách đó trong bảng của nó (trừ bảng luật buộc giữ), bản xuất dữ
  liệu của khách có các dòng đó, và dời / tách / gộp khách mang chúng theo (migration của chính module
  được áp cho bảng của nó ở cả hai database trước, và chỉ ở nơi nó đã có bảng);
- `files` tắt vẫn đọc cấu hình của nó và mở kho, nên xoá tài khoản xoá cả nội dung file, và `conversations`
  xoá bản ghi âm qua kho đó và bộ nối thoại đang bật; không mở được kho thì việc xoá được thử lại (không báo
  xong khi nội dung còn nằm lại).

Đây là nghĩa vụ của pháp luật bảo vệ dữ liệu cá nhân (Việt Nam: Luật 91/2025/QH15, Nghị định
13/2023/NĐ-CP), nên không phụ thuộc bạn còn dùng module hay không. Những việc vẫn chưa làm khi tắt:

- dọn dữ liệu của nó theo thời hạn lưu không chạy (job của module không chạy);
- khách tạo trong lúc đó không có dữ liệu khởi đầu của module;
- event module khác phát trong lúc đó không được giao bù về sau.

Để dữ liệu đó không mất vì sơ ý:

- `autoxide serve` ghi cảnh báo vào log lúc khởi động, và `autoxide doctor` báo
  `modules_turned_off`, khi database còn dòng của một module đang tắt — kèm đúng dòng
  `AUTOXIDE_MODULES=…` để bật lại. (Khi bạn tự đặt `AUTOXIDE_MODULES` thì câu này chỉ là thông
  tin: bạn đã chọn.)
- `autoxide copy`, `autoxide backup create` và `autoxide backup restore` từ chối chạy khi còn dữ
  liệu như vậy, vì chúng chỉ xử lý module đang bật. Chạy chúng với module đó bật, hoặc với
  `AUTOXIDE_MODULES=all`.

Việc kiểm dữ liệu như vậy xem database chính và database của từng placement.

Muốn xoá hẳn dữ liệu của một module: bật module, chạy
`autoxide migrate down <module> --steps 1000` (lệnh
này xoá bảng của nó — backup trước), rồi tắt.

## Nâng cấp từ 0.17 trở về trước

Tới 0.17 mọi module đều chạy. Từ bản này, khi không đặt `AUTOXIDE_MODULES`, tám module thử nghiệm
(`bookings`, `newsletter`, `comments`, `support`, `conversations`, `analytics`, `reporting`,
`automations`) **tắt**.

1. Trước khi cập nhật, xác định bạn đang dùng module nào trong số đó. Không chắc thì cập nhật rồi
   chạy `autoxide doctor`: `modules_turned_off` liệt kê module nào còn bảng có dòng, và đưa dòng
   cần thêm.
2. Thêm dòng đó vào `.env` (hay môi trường của container), vd
   `AUTOXIDE_MODULES=official,newsletter,comments`, rồi khởi động lại. Muốn giữ đúng hành vi của
   0.17 thì dùng `AUTOXIDE_MODULES=all`.
3. Role vẫn giữ permission của module đang tắt, gói vẫn giữ tính năng của nó, dữ liệu đã lưu vẫn
   còn; chúng có hiệu lực lại khi module được bật.

Cả hai trường hợp đều không xoá gì, và lựa chọn đổi lại được bất cứ lúc nào.

## Cho người viết module

Module khai hạng và thứ nó cần trong phần cài `Module`; cả ba đều có mặc định, nên module không
khai gì là chính thức, tắt được và không phụ thuộc ai:

```rust
fn stability(&self) -> Stability { Stability::Experimental }
fn essential(&self) -> bool { false }
fn depends_on(&self) -> Vec<&'static str> { vec!["mail", "files"] }
```

Ghi vào `depends_on` mọi module mà bạn lấy hợp đồng của nó bằng `context.require::<…>()?` trong
`start`. Hợp đồng lấy bằng `.ok()` là tuỳ chọn và không ghi vào đó. Một bài test trong binary khởi
động từng module chỉ với những module nó khai, nên thiếu một tên thì gate đỏ chứ không phải bản
cài của ai đó hỏng. Phần còn lại của hợp đồng xem [MODULE_GUIDE.vi.md](MODULE_GUIDE.vi.md).
