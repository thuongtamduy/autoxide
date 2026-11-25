## D-20261125-installed-module · Module "đã cài" = bảng lịch sử migration có ít nhất một dòng — 2026-11-25 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định: module "đã cài" ở một database nghĩa là bảng lịch sử migration của nó (`<module>_migrations`) có **ít nhất
một dòng**, kiểm ở một chỗ duy nhất `autoxide_kernel::migration::installed`. Bảng có mà rỗng (sau `migrate down` hết) thì
coi như chưa cài: chế độ nghỉ không migrate, không xuất, không xoá, không dời dữ liệu của module đó ở database ấy.
Đính chính D-20261125-dormant-modules (mục 2 và 3b ghi "có `<module>_migrations`").

Lý do: `migrate down` hết để lại bảng lịch sử rỗng; coi đó là "đã cài" thì chế độ nghỉ dựng lại bảng của module người
quản trị vừa gỡ, và bước xuất / xoá / dời đi tìm bảng không còn.

Đã cân nhắc: chỉ kiểm bảng lịch sử có tồn tại (cách ghi cũ) — loại vì lý do trên; kiểm từng bảng nghiệp vụ của module
— loại: module không khai danh sách bảng ở một chỗ máy đọc được, và bảng lịch sử đã là nguồn sự thật của migration.
