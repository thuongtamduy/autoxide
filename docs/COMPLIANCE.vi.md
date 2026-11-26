# Tuân thủ pháp luật (Việt Nam)

[English](COMPLIANCE.md)

Module `compliance` giúp một khách chạy website thương mại tại Việt Nam mà không bỏ sót các nghĩa vụ pháp lý
nền. Module luôn bật (không bán theo gói). Mọi điểm luật hay đổi — danh mục hồ sơ, thời hạn, miền logo được
chấp nhận — là **dữ liệu có ngày hiệu lực** (quy tắc trong `modules/compliance/rules/vn.json`, nhập vào
database), không phải code.

> **Không phải tư vấn pháp lý.** Quy tắc, danh sách kiểm và mẫu đi kèm được viết từ nguồn thứ cấp. Mục nào
> cũng ghi `needs_legal_review: true`: hãy đối chiếu văn bản gốc, cùng người có chuyên môn pháp lý, trước khi
> dựa vào.

Module gồm:

| Phần | Endpoint (dưới `/api/v1/compliance`) |
| --- | --- |
| Định danh (KYC) người bán, người livestream, người tiếp thị liên kết | `/kyc/…`, `/me/kyc…` |
| Xác thực tài khoản đăng nội dung (số điện thoại / định danh) | `/me/verification`, `/me/phone-verifications…` |
| Văn bản pháp lý có phiên bản và bằng chứng chấp nhận | `/documents…`, `/me/acceptances`, `/public/documents/{kind}` |
| Sổ đồng ý | `/me/consents` |
| **Dữ liệu cá nhân**: mục đích và căn cứ xử lý, thời hạn lưu, sổ hoạt động xử lý, nơi lưu dữ liệu | `/purposes…`, `/retention`, `/processing-register`, `/residency`, `/public/purposes` |
| **Sự cố dữ liệu cá nhân** (72 giờ) và **yêu cầu của chủ thể dữ liệu** qua kênh khác | `/incidents…`, `/subject-requests…` |
| **Mã theo dõi quảng cáo và đồng ý cookie** | `/tracking-tags…`, `/cookie-settings/{org_id}`, `/public/consent-config`, `/public/cookie-consents` |
| **Báo vi phạm** (sở hữu trí tuệ, hàng giả) và **khiếu nại của người tiêu dùng**, cung cấp thông tin cho cơ quan | `/cases…`, `/respondent/cases`, `/disclosures`, `/public/reports`, `/public/complaints`, `/public/cases/{id}` |
| Quy tắc pháp lý có ngày hiệu lực, chính sách của khách | `/rules`, `/policies…` |
| **Hồ sơ pháp lý website và Bộ Công Thương** | `/legal-profiles…`, `/websites…`, `/public/badge`, `/public/owner` |

Hướng dẫn này nói về thủ tục với Bộ Công Thương cùng mẫu trang bắt buộc của `content`, rồi dữ liệu cá nhân
(kèm sổ tay sự cố 72 giờ), báo vi phạm cùng khiếu nại của người tiêu dùng, và mã theo dõi quảng cáo với
đồng ý cookie.

## Thủ tục với Bộ Công Thương

Pháp luật Việt Nam (từ 01/7/2026 là Luật Thương mại điện tử 122/2025/QH15 và NĐ 248/2026/NĐ-CP, thay NĐ
52/2013/NĐ-CP và NĐ 85/2021/NĐ-CP; trước đó là hai nghị định này — đối chiếu văn bản gốc) yêu cầu các mục dưới
đây. Quy tắc `moit.checklist.*` phiên bản 2 áp dụng từ 01/7/2026. Website đã thông báo hay đăng ký trước ngày đó
làm lại hồ sơ theo nghị định mới trong hạn chuyển tiếp (một bản tổng quan nêu 30/6/2027 — chưa xác nhận); định danh
điện tử người bán, người livestream áp dụng từ 01/01/2027. Cơ quan tiếp nhận hồ sơ, việc còn dùng online.gov.vn và
logo chưa xác nhận — cần đối chiếu.

