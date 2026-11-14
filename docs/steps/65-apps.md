# Bước 65 — Chợ ứng dụng qua OAuth 2.0 (`apps`)

> Lộ trình: `docs/ROADMAP.md` bước 65 · Quyết định: D-221 (thay mục "để sau" của D-205) · Đặc tả liên quan: 54
> (phân quyền), 40 (MCP), 33 (khách)

## Mục tiêu

Nhà phát triển bên ngoài viết ứng dụng (kế toán, giao hàng, email marketing, AI agent) cắm vào Autoxide như
Shopify App Store: khách bấm "cài", xem ứng dụng xin những quyền gì, đồng ý; ứng dụng nhận token **chỉ trong
các quyền đó, chỉ trong khách đó**, gọi REST / MCP như mọi người gọi khác; khách gỡ là token mất hiệu lực ngay.

## Bắt đầu khi

Bước 54 (phân quyền chuẩn: hợp đồng quyền máy đọc, thu hồi tức thì, key chi tiết) và 33 (khách).

## Thiết kế

- **Máy chủ ủy quyền OAuth 2.0 trong `auth`** theo OAuth 2.1 (bản nháp của IETF, gom RFC 6749 + thực hành tốt):
  - chỉ luồng `authorization_code` + **PKCE bắt buộc** (RFC 7636) và `refresh_token` xoay vòng; luồng
    `client_credentials` cho ứng dụng máy–máy của chính khách;
  - không có luồng implicit, không password grant;
  - `redirect_uri` khớp chính xác đã đăng ký; `state` bắt buộc;
  - phạm vi (`scope`) = **tên permission** (`orders.read`, `catalog.write`…), kèm độ sâu / nút tối đa (bước 34);
  - token truy cập ngắn hạn (mặc định 1 giờ), không mang quyền trong token (thu hồi tức thì, D-205 P11);
  - endpoint chuẩn: `/oauth/authorize`, `/oauth/token`, thu hồi (RFC 7009), kiểm tra token (RFC 7662), tài liệu
    máy chủ (RFC 8414 `/.well-known/oauth-authorization-server`); chỉ báo tài nguyên (RFC 8707) cho MCP.
- **Ứng dụng** (`apps_apps`: chủ ứng dụng (nhà phát triển), tên, mô tả, logo, trang chính sách quyền riêng tư,
  `redirect_uris`, quyền xin, webhook nhận, trạng thái duyệt `draft/in_review/published/suspended`). Ứng dụng
  công khai trên chợ cần **nền tảng duyệt** (kiểm tra quyền xin có tương xứng, chính sách dữ liệu); ứng dụng
  riêng của một khách không cần duyệt.
- **Cài đặt** (`apps_installations`: khách, ứng dụng, quyền đã đồng ý, nút giới hạn, người đồng ý, thời điểm,
  `revoked_at`). Màn hình đồng ý do giao diện dựng từ `GET /api/v1/apps/consent?client_id&scope`, liệt kê quyền
  bằng mô tả của permission (đã có trong catalog), đánh dấu quyền rủi ro (`risk` của D-205). Ứng dụng muốn thêm
  quyền → khách đồng ý lại.
- **Webhook theo ứng dụng:** đăng ký qua module `webhooks` sẵn có, chỉ event ứng dụng có quyền đọc.
- **Chặn lạm dụng:** rate limit theo ứng dụng × khách; audit ghi `actor_kind = app` + `app_id`; khách xem nhật ký
  gọi của ứng dụng (bàn kỹ thuật bước 52); nền tảng tạm khoá ứng dụng vi phạm (token của mọi khách mất hiệu lực).
- **Thanh toán cho ứng dụng trả phí:** để sau (cần chia doanh thu, D-187); bước này chỉ ứng dụng miễn phí hay tự
  thu ngoài.

### API, MCP, quyền, lỗi

- REST `/api/v1/apps`: nhà phát triển (`/developer/apps`, khoá bí mật xoay vòng, gửi duyệt), chợ công khai
  (`/public/apps`), khách (`/installations`, gỡ), nền tảng (`/review`).
- MCP: AI agent bên ngoài nối MCP bằng token OAuth của ứng dụng đã cài (chuẩn ủy quyền của MCP).
- Permission: `apps.install` (khách), `apps.develop` (nhà phát triển), `apps.review` (nhân sự nền tảng).
- Lỗi theo chuẩn OAuth (`invalid_request`, `invalid_scope`, `access_denied`…) cho endpoint OAuth; còn lại
  `apps.not_found`, `apps.scope_not_allowed`, `apps.suspended`.

## Test bắt buộc (cả hai database)

`thieu_pkce_bi_tu_choi`, `redirect_uri_khac_bi_tu_choi`, `token_chi_trong_quyen_da_dong_y`,
`token_khach_a_khong_dung_o_khach_b`, `go_ung_dung_token_mat_hieu_luc_ngay`, `refresh_token_dung_lai_bi_thu_hoi_ca_chuoi`,
`ung_dung_bi_khoa_moi_khach_mat_quyen`, `well_known_dung_rfc_8414`.

## Căn cứ pháp lý và chuẩn

| Nội dung | Văn bản / chuẩn | Ghi chú |
|---|---|---|
| Chuyển dữ liệu cá nhân cho bên thứ ba (ứng dụng) cần thông báo / đồng ý, ghi nhận | Luật 91/2025/QH15; NĐ 356/2025 | Cần đối chiếu: khách là bên kiểm soát, nhà phát triển là bên xử lý |
| OAuth 2.0 / 2.1, PKCE, thu hồi, kiểm tra token, siêu dữ liệu máy chủ | RFC 6749, 7636, 7009, 7662, 8414, 8707; bản nháp OAuth 2.1 | Theo bản ổn định tại lúc làm |

## Agent làm bước này tự quyết

Thời hạn token; thư viện OAuth (tự viết theo RFC hay crate có sẵn — tra `check-library-api`, ghi `STACK.md`); tiêu
chí duyệt ứng dụng. Ghi vào DECISIONS.
