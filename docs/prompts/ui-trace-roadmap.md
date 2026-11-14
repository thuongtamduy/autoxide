# Đề bài: bổ sung lộ trình, đặc tả và API sau khi đối chiếu thiết kế 2

Đề bài này giao cho một agent làm việc trong repo Autoxide. Đọc `CLAUDE.md` trước; mọi luật ở đó áp dụng:
- cả SQLite và Postgres;
- pháp luật Việt Nam (D-186), core không giữ tiền người mua (D-187);
- module sở hữu bảng;
- OpenAPI đầy đủ, i18n, gate.

## Bối cảnh

Thiết kế giao diện 2 (D-200, D-201) đã được đối chiếu hai chiều với lộ trình và đặc tả:
- thứ trên màn hình mà lộ trình không có;
- mục trong lộ trình mà chưa có màn hình.

Kết quả nằm ở `docs/research/ui-traceability.md`, chi tiết ở `docs/research/ui-trace/A-nen-tang.md`
(nền tảng, quản trị chung) và `docs/research/ui-trace/B-thuong-mai.md` (thương mại, website bán hàng).
Mã như `A-b1`, `B-c3` trỏ tới dòng trong hai file đó; đọc dòng gốc để có căn cứ đầy đủ.

Phần thiết kế do agent chính tự sửa. Đề bài này chỉ gồm phần của repo: lộ trình, đặc tả, và API.

## Cách làm

1. **Phần lớn là bước chưa tới (33–49, 51, 52).** Với các bước này, việc là **sửa đặc tả và lộ trình**
   cho đúng, chưa viết code. Bước nào tới lượt làm thì làm theo đặc tả đã sửa.
2. **Phần dính module đã chạy** (`content`, `media`, `forms`, `auth`, `newsletter`) cũng chỉ ghi vào bước
   tương ứng của lộ trình. Không chen ngang thứ tự lộ trình, trừ khi `CLAUDE.md` cho phép.
3. **Mỗi lựa chọn có ảnh hưởng lâu dài** ghi một mục bằng skill `record-decision`. Lấy số kế tiếp chưa
   dùng; D-202 đã dành cho `docs/prompts/ui-api-gaps.md`, và `docs/prompts/ui-help-api.md` lấy số kế tiếp.
   Mục "Hướng đã chọn" dưới đây là hướng agent chính đề xuất. Làm theo, trừ khi thấy trái luật hay trái
   một quyết định "chủ dự án quyết"; khi đó dừng lại và báo.
4. **Quy định pháp luật** ghi căn cứ (văn bản, điều, ngày hiệu lực). Chỗ ghi "cần đối chiếu" thì giữ
   nguyên chữ đó trong quyết định, không tự khẳng định.
5. **Gate.** Sửa tài liệu thì chạy `./gate.sh --quick`. Phần nào có code thì phải chạy `./gate.sh` đầy
   đủ trên cả hai database.

## Hướng đã chọn (ghi thành quyết định)

| # | Câu hỏi | Hướng | Lý do |
|---|---|---|---|
| H1 | Nội dung (bài, trang, media, form) theo website và theo nút tổ chức? | **Có.** Thêm `org_id` (nút sở hữu) cho `content` và `media`, `forms` tuỳ chọn. Quyền theo độ sâu. Đọc công khai lọc theo website: tên miền → nút, dựa trên `kernel_tenant_domains.org_id`. Menu và ngôn ngữ theo website. Thay câu ở `docs/steps/34-organizations-identity.md` dòng 60 bằng mục mới. | Mô hình chuỗi và sàn của D-185 cần cửa hàng và người bán có nội dung, media riêng. Câu cũ là agent quyết, không phải chủ dự án. A-b1, A-c3 |
| H2 | Thiết kế có "Chờ xác nhận" (cửa hàng hay người bán nhận đơn) | **Giữ, thành chính sách.** `orders_policies.acceptance_hours`: mặc định 0, tức tự xác nhận như đặc tả hiện tại. Sàn đặt mức cho người bán. Quá hạn thì tạo việc, rồi huỷ theo chính sách. | Sàn ở Việt Nam thường buộc người bán nhận đơn; chỉ số `cancelled_by_seller` của 46 cần nó. B-b6 |
| H3 | Tách nhánh thành khách riêng, migrate cả đội, chuyển database | **Chỉ nhân sự nền tảng.** Khách chỉ "gửi yêu cầu". Thêm API job cho mặt điều khiển, dry-run mặc định. CLI vẫn là cách chính. | 38:286; A-c6 |
| H4 | Cấp quyền vào hỗ trợ | Giữ trần 8 giờ, mặc định 1–72 như đặc tả. **Khách tự thu hồi được** một lần cấp đang còn hạn. | A-b5 |
| H5 | Hộp việc có nút "Đã xử lý" | **Bỏ** (thiết kế sửa). Việc tự hết khi bản ghi gốc xong; chỉ có Hoãn và Giao. | 51:99 |
| H6 | "Chat với người bán", huy hiệu dùng "tỉ lệ trả lời tin nhắn" | **Không làm trong core.** Chat vẫn ở "Sau core". Huy hiệu tin cậy chỉ dùng dữ liệu có thật. | B-b11 |
| H7 | Thời điểm lập hoá đơn | **Mặc định `on_delivered`.** `on_complete` chỉ khi có căn cứ. Cần đối chiếu NĐ 123/2020 Điều 9, sửa bởi NĐ 70/2025. | B-b5 |
| H8 | COD trên sàn | Tiền COD **không bao giờ** về tài khoản của nhà vận hành sàn. Tiền về tài khoản thu hộ của đối tác có giấy phép, hoặc về thẳng người bán. Với người bán tự thu, phí sàn và khấu trừ thay thu bằng hoá đơn phí định kỳ hoặc công nợ. Cần đối chiếu NĐ 52/2024 và NĐ 117/2025. | D-187; B-b1 |
| H9 | Một giỏ có nhiều nút khác chế độ dòng tiền | Chỉ một mã QR khi mọi nút cùng đối tác thu hộ. Khác chế độ thì checkout trả `payment_actions[]`, mỗi chế độ một mục; giao diện hiện từng bước. | B-b2 |
| H10 | Yêu cầu dữ liệu cá nhân người dùng tự làm (xuất, xoá) | **Không cần duyệt** (thiết kế sửa). Sổ yêu cầu có hạn trả lời chỉ dành cho yêu cầu qua kênh khác. | A-b3 |