- **website bán hàng** (bạn bán hàng của mình) phải **thông báo** với Bộ — sau đó gắn logo xanh "Đã thông
  báo Bộ Công Thương";
- **sàn hay trung gian** (người khác bán qua bạn) phải **đăng ký** — logo đỏ "Đã đăng ký Bộ Công Thương";
  hồ sơ có đề án cung cấp dịch vụ và quy chế quản lý hoạt động;
- website công khai thông tin chủ sở hữu và các chính sách (điều kiện giao dịch chung, vận chuyển, thanh
  toán, đổi trả, kiểm hàng, bảo vệ dữ liệu cá nhân, khiếu nại).

Con người nộp hồ sơ trên [online.gov.vn](https://online.gov.vn); không có API công khai. Autoxide giữ dữ liệu,
báo còn thiếu gì, soạn nháp bộ hồ sơ và hiện logo khi bạn ghi kết quả.

### 1. Hồ sơ pháp lý của chủ sở hữu

Mỗi nút tổ chức một hồ sơ. Hồ sơ của chính khách thuộc nút gốc (bỏ `org_id`); người bán trên sàn có hồ sơ ở
nút của mình. Website dùng hồ sơ của nút của nó, hoặc của nút cấp trên gần nhất có hồ sơ.

```http
POST /api/v1/compliance/legal-profiles
{
  "entity_type": "organization",
  "legal_name": "Cong ty TNHH Vi Du",
  "tax_code": "0101234567",
  "registration_number": "0101234567",
  "registration_issued_on": "2020-01-02",
  "registration_issued_by": "Phong Dang ky kinh doanh Ha Noi",
  "address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "12 Trang Tien"},
  "representative_name": "Nguyen Van A",
  "phone": "024 1234 5678",
  "email": "lienhe@shop.example"
}
```

- `entity_type`: `individual`, `household_business` hoặc `organization`.
- Địa chỉ được kiểm theo danh mục đơn vị hành chính của module `address` (nhập trước:
  `autoxide task address.import`).
- `PATCH /legal-profiles/{id}` sửa một số trường (`""` xoá một trường chữ, `null` xoá `address`); gửi
  `If-Match` với `ETag` đã đọc.
- `kyc_profile_id` gắn hồ sơ định danh của chủ sở hữu; danh sách kiểm của sàn cần nó ở trạng thái `verified`.

Permission: `compliance.legal.read`, `compliance.legal.manage`. Cả hai có phạm vi: nhân sự của người bán chỉ
xem và sửa hồ sơ, website của nút mình.

### 2. Website

```http
POST /api/v1/compliance/websites
{"domain": "shop.example", "kind": "sales"}
```

- `kind`: `sales` (thông báo) hoặc `platform` (đăng ký). Đổi `kind` sau này thì thủ tục làm lại từ đầu.
- `channel`: `website` (mặc định) hoặc `mobile_app`.
- `org_id`: nút của website (vd cửa hàng riêng của một người bán); bỏ trống là cả khách.

### 3. Trang bắt buộc

Soạn từng trang trong `content` — mẫu ở mục sau cho sẵn nội dung khởi đầu — đăng trang, rồi gắn vào website:

```http
PUT /api/v1/compliance/websites/{id}/pages/return_policy
{"entry_id": "<id của trang trong content>"}
```

Loại trang: `owner_info`, `privacy_policy`, `return_policy`, `shipping_policy`, `payment_policy`,
`inspection_policy`, `complaint_policy`, `terms_of_use`, `general_conditions`, và với sàn thêm
`platform_rules`, `seller_agreement`. Trang đã gắn chỉ được tính khi đã đăng.

### 4. Danh sách kiểm

`GET /api/v1/compliance/websites/{id}/checklist` (MCP `compliance.website_checklist`) liệt kê mọi mục của quy
tắc `moit.checklist.sales` hoặc `moit.checklist.platform` đang hiệu lực, mỗi mục có `status`:

| Trạng thái | Nghĩa |
| --- | --- |
| `met` | Đã xong. |
| `missing` | Cần làm; `hint` nói endpoint cần gọi. |
| `unknown` | Autoxide chưa trả lời được (vd chưa module nào trả lời "trang sản phẩm hiện người bán") — tự kiểm. |
| `manual` | Luôn do người kiểm (vd tài khoản của bạn trên online.gov.vn). |

Mỗi mục kèm `legal_basis`. Khi quy định đổi, nhân sự nền tảng nhập phiên bản quy tắc mới
(`POST /api/v1/compliance/rules`) và danh sách kiểm đổi theo — không cần nâng cấp.

**Bán trước khi xong thủ tục** được hiện là "bắt buộc theo luật" (`selling_before_done: "label"`), không bị
chặn: các nguồn tìm được coi đó là vi phạm hành chính bị phạt tiền, không phải lệnh cấm giao dịch. Hãy đối
chiếu với người có chuyên môn pháp lý (quyết định D-358).

### 5. Bộ hồ sơ

`GET /api/v1/compliance/websites/{id}/dossier` (MCP `compliance.website_dossier`) trả về, điền từ dữ liệu của
bạn:

- `form`: các trường tờ khai (chủ sở hữu, website, định danh, trang đã gắn kèm URL đầy đủ, đầu mối khiếu nại);
- `documents`: bản nháp Markdown tiếng Việt — thông tin khai báo; với sàn thêm nháp đề án cung cấp dịch vụ và
  nháp quy chế quản lý hoạt động;
- `checklist` và `disclaimer`.

Hoàn thiện và duyệt bản nháp cùng người có chuyên môn pháp lý, rồi nộp trên online.gov.vn.

### 6. Trạng thái và logo

Ghi từng bước bằng tay:

```http
POST /api/v1/compliance/websites/{id}/status
{"status": "submitted", "reference": "HS-001"}
```

Trạng thái: `not_submitted`, `submitted`, `needs_supplement`, `notified` (website bán hàng), `registered`
(sàn), `rejected`, `revoked`. Với `notified` / `registered`, dán đoạn mã logo Bộ cấp vào `snippet` (hoặc gửi
`link_url` và `image_url`). Chỉ nhận một link tới `online.gov.vn` bọc một ảnh từ `online.gov.vn` — script,
thuộc tính sự kiện, style hay miền khác trả `compliance.logo_snippet_invalid` (422). Đoạn mã không bao giờ
được lưu hay trả lại nguyên văn: Autoxide giữ hai địa chỉ và dựng lại đoạn mã sạch.

### 7. Cho giao diện (công khai, không cần đăng nhập)

| Endpoint | Trả về |
| --- | --- |
| `GET /api/v1/compliance/public/badge` | Logo của website đang xem: `color` (`blue` / `red`), `link_url`, `image_url`, `alt`, `snippet` dựng sẵn. 404 khi website chưa có logo. |
| `GET /api/v1/compliance/public/owner` | Thông tin chủ sở hữu cho chân trang. 404 khi chưa có hồ sơ pháp lý. |

Website được tìm theo `?domain=`, rồi tên miền của request, rồi website của nút đang xem (`?site=<id nút>` hay
tên miền gắn với nút). Cả hai gửi `ETag`; gửi lại trong `If-None-Match` để nhận `304`.

### 8. `autoxide doctor`

Bước kiểm `compliance.moit_readiness` cảnh báo (không bao giờ báo hỏng) khi chưa website nào có hồ sơ pháp lý,
hoặc website chưa nộp còn thiếu mục trong danh sách kiểm — kèm địa chỉ danh sách kiểm của từng website.

## Mẫu trang bắt buộc (`content`)

Module `content` có sẵn nội dung khởi đầu cho các trang bắt buộc, tiếng Anh và tiếng Việt:

```http
GET  /api/v1/content/templates
POST /api/v1/content/templates/{key}/drafts
{"locale": "vi", "variables": {"policy": {"return_days": 7}}}
```

- Khoá: `owner_info`, `privacy_policy`, `return_policy`, `shipping_policy`, `payment_policy`,
  `inspection_policy`, `complaint_policy`, `terms_of_use`, `general_conditions`; với sàn thêm
  `platform_rules`, `seller_agreement`, `seller_info`.
- Bản nháp được điền bằng hồ sơ pháp lý của nút (`owner.*`, `contact.*`, đọc từ `compliance`), rồi bằng
  `variables` bạn gửi (giá trị gửi thắng). Mọi giá trị chỉ vào phần chữ — không bao giờ thêm được thẻ. Còn
  thiếu biến bắt buộc thì trả `content.template_variables_missing` (422) liệt kê đủ một lần.
- Kết quả là trang **nháp** (`content.write`); duyệt, đăng, rồi gắn vào website.
- Mẫu nào cũng mở đầu bằng "Nội dung mẫu — cần người có chuyên môn pháp lý duyệt trước khi dùng".

## Dữ liệu cá nhân

Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15 và Nghị định 356/2025/NĐ-CP (hiệu lực từ 01/01/2026 — đối chiếu văn
bản gốc cùng người có chuyên môn pháp lý). Mỗi khách là bên kiểm soát dữ liệu của chính mình.

### Mục đích và căn cứ xử lý

`GET /purposes` (`compliance.privacy.read`) liệt kê mục đích xử lý: mặc định của quy tắc
`privacy.default_purposes` (`service`, `marketing`, `analytics`, `legal`) gộp với mục đích riêng của khách.
`POST /purposes` thêm, `PATCH /purposes/{key}` sửa (sửa mục mặc định thì lưu bản của khách) — cả hai cần
`compliance.privacy.manage`. Mỗi mục đích có `category`, `legal_basis` (`consent`, `contract`,
`legal_obligation`, `vital_interest`, `public_interest`), loại dữ liệu, bên nhận (tên, nước) và mô tả theo
ngôn ngữ. `GET /public/purposes` (không cần đăng nhập) liệt kê mục đích đang dùng cho thông báo về quyền riêng
tư; nhà cung cấp mã theo dõi đang bật hiện ở đó như bên nhận, kèm nước nhận dữ liệu.

### Thời hạn lưu

Mỗi module khai dữ liệu cá nhân nó giữ và giữ bao lâu (`Module::retention`, đọc từ cấu hình của module). `GET
/retention` liệt kê, mỗi mục so với quy tắc `privacy.retention_limits`: `ok`, `exceeds_limit` hay
`forever_not_allowed` (chỉ `accounting` — bằng chứng luật buộc giữ — được giữ mãi). Giới hạn đi kèm là mặc định
thận trọng, không phải con số của luật. `autoxide doctor` cảnh báo (`compliance.retention`) khi có mục không
`ok`; hạ cấu hình ghi ở `setting`.

### Sổ hoạt động xử lý

`GET /processing-register` trả mục đích, căn cứ, loại dữ liệu, bên nhận, thời hạn lưu và nơi lưu dữ liệu — đầu
vào cho hồ sơ đánh giá tác động xử lý dữ liệu cá nhân; mẫu hồ sơ chính thức cần người có chuyên môn pháp lý.

### Nơi lưu dữ liệu

Có thể buộc một khách lưu dữ liệu ở một nước (Luật An ninh mạng 116/2025/QH15, hiệu lực 01/7/2026, thay Luật
An ninh mạng 2018 và Luật An toàn thông tin mạng 2015; nghị định hướng dẫn, theo nguồn thứ cấp là NĐ 333/2026/NĐ-CP,
thay NĐ 53/2022/NĐ-CP — chưa xác nhận. Ai bắt buộc lưu tại Việt Nam cần đối chiếu pháp lý):

```bash
autoxide placement country main VN          # ghi nước đặt máy chủ của một vị trí dữ liệu
```

```http
PUT /api/v1/compliance/residency
{"tenant_id": "…", "data_residency": "VN"}
```

Chỉ nhân sự nền tảng có `compliance.privacy.manage` đặt được. Bị từ chối (`tenant_residency_violation`, 409)
khi vị trí dữ liệu của khách không ở nước đó hoặc chưa rõ nước; sau này dời khách sang vị trí ở nước khác cũng bị
từ chối cùng mã, đổi nước của vị trí dữ liệu cũng vậy. `GET /residency` cho xem trạng thái của khách; `autoxide
doctor` cảnh báo (`compliance.data_residency`) khách nào chưa đáp ứng.

### Sự cố dữ liệu cá nhân: sổ tay 72 giờ

Báo cáo gửi cơ quan có hạn **72 giờ kể từ khi phát hiện** (quy tắc `privacy.incident`, dữ liệu có ngày hiệu lực).
Làm ngay khi nghi có sự cố:

1. **Ghi ngay** — `POST /incidents` với `title`, `severity` (`low`…`critical`) và `detected_at` (lúc phát hiện,
   không phải lúc bắt đầu). Kết quả có `report_due_at`. Cần `compliance.incidents.manage`.
2. **Đặt người nhận nhắc** — chính sách `incident.alert_emails` của khách (`PUT /policies/incident.alert_emails`
   với danh sách địa chỉ). Một job 15 phút một lần gửi mail ở mốc 24, 48, 60, 68 giờ và ở mốc hạn (mỗi mốc một
   lần), và phát `compliance.incident_deadline_near`.
3. **Khoanh vùng và ghi lại** — `PATCH /incidents/{id}` với `status: "contained"`, loại dữ liệu và số người;
   mỗi phát hiện, mỗi việc đã làm thêm bằng `POST /incidents/{id}/entries` (nhật ký chỉ thêm).
4. **Soạn thông báo** — `GET /incidents/{id}/draft?audience=authority&locale=vi` (và `audience=subjects` cho
   người bị ảnh hưởng). Đây là bản nháp điền từ sự cố và chính sách `privacy.contact`: điền phần còn thiếu, nhờ
   duyệt, rồi tự gửi.
5. **Ghi đã báo** — `PATCH /incidents/{id}` với `authority_notified_at` và `authority_reference` (sự cố thành
   `reported`), rồi `subjects_notified_at` khi đã báo người bị ảnh hưởng; cuối cùng `closed`.

`GET /incidents?overdue=true` liệt kê sự cố quá hạn, `autoxide doctor` (`compliance.privacy_deadlines`) cảnh
báo chúng. Hướng dẫn triển khai trỏ về đây ([DEPLOY.vi.md](DEPLOY.vi.md)).

### Yêu cầu của chủ thể dữ liệu qua kênh khác

Yêu cầu người dùng tự làm trong tài khoản (xuất, xoá) chạy ngay. Yêu cầu nhận qua email, thư, điện thoại hay từ
cơ quan được ghi sổ để không bị sót:

- `POST /subject-requests` với `kind` (`access`, `erasure`, `rectification`, `objection`, `restriction`,
  `withdrawal`), `channel` (`email`, `post`, `phone`, `authority`, `other`), `subject` (ID người dùng, địa chỉ
  email hay số điện thoại) và `received_at`. `due_at` lấy từ quy tắc `privacy.subject_requests` hiệu lực lúc
  nhận: 72 giờ, chặt hơn hạn trả lời 2 ngày làm việc của NĐ 356/2025/NĐ-CP Điều 5 (thay NĐ 13/2023/NĐ-CP). Phiên
  bản 2 của quy tắc ghi thêm số ngày thực hiện theo loại (`fulfil_within_days`, theo nguồn thứ cấp) để tham khảo;
  Autoxide chưa tính hạn thứ hai này — cần đối chiếu văn bản gốc.
- `POST /subject-requests/{id}/fulfil`: `access` của người dùng trả dữ liệu cá nhân của mọi module (gửi cho
  người đó; không lưu lại); `erasure` / `withdrawal` của email hay số điện thoại xoá giá trị đó khỏi sổ đồng ý
  (khoá băm và bản ghi giữ làm bằng chứng, để không gửi lại). Xoá cả tài khoản là `DELETE
  /api/v1/auth/users/{id}`.
- `PATCH /subject-requests/{id}` đặt `in_progress` hay `rejected` (kèm `reason`). `GET
  /subject-requests?overdue=true` liệt kê yêu cầu trễ hạn. Mọi endpoint cần `compliance.privacy.manage`.

Khi tài khoản bị xoá (`auth.user_deleted`), `compliance` xoá số điện thoại đã xác thực và mã đang chờ của người
đó, gỡ người dùng khỏi bằng chứng đồng ý cookie; bằng chứng đồng ý, văn bản đã chấp nhận và kết quả định danh giữ
trong thời gian luật yêu cầu. Bản xuất dữ liệu cá nhân của người dùng (`GET /api/v1/auth/me/export`) có phần của
`compliance`.

## Báo vi phạm và khiếu nại của người tiêu dùng

Vi phạm sở hữu trí tuệ, hàng giả, hàng cấm được báo dưới dạng **vụ việc**; khiếu nại của người tiêu
dùng cũng vậy. Một vụ việc đi: tiếp nhận → (ẩn đối tượng) → phản hồi của bên bị phản ánh → quyết định,
có hạn và lưu vết chỉ thêm. Căn cứ — cần đối chiếu văn bản gốc: Luật Thương mại điện tử 122/2025/QH15 và
NĐ 248/2026/NĐ-CP (từ 01/7/2026; gỡ khi cơ quan có thẩm quyền yêu cầu, trước đó là NĐ 85/2021/NĐ-CP), Luật Sở
hữu trí tuệ, Luật Bảo vệ quyền lợi người tiêu
dùng 19/2023/QH15 và NĐ 55/2024/NĐ-CP (khiếu nại).

| Ai | Endpoint (dưới `/api/v1/compliance`) |
| --- | --- |
| Bất kỳ ai (form công khai, khách suy từ tên miền) | `POST /public/reports`, `POST /public/complaints`, `GET /public/complaints`, `GET /public/cases/{id}?token=`, `GET /public/moderation-reasons` |
| Nhân sự (`compliance.cases.read` / `.handle`) | `GET\|POST /cases`, `GET /cases/{id}`, `POST /cases/{id}/entries`, `…/restrict`, `…/restore`, `…/decision`, `…/close` |
| Bên bị phản ánh (đăng nhập) | `GET /respondent/cases`, `POST /cases/{id}/responses` |
| Nhân sự (`compliance.disclosures.manage`) | `GET\|POST /disclosures` |

**Tiếp nhận.** Form công khai xin token form trước (`GET /api/v1/antispam/token?scope=compliance:reports`,
hay `compliance:complaints`) và bị giới hạn theo khối mạng và email mỗi giờ
(`AUTOXIDE_COMPLIANCE_PUBLIC_REPORTS_PER_IP_PER_HOUR`, mặc định 5). Đối tượng là thứ bị báo, dạng
`<module>.<type>:<id>` (vd `content.entry:…`, `comments.comment:…`; sản phẩm khi có bước 41); phải có
thật. Người gửi nhận `tracking_token` một lần — chỉ lưu SHA-256 — và thư báo đã nhận khi có để lại địa
chỉ (link: `AUTOXIDE_COMPLIANCE_CASE_TRACKING_URL` có `{case}` và `{token}`). Nhân sự ghi vụ nhận qua
thư (`POST /cases`, `reporter_kind` `rights_holder` hay `authority`).

**Hạn** lấy từ quy tắc `cases.deadlines`: số giờ để xác nhận, số ngày làm việc để phản hồi và để quyết
định theo loại (khiếu nại của người tiêu dùng: 3 và 7), và 24 giờ để quyết định yêu cầu của cơ quan.
Ngày làm việc bỏ thứ Bảy, Chủ nhật và ngày lễ ghi trong quy tắc (giờ Việt Nam); Tết âm lịch và ngày
nghỉ bù nhập mỗi năm. Lịch hằng giờ `compliance.case_deadlines` phát event `compliance.case_overdue` và
gửi thư tới `email` của chính sách `complaints.contact`, mỗi mốc hạn một lần.

**Ẩn đối tượng.** Yêu cầu của cơ quan ẩn đối tượng ngay; báo vi phạm sở hữu trí tuệ hay hàng giả cũng
vậy khi khách đặt chính sách `cases.auto_restrict` là `true`. Nhân sự ẩn hay hiện tay (`/restrict`,
`/restore`). `compliance` không bao giờ đụng bảng của module khác: nó phát `compliance.target_restricted`
/ `compliance.target_restored` (hợp đồng kernel `autoxide_kernel::moderation`), module sở hữu tự ẩn —
`content` trả **451** `content.restricted` và bỏ trang khỏi danh sách, sitemap, feed, tìm kiếm;
`comments` ẩn bình luận và khôi phục trạng thái cũ. Đối tượng chỉ hiện lại khi không còn vụ nào khác
đang ẩn nó.

**Bên bị phản ánh** là chủ đối tượng (từ sổ đối tượng của kernel, vd tác giả của trang) và người có
`compliance.cases.respond` trên nút bị phản ánh (`respondent_org_id`, người bán trên sàn). Họ thấy vụ của
mình không kèm cách liên lạc của người báo, và phản hồi trước `respond_by`; muộn hơn thì
`compliance.response_closed` (409).

**Quyết định.** Một người đăng nhập có `compliance.cases.handle` quyết định (`upheld` hay
`partially_upheld` giữ ẩn; `rejected` hiện lại); API key nhận `compliance.session_required`. Lý do hiện cho
cả hai bên và gửi thư cho người báo. Không có tool MCP để quyết định.

**Cung cấp thông tin cho cơ quan.** `POST /disclosures` (người đăng nhập, `compliance.disclosures.manage`)
ghi cơ quan, số văn bản yêu cầu, căn cứ và tên các trường đã đưa, rồi trả giá trị một lần — từ hồ sơ
pháp lý và hồ sơ định danh của người bán mà `compliance` giữ. Bản ghi và event
`compliance.disclosure_recorded` không bao giờ chứa giá trị.

Tool MCP: `compliance.list_cases`, `compliance.get_case` (đọc), `compliance.add_case_note`,
`compliance.restrict_case_target`.

### Bảo vệ người tiêu dùng

- **Ẩn đánh giá.** Nội dung của người tiêu dùng chỉ được ẩn vì các lý do của quy tắc
  `moderation.hide_reasons` (`spam`, `illegal_content`, `personal_data`, `unrelated`, `offensive` — cần
  đối chiếu). Module kiểm duyệt đánh giá hỏi trait `ModerationPolicy` (`autoxide_kernel::moderation`) trước khi
  ẩn — `orders` ánh xạ lý do của mình (`unlawful_content` → `illegal_content`, `abusive_language` → `offensive`,
  `spam_or_advertising` → `spam`, `not_about_product` → `unrelated`), nên lý do khách tự thêm không ẩn được
  đánh giá khi đã cài `compliance`
  (`compliance.hide_reason_not_allowed`, 422) và ghi lý do trong event của chính nó; danh sách công khai ở
  `GET /public/moderation-reasons`. Đánh giá tiêu cực mà hợp lệ vẫn hiện.
- **Điều kiện giao dịch chung** là văn bản pháp lý có phiên bản loại `general_conditions`, công khai ở
  `GET /public/documents/general_conditions`.
- **Kênh khiếu nại.** `GET /public/complaints` cho trang khiếu nại của website liên hệ (chính sách
  `complaints.contact`) và hạn xử lý; khiếu nại là vụ việc loại `consumer_complaint`.

### Nhãn AI

Nội dung viết hay sửa từ gợi ý của AI mang `ai_assisted` (`content`) và alt text của ảnh lấy từ gợi ý
mang `alt_ai_assisted` (`media`), endpoint công khai trả về để giao diện gắn nhãn (Luật Trí tuệ nhân tạo
134/2025/QH15, từ 1/3/2026 — cần đối chiếu nghị định). Xem [`CONTENT.vi.md`](CONTENT.vi.md) và
[`MEDIA.vi.md`](MEDIA.vi.md).

## Mã theo dõi quảng cáo và đồng ý cookie

Khách gắn mã đo của nền tảng quảng cáo mà không phải dán script, và mã chỉ chạy sau khi người xem đồng ý. (Luật
Bảo vệ dữ liệu cá nhân 91/2025/QH15, NĐ 356/2025/NĐ-CP, Luật Bảo vệ quyền lợi người tiêu dùng 19/2023/QH15 —
cookie đo lường có cần đồng ý không, và hồ sơ đánh giá tác động khi chuyển dữ liệu ra nước ngoài, cần đối chiếu
pháp lý; thiết kế chọn cách hiểu chặt: hỏi trước.)

### Nhà cung cấp và mã

`GET /tracking-providers` liệt kê nhà cung cấp cho sẵn; khách chỉ điền mã ID:

| `provider` | Tên | Nhóm | Dạng mã | Dữ liệu đi tới |
| --- | --- | --- | --- | --- |
| `ga4` | Google Analytics 4 | `analytics` | `G-XXXXXXXXXX` | US |
| `gtm` | Google Tag Manager | `marketing` | `GTM-XXXXXXX` | US |
| `google_ads` | Google Ads | `marketing` | `AW-123456789` | US |
| `meta` | Meta Pixel | `marketing` | chữ số | US |
| `tiktok` | TikTok Pixel | `marketing` | chữ và số | SG |
| `zalo` | Zalo Ads Pixel | `marketing` | chữ số | VN |

`POST /tracking-tags {org_id, provider, tracking_id}`, `PATCH /tracking-tags/{id}` (`status`: `active` /
`paused`), `DELETE /tracking-tags/{id}` — cần `compliance.tracking.manage` trên nút của website. Mã sai dạng trả
`compliance.tracking_id_invalid` kèm dạng mong đợi; nhà cung cấp lạ trả `compliance.tracking_provider_unknown`.

### Banner

`PUT /cookie-settings/{org_id}` đặt `enabled`, các nhóm tuỳ chọn được hỏi `categories` (`preferences`,
`analytics`, `marketing`; `necessary` luôn bật và không mang mã nào) và `position`. Nhóm của mã đang bật luôn được
hỏi. Đổi nhóm hay nhà cung cấp đang bật thì tăng `policy_version`: người xem được hỏi lại. `GET
/cookie-consents/stats?org_id=` cho số gộp 30 ngày gần nhất, không bao giờ trả bản ghi riêng.

### Cho giao diện

```http
GET  /api/v1/compliance/public/consent-config?granted=analytics&locale=vi
POST /api/v1/compliance/public/cookie-consents
{"consent_id": "<ngẫu nhiên, giữ trong cookie cần thiết>", "categories": ["analytics"], "policy_version": 3}
```

Hợp đồng với giao diện:

- **Không tải gì trước khi đồng ý.** Không có `granted` thì cấu hình không trả mã nào; có thì chỉ mã đang bật của
  các nhóm đó. Đọc lựa chọn của người xem từ cookie cần thiết của bạn rồi gửi lên.
- **Từ chối dễ như đồng ý.** Hiện `texts.reject_all` nổi bật ngang `texts.accept_all`, ngay lớp đầu; không nhóm nào
  được chọn sẵn (`preselected: false`). `texts.legal` không đổi được.
- **Ghi lựa chọn** bằng `POST /public/cookie-consents` (không cần đăng nhập; giới hạn theo khối mạng bằng
  `AUTOXIDE_COMPLIANCE_COOKIE_CONSENTS_PER_IP_PER_HOUR`). Lưu mã ngẫu nhiên, nhóm, phiên bản và bản băm của khối
  mạng — không bao giờ lưu IP thô — trong số ngày của `privacy.consent_proof_days`. Người xem đã đăng nhập thì lựa
  chọn đi vào cả sổ đồng ý (mục đích `analytics` / `marketing`, phạm vi `website:<id nút>`). `policy_version` cũ
  trả `compliance.cookie_policy_changed` (409): hiện lại banner.
- **Không làm chậm trang.** Tải mã `async`, sau khi trang đã hiện (`loading: after_consent_deferred`); thêm
  `csp_hosts` vào Content-Security-Policy; không tải mã ở trang đăng nhập và trang tài khoản; ở trang thanh toán
  chỉ gửi sự kiện chuyển đổi chuẩn (`view_item`, `add_to_cart`, `begin_checkout`, `purchase`).
- Chưa làm gửi chuyển đổi từ máy chủ tới nền tảng quảng cáo (cần đối chiếu riêng về chuyển dữ liệu cá nhân).
