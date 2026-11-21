## D-20261121-automation-credential · Luật tự động hoá mang quyền của credential đã tạo / bật nó — 2026-11-21 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định:

1. Luật tạo, sửa thân luật hay bật bằng API key ghi thêm `run_as_api_key` (ID key) và `run_as_limit` (bản chụp
   `PrincipalSnapshot` của key: permission, hàng rào nút, loại bên làm) — migration `m0007_rule_credential`. Lúc chạy
   (hành động, kích hoạt số liệu, chạy thử), quyền = quyền HIỆN TẠI của `run_as` ∩ permission của key, trong hàng rào
   nút của key, mang ID và loại của key. Người dùng đăng nhập lưu / bật luật thì hai cột về `NULL` (như trước).
2. Event `auth.api_key_revoked` (chủ đề `auth.api_key:<id>`) tạm dừng mọi luật đang bật mang key đó, lý do
   `credential_revoked`. Bật lại cần người (hay key khác) bật, và luật mang quyền của bên bật.
3. Danh sách địa chỉ IP được phép của key không áp lúc luật chạy (luật chạy trên máy chủ, không có địa chỉ người gọi);
   nó đã được kiểm ở request tạo / bật luật.

Lý do: luật do API key hạn chế tạo trước đây chạy với toàn bộ quyền của chủ key, ngoài hàng rào nút của key, và tiếp
tục chạy sau khi key bị thu hồi.

Đã cân nhắc: tra key hiện tại qua module `auth` mỗi lần chạy (kernel không có hợp đồng tra API key; event thu hồi đủ
và rẻ hơn); cấm API key tạo luật (agent dùng API key là đường chính của sản phẩm). Hết hạn key (không có event) chưa
xử lý: để sau, khi `auth` có event hết hạn.
