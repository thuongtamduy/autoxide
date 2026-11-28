## D-20261128-domain-site-snapshot · Website theo `Host` đọc từ ảnh chụp tên miền đã xác minh, bỏ cache theo `Host` — 2026-11-28 — agent quyết

Chủ đề: Đa khách, tổ chức, gói

Quyết định: `DatabaseRegistry::domain_site` không còn cache riêng theo `Host` (trần 10 000 mục, đầy thì xoá sạch). Ảnh chụp
mọi tên miền `verified` (CORS, D-20261125-tenant-cors) nay giữ cả nút gắn với tên miền (`org_id`), và `domain_site` tra
ảnh chụp đó: `Host` không phải tên miền của ai thì chỉ tra bản đồ, không truy vấn, không để lại mục nào. Ảnh chụp bị cắt
ở trần 100 000 tên miền thì tên miền ngoài ảnh chụp đọc thẳng database (không cache) — website vẫn chạy, chỉ CORS của
chúng fail closed như trước. Lượt đọc ảnh chụp thay được bằng hàm giả trong test (`known_with`), có test cho lượt đọc
treo quá `READ_TIMEOUT`.

Lý do: P16 — dội `Host` ngẫu nhiên làm cache đầy rồi xoá sạch liên tục, tên miền thật phải đọc database mỗi request.
Ảnh chụp đã có, đã dùng chung một lượt đọc cho nhiều request, đã quên ngay khi tên miền đổi (`invalidate_domain`);
dùng nó thì chỉ còn một nguồn sự thật.

Đã cân nhắc: LRU cho cache theo `Host` (thêm thư viện hay tự viết, vẫn để tên lạ chiếm chỗ); chỉ cache tên có thật
(vẫn cần truy vấn cho mỗi tên lạ).
