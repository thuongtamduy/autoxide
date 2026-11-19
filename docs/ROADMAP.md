# Lộ trình

Mỗi bước xong khi `./gate.sh --quick` xanh; agent đánh dấu `[x]`, commit, merge vào `main`. Gate đầy đủ (**cả SQLite
và Postgres**) bắt buộc xanh trước mỗi lần phát hành (D-300). Chi tiết
thư viện: [`STACK.md`](../STACK.md). Cách làm từng bước: [`docs/steps/`](steps/README.md).

Trạng thái: **bước 0–37 và 40 xong** — đã chuyển sang [`docs/roadmap/done.md`](roadmap/done.md) (D-228);
**kế tiếp bước 51**; bước 50 chờ chủ dự án công khai repo (D-199); bản
phát hành mới nhất `v0.14.0`. CI trên GitHub (`.github/workflows/ci.yml`) chạy gate đầy đủ trên mỗi pull request và mỗi lần push lên `main` (D-175, D-197); `./gate.sh` trên máy vẫn là định nghĩa "xong".

**Số bước là mã cố định, không phải thứ tự** (D-228): bước mang số lớn vẫn có thể làm trước. Các bước dưới đây
xếp theo **thứ tự làm**; bước kế tiếp là bước đầu tiên (từ trên xuống) còn mục `[ ]` mà các bước nó phụ
thuộc đã xong.

## Thứ tự làm (D-228)

| Giai đoạn | Bước, theo thứ tự | Vì sao ở đây |
|---|---|---|
| A. Hợp đồng và nền lõi | 33 → 34 → 35, 36, 37 (song song) → 40 | giữ API không vỡ trước khi sửa mọi bảng; đa khách, tổ chức, tiền, tuân thủ là nền của mọi thứ sau |
| B. Lớp cho giao diện và phân quyền | 51 → 68 → 54 → 52 | module thương mại khai luôn phần của mình (51 nên xong trước 41); 68 (menu của website) sau P6 của 51, trước 41; 54 cần P12 của 51 |
| C. Vận hành | 38 → 39 | báo cáo xuyên database, backup, hiệu năng trước khi có dữ liệu thương mại thật |
| D. Module nền dùng chung | 53, 57, 58, 67, 66, 65 | hỗ trợ khách (45 và 61 dùng), phân tích (64 dùng), tự động hoá, trang dựng từ khối và AI tạo trang, SCIM, chợ ứng dụng |
| E. Thương mại | 41 → 42, 43, 44 → 45 → 46, 47, 48, 49, 55 → 56, 59, 60, 61, 62, 63, 64 | module nghiệp vụ cắm vào core đã xong |
| Chờ chủ dự án | 50 | chỉ làm khi chủ dự án nói repo đã công khai |

Bước lớn nhất là 33 (sửa mọi bảng của mọi module): một người làm, không chia song song. Trong một giai đoạn,
bảng phụ thuộc ngay dưới cho biết bước nào chạy song song được.

## Phụ thuộc giữa các bước (D-192)

| Bước | Bắt đầu khi | Song song được với |
|---|---|---|
| 33 Khách hàng, vị trí dữ liệu | 31 và nhóm A của 32 xong (31 giữ API và bản nâng cấp không vỡ khi mọi bảng đổi) | hai mục độc lập của 40 |
| 34 Tổ chức, quyền, danh tính | 33 | — |
| 35 Gói, thuê bao · 36 Tiền, thuế · 37 Tuân thủ | 34 | nhau; hai mục cuối của 35 chờ 36 và 37 |
| 38 Xuyên database, báo cáo gộp | 34 và 36 | 40, 41–45 (39 làm sau: rà index cả bảng của 38) |
| 39 Vận hành, hiệu năng | 38 | 40, 41–46 |
| 40 Năng lực còn thiếu | header bảo mật và MCP: ngay; còn lại: 34 | 35–39 |
| 41 Sản phẩm | 35 và 36 | 37–40 |
| 42 Kho · 43 Khuyến mại · 44 Vận chuyển | 41 | nhau |
| 45 Giỏ hàng, đặt hàng, đơn hàng | 42, 43, 44 và 37 (thông báo) | — |
| 46 Sàn · 47 Khách hàng · 48 Đa kênh · 49 Tiếp thị liên kết | 45 (46 và 49 cần thêm 37) | nhau |
| 50 Công khai repo | chủ dự án nói đã công khai repo (D-199) | mọi bước |
| 51 Hợp đồng cho giao diện (D-200) | nhúng bản ghi: sau 33; hộp việc, tìm, số liệu, hoạt động: sau 34; menu: sau 35 | 36–49 — nên làm trước 41 để module thương mại khai luôn phần của mình |
| 52 Trải nghiệm quản trị nâng cao (D-201) | 51 | 41–49 |
| 68 Menu của website (D-238) | phần chính: đã làm trước theo lệnh chủ dự án; theo khách: 33; theo website: 34; gỡ menu cũ: sau 2027-05-14 | 51–67 |
| 53 Hỗ trợ khách hàng (D-204) | 37 (thông báo, vụ việc khiếu nại) | 41–49 |
| 54 Phân quyền chuẩn hàng đầu (D-205) | 34; P6, P7 cần P12 của 51; P1 làm ở bước 31 | 41–49, 53 |
| 55 Giao diện cửa hàng (D-219, D-236) | 41 (banner và thông tin cửa hàng làm được từ sau 34); khối vùng banner: 67 | 42–49 |
| 56 Chuyển dữ liệu từ nền tảng khác (D-219) | 47 | 48, 49 |
| 57 Phân tích truy cập (D-220) | 37 (đồng ý), 34 | 41–49 |
| 58 Tự động hoá quy trình (D-220) | 51 (P12 duyệt, P15 job) | 41–49, 57 |
| 59 Bán sỉ B2B (D-221) | 45, 47; ký điện tử: 37; mời thầu: 46 | 48, 49 |
| 60 Đặt lịch dịch vụ (D-221) | 34, 36 (37 cho nhắc lịch) | 41–49 |
| 61 Chat chăm sóc khách (D-221) | 40 (SSE), 53, 37 (thông báo) | 41–49 |
| 67 Trang dựng từ khối, AI tạo trang (D-229, D-231, D-234, D-235) | 51 (gồm P19 cho loại nội dung), 37 (nhãn AI, đồng ý, chân trang pháp lý), 34 (website, múi giờ); thử A/B: 57 | 41–49 (module thương mại khai khối ở bước của nó) |
| 62 Sản phẩm bán theo kỳ (D-221) | 45, 36 | 46–49 |
| 63 Đa tiền tệ (D-221) | 36, 41, 44, 45; người bán nước ngoài: 37, 46 | 47–49 |
| 64 Quảng cáo trong sàn (D-221) | 46, 57 | 47–49 |
| 65 Chợ ứng dụng OAuth (D-221) | 54, 33; bán ứng dụng trả phí: 35, 36 | 41–49 |
| 66 SCIM (D-221) | 34, 40 (OIDC) | 35–49 |

Bước lớn nhất là 33 (sửa mọi bảng của mọi module): một người làm, không chia song song.

## Giai đoạn A — Hợp đồng và nền lõi

Xong cả giai đoạn (33–37, 40): xem [`roadmap/done.md`](roadmap/done.md).

## Giai đoạn B — Lớp cho giao diện và phân quyền

## 51. Hợp đồng cho khung quản trị và mặt tiền (thiết kế giao diện 2, D-200)

Đặc tả: [`docs/steps/51-ui-contracts.md`](steps/51-ui-contracts.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).
Thiết kế: [`docs/research/ui-design.md`](research/ui-design.md).

Giao diện (repo riêng, D-097) dựng menu, hộp việc, tìm kiếm, trang tổng quan từ dữ liệu của core để
cùng một giao diện phục vụ mọi mô hình kinh doanh. Module khai phần của mình (mặc định rỗng, D-038);
kernel gom và lọc theo khách, gói, permission ở nút đang chọn. Kernel không có danh sách phân hệ viết cứng.
Bước 41–49 khai phần của module mình ngay trong bước đó nếu phần tương ứng của bước 51 đã xong.

- [x] Nhúng bản ghi của module khác vào nội dung (P6): node `embed` (`kind`, `id`) của `content`, kiểm qua
      sổ đối tượng (D-123) khi lưu, đọc công khai trả tiêu đề, tóm tắt, đường dẫn, ảnh của đối tượng còn
      công khai (`Resource.image` và `resolve_many` đã có, D-238). `content` không biết khái niệm sản phẩm
- [x] Tham chiếu ngược "đang dùng ở đâu" (D-267): module khai tham chiếu tới đối tượng khác qua sổ đối tượng
      (bài, trang, menu, sản phẩm trỏ tới tệp media…); `GET /media/{id}/usages` và tương tự cho loại khác — xong (18/11,
      D-376): `GET /kernel/records/{kind}/{id}/usages`, nguồn của `content` và `menus`; module thương mại khai khi làm
- [x] Sổ điều hướng (P1): `Module::navigation` (nhóm, mục, permission, tính năng, nguồn số đếm, thứ tự); nhóm
      trùng phải khai giống nhau; menu ẩn khi module chưa bật hay thiếu permission, **khoá (vẫn hiện) khi
      thiếu tính năng của gói**; nhãn theo i18n; ETag; tool MCP
- [x] Hộp việc (P2, P10): `WorkSource` tính từ dữ liệu, không bảng việc, không "Đã xử lý" (D-212); sắp theo
      gấp, hạn; số đếm cho menu (cache có khách trong khoá); nguồn chậm không hỏng hộp; khác hộp thư của
      `notifications`; khoá ổn định, "vì sao bạn thấy", hoãn / bỏ hoãn, giao; `SetupStep` đánh dấu bước chặn bán
- [x] Tìm (P3, P9): `FinderSource` gom theo loại, không dấu vẫn tìm, chỉ nguồn người gọi có quyền, áp phạm vi
      nút; toán tử `is:`, `status:`, so sánh, phủ định; phạm vi `node | subtree | accessible`; `resolve` đổi
      mã (số đơn, mã vận đơn, ID) thành bản ghi người gọi được xem
- [x] Số liệu tóm tắt (P4): `SummaryMetric` (tiền, số đếm, tỉ lệ) theo phạm vi nút, số nặng từ bảng dẫn xuất;
      `definition` (i18n key, mặc định `None`) trả đã dịch, gate kiểm mọi số liệu khai và có bản dịch (D-207)
- [x] Hoạt động (P5, P13): câu đọc được theo i18n key trên `EventInfo`, chỉ bản ghi người gọi đọc được ở nút
      của nó; dòng hoạt động và ghi chú nội bộ trên mọi bản ghi theo loại của sổ đối tượng (D-123), nhắc @
      có thông báo; một chỗ dùng chung thay cho mỗi module tự làm ghi chú — xong (17/11): `/kernel/records/{kind}/{id}/
      notes|activity`; nhắc @ phát event `kernel.record_note_added`, `notifications` gửi tin trong ứng dụng (18/11, D-377)
- [x] Schema bộ lọc mỗi danh sách công bố trong OpenAPI (máy, agent dựng chip lọc) và view đã lưu `/me/views`
      (P7): tham số, sắp xếp, cột, nhóm, mật độ; phạm vi cá nhân / nút / khách; bộ lọc nội dung `review`,
      `scheduled`, `ai_generated`, `missing_locale`, `missing_alt`
- [x] Job và hàng loạt (P8, P15, P16; D-213): job API chung (danh sách, tiến độ, file, chạy lại có audit);
      module khai hành động hàng loạt, nhận `ids[]` hoặc `filter` + `expected_count`, `Idempotency-Key`, kết
      quả từng bản ghi, tiến độ qua SSE; xuất danh sách theo bộ lọc thành CSV qua job; job xong thì báo người yêu cầu
      (`kernel.job_finished` → tin trong ứng dụng, D-377)
- [x] Giải thích quyền (P11): `POST /me/can` nhiều cặp (permission, nút), lý do `missing_permission` /
      `module_not_enabled` / `feature_not_in_plan` / `quota_exceeded` — giao diện hiện vì sao bị khoá, ai cấp
      được, nâng gói mở gì — **một phần** (17/11): `POST /me/can` với `unknown_permission`, `module_not_enabled`
      (`required_module`), `missing_permission`, cùng hàm kiểm với request thật. Còn: `feature_not_in_plan` /
      `quota_exceeded`, `upgrade_to` — xong (18/11, D-376) với `feature` / `limit` trong câu hỏi. `grantors` (ai cấp được)
      làm cùng mục "giải thích và mô phỏng" của bước 54
