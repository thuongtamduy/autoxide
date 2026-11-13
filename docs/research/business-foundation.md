# Nền cho kinh doanh đa mô hình: khách hàng, tổ chức, tiền, thuế, tuân thủ

Thiết kế ngày 13/11/2026 (D-185…D-189). Mục tiêu do chủ dự án đặt: một core phục vụ được **mọi
mô hình kinh doanh** dưới đây chỉ bằng **cấu hình cho từng khách hàng**, không phải viết lại khi
thêm khách hay đổi mô hình, và **tuân thủ pháp luật Việt Nam hiện hành**.

Đây là tài liệu nội bộ (tiếng Việt) cho chủ dự án và agent. Việc cụ thể nằm ở các bước 33–38 của
`docs/ROADMAP.md`; lý do từng quyết định ở `docs/DECISIONS.md`.

## 1. Bốn mô hình phải phục vụ (cộng sàn)

| # | Mô hình | Ví dụ | Đòi hỏi chính |
|---|---|---|---|
| 1 | Khách độc lập | Một cửa hàng, một website | Dùng một hay nhiều module; database riêng hoặc chạy một mình |
| 2 | Một khách nhiều cửa hàng | Chuỗi bán lẻ có chi nhánh, mỗi chi nhánh một tên miền | Dữ liệu riêng từng cửa hàng; một database chung hoặc mỗi cửa hàng một database; báo cáo gộp được |
| 3 | SaaS bán theo tính năng | Nền tảng cho thuê phần mềm bán hàng | Mỗi khách một database; bật / tắt module cho từng khách; gói (dùng thử, cơ bản, Pro, VIP…) với hạn mức riêng; tính tiền, gia hạn, thu hồi |
| 4 | Chuỗi cha-con nhiều cấp | Tổng công ty → công ty con → cửa hàng | Cha tạo và sửa dữ liệu cho con, thấy mọi thứ của con, có dữ liệu riêng; cha-con chung hay khác database |
| + | Sàn | Nhiều người bán trên một nền tảng | Người bán tách biệt; người mua dùng chung; định danh người bán; dòng tiền và thuế theo luật |

## 2. Các hệ thống lớn làm thế nào

| Hệ thống | Cách làm | Mượn gì |
|---|---|---|
| AWS SaaS Lens | Tách **mặt điều khiển** (sổ khách hàng, cấp phát, gói, đo dùng, tính tiền) khỏi **mặt ứng dụng**; ba cách đặt dữ liệu: **silo** (riêng), **pool** (chung), **bridge** (trộn theo từng khách / từng phần) | Hai mặt; vị trí dữ liệu chọn theo từng khách |
| Salesforce | Mọi dòng dữ liệu mang `OrgId`, hàng triệu khách chung một database | `tenant_id` trên mọi bảng cấp khách hàng, kể cả khi khách có database riêng |
| Microsoft Dynamics 365 | Cây "business unit"; quyền có **độ sâu**: của mình / đơn vị / đơn vị và con / toàn tổ chức | Cây tổ chức + quyền theo độ sâu |
| Shopify Plus | Một "organization" chứa nhiều cửa hàng, mỗi cửa hàng dữ liệu riêng; quản trị người dùng ở cấp tổ chức; báo cáo gộp nhiều cửa hàng; khách mua của hai cửa hàng là hai tài khoản | Cây khách hàng nhiều database + báo cáo gộp; tài khoản người mua theo từng khách |
| Stripe Billing | Tính năng gắn vào sản phẩm / gói; ứng dụng hỏi "quyền dùng đang có" (entitlements) thay vì suy từ hoá đơn | Gói → quyền dùng; ứng dụng chỉ hỏi quyền dùng |
| Odoo | Nhiều công ty trong một database; luật lọc theo các công ty người dùng được phép | Lọc theo tập đơn vị được phép |

## 3. Năm trụ của nền

### Trụ 1 — Khách hàng và vị trí dữ liệu

