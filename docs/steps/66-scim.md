# Bước 66 — Đồng bộ nhân viên từ hệ thống doanh nghiệp (SCIM 2.0)

> Lộ trình: `docs/ROADMAP.md` bước 66 · Quyết định: D-221 (thay mục "để sau" của D-205) · Đặc tả liên quan: 34
> (tổ chức, membership), 40 P3 (đăng nhập OIDC), 54 (phân quyền)

## Mục tiêu

Khách là doanh nghiệp lớn quản lý nhân viên ở Microsoft Entra ID, Google Workspace, Okta: thêm nhân viên ở đó thì
tài khoản tự có trong Autoxide với đúng nhóm / quyền; nhân viên nghỉ việc thì tài khoản tự khoá ngay, phiên đăng
nhập và API key bị thu hồi — không chờ ai nhớ xoá tay.

## Bắt đầu khi

Bước 34 (membership theo nút, role) và 40 P3 (đăng nhập qua OIDC — SCIM đi kèm SSO). Không phụ thuộc bước 54 nhưng
dùng thu hồi tức thì nếu đã có.

## Thiết kế

- Endpoint SCIM 2.0 trong `auth` dưới `/scim/v2/{tenant}/` (RFC 7643 lược đồ, RFC 7644 giao thức): `Users`,
  `Groups`, `ServiceProviderConfig`, `ResourceTypes`, `Schemas`; `GET` (lọc `filter` mức `eq` / `co` / `sw` trên
  `userName`, `externalId`, `emails`), `POST`, `PUT`, `PATCH` (RFC 7644 §3.5.2), `DELETE`; phân trang
  `startIndex` / `count` theo chuẩn (riêng endpoint này, ngoại lệ có chủ đích với quy ước `cursor` vì chuẩn bắt
  buộc — ghi DECISIONS); trả lỗi theo lược đồ lỗi SCIM.
- **Xác thực:** token SCIM riêng cho mỗi khách (loại API key `scim`, chỉ dùng được ở `/scim`), xoay vòng được.
- **Ánh xạ:** `User` ↔ tài khoản nhân viên của khách (`realm = staff`, bước 34) qua `externalId`; `active=false` →
  khoá tài khoản, thu hồi phiên, refresh token, API key của người đó (cùng transaction ghi trạng thái + event để
  module khác dọn); `DELETE` → khoá và đánh dấu xoá theo quy trình xoá tài khoản sẵn có. `Group` ↔ **luật ánh xạ**
  khách cấu hình: nhóm X → role R ở nút N (bảng `auth_scim_group_mappings`); thành viên nhóm đổi → membership đổi.
- Không ghi đè dữ liệu Autoxide tự quản (mật khẩu, MFA); tài khoản có `externalId` thì đổi email / tên chỉ qua SCIM
  (giao diện khoá trường đó).
- Nhật ký đồng bộ trong audit (`actor_kind = scim`); báo lỗi ánh xạ (nhóm chưa có luật) vào hộp việc.

### Quyền, lỗi

- Permission: `auth.scim.manage` (tạo token, luật ánh xạ). Tính năng gói `auth.scim`.
- Lỗi theo SCIM (`invalidFilter`, `uniqueness`, `mutability`, `invalidValue`) cho endpoint SCIM.

## Test bắt buộc (cả hai database)

`active_false_khoa_va_thu_hoi_phien_ngay`, `nhom_doi_thi_membership_doi`, `patch_theo_rfc_7644`,
`token_scim_khong_dung_duoc_cho_api_thuong`, `token_khach_a_khong_dong_bo_khach_b`, `externalId_trung_bi_tu_choi`,
`loc_eq_co_sw_dung`.

## Căn cứ và chuẩn

RFC 7643, RFC 7644 (SCIM 2.0). Xử lý dữ liệu nhân viên theo Luật 91/2025/QH15 — khách là bên kiểm soát dữ liệu;
cần đối chiếu.

## Agent làm bước này tự quyết

Mức hỗ trợ bộ lọc SCIM; có hỗ trợ `Bulk` không (khuyến nghị: không ở bản đầu). Ghi vào DECISIONS.
