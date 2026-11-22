## D-20261122-module-helpers · Đồ dùng cho người viết module: `autoxide modules`, `helper_tables`, `SignedDuration`, `new-module --external` — 2026-11-22 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định (nối tiếp D-20261121-external-modules, bước 70; phiên điều phối duyệt từ đề xuất của phiên làm P2):

- **Lệnh `autoxide modules` (`--json`)**: mỗi module của binary — kể cả module binary thêm bằng `Autoxide::module` —
  một dòng: tên, hạng, bật / tắt, `essential`, `depends_on`, được bật kèm vì module nào, và với module đang tắt là
  các bảng còn dòng trong database chính (`null` khi không đọc được database). Chỉ đọc: không khởi động module,
  không chạy migration. Tên trường của `--json` là hợp đồng. `Modules::selection_text()` (kernel) cho lệnh đọc giá
  trị `AUTOXIDE_MODULES` đang dùng.
- **`autoxide_kernel::migration::helper_tables(module, &[HelperTable::Events, HelperTable::Jobs])`**: chuỗi migration
  của bảng event (outbox) và bảng job ở dạng hiện tại, tên cố định (`k_events_1_create` … `k_jobs_2_tenancy`), mỗi
  bảng một chuỗi riêng nên thêm bảng về sau là một lời gọi thứ hai ở cuối danh sách. **Chỉ cho module MỚI**: module
  có sẵn giữ nguyên danh sách từng bước của mình (migration đã áp dụng không đổi tên). `HelperTable` là
  `#[non_exhaustive]`; bảng phụ trợ khác (throttle, token, upload…) thêm vào enum khi có module mới cần.
- **`autoxide_kernel::time::SignedDuration`** (re-export của `jiff::SignedDuration`): module cộng trừ `Timestamp` mà
  không phải tự khai `jiff`. Giữ nguyên tên của `jiff` để không lẫn với `std::time::Duration`.
- **`cargo xtask new-module --external <dir> <module> <entity>`**: sinh một workspace Cargo riêng từ
  `examples/external-module` (đổi `bookmarks` / `bookmark`, giữ `acme`), phụ thuộc core bằng git tag của phiên bản
  workspace đang đứng. Không lập repo template trên GitHub (việc ngoài repo).
- **Job CI `external module example`** dùng sccache và mold như job `build tests`, và trên CI build thư viện ngoài
  không tối ưu (`check.sh` tự thêm `--config` khi có `GITHUB_ACTIONS`): lượt đầu của nó là job dài nhất của CI
  (19,5 phút so với ~11 phút của `build tests`, đo trên runner của GitHub ở lô #368).
- **`cargo xtask publish-check` xoá bản giải nén của registry tạm trước khi chạy**: cargo dùng lại bản giải nén của
  gói cùng tên, cùng phiên bản từ lượt trước, làm crate phụ thuộc build với kernel cũ.

Lý do: viết module ví dụ ở P2 lộ ra ba chỗ vướng của người mới — phải chép chuỗi 5–6 migration lịch sử để có event
và job, phải thêm `jiff` chỉ để trừ thời gian, và phải chép tay cả workspace; còn người cài không có chỗ nào thấy
một lượt module nào có, hạng gì, bật hay tắt (`doctor` chỉ có một câu, `/kernel/modules` cần máy chủ đang chạy và
chỉ thấy module đang bật).

Đã cân nhắc: `helper_tables` gộp mọi bảng vào một migration `adopt` chung — thêm bảng về sau sẽ không được nâng vì
migration cùng tên đã áp dụng; đổi module có sẵn sang `helper_tables` — tên migration đã ghi trong database của
mọi bản cài; đặt tên re-export là `time::Duration` — trùng tên với `std::time::Duration` mà `Schedule::new` nhận;
`autoxide modules` xem cả database của từng placement — phải khởi động module, không còn là lệnh chỉ đọc
(`autoxide doctor` đã làm việc đó); template repo trên GitHub — ra ngoài repo, và lệch khỏi ví dụ CI đang kiểm.