- **Khách hàng (tenant)** là đơn vị hợp đồng, tính tiền và tách biệt dữ liệu.
- **Mặt điều khiển** giữ: sổ khách hàng, vị trí dữ liệu, tên miền, gói, thuê bao, quyền dùng,
  danh tính nhân sự và thành viên, hồ sơ định danh (KYC), hồ sơ pháp lý website, tính tiền.
- **Vị trí dữ liệu** chọn theo từng khách: `shared` (chung database với khách khác) hay
  `dedicated` (database riêng). Cả hai chạy trên SQLite (một file mỗi khách) và Postgres.
- **Mọi bảng cấp khách hàng có `tenant_id`**, kể cả trong database riêng: code giống hệt nhau,
  và dời một khách giữa `shared` ↔ `dedicated` chỉ là chép dữ liệu (`autoxide copy` mở rộng).
  Khoá unique và index bắt đầu bằng `tenant_id`.
- **Chế độ một khách** (`single`, mặc định): mặt điều khiển và dữ liệu chung một database, một
  khách duy nhất — người cài một cửa hàng không phải biết tới đa khách hàng (mô hình 1).
- Mỗi request có **ngữ cảnh khách hàng**: suy từ tên miền (trang công khai), API key (gắn với
  một khách), access token (khách đang chọn), hay header của nhân sự nền tảng; kernel chọn đúng
  database (sổ kết nối, mở lười, có giới hạn), module nhận ngữ cảnh qua extractor.
- Job, event, outbox nằm cùng database với dữ liệu (outbox phải cùng transaction); worker và bộ
  phát event chạy trên từng database. Migration chạy cho cả đội database (`migrate --all`).

### Trụ 2 — Tổ chức trong khách hàng, quyền theo độ sâu, danh tính

- **Cây tổ chức** trong mỗi khách: nút có loại tự đặt (`hq`, `region`, `store`, `warehouse`,
  `seller`…), sâu tối đa có giới hạn; bảng tổ tiên – con cháu để tra nhánh bằng một câu có index.
- **Thành viên**: người ↔ nút ↔ role. **Quyền có độ sâu** (như Dynamics): `own` (bản ghi mình
  tạo), `unit` (nút mình), `subtree` (nút mình và mọi nút con), `tenant` (toàn khách).
- **Dữ liệu nghiệp vụ** mang `tenant_id` + `org_id` (nút sở hữu) + người tạo. Cha tạo dữ liệu
  cho con = tạo với `org_id` của con (cần quyền `subtree`); dữ liệu riêng của cha = `org_id` của
  cha.
- **Cài đặt kế thừa theo cây** (múi giờ, tiền tệ, hồ sơ thuế, chính sách): nút con dùng giá trị
  của tổ tiên gần nhất trừ khi tự đặt.
- **Tên miền gắn với nút**: mỗi cửa hàng một tên miền; trang công khai biết mình thuộc nút nào.
- **Danh tính hai lớp**: nhân sự (quản trị, nhân viên, người bán) là tài khoản toàn nền tảng
  trong mặt điều khiển — một người làm cho nhiều khách / nhiều nút; **người mua** là tài khoản
  theo từng khách (như Shopify). Sàn là một khách nên người mua dùng chung tự nhiên.
- **Cây khách hàng xuyên database** (mô hình 2 và 4 khi con có database riêng): khách hàng có
  khách hàng cha trong mặt điều khiển; role ở cha chảy xuống các khách con (uỷ quyền, audit ở cả
  hai phía); thao tác chi tiết = vào ngữ cảnh con (cùng API); **nhìn tổng** = kho báo cáo gộp nhận
  event từ các con (gần thời gian thực, như báo cáo tổ chức của Shopify Plus); cha đẩy dữ liệu
  xuống con bằng job phân phối gọi service của con. Có công cụ tách một nhánh thành khách riêng
  và gộp ngược lại.

### Trụ 3 — Gói, quyền dùng, hạn mức, thuê bao (mô hình 3)

- **Module khai** tính năng (`Module::features`) và hạn mức (`Module::limits`), mặc định rỗng.
- **Gói là dữ liệu, không phải code**: tập module + tính năng + hạn mức + giá theo kỳ; dùng thử;
  gói thêm; ghi đè riêng cho từng khách. Ví dụ hạn mức: số cửa hàng, số nhân sự mỗi cửa hàng,
  dung lượng file, số request, số sản phẩm.
