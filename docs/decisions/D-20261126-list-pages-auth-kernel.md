## D-20261126-list-pages-auth-kernel · Danh sách của `auth` và kernel: trang keyset cho bảng không entity, nguồn gộp, `page_of` — 2026-11-26 — agent quyết

Chủ đề: HTTP, REST, OpenAPI

Quyết định: ghi lại các lựa chọn khi đổi các danh sách còn lại của `auth` và kernel sang `Page<T>` (bước 70 P9 mục 3,
changelog `p9-list-pages-cluster-3a`), bổ sung D-20261125-list-pages:

1. **Bảng của kernel không có entity SeaORM** (dựng câu bằng `sea_query`): phân trang bằng hàm nội bộ của kernel
   `fetch_statement_page` / `fetch_statement_page_by` (`pagination/raw.rs`) — cùng con trỏ, cùng thứ tự, cùng `count`
   với `fetch_page` / `fetch_page_by`, keyset trong database. Không công khai (module dùng entity).
2. **Quyền xem phụ thuộc từng dòng mà SQL không kiểm được** (yêu cầu duyệt: người gọi quyết được permission của
   lệnh; trường tự định nghĩa: permission đọc của loại bản ghi): `fetch_statement_page_filtered` đọc từng lô
   `MAX_LIMIT` dòng từ con trỏ (keyset trong database), giữ dòng được xem tới khi đủ trang + 1; con trỏ là dòng cuối
   được giữ. `count=true` đi hết danh sách một lượt nữa. Không lấy hết rồi cắt, không trần đọc.
3. **Danh sách gộp từ nhiều nguồn cùng khoá UUIDv7**: mỗi nguồn đọc một trang keyset với cùng con trỏ id rồi gộp
   theo id (`merge_id_pages` của kernel; `auth` làm tương tự cho uỷ quyền nhận từ các khách cấp trên — tối đa 16
   tầng, có thể ở database khác): mỗi nguồn cho `limit` dòng đầu sau con trỏ nên `limit` dòng đầu của bản gộp là
   trang đúng; tổng là tổng các nguồn. Hoạt động của bản ghi (ghi chú + event audit) nay xếp theo id (thời điểm ghi)
   thay cho `at`.
4. **`RecordHistory::history_page`** (hợp đồng công khai của kernel, thêm hàm có cài đặt mặc định — không phá
   module cũ): một trang event theo id. Cài đặt mặc định đọc hết qua `history` rồi cắt trong bộ nhớ (chỉ cho nhà
   cung cấp ngoài chưa cài); `audit` cài keyset trên index (tenant_id, subject, id).
5. **Membership theo nút** (`/auth/users/{id}/memberships`, `/auth/me/memberships`): keyset trên các `org_id` khác
   nhau ngay trong SQL (`SELECT DISTINCT org_id … WHERE org_id > ? ORDER BY org_id LIMIT n+1`, unique (tenant_id,
   user_id, org_id, role_id)), con trỏ `text_key_cursor("org_id", …)`; rồi đọc role của các nút trong trang.
6. **Chỗ dùng `page_of`**:
   - dựng từ code: `/auth/permissions`, `/kernel/blocks`, `/kernel/connections/webhooks`, `/kernel/modules` (khoá là
     vị trí khởi động, đủ 5 chữ số), `/kernel/records/types`, `/kernel/sandboxes/promotable`, `/kernel/sync/resources`,
     `/kernel/whats-new` (changelog trong bản build; khoá là vị trí trong changelog), `/kernel/settings` và
     `/kernel/tenants/{id}/settings` (cấu hình module khai, ghép giá trị khách đã đặt);
   - code gộp với dữ liệu khách nhỏ có trần: `/kernel/texts` (câu `ui.*` có trần `AUTOXIDE_TEXT_OVERRIDES_UI_MAX`),
     `/kernel/reasons/{list}` (thứ tự `position` khách đổi được chỉ biết sau khi gộp mặc định của module với dòng của
     khách);
   - tính trên cả cây trong một lượt: `/auth/memberships/counts` (`total` của một nút cần người của mọi nút trên nó,
     như giá trị tồn `inventory`);
   - đi cây / gộp nhiều database: `/auth/delegations/tenants` (cây khách đi từng tầng trên sổ khách, thứ tự (tầng,
     slug)), `/auth/me/tenants` (khách nhà + uỷ quyền + lần cấp hỗ trợ, như độ trễ `reporting`).
7. **Chưa đổi**: `/kernel/records/{kind}/{id}/usages` — gộp từ `ReferenceSource` của nhiều module (`content`,
   `menus`) mà hợp đồng chưa có con trỏ; phân trang trong database cần thêm hàm trang vào trait và cài ở từng module.
   Giữ dòng `api-unpaged-list` có lý do, chờ phiên điều phối quyết.

Lý do: giữ một hình dạng danh sách cho mọi endpoint mà không lấy hết rồi cắt; bảng của kernel viết bằng `sea_query`
cần cùng hành vi trang với bảng có entity; quyền theo dòng không được làm hụt trang hay lộ dòng.

Đã cân nhắc: lọc quyền trong SQL cho trường tự định nghĩa (`target IN (…)`) — loại: loại con `<target>.<khoá>` của
loại khai `with_subtypes` chỉ kiểm được trong code; đổi thứ tự hoạt động sang keyset (at, id) — loại: event audit có
`occurred_at` khác lúc ghi, cần index mới ở `audit` và hai khoá con trỏ cho hai nguồn.

Bổ sung theo vòng kiểm của phiên điều phối (26/11/2026): mặc định của `RecordHistory::history_page` giữ lại (D-038: hàm
mới của trait công khai phải có cài đặt mặc định) nhưng doc nói rõ nó chỉ để tương thích, đọc hết rồi cắt; bên cài
ghi đè bằng keyset. Các dòng `xtask/api-breaking-allow.json` cho `limit` của jobs / activity / devtools events trỏ về
D-20261125-list-pages: tham số riêng của từng endpoint thay bằng tham số trang chung (cùng khoảng 1–200, mặc định 50),
kiểu số đổi từ int64 sang int32 như mọi `limit` khác. Mỗi thay đổi hành vi có một dòng cũ → mới ở changelog
`p9-list-pages-cluster-3a-behaviour.changed.md`.
