## D-20261121-api-shape · Thân request đóng ở mọi endpoint, gate kiểm hình dạng API trên openapi.json — 2026-11-21 — agent quyết

Chủ đề: HTTP, REST, OpenAPI

Quyết định:
- Mọi thân request JSON là object có `#[serde(deny_unknown_fields)]`: trường lạ (thường là gõ sai tên) trả 422
  `invalid_body`, `detail` nêu tên trường và các trường được nhận. 408/484 thân request đã theo quy ước này; thêm cho
  50 kiểu còn lại (đăng nhập, phiên, mật khẩu, email, người dùng và role của `auth`; nhãn, ghi chú, view, câu chữ,
  duyệt, việc hàng loạt, xuất của kernel; `address`, một số thân của `catalog`, `forms`, `promotions`).
- `cargo xtask check` thêm bốn luật đọc `docs/openapi.json` (`xtask/src/api_shape.rs`): `api-closed-body` (thân
  object có `additionalProperties: false`), `api-problem` (4xx/5xx là `application/problem+json` + `Problem`),
  `api-unpaged-list` (`GET` trả mảng phải là `Page<T>`), `api-page-params` (`Page<T>` nhận `limit` và `cursor`).
  Ngoại lệ có chủ đích ghi trong `xtask/api-shape-allow.txt`, mỗi dòng kèm lý do; dòng thừa chỉ cảnh báo.
- Đứng ngoài luật: SCIM 2.0 (`/api/v1/auth/scim/v2/…`, RFC 7643/7644: thân lỗi `application/scim+json`, client gửi
  kèm thuộc tính chỉ đọc mà máy chủ phải bỏ qua) — viết cứng trong luật; dòng giỏ hàng (đặc tả 45: giá / tổng client
  gửi kèm bị bỏ qua); thân ghép bằng `#[serde(flatten)]` (serde không hỗ trợ `deny_unknown_fields` cùng `flatten`);
  JSON Merge Patch của khuyến mại; 139 danh sách `Data<Vec<T>>` có sẵn (tập nhỏ có giới hạn: loại, mức thuế, tỉnh…)
  — ghi trong file ngoại lệ, endpoint mới phải dùng `Page<T>` hoặc thêm dòng có lý do.

Lý do:
- Người thử bản 0.17.0 gửi trường sai tên, máy chủ lặng lẽ bỏ qua: lỗi khó thấy, nhất là với AI agent. Quy ước
  đã có từ đầu (D-239 dựa vào nó) nhưng không có gì giữ, nên 76 thân request lệch.
- Kiểm trên `docs/openapi.json` (đã sinh từ code, gate giữ nó khớp) thay vì đọc mã nguồn: thấy đúng hình dạng client
  thấy, kể cả kiểu đặt lại tên bằng `#[schema(as = …)]`, không phụ thuộc cách viết handler.
- Đóng thân request là thay đổi phá vỡ với client gửi thừa trường; bản 0.x cho phép, ghi ở `changelog.d/` mục
  Changed. `cargo xtask api-diff` không coi "mở → đóng" là phá vỡ (chỉ "bỏ trường của thân đóng").

Đã cân nhắc:
- Kiểm trường lạ một chỗ trong `ValidJson` (đọc thành `serde_json::Value`, so với tên trường bằng `serde_ignored`):
  thêm thư viện, đọc thân hai lần trên đường nóng, vẫn không thấy trường lạ bên trong `flatten`, và tài liệu OpenAPI
  không nói gì — client không biết trước. Thuộc tính serde vừa rẻ vừa hiện trong OpenAPI.
- Đổi 139 danh sách `Data<Vec<T>>` sang `Page<T>`: phá vỡ hình dạng response của nhiều endpoint đang dùng; phải làm
  endpoint mới và khai deprecations (D-059). Để sau: rà từng endpoint, chỉ đổi cái không có giới hạn thật (mục chờ
  trong `docs/ROADMAP.md`).
- Thêm `additionalProperties: false` vào schema bằng hậu xử lý OpenAPI: tài liệu nói một đằng, code làm một nẻo.