- **Kernel cưỡng chế**: module chưa bật cho khách thì route, tool MCP, job, event của nó bị chặn
  (`module_not_enabled`); tính năng hỏi `has_feature`; hạn mức giữ chỗ / trả lại nguyên tử
  (công cụ khoá dòng sẵn có); số đo dùng ghi lại cho tính tiền.
- **Vòng đời thuê bao**: dùng thử → đang dùng → quá hạn (ân hạn) → tạm khoá (chỉ đọc) → huỷ (cho
  xuất dữ liệu) → xoá (sau thời hạn lưu theo luật). Thu hồi module: tắt và giữ dữ liệu theo chính
  sách lưu.
- **Cấp phát**: khách tự đăng ký (kèm KYC khi cần) → tạo khách → chọn vị trí dữ liệu theo gói
  (vd VIP → database riêng) → migrate → dữ liệu mẫu → mời quản trị.

### Trụ 4 — Tiền, sổ cái, thuế, chứng từ

**Luật cứng: core không bao giờ tự giữ tiền.** Nhận tiền người mua rồi chi lại cho người bán là
dịch vụ hỗ trợ thu hộ, chi hộ — một loại trung gian thanh toán cần giấy phép của Ngân hàng Nhà
nước (Nghị định 52/2024/NĐ-CP). Tiền chỉ đi qua **đối tác có giấy phép** (ngân hàng, trung gian
thanh toán) qua trait `PaymentGateway` / `PayoutGateway`; core ghi **nghĩa vụ** bằng sổ cái kép
và đối soát với sao kê của đối tác.

Ba chế độ dòng tiền, chọn cho từng khách / từng sàn:

| Chế độ | Tiền đi thế nào | Hợp với | Thuế khấu trừ thay (NĐ 117/2025) |
|---|---|---|---|
| **Tự thu** | Người mua trả thẳng vào tài khoản người bán (cổng của người bán); nền tảng không chạm tiền, thu phí bằng hoá đơn / thuê bao | Khách độc lập, chuỗi, SaaS, sàn không có thanh toán | Không: nền tảng không có chức năng thanh toán; người bán tự kê khai |
| **Thu hộ qua đối tác** | Người mua trả vào tài khoản thu hộ của đối tác có giấy phép; sổ cái ghi phải trả người bán, phí sàn, thuế khấu trừ; giữ tới hết thời hạn đổi trả rồi lệnh chi hộ | Sàn kiểu Shopee, Lazada | Có: khấu trừ ngay khi giao dịch thành công |
| **Chia tại cổng** | Cổng thanh toán chia tiền ngay cho tài khoản con của người bán và phí của sàn | Sàn có đối tác hỗ trợ chia tiền | Có |

- **Sổ cái kép** (module `ledger`): tài khoản theo khách / nút / người bán; bút toán nợ = có, chỉ
  thêm, sửa sai bằng bút toán đảo; số dư suy từ bút toán; đối soát định kỳ với đối tác.
- **Số thập phân** (`rust_decimal`) để tính thuế, chiết khấu, tỉ giá; lưu vẫn bằng `Money`.
- **Engine thuế** trong core, trung lập: nhóm hàng hoá (mã nhóm thuế), **biểu thuế là dữ liệu có
  ngày hiệu lực** theo vùng thuế (bắt đầu: Việt Nam), giá đã / chưa gồm thuế, cách làm tròn, hồ
  sơ thuế của người bán (doanh nghiệp khấu trừ / hộ kinh doanh / miễn), thuế khấu trừ thay. Mỗi
  dòng chứng từ lưu **ảnh chụp** thuế suất lúc giao dịch, không tính lại lịch sử. Gói dữ liệu
  "vn" cập nhật bằng CLI / API khi luật đổi; `autoxide doctor` cảnh báo thuế suất sắp hết hạn.
  Ví dụ cho thấy vì sao: mức GTGT 8% (giảm 2%) chỉ áp dụng tới hết 31/12/2026.
