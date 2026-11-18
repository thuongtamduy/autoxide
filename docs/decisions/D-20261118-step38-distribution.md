## D-20261118-step38-distribution · Phân phối cha → con và vào ngữ cảnh con xuyên database — 2026-11-18 — agent quyết

Chủ đề: Đa khách, tổ chức, gói

Quyết định:

1. **Khung phân phối ở kernel** (`autoxide_kernel::tenancy::distribution`), bảng thuộc module dùng nó qua
   `distribution_migration(name, module)` (`<module>_distributions`, `<module>_distribution_targets`, ở database của khách
   cha) — như `outbox_migration`. `StartContext::distributor()` cho module bộ phân phối; `publish` chạy trong transaction
   ghi của khách cha, trùng (loại, bản ghi, phiên bản) trả bản cũ; đích là con trực tiếp, mọi cấp dưới hay danh sách (khách
   ngoài cây → `tenancy.distribution_target_invalid`); job `<module>.distribute` (unique key theo đích) mở ngữ cảnh con qua
   sổ database, kiểm còn là cấp dưới và đã bật module (không → `skipped` kèm lý do), gọi `DistributionApply::apply`, rồi
   ghi kết quả ở database cha. `apply` phải idempotent theo `DistributionItem::reference()` (D-184) và `source_version`.
   `status` trả `None` khi khách không có bản đó (module tự trả 404 của mình); `retry_failed` chạy lại đích `failed` với
   unique key mới.
2. **Vào ngữ cảnh con xuyên database**: cơ chế uỷ quyền của bước 34 đã lấy database qua `TenantContext` của request nên
   chạy được khi con có database riêng — chỉ thêm test đầu-cuối. "Audit hai phía" là: việc làm ở con ghi event ở database
   của con với `actor_access = delegation` + ID uỷ quyền (audit của con thấy người của khách cha, ID trỏ về uỷ quyền ở
   cha); việc cấp / thu hồi ghi ở database của cha. Không thêm event `tenancy.delegated_access` cho mỗi request.
3. Testkit thêm `TestAppBuilder::child_tenant(slug, parent, dedicated)` và `principal_in(slug, token, principal)`.

Lý do: nhiều module sẽ phân phối (danh mục, bảng giá, khuyến mại) nên khung chung ở kernel; bảng theo module giữ luật sở
hữu bảng. Một event mỗi request vào con là một lần ghi ở database của cha trên đường đọc của con, nhân theo số request,
mà uỷ quyền đã là bản ghi có audit ở cha; ID uỷ quyền trên mỗi thay đổi ở con đủ để nối hai phía.

Đã cân nhắc: khung phân phối trong từng module (lặp lại); bảng phân phối chung ở kernel (vi phạm sở hữu bảng, khó xoá
theo module); ghi `tenancy.delegated_access` ở cha mỗi request (tốn, ghi xuyên database trên đường request).