## Việc theo bước

Ký hiệu loại: **thiếu** là không có ở đâu; **hở** là đặc tả dùng mà chưa định nghĩa; **lệch** là hai đặc
tả mâu thuẫn nhau.

### Kernel và khung chung (bước 33, 34, 35, 40, 51, 52)

| Mục | Bước | Loại | Nội dung | Gốc |
|---|---|---|---|---|
| Job API chung | 51 | hở | Các endpoint: `GET /api/v1/kernel/jobs?status=&kind=`, `GET /kernel/jobs/{id}` (gồm `progress`, `result`, `file`), `POST /kernel/jobs/{id}/retry` (có audit). Permission đọc và ghi. File kết quả giữ có hạn, tải qua link ký. P8 (51:239) đang dẫn tới endpoint này mà chưa định nghĩa. | A-b2, A-c1 |
| Xuất danh sách theo bộ lọc | 51 | thiếu | Xuất dùng chung cho mọi module: lấy bộ lọc theo schema của P7, chạy bằng job, ra file CSV qua `autoxide_kernel::csv`, báo xong qua `notifications`. Module khai cột được xuất. Thay cho việc mỗi module tự làm (forms, catalog 41:237, orders). | A-b8, B-c6 |
| Yêu cầu duyệt đầy đủ | 51 P12 | hở | Thêm: `GET /kernel/approvals?status=`, `GET /kernel/approvals/{id}`, `POST …/reject {reason}`. Thêm event `kernel.approval_decided` để agent biết kết quả, và tool MCP tương ứng. | A-c2 |
| Xác nhận lại danh tính | 34 hoặc 40 | thiếu | `POST /auth/me/reauth` bằng passkey, TOTP hoặc link, trả token ngắn hạn. Các lệnh nhạy cảm nhận token này thay cho `current_password`. Hiện tài khoản không có mật khẩu không tạo được API key. | A-b6, A-c4 |
| Ngôn ngữ và múi giờ ưa thích | 34 | thiếu | Lưu trong `auth`. Mail và `notifications` gửi chậm dùng hai giá trị này. Thay mục đã hoãn ở D-062. | A-b4 |
| Khách tự thu hồi quyền vào hỗ trợ | 34 | thiếu | Theo H4. | A-b5 |
| Cấu hình hành vi module theo khách | 33 | thiếu | Áp `Module::tenant_settings` cho cấu hình của từng module: từ cấm và số link của `comments`, hạn lưu của `forms`, ngôn ngữ bật của `content`… | A-b7 |
| Danh sách khách cho mặt điều khiển | 33, 35 | hở | `GET /kernel/tenants` thêm bộ lọc `placement_kind`, `schema_state`, `subscription_status`, `near_limit`, hoặc `expand=`, để không phải gọi riêng cho từng dòng. | A-c7 |
| API job đội database | 33 | thiếu | Theo H3: `POST /kernel/placements/migrations`, `POST /kernel/tenants/{id}/moves`; chỉ nhân sự nền tảng, dry-run mặc định. | A-c6 |
| Xuất toàn bộ dữ liệu một khách | 35 | thiếu | Gói tải về chạy qua job khi khách huỷ thuê bao. Nhà vận hành và chính khách đều gọi được. | A-b9 |
| Tạm khoá và chỉ đọc | 33, 35 | lệch | Tách rõ "tạm khoá hẳn" (kernel, chặn mọi route) khỏi "chỉ đọc" (thuê bao quá hạn). Thêm `POST /tenants/{id}/subscription/suspend` nếu cần. | A-3.11 |
| Hạn mức nút | 35 | hở | Ghi rõ hạn mức đếm theo loại nút hay đếm mọi nút. | A-3.12 |
| Nhỏ | 51, 52, 34 | hở | Tên endpoint cho nhật ký request, gửi lại webhook, event theo bản ghi. Permission `kernel.devtools.manage`. Lưu `request_id`, `trace_id`. Event theo bản ghi đọc từ audit, vì outbox bị dọn. Thêm `DELETE /me/work/{key}/snooze`, `GET /memberships?org_id&inherited=true`. `GET /kernel/settings` thêm `last_used_at`, `last_error`; thêm `POST /kernel/settings/{name}/test` (gọi qua trait, không gọi trong transaction ghi). Thêm `requires_approval_for_agent` vào `ModuleTool`. Trợ lý AI trong quản trị: `POST /api/v1/ai/assistant/messages {page, text}`, chạy tool MCP với quyền người gọi, chỉ đọc mặc định (D-109). | A-c10, A-c11, A-c12 |
| Thấp | 34, 35, 37, 38 | thiếu | Chính sách bắt buộc MFA theo role. `last_login_at`. Báo cáo đã lưu và lịch gửi qua email. Thông báo của nền tảng tới quản trị mọi khách. Liên hệ hỗ trợ của nền tảng. | A-b10, B-b14 |

