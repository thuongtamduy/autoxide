## D-20261118-step38-report-facts · Báo cáo gộp: `Module::facts`, kho ở database của khách gốc, hai chặng cục bộ → gốc — 2026-11-18 — agent quyết

Chủ đề: Đa khách, tổ chức, gói

Quyết định: Bước 38 mục 1 làm theo đặc tả 38, với các điểm chốt sau.

1. **Hợp đồng ở kernel** (`autoxide_kernel::report`): `Fact`, `FactKind` (`kind`, mô tả, `Measure::Amount | Quantity`,
   chiều được phép), `FactSource { event, kinds, extract: fn(&Delivered) }`; `Module::facts()` thêm ở CUỐI trait, mặc định
   rỗng (D-038); `StartContext::fact_sources()`. Kernel kiểm lúc khởi động: nguồn chỉ nghe event của chính module và đã
   khai ở `Module::events`; mỗi event một nguồn (thứ tự số liệu trong event là khoá chống trùng); một loại có thể sinh từ
   nhiều event của cùng module khi khai giống hệt (vd `ledger.revenue` từ ghi sổ và đảo sổ); `extract` trả số liệu sai
   khai báo là lỗi nội bộ.
2. **Hai chặng, không transaction xuyên database**: bộ nhận event ghi `reporting_local_facts` ở database nguồn (trùng
   thì bỏ qua — D-054); lịch `reporting.forward` (mỗi `AUTOXIDE_REPORTING_FORWARD_SECONDS`, trên từng database) đẩy theo
   cờ `forwarded_at IS NULL` (không theo con trỏ id), gom khách cùng gốc, một transaction ở database của khách gốc chèn
   số liệu và chỉ cộng vào `reporting_daily` dòng thật sự được chèn, commit rồi mới đặt `forwarded_at` ở nguồn. Cộng dồn
   bằng "chèn dòng 0 nếu chưa có (`ON CONFLICT DO NOTHING`) rồi `UPDATE … SET x = x + ?`" — hai câu SeaORM chạy như nhau
   trên hai database, không cần `excluded.` của upsert. Gốc đang bảo trì → số liệu chờ, cây khác vẫn đẩy; đọc trả
   `reporting.store_unavailable`.
3. **Số liệu không nêu nút thì thuộc nút gốc** (`org_id` = ID khách), không lưu UUID rỗng như phác thảo của đặc tả: lọc
   theo nhánh / nút coi nó như mọi nút khác.
4. **Chiều lọc ở bảng riêng** `reporting_fact_dimensions` (tenant_id, fact_id, name, value) ghi cùng transaction với số
   liệu; `GET /facts` lọc bằng `dimensions=name:value,…` (một tham số có tài liệu OpenAPI) thay cho `dimension.<tên>=`.
5. **Phạm vi xem**: độ sâu `tenant` (không hàng rào) ở khách X → X và mọi khách cấp dưới; độ sâu hẹp hơn → chỉ số liệu
   của X ở các nút của người xem. Danh sách nút tính ở database của X (`OrgTree::descendants`, hàm mới) rồi lọc
   `org_id IN (…)` ở kho, vì cây tổ chức của X có thể không nằm ở database của khách gốc. Lọc `tenant_ids` / `org_ids`
   của client chỉ giao; khách ngoài phạm vi → `reporting.tenant_not_found` (404, như khách không tồn tại).
6. **Nhiều tiền tệ**: tổng luôn theo tiền gốc; `currency=` quy đổi từng nhóm (tiền, ngày) bằng tỉ giá của khách NGƯỜI
   XEM (`dyn Ledger::exchange_rate`, đọc trước, ngoài transaction), làm tròn `HalfUp` rồi cộng; thiếu tỉ giá → liệt kê
   ở `missing_rates`, không cộng vào `converted`.
7. **Số liệu đầu tiên**: `invoices.issued_total` / `invoices.issued_count` (chiều `kind`) từ `invoices.document_issued`;
   `ledger.revenue` (chiều `purpose`, bên có trừ bên nợ trên tài khoản `revenue.*`) và `ledger.tax_output`
   (`tax.vat_output`) từ `ledger.entry_posted` và `ledger.entry_reversed` — payload hai event này thêm `lines`
   (`purpose`, `side`, `amount`; thêm trường, không phá vỡ). Huỷ chứng từ chưa sinh số liệu âm (làm khi module cần).
8. Khoảng ngày tối đa: 366 ngày khi nhóm theo ngày (đặc tả), 3660 ngày với cách nhóm khác.

Lý do: kho ở gốc là một chỗ đọc cho cả cây, không thêm cấu hình; hai chặng giữ event handler ngắn (D-158) và để database
gốc bảo trì không làm hỏng event của con. Bảng chiều riêng cho so khớp chính xác, có index, như nhau trên hai database
(`LIKE` trên JSON phân biệt hoa thường khác nhau giữa SQLite và Postgres).

Đã cân nhắc: một database báo cáo riêng (chưa có số đo cho thấy cần — đặc tả để mở); upsert `ON CONFLICT … DO UPDATE` với
`excluded.` (chạy được nhưng hai câu đơn giản hơn và dùng được SeaORM); lọc chiều bằng `LIKE` trên cột JSON (sai khác hoa
thường giữa hai database); lưu UUID rỗng cho số liệu không có nút (lọc theo phạm vi phải xử lý riêng).