- [x] Agent là người dùng thật (P12): API key `agent`; `ActorKind` chung cho audit, nhật ký, event (D-224);
      lệnh rủi ro (hoàn tiền, chi trả, khoá sổ, xoá dữ liệu) agent gọi trả `202 approval_required`, yêu cầu
      duyệt có hạn, đủ duyệt / từ chối / event / tool MCP, như nhau REST và MCP; việc chờ vào hộp việc — xong
      (17/11, D-365): `x-approval: "agent"`, `/kernel/approvals`, gửi lại kèm `Autoxide-Approval`; MCP từ chối agent
      và chỉ sang REST; (18/11) yêu cầu chờ vào hộp việc của người quyết được (nguồn `kernel.approvals`), tool MCP
      `kernel.list_approvals` / `kernel.decide_approval`
- [x] Cờ theo người dùng cho trợ giúp trong giao diện (P14, D-207): `/api/v1/me/flags` (`kernel_user_flags`),
      chỉ cờ của mình, hạn mức `AUTOXIDE_USER_FLAGS_MAX`, có trong bản xuất dữ liệu, xoá khi xoá tài khoản;
      `tenant_id` thêm ở bước 33 cùng mọi bảng
- [x] Danh sách lý do theo khách (P18, D-223); cài đặt nối ra ngoài có `last_used_at`, `last_error` và thử
- [x] Trường tự định nghĩa trên bản ghi (P19, D-235): module khai đích, khách thêm trường (kiểu, nhãn, bắt buộc,
      mức hiện `internal` / `public` / `buyer_input`), giá trị nằm trên bản ghi, lọc qua bảng index của module ghi
      cùng transaction, dùng chung cho xuất / nhập / tự động hoá; hạn mức theo gói; test trên cả hai database
      — xong phần lõi (17/11): `kernel_custom_fields`, `/kernel/custom-fields`, `CustomFields::validate` / `write_index` /
      `condition`, `content.entry` (giá trị, lọc `cf.<khoá>`); (18/11, D-373) cột `cf.<khoá>` trong xuất, mức `public` chép vào
      bản công khai của bài, hạn mức `kernel.custom_fields` theo gói, tool `kernel.list_custom_fields`. Cột `cf.` khi nhập
      thuộc bước 56; trường `personal_data` áp khi có loại bản ghi gắn với một người (khách hàng, đơn — bước 41–49)
- [x] Nhãn gắn bản ghi (P20, D-235): nhãn nội bộ dùng chung mọi loại bản ghi, lọc `tags_any` / `tags_all`, gắn
      hàng loạt, gộp nhãn, event cho tự động hoá — xong (17/11); gắn / gỡ hàng loạt (18/11): `POST /kernel/tags/{id}/apply`
      (tối đa 500 bản ghi, chọn theo bộ lọc thì lấy id bằng `dry_run` của thao tác hàng loạt)
- [x] Đổi câu chữ hiển thị theo khách (P21, D-235): chỉ câu module cho đổi và không gian `ui.*`, giữ chỗ điền, không
      đổi mã lỗi và câu pháp lý, đọc qua cache không query
- [ ] Nhập dữ liệu (P17, D-220): không làm ở kernel — `imports` (bước 56) thêm nguồn `autoxide` cùng cột với
      xuất chung và đích `content`, `media`, `newsletter` (người đăng ký chỉ khi có bằng chứng đồng ý)
- [x] Khai menu, nguồn việc, nguồn tìm, số liệu cho mọi module đang có; MODULE_GUIDE (EN + VI) mục "khai phần
      của module cho giao diện"; test rò rỉ giữa hai nhánh anh em cho số đếm và kết quả tìm

## 68. Menu của website (`menus`, chủ dự án quyết ngày 14/11/2026, D-238)

Đặc tả: [`docs/steps/68-menus.md`](steps/68-menus.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).
Đề bài: [`docs/prompts/menus-module.md`](prompts/menus-module.md).

Menu của website (thanh danh mục, bảng xổ lớn, chân trang) là module riêng: mục menu trỏ tới địa chỉ hay tới bản ghi
của mọi module qua sổ đối tượng. Không nhầm với sổ điều hướng (P1 của bước 51), là menu của trang quản trị. Phần
chính làm trước bước 33 theo lệnh của chủ dự án; phần theo khách, theo website làm cùng bước 33, 34.

- [x] Kernel mở rộng sổ đối tượng theo D-038: `Resource.image`, `ResourceRef`, `resolve_many` (tra theo lô, theo
      ngôn ngữ), `has_children`, `children`; `content` đăng ký `content.term` (có cây) và tra `content.entry` theo lô
- [x] Module `menus`: menu theo `key` và ngôn ngữ; mục trỏ `url` / đối tượng / nhóm tiêu đề; tự lấy nhóm con có giới
      hạn; biểu tượng, làm nổi, nhãn có hạn, ô gợi ý; 4 tầng, 200 mục; lỗi gom một lần với mã riêng
- [x] API quản trị (`menus.read`, `menus.manage`, `If-Match` bắt buộc khi thay cây) và tool MCP dùng chung service;
      API công khai trả cây đã giải, bỏ đối tượng không còn công khai, thiếu ngôn ngữ thì dùng ngôn ngữ mặc định,
      `ETag` + cache trong bộ nhớ có thời hạn (rút tới mốc nhãn bật / tắt)
- [x] Chuyển từ menu của `content`: bốn endpoint cũ deprecated (sunset 2027-05-14), lệnh
      `menus.import_content_menus` chạy lại không nhân đôi, `menus` không đọc bảng của `content`
- [ ] Gỡ menu cũ của `content` khi qua 2027-05-14: bốn endpoint, trait `LegacyMenus`, bảng `content_menus` và
      `content_menu_items` (migration mới), mục menu trong `docs/CONTENT.md`

## 54. Phân quyền chuẩn hàng đầu (chủ dự án giao ngày 13/11/2026, D-205)

