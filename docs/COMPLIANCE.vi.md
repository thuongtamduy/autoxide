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
| Quy tắc pháp lý có ngày hiệu lực, chính sách của khách | `/rules`, `/policies…` |
| **Hồ sơ pháp lý website và Bộ Công Thương** | `/legal-profiles…`, `/websites…`, `/public/badge`, `/public/owner` |

Hướng dẫn này nói về phần cuối, cùng mẫu trang bắt buộc của `content` đi kèm.

## Thủ tục với Bộ Công Thương

Pháp luật Việt Nam (NĐ 52/2013/NĐ-CP sửa bởi NĐ 85/2021/NĐ-CP; từ 01/7/2026 thêm Luật Thương mại điện tử
122/2025/QH15 và nghị định hướng dẫn — đối chiếu văn bản gốc) yêu cầu:

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