### Nội dung theo website và nút (H1)

| Mục | Bước | Loại | Nội dung | Gốc |
|---|---|---|---|---|
| `org_id` cho `content`, `media` (`forms` tuỳ chọn) | 34 (mục mới) | thiếu | Gồm: bộ lọc `owner_org` và `site`; quyền theo độ sâu; công khai lọc theo tên miền. Media riêng của người bán trên sàn cũng nhờ mục này. | A-b1, A-c3 |
| Bộ lọc danh sách nội dung | 51 P7 | hở | Bài viết thêm `review=pending`, `scheduled=true`, `ai_generated`, `missing_locale=en`. Media thêm `missing_alt=true`. | A-c5 |

### Tuân thủ (bước 37)

| Mục | Loại | Nội dung | Gốc |
|---|---|---|---|
| Sổ yêu cầu của chủ thể dữ liệu | thiếu | Bảng `compliance_subject_requests` với các cột `kind`, `channel`, `subject`, `received_at`, `due_at`, `status`, `job_id`. API: `GET/POST /compliance/subject-requests`, `POST …/{id}/fulfil`, cộng việc trong hộp việc. Hạn trả lời là dữ liệu có ngày hiệu lực; cần đối chiếu Luật 91/2025 và NĐ 356/2025. Không áp cho yêu cầu người dùng tự làm (H10). | A-b3, A-c8 |
| KYC "cần bổ sung" | hở | Trạng thái `needs_info`, hoặc `POST /kyc/profiles/{id}/request-info`. | A-c9 |
| Văn bản và sự cố | hở | Đếm số người đã chấp nhận theo phiên bản. Bản nháp thông báo sự cố gửi cơ quan và gửi người bị ảnh hưởng. | A-c9 |
| "Chặn bán" | hở | Cần đối chiếu NĐ 52/2013, sửa bởi NĐ 85/2021, và Luật TMĐT 2025: bán trước khi thông báo website bị **cấm** hay chỉ **bị phạt**. Nếu cấm thì thêm cổng chặn đặt hàng ở 45/37. Nếu không thì đổi nhãn thành "bắt buộc theo luật". | A-3.5 |

### Tiền, sổ cái, thuế (bước 36)

| Mục | Loại | Nội dung | Gốc |
|---|---|---|---|
| Thống nhất bút toán và tạm giữ giữa 36, 45 và 46 | lệch | Gồm bốn việc: một bộ tên tài khoản; tiền thu hộ ghi vào đâu lúc thu; phí và khấu trừ ghi lúc nào; chỉ giữ một cơ chế tạm giữ (`ledger_holds` hoặc `marketplace_settlements.hold_until`). | B-b3, B-3.2d |
| Một hạn thanh toán VietQR | lệch | Đặc tả 36 ghi 30 phút, đặc tả 45 ghi 1440 phút. Chọn một giá trị. | B-b10, B-c12 |
| Mã số thuế nhận số định danh 12 số | thiếu | Cần đối chiếu TT 86/2024. Tuỳ chọn thêm trait `TaxpayerDirectory::lookup` (bản giả cho test; bản nối thật cần tài khoản đối tác, khi đó dừng lại hỏi). | B-c11 |
| Bút toán theo đơn | thiếu | `GET /ledger/entries?source_module&source_id`. | B-c12 |
| Phí sàn tách GTGT; khấu trừ thay theo loại người bán | lệch | Khấu trừ thay chỉ áp cho hộ và cá nhân kinh doanh; doanh nghiệp tự kê khai. Cần đối chiếu NĐ 117/2025. | B-3.2e, f |