Đặc tả: [`docs/steps/54-authorization.md`](steps/54-authorization.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 34. Mô hình: RBAC theo cây tổ chức có độ sâu (bước 34) + chia sẻ bản ghi một bước + deny
và rào + điều kiện theo ngữ cảnh — luật là dữ liệu có kiểu, không ngôn ngữ chính sách tự do. Đối chiếu với
Google Zanzibar, AWS IAM, GitHub, Microsoft Entra, Salesforce, Shopify, Stripe và chuẩn OpenID AuthZEN.

- [x] Một hàm quyết định duy nhất, thu hồi tức thì (quyền không nằm trong token); test so khớp "lọc danh sách =
      kiểm từng bản ghi" sinh ngẫu nhiên trên cả hai database (D-20261118-step54-one-decision)
- [ ] Cache quyền theo phiên bản quyền của khách (P11): chỉ làm khi benchmark bước 39 cho thấy câu nạp quyền
      đáng kể (D-20261118-step54-one-decision)
- [ ] Deny thắng allow trên role; rào quyền tối đa cho khách con (kể cả admin của khách con); API key chi
      tiết: hạn bắt buộc, chọn tài nguyên, dải IP, lần dùng cuối, xoay vòng, quản trị duyệt key — đã có: deny
      thắng allow (D-20261118-step54-role-denies); hạn bắt buộc, dải IP (D-20261119-step54-api-key-limits)
- [ ] Xác thực lại cho việc nhạy cảm (passkey / MFA gần đây, RFC 9470); điều kiện theo IP và khung giờ
      trên gán role
- [ ] Chia sẻ bản ghi có hạn cho người, nhóm, nút, key (module khai quan hệ `viewer` / `editor`…), không
      trao quyền mình không có
- [ ] Quyền theo trường dữ liệu: trường cá nhân / nhạy cảm / tài chính phân loại trong OpenAPI, bị che khi
      thiếu `<module>.read_personal`, đọc dữ liệu nhạy cảm có audit (Luật 91/2025) — áp cho `auth`,
      `comments`, `forms`, `newsletter`
- [ ] Nâng quyền đúng lúc (role đủ điều kiện, kích hoạt có lý do, hạn, duyệt); tài khoản khẩn cấp; lệnh hai
      người và cặp quyền xung đột do module khai, ngưỡng là dữ liệu
- [ ] Giải thích và mô phỏng (vì sao có / không có quyền, thử role nháp), "ai có quyền trên bản ghi này",
      quyền không dùng 90 ngày, rà soát quyền định kỳ vào hộp việc
- [ ] Endpoint theo chuẩn OpenID AuthZEN Authorization API 1.0; agent hành động thay người dùng có vết
      `on_behalf_of`; số đo quyết định quyền; hướng dẫn `docs/ACCESS.md` (EN + VI)
- [ ] Khách duyệt từng lần khi nhân sự nền tảng xin vào dữ liệu (D-232): `per_request` mặc định, tối đa 4 giờ,
      duyệt qua yêu cầu duyệt, `legal_request` như D-211
- [ ] "Xem như" người dùng chỉ đọc thay cho đăng nhập thay (D-232): người mua phải cấp phép, không xem người
      quyền cao hơn, mọi lệnh ghi bị chặn kể cả qua MCP, không trả secret, audit hai id

## 52. Trải nghiệm quản trị nâng cao (D-201)

Đặc tả: [`docs/steps/52-admin-experience.md`](steps/52-admin-experience.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 51. Những điều nên có theo nghiên cứu các trang quản trị hàng đầu (Stripe, Shopify,
Linear, GitHub, Odoo); mỗi mục chạy và có test trên cả hai database.

- [x] Bàn kỹ thuật: nhật ký request theo khách (API key, đường dẫn mẫu, status, `code`, `request_id` /
      `trace_id`, có hạn lưu, không lưu thân request); lần giao webhook kèm gửi lại; event theo bản ghi đọc
      từ audit; chạy lại job trong dead-letter queue; `kernel.devtools.read` / `kernel.devtools.manage`
      - [x] `GET /webhooks/deliveries?status=`, `POST /webhooks/deliveries/{id}/redeliver` (+ tool MCP, event audit);
            `GET /kernel/devtools/events?object=` đọc từ audit; chạy lại job hỏng đã có từ P15 (`POST /kernel/jobs/{id}/retry`,
            event `kernel.job_retried` vào audit). Còn: tool MCP cho event theo bản ghi.
- [x] Nhật ký lệnh của agent (D-202): lệnh MCP ghi `mcp:<tên tool>`, không ghi tham số; lọc theo
      `actor_kind` (đủ `ActorKind`, D-224) và theo key; dòng của lệnh `202 approval_required` giữ `approval_id`
      - [x] Bảng `kernel_request_log`, ghi bất đồng bộ, `GET /kernel/devtools/requests`, tool `kernel.request_log`,
            lệnh MCP `mcp:<tool>`, `approval_id`, hạn lưu `AUTOXIDE_REQUEST_LOG_DAYS` (D-370)
- [x] Hoàn tác an toàn: lưu trữ / ẩn có `restore`; lệnh ghi lớn có `dry_run` trả trước những gì sẽ đổi
      - `POST /content/entries/{id}/unarchive`; media đã có `restore`, bình luận và người đăng ký bản tin có thao tác ngược
        (`approve` / `unblock`); `dry_run` ở thao tác hàng loạt (P8), nhập menu cũ. Nhập CSV (bước 56) và đổi giá hàng loạt
        (thương mại) thêm `dry_run` khi làm tới.
- [ ] AI trong quản trị (D-109): lời thường thành bộ lọc đã kiểm theo schema (`ai/filters/parse`); trợ lý chạy
      tool MCP với quyền người gọi, mọi lệnh ghi là bản nháp hay yêu cầu duyệt, trả kèm tool đã gọi
      - [x] `POST /kernel/filters/parse`: lời thường thành bộ lọc đã kiểm theo schema (D-372). Còn: trợ lý.
- [ ] Thông báo: lý do nhận, trạng thái xong / lưu, bộ lọc, cấu hình kênh theo loại event; ghim màn hình hay
      dùng (`/me/pins`)
      - [x] `/api/v1/me/pins`: ghim màn hình hay bản ghi, tối đa 20, xếp lại, dữ liệu cá nhân
- [ ] Sức khoẻ người bán và sức khoẻ tuân thủ: chỉ số so với ngưỡng (ngưỡng là dữ liệu có ngày hiệu lực), vi
      phạm, hạn phải xử lý; huy hiệu người bán tin cậy tính từ số liệu thật, công bố tiêu chí
- [ ] Quản trị trên điện thoại (D-232): gợi ý `mobile` trên sổ điều hướng (tối đa 5 mục thanh đáy); tóm tắt
      cuối ngày tự bật theo giờ của người dùng (số liệu, việc gấp, cảnh báo bất thường)
      - [x] Gợi ý `mobile` trên `NavItem`, `GET /kernel/navigation?device=mobile`, tool MCP nhận `device` (D-369)

## Giai đoạn C — Vận hành

## 38. Cây khách hàng xuyên database và báo cáo gộp (D-185)

Đặc tả: [`docs/steps/38-cross-database-reporting.md`](steps/38-cross-database-reporting.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Thiết kế: trụ 2, phần xuyên database. Bắt đầu SAU bước 34 và 36 (báo cáo gộp cần số liệu tiền).

- [x] Kho báo cáo gộp: event của các khách con đổ về khách cha (hay một database báo cáo) gần thời
      gian thực, mang `tenant_id`, `org_id`; báo cáo gộp nhiều cửa hàng, nhiều tiền tệ; người có quyền
      ở cha mới xem — xong (18/11, D-20261118-step38-report-facts): kernel `report` + `Module::facts`, module
      `reporting` (rút ở database nguồn, lịch đẩy lên kho ở database của khách gốc, cộng dồn ngày, dọn),
      `invoices` / `ledger` khai số liệu, `/api/v1/reporting/*` + tool MCP, phạm vi theo cây khách và cây tổ chức,
      quy đổi theo tỉ giá của người xem; test cha / con khác database trên cả hai database
- [x] Cha thao tác trên con: vào ngữ cảnh con bằng cùng API (uỷ quyền của bước 34); job phân phối dữ
      liệu cha → con (vd danh mục, bảng giá) gọi service của con, chạy lại được, không trùng — xong (18/11,
      D-20261118-step38-distribution): test đầu-cuối uỷ quyền vào con có database riêng (audit hai phía, thu hồi);
      kernel `tenancy::distribution` (`Distributor`, `DistributionApply`, `distribution_migration`) với module mẫu
- [x] Tách một nhánh tổ chức thành khách riêng (database riêng) và gộp ngược lại, giữ ID và lịch sử;
      test trên cả hai database — xong (18/11, D-20261118-step38-relocation): kernel `tenancy::relocate` (chính sách theo
      bảng `Module::relocation` cho mọi module, kế hoạch / dry-run với xung đột tham chiếu và unique, chép có lọc + đổi
      `tenant_id` giữ khoá chính, trạng thái chạy lại được, chuyển mặt điều khiển, dọn, file theo khách), audit ghi lại vào
      chuỗi của đích, `reporting.rehome`, CLI `autoxide tenant split|merge|relocation-status`; test khứ hồi so mọi bảng
- [x] Đối chiếu thiết kế 2 (D-210): tách / gộp khách qua API chỉ cho nhân sự nền tảng, khách chỉ gửi yêu cầu — xong
      (18/11, D-20261118-step38-relocation): `POST /api/v1/kernel/relocations` (dry-run mặc định, job) +
      `GET …/{id}`; khách gửi `POST /api/v1/kernel/tenant/relocation-requests`, yêu cầu vào hộp việc của nhân sự nền tảng
- [ ] Khách thử nghiệm (sandbox, D-232): sao cấu hình không sao dữ liệu cá nhân, đối tác buộc dùng bộ giả, mail
      chỉ tới nhân viên, `noindex`, tự hết hạn; đẩy trang / theme / mẫu / luật sang khách thật thành bản nháp
- [ ] Nhóm nút ngoài cây cho báo cáo (D-232): nhóm tự do của `organizations`, nhóm không cấp quyền

## 39. Vận hành và hiệu năng (chủ dự án giao ngày 13/11/2026; số cũ 33, D-189)

Đặc tả: [`docs/steps/39-operations-performance.md`](steps/39-operations-performance.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Khi đã có người dùng thật: biết sớm khi hệ thống kẹt, không mất dữ liệu, xử lý sự cố không phải
đoán, và chịu được dữ liệu cỡ thật. Bắt đầu SAU bước 38: backup, số đo, cảnh báo
theo từng khách hàng và từng database (nền đa khách của bước 33); sau 38 để rà index phủ cả bảng của 33–38. Làm nhóm A trước: số đo tồn đọng dùng để quan sát lúc chạy tải ở nhóm B. Mọi số đo ghi kèm máy,
ngày, cỡ dữ liệu; mọi thay đổi chạy và có test trên cả hai database; không đổi tính đúng lấy tốc độ.

Nhóm A — vận hành:

- [x] Backup tự động theo lịch: job định kỳ (qua `Schedule` của kernel) tạo bản backup như
      `autoxide backup` (`crates/app/src/commands/backup.rs`) rồi ghi vào `Storage` (ổ đĩa hay
      S3), giữ N bản gần nhất, lịch và số bản cấu hình qua `Setting` (sinh lại `.env.example`);
      bản hỏng thì ghi log lỗi và đếm bằng metric. `autoxide doctor` cảnh báo khi bản mới nhất cũ
      hơn ngưỡng. Không giữ transaction ghi trong lúc backup (dùng `Database::begin_snapshot`)
- [x] Diễn tập restore: test đầu-cuối backup → restore vào database trống → chương trình khởi
      động, dữ liệu và file khớp, trên cả hai database (cả chép chéo SQLite ↔ Postgres nếu
      `autoxide copy` hỗ trợ); `docs/DEPLOY.md` (EN + VI) có quy trình restore từng bước
- [x] Số đo tồn đọng (gauge, cập nhật định kỳ, tên có tiền tố như luật code): job đang chờ và tuổi
      của job chờ lâu nhất theo module, job trong dead-letter queue, event chưa giao và tuổi của
      event cũ nhất, kết nối database đang dùng / còn rảnh. Hiện chỉ có bộ đếm cộng dồn
      (`jobs_failed_total`, `events_failed_total`…), không thấy được hàng đợi kẹt
- [x] Dashboard và cảnh báo có sẵn: file dashboard Grafana và luật cảnh báo Prometheus trong repo
      (tỉ lệ 5xx, độ trễ p95, job chờ quá lâu, dead-letter tăng, event tồn, hết kết nối database,
      backup quá cũ), mỗi cảnh báo kèm câu "làm gì"; hướng dẫn dùng trong `docs/DEPLOY.md` (EN + VI)
- [x] Xoay khoá không gián đoạn: `AUTOXIDE_AUTH_TOKEN_SECRET` (và các khoá ký khác) nhận thêm khoá
      cũ trong một khoảng chuyển tiếp — ký bằng khoá mới, kiểm bằng cả hai — để đổi khoá (định kỳ
      hay khi bị lộ) không đăng xuất mọi người; test; quy trình trong `docs/DEPLOY.md`
- [x] Sổ tay xử lý sự cố trong `docs/DEPLOY.md` (EN + VI): hàng đợi kẹt, job trong dead-letter
      (`autoxide jobs retry`), database đầy, lộ khoá, mất admin cuối cùng (`autoxide user …`),
      nâng cấp khi nhiều máy chủ chạy hai phiên bản code cùng lúc (migration theo kiểu mở rộng
      trước, thu hẹp sau). Mỗi tình huống: dấu hiệu, lệnh kiểm, cách sửa

Nhóm B — hiệu năng ở quy mô thật (bài đo hiện có chạy trên database gần như trống, chỉ đi qua
`notes` và `auth`):

- [x] Bộ dữ liệu lớn để đo: một lệnh (vd trong `xtask` hay `scripts/`) sinh nhanh dữ liệu cỡ thật
      trên cả hai database — vd 100 nghìn bài nhiều ngôn ngữ có danh mục, thẻ, tác giả; 50 nghìn
      ảnh kèm bản nhỏ; 1 triệu bình luận; vài nghìn người dùng — chỉ ghi qua service hay đúng định
      dạng bảng của module, không dùng cho dữ liệu thật
- [x] Đo tải đường đọc công khai trên bộ dữ liệu đó, mở rộng `scripts/loadtest.sh`: một bài
      (`/public/resolve`), danh sách bài và theo danh mục (`/public/posts`, `/public/terms/…`),
      `/public/sitemap`, `/public/feed`, `/public/search`, ảnh và bản nhỏ, bình luận của bài
      (`/public/threads`, `/public/comments`). Đặt ngưỡng p95 mỗi đường, ghi vào `docs/BENCHMARKS.md` — đo 19/11/2026: lỗi thống kê SQLite đã sửa (D-20261118-step39-sqlite-optimize); đường còn đỏ (lọc
      theo danh mục, tìm theo từ của `content`; danh sách đầy đủ trên SQLite) ghi ở BENCHMARKS, báo phiên giữ `content`
- [ ] Rà index bằng `EXPLAIN` (SQLite `EXPLAIN QUERY PLAN`, Postgres `EXPLAIN ANALYZE`) cho mọi
      truy vấn danh sách, lọc, đếm của các module trên bộ dữ liệu lớn; index thiếu thêm bằng
      migration mới (không sửa migration cũ); ghi bảng truy vấn → kế hoạch trước / sau
- [x] Nén response (gzip, brotli): đo cỡ và thời gian trên JSON thật (danh sách, `openapi.json`
      hơn 1 MB), rồi quyết nén trong app (`tower-http`) hay để reverse proxy làm; không nén lại
      file đã nén (ảnh). Ghi quyết định; sửa `docs/DEPLOY.md` (EN + VI) theo — nén trong app, mức 1, D-20261119-step39-compression
- [x] Cache cho CDN: `Cache-Control`, `ETag`, `Vary` đúng cho mọi endpoint công khai; file media có
      địa chỉ mang phiên bản thì cache dài hạn (`immutable`); nội dung riêng tư hay xem trước không
      bao giờ được cache chung; hướng dẫn đặt CDN trước Autoxide trong `docs/DEPLOY.md` (EN + VI) — lớp kernel `http/cache.rs`, D-20261119-step39-cdn-cache
- [x] Tìm theo ý nghĩa khi dữ liệu lớn: đo cách duyệt hết vector hiện tại trên bộ dữ liệu lớn so
      với ngưỡng 50 ms của D-110 (`docs/BENCHMARKS.md`, mục "Tìm theo ý nghĩa"); vượt ngưỡng thì
      chọn cách giảm (vector nhỏ hơn, chia nhiều nhân, hay index sau một trait) chạy được trên cả
      hai database, ghi quyết định; không vượt thì ghi số đo và giữ nguyên — vượt (137 ms); `dot` SIMD + chia nhân còn
      20 ms, D-20261119-step39-semantic-scan
- [x] Quan sát theo khách (D-190): log và trace mang trường `tenant_id`; **không** dùng `tenant_id`
      làm nhãn số đo (trái luật code về nhãn); lượng dùng theo khách lấy từ số đo dùng của bước 35
- [ ] Ảnh AVIF (D-232): đo so với WebP trên bộ ảnh mẫu; chỉ thêm (job, tắt mặc định) khi nhẹ hơn ≥ 20% ở chất
      lượng tương đương, không thì ghi "không làm" kèm số đo
- [x] Rút số câu SQL của `GET /auth/me`, `auth.whoami`, `GET /auth/users` (D-302: 3 → 6 câu cho một `GET /auth/me`,
      chậm 35–46%; bước 34 thêm một câu kiểm role bắt buộc MFA, D-317): lấy `has_password`, `last_sign_in_at` từ chính
      dòng người dùng đã đọc, gộp cờ MFA, role bắt buộc MFA và email đang chờ đổi vào một câu; đo lại bằng `scripts/loadtest.sh`, mục tiêu trong ±20% của mốc trước bước 33 — xong trước bước 39 theo lời chủ dự án: 7 → 3 câu, +172% (D-319)
- [x] Login throttling trên Postgres khi bị dò dồn dập một email (D-302: 8500 → 232 request/s cho lượt bị chặn): bộ
      đếm cập nhật một dòng bằng một câu thay vì nhiều câu `UPDATE` nối tiếp; trả 429 không cần khoá dòng khi đã bị khoá — xong trước bước 39 theo lời chủ dự án: lượt bị chặn vốn nhanh, số cũ là bài đo lẫn; đếm một câu (D-321)

## Giai đoạn D — Module nền dùng chung

Module trung lập với nghiệp vụ, không phải core: `support`, `analytics`, `automations`, `apps`; SCIM nằm trong `auth`.

## 53. Hỗ trợ khách hàng (`support`) (chủ dự án giao ngày 13/11/2026, D-204)

Đặc tả: [`docs/steps/53-support.md`](steps/53-support.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 37; song song được với 41–49. Phiếu hỗ trợ; chat thời gian thực là bước 61
(D-221). Khiếu nại có hạn luật định vẫn ở `compliance` (bước 37), tranh chấp ở `marketplace` (bước 46):
`support` chuyển phiếu sang đó, không làm lại.

- [x] Phiếu và tin nhắn: tạo từ website (có tài khoản; vãng lai qua OTP email / số điện thoại; từ token
      xem đơn của bước 45), nhân viên ghi hộ; trạng thái qua một máy trạng thái; tin nhắn chỉ thêm, ghi
      chú nội bộ có nhắc tên, đính kèm qua `media`; người mua vãng lai xem bằng token phiếu, gộp phiếu vào
      tài khoản sau khi đăng ký
- [x] Phạm vi theo nút tổ chức (bước 34): nhân viên thấy phiếu của nút mình và nhánh dưới; người bán trên
      sàn chỉ thấy phiếu gian hàng mình; nhóm nhân viên, quy tắc tự phân công là dữ liệu, chia vòng
- [x] Hạn trả lời (SLA) theo ưu tiên, tính theo giờ làm việc và ngày nghỉ của nút, dừng khi chờ người
      mua; job đánh dấu quá hạn; nguồn việc cho hộp việc (bước 51); báo cáo thời gian phản hồi, tỉ lệ quá
      hạn, mức hài lòng
- [x] Gắn phiếu với đơn, sản phẩm qua sổ đối tượng (D-123); câu trả lời mẫu có biến; chuyển phiếu thành
      khiếu nại người tiêu dùng của `compliance` (gọi lại không tạo hai vụ việc) hay lên sàn
- [x] Tool MCP (cùng permission như REST); gợi ý trả lời bằng AI chỉ là bản nháp (D-109); email trả lời
      vào lại phiếu qua trait `InboundMail` (tắt mặc định); hướng dẫn `docs/SUPPORT.md` (EN + VI); test
      trên cả hai database
- [x] Trả lời tự động câu hỏi đơn giản (D-230): chỉ gửi câu trả lời mẫu đã duyệt hay mẫu câu dựng từ dữ liệu
      chính xác (`FactSource` của module sau), mô hình chỉ phân loại; luật quyết trong Rust, không bao giờ tự động
      cho khiếu nại / hoàn tiền / dữ liệu cá nhân; tắt mặc định, nhãn và "gặp nhân viên"; câu phức tạp → nháp

## 57. Phân tích truy cập và hành vi (`analytics`) (chủ dự án giao ngày 14/11/2026, D-220)

Đặc tả: [`docs/steps/57-analytics.md`](steps/57-analytics.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 37. Số liệu truy cập ngay trong Autoxide, không gửi dữ liệu người đọc cho bên thứ ba.

- [x] Thu sự kiện từ website (endpoint công khai, gom lô, chống bot) và từ event của module; loại sự kiện do
      module khai; phiên ẩn danh không cookie với khoá xoay mỗi ngày, không lưu IP hay user agent đầy đủ
- [x] Gộp theo giờ / ngày, báo cáo chỉ đọc bảng gộp: tổng quan, trang, nguồn, chiến dịch UTM, đối tượng xem
      nhiều, phễu do module khai; số liệu cho trang tổng quan (bước 51) và "thịnh hành" cho `catalog`
- [x] Theo dõi theo người chỉ khi đồng ý (sổ đồng ý bước 37), rút đồng ý thì xoá phần gắn người; hạn lưu là
      cấu hình; tool MCP chỉ đọc; phạm vi theo website của nút; test trên cả hai database

## 58. Tự động hoá quy trình (`automations`) (chủ dự án giao ngày 14/11/2026, D-220)

Đặc tả: [`docs/steps/58-automations.md`](steps/58-automations.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 51 (lệnh chờ duyệt, job API). Luật "khi … thì …" như Shopify Flow; mỗi hành động là một tool
MCP đã có, chạy với quyền của người tạo luật.

- [x] Luật: kích hoạt bằng event, lịch hay gọi tay; điều kiện là cây toán tử cố định (không biểu thức tự do);
      hành động là tool MCP có tham số điền từ event; lưu luật kiểm event, tool, quyền, tham số một lần
- [x] Chạy bằng job ngoài transaction, nhận lại event không chạy hai lần, mỗi bước có `Idempotency-Key`; hành
      động rủi ro chờ duyệt; chống vòng lặp và giới hạn số lần chạy; nhật ký từng lần chạy, chạy lại
- [x] Chạy thử khô, danh mục kích hoạt và hành động người gọi dùng được, mẫu luật do module khai; agent tạo
      luật thì chờ duyệt; người tạo mất quyền thì luật tự tắt; test trên cả hai database
- [x] Cảnh báo bất thường (D-229): kích hoạt theo số liệu tóm tắt so với ngưỡng hay mức nền 4 tuần (doanh thu
      tụt, sắp hết hàng theo tốc độ bán, tỉ lệ bom hàng tăng), thống kê tất định, một lần báo mỗi kỳ

## 67. Trang dựng từ khối, AI tạo trang, bộ giao diện website (chủ dự án giao ngày 14/11/2026, D-229)

Đặc tả: [`docs/steps/67-page-builder.md`](steps/67-page-builder.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 51 và 37. Kéo thả là giao diện ở repo riêng; core cho dữ liệu trang và API. AI chỉ ghép khối có
sẵn, không sinh code; kết quả luôn là bản nháp.

- [ ] Sổ khối của kernel: `Module::blocks()` (tên, nhóm, schema props, gợi ý cho AI, tính năng của gói),
      `BlockData` trả dữ liệu theo lô; `GET /kernel/blocks` chỉ khối của module đang bật
- [ ] Trang có thân `layout` (cây khối JSON) cạnh thân `rich`: kiểm từ chối mặc định, theo cỡ màn hình, theo ngôn
      ngữ; sửa từng phần bằng JSON Patch + `If-Match`; khoá sửa; nháp, lịch sử, xem trước, hẹn giờ dùng lại `content`
- [ ] Khối nền (bố cục, chữ, media, FAQ, thanh thông báo, đếm ngược, popup thu email có đồng ý) và khối của
      `forms`, `newsletter`, `comments`, `media`, `menus` (`menus.menu`, D-238); chân trang pháp lý tự gắn cho
      website bán hàng
- [ ] Đọc công khai trả bố cục kèm dữ liệu khối, một lần gọi mỗi loại khối, khối lỗi không hỏng trang, ETag;
      đo p99 trang 50 khối trên cả hai database
- [ ] Bộ giao diện của website (token màu, chữ, khoảng cách, kế thừa theo nút, kiểm tương phản), khu dùng chung
      nhiều trang, thư viện trang mẫu theo ngành
- [ ] Chủ đề theo mùa, ngày lễ, khung giờ (D-234): bản chủ đề ghi đè một phần theme gốc theo lịch (một lần, hằng
      ngày, hằng tuần, theo múi giờ của website), ưu tiên khi trùng, thư viện chủ đề mẫu, chép sang năm sau, xem
      trước ở một thời điểm; hiệu ứng trang trí chỉ trong danh sách cho phép, không chạy ở trang thanh toán
- [ ] Đầu trang và chân trang của cả website (D-234): bố cục khối có nháp / đăng, theo ngôn ngữ, kế thừa theo nút,
      bản chủ đề kèm được bản riêng; `GET /public/site` một lời gọi cho khung website, cache tới đúng mốc đổi chủ
      đề, không query khi cache nóng; đo p99 trên cả hai database
- [ ] Loại nội dung tự định nghĩa (D-235): khách tạo loại (đội ngũ, dự án, chi nhánh…) với trường riêng của bước
      51, mục dùng lại nháp / lịch sử / bản dịch / SEO, trang chi tiết dựng sẵn, khối danh sách, API công khai có lọc
- [ ] AI tạo trang và sửa tiếp bằng trò chuyện (job, kết quả qua bộ kiểm, thử lại có giới hạn, bản nháp
      `ai_assisted`, lịch sử trò chuyện để hoàn tác); bộ nháp chiến dịch từ một chương trình khuyến mại
- [ ] Kiểm tra sức khoẻ website vào hộp việc: thiếu SEO / alt, link gãy, khối tham chiếu bản ghi đã ẩn, đường dẫn
      404 nhiều → gợi ý chuyển hướng; tool MCP như REST; hướng dẫn EN + VI cho người viết module khai khối
- [ ] Thử A/B hai phiên bản trang (D-231, sau bước 57): chia cố định theo người xem (F5 không đổi), bot thấy bản
      gốc, chỉ thử bố cục và nội dung (không thử giá), số liệu và độ tin cậy, người chọn bản thắng
- [ ] AI tạo ảnh (D-231): `ImageModel` mặc định tắt, chỉ banner / ảnh nền / minh hoạ, cờ `ai_generated` và nhãn,
      `catalog` chặn làm ảnh sản phẩm
- [ ] Khối mã tự viết (D-231): mặc định tắt; mức cách ly (iframe `sandbox`, CSP theo host) chủ cửa hàng bật; mức
      không cách ly cần cả máy chủ và chủ cửa hàng bật, không trên trang thanh toán / tài khoản; audit, sổ script

## 66. Đồng bộ nhân viên qua SCIM 2.0 (chủ dự án giao ngày 14/11/2026, D-221)

Đặc tả: [`docs/steps/66-scim.md`](steps/66-scim.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 34 và 40 (đăng nhập OIDC).

- [ ] Endpoint SCIM 2.0 trong `auth` (Users, Groups, bộ lọc, PATCH theo RFC 7644) với token SCIM riêng mỗi khách
- [ ] Nhân viên `active=false` thì khoá và thu hồi phiên, API key ngay; nhóm ánh xạ sang role ở nút; test trên
      cả hai database

## 65. Chợ ứng dụng qua OAuth 2.0 (`apps`) (chủ dự án giao ngày 14/11/2026, D-221)

Đặc tả: [`docs/steps/65-apps.md`](steps/65-apps.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 54 và 33. Ứng dụng bên ngoài cắm vào như Shopify App Store.

- [ ] Máy chủ ủy quyền OAuth 2.0 trong `auth`: mã ủy quyền + PKCE bắt buộc, refresh token xoay vòng, phạm vi là
      tên permission, thu hồi và kiểm tra token, siêu dữ liệu máy chủ (RFC 7009, 7636, 7662, 8414)
- [ ] Đăng ký ứng dụng, nền tảng duyệt ứng dụng công khai, khách cài / đồng ý / gỡ (token mất hiệu lực ngay),
      webhook theo ứng dụng, audit `actor_kind = app`; test trên cả hai database
- [ ] Bán ứng dụng trả phí (D-232): thuê bao tháng, dùng thử; nền tảng là bên bán (hoá đơn thuê bao của 35), chia
      doanh thu cho nhà phát triển qua `PayoutGateway` (36); nhà phát triển nước ngoài chờ đối chiếu thuế

## Giai đoạn E — Module thương mại (chủ dự án giao ngày 13/11/2026, D-191)

Module nghiệp vụ, không phải core: khái niệm sản phẩm, đơn hàng, kho không vào `kernel`. Thiết kế:
`docs/research/business-foundation.md`, mục 8. Luật chung cho bước 41–49:

- Mỗi module một crate, sở hữu bảng của mình; mọi bảng có `tenant_id`, dữ liệu theo cửa hàng / kho /
  người bán có `org_id`; quyền theo độ sâu của bước 34. Tiền là `Money`; thuế qua engine thuế, mọi
  chứng từ lưu ảnh chụp giá, thuế, thông tin sản phẩm lúc giao dịch.
- Nhất quán giữa module bằng **giữ chỗ → xác nhận → trả lại** (tồn kho, mã giảm giá, hạn mức): giữ
  chỗ có hạn và mã tham chiếu (vd `order_id`), gọi lại không trùng, job trả chỗ hết hạn; không có
  transaction xuyên module. Việc phụ đi qua event (outbox).
- Thứ tự đăng ký module: `catalog` → `inventory`, `promotions`, `shipping` → `orders` →
  `marketplace`. `shipping` không phụ thuộc `orders`: nhận đơn qua event, báo trạng thái lại qua event.
- Bộ nối với đối tác cụ thể (cổng thanh toán, hãng vận chuyển, nhà cung cấp hoá đơn, SMS, eKYC) là
  module riêng sau trait của core; core chỉ có trait và bộ giả cho test. Viết bộ nối thật cần tài
  khoản thử của đối tác — secret thật, nên dừng hỏi chủ dự án.
- Quy định hay đổi (hạn mức khuyến mại, danh mục hàng có điều kiện, đơn vị hành chính) là dữ liệu có
  ngày hiệu lực (D-186). Mỗi bước có test đồng thời trên cả hai database cho mọi chỗ giữ chỗ.

## 41. Sản phẩm (`catalog`)

Đặc tả: [`docs/steps/41-catalog.md`](steps/41-catalog.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 35 và 36. Khai hạn mức (số sản phẩm) và tính năng (nhiều biến thể, bảng giá) theo
bước 35.

- [x] Chuẩn bị chung cho bước 42–49 (commit đầu tiên): `Subscription::optional` (nhận event của module có thể
      chưa cài), bộ CSV của `forms` thành `autoxide_kernel::csv`, slug của `content` thành
      `autoxide_kernel::slug` — để ba bước song song 42–44 không cùng sửa kernel
- [x] Sản phẩm và biến thể: tuỳ chọn (màu, cỡ…), SKU và mã vạch duy nhất trong khách, trạng thái (nháp, đang
      bán, ngừng bán, lưu trữ), ảnh qua `Media`, nội dung nhiều ngôn ngữ, cân nặng và kích thước (cho vận
      chuyển), nhóm thuế của engine thuế, `warranty_months`; slug và đường dẫn công khai
- [x] Danh mục dạng cây, thuộc tính theo danh mục; **thuộc tính bắt buộc là dữ liệu** (xuất xứ, hạn dùng, nhãn
      hàng hoá theo NĐ 43/2017, NĐ 111/2021 — cần đối chiếu); hàng cấm / có điều kiện là dữ liệu kèm giấy tờ
      phải có; `readiness` để lọc và đếm sản phẩm chưa mở bán được (D-223…D-227)
- [x] Bảng giá: theo cửa hàng / nhóm khách (B2B), kế thừa theo cây (chuỗi đặt, cửa hàng ghi đè — bước 34), ngày
      hiệu lực, **lịch sử giá** (khuyến mại cần "giá ngay trước khuyến mại"); giá niêm yết bằng Đồng Việt Nam,
      gồm thuế khi bán cho người tiêu dùng (Luật Giá 16/2023 — cần đối chiếu)
- [x] Bảng giá theo các nền tảng lớn (D-222): một thứ tự chọn bảng (nút gần nhất → công ty B2B → nhóm khách →
      kênh → thị trường → ưu tiên); bảng giá suy ra ghi thành dòng giá thật; làm tròn giá đẹp; sàn giá (giá tối
      thiểu, biên lợi nhuận trên giá vốn); đổi giá hàng loạt có xem trước, duyệt, hẹn giờ, giá tạm thời
- [x] Đọc công khai cho website: danh sách, lọc theo danh mục / thuộc tính / khoảng giá, tìm (hợp đồng tìm kiếm
      của kernel), ETag và cache như `content`, sitemap; giá sau giảm tự động và lọc `on_sale`; sản phẩm của
      người bán trên website của chuỗi (`seller`, `sold_by`, facet, `ProductCard.seller`)
- [ ] Tìm và lọc như các site hàng đầu (D-201): gợi ý tìm gom sản phẩm, danh mục, bài viết, cửa hàng qua một
      trait; khớp đúng dấu xếp trước không dấu; từ đồng nghĩa theo khách; sắp bán chạy và đánh giá; số đếm theo
      từng giá trị lọc (có giới hạn, có index, cả hai database)
- [ ] Thẻ sản phẩm và JSON-LD (D-201): tóm tắt biến thể, điểm đánh giá, đơn giá theo đơn vị cơ sở; GTIN,
      thương hiệu, URL từng biến thể, giá gạch là giá ngay trước khuyến mại
- [ ] Nhập / xuất CSV bằng job (kiểm từng dòng, lỗi theo dòng, chống CSV injection, trạng thái của lần nhập /
      xuất); gợi ý mô tả và SEO bằng AI chỉ thành bản nháp, đánh dấu `ai_assisted` (bước 37)
- [ ] API hợp đồng (`catalog::api`): đọc sản phẩm / biến thể / giá tại một thời điểm để chụp vào chứng từ, sàn
      giá cho `promotions`; điểm cắm `set_listing_gate` cho `marketplace`; event khi giá hay trạng thái đổi;
      tool MCP như REST
- [ ] Loại sản phẩm: hàng vật lý, hàng số (link ký có hạn qua `files`, giới hạn lượt tải), dịch vụ (không
      giao hàng), combo / bộ sản phẩm trừ kho từng thành phần; hàng đặt trước (`sale_mode`, ngày dự kiến có
      hàng, giới hạn suất — D-204)
- [ ] Gợi ý sản phẩm: sản phẩm tương tự theo ý nghĩa (embedding sẵn có của module `ai`); "thường mua cùng"
- [ ] Hỏi đáp và ảnh song ngữ (D-202): `catalog.product` trong sổ đối tượng, nhãn `seller` (permission quản lý
      sản phẩm ở nút sở hữu), event vòng đời (lưu trữ / xoá → đóng và ẩn hỏi đáp); `catalog_product_media`
      ghi đè `alt` theo ngôn ngữ như `media_alt_translations`
- [ ] Danh mục trong menu của website (D-238): `catalog.category` trong sổ đối tượng (`resolve_many` theo ngôn ngữ,
      `children` theo thứ tự hiển thị, chỉ danh mục đang hiện), `catalog.product` có `image`; khối "Bán chạy" của
      bảng xổ là danh sách sản phẩm công khai lọc theo danh mục, sắp `best_selling`
- [ ] Tạo sản phẩm nháp từ ảnh bằng AI (tên, mô tả, danh mục trong cây của khách, thuộc tính, SEO — D-229);
      khối `catalog.product_grid`, `product_card`, `collection` cho trang dựng từ khối (bước 67)
- [ ] Website bán hàng (web-build-api-gaps, 16/11/2026): thông tin hiển thị theo biến thể trong `by-slug`, theo
      ngôn ngữ (vd số người dùng, trong hộp có gì, cách dùng / chăm sóc, dùng được máy rửa bát) — thuộc tính khai
      theo loại sản phẩm, không viết cứng ngành nào; catalog công khai lọc theo người bán và đường dẫn trang sản
      phẩm cho hàng người bán (cùng 46)

## 42. Kho (`inventory`)

Đặc tả: [`docs/steps/42-inventory.md`](steps/42-inventory.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 41. Không bao giờ bán quá tồn khi nhiều request cùng lúc, trên cả hai database.

- [x] Kho là nút tổ chức loại `warehouse` hay cửa hàng (bước 34); tồn theo biến thể × kho; **sổ biến động kho
      chỉ thêm** (nhập, xuất, điều chỉnh, chuyển); tồn hiện tại suy từ sổ và giữ bằng khoá dòng của kernel
- [x] Giữ chỗ tồn: giữ có hạn theo `order_id` → xác nhận khi đặt hàng → trả lại khi huỷ hay hết hạn (job); chọn
      kho theo quy tắc (gần người mua, đủ hàng); test đồng thời "100 người mua 1 cái cuối"
- [x] Chuyển kho (đang đi đường), kiểm kê (chênh lệch thành bút toán điều chỉnh có lý do; lý do là danh sách
      theo khách, D-223), cảnh báo tồn thấp (event), cho phép bán khi hết hàng hay không theo sản phẩm
- [x] Giá vốn bình quân gia quyền theo kho cho báo cáo lãi gộp; lô và hạn dùng (bán trước hết hạn trước, ngưỡng
      sắp hết hạn theo nút), số serial — bật theo sản phẩm
- [x] Nhận event của `orders` (xác nhận, huỷ, trả hàng nhập lại kho) — chịu được nhận lại event
- [x] Nhập hàng (D-194): nhà cung cấp, đơn đặt hàng nhà cung cấp, nhận hàng một phần, trả hàng cho nhà cung
      cấp; công nợ phải trả vào sổ cái, trạng thái ghi sổ của phiếu nhập (ghi lại được), công nợ từng nhà
      cung cấp qua `ledger`
- [x] Hàng đặt trước (D-204): biến thể `preorder` (ngày dự kiến có hàng, giới hạn suất — bước 41 khai); giữ suất
      không vượt giới hạn khi nhiều người đặt cùng lúc; hàng về thì chia cho đơn đặt trước theo thứ tự đặt
- [ ] Tình trạng hàng công khai theo cửa hàng (D-200): mức `in_stock` / `low` (kèm số khi dưới ngưỡng cấu hình)
      / `out` / `preorder`, không lộ số tồn lớn; `catalog` lọc "chỉ hàng có sẵn ở cửa hàng X" qua `api`; ETag,
      cache ngắn
- [ ] Báo khi có hàng lại theo cửa hàng (D-201): người mua đăng ký (xác nhận hai bước, đồng ý riêng cho mục đích
      này), nghe event tồn kho rồi gửi qua `notifications`; mỗi người một lần mỗi đợt có hàng

## 43. Khuyến mại (`promotions`)

Đặc tả: [`docs/steps/43-promotions.md`](steps/43-promotions.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 41. Luật khuyến mại đổi thường xuyên: hạn mức là dữ liệu có ngày hiệu lực, không viết
cứng (Thông tư 39/2025/TT-BCT trần 50% giá ngay trước khuyến mại, có ngoại lệ chương trình tập trung
quốc gia; NĐ 239/2026 sửa NĐ 81/2018 — cần đối chiếu văn bản hiện hành).

- [x] Chương trình là luật, đến người mua bằng ba cách (D-222): giảm tự động; coupon — mã ai có cũng dùng
      được (mã chung hay lô mã riêng); voucher — quyền dùng một lần trong ví của một tài khoản; mã gắn với
      một người luôn là voucher; khung thời gian với ngày kết thúc bắt buộc (D-225)
- [x] Loại khuyến mại: giảm theo sản phẩm, danh mục, giá trị đơn, nhóm khách; mua X tặng Y (mua 2 tính 1,
      món rẻ nhất); miễn phí vận chuyển; flash sale; bậc thang, combo đồng giá, mua kèm deal sốc, chọn quà
      (D-222)
- [x] Điều kiện: phương thức thanh toán, hãng giao, tỉnh, phân khúc, hạng, app / web / quầy, thương hiệu,
      người bán, khách mới, loại trừ hàng đang giảm (D-222); đơn đầu tiên (`first_order_only`); khung giờ
- [ ] API cho `orders`: tính giảm giá cho một giỏ, trả từng dòng giảm kèm lý do (người mua thấy rõ điều
      kiện — Luật 19/2023); tính trong bộ nhớ, đo p99 < 5 ms cho 50 dòng × 500 chương trình (D-222)
- [x] Cộng dồn theo ba lớp sản phẩm / đơn / vận chuyển: độc quyền trong lớp hay toàn giỏ, `combines_with`,
      dừng sau chương trình, chế độ ưu tiên hay lợi nhất cho người mua (tất định); 1 voucher mỗi lớp cho
      mỗi bên trả tiền (D-222)
- [ ] Ai trả tiền giảm giá (sàn / người bán / nhãn hàng / đối tác thanh toán, chia theo tỉ lệ) chia về từng
      dòng; người bán đăng ký tham gia chương trình của sàn; ngân sách tiền, lượt theo ngày (D-222)
- [x] Giữ chỗ lượt dùng (tổng, mỗi người, theo ngày, ngân sách), suất flash sale và quà như tồn kho (giữ →
      xác nhận → trả lại), test đồng thời; chống lạm dụng theo nhiều khoá người mua (tài khoản, số điện
      thoại, địa chỉ — chỉ bản băm), giới hạn theo kỳ (D-222)
- [x] Ví voucher của người mua (D-222): lưu, nhập mã nhận, được tặng qua `grant_voucher` (gọi lại không
      cấp thêm), cấp tay có lý do; có trong xuất dữ liệu cá nhân, xoá tài khoản thì thu hồi voucher chưa dùng
- [x] Quy tắc trả hàng và huỷ cho `orders` (D-222): hoàn theo tỉ lệ, quà khi trả món điều kiện, voucher về
      ví khi huỷ đơn
- [x] Kiểm tuân thủ: chặn chương trình vượt hạn mức đang hiệu lực (cộng mọi khoản giảm trên một đơn vị,
      D-222) và sàn giá; giới hạn số ngày giảm giá là dữ liệu; kiểm trước khi bật, không ghi (D-225)
- [ ] Thông báo khuyến mại theo từng nơi nhận (D-225): xuất dữ liệu thông báo gửi Sở Công Thương nơi tổ
      chức, ghi nhận đã nộp từng nơi; lưu vết chương trình đã chạy
- [ ] Mã cho người mua thấy: mã / voucher dùng được cho giỏ và mã chưa đủ điều kiện kèm lý do ("thêm X để
      dùng"), bộ chọn thay ô nhập mã (D-201); trang sản phẩm hiện "còn X với mã …", giá niêm yết vẫn là giá
      không mã (D-200); số lượt còn lại chỉ hiện khi là số thật
- [ ] Công khai và nhắc: event giá sau giảm tự động cho danh sách công khai; flash sale công khai với suất
      còn là số thật; nhắc khi flash sale bắt đầu (đồng ý riêng cho một lần nhắc, D-225)
- [ ] Chạy thử chương trình trên giỏ mẫu, kiểm xung đột, lịch, số liệu chương trình (D-222)
- [ ] Khối `promotions.flash_sale`, `promotions.voucher_claim` cho trang dựng từ khối (D-229, bước 67);
      `promotions.promotion` trong sổ đối tượng (chỉ công khai khi đang chạy) để menu của website trỏ tới (D-238)
- [ ] Website bán hàng (16/11/2026): `GET /public/promotions` trả số thông báo khuyến mại với Sở Công Thương và
      mức giảm tối đa theo luật đang áp (dữ liệu có ngày hiệu lực, luật số hai)

## 44. Vận chuyển (`shipping`)

Đặc tả: [`docs/steps/44-shipping.md`](steps/44-shipping.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 41. Không phụ thuộc `orders`: nhận đơn qua event, báo trạng thái qua event.

- [x] Sổ địa chỉ của người mua dùng `Address` của module nền `address` (bước 34, D-193); địa chỉ nước ngoài
      dạng chung
- [x] Phí vận chuyển khách tự đặt: vùng theo tỉnh, bậc cân nặng / kích thước, miễn phí từ một giá trị đơn,
      nhận tại cửa hàng (chuỗi); API báo giá cho `orders`; thuế GTGT trên phí giao (Luật 48/2024 — cần đối
      chiếu)
- [x] Trait `ShippingCarrier` (báo giá, tạo vận đơn, huỷ, in nhãn, theo dõi) và bộ giả cho test; trạng thái
      kiểm tài khoản hãng; webhook trạng thái của hãng: kiểm chữ ký, không trùng, ánh xạ về trạng thái chung
- [x] Vận đơn: một đơn nhiều kiện, giao một phần, giao thất bại, hoàn hàng; event cho `orders`
- [x] Thu hộ khi giao (COD): tiền do hãng thu và chuyển trả theo hợp đồng với hãng; sổ cái ghi phải thu từ hãng,
      đối soát với bảng kê COD; lệch thì báo, khớp dòng lệch có lý do và audit; `shipping.cod_settled` mang
      `seller_org_id` và nơi nhận tiền (D-216)
- [ ] Ngày giao dự kiến là ngày cụ thể (D-201): khoảng ngày tuyệt đối kèm giờ chốt trong ngày, theo múi giờ của
      khách; lịch giao, ngày làm việc, ngày nghỉ, giờ chốt giao trong ngày là dữ liệu có ngày hiệu lực; nhận
      tại cửa hàng trả giờ sẵn sàng (chuẩn bị + giờ mở cửa của bước 34) và thời hạn giữ hàng chờ nhận
- [ ] Ước tính phí giao công khai (D-200): theo tỉnh và một sản phẩm / số lượng, chưa cần giỏ, cho trang sản
      phẩm; cùng bảng phí với báo giá của `orders`, ghi rõ "ước tính", ngưỡng miễn phí
- [ ] Câu trả lời tự động về hành trình vận đơn cho `support` (`FactSource`, chỉ cho chủ đơn — D-230)
- [ ] Website bán hàng (16/11/2026): phí và ngày giao dự kiến theo tỉnh khi chưa có giỏ (trang sản phẩm), kèm giờ
      chốt đơn trong ngày của nơi bán

## 45. Giỏ hàng, đặt hàng, đơn hàng (`orders`)

Đặc tả: [`docs/steps/45-orders.md`](steps/45-orders.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 42, 43, 44. Gọi `catalog`, `inventory`, `promotions`, `shipping` qua `api` của chúng;
tiền qua `PaymentGateway` và sổ cái của bước 36.

- [ ] Tạo tài khoản từ số điện thoại đã xác thực qua tra đơn vãng lai (D-202, chuyển từ bước 34 — D-316):
      `POST /api/v1/auth/phone-signups` khi `orders` có token xem đơn, đăng nhập bằng passkey hay OTP, không bắt
      buộc mật khẩu (đặc tả 34 mục "Tạo tài khoản từ số đã xác thực")
- [x] Giỏ hàng cho người mua và vãng lai (gộp khi đăng nhập); giá, giảm, thuế, phí giao luôn tính lại ở máy
      chủ, không tin số từ client; `free_shipping_remaining` (D-201); coupon và voucher tách nhau, điền dữ liệu
      điều kiện cho `promotions`, chụp khoản giảm kèm ai trả tiền vào đơn (D-222); danh sách giỏ bỏ dở
- [ ] Đặt hàng: báo giá cuối → giữ tồn, lượt khuyến mại → số đơn tuần tự (bước 36) → thanh toán theo chế độ
      dòng tiền (tự thu / thu hộ / chia tại cổng / COD; `payment_actions[]`, D-216); `Idempotency-Key` bắt
      buộc; hạn thanh toán đọc từ `ledger` (D-218), hết hạn thì trả chỗ; trạng thái thanh toán công khai
- [ ] Vòng đời đơn: chờ thanh toán → xác nhận → chuẩn bị → đang giao → đã giao → hoàn tất; huỷ theo quy tắc
      (`buyer_cancel_until` tới `preparing`), chờ cửa hàng nhận đơn `acceptance_hours` (D-209); mọi chuyển
      qua service, ghi event và bút toán sổ cái; gửi lại link thanh toán còn hạn
- [ ] Nhận tại cửa hàng và hàng đặt trước: mã nhận 6 số, người nhận hộ, giữ hàng chờ nhận có hạn rồi huỷ /
      hoàn theo chính sách; đặt trước (D-204): tách lần giao, đồng ý điều kiện và thấy ngày giao trước khi
      đặt, dời ngày thì báo và cho huỷ miễn phí, ngừng bán đặt trước thì huỷ và hoàn tiền tự động
- [ ] Đổi trả và hoàn tiền: trả trong hạn chính sách, lý do theo `allowed_return_reasons` (D-209, D-223), duyệt,
      nhận về (nhập kho), hoàn qua cổng, bút toán đảo, hoá đơn điều chỉnh / thay thế (NĐ 123/2020 sửa bởi
      NĐ 70/2025 — cần đối chiếu); hoàn theo tỉ lệ khoản giảm, xử lý quà, voucher về ví (D-222)
- [ ] Trả hàng phía người mua (D-201, D-202): cách gửi trả (lấy tận nơi / bưu cục / cửa hàng), thời gian hoàn
      theo phương thức thanh toán; ảnh kèm (lý do bắt buộc ảnh là chính sách theo nút); `returns/preview`
      trả tiền hoàn, cách gửi và phí, thời gian hoàn, hạn — không ghi
- [ ] Hoá đơn điện tử qua `EInvoiceProvider`, kể cả từ máy tính tiền; `invoice_timing = on_delivered` mặc
      định (D-215); khoản giảm, hàng khuyến mại trên hoá đơn (D-222, cần đối chiếu NĐ 123/2020 sửa bởi NĐ
      70/2025); mail xác nhận, cập nhật trạng thái qua job theo i18n; chứng từ của người mua
- [ ] Mua không cần tài khoản (D-201, D-202): tra đơn bằng số đơn + OTP; token xem đơn để huỷ, đổi địa chỉ
      (OTP lần hai), mở hỗ trợ ở `support` (D-204), qua cùng service; giới hạn tra / gửi lại OTP / hạn token
      là cấu hình; che số điện thoại, địa chỉ tới cấp phường; gắn đơn vãng lai vào tài khoản đã xác thực số
- [ ] Bán tại quầy (POS) qua API cho chuỗi: đơn tạo tại cửa hàng, tiền mặt / thẻ / QR, trừ kho của cửa hàng
      đó; đóng ca có chênh lệch: lý do, ghi sổ cái, cần duyệt khi vượt ngưỡng (D-227)
- [ ] Đánh giá sau mua: chỉ người đã mua, kiểm duyệt không chặn / ẩn trái quy định (Luật 19/2023), lý do ẩn
      theo danh sách (D-223); dùng `comments`, module `reviews` hay đặt ở `orders` — ghi quyết định; lọc có
      ảnh / số sao / sắp xếp, ảnh người mua; đếm đánh giá mới theo mốc đã xem; nhãn `verified_buyer` (D-202)
- [ ] Nút "Đánh giá này hữu ích" (D-203): mỗi người một phiếu, rút được, đếm cùng transaction, `sort=helpful`
      bằng con trỏ, không tự / người bán bỏ phiếu, không có "không hữu ích"; tóm tắt đánh giá bằng AI chỉ là
      bản nháp nhân viên duyệt, gắn nhãn (D-109, D-201)
- [ ] Báo cáo bán hàng theo cửa hàng / kênh / sản phẩm / thời gian, theo quyền độ sâu; event cho kho báo cáo
      gộp (bước 38); xuất đơn qua xuất chung của bước 51 (D-213)
- [ ] Chống "bom hàng" COD (D-194): tỉ lệ nhận hàng thành công của người mua chỉ trong dữ liệu của khách (có
      căn cứ xử lý theo Luật BVDLCN), quy tắc buộc trả trước khi rủi ro cao, danh sách chặn có lý do và thời
      hạn; không chia sẻ danh sách giữa các khách
- [ ] Trả góp qua đối tác (trait của bước 36); bảo hành theo số serial hay theo đơn: phiếu bảo hành, tiếp
      nhận, trạng thái xử lý, hạn bảo hành
- [ ] Khối `orders.add_to_cart`, `buy_now`, `reviews` cho trang dựng từ khối (D-229) và `orders.cart_link` cho đầu
      trang (D-234); câu trả lời tự động về trạng thái đơn cho `support` (`FactSource`, chỉ cho chủ đơn — D-230)
- [ ] Ô người mua nhập lúc đặt hàng (D-235): `orders.order` là đích trường tự định nghĩa (P19 của bước 51) có mức
      `buyer_input`; checkout kiểm cùng bộ gom lỗi; mail xác nhận đọc được các trường này
- [ ] Trả trước một phần cho hàng đặt trước (D-232): tỉ lệ hay số tiền, phần còn lại thu khi sẵn sàng giao,
      người bán huỷ hoàn đủ, người mua huỷ theo điều kiện công bố; không mặc định là "đặt cọc" (BLDS Điều 328)
- [ ] Website bán hàng (16/11/2026): báo giá giỏ trả thêm số điểm tối đa dùng được, cách trả dùng được cho giỏ
      (COD theo từng người bán), ngày giao dự kiến theo từng nơi bán; đơn có sẵn `return_until`,
      `buyer_cancel_until`; đánh giá: số đếm có ảnh và theo biến thể, "chủ đề người mua hay nhắc" kèm bản tóm tắt AI
      đã được người duyệt (D-109: AI chỉ là gợi ý)

## 46. Sàn (`marketplace`)

Đặc tả: [`docs/steps/46-marketplace.md`](steps/46-marketplace.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 45 và 37. Sàn là một khách; người bán là nút loại `seller` (bước 34).

- [ ] Người bán đăng ký: hồ sơ, KYC qua cổng chặn của bước 37 (chưa xác thực thì không đăng bán),
      chấp nhận quy chế sàn và hợp đồng với người bán có phiên bản (bước 37), giấy tờ cho ngành hàng
      có điều kiện; gian hàng của người bán; thông tin người bán hiện trên trang sản phẩm; `marketplace.seller`
      trong sổ đối tượng để menu của website trỏ tới gian hàng (D-238)
- [ ] Duyệt sản phẩm của người bán: hàng cấm, hàng có điều kiện thiếu giấy tờ, báo vi phạm (bước 37)
- [ ] Phí sàn theo ngành hàng là dữ liệu có ngày hiệu lực; một lần đặt hàng tách thành đơn theo
      từng người bán; người bán tự giao hay sàn giao
- [ ] Chi trả cho người bán: tạm giữ tới hết hạn đổi trả, trừ phí sàn và thuế khấu trừ thay (NĐ
      117/2025), lệnh chi qua `PayoutGateway`; bảng đối soát cho người bán; sổ cái khớp từng đồng
- [ ] Tranh chấp người mua – người bán, sàn phân xử, có hạn xử lý; điểm và chế tài người bán; báo cáo
      cho cơ quan có thẩm quyền khi được yêu cầu (có audit)
- [ ] Cung cấp thông tin người bán và doanh thu cho cơ quan thuế theo kỳ (NĐ 117/2025 — cần đối
      chiếu mẫu và hạn); kho của sàn (sàn nhận hàng của người bán, đóng gói, giao — tồn theo người bán
      trong kho của sàn)
- [ ] Đối chiếu thiết kế 2 (D-214, D-216, D-218): bút toán theo tên tài khoản của 36, tạm giữ qua
      `ledger_holds`; COD không về tài khoản sàn; danh sách người bán công khai, đánh giá theo người bán, theo
      dõi gian hàng (đồng ý riêng cho tin hàng mới); huy hiệu chỉ từ dữ liệu thật
- [ ] Quyết toán theo ai trả tiền giảm giá (D-222): voucher sàn thì người bán nhận đủ, voucher shop trừ doanh
      thu người bán; gốc tính phí sàn là dữ liệu có ngày hiệu lực; người bán đồng ý trước khi cùng chịu chi phí
- [ ] Website bán hàng (16/11/2026): hồ sơ người bán công khai có số sản phẩm theo nhóm, tỉ lệ giao đúng hạn,
      trạng thái tạm ngừng; phí sàn công khai (không cần `marketplace.sell`); đăng ký bán trước khi có tài khoản;
      endpoint công khai báo vi phạm cho sản phẩm, đánh giá, livestream (lý do theo danh sách cố định; cơ chế tiếp
      nhận phản ánh của sàn theo luật thương mại điện tử — cần đối chiếu văn bản gốc)

## 47. Khách hàng và giữ chân khách (`customers`)

Đặc tả: [`docs/steps/47-customers.md`](steps/47-customers.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 45. Người mua là tài khoản theo từng khách (bước 34); module này giữ hồ sơ bán hàng.

- [ ] Hồ sơ khách: lịch sử mua (từ event của `orders`), tổng chi; nhãn, ghi chú, trường tự định nghĩa dùng chỗ
      chung của bước 51 (D-237); che số điện thoại / email, xem đủ cần permission và ghi audit (D-226)
- [ ] Nhóm và phân khúc theo quy tắc, dùng cho bảng giá (bước 41) và khuyến mại (bước 43); phân khúc và
      hạng thành viên làm điều kiện khuyến mại (D-222)
- [ ] Điểm thưởng và hạng thành viên: sổ điểm chỉ thêm, điểm có hạn dùng, quy tắc tích / đổi là dữ liệu;
      điểm chưa dùng là nghĩa vụ ghi trong sổ cái; cơ sở tính điểm thống nhất (D-226)
- [ ] Thẻ quà tặng và số dư cửa hàng: chỉ dùng mua hàng của chính khách đó, không rút ra tiền, không chuyển
      ra ngoài — **tắt mặc định**, chỉ bật sau khi đối chiếu NĐ 52/2024 về ví điện tử / thẻ trả trước
      (D-194); số dư là nghĩa vụ trong sổ cái
- [ ] Danh sách yêu thích (gộp danh sách của trình duyệt vào tài khoản khi đăng nhập), sản phẩm vừa xem;
      nhắc giỏ hàng bỏ dở qua `notifications` — chỉ khi người mua đã đồng ý nhận tin quảng cáo (bước 37);
      giới thiệu bạn bè có thưởng
- [ ] Tặng voucher (D-222): sinh nhật, đổi điểm lấy voucher, sau khi mua, giới thiệu bạn — qua
      `grant_voucher`, gọi lại không cấp thêm
- [ ] Báo khi sản phẩm yêu thích có hàng lại hay giảm giá (D-233): người mua tự bật, đồng ý riêng, chỉ giảm thật
      ≥ ngưỡng so với giá mốc, tối đa 1 lần / 7 ngày mỗi sản phẩm, gom lô theo khung giờ
- [ ] Website bán hàng (16/11/2026): đăng nhập / tạo tài khoản công khai cho người mua bằng OTP qua SMS hay
      passkey (gửi SMS qua đối tác, không giữ secret trong code)

## 48. Bán đa kênh (`channels`)

Đặc tả: [`docs/steps/48-channels.md`](steps/48-channels.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 45. Người bán ở Việt Nam bán cùng lúc trên website, Shopee, Lazada, TikTok Shop,
Facebook / Zalo; một kho, một danh sách sản phẩm, đơn từ mọi nơi về một chỗ.

- [ ] Trait `SalesChannel` (đăng sản phẩm, cập nhật giá và tồn, lấy đơn, cập nhật trạng thái giao)
      và bộ giả cho test; bộ nối từng sàn là module riêng, làm khi có tài khoản thử (D-191)
- [ ] Liên kết sản phẩm / biến thể với mã trên từng kênh; giá riêng theo kênh (giá cố định cho một tin
      đăng, hay bảng giá suy ra theo kênh của `catalog` — D-222)
- [ ] Đồng bộ tồn có phần để dành cho từng kênh, không bán quá khi nhiều kênh cùng bán; số đẩy lên kênh ở
      chế độ `reserved` (phần giữ riêng còn lại + phần chia từ tồn chung)
- [ ] Đơn từ kênh khác vào `orders` với kênh nguồn, trừ kho chung, không trùng khi kênh gửi lại; trạng
      thái đơn kênh sau khi xử lý (`resolved` kèm lý do — D-223, `cancelled_by_channel`)
- [ ] Báo cáo theo kênh; đối soát tiền sàn trả với sổ cái
- [ ] Nguồn cấp sản phẩm cho Google Merchant Center và danh mục của Meta (D-204): tệp XML / CSV dựng bằng
      job theo cửa hàng, ngôn ngữ, tiền tệ; link công khai có token; báo lỗi từng sản phẩm (thiếu GTIN,
      ảnh); không lộ số tồn hay giá vốn

## 49. Tiếp thị liên kết và bán qua livestream (`affiliate`)

Đặc tả: [`docs/steps/49-affiliate.md`](steps/49-affiliate.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 45 và 37. Luật TMĐT 2025 buộc xác thực danh tính người livestream và người tiếp thị
liên kết (bước 37).

- [ ] Người tiếp thị đăng ký có KYC (cổng chặn bước 37), link và mã riêng, thời hạn ghi nhận đơn, quy
      tắc hoa hồng theo sản phẩm / danh mục là dữ liệu có ngày hiệu lực
- [ ] Hoa hồng ghi sổ cái khi đơn hoàn tất, trừ lại khi trả hàng; chi trả qua `PayoutGateway` có khấu
      trừ thuế thu nhập cá nhân theo hồ sơ thuế (tỉ lệ và ngưỡng là dữ liệu — các nguồn đang khác nhau
      giữa 2 triệu và 5 triệu đồng / lần, NĐ 253/2026 — cần đối chiếu); chứng từ khấu trừ cho người nhận
- [ ] Phiên livestream: người phát đã xác thực, sản phẩm gắn trong phiên, đơn ghi nhận theo phiên,
      lưu vết phiên theo yêu cầu của luật
- [ ] Đối chiếu đợt 2: `paid_on` khi đánh dấu đã chi
- [ ] Website bán hàng (16/11/2026): danh sách phiên livestream công khai (lịch phát), không chỉ một phiên theo id

## 55. Giao diện cửa hàng qua API: banner, bộ sưu tập, thông tin cửa hàng (D-219)

Đặc tả: [`docs/steps/55-storefront.md`](steps/55-storefront.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 41. Website và app của khách tự vẽ giao diện (D-026: core chỉ có API); bước này cho
họ dữ liệu để vẽ trang chủ, trang danh mục, chân trang mà không phải sửa code giao diện mỗi lần đổi.

- [ ] Vị trí hiển thị và banner (`content`): vị trí theo khoá (vd `home_hero`), banner có ảnh máy tính
      và điện thoại qua `Media`, tiêu đề, nút, link (địa chỉ, bài, danh mục, sản phẩm, bộ sưu tập,
      chương trình khuyến mại), ngôn ngữ, khung giờ hiển thị, thứ tự, áp cho cửa hàng / nhánh (bước 34)
- [ ] Đọc công khai `GET /api/v1/content/public/placements/{key}`: banner đang hiệu lực lúc gọi, ảnh có
      `srcset` và WebP, cache tới mốc bắt đầu / kết thúc gần nhất; banner trỏ tới chương trình khuyến
      mại tự ẩn khi chương trình kết thúc (event tuỳ chọn của `promotions`) — quảng cáo không được hứa
      ưu đãi đã hết (Luật 19/2023, Luật Quảng cáo — cần đối chiếu)
- [ ] Bộ sưu tập sản phẩm (`catalog`): chọn tay (có thứ tự) hay theo quy tắc (danh mục, thuộc tính,
      khoảng giá, hàng mới, đang khuyến mại), sắp theo mới nhất / giá / bán chạy (số bán nhận từ event
      tuỳ chọn của `orders`); trang công khai có slug, SEO, ảnh; dùng được trong menu và banner
- [ ] Thông tin cửa hàng công khai (`organizations`): thương hiệu (tên, logo, biểu tượng trang, liên hệ, mạng xã
      hội) là cài đặt kế thừa theo cây (chuỗi đặt, cửa hàng ghi đè); địa chỉ, giờ mở cửa, ngày nghỉ lấy từ hồ sơ
      địa điểm của bước 34, không lưu hai chỗ (D-237); chân trang lấy thêm thông tin chủ sở hữu và logo Bộ Công
      Thương từ bước 37
- [ ] Liên kết tới đối tượng của module đăng ký sau (sản phẩm, bộ sưu tập, chương trình) qua điểm cắm
      `ContentLinks` trong `content::api`: module đó gắn bộ dựng đường dẫn và bộ kiểm "còn hiển thị
      không"; thiếu module thì link loại đó bị từ chối khi lưu
- [ ] Tool MCP cho banner và bộ sưu tập (soạn, lên lịch, sắp xếp) như REST; AI chỉ gợi ý chữ, đánh dấu
      `ai_assisted` (bước 37); hướng dẫn cho người làm giao diện (EN + VI)
- [ ] Kiểu vị trí banner (D-236): cách hiện (một banner, trượt, lưới, xếp dọc, dải chữ, popup), cỡ và tỉ lệ ảnh
      kiểm khi lưu, thiết bị, loại trang dùng được; bộ vị trí chuẩn có sẵn cho khách mới (dải trên, banner chính,
      giữa trang chủ, đầu danh mục, cột trái / phải, giữa nội dung, trên chân trang, popup); banner chọn trang hiện
      (loại trang, danh mục / sản phẩm / bài cụ thể); banner dạng chữ màu theo theme; đọc nhiều vị trí một lần;
      khối `content.placement` cho trang dựng từ khối và đầu trang
- [ ] Banner trong gian hàng người bán trên sàn (D-233): người bán tự quản trong nút của mình, sàn duyệt theo
      chính sách, link chỉ tới hàng của chính người bán, không phải quảng cáo trong sàn

## 56. Chuyển dữ liệu từ nền tảng khác (`imports`, D-219)

Đặc tả: [`docs/steps/56-imports.md`](steps/56-imports.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 47. Người bán đang ở Shopify, WooCommerce, Haravan, Sapo, KiotViet muốn chuyển sang mà
không nhập lại tay và không mất thứ hạng tìm kiếm của đường dẫn cũ.

- [ ] Module `imports`: tải file xuất (CSV, Excel) của nền tảng cũ; **hồ sơ ánh xạ cột là dữ liệu có
      phiên bản** cho từng nền tảng (định dạng xuất của họ đổi thì sửa dữ liệu, không sửa code), người
      dùng chỉnh ánh xạ được; nhận diện bảng mã và dấu phân cách
- [ ] Chạy thử trước: xem trước số dòng, lỗi theo từng dòng và cột, không ghi gì; chạy thật bằng job
      theo lô, dừng / chạy tiếp được, gọi `api` của `catalog`, `inventory`, `customers`, `orders` (đơn
      cũ nhập làm lịch sử, không trừ kho, không gửi thông báo) — không ghi thẳng vào bảng module khác
- [ ] Chạy lại không trùng: bảng ánh xạ ID nguồn → ID mới theo nền tảng và loại; ảnh tải từ link của
      nền tảng cũ qua bộ chặn SSRF của kernel (`autoxide_kernel::net`), giới hạn cỡ, vào `media`
- [ ] Giữ đường dẫn cũ: tạo chuyển hướng 301 từ đường dẫn sản phẩm / danh mục / bài cũ sang đường dẫn mới
      (chuyển hướng của `content`), báo các đường dẫn không ánh xạ được
- [ ] Dữ liệu cá nhân khi nhập khách hàng (Luật BVDLCN 91/2025): người nhập xác nhận căn cứ xử lý; đồng ý
      nhận quảng cáo **không** tự mang sang nếu không có bằng chứng (bước 37); mật khẩu cũ không nhập —
      khách đặt lại qua link; file tải lên xoá sau khi xong
- [ ] Báo cáo sau khi nhập (đã tạo, bỏ qua, lỗi, cần làm tay); hướng dẫn chuyển từ từng nền tảng (EN + VI)

## 59. Bán sỉ B2B (`wholesale`) (chủ dự án giao ngày 14/11/2026, D-221)

Đặc tả: [`docs/steps/59-wholesale.md`](steps/59-wholesale.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 45 và 47. Bán cho doanh nghiệp trên cùng hệ thống bán lẻ.

- [ ] Công ty mua có nhiều người mua và người duyệt; bảng giá sỉ, bậc giá theo số lượng, số lượng tối thiểu —
      không lộ giá sỉ ra công khai
- [ ] Báo giá có hạn, chấp nhận thành đơn với giá đã chốt; đặt nhanh theo SKU / CSV
- [ ] Trả chậm theo điều khoản, hạn mức tín dụng không vượt khi đặt đồng thời, công nợ trong sổ cái, tuổi nợ,
      nhắc nợ, chặn đơn trả chậm khi quá hạn; hoá đơn ghi mã số thuế công ty; test trên cả hai database
- [ ] Hợp đồng khung (D-232): cam kết số lượng / giá trị, đơn gọi hàng trừ cam kết nguyên tử; mời thầu nhiều
      người bán trên sàn, giá niêm phong tới hạn, trao thầu thành báo giá / hợp đồng; không dùng cho mua sắm công
- [ ] Ký hợp đồng điện tử trong `compliance` (D-232): `SignatureProvider` + bộ giả, ký `simple` (OTP + đồng ý) hay
      `digital` (chữ ký số qua nhà cung cấp được cấp phép), mã băm chống sửa tệp; dùng cho 59, 46, 49
- [ ] Trao đổi chứng từ B2B tự động (EDI, D-233): UBL 2.1 cho đơn, xác nhận đơn, báo giao; vào qua API key
      `service`, ra qua `webhooks`; chống trùng theo ID chứng từ, chặn XXE; EDIFACT / X12 là bộ chuyển đổi riêng

## 60. Đặt lịch dịch vụ (`bookings`) (chủ dự án giao ngày 14/11/2026, D-221)

Đặc tả: [`docs/steps/60-bookings.md`](steps/60-bookings.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 34 và 36; không cần phần bán hàng. Spa, salon, phòng khám, lớp học, sân thể thao.

- [x] Dịch vụ (thời lượng, đệm, sức chứa, cọc), nhân viên và phòng / thiết bị, lịch làm việc lặp và ngày nghỉ
      theo múi giờ của nút; tính giờ trống công khai
- [x] Đặt chỗ không bao giờ chồng giờ khi nhiều người đặt cùng lúc (ô giờ có khoá duy nhất, chạy trên cả hai
      database); giữ tạm khi trả cọc qua đối tác; huỷ, đổi giờ, vắng mặt theo chính sách
- [x] Nhắc lịch qua `notifications`, khách vãng lai đặt bằng OTP, `/me/bookings`, tool MCP; test trên cả hai
      database

## 61. Chat chăm sóc khách (`conversations`) (chủ dự án giao ngày 14/11/2026, D-221)

Đặc tả: [`docs/steps/61-conversations.md`](steps/61-conversations.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 40 (SSE) và 53 (`support`).

- [ ] Cuộc trò chuyện giữa người mua và cửa hàng / người bán, gắn với sản phẩm hay đơn; tin nhắn chỉ thêm, gửi
      lại không trùng, thu hồi trong thời hạn ngắn; hộp thư chung, phân công, đã đọc
- [ ] Tin mới thời gian thực qua SSE, người không online nhận thông báo gom lô; chuyển thành phiếu hỗ trợ
- [ ] Chống lạm dụng (chặn, báo vi phạm, xác thực tài khoản khi khách bật), hạn lưu là cấu hình; câu phức tạp:
      AI soạn nháp cho nhân viên; câu đơn giản đã có câu trả lời chính xác: trả lời tự động qua `AutoResponder`
      của `support` (D-230); test trên cả hai database
- [ ] Gọi điện trong chat (D-232): `VoiceProvider` + bộ giả, yêu cầu gọi lại vào hộp việc, nhật ký cuộc gọi,
      ghi âm tắt mặc định và phải thông báo trước; bộ nối thật cần tài khoản — dừng hỏi chủ dự án

## 62. Sản phẩm bán theo kỳ (`recurring`) (chủ dự án giao ngày 14/11/2026, D-221)

Đặc tả: [`docs/steps/62-recurring.md`](steps/62-recurring.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 45 và 36. Người mua trả cho cửa hàng theo kỳ — khác `plans` (khách trả cho nền tảng, bước 35).

- [ ] Gói bán theo kỳ trên sản phẩm (chu kỳ, giá, dùng thử, số kỳ tối thiểu); mỗi kỳ tự tạo đơn và thu tiền qua
      đối tác, không tạo trùng khi job chạy lại; thanh toán lỗi thì thử lại rồi tạm dừng
- [ ] Người mua tự tạm dừng, bỏ qua kỳ, đổi sản phẩm, đổi phương thức, huỷ dễ như đăng ký; báo trước khi trừ tiền
      và khi đổi giá; không lưu số thẻ; số liệu MRR và tỉ lệ rời bỏ; test trên cả hai database

## 63. Đa tiền tệ và bán xuyên biên giới (chủ dự án giao ngày 14/11/2026, D-221)

Đặc tả: [`docs/steps/63-multi-currency.md`](steps/63-multi-currency.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 36, 41, 44, 45. Mở rộng module có sẵn, không module mới.

- [ ] Dùng tỉ giá có ngày hiệu lực của sổ cái (bước 36, D-237), thêm nguồn tỉ giá nếu cần; thị trường (quốc gia,
      tiền tệ, ngôn ngữ) theo website; giá theo thị trường, đơn chốt tỉ giá lúc đặt
- [ ] Sổ cái ghi số gốc và số quy đổi, chênh lệch tỉ giá; thuế GTGT 0% cho xuất khẩu khi đủ hồ sơ; người mua
      trong nước chỉ VND (Pháp lệnh Ngoại hối — cần đối chiếu); mã HS và xuất xứ cho giao hàng quốc tế; test trên
      cả hai database
- [ ] Người bán nước ngoài trên sàn (D-233): tắt mặc định, điều kiện (giấy tờ, đại diện, thuế) là gói luật có
      ngày hiệu lực, KYC hồ sơ nước ngoài, công bố quốc gia và nơi gửi hàng, chi trả ngoại tệ; hàng nhập khẩu
      cho người mua ngoài phạm vi

## 64. Quảng cáo trong sàn (`ads`) (chủ dự án giao ngày 14/11/2026, D-221)

Đặc tả: [`docs/steps/64-ads.md`](steps/64-ads.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 46 và 57 (`analytics`).

- [ ] Chiến dịch của người bán (vị trí, từ khoá / danh mục, CPC / CPM, giá thầu, ngân sách, lịch); chèn vị trí
      quảng cáo vào danh sách công khai qua điểm cắm của `catalog`, luôn có nhãn "Quảng cáo"
- [ ] Đếm hiển thị và bấm qua `analytics` (chống bấm giả), trừ ngân sách không vượt khi đồng thời; phí là doanh
      thu dịch vụ của sàn thu qua quyết toán hay hoá đơn, không giữ ví trả trước; báo cáo hiệu quả cho người bán;
      test trên cả hai database

## Dọn cho agent (việc ngoài bước, chủ dự án duyệt ngày 18/11/2026)

Làm Autoxide dễ đọc hiểu cho AI agent, cả khi phát triển repo lẫn khi dùng API. Không phải một bước của lộ trình;
không chặn bước nào.

- [x] Mục lục quyết định tự sinh (`cargo xtask decisions` → `docs/decisions/INDEX.md`, theo chủ đề, đánh dấu mục đã bị
      thay); gate báo khi lệch (D-20261118-decision-ids)
- [x] Quyết định mới mỗi mục một file, ID theo ngày và tên (`D-<YYYYMMDD>-<tên>`) để các phiên song song không chọn
      trùng; gate chặn ID trùng và tham chiếu tới ID không có (D-20261118-decision-ids)
- [x] OpenAPI theo module (`GET /api/v1/openapi/<module>.json`), endpoint MCP theo module (`/mcp/<module>`), `/llms.txt`
      trỏ tới cả hai; test trên cả hai database (D-20261118-module-contracts)
- [x] Gate cảnh báo file `.rs` trên 800 dòng, chặn trên 1200 trừ danh sách miễn không được dài thêm; tách sáu file dài
      nhất (D-20261118-file-length)

Ghi nhận, làm sau nếu đáng: `./gate.sh --quick` chậm khi sửa kernel — mọi crate phụ thuộc kernel nên `test-affected`
build và chạy test của cả workspace (ở đợt này, một lượt gate nhanh sau khi sửa `crates/kernel/src/http` lâu hơn nhiều
lần lượt chỉ sửa `xtask`, phần lớn là test e2e của binary). Hướng có thể xét: chỉ chạy test của crate phụ thuộc vào đúng phần
công khai bị đổi, hay tách phần ít đổi của kernel thành crate riêng. Chưa làm.

## Chờ chủ dự án

## 50. Công khai repo (chờ chủ dự án, D-175, D-197, D-199)

Chủ dự án giữ repo riêng tư (D-199). **Agent không làm bước này khi chủ dự án chưa nói repo đã
công khai**; khi đó các bước khác vẫn đi tiếp bình thường. Workflow đã viết sẵn (D-197), công
khai là tự chạy.

- [ ] Chủ dự án công khai repo (Settings → Change visibility); ngay trước đó agent quét lại
      gitleaks toàn lịch sử và tìm tên máy chủ, IP, đường dẫn riêng, thông tin cá nhân. Agent không
      tự đổi chế độ công khai
- [ ] Chủ dự án bật trong Settings → Code security: secret scanning, push protection, Dependabot
      alerts, private vulnerability reporting (`SECURITY.md` dựa vào nó); gitleaks trong `./gate.sh`
      giữ nguyên
- [ ] `CI` và `Security audit` chạy xanh trên GitHub
- [ ] Bản phát hành kế tiếp qua workflow `Release`: image amd64 + arm64 đăng lên `ghcr.io`, chủ
      repo đổi gói image sang Public một lần; `gh attestation verify` xác nhận chứng thực nguồn gốc
      và SBOM; `docs/RELEASING.md` bỏ phần "Khi repo còn riêng tư"
- [ ] Thử cài như người lạ bằng image đã đăng (cả SQLite và Postgres), sửa README /
      `docs/DEPLOY.md` (EN + VI cùng commit) theo chỗ vấp

## Sau core (chưa mở)

GraphQL · tách service — xem bảng "Chưa làm, đã cân nhắc" trong `STACK.md`.

Đã đưa lên lộ trình (D-221): bán sỉ B2B (bước 59), đặt lịch dịch vụ (60), chat chăm sóc khách (61), sản
phẩm bán theo kỳ (62), đa tiền tệ (63), quảng cáo trong sàn (64), chợ ứng dụng OAuth (65), SCIM (66).

Đã đưa vào các bước (D-232): khách thử nghiệm và nhóm nút cho báo cáo (38), AVIF (39), trả trước một phần (45),
quản trị trên điện thoại (52), duyệt từng lần truy cập hỗ trợ và "xem như" chỉ đọc (54), hợp đồng khung, mời thầu và
ký điện tử (59), gọi điện trong chat (61), bán ứng dụng trả phí (65).

Đã viết đặc tả (D-233): hợp đồng tool MCP và `cargo semver-checks` (31), báo yêu thích (47), banner gian hàng
người bán (55), EDI (59), người bán nước ngoài (63). Sau đó: chủ đề theo lịch, đầu / chân trang (67, D-234); trường
tự định nghĩa, nhãn, đổi câu chữ (51), loại nội dung (67), mã theo dõi và đồng ý cookie (37) (D-235); kiểu vị trí
banner (55, D-236); rà lộ trình lần hai (D-237); module `menus` cho menu của website (68, D-238).

Để sau: (trống — ý tưởng mới ghi một dòng ở đây, chưa viết đặc tả, cho tới khi xong giai đoạn A).

Đã cân nhắc và không làm: API dữ liệu tự
sinh từ bảng, plugin WASM nạp lúc chạy, trang quản trị dựng sẵn (`docs/research/frameworks.md`,
D-005, D-026). Thư viện số thập phân và module sổ cái: đã quyết ở D-187, D-188.
