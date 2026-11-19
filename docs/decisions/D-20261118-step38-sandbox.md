## D-20261118-step38-sandbox · Khách thử nghiệm, phần nền: cột của `kernel_tenants`, cùng placement, hạn mức mặc định 1, uỷ quyền vào bằng event — 2026-11-18 — agent quyết

Chủ đề: Đa khách, tổ chức, gói

Quyết định: Bước 38 mục sandbox (D-232), phần nền (thứ tự làm 12), làm theo đặc tả 38 Luồng 4 với các điểm chốt sau.

1. **Dữ liệu**: migration kernel `m0037_tenant_sandbox` thêm vào `kernel_tenants` (mặt điều khiển) `kind` (`production` /
   `sandbox`, NOT NULL mặc định `production`), `sandbox_of`, `sandbox_expires_at`, `sandbox_options` (JSON `products`,
   `role_id`), index `(sandbox_of)` và `(kind, sandbox_expires_at)`. Sandbox có `parent_id` NULL nên không nằm trong
   `tenant_descendants` / `tenant_ancestors`: phân phối `AllChildren`, uỷ quyền theo cây và báo cáo gộp (gốc của sandbox là
   chính nó) tự bỏ nó. `TenantContext` mang `is_sandbox()`, `sandbox_of()`, `sandbox_expires_at()`, `kind()` (sổ database
   đọc hai cột cùng câu tra khách, không thêm truy vấn).
2. **Tạo**: `POST /api/v1/kernel/tenants/{id}/sandboxes` ghi dòng `provisioning` cùng placement với khách gốc (database chung
   hay database riêng của khách), slug `<slug gốc>-sandbox-<8 ký tự cuối của id>`, rồi xếp job `kernel.sandbox_copy` (unique
   key `sandbox_copy:{sandbox}`, có dòng theo dõi ở job API chung) ở database của KHÁCH GỐC — sandbox chưa `active` thì
   worker không chạy việc của nó. Job gieo dữ liệu như khách mới (`TenantAdmin::provision`), kích hoạt, rồi ghi
   `kernel.sandbox_created` ở khách gốc; chạy lại được. Sao cấu hình (mục 14) chen vào job này trước khi kích hoạt.
3. **Hạn mức `kernel.sandboxes`**: kernel tự đếm sandbox chưa xoá của khách và so với mức của gói; khách không thuê bao
   (snapshot `unrestricted`) hay gói không có khoá này → 1 (ngoại lệ có chủ đích của "không thuê bao = không giới hạn",
   D-232); gói đặt `null` → không giới hạn. Vượt → `limit_exceeded` (403). Không giữ chỗ qua `Entitlements::reserve`: bên
   cung cấp coi khoá vắng mặt là 0, trái với mặc định 1.
4. **Uỷ quyền vào sandbox**: `auth` nghe `kernel.sandbox_created` / `_extended` / `_deleted` (ghi ở khách gốc) và tạo / dời
   hạn / thu hồi một dòng `auth_delegation` thường (khách gốc → sandbox, role đã chọn hay role `admin`, nút gốc của sandbox,
   hết hạn cùng sandbox); nhận lại event không tạo dòng thứ hai. `DelegationService::enter` không đổi: nó không đòi khách
   đích là con cháu.
5. **Gia hạn / xoá / hết hạn**: gia hạn đặt lại `now + AUTOXIDE_TENANCY_SANDBOX_DAYS` (1–365, mặc định 30). Xoá = tạm khoá
   rồi xoá hẳn theo luồng xoá khách (`TenantAdmin::erase`). Lịch `kernel.expire_sandboxes` (mỗi giờ, chỉ database điều
   khiển, chỉ chế độ `multi`) xoá sandbox quá hạn và sandbox mà khách gốc đã xoá / gộp.
6. **Header**: lớp HTTP của kernel đặt một ô vào request; chỗ giải khách của request đánh dấu khi khách là sandbox; response
   (REST, MCP, công khai, cả lỗi) mang `Autoxide-Sandbox: true` và `X-Robots-Tag: noindex, nofollow`.
   `GET /api/v1/kernel/tenant` thêm `kind`, `sandbox_of`, `sandbox_expires_at` (struct chuyển sang `tenancy/current.rs`).
7. **Tool MCP** theo quy ước tên của kernel (động từ + danh từ): `kernel.list_sandboxes`, `kernel.create_sandbox`,
   `kernel.extend_sandbox` thay cho `kernel.sandboxes.list|create|extend` của đặc tả. Không có tool xoá.
8. Khách gốc bị tạm khoá chưa chặn sandbox của nó ở mỗi request (cần thêm một lần tra khách gốc mỗi request): sandbox vẫn
   chỉ vào được bằng uỷ quyền của khách gốc, mà người của khách bị khoá không đăng nhập được.

Căn cứ pháp lý: Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15 + NĐ 356/2025/NĐ-CP (hiệu lực 01/01/2026), nguyên tắc tối thiểu
hoá — sandbox chỉ mang cấu hình, dữ liệu nhập thử bị xoá cùng sandbox khi hết hạn; website thử mang `noindex`. Có phải
thông báo website thử với Bộ Công Thương (NĐ 52/2013/NĐ-CP sửa bởi NĐ 85/2021/NĐ-CP, Luật Thương mại điện tử 2025) hay
không: cần đối chiếu văn bản gốc / người có chuyên môn pháp lý.

Lý do: dùng lại sổ khách, luồng cấp phát, luồng xoá và uỷ quyền có sẵn thay vì dựng đường riêng; cùng placement giữ đúng vị
trí dữ liệu khách đã chọn và không cần database mới.

Đã cân nhắc: bảng sandbox riêng ở mặt điều khiển (lặp sổ khách, mọi chỗ tra khách phải hỏi hai bảng); sandbox là khách con
(`parent_id` = khách gốc — lẫn vào báo cáo gộp và phân phối); job sao chạy ở sandbox (worker bỏ qua khách `provisioning`);
giữ chỗ hạn mức qua `plans` (mặc định 0 khi gói không khai).