### Sản phẩm, khuyến mại, vận chuyển (41, 43, 44)

| Mục | Bước | Loại | Nội dung | Gốc |
|---|---|---|---|---|
| Sản phẩm của người bán trên website của chuỗi | 41 | thiếu | Lọc `/public/products?seller=&sold_by=` có `facets`. `ProductCard.seller`. Cột `warranty_months`. Điểm cắm `CatalogExtensions::set_listing_gate` mà 46 đang giả định. | B-b4, B-c3 |
| Giá giảm tự động và flash sale trên danh sách công khai | 43, 41 | lệch | Giá hiển thị sau giảm tự động, bằng endpoint hoặc cột cập nhật từ event. Flash sale công khai: giờ và suất còn. Lọc `on_sale`. Điều kiện `first_order_only`. | B-b7, B-c10 |
| GTGT trên phí giao | 44, 45 | thiếu | `shipping_methods.tax_category`, dòng thuế cho phí giao. Cần đối chiếu Luật 48/2024. | B-b5, B-c9 |
| Thường mua cùng | 41 | thiếu (thấp) | Bảng dẫn xuất đồng xuất hiện trong đơn. | B-c14 |

### Đơn hàng (45)

| Mục | Loại | Nội dung | Gốc |
|---|---|---|---|
| Chính sách đơn | thiếu | `invoice_timing` thêm `on_delivered` làm mặc định (H7). `buyer_cancel_until` thêm `preparing`. Thêm `allowed_return_reasons`. Thêm `acceptance_hours` (H2). | B-b5, B-b6, B-c8 |
| Nhiều chế độ dòng tiền trong một checkout | thiếu | Theo H9. | B-b2, B-c2 |
| Nhận tại cửa hàng | thiếu | `POST /orders/{id}/pickup/ready` (gửi mã nhận 6 số). `POST …/pickup/handover {code | proxy_name, proxy_phone}`. Lọc `GET /orders?pickup_org_id&pickup_status`. | B-c7 |
| Chứng từ của người mua | thiếu | `GET /me/orders/{id}/documents`, kèm bản dùng token xem đơn vãng lai. | B-c5 |
| Giỏ bỏ dở, gửi lại link thanh toán, xuất đơn | thiếu | `GET /orders/carts?status=abandoned`. `POST /orders/{id}/payment-link`. Xuất đơn đi qua cơ chế xuất chung của 51. | B-b9, B-c6 |

### Sàn, khách hàng (46, 47)

| Mục | Bước | Loại | Nội dung | Gốc |
|---|---|---|---|---|
| COD trên sàn | 46, 44, 36 | thiếu | Theo H8. Cột `ledger_flow_settings.cod_remittance`; lỗi `marketplace.cod_flow_unsupported`; event `shipping.cod_settled` mang `seller_org_id`. | B-b1, B-c1 |
| Danh sách người bán, đánh giá theo người bán, theo dõi gian hàng | 46, 47 | thiếu | `GET /public/sellers`, `GET /public/sellers/{slug}/reviews`, `POST/DELETE /me/followed-sellers/{slug}`. Bảng `marketplace_followers`. Báo hàng mới qua `notifications`, cần đồng ý riêng. | B-b8, B-c4 |
| Huy hiệu người bán tin cậy | 46 | lệch | Theo H6. | B-b11 |
| Gộp danh sách đã lưu khi đăng nhập | 47 | thiếu (thấp) | `POST /me/wishlist/merge`. | B-c13 |

### Tài liệu thiết kế

`docs/research/ui-design.md`: bảng phân hệ thêm hàng cho bước 47 (điểm, hạng, CRM), 48 (kênh bán) và
49 (tiếp thị liên kết, livestream); thêm hai vai trò "người bán trên sàn" và "người tiếp thị" (B-b12).
Agent chính sẽ vẽ màn hình tương ứng; teammate chỉ cần cập nhật bảng.

## Báo cáo khi xong

Báo cáo bằng tiếng Việt, lời thường, cho chủ dự án mới học Rust. Nội dung báo cáo:
- các quyết định đã ghi (số và tên);
- mục nào đã vào lộ trình, ở bước mấy;
- chỗ nào còn ghi "cần đối chiếu văn bản gốc";
- gate có xanh không.

Commit trên nhánh làm việc, không push khi chủ dự án chưa yêu cầu.
