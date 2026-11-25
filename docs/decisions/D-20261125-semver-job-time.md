## D-20261125-semver-job-time · Job `semver` của CI: một target dir chung cho mọi crate và chia 4 máy — 2026-11-25 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: `cargo xtask semver` gọi `cargo semver-checks` với `CARGO_TARGET_DIR=target/semver-checks` (một target dir
chung cho mọi crate, xoá ở đầu mỗi lần chạy) và nhận `--shard k/n` (crate thứ k, k+n, … của danh sách). Job CI tách
thành `semver (plan)` → `semver 1/4` … `semver 4/4` → job gộp giữ tên check cũ "Rust API compatibility
(cargo-semver-checks)", in một mục `### Breaking (Rust)` duy nhất. PR sửa `xtask/src/semver.rs` chạy đủ mọi crate như
PR phát hành, để mỗi lần sửa bộ kiểm đều chứng minh nó còn xong trong giới hạn. Không đổi phiên bản công cụ (0.51.0),
không bỏ crate hay lint nào.

Lý do: ở PR phát hành `v0.19.0` (#495) job so 28 crate và bị huỷ sau 90 phút. Đọc mã nguồn `cargo-semver-checks`
0.51.0: mỗi crate được dựng trong một project giả riêng (`target/semver-checks/local-<crate>-…`) với target dir riêng,
cho cả baseline lẫn bản hiện tại — 28 × 2 lần biên dịch lại toàn bộ phụ thuộc. Bước so sánh chỉ mất dưới 1 giây mỗi
crate; gần như toàn bộ thời gian là bước dựng rustdoc JSON. Đo trên máy 4 nhân (cùng số nhân với runner
`ubuntu-24.04` của GitHub), so `v0.18.0` → `v0.19.0`, ngày 2026-11-25:

| Crate             | Cách cũ (dựng hiện tại + baseline) | Target dir chung, chạy riêng |
| ----------------- | ---------------------------------- | ---------------------------- |
| `autoxide-kernel` | 1 phút 57 giây (55 + 55 giây)      | 1 phút 18 giây (54 + 17)     |
| `autoxide-address`| 3 phút 30 giây (103 + 102 giây)    | 2 phút 5 giây (58 + 63)      |
| `autoxide-orders` | 7 phút 52 giây (236 + 230 giây)    | 6 phút 45 giây (209 + 190)   |

Đủ 28 crate trên một máy với target dir chung: 57 phút 47 giây (cách cũ: quá 90 phút ở CI). Dùng chung chỉ cứu được
một phần vì mỗi project giả chỉ kéo đồ thị phụ thuộc của một crate, nên feature của phụ thuộc hợp nhất khác nhau giữa
các crate và cargo phải biên dịch lại (vd `autoxide-kernel` có 20 bản biên dịch khác nhau trong một lần chạy) — crate có
cùng tập feature với crate trước thì gần như miễn phí (`idempotency`, `ai`, `address`, `audit`… 6–14 giây), crate khác
tập thì vẫn tốn 1–5 phút (`autoxide` 8 phút 15 giây, `plans` 5 phút 18 giây, `content`, `inventory` ~5 phút). Vì vậy
cần thêm chia máy: 4 phần chạy song song. Kết quả không đổi: cùng project giả, cùng feature, cùng lệnh `cargo doc` do
công cụ tự dựng — chỉ thư mục biên dịch là chung. Đã so cách cũ và cách mới trên `autoxide-kernel` từ `v0.14.0`: cả hai
báo đúng 6 lỗi, nội dung giống hệt. Thư mục chung bị xoá ở đầu mỗi lần chạy vì bản baseline và bản hiện tại của cùng
crate ghi cùng một file `doc/<crate>.json`: trong một lần chạy mỗi bản dựng đúng một lần và được đọc ngay, nhưng giữ
thư mục sang lần sau thì cargo có thể coi bản hiện tại là "fresh" và công cụ đọc nhầm JSON của baseline.

Kết quả `v0.18.0` → `v0.19.0` (chạy bằng cách mới, đủ 28 crate, so như bản patch): không crate nào có thay đổi phá
vỡ — `CHANGELOG.md` của 0.19.0 không thiếu mục nào.

Đã cân nhắc:
- Chỉ nới `timeout-minutes`: một job hơn 1,5 giờ chặn phát hành từ rc.2, và lớn dần theo số module.
- Tự dựng rustdoc JSON một lần cho cả workspace (`cargo rustdoc` với `RUSTC_BOOTSTRAP`) rồi đưa cho công cụ qua
  `--baseline-rustdoc` / `--current-rustdoc`: nhanh nhất, nhưng feature hợp nhất trên cả workspace khác feature của
  project giả từng crate, nên phần công khai được so có thể khác cách cũ — trái yêu cầu "kết quả giống hệt".
- Cache rustdoc JSON của baseline theo tag: baseline của một tag không đổi, nhưng mỗi tag chỉ được so ở đúng một PR phát
  hành (PR thường so với nhánh gốc của PR), nên cache gần như không bao giờ trúng; thêm bước lưu cache ở `main` là
  phức tạp hơn mà không cứu lần chạy đầu.
- Chia máy mà không dùng chung target dir: mỗi phần vẫn biên dịch lại phụ thuộc cho từng crate (ước lượng từ bảng trên:
  hơn 25 phút mỗi phần trong 4 phần), sát giới hạn và lớn dần theo số module.