- **Thuế theo mặt hàng**: module sản phẩm gắn nhóm thuế cho từng mặt hàng (GTGT 0% / 5% / 10%,
  mức giảm tạm thời, không chịu thuế, thuế tiêu thụ đặc biệt cho hàng đặc thù); engine trả các
  dòng thuế.
- **Chứng từ**: đánh số tuần tự không nhảy số theo phạm vi (khách / người bán / năm / ký hiệu);
  hoá đơn điện tử qua trait `EInvoiceProvider` (nhà cung cấp hoá đơn điện tử); sàn phát hành thay
  người bán khi được uỷ quyền.

### Trụ 5 — Tuân thủ pháp luật Việt Nam

| Yêu cầu | Căn cứ (đối chiếu ngày 13/11/2026) | Nền đáp ứng |
|---|---|---|
| Xác thực danh tính người bán trước khi cho bán; cả người livestream, tiếp thị liên kết | Luật TMĐT 122/2025/QH15, Điều 17, hiệu lực 01/7/2026; có thể qua VNeID | Hồ sơ định danh (cá nhân / tổ chức), trait `IdentityVerifier` (VNeID qua nhà cung cấp, eKYC), trạng thái và hạn; **cổng chặn**: nút loại người bán chưa xác thực thì không đăng bán, không livestream |
| Website bán hàng phải **thông báo** (logo xanh "Đã thông báo"); nền tảng trung gian (sàn) phải **đăng ký** (logo đỏ "Đã đăng ký") với Bộ Công Thương | NĐ 52/2013 + 85/2021; từ 01/7/2026 theo Luật TMĐT 2025 và nghị định hướng dẫn (các nguồn nêu NĐ 248/2026 — cần đối chiếu văn bản gốc) | Xem mục 4 |
| Tài khoản phải xác thực bằng số điện thoại hoặc định danh mới được đăng bài, bình luận, livestream | NĐ 147/2024/NĐ-CP | Cờ "đã xác thực số điện thoại / định danh" trên tài khoản; khách bật yêu cầu cho bình luận, đánh giá |
| Bảo vệ dữ liệu cá nhân: căn cứ xử lý, quyền của chủ thể, báo vi phạm trong 72 giờ kể từ khi phát hiện | Luật BVDLCN 91/2025/QH15 + NĐ 356/2025, hiệu lực 01/01/2026 | Sổ đồng ý / mục đích; xuất và xoá dữ liệu cá nhân (đã có); mã hoá dữ liệu nhạy cảm; quy trình sự cố 72 giờ; thời hạn lưu theo loại dữ liệu |
| An ninh mạng, lưu trữ dữ liệu người dùng | Luật An ninh mạng 116/2025/QH15 (hiệu lực 01/7/2026), NĐ 53/2022 | Vị trí dữ liệu theo khách (lưu tại Việt Nam được chọn ở mức khách); thời hạn lưu log và audit cấu hình được |
| Sàn có thanh toán khấu trừ và nộp thuế thay người bán | NĐ 117/2025/NĐ-CP (cần đối chiếu NĐ 68/2026 về hộ kinh doanh) | Engine thuế + sổ cái + báo cáo |
| Sở hữu trí tuệ, hàng giả: tiếp nhận phản ánh, gỡ nội dung / hàng vi phạm | Luật TMĐT 2025, Luật SHTT | Cơ chế báo vi phạm → ẩn → phản hồi của người bị báo → quyết định, có hạn xử lý, lưu vết; license thư viện kiểm bằng `cargo deny` (đã có) |

**AI không phải luật sư.** Mỗi tính năng tuân thủ ghi căn cứ pháp lý trong `docs/DECISIONS.md`;
quy định hay đổi (biểu thuế, danh mục hồ sơ, thời hạn) để dạng **dữ liệu có ngày hiệu lực**; chỗ
các nguồn chưa thống nhất ghi rõ "cần đối chiếu văn bản gốc / người có chuyên môn pháp lý" trước
khi chạy thật.

## 4. Đăng ký với Bộ Công Thương: hệ thống làm gì

