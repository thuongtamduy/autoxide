## D-20261118-step38-relocation · Tách / gộp khách: chính sách theo bảng, giữ khoá chính, audit ghi lại ở đích, chỉ nhân sự nền tảng chạy — 2026-11-18 — agent quyết

Chủ đề: Đa khách, tổ chức, gói

Quyết định: Bước 38 mục 3 và mục D-210 làm theo đặc tả 38 (Luồng 3), với các điểm chốt sau.

1. **Hợp đồng ở kernel** (`autoxide_kernel::tenancy::relocate`): `Module::relocation()` thêm ở CUỐI trait, mặc định
   `Relocation::inferred()` (D-038). Chính sách theo bảng (`TablePolicy`): `ByOrg`, `FollowParent`, `TenantWide { Split::Stay
   | Copy, Merge::Import | Skip }`, `MustBeEmpty`, `Custom(Arc<dyn RelocateTable>)` như đặc tả, thêm ba loại: `OrgTree` (bảng
   soi cây tổ chức — nút cha của gốc nhánh thành `NULL` khi tách, thành nút nhận khi gộp), `Referenced` (dòng được CHÉP theo
   dòng trỏ tới nó — tài khoản của người có membership ở nút trong nhánh), `HandledBy` (bảng do bảng khác của module xử lý:
   `kernel_org_closure`, `audit_chain`). Kèm `Relocation::sealed(bảng, cột, mục đích, cột dòng)`: secret mã hoá theo khách
   (`Sealer`, AAD có khách) được mở bằng khách nguồn và mã hoá lại cho khách đích khi chép. Bảng không khai thì suy từ cột
   (cảnh báo `relocation_policy_inferred` trong kế hoạch); test đầu-cuối của binary (`split_and_merge_with_every_module`) chặn
   bảng cấp khách nào chưa khai (`TenantRelocations::uncovered_tables`). Kernel kiểm lúc khởi động: bảng thuộc chính module,
   không khai hai lần; lúc lập kế hoạch: không vòng, `FollowParent` chỉ theo bảng tạo trước.
2. **Nguồn và đích luôn là hai database khác nhau** (`tenancy.relocation_same_database`). Phần lớn bảng có khoá chính chỉ
   `id`, nên "giữ khoá chính" và "chép trong một transaction ở đích rồi mới xoá nguồn" không làm được trong cùng một database.
   Muốn tách sang placement chung hay gộp hai khách cùng database: dời một khách trước bằng `autoxide tenant move` (bước 33).
3. **Khách mới khi tách có ID = ID nút gốc nhánh** (nút gốc của khách có ID bằng ID khách, D-198), khách cha = khách nguồn
   trừ khi `--no-keep-parent`. Gộp đặt gốc của khách bị gộp làm con của nút nhận; nên tách rồi gộp lại trả mọi dòng về đúng
   chỗ cũ (test so mọi bảng cấp khách của khách nguồn trước tách và sau gộp, trừ audit và hàng đợi — hai thứ thêm dòng theo
   thiết kế).
4. **Trạng thái** ở bảng mặt điều khiển `kernel_relocations` (migration kernel `m0035_create_relocations`): `requested` (khách
   gửi) → `planned` → `frozen` → `copied` → `verified` → `switched` → `cleaned` → `done`, hoặc `failed` (có xung đột lúc bắt
   đầu, hay hàng đợi không rỗng kịp — khách mở lại, chưa đổi gì). Lease `runner`/`runner_until` như dời khách; mỗi bước chạy
   lại được (`--resume`, job `kernel.relocate`). `frozen`: khách thành `maintenance` (503, worker vẫn chạy), chờ outbox và
   job ĐẾN HẠN của khách chạy hết trong `AUTOXIDE_RELOCATION_DRAIN_SECONDS` (mặc định 300), rồi `moving` (worker dừng việc
   của khách), chờ TTL cache + request đang chạy; job hẹn giờ sau và dòng đã giao ở lại nguồn, không chép. Gộp khoá cả khách
   nhận. `switched`: một transaction ở database điều khiển — khách mới `active` / khách bị gộp thành trạng thái mới `merged`
   (request nhận `tenant_not_found`; không thêm cột `merged_into` — khách nhận đọc từ lượt gộp trong `kernel_relocations`),
   tên miền gắn nút trong nhánh và khách con của khách bị gộp đi theo. API key và membership là bảng `ByOrg` của `auth` nên
   đi cùng lúc chép, không ở bước chuyển.