Hồ sơ nộp trên cổng của nhà nước do con người làm (không có API công khai); hệ thống lo phần còn
lại để hồ sơ **đủ và đúng** ngay từ đầu:

1. **Hồ sơ pháp lý chủ website** trong mặt điều khiển, theo khách và theo người bán: tên, mã số
   thuế, địa chỉ, người đại diện, số điện thoại, email, giấy đăng ký kinh doanh.
2. **Loại website** theo mô hình: website bán hàng (thông báo → logo xanh) hay nền tảng trung
   gian / sàn (đăng ký → logo đỏ). Mỗi tên miền một trạng thái.
3. **Trang bắt buộc** có mẫu sẵn (module `content`): thông tin chủ sở hữu, chính sách bảo vệ dữ
   liệu cá nhân, đổi trả, vận chuyển, thanh toán, kiểm hàng, giải quyết khiếu nại, điều khoản sử
   dụng; với sàn thêm **quy chế hoạt động**, mẫu hợp đồng với người bán, và **thông tin người
   bán hiện trên trang sản phẩm**.
4. **Danh sách kiểm tuân thủ là dữ liệu có phiên bản** (thay được khi nghị định đổi mà không sửa
   code): API và `autoxide doctor` liệt kê thứ còn thiếu cho từng tên miền trước khi nộp.
5. **Xuất bộ hồ sơ**: dữ liệu điền tờ khai, bản nháp đề án và quy chế từ cấu hình thật của hệ
   thống (luồng giao dịch, dòng tiền, cách xử lý khiếu nại).
6. **Logo**: lưu trạng thái (chưa nộp / đã nộp / đã thông báo / đã đăng ký / bị yêu cầu bổ sung)
   và đoạn mã logo cơ quan cấp cho từng tên miền; endpoint công khai trả về để giao diện gắn ở
   chân trang.
7. **Nghĩa vụ sau đăng ký** của sàn: kênh tiếp nhận khiếu nại, gỡ hàng vi phạm trong hạn, cung cấp
   thông tin người bán khi cơ quan yêu cầu, báo cáo định kỳ — các mục này có trong nhóm tuân thủ
   và trong sổ tay vận hành.

## 5. Cấu hình cho từng mô hình

| Mô hình | Khách hàng | Vị trí dữ liệu | Cây tổ chức | Dòng tiền | Pháp lý website |
|---|---|---|---|---|---|
| 1 Khách độc lập | Một (chế độ `single`) | Chung với mặt điều khiển, hay riêng | Một nút (hay vài nút) | Tự thu | Website bán hàng: thông báo, logo xanh |
| 2 Một khách nhiều cửa hàng | Một khách, cửa hàng là nút; **hoặc** khách cha + mỗi cửa hàng một khách con | Chung; hoặc mỗi cửa hàng database riêng, báo cáo gộp ở cha | Trụ sở → (vùng) → cửa hàng | Tự thu theo cửa hàng hay theo trụ sở | Mỗi tên miền bán hàng một thông báo |
| 3 SaaS bán tính năng | Mỗi khách hàng của SaaS một khách | Theo gói: thường chung, gói cao database riêng | Theo từng khách | Thuê bao SaaS: tài khoản của nhà vận hành; bán hàng của khách: tự thu | Từng khách thông báo website của họ |
| 4 Chuỗi cha-con nhiều cấp | Một khách cây sâu; **hoặc** cây khách hàng xuyên database | Trộn được | Nhiều cấp, quyền `subtree` ở cha | Theo từng nút / từng khách | Theo từng tên miền |
| Sàn | Một khách (sàn), người bán là nút loại `seller` đã KYC | Chung hay riêng | Sàn → người bán → kho / cửa hàng | Tự thu / thu hộ qua đối tác / chia tại cổng | Sàn: đăng ký, logo đỏ; người bán: KYC |

## 6. Thứ tự làm và cái giá

Các bước 33–38 của `docs/ROADMAP.md` (làm theo thứ tự; vận hành, hiệu năng và năng lực còn
thiếu dời sau để dựng sẵn trên nền đa khách):

1. **33** khách hàng, vị trí dữ liệu, mặt điều khiển, `tenant_id` trên mọi bảng cấp khách hàng,
   mã hoá secret và cấu hình theo khách, tên miền riêng.
2. **34** cây tổ chức, quyền theo độ sâu, danh tính hai lớp, uỷ quyền, đăng nhập bằng link, mời
   người dùng.
3. **35** gói, quyền dùng, hạn mức, thuê bao, tính tiền SaaS, cấp phát.
4. **36** sổ cái, số thập phân, thanh toán qua đối tác, thuế, chứng từ, hoá đơn điện tử.
5. **37** tuân thủ: KYC, hồ sơ và logo Bộ Công Thương, dữ liệu cá nhân, văn bản pháp lý có
   phiên bản, bảo vệ người tiêu dùng, gắn nhãn nội dung AI, SMS, gỡ vi phạm.
6. **38** cây khách hàng xuyên database, báo cáo gộp, phân phối dữ liệu cha → con.

Module thương mại (sản phẩm, giỏ hàng, đơn hàng, kho, vận chuyển, sàn) là module nghiệp vụ, dựng
**sau** nền, dùng `tenant_id` + `org_id` + engine thuế + sổ cái ngay từ đầu.

**Cái giá biết trước**: bước 33 sửa mọi bảng của 13 module (migration mới, không sửa migration
cũ); đường nóng mỗi request đọc thêm ngữ cảnh khách và quyền theo nút (đo trước / sau); người
viết module phải phân loại bảng (mặt điều khiển / cấp khách hàng) và chọn bảng nào có `org_id` —
hướng dẫn viết module ghi quy tắc, `cargo xtask check` và testkit kiểm.

## 7. Bổ sung sau rà soát (D-190)

Rà lại thiết kế trên thấy những chỗ sau sẽ phải sửa về sau nếu không đặt từ đầu:

| Chỗ hở | Vì sao phải làm từ đầu | Bước |
|---|---|---|
| Mọi thứ ngoài bảng cũng theo khách: khoá file lưu trữ, cache trong bộ nhớ, rate limiting, idempotency key, con trỏ pagination, token và URL đã ký | `tenant_id` trên bảng chưa đủ: cache tìm theo ý nghĩa của `content` hay một URL ký của `media` dùng chung giữa các khách là rò rỉ dữ liệu | 33 |
| Cấu hình và secret theo khách (khoá cổng thanh toán, SMTP, hoá đơn điện tử, AI của từng khách), mã hoá khi lưu | Khách SaaS và người bán trên sàn mang tài khoản đối tác của chính họ; biến môi trường chỉ có một bộ cho cả máy chủ. Vì vậy mục mã hoá secret dời từ 37 lên 33 | 33 |
| Tên miền riêng: xác minh bằng DNS TXT, endpoint cho reverse proxy cấp TLS tự động | Mô hình 2 cần mỗi cửa hàng một tên miền; không xác minh thì ai cũng gắn được tên miền của người khác | 33 |
| Nhiều database lệch phiên bản migration; job queue chung bị một khách chiếm | Đội database không bao giờ migrate xong cùng lúc; một khách lớn không được làm chậm khách khác | 33 |
| Nhân sự nền tảng vào dữ liệu của khách: có thời hạn, có lý do, khách xem được audit | Luật BVDLCN 2025: xử lý dữ liệu cá nhân phải có căn cứ và để lại vết | 34 |
| Lưu chứng từ kế toán ít nhất 10 năm, khoá sổ theo kỳ | Luật Kế toán 88/2015, Điều 41; NĐ 174/2016. Xoá khách hay xoá dữ liệu cá nhân không được xoá phần kế toán | 36 |
| Hoá đơn khởi tạo từ máy tính tiền | NĐ 123/2020 sửa bởi NĐ 70/2025: hộ kinh doanh doanh thu từ 1 tỷ đồng / năm, từ 01/6/2025; ngưỡng hạ dần theo lộ trình — để dạng dữ liệu | 36 |
| Văn bản pháp lý có phiên bản và bằng chứng chấp nhận | Luật Giao dịch điện tử 20/2023: điều khoản, quy chế sàn, hợp đồng với người bán là hợp đồng điện tử; phải chứng minh được ai chấp nhận bản nào | 37 |
| Bảo vệ người tiêu dùng | Luật 19/2023/QH15 + NĐ 55/2024 (từ 01/7/2024): nghĩa vụ của nền tảng số, không chặn đánh giá, điều kiện giao dịch chung, khiếu nại | 37 |
| Gắn nhãn nội dung do AI tạo | Luật Trí tuệ nhân tạo 134/2025/QH15 (từ 01/3/2026): nội dung AI tạo hay sửa đưa ra công chúng phải được thông báo rõ. Core đã có gợi ý AI cho `content`, nên phải ghi nguồn gốc | 37 |
| Gửi SMS | NĐ 147/2024 xác thực bằng số điện thoại cần mã một lần qua SMS; core mới có email | 37 |
| Quan sát theo khách | Log và trace mang `tenant_id`; không dùng nó làm nhãn số đo vì số khách không giới hạn | 39 |