5. **Xung đột tìm ở kế hoạch, không giữa lúc chép**: tham chiếu vắt qua ranh giới theo foreign key trong module (dòng đi mà
   dòng cha không có ở đích; dòng ở lại mà dòng cha bị xoá) và giá trị unique đã có ở khách nhận (unique index thường, đọc từ
   catalog của database). Dòng mà khoá chính đã có ở đích khi gộp là bản đã chép khi tách — bỏ qua (`ON CONFLICT DO NOTHING`).
   Có xung đột: lượt lưu `failed` kèm báo cáo, trả `tenancy.relocation_conflict`.
6. **Audit**: chuỗi băm không dời dòng được; `audit` cài `Custom` — dòng về thứ đi theo (`subject` chứa ID của nút trong
   nhánh hay của một dòng đi / được chép) được ghi lại vào chuỗi của khách nhận bằng đúng cách băm của bộ ghi (giữ `id`,
   `occurred_at`, `recorded_at`, phần dữ liệu), dòng gốc ở lại nguồn. Gộp: mọi dòng của khách bị gộp được ghi lại, trừ `id`
   khách nhận đã có. Sổ cái đi theo sổ (`FollowParent` theo `book_id`, kể cả tài khoản đối tượng là nút ngoài nhánh); chuỗi
   `ledger-v1` không băm `tenant_id` nên kiểm được ở đích.
7. **File**: khoá `tenants/<khách>/<module>/<id>/…` (D-259) mà `<id>` là dòng đi (gộp: mọi file) được chép sang
   `tenants/<khách đích>/…` trước khi chép dòng và xoá ở nguồn ở bước dọn. File của `files_file` (không gắn nút) ở lại.
8. **Dữ liệu cá nhân theo khách** (người đăng ký bản tin, đồng ý, định danh, bình luận, thông báo): `Split::Stay` khi tách,
   `Merge::Import` khi gộp; tài khoản nhân viên có membership ở nút trong nhánh được chép (`Referenced`). Không có cờ chép
   người mua — làm khi module có người mua cần (bước 47). Lệnh và API bắt buộc `reason` (lưu cùng lượt, vào audit) và in cảnh
   báo căn cứ pháp lý (Luật 91/2025/QH15 + NĐ 356/2025 — cần đối chiếu văn bản gốc / người có chuyên môn pháp lý).
9. **Kho báo cáo**: kernel ghi `kernel.relocation_finished` (kèm khách gốc trước / sau) ở khách nhận; `reporting` nghe và xếp
   job `reporting.rehome`: chép số liệu của phần đã đi từ kho của gốc cũ sang kho của gốc mới với khách nguồn mới, tính lại
   `reporting_daily` hai bên, xoá ở kho cũ; nhật ký `reporting_rehomes`. Kernel ghi thêm `kernel.relocation_started` /
   `kernel.relocation_failed` ở khách nguồn (tên `kernel.*` thay `tenancy.*` của đặc tả: event của module `kernel`).
10. **D-210**: REST `POST /api/v1/kernel/relocations` (dry-run mặc định, chạy thật bằng job, `Location` tới
    `GET /api/v1/kernel/relocations/{id}`) chỉ cho nhân sự nền tảng (`kernel.tenants.relocate` + tài khoản khách nền tảng —
    permission mới thay `kernel.platform.manage` không có trong code). Khách chỉ gửi yêu cầu:
    `POST /api/v1/kernel/tenant/relocation-requests` (`kernel.relocations.request`) lưu lượt `requested`, vào hộp việc của
    nhân sự nền tảng (nguồn việc `kernel.relocation_requests`); nhân sự chạy nó bằng `request_id`. Không có tool MCP. CLI:
    `autoxide tenant split|merge|relocation-status`.

Lý do: chính sách theo bảng giữ luật sở hữu bảng (module tự nói bảng của mình đi thế nào) và để kernel làm phần chung (chép
theo kiểu cột, kiểm, chuyển, dọn) một lần cho mọi module. Đòi hai database khác nhau là cách duy nhất giữ khoá chính mà không
transaction xuyên database. ID khách mới = ID nút gốc làm gộp ngược thành phép nghịch đảo chính xác.

Đã cân nhắc: dời trong cùng database bằng `UPDATE tenant_id` (không giữ được dòng audit cùng `id` ở hai chuỗi, không chép được
cấu hình `Copy`); cột `merged_into` trên `kernel_tenants` (thêm migration cho thông tin đã có trong lượt gộp); bảng yêu cầu
riêng cho khách (một bảng `kernel_relocations` với trạng thái `requested` là đủ); tool MCP cho tách / gộp (đặc tả không có —
việc của người vận hành, không hoàn tác bằng một lệnh).