Đã cân nhắc và không đưa vào core: mỗi khách một khoá mã hoá riêng để "xoá bằng cách huỷ khoá" cả
trong backup — phức tạp và chưa có yêu cầu luật buộc; khuyến mại phải đăng ký, phí sàn theo ngành
hàng, người bán nước ngoài — thuộc module thương mại.

## Nguồn

Kiến trúc:

- AWS SaaS Lens — [Silo, pool, bridge](https://docs.aws.amazon.com/de_de/wellarchitected/latest/saas-lens/silo-pool-and-bridge-models.html), [Bridge model](https://docs.aws.amazon.com/wellarchitected/latest/saas-lens/bridge-model.html), [Preventing cross tenant access](https://docs.aws.amazon.com/wellarchitected/latest/saas-lens/preventing-cross-tenant-access.html)
- AWS SaaS Architecture Fundamentals — [Control plane vs. application plane](https://docs.aws.amazon.com/whitepapers/latest/saas-architecture-fundamentals/control-plane-vs.-application-plane.html), [Core services](https://docs.aws.amazon.com/whitepapers/latest/saas-architecture-fundamentals/core-services.html)
- Salesforce — [Platform multitenant architecture](https://architect.salesforce.com/fundamentals/platform-multitenant-architecture)
- Microsoft Dynamics 365 — [Security roles and privileges](https://learn.microsoft.com/en-us/dynamics365/customerengagement/on-premises/admin/security-roles-privileges), [Set up security roles](https://learn.microsoft.com/en-us/training/modules/configure-dynamics-365-sales/4-security-roles)
- Shopify — [Organization analytics](https://help.shopify.com/en/manual/organization-settings/analytics), [Multi-store reporting](https://changelog.shopify.com/posts/multi-store-reporting-is-now-available-in-analytics)
- Stripe — [Entitlements](https://docs.stripe.com/billing/entitlements)
- Odoo — [Multi-company guidelines](https://www.odoo.com/documentation/20.0/sv/developer/howtos/company.html)

Pháp lý (báo chí và công ty luật tổng hợp; đối chiếu văn bản gốc trước khi chạy thật):

- Luật TMĐT 2025, xác thực người bán — [LuatVietnam](https://luatvietnam.vn/tin-van-ban-moi/tu-hom-nay-01-7-2026-san-tmdt-phai-xac-thuc-nguoi-ban-truoc-khi-mo-gian-hang-186-109956-article.html), [Dentons LuatViet](https://www.dentonsluatviet.com/vi/insights/alerts/2026/march/30/key-new-highlights-of-the-law-on-ecommerce-2025)
- Luật BVDLCN 2025 — [Thư viện Pháp luật](https://thuvienphapluat.vn/van-ban/Bo-may-hanh-chinh/Luat-Bao-ve-du-lieu-ca-nhan-2025-so-91-2025-QH15-625628.aspx), [EY về NĐ 356/2025](https://www.ey.com/content/dam/ey-unified-site/ey-com/vi-vn/technical/tax/documents/ey-vietnam-legal-alert-march-2026-decree-no356-2025-nd-cp-providing-detailed-guidance-for-implementation-of-personal-data-protection-law-viet.pdf)
- Luật An ninh mạng 2025 — [EY](https://www.ey.com/vi_vn/technical/tax/tax-and-law-updates/luat-an-ninh-mang-2025-cua-viet-nam-cac-diem-chinh-va-yeu-cau-tuan-thu)
- NĐ 117/2025 thuế TMĐT — [VCCI](https://vcci.com.vn/tin-tuc/san-giao-dich-thuong-mai-dien-tu-nop-thue-thay-cho-ho-kinh-doanh-tu-ngay-1-7), [VnEconomy](https://vneconomy.vn/san-da-nop-thue-thay-ho-kinh-doanh-online-co-can-ke-khai-va-phat-hanh-hoa-don.htm)
- NĐ 147/2024 xác thực tài khoản — [LuatVietnam](https://luatvietnam.vn/tin-van-ban-moi/tu-25-12-2024-tai-khoan-mang-xa-hoi-da-xac-thuc-bang-so-dien-thoai-moi-duoc-dang-bai-186-99855-article.html)
- NĐ 52/2024 thanh toán không dùng tiền mặt — [Thư viện Pháp luật](https://thuvienphapluat.vn/van-ban/Tien-te-Ngan-hang/Nghi-dinh-52-2024-ND-CP-thanh-toan-khong-dung-tien-mat-427855.aspx)
- Thuế GTGT 2026 và mức giảm 8% tới 31/12/2026 — [Thư viện Pháp luật](https://thuvienphapluat.vn/phap-luat/chinh-thuc-giam-thue-gtgt-tu-10-xuong-8-tu-ngay-172025-den-het-31122026-mo-rong-doi-tuong-giam-thue-221915.html)
- Lưu trữ chứng từ kế toán 10 năm — [MISA meInvoice](https://www.meinvoice.vn/tin-tuc/6704/luu-tru-hoa-don-dien-tu-khoa-hoc-an-toan/), [Kế toán Thuận Thiên](https://ketoanthuanthien.vn/hoi-dap/quy-dinh-ve-thoi-han-luu-tru-hoa-don-dien-tu-la-bao-lau/)
- Hoá đơn từ máy tính tiền, NĐ 70/2025 — [Sapo](https://www.sapo.vn/blog/nghi-dinh-so-70-2025-nd-cp), [Thời báo Tài chính](https://thoibaotaichinhvietnam.vn/ho-ca-nhan-kinh-doanh-chu-dong-ra-soat-doanh-thu-ap-dung-hoa-don-dien-tu-tu-may-tinh-tien-175242.html)
- Luật Bảo vệ quyền lợi người tiêu dùng 2023 — [Bộ Công Thương](https://moit.gov.vn/tin-tuc/bao-chi-voi-nguoi-dan/luat-bao-ve-quyen-loi-nguoi-tieu-dung-nam-2023-va-van-ban-huong-dan-luat-huong-toi-muc-tieu-nang-cao-trach-nhiem-bao-ve-.html), [LuatVietnam](https://luatvietnam.vn/thuong-mai/luat-bao-ve-quyen-loi-nguoi-tieu-dung-2023-so-19-2023-qh15-259732-d1.html)
- Luật Trí tuệ nhân tạo 2025 — [Cần Thơ PBGDPL](https://pbgdpl.cantho.gov.vn/mot-so-diem-moi-cua-luat-tri-tue-nhan-tao), [LSVN](https://lsvn.vn/doanh-nghiep-can-lam-gi-khi-luat-tri-tue-nhan-tao-2025-co-hieu-luc-a169962.html)
- Thông báo / đăng ký website TMĐT — [Cổng dịch vụ công Bộ Công Thương](https://dichvucong.moit.gov.vn/TTHCOnlineDetail.aspx?DocId=115), [LTV Law](https://ltvlaw.com/tin-tuc/dang-ky-website-cung-cap-dich-vu-tmdt), [Siglaw](https://siglaw.com.vn/thu-tuc-thong-bao-website-thuong-mai-dien-tu.html)
